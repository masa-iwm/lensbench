//! 瞳の標本と OPD（射出瞳の参照球）。解析（lensbench-analysis）と、core の中のソルブ・瞳関数が使う。
//!
//! OPD は OpticStudio と同じ（ROPD「射出瞳」）: 参照球の中心は主波長の主光線の像点、主光線が射出瞳面
//! （像面から EXPP）と交わる点を通る。各光線を像面から参照球まで戻し、(主光線の光路 − 光線の光路) を
//! その光線の波長の波数で表す（主光線はその波長のもの）。ROPD「無限遠」は主光線に垂直な平面。

use crate::error::Result;
use crate::invalid;
use crate::math::*;
use crate::pupil::{PupilGrid, Rim};
use crate::system::{Cols, FieldSel, RayBuf, System, Trace};

/// n×n の格子のうち単位円内の点（[`PupilGrid::Linear`] の [`Rim::Padded`]）
pub fn pupil_grid(n: usize) -> (Vec<f64>, Vec<f64>) {
    let s = PupilGrid::Linear {
        n,
        rim: Rim::Padded,
    }
    .samples();
    (s.px, s.py)
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

/// 瞳のガウス求積（[`PupilGrid::Gauss`]）→ px, py, 重み（和 1）
pub fn pupil_gauss(rings: usize, arms: Option<usize>) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let s = PupilGrid::Gauss { rings, arms }.samples();
    (s.px, s.py, s.wt)
}

/// OpticStudio 26.1 の RSCE・RSCH・RWCE・RWCH の瞳の求積 → px, py, 重み: [`pupil_gauss`] の腕を 2 rings 本にし、90° 回す
/// （腕の角度 90° + (a + 0.5)·180°/rings）。Double Gauss 28 の Ring 2〜6・6 視野・各波長で OpticStudio と相対 1e-10 で一致
/// する（Ring 1 の Hx = 0・Hy ≠ 0 の視野だけは、腕の数・向きを変えても OpticStudio の値にならない）
pub fn pupil_gauss_os(rings: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (px, py, w) = pupil_gauss(rings, Some(2 * rings));
    // 角度 θ + 90°: (cos θ, sin θ) → (−sin θ, cos θ)
    let rx = py.iter().map(|y| -y).collect();
    (rx, px, w)
}

/// OPD [波] の結果（光線と一緒に）
#[non_exhaustive]
pub struct Opd {
    /// 光線の列（位置・方向・光路長・理想レンズの位相と状態・けられ、[`Cols::WAVE`]）
    pub rays: RayBuf,
    pub opd: Vec<f64>,
}

impl Opd {
    pub fn ok(&self, i: usize) -> bool {
        self.rays.ok(i)
    }
}

/// アフォーカル像空間の基準の平面の点: 射出瞳の位置で主光線に垂直な平面（OpticStudio の平面波基準）。
/// 射出瞳が無限遠なら像面の主光線の点で（平面波なので面の位置は定数の違いだけ）
#[doc(hidden)]
pub fn to_ref_plane_point(expp: f64, pc: V3, kc: V3) -> V3 {
    if crate::pupil::exit_pupil_near(expp) {
        add(pc, scale(kc, (expp - pc[2]) / kc[2]))
    } else {
        pc
    }
}

#[doc(hidden)]
pub fn to_ref(s: &System, w: usize, pc: V3, kc: V3, q: V3, k: V3, opl: f64) -> f64 {
    ref_opl(s, w, pc, kc)(q, k, opl)
}

/// [`to_ref`] を同じ参照の主光線 (pc, kc) で何本もの光線に使う形: 基準の面と像空間の屈折率を 1 度だけ求め、
/// 光線 (q, k, opl) ごとの換算を返す（光線ごとに求め直すと、面の数だけの走査が光線の本数ぶん掛かる）。値は
/// [`to_ref`] と同じビット
#[doc(hidden)]
pub fn ref_opl(s: &System, w: usize, pc: V3, kc: V3) -> impl Fn(V3, V3, f64) -> f64 {
    let r = crate::pupil::RefSurface::of(s, pc, kc);
    let ni = s.image_index(w);
    move |q, k, opl| r.opl(ni, q, k, opl)
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
    Ok(opd_chief(s, field, px, py, w, ref_primary)?.0)
}

/// [`opd`] と、参照球の中心に使った主光線の追跡（ref_primary なら主波長、でなければ波長 w）
#[doc(hidden)]
pub fn opd_chief(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: Option<usize>,
    ref_primary: bool,
) -> Result<(Opd, Trace)> {
    crate::pupil::check_opd_reference(s)?;
    let w = w.unwrap_or(s.primary);
    let mut r = RayBuf::with_cols(Cols::WAVE);
    s.try_trace_into(field, px, py, Some(w), None, &mut r)?;
    let rc = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
    let rp = if w == s.primary || !ref_primary {
        rc.clone()
    } else {
        s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?
    };
    for (t, wi) in [(&rc, w), (&rp, s.primary)] {
        let c = &t.rays[0];
        if let Some(surf) = c.status.surf() {
            let why = c.status.why();
            invalid!(
                "the chief ray (wavelength {}) doesn't reach the image (surface {}, {why}); no OPD reference",
                wi + 1,
                surf
            );
        }
    }
    let (pc, kc) = (rp.rays[0].xyz, rp.rays[0].lmn);
    let c = &rc.rays[0];
    let to = ref_opl(s, w, pc, kc);
    let lc = to(c.xyz, c.lmn, c.opl) + c.lens_opd;
    let wl = s.wave(w).mm();
    let opd = (0..r.len())
        .map(|i| (lc - (to(r.xyz[i], r.lmn[i], r.opl[i]) + r.lens_opd[i])) / wl)
        .collect();
    Ok((Opd { rays: r, opd }, rp))
}
