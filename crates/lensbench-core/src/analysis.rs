//! 実光線による解析: 瞳の標本、スポット、OPD（射出瞳の参照球）、波面マップ、Zernike。
//!
//! OPD は OpticStudio と同じ（ROPD「射出瞳」）: 参照球の中心は主波長の主光線の像点、主光線が射出瞳面
//! （像面から EXPP）と交わる点を通る。各光線を像面から参照球まで戻し、(主光線の光路 − 光線の光路) を
//! その光線の波長の波数で表す（主光線はその波長のもの）。ROPD「無限遠」は主光線に垂直な平面。

use crate::coating::C64;
use crate::error::Result;
use crate::math::*;
use crate::system::{FieldSel, System, Trace};
use crate::Waves;
use crate::{invalid, notimpl, par::scoped};

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

/// 瞳のガウス求積の点（px, py, 重み）
pub(crate) type PupilPts = (Vec<f64>, Vec<f64>, Vec<f64>);

/// [`pupil_gauss`]（arms は既定）を、スレッドごとに覚えて使い回す。
/// 評価関数（最適化・差し替え探索）は項ごと・反復ごとに同じ点を何度も使うため。値は [`pupil_gauss`] と同じ
pub(crate) fn pupil_gauss_shared(rings: usize) -> std::rc::Rc<PupilPts> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    thread_local! {
        static CACHE: RefCell<HashMap<usize, Rc<PupilPts>>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(rings)
            .or_insert_with(|| Rc::new(pupil_gauss(rings, None)))
            .clone()
    })
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

pub(crate) fn to_ref(s: &System, w: usize, pc: V3, kc: V3, q: V3, k: V3, opl: f64) -> f64 {
    let ni = s.image_index(w);
    let expp = s.exit_pupil();
    if s.afocal {
        // アフォーカル像空間: 射出瞳の位置で主光線に垂直な平面まで延ばす（OpticStudio の平面波基準）。
        // 射出瞳が無限遠なら像面の主光線の点で（平面波なので面の位置は定数の違いだけ）
        let pe = if expp.is_finite() && expp.abs() <= 1e9 {
            add(pc, scale(kc, (expp - pc[2]) / kc[2]))
        } else {
            pc
        };
        return opl + ni * dot(sub(pe, q), kc) / dot(k, kc);
    }
    if s.ropd == 1 || !expp.is_finite() || expp.abs() > 1e9 {
        return opl - ni * dot(sub(q, pc), kc) / dot(k, kc);
    }
    let sc = (expp - pc[2]) / kc[2];
    let rr = sc.abs();
    let dd = sub(q, pc);
    let b = dot(dd, k);
    let c = dot(dd, dd) - rr * rr;
    // 基準球に届かない光線は NaN（接点に丸めない）
    let disc = (b * b - c).sqrt();
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
    if !s.afocal && s.ropd != 1 && s.ropd != 2 {
        notimpl!("OPD の基準 {}（射出瞳・無限遠のみ）", s.ropd);
    }
    if !s.afocal && s.ropd != 1 && s.exit_pupil().is_nan() {
        invalid!(
            "射出瞳が求められない（瞳の算出の補助の光線が解けない）。OPD の基準が決められない"
        );
    }
    let w = w.unwrap_or(s.primary);
    let r = s.try_trace(field, px, py, Some(w), None, false)?;
    let rc = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
    let rp = if w == s.primary || !ref_primary {
        rc.clone()
    } else {
        s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?
    };
    for (t, wi) in [(&rc, w), (&rp, s.primary)] {
        let c = &t.rays[0];
        if c.err != 0 {
            let why = if c.tir {
                "で全反射"
            } else {
                "に当たらない"
            };
            invalid!(
                "主光線（波長 {}）が像面に届かない（面 {}{why}）。OPD の基準が決められない",
                wi + 1,
                c.err
            );
        }
    }
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

/// スポット（w None なら全波長を重み付きで）。RMS は OpticStudio の RSCE・RSCH と同じ（ガウス求積、けられた光線は除く）。
/// 腕の数は 4 rings + 2（[`spot_arms`] で指定できる）
pub fn spot(s: &System, field: FieldSel, w: Waves, rings: usize) -> Result<Spot> {
    let w = w.one();
    spot_arms(s, field, Waves::all_or(w), rings, None)
}

/// [`spot`] で、求積の腕（角度の分割）の数 arms を指定する（None なら 4 rings + 2）。OpticStudio の RSCE・RSCH は
/// 腕が 2 rings で、回転対称でない系（Zernike・Grid Sag など）では角度の求積が収束しきらず、腕を増やした値と
/// 相対 1e-5〜1e-4 違う（Some(2 * rings) で OpticStudio と丸め誤差まで合う）
pub fn spot_arms(
    s: &System,
    field: FieldSel,
    w: Waves,
    rings: usize,
    arms: Option<usize>,
) -> Result<Spot> {
    let w = w.one();
    let (px, py, wts) = pupil_gauss(rings, arms);
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let (mut x, mut y, mut wt, mut wv) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for &wi in &ws {
        let r = s.try_trace(field, &px, &py, Some(wi), None, false)?;
        for i in 0..px.len() {
            if r.ok(i) {
                let (a, b) = s.image_point(r.rays[i].xyz, r.rays[i].lmn);
                x.push(a);
                y.push(b);
                // 単色は重みによらない（重み 0 の波長でも求める）
                wt.push(wts[i] * if ws.len() == 1 { 1.0 } else { s.weights[wi] });
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
    let chief = if rc.traced(0) {
        s.image_point(rc.rays[0].xyz, rc.rays[0].lmn)
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
    let (hx, hy) = chief;
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
    if n < 4 || !n.is_multiple_of(2) {
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

// ------------------------------------------------------------------------------------------ MTF
/// MTF（変調度）。freqs [cycles/mm] ごとに子午（像面の y 方向の周波数）と球欠（x 方向）
#[derive(Debug, Clone)]
pub struct Mtf {
    pub freqs: Vec<f64>,
    pub tan: Vec<f64>,
    pub sag: Vec<f64>,
    /// 回折限界の遮断周波数 [cycles/mm]（主波長、(x, y) 方向）
    pub cutoff: (f64, f64),
}

/// 単位円を ±d/2 ずらした 2 つの円の重なり（レンズ形）の求積: (ずらす向きの座標 t, 直交する座標 u, 重み)。
/// t は両側の先端で幅が sqrt で 0 になるので、t = a (1 − τ²)（a = 1 − d/2）と置いて τ で Gauss-Legendre、
/// u は各 t で幅いっぱいに Gauss-Legendre。重みの和はレンズの面積（d = 0 で π）
/// [`lens_quad`] の Gauss-Legendre の節点と重み（n ごとに 1 回だけ作る）
pub(crate) struct LensGl {
    gt: Vec<f64>,
    wt: Vec<f64>,
    gu: Vec<f64>,
    wu: Vec<f64>,
}

impl LensGl {
    pub(crate) fn new(n: usize) -> Self {
        let (gt, wt) = gauss_legendre(n.div_ceil(2).max(2));
        let (gu, wu) = gauss_legendre(n.max(2));
        LensGl { gt, wt, gu, wu }
    }
}

pub(crate) fn lens_quad(d: f64, gl: &LensGl) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let a = 1.0 - 0.5 * d;
    let (mut t, mut u, mut w) = (Vec::new(), Vec::new(), Vec::new());
    if a <= 0.0 {
        return (t, u, w);
    }
    let (gt, wt, gu, wu) = (&gl.gt, &gl.wt, &gl.gu, &gl.wu);
    let m = gt.len() * 2 * gu.len();
    t.reserve(m);
    u.reserve(m);
    w.reserve(m);
    for (&z, &wz) in gt.iter().zip(wt) {
        // τ ∈ [0, 1]
        let tau = 0.5 * (z + 1.0);
        let y = a * (1.0 - tau * tau);
        let dy = 2.0 * a * tau * 0.5 * wz;
        let x = (1.0 - (y + 0.5 * d).powi(2)).max(0.0).sqrt();
        for sgn in [1.0, -1.0] {
            for (&g, &wg) in gu.iter().zip(wu) {
                t.push(sgn * y);
                u.push(x * g);
                w.push(dy * x * wg);
            }
        }
    }
    (t, u, w)
}

/// [`lens_quad`] の d = 0（単位円）の重みの和（点を並べずに求める。単位円の面積 π の近似）
pub(crate) fn lens_area(gl: &LensGl) -> f64 {
    let su: f64 = gl.wu.iter().sum();
    let mut acc = 0.0;
    for (&z, &wz) in gl.gt.iter().zip(&gl.wt) {
        let tau = 0.5 * (z + 1.0);
        let y = 1.0 - tau * tau;
        let dy = tau * wz;
        let x = (1.0 - y * y).max(0.0).sqrt();
        acc += 2.0 * dy * x * su;
    }
    acc
}

/// 像空間の方向余弦 (L, M) の、正規化瞳 (px, py) に対する傾き J = ∂(L, M)/∂(px, py)（2×2、行が L・M）。
/// 瞳の縁の光線の差で見積もる。これらは基準の光線なので開口・遮蔽は無視する（OpticStudio と同じ）。
/// 縁の光線が像面まで届かない（面を外れるなど）視野では、軸ごとに内側の点（±0.75, ±0.5, ±0.25）の
/// 差で、それも届かないなら主光線との片側の差で見積もる
pub(crate) fn pupil_jacobian(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    if s.afocal {
        return afocal_jacobian(s, field, w);
    }
    let r = s.try_trace(
        field,
        &[1.0, -1.0, 0.0, 0.0],
        &[0.0, 0.0, 1.0, -1.0],
        Some(w),
        None,
        false,
    )?;
    let jm = if (0..4).all(|i| r.traced(i)) {
        let d = |i: usize, j: usize, c: usize| 0.5 * (r.rays[i].lmn[c] - r.rays[j].lmn[c]);
        [[d(0, 1, 0), d(2, 3, 0)], [d(0, 1, 1), d(2, 3, 1)]]
    } else {
        pupil_jacobian_inner(s, field, w)?
    };
    if !(jm[0][0].abs() > 1e-12 && jm[1][1].abs() > 1e-12) {
        notimpl!("MTF: 像空間の開口が 0（アフォーカル？）");
    }
    Ok(jm)
}

/// アフォーカル像空間の [`pupil_jacobian`]: 像の座標は角度 θ [単位]、ビームの幅 X [mm] とすると
/// 位相は 2π X θ u / λ（u = 1 単位のラジアン）なので、−∂X/∂p · u を ∂L/∂p の代わりにする。X は OPD の基準と同じ
/// 射出瞳の位置の主光線に垂直な平面での位置。符号は像面の前に焦点距離 F の理想レンズを置いた
/// 系（L′ = L − X/F、x = F θ）と同じ向き。けられて縁の光線が通らないときは内側の点で差をとる
fn afocal_jacobian(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    const A: [f64; 4] = [1.0, 0.75, 0.5, 0.25];
    let (mut px, mut py) = (vec![0.0], vec![0.0]);
    for axis in 0..2 {
        for a in A {
            for v in [a, -a] {
                let (x, y) = if axis == 0 { (v, 0.0) } else { (0.0, v) };
                px.push(x);
                py.push(y);
            }
        }
    }
    let r = s.try_trace(field, &px, &py, Some(w), None, false)?;
    if !r.traced(0) {
        notimpl!("MTF: 主光線が像面に届かない（瞳の大きさが決められない）");
    }
    let (q0, k0) = (r.rays[0].xyz, r.rays[0].lmn);
    // 射出瞳の位置で主光線に垂直な平面（OPD の基準の平面、[`to_ref`]）まで光線を延ばした点の、主光線からの差。
    // 射出瞳が無限遠なら像面の主光線の点を通る平面
    let expp = s.exit_pupil();
    let pe = if expp.is_finite() && expp.abs() <= 1e9 {
        add(q0, scale(k0, (expp - q0[2]) / k0[2]))
    } else {
        q0
    };
    // 平面の向き: e2 = ŷ から主光線の成分を除いたもの、e1 = e2 × k0（主光線が y・z 面で傾くとき、e2 の向きの
    // 1 rad が atan(m/n) の 1 rad）
    let e2 = {
        let v = sub([0.0, 1.0, 0.0], scale(k0, k0[1]));
        scale(v, 1.0 / dot(v, v).sqrt())
    };
    let e1 = [
        e2[1] * k0[2] - e2[2] * k0[1],
        e2[2] * k0[0] - e2[0] * k0[2],
        e2[0] * k0[1] - e2[1] * k0[0],
    ];
    let perp = |i: usize| {
        let (q, k) = (r.rays[i].xyz, r.rays[i].lmn);
        let x = add(q, scale(k, dot(sub(pe, q), k0) / dot(k, k0)));
        let d = sub(x, pe);
        [dot(d, e1), dot(d, e2)]
    };
    let u = s.afocal_unit.radians();
    let mut col = [[0.0; 2]; 2];
    for (axis, c) in col.iter_mut().enumerate() {
        let k = |ia: usize, neg: bool| 1 + axis * 2 * A.len() + 2 * ia + neg as usize;
        let Some(ia) = (0..A.len()).find(|&ia| r.traced(k(ia, false)) && r.traced(k(ia, true)))
        else {
            notimpl!("MTF: 瞳の軸の上の光線がけられる（瞳の大きさが決められない）");
        };
        let (p, m) = (perp(k(ia, false)), perp(k(ia, true)));
        let h = 2.0 * A[ia];
        *c = [-(p[0] - m[0]) / h * u, -(p[1] - m[1]) / h * u];
    }
    let jm = [[col[0][0], col[1][0]], [col[0][1], col[1][1]]];
    if !(jm[0][0].abs() > 1e-12 && jm[1][1].abs() > 1e-12) {
        notimpl!("MTF: アフォーカル像空間のビームの幅が 0");
    }
    Ok(jm)
}

/// [`pupil_jacobian`] のフォールバック: 瞳の縁が届かないとき、軸ごとに届く内側の点で差をとる
fn pupil_jacobian_inner(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    const A: [f64; 3] = [0.75, 0.5, 0.25];
    // 主光線、x の軸の ±a、y の軸の ±a の順
    let (mut px, mut py) = (vec![0.0], vec![0.0]);
    for axis in 0..2 {
        for a in A {
            for v in [a, -a] {
                let (x, y) = if axis == 0 { (v, 0.0) } else { (0.0, v) };
                px.push(x);
                py.push(y);
            }
        }
    }
    let r = s.try_trace(field, &px, &py, Some(w), None, false)?;
    let lm = |i: usize| [r.rays[i].lmn[0], r.rays[i].lmn[1]];
    let mut col = [[0.0; 2]; 2];
    for (axis, c) in col.iter_mut().enumerate() {
        let k = |ia: usize, neg: bool| 1 + axis * 2 * A.len() + 2 * ia + neg as usize;
        let sym = (0..A.len()).find(|&ia| r.traced(k(ia, false)) && r.traced(k(ia, true)));
        let d = if let Some(ia) = sym {
            let (p, m) = (lm(k(ia, false)), lm(k(ia, true)));
            let h = 2.0 * A[ia];
            [(p[0] - m[0]) / h, (p[1] - m[1]) / h]
        } else {
            if !r.traced(0) {
                notimpl!("MTF: 主光線が像面に届かない（瞳の大きさが決められない）");
            }
            let o = lm(0);
            let one = (0..A.len())
                .flat_map(|ia| [(ia, false), (ia, true)])
                .find(|&(ia, neg)| r.traced(k(ia, neg)));
            let Some((ia, neg)) = one else {
                notimpl!("MTF: 瞳の内側の光線もけられる（瞳の大きさが決められない）");
            };
            let p = lm(k(ia, neg));
            let h = if neg { -A[ia] } else { A[ia] };
            [(p[0] - o[0]) / h, (p[1] - o[1]) / h]
        };
        *c = d;
    }
    // col[axis] = ∂(L, M)/∂p_axis → 行が L・M
    Ok([[col[0][0], col[1][0]], [col[0][1], col[1][1]]])
}

/// 周波数 1 cycle/mm あたりの正規化瞳でのずれ (δx, δy)。OpticStudio の FFT MTF と同じく瞳の座標で計算する
/// （計算面は主光線に垂直、像面の向きによらない）: 球欠（dir 0）は px の向きに λ / (∂L/∂px)、子午（dir 1）は
/// py の向きに λ / (∂M/∂py) ずらし、J の非対角（斜めの視野での瞳の歪み）は使わない
pub(crate) fn pupil_shift(j: &[[f64; 2]; 2], lam: f64, dir: usize) -> (f64, f64) {
    if dir == 0 {
        (lam / j[0][0], 0.0)
    } else {
        (0.0, lam / j[1][1])
    }
}

/// 瞳の OPD の傾きの最大 [波 / 正規化瞳の半径]。33×33 の格子の隣り合う点の差で見積もる（けられた点は飛ばす）
pub fn opd_slope_max(s: &System, field: FieldSel, w: usize) -> Result<f64> {
    let m = 33usize;
    let h = 2.0 / (m - 1) as f64;
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for i in 0..m {
        for j in 0..m {
            px.push(-1.0 + h * j as f64);
            py.push(-1.0 + h * i as f64);
        }
    }
    let o = opd(s, field, &px, &py, Some(w), true)?;
    let ok = |k: usize| px[k].hypot(py[k]) <= 1.0 + 1e-12 && o.ok(k) && o.opd[k].is_finite();
    let mut g: f64 = 0.0;
    for i in 0..m {
        for j in 0..m {
            let k = i * m + j;
            if !ok(k) {
                continue;
            }
            for k2 in [k + 1, k + m] {
                if (k2 == k + 1 && j + 1 < m || k2 == k + m && i + 1 < m) && ok(k2) {
                    g = g.max((o.opd[k2] - o.opd[k]).abs() / h);
                }
            }
        }
    }
    Ok(g)
}

/// 求積の点の数の目安を OPD の傾きの最大 g [波 / 半径] から決める。ずらした 2 点の位相差は半径あたり最大 2g の
/// 速さで回るので、Gauss-Legendre が収束するよう n ≈ 3g（D4 の視野 3、g = 38 で n = 96 が 1e-10 に収束）。32〜4096
/// （上限が 1024 だと E0・E1 の視野 3（g ≈ 600〜700）で足りず、高い周波数で MTF が 0.01 ほど浮いた）
pub fn mtf_points(g: f64) -> usize {
    ((3.0 * g).ceil() as usize + 16).clamp(32, 4096)
}

/// 補間の格子の一辺の区間数の初めの上限。OPD がなめらかな系（E0・E1 で n が 2000 を超える）は 256 で足りる。
/// 足りるかは [`mtf_interp`] で確かめ、足りなければ求積の点数 n まで細かくする
const INTERP_GRID_START: usize = 256;

/// 補間の誤差の許し [波]（格子の区間の中点で、直接追跡した OPD との差の最大）。追跡の丸めで OPD が数百波の
/// 系は 1e-9 程度の差が残る（E1: 格子 256〜2763 で 6〜8e-10）。高次の Zernike 面の系（Z2: 256 で 3.6e-9）は
/// n まで細かくなり、従来と同じ値になる
const INTERP_TOL: f64 = 2e-9;

/// 補間の格子 n でも残る差のうち、その区間を直接追跡に回す差 [波]（収束の許し [`INTERP_TOL`] とは別）。
/// 格子の粗い系（n ≤ 256 で格子が初めから n の A0 など）の 1e-6 波以下の補間の誤差は拾わず、従来の値と速さの
/// まま。拾うのは OPD の段差（Z3: 1 波以上）と、格子 n では粗すぎる系（B1・B2・G0〜G3・Z0・Z1 の一部の区間）
const INTERP_MASK_TOL: f64 = 1e-6;

/// MTF の補間（求積の点数 n）。格子の一辺を min(n, [`INTERP_GRID_START`]) から始め、区間の中点の
/// 標本で補間と直接追跡の差が [`INTERP_TOL`] を超えれば 2 倍にする（上限 n。n 以下の格子は従来と同じ）。
/// 格子 n でも差が [`INTERP_MASK_TOL`] を超えるとき（外挿しない Zernike 面の正規化半径の縁など、OPD に
/// 段差がある系。Z3）は、全区間の中点で確かめ、差がそれを超える区間の角の格子点を NaN にする。足にその点を
/// 含む求積点は直接追跡になる。中点の標本（一辺およそ 16 点）の半分以上で超えるときは、格子をすべて NaN にして
/// 全部直接追跡
fn mtf_interp(s: &System, field: FieldSel, wi: usize, n: usize) -> Result<PupilInterp> {
    let n = n.max(32);
    let mut g = n.min(INTERP_GRID_START);
    loop {
        let mut ip = PupilInterp::new(s, field, wi, g)?;
        let cells = ((2.0 / ip.h).round() as usize).max(1);
        if g < n {
            let e = interp_errors(s, field, wi, &ip, cells.div_ceil(48).max(1))?
                .iter()
                .fold(0.0_f64, |m, &(_, _, e)| m.max(e));
            if e <= INTERP_TOL {
                return Ok(ip);
            }
            g = (2 * g).min(n);
            continue;
        }
        // 格子 n: 段差・粗すぎる格子を一辺およそ 16 点の中点で探す（安く済ませる。見つからなければ従来と同じ）
        let errs = interp_errors(s, field, wi, &ip, cells.div_ceil(16).max(1))?;
        let nb = errs.iter().filter(|x| x.2 > INTERP_MASK_TOL).count();
        if 2 * nb >= errs.len() && nb > 0 {
            // 瞳の半分以上で補間が効かない（Grid Sag の G1・G2 など、OPD が格子の線ごとに折れる系）:
            // 全区間の確かめを省き、すべて直接追跡にする
            ip.w.fill(f64::NAN);
        } else if nb > 0 {
            let bad: Vec<(usize, usize)> = interp_errors(s, field, wi, &ip, 1)?
                .into_iter()
                .filter(|&(_, _, e)| e > INTERP_MASK_TOL)
                .map(|(ix, iy, _)| (ix, iy))
                .collect();
            ip.mask_cells(&bad);
        }
        return Ok(ip);
    }
}

/// 補間の格子 ip の区間の中点（step 区間おき）で、補間と直接追跡の差 [波]: (区間の番号 ix, iy, 差)。
/// 中点が瞳（半径 1 に区間 1 つ分を足した円）の外の区間と、補間・直接追跡のどちらかができない点は除く
fn interp_errors(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: &PupilInterp,
    step: usize,
) -> Result<Vec<(usize, usize, f64)>> {
    let cells = ((2.0 / ip.h).round() as usize).max(1);
    let lim = if step == 1 { (1.0 + ip.h).powi(2) } else { 1.0 };
    let (mut px, mut py, mut id) = (Vec::new(), Vec::new(), Vec::new());
    for iy in (0..cells).step_by(step) {
        for ix in (0..cells).step_by(step) {
            let x = -1.0 + (ix as f64 + 0.5) * ip.h;
            let y = -1.0 + (iy as f64 + 0.5) * ip.h;
            if x * x + y * y < lim {
                px.push(x);
                py.push(y);
                id.push((ix, iy));
            }
        }
    }
    let o = opd(s, field, &px, &py, Some(wi), true)?;
    let mut out = Vec::new();
    for k in 0..px.len() {
        if let (Some(v), true) = (ip.eval(px[k], py[k]), o.ok(k) && o.opd[k].is_finite()) {
            out.push((id[k].0, id[k].1, (v - o.opd[k]).abs()));
        }
    }
    Ok(out)
}

/// MTF の瞳の OPD の求め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MtfMethod {
    /// 既定。[`MtfMethod::Interp`]。ただし求積の点数 n を自動で決めて [`AUTO_FFT_N`] を超え（収差が大きい系）、
    /// 周波数が [`AUTO_FFT_FREQS`] 個以上のときは [`MtfMethod::Fft`]
    #[default]
    Auto,
    /// 瞳の格子を 1 度追跡し、求積点では 6 点の Lagrange 補間（格子にけられた点がある所だけ直接追跡）
    Interp,
    /// 求積点をすべて直接追跡（遅い。補間の確かめ用）
    Direct,
    /// 瞳の格子の瞳関数の自己相関を FFT で全周波数まとめて求める（周波数が多いとき速い）
    Fft,
}

/// 正規化瞳の正方格子で追跡した OPD の補間（各軸 6 点の Lagrange、5 次）
struct PupilInterp {
    /// 格子の点数（一辺）・間隔・始点
    m: usize,
    h: f64,
    x0: f64,
    /// OPD [波]（けられ・止まった点は NaN）、行優先 [iy][ix]
    w: Vec<f64>,
}

const LAG: usize = 6;

impl PupilInterp {
    /// 一辺 n 区間で [-1, 1] を覆い、補間の足が届くよう外側に 3 点ずつ足す
    fn new(s: &System, field: FieldSel, w: usize, n: usize) -> Result<PupilInterp> {
        crate::par::check_cancel()?;
        let h = 2.0 / n as f64;
        let pad = LAG / 2;
        let m = n + 1 + 2 * pad;
        let x0 = -1.0 - pad as f64 * h;
        let (mut px, mut py) = (Vec::with_capacity(m * m), Vec::with_capacity(m * m));
        for iy in 0..m {
            for ix in 0..m {
                px.push(x0 + ix as f64 * h);
                py.push(x0 + iy as f64 * h);
            }
        }
        let o = opd(s, field, &px, &py, Some(w), true)?;
        let w = (0..m * m)
            .map(|i| {
                if o.ok(i) && o.opd[i].is_finite() {
                    o.opd[i]
                } else {
                    f64::NAN
                }
            })
            .collect();
        Ok(PupilInterp { m, h, x0, w })
    }

    /// 1 点の補間。足の 36 点のどれかが NaN（けられ）なら None
    fn eval(&self, x: f64, y: f64) -> Option<f64> {
        let (wx, ix) = self.weights(x)?;
        let (wy, iy) = self.weights(y)?;
        let mut acc = 0.0;
        for (a, &cy) in wy.iter().enumerate() {
            let o = (iy + a) * self.m + ix;
            let row: &[f64; LAG] = self.w[o..o + LAG].try_into().unwrap();
            let mut r = 0.0;
            for k in 0..LAG {
                r += row[k] * wx[k];
            }
            acc += r * cy;
        }
        acc.is_finite().then_some(acc)
    }

    /// 1 軸の Lagrange の重みと足の最初の番号
    fn weights(&self, x: f64) -> Option<([f64; LAG], usize)> {
        let t = (x - self.x0) / self.h;
        if !t.is_finite() {
            return None;
        }
        // floor（libm を呼ばない）
        let fl = t as isize - ((t as isize) as f64 > t) as isize;
        let i = fl - (LAG as isize / 2 - 1);
        if i < 0 || i as usize + LAG > self.m {
            return None;
        }
        let u = t - i as f64; // 足の点 0..LAG に対する位置
                              // Lagrange の重み: Π_{j≠k}(u − j) / Π_{j≠k}(k − j)。分母は定数
        const DEN: [f64; LAG] = [-120.0, 24.0, -12.0, 12.0, -24.0, 120.0];
        let d: [f64; LAG] = std::array::from_fn(|j| u - j as f64);
        // 左からの積と右からの積
        let mut left = [1.0; LAG];
        let mut right = [1.0; LAG];
        for k in 1..LAG {
            left[k] = left[k - 1] * d[k - 1];
            right[LAG - 1 - k] = right[LAG - k] * d[LAG - k];
        }
        let wv: [f64; LAG] = std::array::from_fn(|k| left[k] * right[k] / DEN[k]);
        Some((wv, i as usize))
    }
}

impl PupilInterp {
    /// 区間 (ix, iy)（[-1, 1] を区切った区間の番号）の 4 隅の格子点を NaN にする（補間させず直接追跡させる）
    fn mask_cells(&mut self, cells: &[(usize, usize)]) {
        let pad = LAG / 2;
        for &(ix, iy) in cells {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (gx, gy) = (pad + ix + dx, pad + iy + dy);
                if gx < self.m && gy < self.m {
                    self.w[gy * self.m + gx] = f64::NAN;
                }
            }
        }
    }
}

/// 点 (x, y) の OPD [波]。補間できない点は直接追跡する。けられ・止まった点は NaN
fn opd_at(
    s: &System,
    field: FieldSel,
    w: usize,
    ip: Option<&PupilInterp>,
    x: &[f64],
    y: &[f64],
) -> Result<Vec<f64>> {
    let mut out = vec![f64::NAN; x.len()];
    let mut miss = Vec::new();
    match ip {
        Some(ip) => {
            // 点ごとに独立（並列でも値は同じ）
            use rayon::prelude::*;
            let f = |(i, o): (usize, &mut f64)| {
                if let Some(v) = ip.eval(x[i], y[i]) {
                    *o = v;
                }
            };
            if x.len() >= 4096 && !crate::par::sequential() {
                out.par_iter_mut()
                    .enumerate()
                    .with_min_len(1024)
                    .for_each(scoped(f));
            } else {
                out.iter_mut().enumerate().for_each(f);
            }
            miss.extend((0..x.len()).filter(|&i| out[i].is_nan()));
        }
        None => miss.extend(0..x.len()),
    }
    if !miss.is_empty() {
        let mx: Vec<f64> = miss.iter().map(|&i| x[i]).collect();
        let my: Vec<f64> = miss.iter().map(|&i| y[i]).collect();
        let o = opd(s, field, &mx, &my, Some(w), true)?;
        for (k, &i) in miss.iter().enumerate() {
            if o.ok(k) && o.opd[k].is_finite() {
                out[i] = o.opd[k];
            }
        }
    }
    Ok(out)
}

impl PupilInterp {
    /// 1 軸が一定の点の列の補間（行ごとに固定の軸を先に縮約するので [`PupilInterp::eval`] より速い）。
    /// fixed_x なら点は (c, v[i])、そうでなければ (v[i], c)。2 本の列 ca・cb を同じ v で求め、補間できない点は
    /// NaN のまま。fixed_x の値は [`PupilInterp::eval`] とビットまで同じ（足し算の順が同じ）
    #[allow(clippy::too_many_arguments)]
    fn eval_lines(
        &self,
        fixed_x: bool,
        ca: f64,
        cb: f64,
        v: &[f64],
        oa: &mut [f64],
        ob: &mut [f64],
        col: &mut [Vec<f64>; 2],
    ) {
        let m = self.m;
        let mut have = [false; 2];
        for (j, c) in [ca, cb].into_iter().enumerate() {
            let Some((wc, ic)) = self.weights(c) else {
                continue;
            };
            have[j] = true;
            let cj = &mut col[j];
            cj.clear();
            if fixed_x {
                cj.extend((0..m).map(|iy| {
                    let o = iy * m + ic;
                    let row: &[f64; LAG] = self.w[o..o + LAG].try_into().unwrap();
                    let mut r = 0.0;
                    for k in 0..LAG {
                        r += row[k] * wc[k];
                    }
                    r
                }));
            } else {
                cj.resize(m, 0.0);
                for (a, &wa) in wc.iter().enumerate() {
                    let row = &self.w[(ic + a) * m..(ic + a + 1) * m];
                    for (o, &r) in cj.iter_mut().zip(row) {
                        *o += r * wa;
                    }
                }
            }
        }
        if !have[0] && !have[1] {
            return;
        }
        for (i, &x) in v.iter().enumerate() {
            let Some((wv, iv)) = self.weights(x) else {
                continue;
            };
            for (j, o) in [&mut oa[i], &mut ob[i]].into_iter().enumerate() {
                if !have[j] {
                    continue;
                }
                let c = &col[j][iv..iv + LAG];
                let mut acc = 0.0;
                for a in 0..LAG {
                    acc += c[a] * wv[a];
                }
                if acc.is_finite() {
                    *o = acc;
                }
            }
        }
    }

    /// 格子の OPD が x ↔ y の入れ替えと y の反転で変わらない（|差| ≤ 1e-9 波、けられの形も同じ）か。
    /// 軸上の回転対称な系では子午と球欠の MTF が同じになる
    fn symmetric(&self) -> bool {
        let m = self.m;
        let same = |a: f64, b: f64| (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9;
        (0..m).all(|iy| {
            (0..m).all(|ix| {
                let v = self.w[iy * m + ix];
                same(v, self.w[ix * m + iy]) && same(v, self.w[(m - 1 - iy) * m + ix])
            })
        })
    }
}

/// 子午と球欠の MTF が同じになるか（軸上の回転対称な系）: 格子の OPD が対称（[`PupilInterp::symmetric`]）で、
/// 瞳の縮尺 J が x・y で同じ
fn axis_symmetric(jm: &[[f64; 2]; 2], ip: &PupilInterp) -> bool {
    let a = jm[0][0].abs();
    (jm[0][0] - jm[1][1]).abs() <= 1e-12 * a
        && jm[0][1].abs() <= 1e-12 * a
        && jm[1][0].abs() <= 1e-12 * a
        && ip.symmetric()
}

/// 直接追跡の点（補間できなかった点）を埋める
fn fill_direct(
    s: &System,
    field: FieldSel,
    w: usize,
    x: &[f64],
    y: &[f64],
    out: &mut [f64],
) -> Result<()> {
    let miss: Vec<usize> = (0..x.len()).filter(|&i| out[i].is_nan()).collect();
    if !miss.is_empty() {
        let mx: Vec<f64> = miss.iter().map(|&i| x[i]).collect();
        let my: Vec<f64> = miss.iter().map(|&i| y[i]).collect();
        let o = opd(s, field, &mx, &my, Some(w), true)?;
        for (k, &i) in miss.iter().enumerate() {
            if o.ok(k) && o.opd[k].is_finite() {
                out[i] = o.opd[k];
            }
        }
    }
    Ok(())
}

/// exp(2πi Δ)
#[inline]
fn phasor(dw: f64) -> C64 {
    let ph = 2.0 * std::f64::consts::PI * dw;
    let (s, c) = ph.sin_cos();
    C64::new(c, s)
}

/// Σ w[i] exp(2πi (a[i] − b[i]))（a・b のどちらかが有限でない点は飛ばす）。位相因子は [`crate::trig::cis_scaled`] で
/// まとめて求め（libm の sin_cos と相対 1e-13 程度の差）、和は i の順に取る
fn phasor_sum(a: &[f64], b: &[f64], w: &[f64]) -> C64 {
    let (mut ph, mut am) = (Vec::with_capacity(w.len()), Vec::with_capacity(w.len()));
    for i in 0..w.len() {
        if a[i].is_finite() && b[i].is_finite() {
            ph.push(2.0 * std::f64::consts::PI * (a[i] - b[i]));
            am.push(w[i]);
        }
    }
    let mut z = vec![C64::new(0.0, 0.0); ph.len()];
    crate::trig::cis_scaled(&ph, &am, &mut z);
    z.iter().fold(C64::new(0.0, 0.0), |acc, &v| acc + v)
}

/// 補間できなかった点（直接追跡する）: (xa, ya, xb, yb, 重み, a, b)。a・b は補間できた値か NaN
type Miss = (f64, f64, f64, f64, f64, f64, f64);

/// ずらす向きが軸に沿うときの重なりの求積（[`lens_quad`] と同じ点と重み）。[`lens_quad`] の行（t が一定）
/// ごとに、補間の固定の軸を先に縮約する（[`PupilInterp::eval_lines`]）。和は行ごとに取って行の順に足し、
/// 補間できなかった点は最後に直接追跡して足す（スレッド数によらず同じ値）
#[allow(clippy::too_many_arguments)]
fn shifted_rows(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: &PupilInterp,
    gl: &LensGl,
    d: f64,
    (dx, dy): (f64, f64),
    (ex, ey): (f64, f64),
) -> Result<C64> {
    let a = 1.0 - 0.5 * d;
    let mut acc = C64::new(0.0, 0.0);
    if a <= 0.0 {
        return Ok(acc);
    }
    let fixed_x = ey == 0.0;
    let nr = gl.gt.len() * 2;
    let row = |r: usize, buf: &mut [Vec<f64>; 5]| -> (C64, Vec<Miss>) {
        let (z, wz) = (gl.gt[r / 2], gl.wt[r / 2]);
        let sgn = if r.is_multiple_of(2) { 1.0 } else { -1.0 };
        let tau = 0.5 * (z + 1.0);
        let y = a * (1.0 - tau * tau);
        let dyq = 2.0 * a * tau * 0.5 * wz;
        let x = (1.0 - (y + 0.5 * d).powi(2)).max(0.0).sqrt();
        let t = sgn * y;
        let [var, oa, ob, ca, cb] = buf;
        // 点 (t, u)（u = x·g）を回した (X, Y) = (t ex − u ey, t ey + u ex)。行の中で一定の軸は t だけで決まる
        var.clear();
        let (fa, fb) = if fixed_x {
            var.extend(gl.gu.iter().map(|&g| (x * g) * ex));
            let xx = t * ex;
            (xx + 0.5 * dx, xx - 0.5 * dx)
        } else {
            var.extend(gl.gu.iter().map(|&g| -((x * g) * ey)));
            let yy = t * ey;
            (yy + 0.5 * dy, yy - 0.5 * dy)
        };
        let nu = var.len();
        oa.clear();
        oa.resize(nu, f64::NAN);
        ob.clear();
        ob.resize(nu, f64::NAN);
        let mut col = [std::mem::take(ca), std::mem::take(cb)];
        ip.eval_lines(fixed_x, fa, fb, var, oa, ob, &mut col);
        [*ca, *cb] = col;
        let mut miss = Vec::new();
        let ws: Vec<f64> = gl.wu.iter().map(|&wg| dyq * x * wg).collect();
        for (i, &w) in ws.iter().enumerate() {
            let (va, vb) = (oa[i], ob[i]);
            if va.is_nan() || vb.is_nan() {
                let (xa, ya, xb, yb) = if fixed_x {
                    (fa, var[i], fb, var[i])
                } else {
                    (var[i], fa, var[i], fb)
                };
                miss.push((xa, ya, xb, yb, w, va, vb));
            }
        }
        // 補間できた点（NaN でない点）の和。oa・ob は補間できないと NaN、できれば有限
        let sum = phasor_sum(oa, ob, &ws);
        (sum, miss)
    };
    let new_buf = || -> [Vec<f64>; 5] { Default::default() };
    let rows: Vec<(C64, Vec<Miss>)> = if nr * gl.gu.len() >= 4096 && !crate::par::sequential() {
        use rayon::prelude::*;
        let cx = crate::par::current();
        (0..nr)
            .into_par_iter()
            .with_min_len((1024 / gl.gu.len()).max(1))
            .map_init(new_buf, |b, r| crate::par::with_ctx(&cx, || row(r, b)))
            .collect()
    } else {
        let mut b = new_buf();
        (0..nr).map(|r| row(r, &mut b)).collect()
    };
    let mut miss: Vec<Miss> = Vec::new();
    for (sum, m) in rows {
        acc += sum;
        miss.extend(m);
    }
    if !miss.is_empty() {
        let mut va: Vec<f64> = miss.iter().map(|m| m.5).collect();
        let mut vb: Vec<f64> = miss.iter().map(|m| m.6).collect();
        let (xa, ya): (Vec<f64>, Vec<f64>) = miss.iter().map(|m| (m.0, m.1)).unzip();
        let (xb, yb): (Vec<f64>, Vec<f64>) = miss.iter().map(|m| (m.2, m.3)).unzip();
        fill_direct(s, field, wi, &xa, &ya, &mut va)?;
        fill_direct(s, field, wi, &xb, &yb, &mut vb)?;
        for (i, m) in miss.iter().enumerate() {
            if va[i].is_finite() && vb[i].is_finite() {
                acc += phasor(va[i] - vb[i]) * m.4;
            }
        }
    }
    Ok(acc)
}

/// 瞳の格子の行ごとの自己相関の和 r[s] = Σ_行 Σ_i P[i + s] P*[i]（s = 0..L、負の s は L + s）。
/// rows は P を行優先で並べたもの（一辺 ns）、L ≥ 2 ns で巡回の重なりなし
fn row_autocorr(p: &[C64], ns: usize, l: usize) -> Vec<C64> {
    use rayon::prelude::*;
    let fft = rustfft::FftPlanner::<f64>::new().plan_fft_forward(l);
    let zero = C64::new(0.0, 0.0);
    // 行を決まった大きさの塊に分けて塊ごとに |F|² を足し、塊の順に足す（スレッド数によらず同じ値）
    const CH: usize = 16;
    let part = |rows: &[C64]| -> Vec<f64> {
        let mut buf = vec![zero; l];
        let mut scratch = vec![zero; fft.get_inplace_scratch_len()];
        let mut acc = vec![0.0; l];
        for row in rows.chunks(ns) {
            if row.iter().all(|c| c.re == 0.0 && c.im == 0.0) {
                continue;
            }
            buf[..ns].copy_from_slice(row);
            buf[ns..].fill(zero);
            fft.process_with_scratch(&mut buf, &mut scratch);
            for (a, f) in acc.iter_mut().zip(&buf) {
                *a += f.norm_sqr();
            }
        }
        acc
    };
    let parts: Vec<Vec<f64>> = if !crate::par::sequential() {
        p.par_chunks(ns * CH).map(scoped(part)).collect()
    } else {
        p.chunks(ns * CH).map(part).collect()
    };
    let mut sp = vec![zero; l];
    for q in parts {
        for (a, v) in sp.iter_mut().zip(q) {
            a.re += v;
        }
    }
    let ifft = rustfft::FftPlanner::<f64>::new().plan_fft_inverse(l);
    ifft.process(&mut sp);
    let k = 1.0 / l as f64;
    sp.iter().map(|c| c * k).collect()
}

/// 自己相関 r（[`row_autocorr`]）の非整数のずれ s での値（4 点の Lagrange 補間、r[−s] = r[s]*）。
/// 線形補間より 1 桁ほど合う（A0 の標本 256 で 9.8e-4 → 2.1e-4）
fn autocorr_at(r: &[C64], s: f64) -> C64 {
    let l = r.len() as isize;
    let at = |i: isize| -> C64 {
        if i >= 0 {
            r[i as usize]
        } else {
            r[(l + i) as usize]
        }
    };
    let i0 = s.floor();
    let u = s - i0;
    let i0 = i0 as isize;
    if u == 0.0 {
        return at(i0);
    }
    // 点 i0−1, i0, i0+1, i0+2
    let w = [
        -u * (u - 1.0) * (u - 2.0) / 6.0,
        (u + 1.0) * (u - 1.0) * (u - 2.0) / 2.0,
        -(u + 1.0) * u * (u - 2.0) / 2.0,
        (u + 1.0) * u * (u - 1.0) / 6.0,
    ];
    (0..4).map(|k| at(i0 - 1 + k as isize) * w[k]).sum()
}

/// 周波数・向きごとの求積の点数（[`MtfMethod::Auto`]・[`MtfMethod::Interp`] で点数が自動のとき。上限は自動の n）。
/// 求積するのは位相差 D(p) = W(p + s/2) − W(p − s/2)（s はずらし (dx, dy)）の位相因子なので、要る点数は W ではなく
/// D の傾きで決まる。D の傾きの最大 g_D [波 / 半径] を、重なりの中の [`SHIFT_GRID`]² 点で補間した D の隣り合う
/// 点の差で見積もり、n_D = [`SHIFT_FAC`]·g_D + 16（32〜4096）にする。自動の n（[`mtf_points`]）は W の傾き g から D の
/// 傾きの上限 2g を見込むが、低い周波数（ずらしが小さい）では D ≈ s·∇W で傾きはずっと小さい（E1 の視野 3、
/// 20 cycles/mm で g_D ≈ 40 に対し g ≈ 620）。補間の格子がすべて有限（けられなし）のときだけ呼ぶ
fn shift_points(ip: &PupilInterp, dx: f64, dy: f64) -> usize {
    let m = SHIFT_GRID;
    let d = dx.hypot(dy);
    let a = 1.0 - 0.5 * d;
    if a <= 0.0 {
        return 32;
    }
    let (ex, ey) = if d > 0.0 {
        (dx / d, dy / d)
    } else {
        (1.0, 0.0)
    };
    // 重なり（レンズ形）の中の m×m 点: ずらす向きの座標 t は区間の中心、直交する座標 u は各 t の弦を m 等分した
    // 区間の中心（[`lens_quad`] と同じ形の領域を覆う。周波数が高いと領域が細く、正方格子では点が足りない）
    let mut dv = vec![f64::NAN; m * m];
    let mut pt = vec![(0.0, 0.0); m * m];
    for i in 0..m {
        let t = a * (-1.0 + (2 * i + 1) as f64 / m as f64);
        let x = (1.0 - (t.abs() + 0.5 * d).powi(2)).max(0.0).sqrt();
        for j in 0..m {
            let u = x * (-1.0 + (2 * j + 1) as f64 / m as f64);
            let (px, py) = (t * ex - u * ey, t * ey + u * ex);
            pt[i * m + j] = (px, py);
            if let (Some(va), Some(vb)) = (
                ip.eval(px + 0.5 * dx, py + 0.5 * dy),
                ip.eval(px - 0.5 * dx, py - 0.5 * dy),
            ) {
                dv[i * m + j] = va - vb;
            }
        }
    }
    let mut g: f64 = 0.0;
    let mut slope = |k: usize, k2: usize| {
        let (v, v2) = (dv[k], dv[k2]);
        let dist = (pt[k2].0 - pt[k].0).hypot(pt[k2].1 - pt[k].1);
        if v.is_finite() && v2.is_finite() && dist > 0.0 {
            g = g.max((v2 - v).abs() / dist);
        }
    };
    for i in 0..m {
        for j in 0..m {
            let k = i * m + j;
            if j + 1 < m {
                slope(k, k + 1);
            }
            if i + 1 < m {
                slope(k, k + m);
            }
        }
    }
    ((SHIFT_FAC * g).ceil() as usize + 16).clamp(32, 4096)
}

/// [`shift_points`] の格子の一辺の点数
const SHIFT_GRID: usize = 33;
/// [`shift_points`] で点数を減らす自動の n の下限。n が小さい系は見積もり（補間 2·33² 点）が求積（n² 点）と
/// 同じくらいかかり、速くならない
const SHIFT_MIN_N: usize = 128;
/// [`shift_points`] の傾き g_D あたりの点数。位相が g_D [波 / 半径] で回る因子を Gauss-Legendre で求めるには
/// 半径あたり 2π g_D 点ほど要る（E1 の視野 3 で 4 では 1e-5 ずれ、6 で 1e-12 に収束）。余裕を見て 8。
/// 例題 38 系の遮断周波数の 1〜99% で、点数を減らさないときとの差は 1e-7 以下（補間の誤差と同じ桁）
const SHIFT_FAC: f64 = 8.0;

/// [`MtfMethod::Auto`] が FFT に切り替える求積の点数 n（自動で決めた値がこれを超えるとき）。求積は点が n² 個で
/// 周波数ごとに足すので、E0・E1 の軸外（OPD の傾き 600〜700 波、n が 2000 超）は FFT の 4 倍ほど遅い。
/// FFT の瞳の標本は 2n（上限 [`FFT_MAX`]）で、求積との差は E0・E1 で 7e-5 以下
pub const AUTO_FFT_N: usize = 1024;

/// [`MtfMethod::Auto`] が FFT に切り替える周波数の数の下限。求積は周波数ごとに時間がかかり、FFT は周波数の数に
/// よらない。n が [`AUTO_FFT_N`] を超える系では、周波数がおよそ 7 個で時間が並ぶ（E1: 2 個で求積 1.8 s・FFT 3.0 s、
/// 20 個で 6.6 s・3.4 s）
pub const AUTO_FFT_FREQS: usize = 8;

/// [`MtfMethod::Fft`] の瞳の標本の数（直径）の範囲
const FFT_MIN: usize = 512;
const FFT_MAX: usize = 4096;

/// 1 波長の複素 OTF を FFT の自己相関で求める（[`MtfMethod::Fft`]）。返す値は [`wave_otf`] と同じ形。
/// 瞳の標本の OPD は [`PupilInterp`] の補間。球欠は行ごと、子午は列ごとの 1 次元の自己相関の和で、
/// 周波数のずれ（標本の間隔の非整数倍）は自己相関を 4 点で補間する
fn wave_otf_fft(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: Option<usize>,
    freqs: &[f64],
) -> Result<WaveOtf> {
    let jm = pupil_jacobian(s, field, wi)?;
    let n = match n {
        Some(n) => n,
        None => mtf_points(opd_slope_max(s, field, wi)?),
    };
    let lam = s.wavelengths[wi] * 1e-3;
    let ip = mtf_interp(s, field, wi, n)?;
    // 瞳の標本（直径 ns 点、区間の中心）。円の縁は中心が内側かどうかで 0・1 にする（面積の割合を振幅にすると
    // 縁どうしの重なりが割合の積になって h に比例してずれる）。補間との差は ns² に反比例
    // （A0 などで ns 128: 1.3e-3、256: 2.1e-4、512: 5e-5）。標本は求積の点数の 2 倍にする。上限 2048 では E0・E1 の
    // 視野 3（n が 2000 超、OPD の傾き 600〜700 波）で位相の標本が足りず、高い周波数で MTF が 1.7e-3 浮いた（4096 で
    // 求積と 2e-5）
    let ns = (2 * n).clamp(FFT_MIN, FFT_MAX);
    let h = 2.0 / ns as f64;
    let cx: Vec<f64> = (0..ns).map(|i| -1.0 + h * (i as f64 + 0.5)).collect();
    let inside = |i: usize| {
        let (x, y) = (cx[i % ns], cx[i / ns]);
        x * x + y * y <= 1.0
    };
    // 各行の OPD（行ごとに補間、補間できない点は直接追跡）
    let mut wv = vec![f64::NAN; ns * ns];
    {
        use rayon::prelude::*;
        let f = |(iy, row): (usize, &mut [f64])| {
            let mut tmp = vec![f64::NAN; ns];
            let mut col = [Vec::new(), Vec::new()];
            ip.eval_lines(false, cx[iy], cx[iy], &cx, row, &mut tmp, &mut col);
        };
        if !crate::par::sequential() {
            wv.par_chunks_mut(ns).enumerate().for_each(scoped(f));
        } else {
            wv.chunks_mut(ns).enumerate().for_each(f);
        }
    }
    let need: Vec<usize> = (0..ns * ns)
        .filter(|&i| wv[i].is_nan() && inside(i))
        .collect();
    if !need.is_empty() {
        let mx: Vec<f64> = need.iter().map(|&i| cx[i % ns]).collect();
        let my: Vec<f64> = need.iter().map(|&i| cx[i / ns]).collect();
        let mut v = vec![f64::NAN; need.len()];
        fill_direct(s, field, wi, &mx, &my, &mut v)?;
        for (k, &i) in need.iter().enumerate() {
            wv[i] = v[k];
        }
    }
    let zero = C64::new(0.0, 0.0);
    let mut e0 = 0.0;
    let mut p: Vec<C64> = (0..ns * ns)
        .map(|i| {
            if inside(i) && wv[i].is_finite() {
                e0 += 1.0;
                phasor(wv[i])
            } else {
                zero
            }
        })
        .collect();
    drop(wv);
    if !(e0 > 0.0) {
        invalid!("MTF: 有効な光線がない（すべてけられた）");
    }
    let l = (2 * ns).next_power_of_two();
    // 球欠: 行（x の向き）の自己相関、子午: 列（y の向き）
    let rs = row_autocorr(&p, ns, l);
    let rt = if axis_symmetric(&jm, &ip) {
        rs.clone()
    } else {
        // その場で転置する（ns = 4096 で瞳の標本は 1 枚 268 MB。写しを作らない）
        for iy in 0..ns {
            for ix in iy + 1..ns {
                p.swap(iy * ns + ix, ix * ns + iy);
            }
        }
        row_autocorr(&p, ns, l)
    };
    let (mut sag, mut tan) = (Vec::new(), Vec::new());
    let (sx, _) = pupil_shift(&jm, lam, 0);
    let (_, sy) = pupil_shift(&jm, lam, 1);
    for &f in freqs {
        let (dx, dy) = (sx * f, sy * f);
        sag.push(if dx.abs() < 2.0 {
            autocorr_at(&rs, dx / h)
        } else {
            zero
        });
        tan.push(if dy.abs() < 2.0 {
            autocorr_at(&rt, dy / h)
        } else {
            zero
        });
    }
    Ok((sag, tan, (e0, e0)))
}

/// 回折 MTF。瞳関数 P = exp(2πi W)（W は主波長の主光線を基準にした OPD [波]）の自己相関を、ずらした瞳の点で
/// 直接求積する（OPD は [`MtfMethod`] の既定で瞳の格子の補間、FFT は [`MtfMethod::Fft`]）。周波数 ν の縞は、像空間の方向余弦で λν だけずれた
/// 2 つの光線の干渉なので、正規化瞳では [`pupil_shift`] の δν ずらす（瞳から方向余弦への写像は線形とみなす）。w None なら全波長を重みで足した多色（複素 OTF の和、波長ごとに光量で規格化）。
/// 重なりの積分は [`lens_quad`]（n は求積の点の数の目安、点は約 n² 個）
pub fn mtf(s: &System, field: FieldSel, w: Waves, n: Option<usize>, freqs: &[f64]) -> Result<Mtf> {
    let w = w.one();
    mtf_with(s, field, Waves::all_or(w), n, freqs, MtfMethod::default())
}

/// 1 波長の複素 OTF（光量で割る前）: (球欠, 子午, ずらさない瞳の光量 (球欠, 子午))。光量は向きごとに
/// その向きの求積と同じ点と重みで求める
type WaveOtf = (Vec<C64>, Vec<C64>, (f64, f64));

/// 1 波長の複素 OTF（[`WaveOtf`]）
fn wave_otf(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    only: Option<usize>,
) -> Result<WaveOtf> {
    if method == MtfMethod::Fft && !freqs.is_empty() {
        return wave_otf_fft(s, field, wi, n, freqs);
    }
    let auto = n.is_none();
    let n = match n {
        Some(n) => n,
        None => mtf_points(opd_slope_max(s, field, wi)?),
    };
    if method == MtfMethod::Auto && auto && n > AUTO_FFT_N && freqs.len() >= AUTO_FFT_FREQS {
        return wave_otf_fft(s, field, wi, Some(n), freqs);
    }
    let jm = pupil_jacobian(s, field, wi)?;
    let lam = s.wavelengths[wi] * 1e-3;
    // 補間の格子: 求積の点数と同じくらいの細かさ（6 点補間の誤差は h^6 で小さい）
    let ip = match method {
        MtfMethod::Auto | MtfMethod::Interp if !freqs.is_empty() => {
            Some(mtf_interp(s, field, wi, n)?)
        }
        _ => None,
    };
    // 軸上の回転対称な系（格子の OPD が対称で、瞳の縮尺が x・y で同じ）なら子午は球欠と同じ
    let sym = ip.as_ref().is_some_and(|ip| axis_symmetric(&jm, ip));
    // only の向きだけ求める（もう一方は 0 のまま。対称なら球欠を求めて子午に写す）
    let dirs: &[usize] = match (sym, only) {
        (true, _) => &[0],
        (false, Some(0)) => &[0],
        (false, Some(_)) => &[1],
        (false, None) => &[0, 1],
    };
    let jobs: Vec<(usize, usize)> = (0..freqs.len())
        .flat_map(|k| dirs.iter().map(move |&d| (k, d)))
        .collect();
    // 正規化瞳でのずれ（dir 0: 像面の x、1: y の周波数）
    let shift = |(k, dir): (usize, usize)| {
        let (sx, sy) = pupil_shift(&jm, lam, dir);
        (sx * freqs[k], sy * freqs[k])
    };
    // 補間の格子がすべて有限なら、単位円の中の点はどれも補間できて有限（けられなし）。このときは
    // - 瞳の光量を点ごとの OPD から数えなくてよい（E0・E1 の軸外で 1 波長 0.4〜0.7 s かかっていた）
    // - 自動の点数なら、周波数・向きごとにずらしの位相差の傾きで点数を減らす（[`shift_points`]）
    // 和はその点数での瞳の光量（[`lens_area`]）で割り、光量は単位円の面積 π とする（点数による縁の誤差を
    // 打ち消し、周波数 0 で 1）
    let all_finite = ip
        .as_ref()
        .is_some_and(|ip| ip.w.iter().all(|v| v.is_finite()));
    let (gls, nks, e0): (Vec<(usize, LensGl)>, Vec<usize>, f64) = if all_finite {
        let ip = ip.as_ref().unwrap();
        let nks: Vec<usize> = jobs
            .iter()
            .map(|&j| {
                let (dx, dy) = shift(j);
                if auto && n >= SHIFT_MIN_N {
                    shift_points(ip, dx, dy).min(n)
                } else {
                    n
                }
            })
            .collect();
        let mut uniq = nks.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let gls = crate::par::map(&uniq, |&m| (m, LensGl::new(m)));
        (gls, nks, std::f64::consts::PI)
    } else {
        // ずらさない瞳の光量（けられを含む）
        let gl = LensGl::new(n);
        let (t0, u0, w0) = lens_quad(0.0, &gl);
        let o0 = opd_at(s, field, wi, ip.as_ref(), &u0, &t0)?;
        let e0: f64 = (0..t0.len())
            .filter(|&i| o0[i].is_finite())
            .map(|i| w0[i])
            .sum();
        if !(e0 > 0.0) {
            invalid!("MTF: 有効な光線がない（すべてけられた）");
        }
        // 瞳が開口・遮蔽で切れている（けられた点がある）なら縁に合わせた求積にする
        if o0.iter().any(|v| !v.is_finite()) {
            return clipped_otf(s, field, wi, ip.as_ref(), n, &jm, lam, freqs, sym);
        }
        (vec![(n, gl)], vec![n; jobs.len()], e0)
    };
    // 周波数・向きごとに独立（並列でも値は同じ。和は周波数・向きの中で順に取る）
    let one = |ji: usize| -> Result<C64> {
        // GUI で系が変わったら途中でやめる（周波数・向きごとに確かめる）
        crate::par::check_cancel()?;
        let (dx, dy) = shift(jobs[ji]);
        let d = dx.hypot(dy);
        // ずらす向き (ex, ey) と直交する向き (−ey, ex)
        let (ex, ey) = if d > 0.0 {
            (dx / d, dy / d)
        } else {
            (1.0, 0.0)
        };
        let gl = &gls.iter().find(|g| g.0 == nks[ji]).unwrap().1;
        let scale = if all_finite { e0 / lens_area(gl) } else { 1.0 };
        if let Some(ip) = &ip {
            if ey == 0.0 || ex == 0.0 {
                return Ok(shifted_rows(s, field, wi, ip, gl, d, (dx, dy), (ex, ey))? * scale);
            }
        }
        let (t, u, w) = lens_quad(d, gl);
        let mut acc = C64::new(0.0, 0.0);
        if t.is_empty() {
            return Ok(acc);
        }
        let m = t.len();
        let (mut xa, mut ya, mut xb, mut yb) =
            (vec![0.0; m], vec![0.0; m], vec![0.0; m], vec![0.0; m]);
        for i in 0..m {
            let (x, y) = (t[i] * ex - u[i] * ey, t[i] * ey + u[i] * ex);
            xa[i] = x + 0.5 * dx;
            ya[i] = y + 0.5 * dy;
            xb[i] = x - 0.5 * dx;
            yb[i] = y - 0.5 * dy;
        }
        let a = opd_at(s, field, wi, ip.as_ref(), &xa, &ya)?;
        let b = opd_at(s, field, wi, ip.as_ref(), &xb, &yb)?;
        acc += phasor_sum(&a, &b, &w);
        Ok(acc * scale)
    };
    let accs: Vec<Result<C64>> = if jobs.len() > 1 && !crate::par::sequential() {
        use rayon::prelude::*;
        (0..jobs.len()).into_par_iter().map(scoped(one)).collect()
    } else {
        (0..jobs.len()).map(one).collect()
    };
    let zero = C64::new(0.0, 0.0);
    let (mut sag, mut tan) = (vec![zero; freqs.len()], vec![zero; freqs.len()]);
    for (&(k, dir), acc) in jobs.iter().zip(accs) {
        let acc = acc?;
        if dir == 0 {
            sag[k] = acc;
            if sym {
                tan[k] = acc;
            }
        } else {
            tan[k] = acc;
        }
    }
    Ok((sag, tan, (e0, e0)))
}

/// [`pupil_segments`] の線ごとの走査の点の数（ふつうの線、区間の数が変わる所の近くの線）
const SEG_SCAN: usize = 65;
const SEG_SCAN_FINE: usize = 257;

/// 瞳を向き dir の線（dir 0: y = v の線で x の向き、1: x = v の線で y の向き）で切ったときの、光線が通る区間
/// （線に沿う座標の昇順）。単位円の弦を等間隔の点（fine の線は [`SEG_SCAN_FINE`]、他は [`SEG_SCAN`]）で
/// 走査し、通る・通らないが変わる所を二分法で詰める。走査の間隔より狭い穴・島は見落とす（区間は区間の数が
/// 変わる所でだけ 0 に縮むので、その近くの線は細かく走査する）
fn pupil_segments(
    s: &System,
    field: FieldSel,
    wi: usize,
    dir: usize,
    vs: &[f64],
    fine: &[bool],
) -> Result<Vec<Vec<(f64, f64)>>> {
    let xy = |a: f64, v: f64| if dir == 0 { (a, v) } else { (v, a) };
    let valid = |pts: &[(f64, f64)]| -> Result<Vec<bool>> {
        let (px, py): (Vec<f64>, Vec<f64>) = pts.iter().copied().unzip();
        let o = opd(s, field, &px, &py, Some(wi), true)?;
        Ok((0..pts.len())
            .map(|k| o.ok(k) && o.opd[k].is_finite())
            .collect())
    };
    let m: Vec<usize> = fine
        .iter()
        .map(|&f| if f { SEG_SCAN_FINE } else { SEG_SCAN })
        .collect();
    let mut off = vec![0usize; vs.len() + 1];
    for i in 0..vs.len() {
        off[i + 1] = off[i] + m[i];
    }
    // 弦の端は単位円のわずかに内側（丸めで外に出ない）
    let chord: Vec<f64> = vs
        .iter()
        .map(|&v| (1.0 - v * v).max(0.0).sqrt() * (1.0 - 1e-12))
        .collect();
    let at = |i: usize, j: usize| chord[i] * (-1.0 + 2.0 * j as f64 / (m[i] - 1) as f64);
    let mut pts = Vec::with_capacity(off[vs.len()]);
    for (i, &v) in vs.iter().enumerate() {
        for j in 0..m[i] {
            pts.push(xy(at(i, j), v));
        }
    }
    let ok = valid(&pts)?;
    // 変わり目 (線, 走査の番号, 通る側, 通らない側) を二分法で詰める
    let mut edges: Vec<(usize, usize, f64, f64)> = Vec::new();
    for i in 0..vs.len() {
        for j in 0..m[i] - 1 {
            let (a, b) = (ok[off[i] + j], ok[off[i] + j + 1]);
            if a != b {
                let (lo, hi) = if a {
                    (at(i, j), at(i, j + 1))
                } else {
                    (at(i, j + 1), at(i, j))
                };
                edges.push((i, j, lo, hi));
            }
        }
    }
    for _ in 0..44 {
        if edges.is_empty() {
            break;
        }
        let mid: Vec<(f64, f64)> = edges
            .iter()
            .map(|e| xy(0.5 * (e.2 + e.3), vs[e.0]))
            .collect();
        let g = valid(&mid)?;
        for (e, &good) in edges.iter_mut().zip(&g) {
            let c = 0.5 * (e.2 + e.3);
            if good {
                e.2 = c;
            } else {
                e.3 = c;
            }
        }
    }
    let mut segs = vec![Vec::new(); vs.len()];
    let mut ei = 0;
    for i in 0..vs.len() {
        let mut start = ok[off[i]].then(|| at(i, 0));
        for j in 0..m[i] - 1 {
            if ei < edges.len() && edges[ei].0 == i && edges[ei].1 == j {
                let x = 0.5 * (edges[ei].2 + edges[ei].3);
                match start.take() {
                    Some(a) => segs[i].push((a, x)),
                    None => start = Some(x),
                }
                ei += 1;
            }
        }
        if let Some(a) = start {
            segs[i].push((a, at(i, m[i] - 1)));
        }
    }
    Ok(segs)
}

/// [`clipped_otf`] の線の位置と重み（向き dir、[`pupil_segments`] と同じ線）。v = sin θ と置いて θ で
/// Gauss-Legendre、全部で約 n 本。光線が通る区間の数が変わる θ（開口の縁が線と平行な所、遮蔽の端など。
/// 線ごとの光量が跳ぶ・折れる）を [`LINE_SCAN`] 本の走査と二分法で求め、そこで θ の範囲を分ける
fn line_nodes(
    s: &System,
    field: FieldSel,
    wi: usize,
    dir: usize,
    n: usize,
) -> Result<(Vec<f64>, Vec<f64>, Vec<bool>)> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let xy = |a: f64, v: f64| if dir == 0 { (a, v) } else { (v, a) };
    // θ の線ごとの、通る区間の数（弦の COUNT_SCAN 点の走査で数える。小さい区間が見える所まで
    // 変わり目がずれないよう、区間を求める走査より細かい）
    let counts = |th: &[f64]| -> Result<Vec<usize>> {
        let (mut px, mut py) = (Vec::new(), Vec::new());
        for &t in th {
            let v = t.sin();
            let c = (1.0 - v * v).max(0.0).sqrt() * (1.0 - 1e-12);
            for j in 0..COUNT_SCAN {
                let (x, y) = xy(c * (-1.0 + 2.0 * j as f64 / (COUNT_SCAN - 1) as f64), v);
                px.push(x);
                py.push(y);
            }
        }
        let o = opd(s, field, &px, &py, Some(wi), true)?;
        let ok = |k: usize| o.ok(k) && o.opd[k].is_finite();
        Ok((0..th.len())
            .map(|i| {
                (0..COUNT_SCAN)
                    .filter(|&j| ok(i * COUNT_SCAN + j) && (j == 0 || !ok(i * COUNT_SCAN + j - 1)))
                    .count()
            })
            .collect())
    };
    let th: Vec<f64> = (0..LINE_SCAN)
        .map(|j| -FRAC_PI_2 + PI * j as f64 / (LINE_SCAN - 1) as f64)
        .collect();
    let c = counts(&th)?;
    let mut br: Vec<(f64, f64, usize, usize)> = (0..LINE_SCAN - 1)
        .filter(|&j| c[j] != c[j + 1])
        .map(|j| (th[j], th[j + 1], c[j], c[j + 1]))
        .collect();
    for _ in 0..24 {
        if br.is_empty() {
            break;
        }
        let mid: Vec<f64> = br.iter().map(|b| 0.5 * (b.0 + b.1)).collect();
        let cm = counts(&mid)?;
        for (b, (&m, &k)) in br.iter_mut().zip(mid.iter().zip(&cm)) {
            // 区間の数が両端のどちらとも違うなら（変わり目が 2 つ）、左の変わり目を追う
            if k == b.2 {
                b.0 = m;
            } else {
                b.1 = m;
                b.3 = k;
            }
        }
    }
    let mut cuts = vec![-FRAC_PI_2];
    cuts.extend(br.iter().map(|b| 0.5 * (b.0 + b.1)));
    cuts.push(FRAC_PI_2);
    let (mut vs, mut wv, mut near) = (Vec::new(), Vec::new(), Vec::new());
    // 変わり目から走査の線 1 本分の間の線は細かく走査する（[`pupil_segments`]）
    let band = PI / (LINE_SCAN - 1) as f64;
    let inner = &cuts[1..cuts.len() - 1];
    for p in cuts.windows(2) {
        let (a, b) = (p[0], p[1]);
        if !(b > a) {
            continue;
        }
        let m = ((n as f64 * (b - a) / PI).ceil() as usize).max(4);
        let (gz, gw) = gauss_legendre(m);
        let (c, r) = (0.5 * (a + b), 0.5 * (b - a));
        for (&z, &w) in gz.iter().zip(&gw) {
            let t = c + r * z;
            vs.push(t.sin());
            wv.push(r * w * t.cos());
            near.push(inner.iter().any(|&q| (t - q).abs() < band));
        }
    }
    Ok((vs, wv, near))
}

/// [`clipped_otf`] の線の数の下限。ずらした区間どうしの重なり方が変わる所（遮蔽の内側の縁が重なり始めるなど、
/// 周波数で動く）で線ごとの値が折れるので、OPD が滑らかでも線は多めにとる
const LINES_MIN: usize = 128;

/// [`line_nodes`] で線ごとの区間の数を数える弦の走査の点の数
const COUNT_SCAN: usize = SEG_SCAN_FINE;

/// [`line_nodes`] の θ の走査の線の数
const LINE_SCAN: usize = 65;

/// 区間の列 a と b（それぞれ昇順で重ならない）の共通部分
fn intersect_segs(a: &[(f64, f64)], b: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if hi > lo {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// 瞳が開口・遮蔽で切れているときの 1 波長の複素 OTF（[`WaveOtf`]）。[`lens_quad`] の点は瞳の縁の
/// 不連続をまたぐので、ずらす向きの縁で値が跳び、求積の誤差が周波数ごとにばらつく（周波数 0 でも 1 から
/// 1e-2 ずれ、点を増やしても収束が遅い）。ここでは瞳をずらす向きの線で切り、各線で光線が通る区間
/// （[`pupil_segments`]）を ±δ/2 ずらした共通部分を Gauss-Legendre で積分する。線の位置は [`line_nodes`]。
/// 光量は同じ線の区間の長さの和（周波数 0 で 1 になる）
#[allow(clippy::too_many_arguments)]
fn clipped_otf(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: Option<&PupilInterp>,
    n: usize,
    jm: &[[f64; 2]; 2],
    lam: f64,
    freqs: &[f64],
    sym: bool,
) -> Result<WaveOtf> {
    let (gz, gw) = gauss_legendre(n.max(8));
    let dirs: &[usize] = if sym { &[0] } else { &[0, 1] };
    let mut segs = [Vec::new(), Vec::new()];
    let mut lines = [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())];
    let mut e0 = [0.0; 2];
    for &dir in dirs {
        let (vs, wv, near) = line_nodes(s, field, wi, dir, n.max(LINES_MIN))?;
        segs[dir] = pupil_segments(s, field, wi, dir, &vs, &near)?;
        lines[dir] = (vs, wv);
        let wv = &lines[dir].1;
        e0[dir] = segs[dir]
            .iter()
            .zip(wv)
            .map(|(sg, &w)| w * sg.iter().map(|(a, b)| b - a).sum::<f64>())
            .sum();
        if !(e0[dir] > 0.0) {
            invalid!("MTF: 有効な光線がない（すべてけられた）");
        }
    }
    if sym {
        e0[1] = e0[0];
    }
    let one = |(k, dir): (usize, usize)| -> Result<C64> {
        // GUI で系が変わったら途中でやめる（周波数・向きごとに確かめる）
        crate::par::check_cancel()?;
        let (sx, sy) = pupil_shift(jm, lam, dir);
        let dl = if dir == 0 { sx } else { sy } * freqs[k];
        let h = 0.5 * dl.abs();
        let xy = |a: f64, v: f64| if dir == 0 { (a, v) } else { (v, a) };
        let (vs, wv) = &lines[dir];
        let (mut xa, mut ya, mut xb, mut yb, mut w) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut ov = Vec::new();
        for (i, sg) in segs[dir].iter().enumerate() {
            let lo: Vec<(f64, f64)> = sg.iter().map(|&(a, b)| (a - h, b - h)).collect();
            let hi: Vec<(f64, f64)> = sg.iter().map(|&(a, b)| (a + h, b + h)).collect();
            intersect_segs(&lo, &hi, &mut ov);
            for &(a, b) in &ov {
                let (c, r) = (0.5 * (a + b), 0.5 * (b - a));
                for (&g, &wg) in gz.iter().zip(&gw) {
                    let t = c + r * g;
                    let (x1, y1) = xy(t + 0.5 * dl, vs[i]);
                    let (x2, y2) = xy(t - 0.5 * dl, vs[i]);
                    xa.push(x1);
                    ya.push(y1);
                    xb.push(x2);
                    yb.push(y2);
                    w.push(wv[i] * r * wg);
                }
            }
        }
        let mut acc = C64::new(0.0, 0.0);
        if w.is_empty() {
            return Ok(acc);
        }
        let oa = opd_at(s, field, wi, ip, &xa, &ya)?;
        let ob = opd_at(s, field, wi, ip, &xb, &yb)?;
        acc += phasor_sum(&oa, &ob, &w);
        Ok(acc)
    };
    let jobs: Vec<(usize, usize)> = (0..freqs.len())
        .flat_map(|k| dirs.iter().map(move |&d| (k, d)))
        .collect();
    let accs: Vec<Result<C64>> = if jobs.len() > 1 && !crate::par::sequential() {
        use rayon::prelude::*;
        jobs.par_iter().copied().map(scoped(one)).collect()
    } else {
        jobs.iter().map(|&j| one(j)).collect()
    };
    let zero = C64::new(0.0, 0.0);
    let (mut sag, mut tan) = (vec![zero; freqs.len()], vec![zero; freqs.len()]);
    for (&(k, dir), acc) in jobs.iter().zip(accs) {
        let acc = acc?;
        if dir == 0 {
            sag[k] = acc;
            if sym {
                tan[k] = acc;
            }
        } else {
            tan[k] = acc;
        }
    }
    Ok((sag, tan, (e0[0], e0[1])))
}

/// 遮断周波数（主波長、x・y）
fn mtf_cutoff(s: &System, field: FieldSel) -> Result<(f64, f64)> {
    let jm = pupil_jacobian(s, field, s.primary)?;
    let lam = s.wavelengths[s.primary] * 1e-3;
    let cut = |dir: usize| {
        let (sx, sy) = pupil_shift(&jm, lam, dir);
        2.0 / sx.hypot(sy)
    };
    Ok((cut(0), cut(1)))
}

/// 波長ごとの複素 OTF（[`wave_otf`] の結果）を重みで足して MTF にする
fn mtf_sum(freqs: &[f64], otfs: &[(f64, &WaveOtf)], cutoff: (f64, f64)) -> Mtf {
    let mut tan = vec![C64::new(0.0, 0.0); freqs.len()];
    let mut sag = tan.clone();
    let mut wsum = 0.0;
    for &(wt, (sg, tn, e0)) in otfs {
        wsum += wt;
        for k in 0..freqs.len() {
            sag[k] += sg[k] * (wt / e0.0);
            tan[k] += tn[k] * (wt / e0.1);
        }
    }
    Mtf {
        freqs: freqs.to_vec(),
        tan: tan.iter().map(|c| c.norm() / wsum).collect(),
        sag: sag.iter().map(|c| c.norm() / wsum).collect(),
        cutoff,
    }
}

fn check_n(n: Option<usize>) -> Result<()> {
    if let Some(n) = n {
        if n < 4 {
            invalid!("MTF の瞳の格子 {n} は 4 以上にする");
        }
    }
    Ok(())
}

/// [`mtf`] で瞳の OPD の求め方を選ぶ
pub fn mtf_with(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
) -> Result<Mtf> {
    let w = w.one();
    mtf_only(s, field, Waves::all_or(w), n, freqs, method, None)
}

/// [`mtf_with`] で、only Some(0) なら球欠、Some(1) なら子午だけを求める（もう一方の値は使えない。評価関数の
/// 片方の向きの項用。けられのある系・FFT では両方求める）
pub(crate) fn mtf_only(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    only: Option<usize>,
) -> Result<Mtf> {
    let w = w.one();
    check_n(n)?;
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let mut otfs = Vec::new();
    for &wi in &ws {
        let wt = if ws.len() == 1 { 1.0 } else { s.weights[wi] };
        if !(wt > 0.0) {
            continue;
        }
        otfs.push((wt, wave_otf(s, field, wi, n, freqs, method, only)?));
    }
    let refs: Vec<(f64, &_)> = otfs.iter().map(|(wt, o)| (*wt, o)).collect();
    Ok(mtf_sum(freqs, &refs, mtf_cutoff(s, field)?))
}

/// 多色と各波長の MTF（OpticStudio の表示と同じ組）
#[derive(Debug, Clone)]
pub struct MtfWaves {
    /// 多色（[`mtf`] の w None と同じ値）
    pub poly: Mtf,
    /// 波長ごと（[`mtf`] の w Some(i) と同じ値。重み 0 の波長も含む）
    pub waves: Vec<Mtf>,
}

/// 多色と各波長の MTF を 1 回の計算で求める（波長ごとの OTF を使い回すので、別々に呼ぶより約 2 倍速い）
pub fn mtf_waves(
    s: &System,
    field: FieldSel,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
) -> Result<MtfWaves> {
    check_n(n)?;
    let cutoff = mtf_cutoff(s, field)?;
    let mut otfs = Vec::new();
    for wi in 0..s.wavelengths.len() {
        otfs.push(wave_otf(s, field, wi, n, freqs, method, None)?);
    }
    let multi = otfs.len() > 1;
    let poly: Vec<(f64, &_)> = otfs
        .iter()
        .enumerate()
        .filter_map(|(wi, o)| {
            let wt = if multi { s.weights[wi] } else { 1.0 };
            (wt > 0.0).then_some((wt, o))
        })
        .collect();
    if poly.is_empty() {
        invalid!("MTF: 重みが正の波長がない");
    }
    Ok(MtfWaves {
        poly: mtf_sum(freqs, &poly, cutoff),
        waves: otfs
            .iter()
            .map(|o| mtf_sum(freqs, &[(1.0, o)], cutoff))
            .collect(),
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

/// 動径の番号と、それを使う項（番号, cos なら true）
type RadUsers = (usize, Vec<(usize, bool)>);

/// 項ごとに前もって求めた Zernike 多項式。動径は R_n^m(r) = r^m · P(r²)（P は r² の多項式、正規化の係数込み）で、
/// cos と sin の組は同じ P を使う。r^m cos(mθ)・r^m sin(mθ) は (x + iy)^m の実部・虚部（平方根・割り算・三角関数なし）
pub struct ZernBasis {
    /// 異なる (n, m) ごとの P の係数（r² の高次から、Horner 用）を続けて置いたもの
    coef: Vec<f64>,
    /// 異なる (n, m) ごとの (coef の始まり, 長さ, m)
    rad: Vec<(usize, usize, usize)>,
    /// 項ごとの (rad の番号, cos なら true)
    terms: Vec<(usize, bool)>,
    mmax: usize,
    /// m ごとの、その m の動径の番号と、それを使う項（番号, cos なら true）。eval_cols で m の小さい順に回す
    by_m: Vec<Vec<RadUsers>>,
}

impl ZernBasis {
    pub fn new(kind: ZernKind, nm: &[(usize, usize, bool)]) -> ZernBasis {
        assert!(nm.iter().all(|t| t.0 < 64), "Zernike の次数は 63 まで");
        let mut coef = Vec::new();
        let mut rad: Vec<(usize, usize, usize)> = Vec::new();
        let mut keys: Vec<(usize, usize)> = Vec::new();
        let terms: Vec<(usize, bool)> = nm
            .iter()
            .map(|&(n, m, c)| {
                let ri = match keys.iter().position(|&k| k == (n, m)) {
                    Some(i) => i,
                    None => {
                        let norm = if kind == ZernKind::Standard {
                            (((n + 1) * if m > 0 { 2 } else { 1 }) as f64).sqrt()
                        } else {
                            1.0
                        };
                        // r^(n-2k) = r^m (r²)^((n-m)/2-k): k = 0 が r² の最高次
                        let start = coef.len();
                        for k in 0..=(n - m) / 2 {
                            let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
                            let ck = sgn * fact(n - k)
                                / (fact(k) * fact((n + m) / 2 - k) * fact((n - m) / 2 - k));
                            coef.push(ck * norm);
                        }
                        keys.push((n, m));
                        rad.push((start, coef.len() - start, m));
                        rad.len() - 1
                    }
                };
                (ri, c)
            })
            .collect();
        let mmax = nm.iter().map(|t| t.1).max().unwrap_or(0);
        let mut by_m: Vec<Vec<RadUsers>> = vec![Vec::new(); mmax + 1];
        for (i, r) in rad.iter().enumerate() {
            let users = terms
                .iter()
                .enumerate()
                .filter(|t| t.1 .0 == i)
                .map(|(j, t)| (j, t.1))
                .collect();
            by_m[r.2].push((i, users));
        }
        ZernBasis {
            coef,
            rad,
            terms,
            mmax,
            by_m,
        }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// 点 (px, py) での各項を out に書く（stride おきに、out[0], out[stride], ...）
    pub fn eval(&self, px: f64, py: f64, out: &mut [f64], stride: usize) {
        let r2 = px * px + py * py;
        // (x + iy)^m = r^m (cos mθ + i sin mθ)
        let mut cm = [0.0f64; 64];
        let mut sm = [0.0f64; 64];
        cm[0] = 1.0;
        for m in 1..=self.mmax {
            cm[m] = cm[m - 1] * px - sm[m - 1] * py;
            sm[m] = sm[m - 1] * px + cm[m - 1] * py;
        }
        // cos と sin の組は隣り合うので、直前の項と同じ動径なら使い回す
        let (mut last, mut h) = (usize::MAX, 0.0);
        for (j, &(ri, cos)) in self.terms.iter().enumerate() {
            let (st, len, m) = self.rad[ri];
            if ri != last {
                let c = &self.coef[st..st + len];
                h = c[0];
                for &ck in &c[1..] {
                    h = h * r2 + ck;
                }
                last = ri;
            }
            out[j * stride] = h * if cos { cm[m] } else { sm[m] };
        }
    }
}

impl ZernBasis {
    /// 点の並び (px[i], py[i]) での各項を列優先で out に書く（項 j が out[j*ld..j*ld+len]、len = px.len() ≤ ld。
    /// 列の残り ld − len 個には触れない）。点についての演算は eval と同じ順なので、値は eval とビットまで同じ。
    /// 点の向きに回すのでベクトル化が効く
    pub fn eval_cols(&self, px: &[f64], py: &[f64], out: &mut [f64], ld: usize) {
        let len = px.len().min(py.len());
        assert!(len <= ld && out.len() >= ld * self.terms.len());
        let (px, py) = (&px[..len], &py[..len]);
        let r2: Vec<f64> = px.iter().zip(py).map(|(x, y)| x * x + y * y).collect();
        let mut cm = vec![1.0; len];
        let mut sm = vec![0.0; len];
        let mut h = vec![0.0; len];
        for (m, rads) in self.by_m.iter().enumerate() {
            if m > 0 {
                for i in 0..len {
                    let (c, s) = (cm[i], sm[i]);
                    cm[i] = c * px[i] - s * py[i];
                    sm[i] = s * px[i] + c * py[i];
                }
            }
            for (ri, users) in rads {
                let (st, ln, _) = self.rad[*ri];
                let c = &self.coef[st..st + ln];
                h.iter_mut().for_each(|v| *v = c[0]);
                for &ck in &c[1..] {
                    for (v, &q) in h.iter_mut().zip(&r2) {
                        *v = *v * q + ck;
                    }
                }
                for &(j, cos) in users {
                    let col = &mut out[j * ld..j * ld + len];
                    let a = if cos { &cm } else { &sm };
                    for ((o, &hv), &av) in col.iter_mut().zip(&h).zip(a) {
                        *o = hv * av;
                    }
                }
            }
        }
    }
}

/// 瞳の点での Zernike 項（Standard は OpticStudio の正規化、Fringe は正規化なし）
pub fn zernike_row(px: f64, py: f64, kind: ZernKind, nm: &[(usize, usize, bool)]) -> Vec<f64> {
    let b = ZernBasis::new(kind, nm);
    let mut out = vec![0.0; nm.len()];
    b.eval(px, py, &mut out, 1);
    out
}

/// 内積（4 本の和に分けて足す。順序は固定なのでスレッド数によらず同じ値）
#[inline]
fn dot4(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut s = [0.0f64; 4];
    let (ca, ra) = a.as_chunks::<4>();
    let (cb, rb) = b.as_chunks::<4>();
    for (x, y) in ca.iter().zip(cb) {
        for k in 0..4 {
            s[k] += x[k] * y[k];
        }
    }
    let mut t = (s[0] + s[1]) + (s[2] + s[3]);
    for (x, y) in ra.iter().zip(rb) {
        t += x * y;
    }
    t
}

/// 最小二乗（Householder QR）: A x ≈ b
pub fn lstsq(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let m = a.len();
    if m == 0 {
        return Vec::new();
    }
    let n = a[0].len();
    let mut q = vec![0.0; m * n];
    for (i, row) in a.iter().enumerate() {
        for (j, &v) in row.iter().enumerate() {
            q[j * m + i] = v;
        }
    }
    lstsq_cols(&mut q, m, b)
}

/// 最小二乗（Householder QR）。q は A の列優先（列 j が q[j*m..(j+1)*m]）で、分解で上書きする
pub(crate) fn lstsq_cols(q: &mut [f64], m: usize, b: &[f64]) -> Vec<f64> {
    use rayon::prelude::*;
    if m == 0 {
        return Vec::new();
    }
    let n = q.len() / m;
    let mut y = b.to_vec();
    let mut rdiag = vec![0.0; n];
    let par = m * n >= 1 << 16 && !crate::par::sequential();
    for k in 0..n {
        let (done, rest) = q.split_at_mut((k + 1) * m);
        let ck = &mut done[k * m..];
        let nrm = dot4(&ck[k..], &ck[k..]).sqrt();
        if nrm == 0.0 {
            rdiag[k] = 0.0;
            continue;
        }
        let alpha = if ck[k] > 0.0 { -nrm } else { nrm };
        let mut v: Vec<f64> = ck[k..].to_vec();
        v[0] -= alpha;
        let vn = dot4(&v, &v);
        if vn == 0.0 {
            rdiag[k] = alpha;
            continue;
        }
        let refl = |col: &mut [f64]| {
            let s = dot4(&col[k..], &v) * 2.0 / vn;
            for (c, vi) in col[k..].iter_mut().zip(&v) {
                *c -= s * vi;
            }
        };
        refl(ck);
        // 残りの列は互いに独立（列ごとに同じ計算なので並列でも同じ値）
        if par {
            rest.par_chunks_mut(m).for_each(scoped(refl));
        } else {
            rest.chunks_mut(m).for_each(refl);
        }
        refl(&mut y);
        rdiag[k] = ck[k];
    }
    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let s: f64 = (k + 1..n).map(|j| q[j * m + k] * x[j]).sum();
        x[k] = if rdiag[k] != 0.0 {
            (y[k] - s) / rdiag[k]
        } else {
            0.0
        };
    }
    x
}

/// Zernike の当てはめで点を分ける塊の大きさ
const ZCH: usize = 256;

/// 列優先の塊 a（列の長さ ld は 4 の倍数、列の数 np は 4 の倍数で、足りない所は 0）の AᵀA の下三角
/// （i < n、j ≤ i）を g（n×n、行優先）に書く。各要素は、点を 4 つずつに分けた 4 本の和を最後に
/// (s0 + s1) + (s2 + s3) で足す（和の順は列の組の大きさや CPU によらず同じ）。列の組（TU 列 × TW 列）ごとに
/// 足すので、読み込み 1 回あたりの積和が多い
#[inline(always)]
#[cfg_attr(
    all(target_arch = "wasm32", target_feature = "simd128"),
    allow(dead_code) // simd128 では gram_block_simd128（試験で照合に使う）
)]
fn gram_tiles<const TU: usize, const TW: usize>(
    a: &[f64],
    ld: usize,
    np: usize,
    n: usize,
    g: &mut [f64],
) {
    const L: usize = 4;
    let h = ld / L;
    let col = |k: usize| -> &[f64] { &a[k * ld..(k + 1) * ld] };
    for bi in (0..np).step_by(TU) {
        let ci: [&[f64]; TU] = std::array::from_fn(|k| col(bi + k));
        for bj in (0..bi + TU).step_by(TW) {
            let cj: [&[f64]; TW] = std::array::from_fn(|k| col(bj + k));
            let mut acc = [[[0.0f64; L]; TW]; TU];
            for q in 0..h {
                for u in 0..TU {
                    let x = &ci[u][q * L..q * L + L];
                    for w in 0..TW {
                        let y = &cj[w][q * L..q * L + L];
                        for l in 0..L {
                            acc[u][w][l] += x[l] * y[l];
                        }
                    }
                }
            }
            for (u, au) in acc.iter().enumerate() {
                let i = bi + u;
                if i >= n {
                    break;
                }
                for (w, s) in au.iter().enumerate() {
                    let j = bj + w;
                    if j > i {
                        break;
                    }
                    g[i * n + j] = (s[0] + s[1]) + (s[2] + s[3]);
                }
            }
        }
    }
}

/// [`gram_tiles`] と同じ計算を AVX の命令で（4 列 × 2 列の組、積と和は別の命令で FMA は使わないので値は同じ）
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn gram_block_avx(a: &[f64], ld: usize, np: usize, n: usize, g: &mut [f64]) {
    use std::arch::x86_64::*;
    assert!(ld.is_multiple_of(4) && np.is_multiple_of(4) && a.len() >= ld * np && g.len() >= n * n);
    let base = a.as_ptr();
    for bi in (0..np).step_by(4) {
        for bj in (0..bi + 4).step_by(2) {
            let mut acc = [_mm256_setzero_pd(); 8];
            let pi = [0, 1, 2, 3].map(|k| base.add((bi + k) * ld));
            let pj = [0, 1].map(|k| base.add((bj + k) * ld));
            for p in (0..ld).step_by(4) {
                // 安全: 列 bi..bi+4、bj..bj+2 は np 列の中、p + 4 ≤ ld（上の assert）
                let y0 = _mm256_loadu_pd(pj[0].add(p));
                let y1 = _mm256_loadu_pd(pj[1].add(p));
                for u in 0..4 {
                    let x = _mm256_loadu_pd(pi[u].add(p));
                    acc[2 * u] = _mm256_add_pd(acc[2 * u], _mm256_mul_pd(x, y0));
                    acc[2 * u + 1] = _mm256_add_pd(acc[2 * u + 1], _mm256_mul_pd(x, y1));
                }
            }
            for u in 0..4 {
                let i = bi + u;
                if i >= n {
                    break;
                }
                for w in 0..2 {
                    let j = bj + w;
                    if j > i {
                        break;
                    }
                    let mut s = [0.0f64; 4];
                    _mm256_storeu_pd(s.as_mut_ptr(), acc[2 * u + w]);
                    g[i * n + j] = (s[0] + s[1]) + (s[2] + s[3]);
                }
            }
        }
    }
}

/// [`gram_tiles`] と同じ計算を wasm の simd128 で（4 列 × 2 列の組。128 ビットは 2 点なので、点を 4 つずつに
/// 分けた 4 本の和を 2 つのレジスタに持つ。積と和は別の命令なので値は同じ）
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn gram_block_simd128(a: &[f64], ld: usize, np: usize, n: usize, g: &mut [f64]) {
    use std::arch::wasm32::*;
    assert!(ld.is_multiple_of(4) && np.is_multiple_of(4) && a.len() >= ld * np && g.len() >= n * n);
    let base = a.as_ptr();
    // 安全: 列 k の p..p+2 は k < np、p + 2 ≤ ld なので a の中（上の assert）。v128_load は整列を求めない
    let ld2 = |k: usize, p: usize| unsafe { v128_load(base.add(k * ld + p) as *const v128) };
    for bi in (0..np).step_by(4) {
        for bj in (0..bi + 4).step_by(2) {
            // acc[u][w][h]: 点 4 つの組の中の 2h, 2h+1 番目
            let mut acc = [[[f64x2_splat(0.0); 2]; 2]; 4];
            for p in (0..ld).step_by(4) {
                let y = [0, 1].map(|w| [ld2(bj + w, p), ld2(bj + w, p + 2)]);
                for (u, au) in acc.iter_mut().enumerate() {
                    let x = [ld2(bi + u, p), ld2(bi + u, p + 2)];
                    for (w, aw) in au.iter_mut().enumerate() {
                        for h in 0..2 {
                            aw[h] = f64x2_add(aw[h], f64x2_mul(x[h], y[w][h]));
                        }
                    }
                }
            }
            for (u, au) in acc.iter().enumerate() {
                let i = bi + u;
                if i >= n {
                    break;
                }
                for (w, aw) in au.iter().enumerate() {
                    let j = bj + w;
                    if j > i {
                        break;
                    }
                    let (s0, s1) = (
                        f64x2_extract_lane::<0>(aw[0]),
                        f64x2_extract_lane::<1>(aw[0]),
                    );
                    let (s2, s3) = (
                        f64x2_extract_lane::<0>(aw[1]),
                        f64x2_extract_lane::<1>(aw[1]),
                    );
                    g[i * n + j] = (s0 + s1) + (s2 + s3);
                }
            }
        }
    }
}

/// [`gram_tiles`]。AVX があれば 256 ビットの、wasm の simd128 なら 128 ビットのレジスタで
/// （演算と順は同じなので値は同じ）
fn gram_block(a: &[f64], ld: usize, np: usize, n: usize, g: &mut [f64]) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    gram_block_simd128(a, ld, np, n, g);
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx") {
                // 安全: AVX があることを上で確かめた
                return unsafe { gram_block_avx(a, ld, np, n, g) };
            }
        }
        gram_tiles::<2, 2>(a, ld, np, n, g)
    }
}

/// Zernike の当てはめの 1 回分の和: 点の塊（ZCH 点）ごとに項を列優先で求め、残差 r = v − A c（c が None なら
/// r = v）と Aᵀr、c が None なら AᵀA も足す。塊の分け方と足す順は固定なので、値はスレッド数によらない。
/// A は塊の中だけに置く（全体を持って使い回すより、ページの確保がない分だけ速い。項を求める時間は小さい）
fn zern_pass(
    basis: &ZernBasis,
    px: &[f64],
    py: &[f64],
    v: &[f64],
    c: Option<&[f64]>,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    use rayon::prelude::*;
    let n = basis.len();
    // 列の数と長さを 4 の倍数にして、足りない所は 0（積和に効かない）
    let np = n.div_ceil(4) * 4;
    let gram = c.is_none();
    let part = |k: usize| {
        let lo = k * ZCH;
        let hi = (lo + ZCH).min(v.len());
        let len = hi - lo;
        let ld = len.div_ceil(4) * 4;
        let mut a = vec![0.0; ld * np];
        basis.eval_cols(&px[lo..hi], &py[lo..hi], &mut a, ld);
        let mut r = v[lo..hi].to_vec();
        if let Some(c) = c {
            for (col, &cj) in a.chunks_exact(ld).zip(c) {
                r.iter_mut().zip(col).for_each(|(ri, z)| *ri -= z * cj);
            }
        }
        let atr: Vec<f64> = a
            .chunks_exact(ld)
            .take(n)
            .map(|col| dot4(col, &r))
            .collect();
        let mut g = Vec::new();
        if gram {
            g = vec![0.0; n * n];
            gram_block(&a, ld, np, n, &mut g);
        }
        (g, atr, r)
    };
    let nch = v.len().div_ceil(ZCH);
    let parts: Vec<_> = if crate::par::sequential() {
        (0..nch).map(part).collect()
    } else {
        (0..nch).into_par_iter().map(scoped(part)).collect()
    };
    let mut g = if gram { vec![0.0; n * n] } else { Vec::new() };
    let mut atr = vec![0.0; n];
    let mut r = Vec::with_capacity(v.len());
    for (pg, pa, pr) in parts {
        g.iter_mut().zip(&pg).for_each(|(x, y)| *x += y);
        atr.iter_mut().zip(&pa).for_each(|(x, y)| *x += y);
        r.extend(pr);
    }
    if gram {
        for i in 0..n {
            for j in 0..i {
                g[j * n + i] = g[i * n + j];
            }
        }
    }
    (g, atr, r)
}

/// 対称正定値 g（n×n）の Cholesky 分解 L（下三角、行優先）
fn cholesky(g: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = (0..j).map(|k| l[i * n + k] * l[j * n + k]).sum();
            if i == j {
                let d = g[i * n + i] - s;
                if !(d > 1e-14 * g[i * n + i].abs()) {
                    return None;
                }
                l[i * n + i] = d.sqrt();
            } else {
                l[i * n + j] = (g[i * n + j] - s) / l[j * n + j];
            }
        }
    }
    Some(l)
}

fn chol_solve(l: &[f64], n: usize, b: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; n];
    for i in 0..n {
        let s: f64 = (0..i).map(|k| l[i * n + k] * y[k]).sum();
        y[i] = (b[i] - s) / l[i * n + i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|k| l[k * n + i] * x[k]).sum();
        x[i] = (y[i] - s) / l[i * n + i];
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
    if nterms == 0 || (kind == ZernKind::Fringe && nterms > 37) || nterms > 2016 {
        invalid!("項数 {nterms}（Fringe は 1〜37、Standard は 1〜2016）");
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
    let basis = ZernBasis::new(kind, &nm);
    let (mut qx, mut qy, mut v) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..px.len() {
        if r.ok(i) && r.opd[i].is_finite() {
            qx.push(px[i]);
            qy.push(py[i]);
            v.push(r.opd[i]);
        }
    }
    if v.len() < nm.len().max(3) {
        invalid!("有効な光線 {} 本が項数 {} より少ない", v.len(), nm.len());
    }
    // 正規方程式 (AᵀA) c = Aᵀv を点の塊ごとに足し（塊の分け方と足す順は固定なので、値はスレッド数によらない）、
    // Cholesky で解いて残差で 1 回補正する（Zernike の項は円の上でほぼ直交なので条件数は小さい）
    let nt = nm.len();
    let m = v.len();
    let (g, atv, _) = zern_pass(&basis, &qx, &qy, &v, None);
    // 正規方程式は条件数の 2 乗で効くので、Cholesky が通らない・補正が大きい（項数が格子に比べて多いなど）
    // ときは QR で解き直す
    let normal = cholesky(&g, nt).and_then(|ch| {
        let mut c = chol_solve(&ch, nt, &atv);
        let (_, atr, _) = zern_pass(&basis, &qx, &qy, &v, Some(&c));
        let dc = chol_solve(&ch, nt, &atr);
        let cn = c.iter().map(|x| x * x).sum::<f64>().sqrt();
        let dn = dc.iter().map(|x| x * x).sum::<f64>().sqrt();
        c.iter_mut().zip(&dc).for_each(|(x, d)| *x += d);
        (dn.is_finite() && dn <= 1e-6 * cn.max(1e-300)).then_some(c)
    });
    let c = match normal {
        Some(c) => c,
        None => {
            let mut q = vec![0.0; m * nt];
            let mut row = vec![0.0; nt];
            for (i, (&x, &y)) in qx.iter().zip(&qy).enumerate() {
                basis.eval(x, y, &mut row, 1);
                for (j, &z) in row.iter().enumerate() {
                    q[j * m + i] = z;
                }
            }
            lstsq_cols(&mut q, m, &v)
        }
    };
    let (_, _, res) = zern_pass(&basis, &qx, &qy, &v, Some(&c));
    // 重心基準: 1・px・py の最小二乗（QR、列優先）
    let mut q3: Vec<f64> = std::iter::repeat_n(1.0, m)
        .chain(qx.iter().copied())
        .chain(qy.iter().copied())
        .collect();
    let t = lstsq_cols(&mut q3, m, &v);
    let vt: Vec<f64> = qx
        .iter()
        .zip(&qy)
        .zip(&v)
        .map(|((&x, &y), vi)| vi - (t[0] + x * t[1] + y * t[2]))
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
    fn radial(n: usize, m: usize, r: f64) -> f64 {
        (0..=(n - m) / 2)
            .map(|k| {
                let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
                sgn * fact(n - k) / (fact(k) * fact((n + m) / 2 - k) * fact((n - m) / 2 - k))
                    * r.powi((n - 2 * k) as i32)
            })
            .sum()
    }

    #[test]
    fn zernike_terms_match_the_definition() {
        // 漸化式・前計算の係数による項が、階乗と atan2 による定義どおりの値と一致する
        for kind in [ZernKind::Standard, ZernKind::Fringe] {
            let nm = zern_nm(kind, if kind == ZernKind::Fringe { 37 } else { 120 });
            for &(px, py) in &[
                (0.0, 0.0),
                (0.3, -0.2),
                (-0.7, 0.7),
                (1.0, 0.0),
                (0.05, 0.99),
            ] {
                let z = zernike_row(px, py, kind, &nm);
                let (r, a) = (f64::hypot(px, py), f64::atan2(py, px));
                for (k, &(n, m, c)) in nm.iter().enumerate() {
                    let ang = if c {
                        (m as f64 * a).cos()
                    } else {
                        (m as f64 * a).sin()
                    };
                    let nrm = if kind == ZernKind::Standard {
                        (((n + 1) * if m > 0 { 2 } else { 1 }) as f64).sqrt()
                    } else {
                        1.0
                    };
                    let want = radial(n, m, r) * ang * nrm;
                    assert!(
                        (z[k] - want).abs() < 1e-9,
                        "{kind:?} {n} {m} {px} {py}: {} {want}",
                        z[k]
                    );
                }
            }
        }
    }

    #[test]
    fn pupil_jacobian_falls_back_when_the_rim_is_vignetted() {
        // 絞りの面に絞りの半径の 0.9 倍の開口を置くと瞳の縁の光線がけられる。内側の点の差で見積もった J は
        // 縁の差の J と瞳の収差の分しか違わず、MTF も計算できる
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        for k in 0..s.fields.len() {
            let f = FieldSel::Index(k);
            let j0 = pupil_jacobian(&s, f, s.primary).unwrap();
            // けられないときもフォールバックの見積もりは近い
            let ji = pupil_jacobian_inner(&s, f, s.primary).unwrap();
            let mut v = s.clone();
            let st = v.stop();
            v.surfaces[st].ap = crate::surface::Aperture::Circular {
                min: 0.0,
                max: 0.9 * v.stop_radius().0.abs(),
                dx: 0.0,
                dy: 0.0,
            };
            v.changed().unwrap();
            let r = v
                .try_trace(
                    f,
                    &[1.0, -1.0, 0.0, 0.0],
                    &[0.0, 0.0, 1.0, -1.0],
                    Some(v.primary),
                    None,
                    false,
                )
                .unwrap();
            assert!((0..4).any(|i| !r.ok(i)), "field {k}: 縁がけられていない");
            let j1 = pupil_jacobian(&v, f, v.primary).unwrap();
            for (a, b) in [(j0, j1), (j0, ji)] {
                for r in 0..2 {
                    for c in 0..2 {
                        let tol = 0.02 * j0[r][r].abs().max(j0[c][c].abs());
                        assert!((a[r][c] - b[r][c]).abs() < tol, "field {k}: {a:?} {b:?}");
                    }
                }
            }
            assert!(
                mtf(&v, f, Waves::One(v.primary), Some(16), &[0.0, 20.0]).is_ok(),
                "field {k}"
            );
        }
    }

    #[test]
    fn column_basis_and_gram_match_the_row_forms() {
        // eval_cols は eval とビットまで同じ。AᵀA は AVX・simd128 の有無（gram_block と gram_tiles）で同じで、
        // 1 点ずつの外積の和と丸めの範囲で一致する
        let mut st = 7u64;
        let mut rnd = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        for (kind, nt) in [
            (ZernKind::Fringe, 37),
            (ZernKind::Standard, 45),
            (ZernKind::Standard, 7),
        ] {
            let nm = zern_nm(kind, nt);
            let b = ZernBasis::new(kind, &nm);
            let len: usize = 203;
            let (px, py): (Vec<f64>, Vec<f64>) =
                (0..len).map(|_| (rnd() * 0.7, rnd() * 0.7)).unzip();
            let ld = len.div_ceil(4) * 4;
            let np = nt.div_ceil(4) * 4;
            let mut a = vec![0.0; ld * np];
            b.eval_cols(&px, &py, &mut a, ld);
            let mut row = vec![0.0; nt];
            let mut want = vec![0.0; nt * nt];
            for i in 0..len {
                b.eval(px[i], py[i], &mut row, 1);
                for j in 0..nt {
                    assert_eq!(
                        a[j * ld + i].to_bits(),
                        row[j].to_bits(),
                        "{kind:?} {i} {j}"
                    );
                    for k in 0..=j {
                        want[j * nt + k] += row[j] * row[k];
                    }
                }
            }
            let (mut g1, mut g2) = (vec![0.0; nt * nt], vec![0.0; nt * nt]);
            gram_block(&a, ld, np, nt, &mut g1);
            gram_tiles::<2, 2>(&a, ld, np, nt, &mut g2);
            for j in 0..nt {
                for k in 0..=j {
                    let (x, y, w) = (g1[j * nt + k], g2[j * nt + k], want[j * nt + k]);
                    assert_eq!(x.to_bits(), y.to_bits(), "{kind:?} {j} {k}");
                    assert!(
                        (x - w).abs() <= 1e-13 * (1.0 + w.abs()),
                        "{kind:?} {j} {k}: {x} {w}"
                    );
                }
            }
        }
    }

    #[test]
    fn zernike_fit_is_the_same_for_any_thread_count() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/C0/system.zmx"), &opt).unwrap();
        let f = FieldSel::Index(1);
        let run = |t| {
            crate::with_threads(Some(t), || {
                zernike(&s, f, None, ZernKind::Fringe, 37, 96, false).unwrap()
            })
        };
        let (a, b) = (run(1), run(3));
        for (x, y) in a.coef.iter().zip(&b.coef) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
        assert_eq!(a.rms_fit.to_bits(), b.rms_fit.to_bits());
    }

    #[test]
    fn many_terms_on_a_coarse_grid_fall_back_to_qr() {
        // 格子 32・Standard 496 項（次数 30）は正規方程式では解けない（条件数が大きい）ので QR で解く。
        // 行列を zernike_row で作って lstsq で解いた値と比べる
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let f = FieldSel::Index(1);
        let (nt, n) = (496, 32);
        let z = zernike(&s, f, None, ZernKind::Standard, nt, n, false).unwrap();
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
        let o = opd(&s, f, &px, &py, None, false).unwrap();
        let nm = zern_nm(ZernKind::Standard, nt);
        let (mut a, mut v) = (Vec::new(), Vec::new());
        for i in 0..px.len() {
            if o.ok(i) && o.opd[i].is_finite() {
                a.push(zernike_row(px[i], py[i], ZernKind::Standard, &nm));
                v.push(o.opd[i]);
            }
        }
        let c = lstsq(&a, &v);
        let e = c
            .iter()
            .zip(&z.coef)
            .map(|(p, q)| (p - q).abs())
            .fold(0.0, f64::max);
        assert!(e < 1e-9, "{e}");
    }

    #[test]
    fn lstsq_exact() {
        let a = vec![vec![1.0, 0.0], vec![1.0, 1.0], vec![1.0, 2.0]];
        let x = lstsq(&a, &[1.0, 3.0, 5.0]);
        assert!((x[0] - 1.0).abs() < 1e-14 && (x[1] - 2.0).abs() < 1e-14);
    }

    #[test]
    fn mtf_of_a_perfect_lens_is_the_diffraction_limit() {
        // 無限遠の物体、絞り径 10 mm、焦点距離 100 mm の理想レンズ（F/10）、像は焦点
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
                   XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 100\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::zmx::parse_text(txt).unwrap();
        let s = crate::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let nc = 1.0 / (0.55e-3 * 10.0);
        let freqs: Vec<f64> = (0..10).map(|i| nc * i as f64 / 10.0).collect();
        let m = mtf(&s, FieldSel::Index(0), Waves::All, Some(128), &freqs).unwrap();
        // 遮断周波数は 2 sinθ / λ（sinθ = 5/√(100² + 5²)、F/# からの 1/(λF) より 0.12 % 低い）
        let sin = 5.0 / (100.0f64 * 100.0 + 25.0).sqrt();
        let c = 2.0 * sin / 0.55e-3;
        assert!(
            (m.cutoff.0 - c).abs() / c < 1e-9 && (m.cutoff.1 - c).abs() / c < 1e-9,
            "{:?}",
            m.cutoff
        );
        for (k, &f) in freqs.iter().enumerate() {
            let v: f64 = f / c;
            let ideal = 2.0 / std::f64::consts::PI * (v.acos() - v * (1.0 - v * v).sqrt());
            assert!((m.tan[k] - ideal).abs() < 1e-4, "{v} {} {ideal}", m.tan[k]);
            assert!((m.sag[k] - ideal).abs() < 1e-4, "{v} {} {ideal}", m.sag[k]);
        }
    }

    #[test]
    fn mtf_points_per_shift_match_the_full_count() {
        // 理想レンズ（F/10）の像面を 20 mm ずらす（焦点ずれ約 45 波、自動の n ≈ 290）。周波数ごとに点数を減らした
        // 自動の値と、同じ n で減らさない値は同じ。片方の向きだけ求めても値は同じ
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
                   XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 100\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::zmx::parse_text(txt).unwrap();
        let s = crate::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let s = crate::sweep::defocus(&s, 20.0).unwrap();
        let freqs = [2.0, 10.0, 60.0];
        for f in 0..2 {
            let fs = FieldSel::Index(f);
            let n = mtf_points(opd_slope_max(&s, fs, 0).unwrap());
            assert!(n >= SHIFT_MIN_N, "{n}");
            let a = mtf(&s, fs, Waves::All, None, &freqs).unwrap();
            let b = mtf(&s, fs, Waves::All, Some(n), &freqs).unwrap();
            let t = mtf_only(&s, fs, Waves::All, None, &freqs, MtfMethod::Auto, Some(1)).unwrap();
            let g = mtf_only(&s, fs, Waves::All, None, &freqs, MtfMethod::Auto, Some(0)).unwrap();
            for k in 0..freqs.len() {
                assert!(
                    (a.tan[k] - b.tan[k]).abs() < 1e-8,
                    "{f} {k} {} {}",
                    a.tan[k],
                    b.tan[k]
                );
                assert!(
                    (a.sag[k] - b.sag[k]).abs() < 1e-8,
                    "{f} {k} {} {}",
                    a.sag[k],
                    b.sag[k]
                );
                assert_eq!(t.tan[k], a.tan[k]);
                assert_eq!(g.sag[k], a.sag[k]);
            }
        }
    }

    #[test]
    fn a_point_at_the_front_focus_leaves_a_plane_wave() {
        // 物体点が理想レンズの前側焦点: 像空間は平行光（像は無限遠）。OPD は 0（以前は理想レンズの位相が NaN）
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
                   XFLN 0 0\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 100\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 50\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::zmx::parse_text(txt).unwrap();
        let s = crate::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let (px, py) = pupil_grid(9);
        for f in 0..2 {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
                .unwrap();
            let l: Vec<f64> = (0..px.len())
                .filter(|&i| t.ok(i))
                .map(|i| {
                    // 平行光の波面（主光線に垂直な面）までの光路
                    let k = t.rays[0].lmn;
                    let d = crate::math::sub(t.rays[i].xyz, t.rays[0].xyz);
                    t.opl_os[i] + t.rays[i].lens_opd - crate::math::dot(d, k)
                })
                .collect();
            let (lo, hi) = l
                .iter()
                .fold((f64::MAX, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
            assert!(hi - lo < 1e-9, "視野 {f}: {lo} {hi}");
        }
    }
}
