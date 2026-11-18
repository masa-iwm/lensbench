//! 実光線の追跡の部品（屈折・回折・Fresnel の光路長・理想レンズ・光線の状態と記録）。偏光の面ごとの処理は
//! trace/pol.rs、開口の判定は trace/aperture.rs、面の形・交点・法線は [`crate::shape`]、面の働きは [`crate::action`]。
//! 追跡のカーネル本体は [`crate::batch`]（1 本ずつとまとめ追跡を 1 つのジェネリックな実装で）。

mod aperture;
mod pol;
pub(crate) mod uniaxial;

#[doc(hidden)]
pub use aperture::{aperture_margin, aperture_out};
pub(crate) use pol::{cmv, pol_step, rotate_min, CryState, CNAN3, CV3};
pub use pol::{CryKind, PolKind, PolSurf};

use crate::math::*;
use crate::system::System;

pub use crate::shape::{powi, sag, sag_toroidal};

/// 光線の追跡が止まったか・その理由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RayStatus {
    /// 最後の面まで届いた（けられたかは [`RayOut::vig`]）
    #[default]
    Ok,
    /// 面 surf に当たらなかった（交点がない・面の定義域の外・媒質の外・数値が有限でない）
    Missed { surf: usize },
    /// 面 surf で全反射した
    Tir { surf: usize },
    /// 光線のエイミング（瞳座標・絞りの点を通る光線の解き）に失敗した。stop は絞りの面
    AimFailed { stop: usize },
}

impl RayStatus {
    /// 最後の面まで届いた
    #[inline]
    pub fn is_ok(self) -> bool {
        self == RayStatus::Ok
    }
    /// 止まった面（エイミングの失敗は絞りの面）。届いたなら None
    #[inline]
    pub fn surf(self) -> Option<usize> {
        match self {
            RayStatus::Ok => None,
            RayStatus::Missed { surf } | RayStatus::Tir { surf } => Some(surf),
            RayStatus::AimFailed { stop } => Some(stop),
        }
    }
    /// 数で表した止まった面（届いたなら 0）。Python・JSON の `err` の欄（前の約束と同じ数）
    #[inline]
    pub fn code(self) -> usize {
        self.surf().unwrap_or(0)
    }
    /// 誤りの文に入れる理由（英語、訳は lang/ja.tsv）
    pub fn why(self) -> &'static str {
        match self {
            RayStatus::Ok => "ok",
            RayStatus::Missed { .. } => "missed",
            RayStatus::Tir { .. } => "total reflection",
            RayStatus::AimFailed { .. } => "ray aiming failed",
        }
    }
    /// 理由の名前（"ok"・"missed"・"tir"・"aim_failed"）
    pub fn name(self) -> &'static str {
        match self {
            RayStatus::Ok => "ok",
            RayStatus::Missed { .. } => "missed",
            RayStatus::Tir { .. } => "tir",
            RayStatus::AimFailed { .. } => "aim_failed",
        }
    }
}

/// 追跡した光線の本数を、届いたか・止まった理由で数えたもの（解析の結果で、何本がなぜ除かれたかを示す）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RayCounts {
    /// 追跡した本数
    pub total: usize,
    /// 最後の面まで届き、けられなかった本数
    pub ok: usize,
    /// 面の開口でけられた本数
    pub vignetted: usize,
    /// 面に当たらなかった本数（[`RayStatus::Missed`]）
    pub missed: usize,
    /// 全反射した本数（[`RayStatus::Tir`]）
    pub tir: usize,
    /// エイミングに失敗した本数（[`RayStatus::AimFailed`]）
    pub aim_failed: usize,
}

impl RayCounts {
    /// 1 本を数える（vig はけられた面、0 ならけられていない）
    pub fn add(&mut self, st: RayStatus, vig: usize) {
        self.total += 1;
        match st {
            RayStatus::Ok if vig == 0 => self.ok += 1,
            RayStatus::Ok => self.vignetted += 1,
            RayStatus::Missed { .. } => self.missed += 1,
            RayStatus::Tir { .. } => self.tir += 1,
            RayStatus::AimFailed { .. } => self.aim_failed += 1,
        }
    }

    /// 本数を足し合わせる
    pub fn merge(&mut self, o: &RayCounts) {
        self.total += o.total;
        self.ok += o.ok;
        self.vignetted += o.vignetted;
        self.missed += o.missed;
        self.tir += o.tir;
        self.aim_failed += o.aim_failed;
    }

    /// 除かれた本数（届かなかった・けられた）
    pub fn lost(&self) -> usize {
        self.total - self.ok
    }
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RayOut {
    /// 最後の面の局所座標での位置・方向（方向は物理的な向き）
    pub xyz: V3,
    pub lmn: V3,
    /// 幾何的な光路長（理想レンズの位相を含まない）
    pub opl: f64,
    /// 追跡が止まったか・その理由。止まった光線の位置・方向・光路長・記録は NaN
    pub status: RayStatus,
    /// けられた面（0 = けられていない）。けられても追跡は続く
    pub vig: usize,
    /// 大域の位置・方向
    pub p: V3,
    pub k: V3,
    /// 理想レンズの位相（OPD に足す）
    pub lens_opd: f64,
}

/// 面ごとの記録（追跡の内側で 1 本ずつ書く。結果は [`crate::system::RecSurf`] に並べ替える）
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RecEntry {
    pub surf: usize,
    pub xyz: V3,
    pub lmn: V3,
    pub opl: f64,
    pub p: V3,
}

/// 交点のニュートン法を止める刻みの大きさ: [`crate::tol::NEWTON_STEP_MIN`] か、距離 s・矢高 z の丸め（8 ulp）の大きい方。
/// 矢高が数十 mm の大きな面では q[2] − z の丸めが下限を超え、収束しても 60 反復まで回っていた
#[inline(always)]
pub(crate) fn newton_tol(s: f64, z: f64) -> f64 {
    (8.0 * f64::EPSILON * (s.abs() + z.abs())).max(crate::tol::NEWTON_STEP_MIN)
}

/// 追跡の内側で面ごとの記録を受け取る（k は無視しない面の通し番号）
pub(crate) trait RecSink {
    fn put(&mut self, k: usize, e: RecEntry);
    /// 通し番号 k の記録を受け取るか（受け取らない面は記録の値を作らない。既定は全面）
    #[inline(always)]
    fn wants(&self, _k: usize) -> bool {
        true
    }
}

impl RecSink for [RecEntry] {
    #[inline(always)]
    fn put(&mut self, k: usize, e: RecEntry) {
        self[k] = e;
    }
}

impl RecEntry {
    pub const NAN: RecEntry = RecEntry {
        surf: 0,
        xyz: NAN3,
        lmn: NAN3,
        opl: f64::NAN,
        p: NAN3,
    };
}

/// 方向 k を単位法線 nv の面で屈折する（屈折率 n1 → n2、mirror なら反射）。全反射なら None
#[inline]
fn bend(k: V3, nv: V3, n1: f64, n2: f64, mirror: bool) -> Option<V3> {
    let cosi = dot(k, nv);
    if mirror {
        return Some(sub(k, scale(nv, 2.0 * cosi)));
    }
    if n1 == n2 {
        return Some(k);
    }
    let mu = n1 / n2;
    let disc = 1.0 - mu * mu * (1.0 - cosi * cosi);
    if !(disc >= 0.0) {
        return None;
    }
    Some(add(
        scale(k, mu),
        scale(nv, cosi.signum() * disc.sqrt() - mu * cosi),
    ))
}

/// [`fresnel_opl`] の線分上の積分の点: 5 点 Gauss–Legendre を 16 区間（被積分関数は滑らか。区間を倍にしても
/// 1e-13 以下しか変わらない）。(線分上の位置 0..1, 重みの半分)。和を区間の数で割るのは足した後
const FRESNEL_NP: usize = 16;
const FRESNEL_NODES: [(f64, f64); 5 * FRESNEL_NP] = {
    const GX: [f64; 5] = [
        -0.906_179_845_938_664,
        -0.538_469_310_105_683,
        0.0,
        0.538_469_310_105_683,
        0.906_179_845_938_664,
    ];
    const GW: [f64; 5] = [
        0.236_926_885_056_189,
        0.478_628_670_499_366,
        0.568_888_888_888_889,
        0.478_628_670_499_366,
        0.236_926_885_056_189,
    ];
    const NP: usize = FRESNEL_NP;
    let mut out = [(0.0, 0.0); 5 * NP];
    let mut i = 0;
    while i < NP {
        let mut g = 0;
        while g < 5 {
            out[i * 5 + g] = ((i as f64 + 0.5 + 0.5 * GX[g]) / NP as f64, 0.5 * GW[g]);
            g += 1;
        }
        i += 1;
    }
    out
};

/// Fresnel・Cylinder Fresnel の面の光路長の足し分。基準点 a から交点 b への線分上の
/// ∫ (n2 k2 − n1 k1)·dr（k1 は入射の方向で一定、k2 は線分上の各点の傾きでの屈折の方向）。
/// 平面の上の段差の高さを位相に置き換えたもので、OpticStudio 26.1 と一致する（合成系 F1・C1 で 1e-10。
/// 基準点は同じ視野・波長の主光線の交点）。途中で全反射すれば None
pub(crate) fn fresnel_opl(
    fb: &crate::action::FresnelBend,
    mirror: bool,
    k1: V3,
    n1: f64,
    n2: f64,
    a: [f64; 2],
    b: [f64; 2],
) -> Option<f64> {
    let (ux, uy) = (b[0] - a[0], b[1] - a[1]);
    if ux == 0.0 && uy == 0.0 {
        return Some(0.0);
    }
    let mut t = 0.0;
    for &(f, w) in FRESNEL_NODES.iter() {
        let (x, y) = (a[0] + ux * f, a[1] + uy * f);
        let nv = normalize(fb.normal(x, y));
        let k2 = bend(k1, nv, n1, n2, mirror)?;
        t += w * ((n2 * k2[0] - n1 * k1[0]) * ux + (n2 * k2[1] - n1 * k1[1]) * uy);
    }
    let t = t / FRESNEL_NP as f64;
    t.is_finite().then_some(t)
}

/// 回折面（Binary 2）の方向の変化: 屈折の後の方向 k（屈折率 n）の接線成分に、位相の勾配 g = (λ/2π)∇Φ の
/// 接平面への射影を足す（n′k′_t = n k_t + g_t）。法線成分は向きを保って長さを合わせる。エバネッセントなら None
#[inline]
pub(crate) fn diffract(k: V3, nv: V3, n: f64, g: [f64; 2]) -> Option<V3> {
    let gv = [g[0], g[1], 0.0];
    let gt = sub(gv, scale(nv, dot(gv, nv)));
    let m = add(scale(k, n), gt);
    let mn = dot(k, nv);
    let tv = sub(m, scale(nv, dot(m, nv)));
    let q = n * n - dot(tv, tv);
    if !(q >= 0.0) {
        return None;
    }
    Some(scale(add(tv, scale(nv, mn.signum() * q.sqrt())), 1.0 / n))
}

const NAN3: V3 = [f64::NAN; 3];

/// 面ごとの空間の進む向き（面 0 から +1、鏡の面の後で反転）。σ[j] は面 j に向かう空間の向き
pub(crate) fn space_signs(sys: &System) -> Vec<f64> {
    let mut out = Vec::with_capacity(sys.surfaces.len());
    let mut s = 1.0;
    for d in &sys.surfaces {
        out.push(s);
        if d.is_mirror() && !d.ignore && !d.is_coord_break() {
            s = -s;
        }
    }
    out
}

/// 理想レンズ（焦点距離 f の薄肉、z = 0 面）: 新しい方向と OPD 用の位相
pub(crate) fn paraxial(f: f64, pl: V3, kl: V3, z_obj: f64) -> (V3, f64) {
    let sz = kl[2].signum();
    let t = [kl[0] / kl[2].abs(), kl[1] / kl[2].abs()];
    let r = [pl[0], pl[1]];
    let t2 = [t[0] - r[0] / f, t[1] - r[1] / f];
    let nn = (1.0 + t2[0] * t2[0] + t2[1] * t2[1]).sqrt();
    let k2 = [t2[0] / nn, t2[1] / nn, sz / nn];
    let rr = [r[0], r[1], 0.0];
    if crate::tol::infinite_dist(z_obj) {
        let ff = [f * t[0], f * t[1], f];
        // 入ってくる平行光の面内の位相は実際の方向の k·r（光が −z へ進むときも x・y の成分はそのまま）
        let kin = [kl[0], kl[1], kl[2].abs()];
        return (
            k2,
            f.signum() * (norm(ff) - norm(sub(ff, rr)) - dot(kin, rr)),
        );
    }
    let p = [r[0] + t[0] * z_obj, r[1] + t[1] * z_obj, z_obj];
    let zi = 1.0 / (1.0 / f + 1.0 / z_obj);
    let s_in = (-z_obj).signum() * norm(sub(rr, p));
    let s_in0 = (-z_obj).signum() * norm(p);
    if crate::tol::infinite_dist(zi) {
        // 物体が前側焦点にある（出ていく光は平行光）: 平行光の波面までの道のり C − k2·r を一定にする
        return (k2, (s_in0 - s_in) + dot(k2, rr));
    }
    let pi = [p[0] * (zi / z_obj), p[1] * (zi / z_obj), zi];
    let s_out = zi.signum() * norm(sub(pi, rr));
    let s_out0 = zi.signum() * norm(pi);
    (k2, (s_in0 + s_out0) - (s_in + s_out))
}
