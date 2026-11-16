//! 実光線の追跡の部品（屈折・回折・Fresnel の光路長・開口・偏光の面ごとの処理）。面の形・交点・法線は
//! [`crate::shape`]、面の働きは [`crate::action`]。追跡のカーネル本体は [`crate::batch`]（1 本ずつとまとめ追跡を
//! 1 つのジェネリックな実装で）。

use crate::coating::{Iface, C64};
use crate::math::*;
use crate::surface::Aperture;
use crate::system::System;
use crate::Error;

pub use crate::shape::{powi, sag, sag_toroidal};

/// 複素の 3 次元ベクトル（電場）
pub(crate) type CV3 = [C64; 3];

/// 偏光追跡での面ごとの処理（波長ごとに用意する）
#[derive(Debug, Clone)]
pub enum PolKind {
    None,
    /// Jones 行列 [[a, b], [c, d]]
    Jones([C64; 4]),
    /// 境界の s・p 係数
    Iface(Iface),
}

#[derive(Debug, Clone)]
pub struct PolSurf {
    pub kind: PolKind,
    /// 複屈折材の板の入口（この波長の値）か出口
    pub cry: Option<CryKind>,
    /// 板の出口のすぐ後（厚さ 0、間に面なし）に次の板の入口が来る貼り合わせ（公開 #73）: 次の板の
    /// (結晶軸, no, ne)。この面で前の板の o・e から次の板の o・e への係数を 1 回でかけ、次の板の入口では係数をかけない
    pub cement: Option<(V3, f64, f64)>,
    /// 貼り合わせの次の板の入口（係数は前の板の出口の面でかけ済み）
    pub cemented: bool,
    /// 面の前の媒質の内部透過率（強度、1 mm あたり）。吸収がなければ None
    pub it: Option<f64>,
    /// ln(it) / 2（振幅の吸収 exp(la · 距離) を pow なしで求める）
    pub la: f64,
}

impl PolSurf {
    pub fn new(kind: PolKind, it: Option<f64>) -> PolSurf {
        PolSurf {
            kind,
            cry: None,
            cement: None,
            cemented: false,
            it,
            la: it.map_or(0.0, |t| 0.5 * t.ln()),
        }
    }
}

/// 複屈折材の板の入口（常光・異常光の屈折率、結晶軸、モード）と出口
#[derive(Debug, Clone, Copy)]
pub enum CryKind {
    In {
        mode: u8,
        axis: V3,
        no: f64,
        ne: f64,
    },
    Out,
}

/// 板の中を進む光線の結晶軸（今の面の局所座標）と、板の中の長さ
#[derive(Debug, Clone, Copy)]
pub(crate) struct CryState {
    pub mode: u8,
    pub axis: V3,
    pub no: f64,
    pub ne: f64,
    pub len: f64,
}

/// 板の出口で、板の中の方向 k の光線の電場に異常光の位相を付ける（[`crate::surface::Bire`]）。λ は µm、長さは mm
fn crystal_phase(c: &CryState, e: &mut [CV3], k: V3, lam: f64) {
    let k = normalize(k);
    let a = normalize(c.axis);
    let ca = dot(a, k);
    let ep = sub(a, scale(k, ca));
    let s = norm(ep);
    if c.mode == 0 {
        // 常光の成分（k と a を含む面に垂直）だけ
        let es = if s > 1e-12 {
            normalize(cross(k, scale(ep, 1.0 / s)))
        } else {
            return;
        };
        for x in e.iter_mut() {
            let v = cdot(*x, es);
            *x = [0, 1, 2].map(|i| v * es[i]);
        }
        return;
    }
    if !(s > 1e-12) {
        // 結晶軸に沿って進む光線は常光と同じ
        return;
    }
    let ep = scale(ep, 1.0 / s);
    let sa2 = 1.0 - ca * ca;
    let nth = 1.0 / (ca * ca / (c.no * c.no) + sa2 / (c.ne * c.ne)).sqrt();
    let ph = -2.0 * std::f64::consts::PI * (nth - c.no) * c.len * 1e3 / lam;
    let f = C64::from_polar(1.0, ph) - C64::new(1.0, 0.0);
    for x in e.iter_mut() {
        let v = cdot(*x, ep) * f;
        *x = [0, 1, 2].map(|i| x[i] + v * ep[i]);
    }
}

#[inline]
pub(crate) fn cmv(m: &M3, e: CV3) -> CV3 {
    [0, 1, 2].map(|i| e[0] * m[i][0] + e[1] * m[i][1] + e[2] * m[i][2])
}
#[inline]
fn cdot(e: CV3, v: V3) -> C64 {
    e[0] * v[0] + e[1] * v[1] + e[2] * v[2]
}
#[inline]
fn comb(a: C64, u: V3, b: C64, v: V3) -> CV3 {
    [0, 1, 2].map(|i| a * u[i] + b * v[i])
}

/// 単位ベクトル a を b へ移す最小の回転（ax = a × b、c = a·b）を e にかける（Rodrigues）
pub(crate) fn rotate_min(e: CV3, ax: V3, c: f64) -> CV3 {
    if 1.0 + c <= 1e-15 {
        return CNAN3;
    }
    // R e = c e + ax × e + ax (ax·e) / (1 + c)
    let ae = cdot(e, ax);
    let x = [
        e[2] * ax[1] - e[1] * ax[2],
        e[0] * ax[2] - e[2] * ax[0],
        e[1] * ax[0] - e[0] * ax[1],
    ];
    [0, 1, 2].map(|i| e[i] * c + x[i] + ae * (ax[i] / (1.0 + c)))
}

pub(crate) const CNAN3: CV3 = [C64::new(f64::NAN, f64::NAN); 3];

/// 面での電場の変化（局所座標）。kin・kl は面の前後の方向、nv は法線
pub(crate) fn pol_surface(
    ps: &PolSurf,
    e: &mut [CV3],
    kin: V3,
    kl: V3,
    nv: V3,
    cosi: f64,
    mirror: bool,
) -> Result<(), Error> {
    match &ps.kind {
        PolKind::None => {}
        PolKind::Jones(m) => {
            // OpticStudio は、局所の ±z 軸を光線へ移す最小の回転が x・y を運んだ横の基底で行列をかける
            let sz = if kl[2] > 0.0 {
                1.0
            } else if kl[2] < 0.0 {
                -1.0
            } else {
                0.0
            };
            let h = scale(add(kl, [0.0, 0.0, sz]), 1.0 / (1.0 + kl[2].abs()));
            let u = sub([1.0, 0.0, 0.0], scale(h, kl[0]));
            let v = sub([0.0, 1.0, 0.0], scale(h, kl[1]));
            for x in e.iter_mut() {
                let (eu, ev) = (cdot(*x, u), cdot(*x, v));
                *x = comb(m[0] * eu + m[1] * ev, u, m[2] * eu + m[3] * ev, v);
            }
        }
        PolKind::Iface(f) => {
            let mut sv = cross(kin, nv);
            if !(norm(sv) > 1e-12) {
                sv = cross(kin, [1.0, 0.0, 0.0]);
            }
            let sv = normalize(sv);
            let pin = cross(sv, kin);
            let mut pout = cross(sv, kl);
            if mirror {
                pout = scale(pout, -1.0);
            }
            let tc = f.eval_cos(cosi.abs().clamp(0.0, 1.0))?;
            for x in e.iter_mut() {
                let (es, ep) = (cdot(*x, sv), cdot(*x, pin));
                *x = comb(tc[0] * es, sv, tc[1] * ep, pout);
            }
        }
    }
    Ok(())
}

/// 複屈折材の板の境界の係数（公開 #52）: 板の中の電場を常光の成分（k と結晶軸を含む面に垂直）と異常光の成分に
/// 分け、常光は no、異常光は n(θ) = 1 / √(cos²θ / no² + sin²θ / ne²)（θ は板の中の方向 k と結晶軸の角）を板の側の
/// 屈折率にした境界の係数をかける。板へ入る面は外の電場に両方の係数をかけてから板の中の成分を取り、板から出る面は
/// 成分に分けてからそれぞれの係数をかけて足す。光線の道は常光のまま（walk-off は扱わない）。
/// `x` = (結晶軸, no, ne, 板の中の方向, 入る面か)。結晶軸に沿う光線は常光だけなので [`pol_surface`] と同じ
#[allow(clippy::too_many_arguments)]
fn pol_surface_crystal(
    ps: &PolSurf,
    e: &mut [CV3],
    kin: V3,
    kl: V3,
    nv: V3,
    cosi: f64,
    x: (V3, f64, f64, V3, bool),
) -> Result<(), Error> {
    let (axis, no, ne, kc, entering) = x;
    let PolKind::Iface(f) = &ps.kind else {
        return pol_surface(ps, e, kin, kl, nv, cosi, false);
    };
    let k = normalize(kc);
    let a = normalize(axis);
    let ca = dot(a, k);
    let ep = sub(a, scale(k, ca));
    let sn = norm(ep);
    if !(sn > 1e-12) {
        return pol_surface(ps, e, kin, kl, nv, cosi, false);
    }
    let ee = scale(ep, 1.0 / sn);
    let eo = normalize(cross(k, ee));
    let nth = 1.0 / (ca * ca / (no * no) + (1.0 - ca * ca) / (ne * ne)).sqrt();
    let mut sv = cross(kin, nv);
    if !(norm(sv) > 1e-12) {
        sv = cross(kin, [1.0, 0.0, 0.0]);
    }
    let sv = normalize(sv);
    let pin = cross(sv, kin);
    let pout = cross(sv, kl);
    let c0 = cosi.abs().clamp(0.0, 1.0);
    let to = f.eval_cos(c0)?;
    let te = f.eval_cos_crystal(c0, entering, nth)?;
    let sp = |t: &[C64; 2], x: CV3| comb(t[0] * cdot(x, sv), sv, t[1] * cdot(x, pin), pout);
    for x in e.iter_mut() {
        *x = if entering {
            comb(cdot(sp(&to, *x), eo), eo, cdot(sp(&te, *x), ee), ee)
        } else {
            let (xo, xe) = (eo.map(|v| cdot(*x, eo) * v), ee.map(|v| cdot(*x, ee) * v));
            let (a, b) = (sp(&to, xo), sp(&te, xe));
            [0, 1, 2].map(|i| a[i] + b[i])
        };
    }
    Ok(())
}

/// 板の中の方向 k と結晶軸 axis での常光・異常光の振動の向きと、異常光の屈折率 n(θ)。結晶軸に沿う光線は
/// 両方とも常光（向きは k に垂直な任意の組、屈折率は no）
fn crystal_basis(axis: V3, k: V3, no: f64, ne: f64) -> (V3, V3, f64) {
    let k = normalize(k);
    let a = normalize(axis);
    let ca = dot(a, k);
    let ep = sub(a, scale(k, ca));
    let sn = norm(ep);
    if !(sn > 1e-12) {
        let mut u = cross(k, [1.0, 0.0, 0.0]);
        if !(norm(u) > 1e-6) {
            u = cross(k, [0.0, 1.0, 0.0]);
        }
        let u = normalize(u);
        return (u, cross(k, u), no);
    }
    let ee = scale(ep, 1.0 / sn);
    let eo = normalize(cross(k, ee));
    let nth = 1.0 / (ca * ca / (no * no) + (1.0 - ca * ca) / (ne * ne)).sqrt();
    (eo, ee, nth)
}

/// 複屈折材の板どうしの貼り合わせの境界（公開 #73）: 前の板（`a`、板の中の方向 kin）の電場を o・e の成分に分け、
/// 各成分を次の板（`b`、板の中の方向 kl）の o・e の基底へ、前の板の屈折率（no₁ か n₁(θ)）→ 次の板の屈折率
/// （no₂ か n₂(θ)）の境界の係数で移して足す。境界は 1 回（出口・入口の 2 段に分けない）。
/// `a`・`b` = (結晶軸, no, ne)。結晶軸はどちらもこの面の局所座標
#[allow(clippy::too_many_arguments)]
fn pol_surface_cement(
    ps: &PolSurf,
    e: &mut [CV3],
    kin: V3,
    kl: V3,
    nv: V3,
    cosi: f64,
    a: (V3, f64, f64),
    b: (V3, f64, f64),
) -> Result<(), Error> {
    let PolKind::Iface(f) = &ps.kind else {
        return pol_surface(ps, e, kin, kl, nv, cosi, false);
    };
    let (eo1, ee1, n1e) = crystal_basis(a.0, kin, a.1, a.2);
    let (eo2, ee2, n2e) = crystal_basis(b.0, kl, b.1, b.2);
    let mut sv = cross(kin, nv);
    if !(norm(sv) > 1e-12) {
        sv = cross(kin, [1.0, 0.0, 0.0]);
    }
    let sv = normalize(sv);
    let pin = cross(sv, kin);
    let pout = cross(sv, kl);
    let c0 = cosi.abs().clamp(0.0, 1.0);
    // t[i][j]: 前の板の成分 i（0 = o、1 = e）→ 次の板の成分 j の (s, p)
    let (n1s, n2s) = ([a.1, n1e], [b.1, n2e]);
    let mut t = [[[C64::new(0.0, 0.0); 2]; 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            t[i][j] = f.eval_cos_media(c0, Some(n1s[i]), Some(n2s[j]))?;
        }
    }
    let sp = |t: &[C64; 2], x: CV3| comb(t[0] * cdot(x, sv), sv, t[1] * cdot(x, pin), pout);
    let (b1, b2) = ([eo1, ee1], [eo2, ee2]);
    for x in e.iter_mut() {
        let mut y = [C64::new(0.0, 0.0); 3];
        for i in 0..2 {
            let xi = b1[i].map(|v| cdot(*x, b1[i]) * v);
            for j in 0..2 {
                let c = cdot(sp(&t[i][j], xi), b2[j]);
                for m in 0..3 {
                    y[m] += c * b2[j][m];
                }
            }
        }
        *x = y;
    }
    Ok(())
}

/// 光線の追跡が止まったか・その理由（KRN-10）
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

#[derive(Debug, Clone, Copy)]
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
pub struct RecEntry {
    pub surf: usize,
    pub xyz: V3,
    pub lmn: V3,
    pub opl: f64,
    pub p: V3,
}

/// 交点のニュートン法を止める刻みの大きさ: 1e-14 mm か、距離 s・矢高 z の丸め（8 ulp）の大きい方。
/// 矢高が数十 mm の大きな面では q[2] − z の丸めが 1e-14 を超え、収束しても 60 反復まで回っていた（KRN-13）
#[inline(always)]
pub(crate) fn newton_tol(s: f64, z: f64) -> f64 {
    (8.0 * f64::EPSILON * (s.abs() + z.abs())).max(1e-14)
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
    // 5 点 Gauss–Legendre を 16 区間（被積分関数は滑らか。区間を倍にしても 1e-13 以下しか変わらない）
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
    const NP: usize = 16;
    let mut t = 0.0;
    for i in 0..NP {
        for (gx, gw) in GX.iter().zip(GW) {
            let f = (i as f64 + 0.5 + 0.5 * gx) / NP as f64;
            let (x, y) = (a[0] + ux * f, a[1] + uy * f);
            let nv = normalize(fb.normal(x, y));
            let k2 = bend(k1, nv, n1, n2, mirror)?;
            t += 0.5 * gw * ((n2 * k2[0] - n1 * k1[0]) * ux + (n2 * k2[1] - n1 * k1[1]) * uy);
        }
    }
    let t = t / NP as f64;
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

/// 開口の外か（遮蔽なら内側が「外」）。pl は面の局所座標の交点
#[inline]
pub(crate) fn aperture_out(a: Aperture, pl: V3) -> bool {
    match a {
        Aperture::Circular { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            match ring_side(u, v, min - 1e-9, max + 1e-9) {
                Some(inside) => !inside,
                None => {
                    let r = u.hypot(v);
                    r > max + 1e-9 || r < min - 1e-9
                }
            }
        }
        Aperture::Floating { semi } => match ring_side(pl[0], pl[1], 0.0, semi + 1e-9) {
            Some(inside) => !inside,
            None => (pl[0]).hypot(pl[1]) > semi + 1e-9,
        },
        Aperture::Obsc { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            match ring_side(u, v, min - 1e-9, max + 1e-9) {
                Some(inside) => inside,
                None => {
                    let r = u.hypot(v);
                    r <= max + 1e-9 && r >= min - 1e-9
                }
            }
        }
        Aperture::Rect { hx, hy, dx, dy } => {
            (pl[0] - dx).abs() > hx + 1e-9 || (pl[1] - dy).abs() > hy + 1e-9
        }
        Aperture::Ellipse { hx, hy, dx, dy } => {
            let (u, v) = ((pl[0] - dx) / hx, (pl[1] - dy) / hy);
            u * u + v * v > 1.0 + 1e-9
        }
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => spider_hit(width, arms, pl[0] - dx, pl[1] - dy),
        Aperture::None => false,
    }
}

/// 半径 r = hypot(u, v) が lo < r < hi（Some(true)）か r < lo・r > hi（Some(false)）かを、半径の 2 乗で十分に離れて
/// いるときだけ決める（境界の近く・NaN・無限大は None で、呼ぶ側が hypot で決める。[`crate::batch`] の開口と同じ幅）。
/// hypot は遅いので、ほとんどの光線をこれで決める（決めた結果は hypot と同じ）
#[inline(always)]
fn ring_side(u: f64, v: f64, lo: f64, hi: f64) -> Option<bool> {
    // 半径の 2 乗の丸めの誤差（数 ulp）より十分大きい幅
    const M: f64 = 1e-12;
    let r2 = u * u + v * v;
    let (has_lo, has_hi) = (lo > 0.0, hi > 0.0);
    let (lo2, hi2) = (lo * lo, hi * hi);
    let inside = has_hi & (r2 < hi2 * (1.0 - M)) & (!has_lo | (r2 > lo2 * (1.0 + M)));
    let outside = (has_lo & (r2 < lo2 * (1.0 - M)))
        | (has_hi & (r2 > hi2 * (1.0 + M)) & (r2 < f64::INFINITY));
    if inside {
        Some(true)
    } else if outside {
        Some(false)
    } else {
        None
    }
}

/// スパイダーの腕に当たるか（x y は開口の中心から）。腕 k は角度 2πk/arms の向きの、中心から外への幅 width の帯
/// （境界を含む）。OpticStudio 2026 R1.01 と同じく余裕をとらない: 幅 0 では +x の腕の上（y = 0、x ≥ 0）だけ遮り、
/// cos 90° などが 0 にならないほかの腕は中心のほか遮らない（実測）
pub(crate) fn spider_hit(width: f64, arms: u32, x: f64, y: f64) -> bool {
    let hw = 0.5 * width;
    (0..arms).any(|k| {
        let (s, c) = (std::f64::consts::TAU * k as f64 / arms as f64).sin_cos();
        x * c + y * s >= 0.0 && (y * c - x * s).abs() <= hw
    })
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
    if !z_obj.is_finite() || z_obj.abs() > 1e12 {
        let ff = [f * t[0], f * t[1], f];
        let kin = scale(kl, sz);
        return (
            k2,
            f.signum() * (norm(ff) - norm(sub(ff, rr)) - dot(kin, rr)),
        );
    }
    let p = [r[0] + t[0] * z_obj, r[1] + t[1] * z_obj, z_obj];
    let zi = 1.0 / (1.0 / f + 1.0 / z_obj);
    let s_in = (-z_obj).signum() * norm(sub(rr, p));
    let s_in0 = (-z_obj).signum() * norm(p);
    if !zi.is_finite() || zi.abs() > 1e12 {
        // 物体が前側焦点にある（出ていく光は平行光）: 平行光の波面までの道のり C − k2·r を一定にする
        return (k2, (s_in0 - s_in) + dot(k2, rr));
    }
    let pi = [p[0] * (zi / z_obj), p[1] * (zi / z_obj), zi];
    let s_out = zi.signum() * norm(sub(pi, rr));
    let s_out0 = zi.signum() * norm(pi);
    (k2, (s_in0 + s_out0) - (s_in + s_out))
}

/// 面 j での電場の変化を 1 本の光線について（複屈折材の板の入口・出口・貼り合わせを含む）。g は
/// (面の前の方向, 後の方向, 法線, 入射角の cos)。係数のエラーは err に持つ（束を止めるかは追跡の後で決める）
#[allow(clippy::too_many_arguments)]
pub(crate) fn pol_step(
    sys: &System,
    surf: &[PolSurf],
    j: usize,
    e: &mut [CV3],
    cry_state: &mut Option<CryState>,
    err: &mut Option<Error>,
    g: (V3, V3, V3, f64),
    mirror: bool,
    lam: f64,
) {
    let (kin, kl, nv, cosi) = g;
    // 複屈折材の板の出口: 板の中の方向 kin で異常光の位相を付けてから境界の係数
    let cry = surf[j].cry;
    // 境界の係数を常光・異常光で分ける板の面（公開 #52）: (結晶軸, no, ne, 板の中の方向, 入る面か)
    let mut xc = None;
    // 貼り合わせ（公開 #73）: 前の板の出口で次の板の o・e へ 1 回で移す
    let mut cem = None;
    if let (Some(CryKind::Out), Some(c)) = (cry, cry_state.take()) {
        crystal_phase(&c, e, kin, lam);
        xc = Some((c.axis, c.no, c.ne, kin, false));
        if let Some(b) = surf[j].cement {
            cem = Some(((c.axis, c.no, c.ne), b));
        }
    }
    if let Some(CryKind::In { axis, no, ne, .. }) = cry {
        xc = Some((axis, no, ne, kl, true));
    }
    let r = match (xc, cem) {
        _ if surf[j].cemented && !mirror => Ok(()),
        (_, Some((a, b))) if !mirror => pol_surface_cement(&surf[j], e, kin, kl, nv, cosi, a, b),
        (Some(x), _) if !mirror => pol_surface_crystal(&surf[j], e, kin, kl, nv, cosi, x),
        _ => pol_surface(&surf[j], e, kin, kl, nv, cosi, mirror),
    };
    if let Err(x) = r {
        // 光線ごとに持ち、束を止めるかは追跡の後で決める（後ろの面でけられる光線もあるため）
        err.get_or_insert_with(|| x.at(sys.surf_label(j)));
    }
    if let Some(CryKind::In { mode, axis, no, ne }) = cry {
        *cry_state = Some(CryState {
            mode,
            axis,
            no,
            ne,
            len: 0.0,
        });
    }
}
