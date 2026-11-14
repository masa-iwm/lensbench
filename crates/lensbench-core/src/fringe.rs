//! 単色の干渉縞: 試料・参照の 2 つの状態（多重構成の構成、プロジェクトの状態）の光を重ねた縞。
//!
//! - 瞳上（[`interferogram`]、OpticStudio の Interferogram に当たる）: 2 つの光束の波面（それぞれの主光線・参照球に
//!   対する OPD [波]）を同じ正規化瞳座標で引き、Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2 の縞
//!   I = (1 + cos 2πΔ) / 2 を出す。光束 2 を省くと参照球との干渉（W1 だけ）。
//!   格子は OpticStudio の Interferogram と同じ [`pupil_grid`]。瞳は正規化した入射瞳の座標で、OpticStudio の
//!   「Use Exit Pupil」を外したときと同じ（既定の射出瞳の形を使う設定では、軸外で射出瞳の形に合わせて点を
//!   取り直すので、縞の位置が少し違う）。A0 系の 16 条件で強度の差は 2e-9 以下（非公開側の基準）。
//! - 像面上（[`detector`]）: 2 本の経路の光線の格子を像面（検出面）の画素へ補間し（[`crate::raygrid::rasterize`]）、
//!   画素ごとに場を足して I = |A_s e^{ikL_s} + A_r e^{ikL_r}|² を出す。瞳は一様に照らす
//!   （強度は瞳の正規化座標の面積の割合 / 検出面の面積）。偏光を指定すると Jones の場で重ねる。

use crate::coating::{Coatings, C64};
use crate::raygrid::{self as rg, InputPol, RayGrid};
use crate::system::{FieldSel, System};
use crate::{analysis, invalid, Result};

// ---------------------------------------------------------------------------------------------------------------
// 瞳上

/// 瞳上の干渉縞の設定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PupilOpts {
    /// 格子の一辺（4 以上の偶数、[`pupil_grid`]）
    pub n: usize,
    /// OPD の差にかける数（ダブルパスなら 2）
    pub scale: f64,
    /// 傾きの縞の数（瞳の直径あたり、x・y。OpticStudio の X-Tilt・Y-Tilt）
    pub xtilt: f64,
    pub ytilt: f64,
}

impl Default for PupilOpts {
    fn default() -> Self {
        PupilOpts {
            n: 64,
            scale: 1.0,
            xtilt: 0.0,
            ytilt: 0.0,
        }
    }
}

/// 瞳上の干渉縞
#[derive(Debug, Clone)]
pub struct Interferogram {
    /// 格子の正規化瞳座標（x・y 共通）
    pub p: Vec<f64>,
    /// Δ [波]（[iy][ix]、どちらかの光束が通らない点は NaN）
    pub delta: Vec<Vec<f64>>,
    /// 縞の強度 (1 + cos 2πΔ) / 2（外は NaN）
    pub intensity: Vec<Vec<f64>>,
    /// scale·(W1 − W2) の P-V・RMS [波]（傾きの縞は含めない。OpticStudio の Peak to Valley と同じ）
    pub pv: f64,
    pub rms: f64,
}

/// 光束 1（系 `beam1`）と光束 2（`beam2`、None なら参照球）の瞳上の干渉縞。視野 `field` と波長 `w`（None なら主波長）は
/// どちらの系でも同じ番号を使う。各光束の OPD はその光束の波長での [波]（波長が違えば警告せずそのまま引く）
pub fn interferogram(
    beam1: &System,
    beam2: Option<&System>,
    field: FieldSel,
    w: Option<usize>,
    o: &PupilOpts,
) -> Result<Interferogram> {
    if !o.scale.is_finite() || !o.xtilt.is_finite() || !o.ytilt.is_finite() {
        invalid!("scale・xtilt・ytilt は有限の数");
    }
    if o.n < 4 || !o.n.is_multiple_of(2) {
        invalid!("格子の一辺 {} は 4 以上の偶数", o.n);
    }
    let n = o.n;
    let p = pupil_grid(n);
    let w1 = opd_grid(beam1, field, w, &p)?;
    let w2 = match beam2 {
        Some(s) => Some(opd_grid(s, field, w, &p)?),
        None => None,
    };
    let mut delta = vec![vec![f64::NAN; n]; n];
    let mut inten = vec![vec![f64::NAN; n]; n];
    let mut v = Vec::new();
    for iy in 0..n {
        for ix in 0..n {
            let a = w1[iy][ix];
            let b = match &w2 {
                Some(w2) => w2[iy][ix],
                None => 0.0,
            };
            if !(a.is_finite() && b.is_finite()) {
                continue;
            }
            let d = o.scale * (a - b) + 0.5 * (o.xtilt * p[ix] + o.ytilt * p[iy]);
            delta[iy][ix] = d;
            inten[iy][ix] = 0.5 * (1.0 + (2.0 * std::f64::consts::PI * d).cos());
            // P-V・RMS は傾きを除いた OPD の差（OpticStudio と同じ）
            v.push(o.scale * (a - b));
        }
    }
    if v.is_empty() {
        invalid!("2 つの光束が重なる点がない");
    }
    let (pv, rms) = pv_rms(&v);
    Ok(Interferogram {
        p,
        delta,
        intensity: inten,
        pv,
        rms,
    })
}

/// 瞳上の干渉縞の格子 p_i = (i − n/2 + 1)/(n/2 − 1)（n は 4 以上の偶数）。OpticStudio の Interferogram の格子で、
/// Wavefront Map の格子（[`analysis::wavefront_grid`]、(i − n/2)/(n/2 − 1)）とは 1 点ずれる（OpticStudio 2026 R1 で確認）
pub fn pupil_grid(n: usize) -> Vec<f64> {
    let h = (n / 2) as f64 - 1.0;
    (0..n).map(|i| (i as f64 - h) / h).collect()
}

/// 格子 p × p の OPD [波]（波長 w 自身の主光線・参照球に対して。瞳の外・けられは NaN）。[iy][ix]
pub fn opd_grid(s: &System, field: FieldSel, w: Option<usize>, p: &[f64]) -> Result<Vec<Vec<f64>>> {
    let n = p.len();
    let (mut px, mut py, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for (iy, &y) in p.iter().enumerate() {
        for (ix, &x) in p.iter().enumerate() {
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
                idx.push((iy, ix));
            }
        }
    }
    let r = analysis::opd(s, field, &px, &py, w, false)?;
    let mut g = vec![vec![f64::NAN; n]; n];
    for (q, &(iy, ix)) in idx.iter().enumerate() {
        if r.ok(q) && r.opd[q].is_finite() {
            g[iy][ix] = r.opd[q];
        }
    }
    Ok(g)
}

fn pv_rms(v: &[f64]) -> (f64, f64) {
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let rms = (v.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let pv =
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
    (pv, rms)
}

// ---------------------------------------------------------------------------------------------------------------
// 像面上

/// 像面上の干渉縞の設定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorOpts {
    /// 瞳の光線の格子の一辺（2 以上）
    pub pupil: usize,
    /// 画素の数（一辺）
    pub npx: usize,
    /// 検出面の範囲: 中心 (x, y) と一辺の長さ [mm]（None なら 2 本の経路が届く範囲を覆う正方形）
    pub center: Option<(f64, f64)>,
    pub width: Option<f64>,
    /// 試料・参照の強度にかける数
    pub sample_power: f64,
    pub reference_power: f64,
    /// 参照の経路に足す光路長 [mm]（縞の位相をずらす）
    pub delay: f64,
}

impl Default for DetectorOpts {
    fn default() -> Self {
        DetectorOpts {
            pupil: 65,
            npx: 256,
            center: None,
            width: None,
            sample_power: 1.0,
            reference_power: 1.0,
            delay: 0.0,
        }
    }
}

/// 偏光を扱うときの入射偏光と膜（None ならスカラー）
#[derive(Debug, Clone, Copy)]
pub struct Polarized<'a> {
    pub sample: InputPol,
    pub reference: InputPol,
    pub coat: &'a Coatings,
}

/// 像面上の干渉縞（画素 (i, j) の中心は (x0 + i·dx, y0 + j·dx)、番号 j·n + i）
#[derive(Debug, Clone)]
pub struct DetectorFringe {
    pub n: usize,
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    /// 波長 [µm]
    pub wavelength: f64,
    /// 試料・参照のどちらが届いたか
    pub ok_sample: Vec<bool>,
    pub ok_reference: Vec<bool>,
    /// 試料・参照だけの強度（power をかけた値。届かない画素は 0）
    pub i_sample: Vec<f64>,
    pub i_reference: Vec<f64>,
    /// 重ねた強度
    pub intensity: Vec<f64>,
    /// 光路差 (L_s − L_r − delay) / λ [波]（両方届く画素だけ、ほかは NaN）
    pub opd: Vec<f64>,
    /// 縞の明暗の比（可視度） 2|A_s·A_r*| / (|A_s|² + |A_r|²)（両方届く画素だけ、ほかは NaN）
    pub visibility: Vec<f64>,
    /// 折り返した三角形の割合（試料・参照）
    pub fold: (f64, f64),
}

/// 1 本の経路を瞳 G × G で波長 w の光線の格子にする（場は exp(+i k L) の約束の複素ベクトル、スカラーなら (1, 0, 0)）。
/// 光路長は物体から像面まで（理想レンズがあれば位相込み）
fn trace_grid(
    sys: &System,
    field: FieldSel,
    g: usize,
    w: usize,
    pol: Option<(InputPol, &Coatings)>,
) -> Result<(RayGrid, Vec<[C64; 3]>)> {
    let mut idx = Vec::new();
    for iy in 0..g {
        for ix in 0..g {
            let (px, py) = grid_p(g, ix, iy);
            if px * px + py * py <= 1.0 {
                idx.push(iy * g + ix);
            }
        }
    }
    let pxs: Vec<f64> = idx.iter().map(|&q| grid_p(g, q % g, q / g).0).collect();
    let pys: Vec<f64> = idx.iter().map(|&q| grid_p(g, q % g, q / g).1).collect();
    let mut r = RayGrid::new(g, 1);
    let mut e = vec![[C64::new(0.0, 0.0); 3]; g * g];
    let t = sys.try_trace(field, &pxs, &pys, Some(w), None, false)?;
    let phase = sys.surfaces.iter().any(|s| {
        !s.ignore
            && matches!(
                s.typ,
                crate::surface::SurfType::Paraxial | crate::surface::SurfType::User
            )
    });
    let c_lens = if phase {
        let c = sys.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
        if !c.traced(0) {
            invalid!("主光線が像面に届かない");
        }
        c.rays[0].lens_opd
    } else {
        0.0
    };
    let pt = match pol {
        Some((p, coat)) => {
            Some(sys.try_trace_pol_opts(field, &pxs, &pys, Some(w), &[p.jones()], coat, false)?)
        }
        None => None,
    };
    for (m, &q) in idx.iter().enumerate() {
        if !t.ok(m) {
            continue;
        }
        let ray = &t.rays[m];
        r.l[0][q] = if phase {
            t.opl_os[m] + ray.lens_opd - c_lens
        } else {
            t.opl_os[m]
        };
        r.xs[q] = ray.xyz[0];
        r.ys[q] = ray.xyz[1];
        r.sx[q] = pxs[m];
        r.sy[q] = pys[m];
        e[q] = match &pt {
            Some(pt) => {
                if !pt.trace.ok(m) {
                    continue;
                }
                let v = pt.e[m][0];
                // 伝搬の位相を除いた振幅の共役が exp(+i k L) の約束の場（raygrid::trace_jones と同じ）
                [v[0].conj(), v[1].conj(), v[2].conj()]
            }
            None => [C64::new(1.0, 0.0), C64::new(0.0, 0.0), C64::new(0.0, 0.0)],
        };
        r.ok[q] = true;
    }
    Ok((r, e))
}

fn grid_p(g: usize, ix: usize, iy: usize) -> (f64, f64) {
    (
        -1.0 + 2.0 * ix as f64 / (g - 1) as f64,
        -1.0 + 2.0 * iy as f64 / (g - 1) as f64,
    )
}

/// 場の 3 成分を画素へ（x・y は rasterize の ex・ey、z は ex に入れ替えてもう一度）
fn raster(
    r: &RayGrid,
    e: &[[C64; 3]],
    npx: usize,
    x0: f64,
    y0: f64,
    dx: f64,
) -> (rg::PixelData, Vec<C64>) {
    let mut a = r.clone();
    a.ex = e.iter().map(|v| v[0]).collect();
    a.ey = e.iter().map(|v| v[1]).collect();
    let p = rg::rasterize(&a, npx, x0, y0, dx, f64::INFINITY);
    let mut b = r.clone();
    b.ex = e.iter().map(|v| v[2]).collect();
    b.ey = vec![C64::new(0.0, 0.0); e.len()];
    let pz = rg::rasterize(&b, npx, x0, y0, dx, f64::INFINITY);
    (p, pz.ex)
}

/// 試料（系 `sample`）と参照（`reference`）の 2 本の経路の像面上の干渉縞。視野 `field` と波長 `w`（None なら主波長）
/// はどちらも同じ番号で、波長は同じでなければならない。`pol` を指定すると偏光追跡の場で重ねる
pub fn detector(
    sample: &System,
    reference: &System,
    field: FieldSel,
    w: Option<usize>,
    o: &DetectorOpts,
    pol: Option<Polarized>,
) -> Result<DetectorFringe> {
    if o.pupil < 2 {
        invalid!("瞳の格子は 2 以上");
    }
    if o.npx < 1 {
        invalid!("画素の数は 1 以上");
    }
    if !(o.sample_power >= 0.0 && o.reference_power >= 0.0) || !o.delay.is_finite() {
        invalid!("強度の重みは 0 以上、delay は有限の数");
    }
    let ws = w.unwrap_or(sample.primary);
    let wr = w.unwrap_or(reference.primary);
    if ws >= sample.wavelengths.len() || wr >= reference.wavelengths.len() {
        invalid!("波長の番号が範囲外");
    }
    let lam = sample.wavelengths[ws];
    if (reference.wavelengths[wr] - lam).abs() > 1e-12 * lam {
        invalid!(
            "試料と参照の波長が違う（{} µm・{} µm）",
            lam,
            reference.wavelengths[wr]
        );
    }
    let (rs, es) = trace_grid(sample, field, o.pupil, ws, pol.map(|p| (p.sample, p.coat)))?;
    let (rr, er) = trace_grid(
        reference,
        field,
        o.pupil,
        wr,
        pol.map(|p| (p.reference, p.coat)),
    )?;
    // 範囲
    let (cx, cy, wd) = {
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for r in [&rs, &rr] {
            for q in 0..r.ok.len() {
                if r.ok[q] {
                    x0 = x0.min(r.xs[q]);
                    x1 = x1.max(r.xs[q]);
                    y0 = y0.min(r.ys[q]);
                    y1 = y1.max(r.ys[q]);
                }
            }
        }
        if x0 > x1 {
            invalid!("像面に届く光線がない");
        }
        let c = o.center.unwrap_or((0.5 * (x0 + x1), 0.5 * (y0 + y1)));
        let wd = o.width.unwrap_or_else(|| {
            let h = (x1 - c.0)
                .abs()
                .max((x0 - c.0).abs())
                .max((y1 - c.1).abs())
                .max((y0 - c.1).abs());
            2.0 * h
        });
        (c.0, c.1, wd)
    };
    if !(wd > 0.0) {
        invalid!("検出面の幅 {wd} mm");
    }
    let n = o.npx;
    let dx = if n > 1 { wd / (n - 1) as f64 } else { wd };
    let x0 = cx - 0.5 * dx * (n - 1) as f64;
    let y0 = cy - 0.5 * dx * (n - 1) as f64;
    let (ps, zs) = raster(&rs, &es, n, x0, y0, dx);
    let (pr, zr) = raster(&rr, &er, n, x0, y0, dx);
    let k = 2.0 * std::f64::consts::PI / (lam * 1e-3);
    let n2 = n * n;
    let pi = std::f64::consts::PI;
    let mut out = DetectorFringe {
        n,
        x0,
        y0,
        dx,
        wavelength: lam,
        ok_sample: ps.ok.clone(),
        ok_reference: pr.ok.clone(),
        i_sample: vec![0.0; n2],
        i_reference: vec![0.0; n2],
        intensity: vec![0.0; n2],
        opd: vec![f64::NAN; n2],
        visibility: vec![f64::NAN; n2],
        fold: (ps.fold_fraction, pr.fold_fraction),
    };
    for q in 0..n2 {
        // 振幅 = √(強度) × 場（強度は瞳の面積の割合 / 検出面の面積）
        let amp = |p: &rg::PixelData, z: &[C64], pw: f64| -> [C64; 3] {
            let a = (pw * p.i[q] / pi).sqrt();
            [p.ex[q] * a, p.ey[q] * a, z[q] * a]
        };
        let n2v = |v: &[C64; 3]| v.iter().map(|c| c.norm_sqr()).sum::<f64>();
        let a_s = if ps.ok[q] {
            amp(&ps, &zs, o.sample_power)
        } else {
            [C64::new(0.0, 0.0); 3]
        };
        let a_r = if pr.ok[q] {
            amp(&pr, &zr, o.reference_power)
        } else {
            [C64::new(0.0, 0.0); 3]
        };
        let (is, ir) = (n2v(&a_s), n2v(&a_r));
        out.i_sample[q] = is;
        out.i_reference[q] = ir;
        if ps.ok[q] && pr.ok[q] {
            let dl = ps.l[0][q] - pr.l[0][q] - o.delay;
            let cross: C64 = (0..3).map(|c| a_s[c] * a_r[c].conj()).sum::<C64>();
            let ph = C64::from_polar(1.0, k * dl);
            out.intensity[q] = is + ir + 2.0 * (cross * ph).re;
            out.opd[q] = dl / (lam * 1e-3);
            out.visibility[q] = if is + ir > 0.0 {
                2.0 * cross.norm() / (is + ir)
            } else {
                0.0
            };
        } else {
            out.intensity[q] = is + ir;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 入射瞳径 10・f 50 の理想レンズ、像面は焦点から dz。視野は角度 ys [deg]
    fn lens(dz: f64, ys: &[f64]) -> System {
        let yf: Vec<String> = ys.iter().map(|v| v.to_string()).collect();
        let zf: Vec<&str> = ys.iter().map(|_| "0").collect();
        let one: Vec<&str> = ys.iter().map(|_| "1").collect();
        let txt = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 {} 1 0 0 0\n\
             XFLN {}\nYFLN {}\nFWGN {}\nWAVM 1 0.55 1\nPWAV 1\n\
             SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
             SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 50\n  PARM 2 1\n  DISZ {}\n\
             SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
            ys.len(),
            zf.join(" "),
            yf.join(" "),
            one.join(" "),
            50.0 + dz
        );
        let raw = crate::zmx::parse_text(&txt).unwrap();
        crate::zmx::build(&raw, &Default::default(), &[], None).unwrap()
    }

    #[test]
    fn same_beam_gives_only_the_tilt_fringes() {
        let s = lens(0.0, &[0.0, 1.0]);
        let o = PupilOpts {
            n: 32,
            scale: 1.0,
            xtilt: 3.0,
            ytilt: -1.0,
        };
        let r = interferogram(&s, Some(&s), FieldSel::Index(1), None, &o).unwrap();
        let mut seen = 0;
        for (iy, row) in r.delta.iter().enumerate() {
            for (ix, &d) in row.iter().enumerate() {
                if d.is_finite() {
                    seen += 1;
                    assert!(
                        (d - 0.5 * (3.0 * r.p[ix] - r.p[iy])).abs() < 1e-12,
                        "{iy} {ix} {d}"
                    );
                }
            }
        }
        assert!(seen > 700, "{seen}");
    }

    #[test]
    fn defocus_difference_is_the_difference_of_wavefronts() {
        let a = lens(0.0, &[0.0]);
        let b = lens(0.05, &[0.0]);
        let o = PupilOpts {
            n: 32,
            scale: 2.0,
            ..Default::default()
        };
        let r = interferogram(&a, Some(&b), FieldSel::Index(0), None, &o).unwrap();
        let p = pupil_grid(32);
        let wa = opd_grid(&a, FieldSel::Index(0), None, &p).unwrap();
        let wb = opd_grid(&b, FieldSel::Index(0), None, &p).unwrap();
        for iy in 0..32 {
            for ix in 0..32 {
                let d = r.delta[iy][ix];
                if d.is_finite() {
                    assert!((d - 2.0 * (wa[iy][ix] - wb[iy][ix])).abs() < 1e-12);
                    let i = 0.5 * (1.0 + (2.0 * std::f64::consts::PI * d).cos());
                    assert!((r.intensity[iy][ix] - i).abs() < 1e-12);
                }
            }
        }
        // 焦点ずれの波面 n (1 − cos θ) dz（縁で sin θ = 5/√(50² + 5²)）の 2 倍が P-V
        let sin2: f64 = 25.0 / (2500.0 + 25.0);
        let want = 2.0 * 0.05 * (1.0 - (1.0 - sin2).sqrt()) / 0.55e-3;
        assert!((r.pv - want).abs() < 2e-3 * want, "{} {want}", r.pv);
        // 参照球との干渉は光束 1 の波面だけ（理想レンズの焦点なら平ら）
        let r0 = interferogram(&a, None, FieldSel::Index(0), None, &o).unwrap();
        assert!(r0.pv < 1e-9, "{}", r0.pv);
    }

    #[test]
    fn same_path_on_the_detector_is_fully_coherent_and_delay_turns_the_phase() {
        // 焦点から 2 mm 外した面（瞳の像は半径 0.2 mm の円）
        let s = lens(2.0, &[0.0]);
        let o = DetectorOpts {
            pupil: 33,
            npx: 9,
            center: Some((0.0, 0.0)),
            width: Some(0.2),
            ..Default::default()
        };
        let r = detector(&s, &s, FieldSel::Index(0), None, &o, None).unwrap();
        let mut seen = 0;
        for q in 0..r.n * r.n {
            if r.ok_sample[q] && r.ok_reference[q] {
                seen += 1;
                assert!(r.opd[q].abs() < 1e-12);
                assert!((r.visibility[q] - 1.0).abs() < 1e-12);
                assert!((r.intensity[q] - 4.0 * r.i_sample[q]).abs() < 1e-9 * r.intensity[q]);
            }
        }
        assert!(seen > 20, "{seen}");
        // delay = λ/2 なら暗い
        let o2 = DetectorOpts {
            delay: 0.55e-3 / 2.0,
            ..o
        };
        let r2 = detector(&s, &s, FieldSel::Index(0), None, &o2, None).unwrap();
        for q in 0..r2.n * r2.n {
            if r2.ok_sample[q] && r2.ok_reference[q] {
                assert!(
                    r2.intensity[q].abs() < 1e-9 * r2.i_sample[q],
                    "{}",
                    r2.intensity[q]
                );
                assert!((r2.opd[q] + 0.5).abs() < 1e-9);
            }
        }
        // 一様な瞳: 強度の積分は 1
        let o3 = DetectorOpts {
            pupil: 129,
            npx: 129,
            center: Some((0.0, 0.0)),
            width: Some(0.5),
            ..Default::default()
        };
        let r3 = detector(&s, &s, FieldSel::Index(0), None, &o3, None).unwrap();
        let tot: f64 = r3.i_sample.iter().sum::<f64>() * r3.dx * r3.dx;
        assert!((tot - 1.0).abs() < 0.03, "{tot}");
    }

    /// 物体距離 100・f 50 の理想レンズ（1:1、入射瞳径 10）、像面は像点から dz。視野は物体高 ys [mm]
    fn relay(dz: f64, ys: f64) -> System {
        let txt = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 1 0 1 1 0 0 0\n\
             XFLN 0\nYFLN {ys}\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
             SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 100\n\
             SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 50\n  PARM 2 1\n  DISZ {}\n\
             SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
            100.0 + dz
        );
        let raw = crate::zmx::parse_text(&txt).unwrap();
        crate::zmx::build(&raw, &Default::default(), &[], None).unwrap()
    }

    #[test]
    fn two_point_images_give_the_path_difference_of_two_spherical_waves() {
        // 像点の 2 mm 後ろの面で、像点 y = 0 と y = 0.05 の 2 つの光束（物体高 0・−0.05）を重ねる。理想レンズなので
        // どちらも像点からの球面波: L(y) = 定数 + √(2² + (y − y_i)²)
        let yb = 0.05;
        let s1 = relay(2.0, -yb);
        let s0 = relay(2.0, 0.0);
        let o = DetectorOpts {
            pupil: 65,
            npx: 41,
            center: Some((0.0, 0.5 * yb)),
            width: Some(0.1),
            ..Default::default()
        };
        let b = detector(&s1, &s0, FieldSel::Index(0), None, &o, None).unwrap();
        let n = b.n;
        let i = n / 2;
        let (j0, j1) = (n / 2 - 15, n / 2 + 15);
        let (q0, q1) = (j0 * n + i, j1 * n + i);
        assert!(b.opd[q0].is_finite() && b.opd[q1].is_finite());
        let y = |j: usize| b.y0 + j as f64 * b.dx;
        let sph = |y: f64, yf: f64| (4.0 + (y - yf) * (y - yf)).sqrt();
        let want = (sph(y(j1), yb) - sph(y(j0), yb)) - (sph(y(j1), 0.0) - sph(y(j0), 0.0));
        let got = (b.opd[q1] - b.opd[q0]) * 0.55e-3;
        // 画素への補間（三角形の 1 次）の誤差の分だけ許す
        assert!((got - want).abs() < 1e-3 * want.abs(), "{got} {want}");
        assert!(
            want.abs() / 0.55e-3 > 1.0,
            "縞が 1 本以上 {}",
            want / 0.55e-3
        );
        // 重ねた強度は 2 つの強度と光路差の cos
        for q in [q0, q1] {
            let (a, c) = (b.i_sample[q], b.i_reference[q]);
            let ph = 2.0 * std::f64::consts::PI * b.opd[q];
            let want = a + c + 2.0 * (a * c).sqrt() * ph.cos();
            assert!(
                (b.intensity[q] - want).abs() < 1e-9 * (a + c),
                "{} {want}",
                b.intensity[q]
            );
            assert!((b.visibility[q] - 2.0 * (a * c).sqrt() / (a + c)).abs() < 1e-12);
        }
    }
}
