//! 像の模擬（OpticStudio の Image Simulation 相当）: 物体の視野に置いた画像を、画角ごとの回折 PSF・歪曲・
//! 相対照度（周辺光量）・波長ごとの位置（倍率色収差）をかけて像面の検出器の画素に写す。
//!
//! 1. 元の画像は視野の中心に置き、画像の高さ（全体）が [`ImageSimOpts::field_height`]（視野の単位）。画素は正方形。
//!    行 0 が視野の +y、列 0 が −x
//! 2. 写像の格子（[`ImageSimOpts::map_grid`]）の各点で主波長の主光線の像点（歪曲）と相対照度を求め、元の画素ごとに
//!    双線形に補間して、その像点へ画素の明るさを検出器の格子に双線形に置く
//! 3. PSF の格子（[`ImageSimOpts::psf_grid`]）の各点で回折 PSF（FFT PSF、像面の上。中心は主波長の主光線の像点なので
//!    波長ごとの位置のずれを含む）を求めて検出器の画素に積分し、和を 1 にする。PSF の格子の間は、格子点ごとに
//!    双線形の重みをかけた像を作ってその点の PSF で畳み込み（FFT）、足し合わせる（PSF を双線形に混ぜるのと同じ）
//!
//! 明るさは、一様な元の画像（値 1）が収差なしで相対照度そのものになる単位（元の画素ごとに像の面積 / 検出器の画素の面積を
//! かける。相対照度は歪曲による面積の変化を含むので、それ以上は割らない。OpticStudio と同じく検出器の画素の大きさに依らない）。
//! グレーの画像は重みのある全波長（波長の重みで足す）か 1 波長、RGB はチャンネルごとに 1 波長。

use crate::illum::relative_illumination;
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
    /// PSF の瞳の格子（None で自動）
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
    /// 検出器の中心（像面の局所座標。軸上の主光線の像点）
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
    let (w0, h0) = (src.width, src.height);
    if w0 == 0 || h0 == 0 || src.channels.iter().any(|c| c.len() != w0 * h0) {
        invalid!("image simulation: the source image is empty or its channels have the wrong size");
    }
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
    let field_of = |i: f64, j: f64| -> (f64, f64) {
        let fx = (i + 0.5 - w0 as f64 / 2.0) * fpx;
        let fy = (h0 as f64 / 2.0 - (j + 0.5)) * fpx;
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
    for b in 0..m {
        for a in 0..m {
            let h = grid_field(a, b, m, m);
            pos[b * m + a] = chief(h)?;
            if o.relative_illumination {
                ri[b * m + a] = relative_illumination(s, FieldSel::Norm(h.0, h.1), s.primary, 64)?;
            }
        }
    }
    let center = chief((0.0, 0.0))?;
    // 軸上の元の 1 画素の像の大きさ（写像の格子の中央の差分）
    let pixel_img = {
        let d = 1e-3;
        let (x1, _) = chief((d * fpx / nx, 0.0))?;
        let (_, y1) = chief((0.0, d * fpx / ny))?;
        ((x1 - center.0).abs() / d, (y1 - center.1).abs() / d)
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
                    let p = psf(
                        s,
                        FieldSel::Norm(h.0, h.1),
                        Waves::One(w),
                        PsfOpts {
                            pupil: o.pupil,
                            image: o.psf_points,
                            pixel: None,
                            plane: PsfPlane::Image,
                        },
                    )?;
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
            for (k, img) in ideal.iter().enumerate() {
                let acc = &mut out[c];
                match &kernels[k] {
                    None => {
                        for (a, b) in acc.iter_mut().zip(img) {
                            *a += kw * b;
                        }
                    }
                    Some((n, ker)) => {
                        if img.iter().all(|v| *v == 0.0) {
                            continue;
                        }
                        let conv = convolve(img, w1, h1, ker, *n);
                        for (a, b) in acc.iter_mut().zip(&conv) {
                            *a += kw * b;
                        }
                    }
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

/// 像 img（w × h）と核 ker（n × n、中心が原点）の畳み込み（FFT、像の外は 0）
fn convolve(img: &[f64], w: usize, h: usize, ker: &[f64], n: usize) -> Vec<f64> {
    let r = n / 2;
    let (pw, ph) = ((w + n).next_power_of_two(), (h + n).next_power_of_two());
    let mut a = vec![Complex::new(0.0, 0.0); pw * ph];
    let mut b = vec![Complex::new(0.0, 0.0); pw * ph];
    for y in 0..h {
        for x in 0..w {
            a[y * pw + x].re = img[y * w + x];
        }
    }
    // 核の中心を原点に（負の添字は端に回す）
    for y in 0..n {
        for x in 0..n {
            let (px, py) = ((x + pw - r) % pw, (y + ph - r) % ph);
            b[py * pw + px].re = ker[y * n + x];
        }
    }
    fft2(&mut a, pw, ph, false);
    fft2(&mut b, pw, ph, false);
    for (u, v) in a.iter_mut().zip(&b) {
        *u *= v;
    }
    fft2(&mut a, pw, ph, true);
    let sc = 1.0 / (pw * ph) as f64;
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = a[y * pw + x].re * sc;
        }
    }
    out
}

fn fft2(a: &mut [Complex<f64>], w: usize, h: usize, inverse: bool) {
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let (fr, fc) = if inverse {
        (planner.plan_fft_inverse(w), planner.plan_fft_inverse(h))
    } else {
        (planner.plan_fft_forward(w), planner.plan_fft_forward(h))
    };
    for row in a.chunks_exact_mut(w) {
        fr.process(row);
    }
    let mut col = vec![Complex::new(0.0, 0.0); h];
    for x in 0..w {
        for y in 0..h {
            col[y] = a[y * w + x];
        }
        fc.process(&mut col);
        for y in 0..h {
            a[y * w + x] = col[y];
        }
    }
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

    #[test]
    fn splat_conserves_energy_inside() {
        let mut img = vec![0.0; 16];
        splat(&mut img, 4, 4, 1.3, 2.6, 2.0);
        assert!((img.iter().sum::<f64>() - 2.0).abs() < 1e-12);
    }
}
