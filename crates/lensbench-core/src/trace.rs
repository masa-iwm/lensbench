//! 実光線の追跡の核（1 本ずつ）。
//!
//! 光線が面に当たらない・全反射するとその面で止まり（err = 面番号、以後 NaN）、開口の外に出た光線は
//! 先へ進む（vig = 最初の面）。OpticStudio のバッチ追跡と同じ扱い。

use crate::coating::{Iface, C64};
use crate::math::*;
use crate::surface::{Aperture, SurfType, Surface};
use crate::system::{Frame, Frames, Rel, System};
use crate::Error;
use std::sync::atomic::{AtomicU8, Ordering};

/// 浮動開口（[`Aperture::Floating`]）で光線をけるか。0 は未設定（環境変数 LENSBENCH_FLOATING_CLIP を見る）、1 ける、2 けない
static FLOATING_CLIP: AtomicU8 = AtomicU8::new(0);

/// 浮動開口（半直径を固定した面・.zmx の FLAP）の外の光線をけるか（既定はける。OpticStudio は半直径を固定した面を
/// 浮動開口にしてける）。環境変数 LENSBENCH_FLOATING_CLIP=0 でも切れる。光線エイミングの追跡は開口を見ない
pub fn set_floating_clip(on: bool) {
    FLOATING_CLIP.store(if on { 1 } else { 2 }, Ordering::Relaxed);
}

/// 浮動開口で光線をけるか（[`set_floating_clip`]）
pub fn floating_clip() -> bool {
    match FLOATING_CLIP.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let on = std::env::var("LENSBENCH_FLOATING_CLIP")
                .map(|v| !matches!(v.trim(), "0" | "false" | "off" | "no"))
                .unwrap_or(true);
            set_floating_clip(on);
            on
        }
    }
}

/// 複素の 3 次元ベクトル（電場）
pub type CV3 = [C64; 3];

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
pub struct CryState {
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

/// 1 本の光線の偏光の状態: 電場（偏光の入力ごと、今いる面の局所座標）
pub struct PolRay<'a> {
    pub e: Vec<CV3>,
    pub surf: &'a [PolSurf],
    /// 係数を求められなかったとき（膜の表の範囲外など）。電場は NaN にする
    pub err: Option<Error>,
    /// 複屈折材の板の中を進んでいるとき
    pub cry: Option<CryState>,
}

#[inline]
pub(crate) fn cmv(m: &M3, e: CV3) -> CV3 {
    [0, 1, 2].map(|i| e[0] * m[i][0] + e[1] * m[i][1] + e[2] * m[i][2])
}
#[inline]
pub(crate) fn cmtv(m: &M3, e: CV3) -> CV3 {
    [0, 1, 2].map(|i| e[0] * m[0][i] + e[1] * m[1][i] + e[2] * m[2][i])
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

#[derive(Debug, Clone, Copy)]
pub struct RayOut {
    /// 最後の面の局所座標での位置・方向（方向は物理的な向き）
    pub xyz: V3,
    pub lmn: V3,
    /// 幾何的な光路長（理想レンズの位相を含まない）
    pub opl: f64,
    pub err: usize,
    /// err の面で全反射した（当たらなかったのではない）
    pub tir: bool,
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

/// 追跡の内側で面ごとの記録を受け取る（k は無視しない面の通し番号）
pub trait RecSink {
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

/// Fresnel・Cylinder Fresnel の屈折に使う面の法線（規格化前、局所座標）。
/// Fresnel は偶数次非球面（c・k・asph）、Cylinder Fresnel は [`crate::surface::CylFresnel`] の面（y だけ）の傾き
fn fresnel_normal(d: &Surface, x: f64, y: f64) -> V3 {
    match (d.typ, d.cylf.as_deref()) {
        (SurfType::CylFresnel, Some(f)) => {
            let (_, dz) = sag(f.c, f.k, &f.coef, y * y);
            [0.0, -2.0 * dz * y, 1.0]
        }
        (SurfType::CylFresnel, None) => [0.0, 0.0, 1.0],
        _ => {
            let (_, dz) = sag(d.c, d.k, &d.asph, x * x + y * y);
            [-2.0 * dz * x, -2.0 * dz * y, 1.0]
        }
    }
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
fn fresnel_opl(d: &Surface, k1: V3, n1: f64, n2: f64, a: [f64; 2], b: [f64; 2]) -> Option<f64> {
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
    let mirror = d.mirror;
    let mut t = 0.0;
    for i in 0..NP {
        for (gx, gw) in GX.iter().zip(GW) {
            let f = (i as f64 + 0.5 + 0.5 * gx) / NP as f64;
            let (x, y) = (a[0] + ux * f, a[1] + uy * f);
            let nv = normalize(fresnel_normal(d, x, y));
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
fn diffract(k: V3, nv: V3, n: f64, g: [f64; 2]) -> Option<V3> {
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

/// 偶数次非球面のサグと d サグ / d(r^2)
pub fn sag(c: f64, k: f64, asph: &[f64], r2: f64) -> (f64, f64) {
    let arg = 1.0 - (1.0 + k) * c * c * r2;
    let root = if c != 0.0 { arg.max(0.0).sqrt() } else { 1.0 };
    let mut z = c * r2 / (1.0 + root);
    let mut dz = if c != 0.0 {
        if arg > 0.0 {
            c / (2.0 * root)
        } else {
            f64::NAN
        }
    } else {
        0.0
    };
    for (i, &a) in asph.iter().enumerate() {
        if a != 0.0 {
            let i = (i + 1) as i32;
            z += a * powi(r2, i);
            dz += i as f64 * a * powi(r2, i - 1);
        }
    }
    (z, dz)
}

/// x^n（n ≥ 0）を 2 乗の繰り返しで。`f64::powi` と同じ手順（掛ける順も同じ）で、まとめて追う
/// [`crate::batch`] が光線ごとに同じビットを出せるよう手順を固定する
#[inline(always)]
pub fn powi(mut x: f64, n: i32) -> f64 {
    debug_assert!(n >= 0);
    let mut n = n as u32;
    let mut r = 1.0;
    loop {
        if n & 1 != 0 {
            r *= x;
        }
        n /= 2;
        if n == 0 {
            return r;
        }
        x *= x;
    }
}

/// トロイダル面のサグと (dz/dx, dz/dy)。YZ 面の曲線 z(y)（c・k・asph）を、頂点から 1/cx 離れた
/// Y 軸に平行な軸の周りに回した面（cx = 0 は円柱面）。軸の向こう側の点は NaN
pub fn sag_toroidal(c: f64, k: f64, asph: &[f64], cx: f64, x: f64, y: f64) -> (f64, f64, f64) {
    let (zy, dzr) = sag(c, k, asph, y * y);
    let dzy = 2.0 * y * dzr;
    if cx == 0.0 {
        return (zy, 0.0, dzy);
    }
    // 回転の円の曲率は点ごとに cx / (1 − cx z(y))（軸までの距離が 1/cx − z(y)）
    let b = 1.0 - cx * zy;
    if !(b > 0.0) {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let ce = cx / b;
    let arg = 1.0 - ce * ce * x * x;
    if !(arg > 0.0) {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let root = arg.sqrt();
    (zy + ce * x * x / (1.0 + root), ce * x / root, dzy / root)
}

const NAN3: V3 = [f64::NAN; 3];

/// 二次曲面との交点の根（頂点面上の点からの距離）: aa s² − 2 gg s + ff = 0 の根 ff / (gg + σ √disc)（σ はその空間の
/// 進む向き: 鏡で反射するたびに反転、[`space_signs`]）。OpticStudio と同じく、光線が空間の向きと逆（gg σ < 0、
/// 90° を超える視野の光線が面 1 へ向かうなど）でも同じ式の根を採る（以前は gg の符号で頂点に近い根を採っていた）。
/// 桁落ちしないよう gg σ < 0 では同じ根を (gg − σ √disc) / aa で求める
#[inline(always)]
pub(crate) fn conic_root(ff: f64, gg: f64, aa: f64, disc: f64, sig: f64) -> f64 {
    let q = sig * disc.sqrt();
    if gg * sig >= 0.0 {
        ff / (gg + q)
    } else if aa != 0.0 {
        (gg - q) / aa
    } else {
        ff / (2.0 * gg)
    }
}

/// 面ごとの空間の進む向き（面 0 から +1、鏡の面の後で反転）。σ[j] は面 j に向かう空間の向き
pub(crate) fn space_signs(sys: &System) -> Vec<f64> {
    let mut out = Vec::with_capacity(sys.surfaces.len());
    let mut s = 1.0;
    for d in &sys.surfaces {
        out.push(s);
        if d.mirror && !d.ignore && d.typ != SurfType::CoordBrk {
            s = -s;
        }
    }
    out
}

/// トロイダル面との交点までの距離（頂点面上の pl から方向 kl に）。収束しなければ true も返す。
/// 初期値は y だけの二次曲面（円柱面）との交点、その後ニュートン法
pub(crate) fn toroidal_isect(d: &Surface, pl: V3, kl: V3, sig: f64) -> (f64, bool) {
    let (c, kc) = (d.c, d.k);
    let mut s = 0.0;
    if c != 0.0 {
        let ff = c * pl[1] * pl[1];
        let gg = kl[2] - c * pl[1] * kl[1];
        let aa = c * (kl[1] * kl[1] + (1.0 + kc) * kl[2] * kl[2]);
        let disc = gg * gg - aa * ff;
        if disc >= 0.0 {
            let s0 = conic_root(ff, gg, aa, disc, sig);
            if s0.is_finite() {
                s = s0;
            }
        }
    }
    let mut bad = false;
    if d.cx != 0.0 || d.has_asph() || s == 0.0 && c != 0.0 {
        let mut ds = f64::NAN;
        for _ in 0..60 {
            let q = add(pl, scale(kl, s));
            let (z, zx, zy) = sag_toroidal(c, kc, &d.asph, d.cx, q[0], q[1]);
            ds = (q[2] - z) / (kl[2] - zx * kl[0] - zy * kl[1]);
            s -= ds;
            if !(ds.abs() > 1e-14) {
                break;
            }
        }
        bad = !(ds.abs() <= 1e-10);
    }
    (s, bad)
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

/// 大域の位置 p・方向 k、光路長 opl の光線を、面 first の直前から面 last まで、波長 w で追跡する。
/// rec には無視しない面ごとに 1 つずつ（first..=last の順に）書く（足りる長さを渡す）
#[allow(clippy::too_many_arguments)]
pub fn trace_ray(
    sys: &System,
    fr: &Frames,
    w: usize,
    p: V3,
    k: V3,
    opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    rec: Option<&mut [RecEntry]>,
) -> RayOut {
    trace_core(sys, fr, w, p, k, opl, first, last, floating_clip, rec, None)
}

/// trace_ray に偏光を加えたもの（電場 pol.e を面ごとに更新する）
#[allow(clippy::too_many_arguments)]
pub fn trace_ray_pol(
    sys: &System,
    fr: &Frames,
    w: usize,
    p: V3,
    k: V3,
    opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    rec: Option<&mut [RecEntry]>,
    pol: &mut PolRay,
) -> RayOut {
    trace_core(
        sys,
        fr,
        w,
        p,
        k,
        opl,
        first,
        last,
        floating_clip,
        rec,
        Some(pol),
    )
}

/// 交点の求め方（面ごとに前計算する。[`surf_kind`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Isect {
    /// 頂点の平面（Fresnel もここ）
    Plane,
    /// 回転対称の曲面（asph: 非球面項があれば Newton で詰める）
    Std {
        asph: bool,
    },
    Toroidal,
    /// 自由曲面（Zernike/Grid Sag）
    Free,
    /// Cylinder Fresnel の基板（y だけの関数）
    CylSub,
    /// 形を傾けた Gradient 9（標準面 + x tan α + y tan β）
    Grad9Tilt,
}

/// 法線の求め方（Paraxial・座標ブレークは面の種類で別に扱う）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Norm {
    Flat,
    Conic,
    Asph,
    Toroidal,
    Free,
    Fresnel,
    Grad9Tilt,
}

/// 追跡の内側で面ごとに分ける種類（光線ごと・面ごとの判定を系の更新時の 1 回にする）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfKind {
    pub isect: Isect,
    pub norm: Norm,
    /// Fresnel・Cylinder Fresnel（光路長の補正がある）
    pub fresnel: bool,
}

/// 回転対称の面（[`Isect::Std`]）で非球面の項を使うか: 偶数次非球面・Binary 2 で項があるとき。標準面などに残った
/// 項は使わない（[`crate::system::System::check`] で誤りにする）
pub(crate) fn std_asph(d: &Surface) -> bool {
    matches!(d.typ, SurfType::EvenAsph | SurfType::Binary2) && d.has_asph()
}

pub(crate) fn surf_kind(d: &Surface) -> SurfKind {
    let curved = matches!(
        d.typ,
        SurfType::Standard
            | SurfType::EvenAsph
            | SurfType::Toroidal
            | SurfType::Binary2
            | SurfType::Gradium
            | SurfType::Gradient9
    ) && (d.c != 0.0 || !d.asph.is_empty() || d.cx != 0.0);
    let free = crate::freeform::is_freeform(d.typ);
    let g9tilt = d.typ == SurfType::Gradient9 && crate::grin::grad9_tilted(d);
    let fresnel = matches!(d.typ, SurfType::Fresnel | SurfType::CylFresnel);
    let isect = if g9tilt {
        Isect::Grad9Tilt
    } else if d.typ == SurfType::CylFresnel && (d.c != 0.0 || d.has_asph()) {
        Isect::CylSub
    } else if free {
        Isect::Free
    } else if curved && d.typ == SurfType::Toroidal {
        Isect::Toroidal
    } else if curved {
        // 交点と法線は同じ判定で（非球面の項を使う型だけ。以前は交点だけ型を見ずに非球面の項を使っていた）
        Isect::Std { asph: std_asph(d) }
    } else {
        Isect::Plane
    };
    let norm = if g9tilt {
        Norm::Grad9Tilt
    } else if free {
        Norm::Free
    } else if d.typ == SurfType::Toroidal {
        Norm::Toroidal
    } else if fresnel {
        Norm::Fresnel
    } else if std_asph(d) {
        Norm::Asph
    } else if d.c == 0.0 {
        Norm::Flat
    } else {
        Norm::Conic
    };
    SurfKind {
        isect,
        norm,
        fresnel,
    }
}

// 偏光なしの追跡に偏光の分岐が残らないよう、呼び出し側ごとに展開する
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn trace_core<R: RecSink + ?Sized>(
    sys: &System,
    fr: &Frames,
    w: usize,
    mut p: V3,
    mut k: V3,
    mut opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    mut rec: Option<&mut R>,
    mut pol: Option<&mut PolRay>,
) -> RayOut {
    let sf = &sys.surfaces;
    let kinds = sys.surf_kinds();
    let mut err = 0usize;
    let mut tir = false;
    let mut vig = 0usize;
    let mut nprev = sys.index_before(first, w);
    let mut lens_opd = 0.0;
    let mut loc = (NAN3, NAN3);
    // 直前に通った面の番号（その局所座標 loc から次の面へ直接移る）
    let mut prev: Option<usize> = None;
    let mut last_frame: Option<Frame> = None;
    // 記録の次の位置（無視しない面ごとに 1 つ）
    let mut ri = 0usize;
    // 直前の面の後ろが GRIN の媒質なら、その面の番号（次の面までは媒質の中を曲がりながら進む）
    let mut grin_from: Option<usize> = None;
    // GRIN の媒質に入ったときの方向（座標ブレークを挟んで進むあいだ持つ）
    let mut grin_k0: Option<V3> = None;
    let lam = sys.wavelengths[w];
    let sigs = sys.space_signs();
    for j in first..=last {
        let d = &sf[j];
        if d.ignore {
            continue;
        }
        let f = fr.surf[j].unwrap();
        let (o, rm) = (f.o, f.r);
        last_frame = Some(f);
        if err != 0 {
            let pl = NAN3;
            loc = (pl, NAN3);
            p = NAN3;
            k = NAN3;
            prev = Some(j);
            if let Some(r) = rec.as_deref_mut() {
                r.put(
                    ri,
                    RecEntry {
                        surf: j,
                        xyz: pl,
                        lmn: NAN3,
                        opl: f64::NAN,
                        p: NAN3,
                    },
                );
                ri += 1;
            }
            continue;
        }
        // GRIN の媒質の中を、GRIN の面の局所座標で面 j との交点まで進める（交点は面の上なので、下の交点の計算は
        // 飛ばす）。間に座標ブレークがあれば、その面までで一度止め、次の面で続きを進める（媒質は GRIN の面の座標の
        // まま続く）
        let mut on_surf = false;
        // 光線を面 j の局所座標へ移す変換を、直前の面からでなく GRIN の面から（媒質の中を進めたとき）
        let mut from_grin: Option<(M3, V3)> = None;
        if let Some(gj) = grin_from.take() {
            let gs = &sf[gj];
            let g = gs.grin.as_deref().unwrap();
            let fg = fr.surf[gj].unwrap();
            let rr = mm(&transpose(&rm), &fg.r);
            let tt = mtv(&rm, sub(fg.o, o));
            // 直前の面が座標ブレークなら、その局所座標から GRIN の面の局所座標へ戻す
            let pj = prev.unwrap_or(gj);
            if pj != gj {
                let fp = fr.surf[pj].unwrap();
                let rb = mm(&transpose(&fg.r), &fp.r);
                let tb = mtv(&fg.r, sub(fp.o, fg.o));
                loc = (add(mv(&rb, loc.0), tb), mv(&rb, loc.1));
                if let Some(pr) = pol.as_deref_mut() {
                    pr.e.iter_mut().for_each(|e| *e = cmv(&rb, *e));
                }
            }
            let max_len = 100.0 + 100.0 * gs.t.abs();
            let ex = if d.typ == SurfType::CoordBrk {
                // 座標ブレークの面（平面 z = 0）は光線の後ろにもありうる（厚さ 0 の傾けた座標ブレークなど）。
                // 均質な媒質と同じく、そのときは媒質の中を逆向きにたどって交点を求める（光路長は負）
                let fx = |q: V3| add(mv(&rr, q), tt)[2];
                let back = fx(loc.0) * mv(&rr, loc.1)[2] > 0.0;
                if back {
                    crate::grin::propagate(g, lam, loc.0, scale(loc.1, -1.0), fx, max_len).map(
                        |e| crate::grin::Exit {
                            k: scale(e.k, -1.0),
                            opl: -e.opl,
                            ..e
                        },
                    )
                } else {
                    crate::grin::propagate(g, lam, loc.0, loc.1, fx, max_len)
                }
            } else {
                crate::grin::next_sag(d).and_then(|sg| {
                    let fx = |q: V3| {
                        let q2 = add(mv(&rr, q), tt);
                        q2[2] - sg(q2[0], q2[1])
                    };
                    crate::grin::propagate(g, lam, loc.0, loc.1, fx, max_len)
                })
            };
            match ex {
                Some(e) => {
                    // 媒質に入ったときの方向（GRIN の面の局所座標）
                    let k0 = *grin_k0.get_or_insert(loc.1);
                    if let (Some(pr), false) = (pol.as_deref_mut(), d.typ == SurfType::CoordBrk) {
                        // 電場は、媒質に入ったときからの方向の変化と同じ最小の回転で回す（媒質の中の偏光の変化は
                        // 見ない。座標ブレークで区切っても同じ回転になるよう、次の実在の面でまとめて回す）
                        let (ax, c) = (cross(k0, e.k), dot(k0, e.k));
                        pr.e.iter_mut().for_each(|x| *x = rotate_min(*x, ax, c));
                    }
                    opl += e.opl;
                    nprev = e.n;
                    loc = (e.p, e.k);
                    on_surf = true;
                    from_grin = Some((rr, tt));
                    // 座標ブレークの後ろも同じ媒質
                    if d.typ == SurfType::CoordBrk {
                        grin_from = Some(gj);
                    } else {
                        grin_k0 = None;
                    }
                }
                None => {
                    err = j;
                    loc = (NAN3, NAN3);
                    grin_k0 = None;
                }
            }
        }
        let (mut pl, mut kl) = match (prev, fr.rel[j], from_grin) {
            (_, _, Some((rr, tt))) => (add(mv(&rr, loc.0), tt), mv(&rr, loc.1)),
            (
                Some(_),
                Some(Rel {
                    shift: Some(dz), ..
                }),
                None,
            ) => ([loc.0[0], loc.0[1], loc.0[2] + dz], loc.1),
            (Some(_), Some(q), None) => (add(mv(&q.r, loc.0), q.t), mv(&q.r, loc.1)),
            _ => (mtv(&rm, sub(p, o)), mtv(&rm, k)),
        };
        // 電場も方向と同じ変換で今の面の局所座標へ
        if let Some(pr) = pol.as_deref_mut() {
            match (prev, fr.rel[j], from_grin) {
                (_, _, Some((rr, _))) => pr.e.iter_mut().for_each(|e| *e = cmv(&rr, *e)),
                (Some(_), Some(Rel { shift: Some(_), .. }), None) => {}
                (Some(_), Some(q), None) => pr.e.iter_mut().for_each(|e| *e = cmv(&q.r, *e)),
                _ => pr.e.iter_mut().for_each(|e| *e = cmtv(&rm, *e)),
            }
            // 板の結晶軸も同じ変換で
            if let Some(c) = pr.cry.as_mut() {
                c.axis = match (prev, fr.rel[j], from_grin) {
                    (_, _, Some((rr, _))) => mv(&rr, c.axis),
                    (Some(_), Some(Rel { shift: Some(_), .. }), None) => c.axis,
                    (Some(_), Some(q), None) => mv(&q.r, c.axis),
                    _ => mtv(&rm, c.axis),
                };
            }
        }
        prev = Some(j);
        // 頂点面へ
        let s = if on_surf { 0.0 } else { -pl[2] / kl[2] };
        if !s.is_finite() {
            err = j;
        }
        pl = add(pl, scale(kl, s));
        opl += nprev * s;
        let mut s_tot = s;
        let kd = kinds[j];
        // GRIN の媒質から来た光線はもう面の上
        let (s, bad) = match kd.isect {
            _ if err != 0 || on_surf => (0.0, false),
            Isect::Plane => (0.0, false),
            Isect::Grad9Tilt => crate::freeform::isect_with(d.c, d.k, pl, kl, sigs[j], |x, y| {
                crate::grin::grad9_sag(d, x, y)
            }),
            Isect::CylSub => crate::freeform::isect_with(0.0, 0.0, pl, kl, sigs[j], |_, y| {
                let (z, dz) = sag(d.c, d.k, &d.asph, y * y);
                (z, 0.0, 2.0 * dz * y)
            }),
            Isect::Free => crate::freeform::isect(d, pl, kl, sigs[j]),
            Isect::Toroidal => toroidal_isect(d, pl, kl, sigs[j]),
            Isect::Std { asph } => {
                let (c, kc) = (d.c, d.k);
                let mut s = 0.0;
                let mut bad = false;
                if c != 0.0 {
                    let ff = c * (pl[0] * pl[0] + pl[1] * pl[1]);
                    let gg = kl[2] - c * (pl[0] * kl[0] + pl[1] * kl[1]);
                    let aa = c * (kl[0] * kl[0] + kl[1] * kl[1] + (1.0 + kc) * kl[2] * kl[2]);
                    let disc = gg * gg - aa * ff;
                    if !(disc >= 0.0) {
                        bad = true;
                    }
                    s = conic_root(ff, gg, aa, disc.max(0.0), sigs[j]);
                    if !s.is_finite() {
                        bad = true;
                    }
                }
                if asph && !bad {
                    let mut ds = 0.0;
                    for _ in 0..60 {
                        let q = add(pl, scale(kl, s));
                        let r2 = q[0] * q[0] + q[1] * q[1];
                        let (z, dz) = sag(c, kc, &d.asph, r2);
                        ds = (q[2] - z) / (kl[2] - 2.0 * dz * (q[0] * kl[0] + q[1] * kl[1]));
                        s -= ds;
                        if !(ds.abs() > 1e-14) {
                            break;
                        }
                    }
                    if !(ds.abs() <= 1e-10) {
                        bad = true;
                    }
                }
                (s, bad)
            }
        };
        if err == 0 && !on_surf && kd.isect != Isect::Plane {
            if bad {
                err = j;
            }
            pl = add(pl, scale(kl, s));
            opl += nprev * s;
            s_tot += s;
        }
        // 通ってきた媒質の吸収（振幅は内部透過率の 1/2 乗）
        if let Some(pr) = pol.as_deref_mut() {
            if let Some(c) = pr.cry.as_mut() {
                c.len += s_tot;
            }
            if pr.surf[j].it.is_some() {
                // 距離 0 は 1（t = 0 でも pow と同じ）
                let a = if s_tot == 0.0 {
                    1.0
                } else {
                    (pr.surf[j].la * s_tot).exp()
                };
                pr.e.iter_mut().for_each(|e| *e = e.map(|c| c * a));
            }
        }
        // 開口
        let clip = match d.ap {
            Aperture::None => None,
            Aperture::Floating { .. } if !floating_clip => None,
            a => Some(a),
        };
        if let Some(a) = clip {
            let out = aperture_out(a, pl);
            if out && vig == 0 && err == 0 {
                vig = j;
            }
        }
        let mut nn = d.n[w];
        // GRIN の媒質へは交点の屈折率で
        if let (Some(g), false, 0) = (d.grin.as_deref(), d.mirror, err) {
            match g.eval(lam, pl) {
                Some((n, _)) => nn = n,
                None => err = j,
            }
        }
        match d.typ {
            SurfType::CoordBrk => nn = nprev,
            SurfType::Paraxial => {
                let (k2, dl) = paraxial(d.focal, pl, kl, sys.lens_conjugate(j));
                // 電場は方向の変化と同じ最小の回転で回す（振幅は変えない。OpticStudio との照合は未確認）
                if let Some(pr) = pol.as_deref_mut() {
                    if err == 0 {
                        let (ax, c) = (cross(kl, k2), dot(kl, k2));
                        pr.e.iter_mut().for_each(|e| *e = rotate_min(*e, ax, c));
                    }
                }
                kl = k2;
                lens_opd += dl;
            }
            SurfType::User => {
                let out = d.user.as_ref().and_then(|u| {
                    u.instance()
                        .deflect(pl, kl, nprev, nn, lam)
                        .filter(|(k2, dl)| k2.iter().all(|v| v.is_finite()) && dl.is_finite())
                });
                match out {
                    Some((k2, dl)) if err == 0 => {
                        // 電場は理想レンズと同じく、方向の変化と同じ最小の回転で回す
                        if let Some(pr) = pol.as_deref_mut() {
                            let (ax, c) = (cross(kl, k2), dot(kl, k2));
                            pr.e.iter_mut().for_each(|e| *e = rotate_min(*e, ax, c));
                        }
                        kl = k2;
                        lens_opd += dl;
                    }
                    Some(_) => {}
                    None => {
                        if err == 0 {
                            err = j;
                        }
                    }
                }
            }
            _ => {
                let nvec = match kd.norm {
                    Norm::Free => {
                        let (_, zx, zy) = crate::freeform::sag_xy(d, pl[0], pl[1]);
                        if !(zx.is_finite() && zy.is_finite()) && err == 0 {
                            err = j;
                        }
                        [-zx, -zy, 1.0]
                    }
                    Norm::Toroidal => {
                        let (_, zx, zy) = sag_toroidal(d.c, d.k, &d.asph, d.cx, pl[0], pl[1]);
                        if !(zx.is_finite() && zy.is_finite()) && err == 0 {
                            err = j;
                        }
                        [-zx, -zy, 1.0]
                    }
                    Norm::Grad9Tilt => {
                        let (_, zx, zy) = crate::grin::grad9_sag(d, pl[0], pl[1]);
                        if !(zx.is_finite() && zy.is_finite()) && err == 0 {
                            err = j;
                        }
                        [-zx, -zy, 1.0]
                    }
                    Norm::Fresnel => {
                        let nv = fresnel_normal(d, pl[0], pl[1]);
                        if !(nv[0].is_finite() && nv[1].is_finite()) && err == 0 {
                            err = j;
                        }
                        nv
                    }
                    Norm::Asph => {
                        let (_, dz) = sag(d.c, d.k, &d.asph, pl[0] * pl[0] + pl[1] * pl[1]);
                        if !dz.is_finite() && err == 0 {
                            err = j;
                        }
                        [-2.0 * dz * pl[0], -2.0 * dz * pl[1], 1.0]
                    }
                    Norm::Flat => [0.0, 0.0, 1.0],
                    Norm::Conic => [-d.c * pl[0], -d.c * pl[1], 1.0 - d.c * (1.0 + d.k) * pl[2]],
                };
                let nv = normalize(nvec);
                let cosi = dot(kl, nv);
                let kin = kl;
                if d.mirror {
                    kl = sub(kl, scale(nv, 2.0 * cosi));
                    nn = nprev;
                } else if d.typ != SurfType::Jones && nn != nprev {
                    let mu = nprev / nn;
                    let disc = 1.0 - mu * mu * (1.0 - cosi * cosi);
                    if disc < 0.0 && err == 0 {
                        err = j;
                        tir = true;
                    }
                    kl = add(
                        scale(kl, mu),
                        scale(nv, cosi.signum() * disc.max(0.0).sqrt() - mu * cosi),
                    );
                }
                if let Some(pr) = pol.as_deref_mut() {
                    if err == 0 {
                        // 複屈折材の板の出口: 板の中の方向 kin で異常光の位相を付けてから境界の係数
                        let cry = pr.surf[j].cry;
                        // 境界の係数を常光・異常光で分ける板の面（公開 #52）: (結晶軸, no, ne, 板の中の方向, 入る面か)
                        let mut xc = None;
                        // 貼り合わせ（公開 #73）: 前の板の出口で次の板の o・e へ 1 回で移す
                        let mut cem = None;
                        if let (Some(CryKind::Out), Some(c)) = (cry, pr.cry.take()) {
                            crystal_phase(&c, &mut pr.e, kin, lam);
                            xc = Some((c.axis, c.no, c.ne, kin, false));
                            if let Some(b) = pr.surf[j].cement {
                                cem = Some(((c.axis, c.no, c.ne), b));
                            }
                        }
                        if let Some(CryKind::In { axis, no, ne, .. }) = cry {
                            xc = Some((axis, no, ne, kl, true));
                        }
                        let r = match (xc, cem) {
                            _ if pr.surf[j].cemented && !d.mirror => Ok(()),
                            (_, Some((a, b))) if !d.mirror => {
                                pol_surface_cement(&pr.surf[j], &mut pr.e, kin, kl, nv, cosi, a, b)
                            }
                            (Some(x), _) if !d.mirror => {
                                pol_surface_crystal(&pr.surf[j], &mut pr.e, kin, kl, nv, cosi, x)
                            }
                            _ => pol_surface(&pr.surf[j], &mut pr.e, kin, kl, nv, cosi, d.mirror),
                        };
                        if let Err(e) = r {
                            // 光線ごとに持ち、束を止めるかは追跡の後で決める（後ろの面でけられる光線もあるため）
                            pr.err.get_or_insert_with(|| e.at(sys.surf_label(j)));
                        }
                        if let Some(CryKind::In { mode, axis, no, ne }) = cry {
                            pr.cry = Some(CryState {
                                mode,
                                axis,
                                no,
                                ne,
                                len: 0.0,
                            });
                        }
                    }
                }
                if kd.fresnel && err == 0 {
                    let r0 = fr.fres_ref.get(j).copied().unwrap_or([0.0; 2]);
                    match fresnel_opl(d, kin, nprev, nn, r0, [pl[0], pl[1]]) {
                        Some(e) => opl += e,
                        None => err = j,
                    }
                }
                if let (SurfType::Binary2, Some(b), 0) = (d.typ, d.bin2.as_deref(), err) {
                    let sc = sys.wavelengths[w] * 1e-3 / (2.0 * std::f64::consts::PI);
                    let (ph, gx, gy) = b.phase(pl[0], pl[1]);
                    opl += sc * ph;
                    match diffract(kl, nv, nn, [sc * gx, sc * gy]) {
                        Some(k2) => {
                            // 電場は方向の変化と同じ最小の回転で回す（振幅は変えない。OpticStudio との照合は未確認）
                            if let Some(pr) = pol.as_deref_mut() {
                                let (ax, c) = (cross(kl, k2), dot(kl, k2));
                                pr.e.iter_mut().for_each(|e| *e = rotate_min(*e, ax, c));
                            }
                            kl = k2;
                        }
                        None => err = j,
                    }
                }
            }
        }
        if err != 0 {
            pl = NAN3;
            kl = NAN3;
            opl = f64::NAN;
        }
        nprev = nn;
        loc = (pl, kl);
        if d.grin.is_some() && !d.mirror && err == 0 {
            grin_from = Some(j);
        } else if err != 0 {
            grin_from = None;
        }
        if let Some(r) = rec.as_deref_mut() {
            if !r.wants(ri) {
                ri += 1;
                continue;
            }
            p = add(o, mv(&rm, pl));
            r.put(
                ri,
                RecEntry {
                    surf: j,
                    xyz: pl,
                    lmn: kl,
                    opl,
                    p,
                },
            );
            ri += 1;
        }
    }
    if let Some(pr) = pol {
        if err != 0 || pr.err.is_some() {
            pr.e.iter_mut().for_each(|e| *e = CNAN3);
        }
    }
    // 大域の位置・方向は最後の面でだけ求める
    if let Some(f) = last_frame {
        p = add(f.o, mv(&f.r, loc.0));
        k = mv(&f.r, loc.1);
    }
    RayOut {
        xyz: loc.0,
        lmn: loc.1,
        opl,
        err,
        tir,
        vig,
        p,
        k,
        lens_opd,
    }
}

/// 面 d の局所座標の点 pl での単位法線（+z 寄り。追跡の内側と同じ式。理想レンズ・Jones・座標ブレークなど平らな面は (0, 0, 1)）
pub(crate) fn local_normal(d: &Surface, pl: V3) -> V3 {
    if matches!(
        d.typ,
        SurfType::CoordBrk | SurfType::Paraxial | SurfType::Jones | SurfType::User
    ) {
        return [0.0, 0.0, 1.0];
    }
    let v = match surf_kind(d).norm {
        Norm::Free => {
            let (_, zx, zy) = crate::freeform::sag_xy(d, pl[0], pl[1]);
            [-zx, -zy, 1.0]
        }
        Norm::Toroidal => {
            let (_, zx, zy) = sag_toroidal(d.c, d.k, &d.asph, d.cx, pl[0], pl[1]);
            [-zx, -zy, 1.0]
        }
        Norm::Grad9Tilt => {
            let (_, zx, zy) = crate::grin::grad9_sag(d, pl[0], pl[1]);
            [-zx, -zy, 1.0]
        }
        Norm::Fresnel => fresnel_normal(d, pl[0], pl[1]),
        Norm::Asph => {
            let (_, dz) = sag(d.c, d.k, &d.asph, pl[0] * pl[0] + pl[1] * pl[1]);
            [-2.0 * dz * pl[0], -2.0 * dz * pl[1], 1.0]
        }
        Norm::Flat => [0.0, 0.0, 1.0],
        Norm::Conic => [-d.c * pl[0], -d.c * pl[1], 1.0 - d.c * (1.0 + d.k) * pl[2]],
    };
    normalize(v)
}
