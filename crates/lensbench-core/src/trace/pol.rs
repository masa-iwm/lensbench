//! 偏光追跡の面ごとの処理: 境界の s・p 係数・Jones 行列・複屈折材の板（入口・出口・貼り合わせ）で電場を変える。
//! 追跡のカーネル（[`crate::batch`]）が面ごとに [`pol_step`] を呼ぶ。

use crate::coating::{Iface, C64};
use crate::math::*;
use crate::system::System;
use crate::tol::PARALLEL_SIN;
use crate::Error;

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
    /// 板の出口のすぐ後（厚さ 0、間に面なし）に次の板の入口が来る貼り合わせ: 次の板の
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
        let es = if s > PARALLEL_SIN {
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
    if !(s > PARALLEL_SIN) {
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
fn pol_surface(
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
            if !(norm(sv) > PARALLEL_SIN) {
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

/// 複屈折材の板の境界の係数: 板の中の電場を常光の成分（k と結晶軸を含む面に垂直）と異常光の成分に
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
    if !(sn > PARALLEL_SIN) {
        return pol_surface(ps, e, kin, kl, nv, cosi, false);
    }
    let ee = scale(ep, 1.0 / sn);
    let eo = normalize(cross(k, ee));
    let nth = 1.0 / (ca * ca / (no * no) + (1.0 - ca * ca) / (ne * ne)).sqrt();
    let mut sv = cross(kin, nv);
    if !(norm(sv) > PARALLEL_SIN) {
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
    if !(sn > PARALLEL_SIN) {
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

/// 複屈折材の板どうしの貼り合わせの境界: 前の板（`a`、板の中の方向 kin）の電場を o・e の成分に分け、
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
    if !(norm(sv) > PARALLEL_SIN) {
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
    // 境界の係数を常光・異常光で分ける板の面: (結晶軸, no, ne, 板の中の方向, 入る面か)
    let mut xc = None;
    // 貼り合わせ: 前の板の出口で次の板の o・e へ 1 回で移す
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
