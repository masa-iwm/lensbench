//! 像の模擬（OpticStudio の Image Simulation 相当）: 物体の視野に置いた画像を、画角ごとの回折 PSF・歪曲・
//! 相対照度（周辺光量）・波長ごとの位置（倍率色収差）をかけて像面の検出器の画素に写す。
//!
//! 1. 元の画像は中心の視野（[`ImageSimOpts::center_field`]、既定は軸上）に置き、画像の高さ（全体）が
//!    [`ImageSimOpts::field_height`]（視野の単位）。画素は正方形。行 0 が視野の +y、列 0 が −x。置く前に元の画像を
//!    反転・回転できる（[`ImageSimOpts::flip_source`]・[`ImageSimOpts::rotate`]）
//! 2. 写像の格子（[`ImageSimOpts::map_grid`]）の各点で主波長の主光線の像点（歪曲）と相対照度を求め、元の画素ごとに
//!    双線形に補間して、その像点へ画素の明るさを検出器の格子に双線形に置く
//! 3. PSF の格子（[`ImageSimOpts::psf_grid`]）の各点で回折 PSF（FFT PSF、像面の上。中心は主波長の主光線の像点なので
//!    波長ごとの位置のずれを含む）を求めて検出器の画素に積分し、和を 1 にする。PSF の格子の間は、格子点ごとに
//!    双線形の重みをかけた像を作ってその点の PSF で畳み込み（FFT）、足し合わせる（PSF を双線形に混ぜるのと同じ）
//!
//! 明るさは、一様な元の画像（値 1）が収差なしで相対照度そのものになる単位（元の画素ごとに像の面積 / 検出器の画素の面積を
//! かける。相対照度は歪曲による面積の変化を含むので、それ以上は割らない。OpticStudio と同じく検出器の画素の大きさに依らない）。
//! グレーの画像は重みのある全波長（波長の重みで足す）か 1 波長、RGB はチャンネルごとに 1 波長。

use crate::illum::exit_cone;
use crate::psf::{psf, PsfOpts, PsfPlane};
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::Waves;
use rustfft::num_complex::Complex;

/// 収差のかけ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Aberrations {
    /// 歪曲と相対照度だけ（PSF をかけない）
    None,
    /// 回折 PSF（FFT PSF）
    #[default]
    Diffraction,
    /// 幾何 PSF（瞳の格子の光線の像面の点の密度。OpticStudio の Aberrations = Geometric）
    Geometric,
    /// Huygens PSF（像面の上で瞳の各点からの球面波を足す。像面が傾いた系・斜めの視野でも像面の上の PSF。
    /// GPU があれば GPU で足す）
    Huygens,
}

/// 画像の反転（OpticStudio の Flip Source・Flip Image）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flip {
    #[default]
    None,
    /// 上下（行の順を逆に）
    TopBottom,
    /// 左右（列の順を逆に）
    LeftRight,
    /// 上下と左右（180 度の回転と同じ）
    Both,
}

/// 元の画像の回転（反時計回り、見たままの向き。OpticStudio の Rotation）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotate {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

/// 画像を反転する
pub fn flip(im: &SimImage, f: Flip) -> SimImage {
    let (w, h) = (im.width, im.height);
    let (fx, fy) = match f {
        Flip::None => return im.clone(),
        Flip::TopBottom => (false, true),
        Flip::LeftRight => (true, false),
        Flip::Both => (true, true),
    };
    let channels = im
        .channels
        .iter()
        .map(|c| {
            (0..w * h)
                .map(|k| {
                    let (i, j) = (k % w, k / w);
                    let (i, j) = (
                        if fx { w - 1 - i } else { i },
                        if fy { h - 1 - j } else { j },
                    );
                    c[j * w + i]
                })
                .collect()
        })
        .collect();
    SimImage {
        width: w,
        height: h,
        channels,
    }
}

/// 画像を反時計回りに回す（90・270 度は幅と高さが入れ替わる）
pub fn rotate(im: &SimImage, r: Rotate) -> SimImage {
    let (w, h) = (im.width, im.height);
    let (w1, h1) = match r {
        Rotate::R0 => return im.clone(),
        Rotate::R180 => return flip(im, Flip::Both),
        Rotate::R90 | Rotate::R270 => (h, w),
    };
    // 出力の画素 (i, j) が読む元の画素（反時計回り 90 度: 出力の上の行は元の右の列を上から下へ）
    let at = |i: usize, j: usize| {
        if r == Rotate::R90 {
            (w - 1 - j, i)
        } else {
            (j, h - 1 - i)
        }
    };
    let channels = im
        .channels
        .iter()
        .map(|c| {
            (0..w1 * h1)
                .map(|k| {
                    let (i0, j0) = at(k % w1, k / w1);
                    c[j0 * w + i0]
                })
                .collect()
        })
        .collect();
    SimImage {
        width: w1,
        height: h1,
        channels,
    }
}

/// 像の模擬の設定
#[derive(Debug, Clone)]
pub struct ImageSimOpts {
    /// 元の画像の高さ（全体）[視野の単位]
    pub field_height: f64,
    /// PSF の格子の点の数 (x, y)（元の画像の範囲に等間隔、端を含む）
    pub psf_grid: (usize, usize),
    /// 歪曲・相対照度の格子の一辺（元の画像の範囲に等間隔、端を含む）
    pub map_grid: usize,
    /// PSF の瞳の格子（None で自動。幾何 PSF は [`GEO_PUPIL`]）
    pub pupil: Option<usize>,
    /// PSF の格子の一辺（FFT PSF の像の格子。検出器の画素に積分する前）
    pub psf_points: usize,
    /// 検出器の画素の大きさ [レンズ単位]（None で元の 1 画素の軸上の像の大きさ）
    pub pixel: Option<f64>,
    /// 検出器の画素の数 (x, y)（None で元の画像と同じ）
    pub size: Option<(usize, usize)>,
    pub aberrations: Aberrations,
    /// 相対照度をかける
    pub relative_illumination: bool,
    /// RGB のチャンネルの波長の番号（0 から、R・G・B の順）。None なら系の波長 1〜3 を短い方から B・G・R に
    pub channel_waves: Option<[usize; 3]>,
    /// グレーの画像の波長の番号（0 から）。None ならウェイトのある全波長
    pub wave: Option<usize>,
    /// 元の画像の中心の視野 (x, y) [視野の単位]（OpticStudio の Field。検出器の中心はその主波長の主光線の像点）
    pub center_field: (f64, f64),
    /// 元の画像の反転（回転の前）
    pub flip_source: Flip,
    /// 元の画像の回転（反転の後）
    pub rotate: Rotate,
    /// 検出器の像の反転
    pub flip_image: Flip,
}

impl Default for ImageSimOpts {
    fn default() -> Self {
        ImageSimOpts {
            field_height: 0.0,
            psf_grid: (5, 5),
            map_grid: 17,
            pupil: None,
            psf_points: 64,
            pixel: None,
            size: None,
            aberrations: Aberrations::Diffraction,
            relative_illumination: true,
            channel_waves: None,
            wave: None,
            center_field: (0.0, 0.0),
            flip_source: Flip::None,
            rotate: Rotate::R0,
            flip_image: Flip::None,
        }
    }
}

/// 画像（行優先、行 0 が上）。チャンネルは 1（グレー）か 3（R・G・B）
#[derive(Debug, Clone, PartialEq)]
pub struct SimImage {
    pub width: usize,
    pub height: usize,
    pub channels: Vec<Vec<f64>>,
}

/// 像の模擬の結果
#[derive(Debug, Clone)]
pub struct ImageSimOut {
    /// 検出器の像（行 0 が像面の +y、列 0 が −x）
    pub image: SimImage,
    /// 検出器の画素の大きさ [レンズ単位]
    pub pixel: f64,
    /// 検出器の中心（像面の局所座標。中心の視野の主波長の主光線の像点）
    pub center: (f64, f64),
    pub warnings: Vec<String>,
}

/// 元の画像の高さ（全体）の既定: 最大の視野（半径）の √2 倍（正方形の画像の角が最大の視野に届く。OpticStudio と同じ）
pub fn default_field_height(s: &System) -> f64 {
    s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max) * std::f64::consts::SQRT_2
}

/// 規則的な格子の上の双線形補間（値は格子点ごとの配列、u・v は 0〜n−1 の格子の座標）
fn bilinear<T: Copy>(
    n: (usize, usize),
    u: f64,
    v: f64,
    at: impl Fn(usize, usize) -> T,
    mix: impl Fn(T, T, f64) -> T,
) -> T {
    // 格子の外は端の升を線形に延ばす（元の画素の像の面積を端でも差分で求めるため）
    let cell = |t: f64, n: usize| (t.floor().max(0.0) as usize).min(n.saturating_sub(2));
    let (i0, j0) = (cell(u, n.0), cell(v, n.1));
    let (i1, j1) = ((i0 + 1).min(n.0 - 1), (j0 + 1).min(n.1 - 1));
    let (fu, fv) = (u - i0 as f64, v - j0 as f64);
    let a = mix(at(i0, j0), at(i1, j0), fu);
    let b = mix(at(i0, j1), at(i1, j1), fu);
    mix(a, b, fv)
}

/// 1 次元の格子の位置 t（0〜n−1）の双線形の重み（点 k の重み）
fn hat(t: f64, k: usize, n: usize) -> f64 {
    if n == 1 {
        return 1.0;
    }
    let t = t.clamp(0.0, (n - 1) as f64);
    (1.0 - (t - k as f64).abs()).max(0.0)
}

/// 像の模擬
pub fn image_sim(s: &System, src: &SimImage, o: &ImageSimOpts) -> Result<ImageSimOut> {
    let mut r = sim(s, src, o)?;
    r.image = flip(&r.image, o.flip_image);
    Ok(r)
}

fn sim(s: &System, src: &SimImage, o: &ImageSimOpts) -> Result<ImageSimOut> {
    if src.width == 0
        || src.height == 0
        || src
            .channels
            .iter()
            .any(|c| c.len() != src.width * src.height)
    {
        invalid!("image simulation: the source image is empty or its channels have the wrong size");
    }
    let src = &rotate(&flip(src, o.flip_source), o.rotate);
    let (w0, h0) = (src.width, src.height);
    if !(src.channels.len() == 1 || src.channels.len() == 3) {
        invalid!("image simulation: the source image must be gray (1 channel) or RGB (3 channels)");
    }
    if !(o.field_height > 0.0) {
        invalid!(
            "image simulation: field height {} must be positive",
            o.field_height
        );
    }
    if o.psf_grid.0 == 0 || o.psf_grid.1 == 0 || o.map_grid < 2 {
        invalid!(
            "image simulation: PSF grid {:?} must be 1 or more and the map grid {} 2 or more",
            o.psf_grid,
            o.map_grid
        );
    }
    let mut warnings = Vec::new();
    // 視野の単位の元の画素の大きさと、元の画素 (i, j) の中心の視野
    let fpx = o.field_height / h0 as f64;
    let (nx, ny) = s.field_norm_values();
    let (fcx, fcy) = o.center_field;
    if !(fcx.is_finite() && fcy.is_finite()) {
        invalid!("image simulation: center field ({fcx}, {fcy})");
    }
    let field_of = |i: f64, j: f64| -> (f64, f64) {
        let fx = fcx + (i + 0.5 - w0 as f64 / 2.0) * fpx;
        let fy = fcy + (h0 as f64 / 2.0 - (j + 0.5)) * fpx;
        (fx / nx, fy / ny)
    };
    // 波長ごとの (波長の番号, 重み) をチャンネルごとに
    let chans: Vec<Vec<(usize, f64)>> = if let (1, Some(w)) = (src.channels.len(), o.wave) {
        if w >= s.wavelengths.len() {
            let (wn, nw) = (w + 1, s.wavelengths.len());
            invalid!("wavelength number {wn} ({nw} in all)");
        }
        vec![vec![(w, 1.0)]]
    } else if src.channels.len() == 1 {
        let ws: Vec<(usize, f64)> = (0..s.wavelengths.len())
            .filter(|&w| s.weights[w] > 0.0)
            .map(|w| (w, s.weights[w]))
            .collect();
        let tot: f64 = ws.iter().map(|x| x.1).sum();
        if !(tot > 0.0) {
            invalid!("no weighted wavelength");
        }
        vec![ws.into_iter().map(|(w, k)| (w, k / tot)).collect()]
    } else {
        let cw = match o.channel_waves {
            Some(c) => c,
            None => {
                if s.wavelengths.len() < 3 {
                    invalid!("image simulation: an RGB image needs 3 wavelengths (or give the channel wavelengths)");
                }
                let mut idx = [0usize, 1, 2];
                idx.sort_by(|&a, &b| s.wave(b).mm().total_cmp(&s.wave(a).mm()));
                idx
            }
        };
        for &w in &cw {
            if w >= s.wavelengths.len() {
                let (wn, nw) = (w + 1, s.wavelengths.len());
                invalid!("wavelength number {wn} ({nw} in all)");
            }
        }
        cw.iter().map(|&w| vec![(w, 1.0)]).collect()
    };
    // 写像の格子: 主波長の主光線の像点と相対照度
    let m = o.map_grid;
    let grid_field = |a: usize, b: usize, na: usize, nb: usize| -> (f64, f64) {
        let t = |k: usize, n: usize, len: usize| {
            if n == 1 {
                (len as f64 - 1.0) / 2.0
            } else {
                k as f64 * (len as f64 - 1.0) / (n - 1) as f64
            }
        };
        field_of(t(a, na, w0), t(b, nb, h0))
    };
    let chief = |h: (f64, f64)| -> Result<(f64, f64)> {
        let c = s.try_trace(
            FieldSel::Norm(h.0, h.1),
            &[0.0],
            &[0.0],
            Some(s.primary),
            None,
            false,
        )?;
        if !c.traced(0) {
            invalid!("image simulation: can't trace the chief ray of field ({:.4}, {:.4}) (normalized); reduce the field height", h.0, h.1);
        }
        Ok((c.rays[0].xyz[0], c.rays[0].xyz[1]))
    };
    let mut pos = vec![(0.0, 0.0); m * m];
    let mut ri = vec![1.0; m * m];
    // 軸上の光束は格子の点ごとに求め直さない（[`relative_illumination`] と同じ値）
    let a0 = if o.relative_illumination {
        exit_cone(s, FieldSel::Norm(0.0, 0.0), s.primary, 64)?
    } else {
        1.0
    };
    for b in 0..m {
        for a in 0..m {
            let h = grid_field(a, b, m, m);
            pos[b * m + a] = chief(h)?;
            if o.relative_illumination {
                ri[b * m + a] = exit_cone(s, FieldSel::Norm(h.0, h.1), s.primary, 64)? / a0;
            }
        }
    }
    let center = chief((fcx / nx, fcy / ny))?;
    // 軸上の元の 1 画素の像の大きさ（軸上の差分。中心の視野が軸の外でも軸上で決める。OpticStudio も同じ）
    let pixel_img = {
        let d = 1e-3;
        let c0 = chief((0.0, 0.0))?;
        let (x1, _) = chief((d * fpx / nx, 0.0))?;
        let (_, y1) = chief((0.0, d * fpx / ny))?;
        ((x1 - c0.0).abs() / d, (y1 - c0.1).abs() / d)
    };
    let pixel = match o.pixel {
        Some(p) if p > 0.0 => p,
        Some(p) => invalid!("image simulation: detector pixel {p}"),
        None => pixel_img.0.max(pixel_img.1),
    };
    let (w1, h1) = o.size.unwrap_or((w0, h0));
    if w1 == 0 || h1 == 0 {
        invalid!("image simulation: detector size {w1} × {h1}");
    }
    // 元の画素 → 写像の格子の座標
    let to_map = |i: f64, j: f64| -> (f64, f64) {
        let t = |k: f64, len: usize| k * (m - 1) as f64 / (len as f64 - 1.0).max(1.0);
        (t(i, w0), t(j, h0))
    };
    // 元の 1 画素の像が検出器の画素より大きいと点で置くと隙間ができるので、元の画素を q × q に分けて置く
    // （q は写像の格子の升ごとの元の 1 画素の像の大きさの最大から）
    let q = {
        let (sx, sy) = (
            (w0 as f64 - 1.0).max(1.0) / (m - 1) as f64,
            (h0 as f64 - 1.0).max(1.0) / (m - 1) as f64,
        );
        let d = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
        let mut big = 0.0f64;
        for b in 0..m {
            for a in 0..m - 1 {
                big = big.max(d(pos[b * m + a], pos[b * m + a + 1]) / sx);
                big = big.max(d(pos[a * m + b], pos[(a + 1) * m + b]) / sy);
            }
        }
        ((2.0 * big / pixel).ceil() as usize).clamp(1, 32)
    };
    let mix2 =
        |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let mix1 = |a: f64, b: f64, t: f64| a + (b - a) * t;
    // PSF の格子
    let (gx, gy) = o.psf_grid;
    let gtx = |i: usize| {
        if gx == 1 {
            0.0
        } else {
            i as f64 * (gx - 1) as f64 / (w0 as f64 - 1.0).max(1.0)
        }
    };
    let gty = |j: usize| {
        if gy == 1 {
            0.0
        } else {
            j as f64 * (gy - 1) as f64 / (h0 as f64 - 1.0).max(1.0)
        }
    };
    let mut out = vec![vec![0.0; w1 * h1]; src.channels.len()];
    for (c, ws) in chans.iter().enumerate() {
        for &(w, kw) in ws {
            // 格子点ごとの PSF（検出器の画素に積分した核、奇数の一辺、中心が主波長の主光線の像点）
            let kernels: Vec<Option<(usize, Vec<f64>)>> = (0..gx * gy)
                .map(|k| -> Result<Option<(usize, Vec<f64>)>> {
                    if o.aberrations == Aberrations::None {
                        return Ok(None);
                    }
                    let h = grid_field(k % gx, k / gx, gx, gy);
                    if o.aberrations == Aberrations::Geometric {
                        let n = o.pupil.unwrap_or(GEO_PUPIL);
                        return geo_kernel(s, FieldSel::Norm(h.0, h.1), w, n, pixel).map(Some);
                    }
                    let p = if o.aberrations == Aberrations::Huygens {
                        crate::huygens::huygens_psf(
                            s,
                            FieldSel::Norm(h.0, h.1),
                            Waves::One(w),
                            crate::huygens::HuygensOpts {
                                pupil: o.pupil,
                                image: o.psf_points,
                                ..Default::default()
                            },
                        )?
                    } else {
                        psf(
                            s,
                            FieldSel::Norm(h.0, h.1),
                            Waves::One(w),
                            PsfOpts {
                                pupil: o.pupil,
                                image: o.psf_points,
                                pixel: None,
                                plane: PsfPlane::Image,
                            },
                        )?
                    };
                    for wn in &p.warnings {
                        if !warnings.contains(wn) {
                            warnings.push(wn.clone());
                        }
                    }
                    Ok(Some(bin_kernel(&p, pixel)))
                })
                .collect::<Result<_>>()?;
            // 格子点ごとの重みつきの理想の像
            let mut ideal = vec![vec![0.0; w1 * h1]; gx * gy];
            let ch = &src.channels[c];
            for j in 0..h0 {
                for i in 0..w0 {
                    let v = ch[j * w0 + i];
                    if v == 0.0 {
                        continue;
                    }
                    let (tx, ty) = (gtx(i), gty(j));
                    for sj in 0..q {
                        for si in 0..q {
                            let fi = i as f64 + (si as f64 + 0.5) / q as f64 - 0.5;
                            let fj = j as f64 + (sj as f64 + 0.5) / q as f64 - 0.5;
                            let (u, t) = to_map(fi, fj);
                            let (x, y) = bilinear((m, m), u, t, |a, b| pos[b * m + a], mix2);
                            let r = bilinear((m, m), u, t, |a, b| ri[b * m + a], mix1);
                            // 検出器の格子の座標（列、行）
                            let cx = (x - center.0) / pixel + w1 as f64 / 2.0 - 0.5;
                            let cy = h1 as f64 / 2.0 - 0.5 - (y - center.1) / pixel;
                            // 元の画素の像の面積（写像の差分）をかける: 一様な元の画像の明るさが相対照度そのものになる
                            // （相対照度は歪曲による面積の変化を含むので、面積で割ると二重になる）
                            let at = |di: f64, dj: f64| {
                                let (u, t) = to_map(fi + di, fj + dj);
                                bilinear((m, m), u, t, |a, b| pos[b * m + a], mix2)
                            };
                            let (xa, xb, ya, yb) =
                                (at(-0.5, 0.0), at(0.5, 0.0), at(0.0, -0.5), at(0.0, 0.5));
                            let area = ((xb.0 - xa.0) * (yb.1 - ya.1)
                                - (xb.1 - xa.1) * (yb.0 - ya.0))
                                .abs();
                            let e = v * r * area / (pixel * pixel) / (q * q) as f64;
                            // 分けた 1 片の像の一辺（検出器の画素の単位）。点でなく正方形として重なりで配る（端の画素も欠けない）
                            let side = area.sqrt() / pixel / q as f64;
                            for kj in 0..gy {
                                let wy = hat(ty, kj, gy);
                                if wy == 0.0 {
                                    continue;
                                }
                                for ki in 0..gx {
                                    let wx = hat(tx, ki, gx);
                                    if wx == 0.0 {
                                        continue;
                                    }
                                    splat_box(
                                        &mut ideal[kj * gx + ki],
                                        w1,
                                        h1,
                                        cx,
                                        cy,
                                        side,
                                        e * wx * wy,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            // 格子点ごとの像と核の畳み込みの和（周波数の上で足して、逆変換は 1 回）
            let mut items = Vec::new();
            for (k, img) in ideal.iter().enumerate() {
                let acc = &mut out[c];
                match &kernels[k] {
                    None => {
                        for (a, b) in acc.iter_mut().zip(img) {
                            *a += kw * b;
                        }
                    }
                    Some((n, ker)) => {
                        if img.iter().any(|v| *v != 0.0) {
                            items.push((img.as_slice(), ker.as_slice(), *n));
                        }
                    }
                }
            }
            if !items.is_empty() {
                let conv = convolve_sum(&items, w1, h1);
                for (a, b) in out[c].iter_mut().zip(&conv) {
                    *a += kw * b;
                }
            }
        }
    }
    Ok(ImageSimOut {
        image: SimImage {
            width: w1,
            height: h1,
            channels: out,
        },
        pixel,
        center,
        warnings,
    })
}

/// 値 e を検出器の格子の座標 (cx, cy)（列、行）に双線形に置く
fn splat(img: &mut [f64], w: usize, h: usize, cx: f64, cy: f64, e: f64) {
    let (x0, y0) = (cx.floor(), cy.floor());
    let (fx, fy) = (cx - x0, cy - y0);
    for (dx, wx) in [(0i64, 1.0 - fx), (1, fx)] {
        for (dy, wy) in [(0i64, 1.0 - fy), (1, fy)] {
            let (x, y) = (x0 as i64 + dx, y0 as i64 + dy);
            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                img[y as usize * w + x as usize] += e * wx * wy;
            }
        }
    }
}

/// 値 e を、中心 (cx, cy)・一辺 side（検出器の画素の単位）の正方形として、画素との重なりの割合で配る
fn splat_box(img: &mut [f64], w: usize, h: usize, cx: f64, cy: f64, side: f64, e: f64) {
    if !(side > 1e-9) {
        return splat(img, w, h, cx, cy, e);
    }
    // 1 次元の重なり: 画素 k は [k − 0.5, k + 0.5]
    let parts = |c: f64, n: usize| -> Vec<(usize, f64)> {
        let (a, b) = (c - side / 2.0, c + side / 2.0);
        let (k0, k1) = (
            (a + 0.5).floor().max(0.0) as i64,
            (b + 0.5).floor().min(n as f64 - 1.0) as i64,
        );
        (k0..=k1)
            .filter_map(|k| {
                let o = (b.min(k as f64 + 0.5) - a.max(k as f64 - 0.5)) / side;
                (o > 0.0).then_some((k as usize, o))
            })
            .collect()
    };
    let (px, py) = (parts(cx, w), parts(cy, h));
    for &(y, wy) in &py {
        for &(x, wx) in &px {
            img[y * w + x] += e * wx * wy;
        }
    }
}

/// 幾何 PSF の瞳の格子の一辺の既定
pub const GEO_PUPIL: usize = 64;

/// 幾何 PSF の核（OpticStudio の Aberrations = Geometric）: 瞳の正方格子（一辺 n の各区間の中心、円の中）の波長 w の
/// 光線が像面に当たる点を、主波長の主光線の像点を中心に検出器の画素へ配る（重みは系のアポダイゼーションの強度、
/// けられた光線は除く）。和を 1 にした奇数の一辺の核
fn geo_kernel(
    s: &System,
    field: FieldSel,
    w: usize,
    n: usize,
    pixel: f64,
) -> Result<(usize, Vec<f64>)> {
    if n < 2 {
        invalid!("image simulation: geometric PSF pupil grid {n}");
    }
    let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
    if !c.traced(0) {
        invalid!("image simulation: the chief ray doesn't reach the image (geometric PSF)");
    }
    let c0 = (c.rays[0].xyz[0], c.rays[0].xyz[1]);
    let (px, py) = lensbench_core::opd::pupil_grid(n);
    let t = s.try_trace(field, &px, &py, Some(w), None, false)?;
    let wts = crate::analysis::apodized(s, field, &px, &py, &vec![1.0; px.len()])?;
    let pts: Vec<(f64, f64, f64)> = (0..px.len())
        .filter(|&i| t.traced(i) && wts[i] > 0.0)
        .map(|i| (t.rays[i].xyz[0] - c0.0, t.rays[i].xyz[1] - c0.1, wts[i]))
        .filter(|p| p.0.is_finite() && p.1.is_finite())
        .collect();
    if pts.is_empty() {
        invalid!("image simulation: no ray of the pupil reaches the image (geometric PSF)");
    }
    let half = pts
        .iter()
        .fold(0.0f64, |a, p| a.max(p.0.abs()).max(p.1.abs()));
    let r = (half / pixel).ceil() as usize + 1;
    let m = 2 * r + 1;
    let mut k = vec![0.0; m * m];
    for &(x, y, wt) in &pts {
        splat(&mut k, m, m, x / pixel + r as f64, r as f64 - y / pixel, wt);
    }
    let tot: f64 = k.iter().sum();
    for v in &mut k {
        *v /= tot;
    }
    Ok((m, k))
}

/// FFT PSF の格子を検出器の画素（大きさ pixel）の核に積分する（各標本を双線形に置く。和を 1 に）。
/// 返り値は (一辺 n（奇数）, 行優先の値。行 0 が +y)
fn bin_kernel(p: &crate::psf::Psf, pixel: f64) -> (usize, Vec<f64>) {
    let (xs, ys) = (p.i.x(), p.i.y());
    let half_w = xs.iter().chain(ys).fold(0.0f64, |a, v| a.max(v.abs()));
    let r = (half_w / pixel).ceil() as usize + 1;
    let n = 2 * r + 1;
    let mut k = vec![0.0; n * n];
    // PSF の標本の間隔が検出器の画素より大きいと点で置くと櫛になるので、標本を q × q に分けて置く
    let step = if xs.len() > 1 {
        (xs[1] - xs[0]).abs()
    } else {
        0.0
    };
    let q = ((2.0 * step / pixel).ceil() as usize).clamp(1, 32);
    for (row, &y) in p.i.rows().zip(ys) {
        for (v, &x) in row.iter().zip(xs) {
            if !v.is_finite() || *v == 0.0 {
                continue;
            }
            for sj in 0..q {
                for si in 0..q {
                    let ox = ((si as f64 + 0.5) / q as f64 - 0.5) * step;
                    let oy = ((sj as f64 + 0.5) / q as f64 - 0.5) * step;
                    let cx = (x + ox) / pixel + r as f64;
                    let cy = r as f64 - (y + oy) / pixel;
                    splat(&mut k, n, n, cx, cy, *v / (q * q) as f64);
                }
            }
        }
    }
    let tot: f64 = k.iter().sum();
    if tot > 0.0 {
        for v in &mut k {
            *v /= tot;
        }
    }
    (n, k)
}

/// 2・3・5 の積で m 以上の最小の数（FFT の速い長さ）
fn fast_len(m: usize) -> usize {
    let mut n = m.max(1);
    loop {
        let mut k = n;
        for p in [2, 3, 5] {
            while k.is_multiple_of(p) {
                k /= p;
            }
        }
        if k == 1 {
            return n;
        }
        n += 1;
    }
}

/// 2 次元の FFT（行と列の変換の計画を持つ）
struct Fft2 {
    w: usize,
    h: usize,
    row: std::sync::Arc<dyn rustfft::Fft<f64>>,
    col: std::sync::Arc<dyn rustfft::Fft<f64>>,
}

impl Fft2 {
    fn new(w: usize, h: usize, inverse: bool) -> Fft2 {
        let mut planner = rustfft::FftPlanner::<f64>::new();
        let (row, col) = if inverse {
            (planner.plan_fft_inverse(w), planner.plan_fft_inverse(h))
        } else {
            (planner.plan_fft_forward(w), planner.plan_fft_forward(h))
        };
        Fft2 { w, h, row, col }
    }

    fn process(&self, a: &mut [Complex<f64>]) {
        let (w, h) = (self.w, self.h);
        for r in a.chunks_exact_mut(w) {
            self.row.process(r);
        }
        let mut col = vec![Complex::new(0.0, 0.0); h];
        for x in 0..w {
            for y in 0..h {
                col[y] = a[y * w + x];
            }
            self.col.process(&mut col);
            for y in 0..h {
                a[y * w + x] = col[y];
            }
        }
    }
}

/// 像 img（w × h）と核 ker（n × n、中心が原点）の組ごとの畳み込みの和（FFT、像の外は 0）。
/// 像と核を 1 つの複素数の格子（実部と虚部）に入れて 1 回の変換で両方の変換を得て、積を周波数の上で足し、逆変換は 1 回。
/// 格子の大きさは w + n − 1 以上の速い長さ（像の外へ回り込まない）
fn convolve_sum(items: &[(&[f64], &[f64], usize)], w: usize, h: usize) -> Vec<f64> {
    use rayon::prelude::*;
    let nmax = items.iter().map(|t| t.2).max().unwrap_or(1);
    let (pw, ph) = (fast_len(w + nmax - 1), fast_len(h + nmax - 1));
    let (fwd, inv) = (Fft2::new(pw, ph, false), Fft2::new(pw, ph, true));
    let zero = || vec![Complex::new(0.0, 0.0); pw * ph];
    let spectrum = |&(img, ker, n): &(&[f64], &[f64], usize)| -> Vec<Complex<f64>> {
        let r = n / 2;
        let mut z = zero();
        for y in 0..h {
            for x in 0..w {
                z[y * pw + x].re = img[y * w + x];
            }
        }
        // 核の中心を原点に（負の添字は端に回す）
        for y in 0..n {
            for x in 0..n {
                let (px, py) = ((x + pw - r) % pw, (y + ph - r) % ph);
                z[py * pw + px].im = ker[y * n + x];
            }
        }
        fwd.process(&mut z);
        // 実の a・b の変換 A = (Z(k) + Z*(−k)) / 2、B = (Z(k) − Z*(−k)) / 2i から A·B = (Z(k)² − Z*(−k)²) / 4i
        let mut out = zero();
        for ky in 0..ph {
            for kx in 0..pw {
                let zk = z[ky * pw + kx];
                let zm = z[((ph - ky) % ph) * pw + (pw - kx) % pw].conj();
                out[ky * pw + kx] = (zk * zk - zm * zm) * Complex::new(0.0, -0.25);
            }
        }
        out
    };
    // 足す順を番号の順に決める（スレッドの数で値が変わらない）。並列に求めるのは決まった数ずつ（使う記憶を抑える）
    const BATCH: usize = 8;
    let mut acc = zero();
    for batch in items.chunks(BATCH) {
        let sp: Vec<Vec<Complex<f64>>> = if batch.len() > 1 && !lensbench_core::par::sequential() {
            batch
                .par_iter()
                .map(lensbench_core::par::scoped(spectrum))
                .collect()
        } else {
            batch.iter().map(spectrum).collect()
        };
        for v in sp {
            for (u, x) in acc.iter_mut().zip(&v) {
                *u += x;
            }
        }
    }
    inv.process(&mut acc);
    let sc = 1.0 / (pw * ph) as f64;
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = acc[y * pw + x].re * sc;
        }
    }
    out
}

/// 像 img（w × h）と核 ker（n × n、中心が原点）の畳み込み（FFT、像の外は 0）
#[cfg(test)]
fn convolve(img: &[f64], w: usize, h: usize, ker: &[f64], n: usize) -> Vec<f64> {
    convolve_sum(&[(img, ker, n)], w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convolve_with_a_delta_is_identity() {
        let img: Vec<f64> = (0..12).map(|v| v as f64).collect();
        let ker = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let out = convolve(&img, 4, 3, &ker, 3);
        for (a, b) in img.iter().zip(&out) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn convolve_shifts_by_the_kernel_offset() {
        let mut img = vec![0.0; 25];
        img[12] = 1.0;
        // 核の中心から +x に 1
        let mut ker = vec![0.0; 9];
        ker[5] = 1.0;
        let out = convolve(&img, 5, 5, &ker, 3);
        assert!((out[13] - 1.0).abs() < 1e-12);
    }

    /// 組ごとの畳み込みの和が、直接の畳み込み（像の外は 0）の和と同じ（大きさの違う核、奇数と偶数の像）
    #[test]
    fn convolve_sum_matches_direct_convolution() {
        let mut st = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st >> 11) as f64 / (1u64 << 53) as f64
        };
        let (w, h) = (13, 8);
        let imgs: Vec<Vec<f64>> = (0..3)
            .map(|_| (0..w * h).map(|_| rnd()).collect())
            .collect();
        let kers: Vec<(usize, Vec<f64>)> = [3usize, 7, 5]
            .iter()
            .map(|&n| (n, (0..n * n).map(|_| rnd()).collect()))
            .collect();
        let items: Vec<(&[f64], &[f64], usize)> = imgs
            .iter()
            .zip(&kers)
            .map(|(i, (n, k))| (i.as_slice(), k.as_slice(), *n))
            .collect();
        let got = convolve_sum(&items, w, h);
        let mut want = vec![0.0; w * h];
        for (img, ker, n) in &items {
            let r = (*n / 2) as i64;
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    let mut v = 0.0;
                    for ky in 0..*n as i64 {
                        for kx in 0..*n as i64 {
                            let (sx, sy) = (x - (kx - r), y - (ky - r));
                            if sx >= 0 && sy >= 0 && sx < w as i64 && sy < h as i64 {
                                v += ker[(ky * *n as i64 + kx) as usize]
                                    * img[(sy * w as i64 + sx) as usize];
                            }
                        }
                    }
                    want[(y * w as i64 + x) as usize] += v;
                }
            }
        }
        for (a, b) in got.iter().zip(&want) {
            assert!((a - b).abs() < 1e-12, "{a} vs {b}");
        }
    }

    #[test]
    fn splat_conserves_energy_inside() {
        let mut img = vec![0.0; 16];
        splat(&mut img, 4, 4, 1.3, 2.6, 2.0);
        assert!((img.iter().sum::<f64>() - 2.0).abs() < 1e-12);
    }
}
