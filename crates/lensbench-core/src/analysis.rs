//! 実光線による解析: 瞳の標本、スポット、OPD（射出瞳の参照球）、波面マップ、Zernike。
//!
//! OPD は OpticStudio と同じ（ROPD「射出瞳」）: 参照球の中心は主波長の主光線の像点、主光線が射出瞳面
//! （像面から EXPP）と交わる点を通る。各光線を像面から参照球まで戻し、(主光線の光路 − 光線の光路) を
//! その光線の波長の波数で表す（主光線はその波長のもの）。ROPD「無限遠」は主光線に垂直な平面。

use crate::error::Result;
use crate::math::*;
use crate::system::{FieldSel, System, Trace};
use crate::{invalid, notimpl};

/// n×n の格子のうち単位円内の点
pub fn pupil_grid(n: usize) -> (Vec<f64>, Vec<f64>) {
    let g: Vec<f64> = (0..n)
        .map(|i| -1.0 + 2.0 * i as f64 / (n - 1) as f64)
        .collect();
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for &y in &g {
        for &x in &g {
            if x * x + y * y <= 1.0 + 1e-12 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// Gauss-Legendre の節点と重み（[-1, 1]）
pub fn gauss_legendre(n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut x = vec![0.0; n];
    let mut w = vec![0.0; n];
    for i in 0..n {
        let mut z = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut dp = 0.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, z);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * z * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            let pn = if n == 1 { z } else { p1 };
            let pm = if n == 1 { 1.0 } else { p0 };
            dp = n as f64 * (z * pn - pm) / (z * z - 1.0);
            let dz = pn / dp;
            z -= dz;
            if dz.abs() < 1e-16 {
                break;
            }
        }
        x[n - 1 - i] = z;
        w[n - 1 - i] = 2.0 / ((1.0 - z * z) * dp * dp);
    }
    (x, w)
}

/// 瞳のガウス求積: r^2 に Gauss-Legendre（rings）× 等間隔の角度（arms、既定 4 rings + 2）→ px, py, 重み（和 1）
pub fn pupil_gauss(rings: usize, arms: Option<usize>) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let arms = arms.unwrap_or(4 * rings + 2);
    let (u, wu) = gauss_legendre(rings);
    let (mut px, mut py, mut wt) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..rings {
        let r = ((u[i] + 1.0) / 2.0).sqrt();
        for a in 0..arms {
            let ang = (a as f64 + 0.5) * 2.0 * std::f64::consts::PI / arms as f64;
            px.push(r * ang.cos());
            py.push(r * ang.sin());
            wt.push(wu[i] / 2.0 / arms as f64);
        }
    }
    (px, py, wt)
}

/// OPD [波] の結果（光線と一緒に）
pub struct Opd {
    pub trace: Trace,
    pub opd: Vec<f64>,
}

impl Opd {
    pub fn ok(&self, i: usize) -> bool {
        self.trace.ok(i)
    }
}

fn to_ref(s: &System, w: usize, pc: V3, kc: V3, q: V3, k: V3, opl: f64) -> f64 {
    let ni = s.image_index(w);
    let expp = s.exit_pupil();
    if s.ropd == 1 || !expp.is_finite() || expp.abs() > 1e9 {
        return opl - ni * dot(sub(q, pc), kc) / dot(k, kc);
    }
    let sc = (expp - pc[2]) / kc[2];
    let rr = sc.abs();
    let dd = sub(q, pc);
    let b = dot(dd, k);
    let c = dot(dd, dd) - rr * rr;
    let disc = (b * b - c).max(0.0).sqrt();
    let t = if sc < 0.0 { -b - disc } else { -b + disc };
    opl + ni * t
}

/// 視野の光線 (px, py) の OPD [波]。ref_primary: 参照球の中心を主波長の主光線に（OpticStudio の光線の OPD）、
/// false なら波長 w 自身の主光線に（Zernike、波面マップ）
pub fn opd(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: Option<usize>,
    ref_primary: bool,
) -> Result<Opd> {
    if s.ropd != 1 && s.ropd != 2 {
        notimpl!("OPD の基準 {}（射出瞳・無限遠のみ）", s.ropd);
    }
    let w = w.unwrap_or(s.primary);
    let r = s.try_trace(field, px, py, Some(w), None, false)?;
    let rc = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
    let rp = if w == s.primary || !ref_primary {
        rc.clone()
    } else {
        s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?
    };
    let (pc, kc) = (rp.rays[0].xyz, rp.rays[0].lmn);
    let c = &rc.rays[0];
    let lc = to_ref(s, w, pc, kc, c.xyz, c.lmn, c.opl) + c.lens_opd;
    let wl = s.wavelengths[w] * 1e-3;
    let opd = r
        .rays
        .iter()
        .map(|x| (lc - (to_ref(s, w, pc, kc, x.xyz, x.lmn, x.opl) + x.lens_opd)) / wl)
        .collect();
    Ok(Opd { trace: r, opd })
}

#[derive(Debug, Clone)]
pub struct Spot {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub wave: Vec<usize>,
    pub weight: Vec<f64>,
    pub centroid: (f64, f64),
    pub chief: (f64, f64),
    pub rms_centroid: f64,
    pub rms_chief: f64,
    pub geo: f64,
}

/// スポット（w None なら全波長を重み付きで）。RMS は OpticStudio の RSCE・RSCH と同じ（ガウス求積、けられた光線は除く）
pub fn spot(s: &System, field: FieldSel, w: Option<usize>, rings: usize) -> Result<Spot> {
    let (px, py, wts) = pupil_gauss(rings, None);
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let (mut x, mut y, mut wt, mut wv) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for &wi in &ws {
        let r = s.try_trace(field, &px, &py, Some(wi), None, false)?;
        for i in 0..px.len() {
            if r.ok(i) {
                x.push(r.rays[i].xyz[0]);
                y.push(r.rays[i].xyz[1]);
                wt.push(wts[i] * s.weights[wi]);
                wv.push(wi);
            }
        }
    }
    let rc = s.try_trace(
        field,
        &[0.0],
        &[0.0],
        Some(w.unwrap_or(s.primary)),
        None,
        false,
    )?;
    let chief = if rc.ok(0) {
        (rc.rays[0].xyz[0], rc.rays[0].xyz[1])
    } else {
        (f64::NAN, f64::NAN)
    };
    let sw: f64 = wt.iter().sum();
    if x.is_empty() || !(sw > 0.0) {
        return Ok(Spot {
            x,
            y,
            wave: wv,
            weight: wt,
            centroid: (f64::NAN, f64::NAN),
            chief,
            rms_centroid: f64::NAN,
            rms_chief: f64::NAN,
            geo: f64::NAN,
        });
    }
    let avg = |v: &dyn Fn(usize) -> f64| (0..x.len()).map(|i| v(i) * wt[i]).sum::<f64>() / sw;
    let cx = avg(&|i| x[i]);
    let cy = avg(&|i| y[i]);
    let (hx, hy) = (rc.rays[0].xyz[0], rc.rays[0].xyz[1]);
    let r2c = |i: usize| (x[i] - cx).powi(2) + (y[i] - cy).powi(2);
    let rms_c = avg(&r2c).sqrt();
    let rms_h = avg(&|i| (x[i] - hx).powi(2) + (y[i] - hy).powi(2)).sqrt();
    let geo = (0..x.len()).map(r2c).fold(0.0, f64::max).sqrt();
    Ok(Spot {
        centroid: (cx, cy),
        chief,
        rms_centroid: rms_c,
        rms_chief: rms_h,
        geo,
        x,
        y,
        wave: wv,
        weight: wt,
    })
}

/// 格子の一辺 n は 4 以上の偶数（中心の点 p = 0 を含み、端が瞳の縁になる）
fn check_grid(n: usize) -> Result<()> {
    if n < 4 || n % 2 != 0 {
        invalid!("格子の一辺 {n} は 4 以上の偶数にする");
    }
    Ok(())
}

/// 波面マップの格子 p_i = (i − n/2)/(n/2 − 1)（OpticStudio の Wavefront Map と同じ）
pub fn wavefront_grid(n: usize) -> Vec<f64> {
    let h = (n / 2) as f64;
    (0..n).map(|i| (i as f64 - h) / (h - 1.0)).collect()
}

pub struct Wavefront {
    pub p: Vec<f64>,
    /// W[iy][ix]（瞳の外・けられは NaN）
    pub w: Vec<Vec<f64>>,
    pub pv: f64,
    pub rms: f64,
}

pub fn wavefront(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    n: usize,
    ref_primary: bool,
) -> Result<Wavefront> {
    check_grid(n)?;
    let g = wavefront_grid(n);
    let (mut px, mut py, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for (iy, &y) in g.iter().enumerate() {
        for (ix, &x) in g.iter().enumerate() {
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
                idx.push((iy, ix));
            }
        }
    }
    let r = opd(s, field, &px, &py, w, ref_primary)?;
    let mut ww = vec![vec![f64::NAN; n]; n];
    let mut v = Vec::new();
    for (q, &(iy, ix)) in idx.iter().enumerate() {
        if r.ok(q) && r.opd[q].is_finite() {
            ww[iy][ix] = r.opd[q];
            v.push(r.opd[q]);
        }
    }
    if v.is_empty() {
        invalid!("有効な光線がない（すべてけられた）");
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let rms = (v.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let pv =
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
    Ok(Wavefront {
        p: g,
        w: ww,
        pv,
        rms,
    })
}

// ------------------------------------------------------------------------------------------ Zernike
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZernKind {
    Fringe,
    Standard,
}

/// 項 1..nterms の (n, m, cos なら true)
pub fn zern_nm(kind: ZernKind, nterms: usize) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    match kind {
        ZernKind::Fringe => {
            assert!(nterms <= 37, "Fringe Zernike は 37 項まで");
            for g in 0..6usize {
                for m in (0..=g).rev() {
                    let n = 2 * g - m;
                    if m > 0 {
                        out.push((n, m, true));
                        out.push((n, m, false));
                    } else {
                        out.push((n, 0, true));
                    }
                }
            }
            out.push((12, 0, true));
        }
        ZernKind::Standard => {
            let (mut j, mut n) = (1usize, 0usize);
            while out.len() < nterms {
                let mut m = n % 2;
                while m <= n {
                    if m == 0 {
                        out.push((n, 0, true));
                        j += 1;
                    } else {
                        for _ in 0..2 {
                            out.push((n, m, j % 2 == 0));
                            j += 1;
                        }
                    }
                    m += 2;
                }
                n += 1;
            }
        }
    }
    out.truncate(nterms);
    out
}

fn fact(n: usize) -> f64 {
    (1..=n).map(|v| v as f64).product()
}

fn radial(n: usize, m: usize, r: f64) -> f64 {
    (0..=(n - m) / 2)
        .map(|k| {
            let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
            sgn * fact(n - k) / (fact(k) * fact((n + m) / 2 - k) * fact((n - m) / 2 - k))
                * r.powi((n - 2 * k) as i32)
        })
        .sum()
}

/// 瞳の点での Zernike 項（Standard は OpticStudio の正規化、Fringe は正規化なし）
pub fn zernike_row(px: f64, py: f64, kind: ZernKind, nm: &[(usize, usize, bool)]) -> Vec<f64> {
    let r = px.hypot(py);
    let a = py.atan2(px);
    nm.iter()
        .map(|&(n, m, c)| {
            let z = radial(n, m, r)
                * if c {
                    (m as f64 * a).cos()
                } else {
                    (m as f64 * a).sin()
                };
            if kind == ZernKind::Standard {
                z * (((n + 1) * if m > 0 { 2 } else { 1 }) as f64).sqrt()
            } else {
                z
            }
        })
        .collect()
}

/// 最小二乗（Householder QR）: A x ≈ b
pub fn lstsq(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let m = a.len();
    if m == 0 {
        return Vec::new();
    }
    let n = a[0].len();
    let mut q: Vec<Vec<f64>> = (0..n).map(|j| (0..m).map(|i| a[i][j]).collect()).collect(); // 列優先
    let mut y = b.to_vec();
    let mut rdiag = vec![0.0; n];
    for k in 0..n {
        let nrm = q[k][k..].iter().map(|v| v * v).sum::<f64>().sqrt();
        if nrm == 0.0 {
            rdiag[k] = 0.0;
            continue;
        }
        let alpha = if q[k][k] > 0.0 { -nrm } else { nrm };
        let mut v: Vec<f64> = q[k][k..].to_vec();
        v[0] -= alpha;
        let vn = v.iter().map(|x| x * x).sum::<f64>();
        if vn == 0.0 {
            rdiag[k] = alpha;
            continue;
        }
        for col in q.iter_mut().skip(k) {
            let s: f64 = col[k..].iter().zip(&v).map(|(a, b)| a * b).sum::<f64>() * 2.0 / vn;
            for (c, vi) in col[k..].iter_mut().zip(&v) {
                *c -= s * vi;
            }
        }
        let s: f64 = y[k..].iter().zip(&v).map(|(a, b)| a * b).sum::<f64>() * 2.0 / vn;
        for (c, vi) in y[k..].iter_mut().zip(&v) {
            *c -= s * vi;
        }
        rdiag[k] = q[k][k];
    }
    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let s: f64 = (k + 1..n).map(|j| q[j][k] * x[j]).sum();
        x[k] = if rdiag[k] != 0.0 {
            (y[k] - s) / rdiag[k]
        } else {
            0.0
        };
    }
    x
}

#[derive(Debug, Clone)]
pub struct Zernike {
    pub coef: Vec<f64>,
    pub kind: ZernKind,
    pub pv: f64,
    pub pv_centroid: f64,
    pub rms_fit: f64,
    pub max_fit: f64,
    pub rms_chief: f64,
    pub rms_centroid: f64,
    pub variance: f64,
    pub strehl: f64,
}

/// OPD の Zernike 当てはめ [波]（波面マップと同じ n×n の格子で最小二乗。OpticStudio の Zernike Coefficients）
pub fn zernike(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    kind: ZernKind,
    nterms: usize,
    n: usize,
    ref_primary: bool,
) -> Result<Zernike> {
    check_grid(n)?;
    if nterms == 0 || (kind == ZernKind::Fringe && nterms > 37) {
        invalid!("項数 {nterms}（Fringe は 1〜37）");
    }
    let g = wavefront_grid(n);
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for &y in &g {
        for &x in &g {
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    let r = opd(s, field, &px, &py, w, ref_primary)?;
    let nm = zern_nm(kind, nterms);
    let (mut a, mut v, mut b3) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..px.len() {
        if r.ok(i) && r.opd[i].is_finite() {
            a.push(zernike_row(px[i], py[i], kind, &nm));
            v.push(r.opd[i]);
            b3.push(vec![1.0, px[i], py[i]]);
        }
    }
    if v.len() < nm.len().max(3) {
        invalid!("有効な光線 {} 本が項数 {} より少ない", v.len(), nm.len());
    }
    let c = lstsq(&a, &v);
    let res: Vec<f64> = a
        .iter()
        .zip(&v)
        .map(|(row, vi)| vi - row.iter().zip(&c).map(|(p, q)| p * q).sum::<f64>())
        .collect();
    let t = lstsq(&b3, &v);
    let vt: Vec<f64> = b3
        .iter()
        .zip(&v)
        .map(|(row, vi)| vi - row.iter().zip(&t).map(|(p, q)| p * q).sum::<f64>())
        .collect();
    let nn = v.len() as f64;
    let ptp = |x: &[f64]| {
        x.iter().cloned().fold(f64::MIN, f64::max) - x.iter().cloned().fold(f64::MAX, f64::min)
    };
    let mean = v.iter().sum::<f64>() / nn;
    let rms_c = (vt.iter().map(|x| x * x).sum::<f64>() / nn).sqrt();
    Ok(Zernike {
        coef: c,
        kind,
        pv: ptp(&v),
        pv_centroid: ptp(&vt),
        rms_fit: (res.iter().map(|x| x * x).sum::<f64>() / nn).sqrt(),
        max_fit: res.iter().map(|x| x.abs()).fold(0.0, f64::max),
        rms_chief: (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nn).sqrt(),
        rms_centroid: rms_c,
        variance: rms_c * rms_c,
        strehl: (-(2.0 * std::f64::consts::PI * rms_c).powi(2)).exp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gauss_legendre_integrates() {
        let (x, w) = gauss_legendre(5);
        let i: f64 = x.iter().zip(&w).map(|(x, w)| x.powi(8) * w).sum();
        assert!((i - 2.0 / 9.0).abs() < 1e-14);
    }
    #[test]
    fn lstsq_exact() {
        let a = vec![vec![1.0, 0.0], vec![1.0, 1.0], vec![1.0, 2.0]];
        let x = lstsq(&a, &[1.0, 3.0, 5.0]);
        assert!((x[0] - 1.0).abs() < 1e-14 && (x[1] - 2.0).abs() < 1e-14);
    }
}
