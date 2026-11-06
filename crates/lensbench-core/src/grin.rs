//! 屈折率分布（GRIN）の媒質: OpticStudio の GRINSUR8（Gradium、軸方向の分布）と GRINSUR9（Gradient 9、
//! 径方向の分布）。面の後ろ（次の面まで）の媒質の屈折率が場所で変わり、光線は曲がりながら進む。
//!
//! - Gradium: 基準波長の屈折率 n_ref = Σ_{i=0..11} n_i ((z′ + Δz) / z_max)^i（PROFILE.GRD の 13 行の組）。
//!   z′ + Δz = t_x (x − d_x) + t_y (y − d_y) + t_z (z + Δz)、t_z = √(1 − t_x² − t_y²)（Δz は傾ける前の軸方向の
//!   ずれ。OS の Ref n と追跡に一致）。ほかの波長は Gradient 5 の分散
//!   n² = n_ref² + Σ_{i=1..3} K_i (λ² − λ_ref²) / (λ² − L_i)、K_i = Σ_j K_ij n_ref^{j−1}、L_i = Σ_j L_ij n_ref^{j−1}
//!   （SGRIN.DAT の 10 行の組）。面の形は標準面
//! - Gradient 9: n = n0 (1 − (A/2) r²)、A = (K0 + K1/λ² + K2/λ⁴)²、n0 = B + C/λ²（λ は µm、GRADIENT_9.DAT の
//!   6 行の組）。面の形は標準面 + x tan α + y tan β
//!
//! 媒質の中は光線の方程式 d²R/dt² = n∇n（dt = ds / n）を Sharma の Runge–Kutta（Appl. Opt. 21, 984 (1982)）で
//! 解く。刻みは 1 歩の誤差が 1e-12 以下になるよう自動で変える（初めは面の Δt）。OpticStudio は Δt ごとに
//! 解くので、Δt が粗いと刻みの誤差を含む（Δt 0.852 mm の SELFOC の玉で光線の位置 2e-5 mm）。ここでは
//! 収束した値を出す（2026-11-08 決定）

use crate::math::*;
use crate::surface::{SurfType, Surface};
use crate::trace::{sag, sag_toroidal};
use crate::{invalid, Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 媒質の中の刻みの上限（光路の長さ、レンズの単位）。刻みは誤差に合わせてこの範囲で自動で変える
pub const MAX_STEP: f64 = 1.0;

/// Gradient 5 の分散（SGRIN.DAT の組）
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Sgrin {
    pub name: String,
    /// 波長の範囲 [µm]
    pub lmin: f64,
    pub lmax: f64,
    /// 基準波長 [µm]
    pub lref: f64,
    /// K_ij（i = 1..3、j = 1..K_MAX）
    pub k: [Vec<f64>; 3],
    /// L_ij（i = 1..3、j = 1..L_MAX）
    pub l: [Vec<f64>; 3],
}

/// Gradium の分布（PROFILE.GRD の組）
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GradiumProfile {
    pub name: String,
    /// 分散の名前（SGRIN.DAT の組）
    pub family: String,
    /// 玉の素材の厚さ z_max
    pub zmax: f64,
    /// n_0 … n_11
    pub coef: Vec<f64>,
    /// 読んだファイルの名前（PROFILE.GRD など。.zmx の COFN に書く）
    pub file: String,
}

/// Gradient 9 の材料（GRADIENT_9.DAT の組）
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Grad9Mat {
    pub b: f64,
    pub c: f64,
    pub k0: f64,
    pub k1: f64,
    pub k2: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[allow(clippy::large_enum_variant)]
pub enum GrinKind {
    Gradium {
        prof: GradiumProfile,
        disp: Sgrin,
        /// 分布の上の位置のずれ Δz
        dz: f64,
        /// 公差の項 d_x d_y t_x t_y（近軸では使わない）
        dec: [f64; 4],
        /// 端の扱い: 0 両端で止める、1 左（z′ + Δz < 0）だけ止める、2 右だけ止める、3 止めない
        /// （止めない側は端の屈折率の一様な媒質）
        cap: u8,
    },
    Grad9 {
        mat: Grad9Mat,
        /// 形の傾きの項 tan α、tan β
        tan: [f64; 2],
    },
}

/// 面の後ろの屈折率分布
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Grin {
    pub kind: GrinKind,
    /// OpticStudio の刻み Δt（パラメータ 1）。ここでは刻みの初めの値にだけ使う
    pub dt: f64,
}

impl Grin {
    /// 面タイプ
    pub fn surf_type(&self) -> SurfType {
        match self.kind {
            GrinKind::Gradium { .. } => SurfType::Gradium,
            GrinKind::Grad9 { .. } => SurfType::Gradient9,
        }
    }

    /// 刻みの初め（光路）
    pub fn step(&self) -> f64 {
        if self.dt > 0.0 {
            self.dt.min(MAX_STEP)
        } else {
            MAX_STEP
        }
    }

    /// 媒質の局所座標の点 p、波長 lam [µm] の屈折率 n と n∇n。分布の範囲の外（端を止めた側）は None
    pub fn eval(&self, lam: f64, p: V3) -> Option<(f64, V3)> {
        self.eval_opt(lam, p, true)
    }

    /// strict が false なら、端を止めた側の外も分布の多項式をそのまま延ばす（数値積分の途中の段の点のため。
    /// 採った点は strict で確かめる）
    #[inline]
    fn eval_opt(&self, lam: f64, p: V3, strict: bool) -> Option<(f64, V3)> {
        match &self.kind {
            GrinKind::Gradium {
                prof,
                disp,
                dz,
                dec,
                cap,
            } => {
                // Δz は傾ける前の軸方向のずれ: z′ = tx(x−dx) + ty(y−dy) + tz(z+Δz)（OS の Ref n・追跡と一致）
                let [dx, dy, tx, ty] = *dec;
                let tz = (1.0 - tx * tx - ty * ty).sqrt();
                let zp = tx * (p[0] - dx) + ty * (p[1] - dy) + tz * (p[2] + dz) - dz;
                let (n2, dn2) = gradium_n2(prof, disp, *dz, *cap, lam, zp, strict)?;
                if !(n2 > 0.0) {
                    return None;
                }
                let n = n2.sqrt();
                let g = 0.5 * dn2;
                Some((n, [g * tx, g * ty, g * tz]))
            }
            GrinKind::Grad9 { mat, .. } => {
                let (n0, a) = grad9_n0a(mat, lam);
                let r2 = p[0] * p[0] + p[1] * p[1];
                let n = n0 * (1.0 - 0.5 * a * r2);
                if !(n > 0.0) {
                    return None;
                }
                let g = -n * n0 * a;
                Some((n, [g * p[0], g * p[1], 0.0]))
            }
        }
    }

    /// 軸の周りに回転対称か（Gradium の公差の項・Gradient 9 の形の傾きがない）
    pub fn symmetric(&self) -> bool {
        match &self.kind {
            GrinKind::Gradium { dec, .. } => dec.iter().all(|&v| v == 0.0),
            GrinKind::Grad9 { tan, .. } => tan[0] == 0.0 && tan[1] == 0.0,
        }
    }

    /// 頂点（局所座標の原点）の屈折率
    pub fn vertex_index(&self, lam: f64) -> Option<f64> {
        self.eval(lam, [0.0; 3]).map(|v| v.0)
    }

    /// 頂点の基準波長の屈折率（Gradium のパラメータ 3。Gradient 9 は None）
    pub fn ref_index(&self) -> Option<f64> {
        match &self.kind {
            GrinKind::Gradium { prof, dz, dec, .. } => {
                let [dx, dy, tx, ty] = *dec;
                let tz = (1.0 - tx * tx - ty * ty).sqrt();
                Some(profile_nref(prof, (tz * dz - tx * dx - ty * dy) / prof.zmax).0)
            }
            GrinKind::Grad9 { .. } => None,
        }
    }

    /// 近軸の受け渡し: 局所 z = z0 の面で高さ y・傾き u（屈折の後）の光線を、z0 から z0 + t まで
    /// 進めた高さ・傾き・屈折率（z0 は座標ブレークを挟んだ続きの区間。傾き・偏心は見ない）（sgn は展開した向き。直進なら y + sgn t u）。Gradium の分布の軸の偏心・傾きは
    /// 軸上の屈折率の変化としてだけ入れ、Gradient 9 の形の傾きは見ない。傾けない系では小さい実光線の極限と
    /// 一致する（OpticStudio の近軸は G8A で 7e-7、G9A で 2e-6 ずれる。近似とみて合わせない）
    pub fn paraxial(
        &self,
        lam: f64,
        y: f64,
        u: f64,
        z0: f64,
        t: f64,
        sgn: f64,
    ) -> Option<(f64, f64, f64)> {
        match &self.kind {
            GrinKind::Gradium {
                prof,
                disp,
                dz,
                cap,
                dec,
            } => {
                // n u は一定、y は ∫ dz / n(z) で進む。分布の軸の偏心・傾きは軸上の屈折率にだけ効かせる
                let [dx, dy, tx, ty] = *dec;
                let tz = (1.0 - tx * tx - ty * ty).sqrt();
                let c0 = -tx * dx - ty * dy + (tz - 1.0) * dz;
                let nz = |z: f64| {
                    gradium_n2(prof, disp, *dz, *cap, lam, c0 + tz * z, true).map(|v| v.0.sqrt())
                };
                let nv = nz(z0)?;
                let ne = nz(z0 + t)?;
                let np = 64usize;
                let (gx, gw) = crate::analysis::gauss_legendre(8);
                let mut s = 0.0;
                for i in 0..np {
                    for (x, wt) in gx.iter().zip(&gw) {
                        let z = z0 + t * (i as f64 + 0.5 + 0.5 * x) / np as f64;
                        s += 0.5 * wt / nz(z)?;
                    }
                }
                let s = s * t / np as f64;
                Some((y + sgn * nv * u * s, nv * u / ne, ne))
            }
            GrinKind::Grad9 { mat, .. } => {
                let (n0, a) = grad9_n0a(mat, lam);
                let l = sgn * t;
                let g = a.sqrt();
                if g == 0.0 {
                    return Some((y + l * u, u, n0));
                }
                let (s, c) = (g * l).sin_cos();
                Some((y * c + u / g * s, -y * g * s + u * c, n0))
            }
        }
    }
}

/// Gradient 9 の n0 と A（波長 µm）
fn grad9_n0a(m: &Grad9Mat, lam: f64) -> (f64, f64) {
    let l2 = lam * lam;
    let n0 = m.b + m.c / l2;
    let r = m.k0 + m.k1 / l2 + m.k2 / (l2 * l2);
    (n0, r * r)
}

/// Gradium の n_ref と d n_ref / du（u = (z′ + Δz) / z_max）
fn profile_nref(p: &GradiumProfile, u: f64) -> (f64, f64) {
    poly_d(&p.coef, u)
}

/// Gradium の n² と d(n²)/dz′（波長 lam µm、局所の z′）。端を止めた側の外は None
fn gradium_n2(
    p: &GradiumProfile,
    s: &Sgrin,
    dz: f64,
    cap: u8,
    lam: f64,
    zp: f64,
    strict: bool,
) -> Option<(f64, f64)> {
    let mut u = (zp + dz) / p.zmax;
    let mut slope = 1.0 / p.zmax;
    let tol = 1e-12;
    // 止める端（0: 両方、1: 左、2: 右）
    let (lb, rb) = (cap == 0 || cap == 1, cap == 0 || cap == 2);
    if u < 0.0 {
        if lb {
            if u < -tol && strict {
                return None;
            }
        } else {
            u = 0.0;
            slope = 0.0;
        }
    } else if u > 1.0 {
        if rb {
            if u > 1.0 + tol && strict {
                return None;
            }
        } else {
            u = 1.0;
            slope = 0.0;
        }
    }
    let (nr, dnr_du) = profile_nref(p, u);
    let dnr = dnr_du * slope;
    let l2 = lam * lam;
    let q = l2 - s.lref * s.lref;
    let mut n2 = nr * nr;
    let mut d = 2.0 * nr * dnr;
    for i in 0..3 {
        let (k, dk) = poly_d(&s.k[i], nr);
        let (li, dl) = poly_d(&s.l[i], nr);
        let den = l2 - li;
        n2 += k * q / den;
        d += (dk * q / den + k * q * dl / (den * den)) * dnr;
    }
    Some((n2, d))
}

/// Σ_j c_j x^j と d/dx
fn poly_d(c: &[f64], x: f64) -> (f64, f64) {
    let (mut v, mut dv) = (0.0, 0.0);
    for &a in c.iter().rev() {
        dv = dv * x + v;
        v = v * x + a;
    }
    (v, dv)
}

/// Gradient 9 の面の形 z = 標準面 + x tan α + y tan β と (∂z/∂x, ∂z/∂y)
pub fn grad9_sag(d: &Surface, x: f64, y: f64) -> (f64, f64, f64) {
    let (z, dz) = sag(d.c, d.k, &[], x * x + y * y);
    let t = match d.grin.as_deref().map(|g| &g.kind) {
        Some(GrinKind::Grad9 { tan, .. }) => *tan,
        _ => [0.0; 2],
    };
    (
        z + x * t[0] + y * t[1],
        2.0 * dz * x + t[0],
        2.0 * dz * y + t[1],
    )
}

/// Gradient 9 の形に傾きの項があるか
pub fn grad9_tilted(d: &Surface) -> bool {
    matches!(d.grin.as_deref().map(|g| &g.kind), Some(GrinKind::Grad9 { tan, .. }) if tan[0] != 0.0 || tan[1] != 0.0)
}

/// GRIN の媒質の次の面として交点を求められる面の矢高（局所 (x, y)）。対応しない面タイプは None。
/// 座標ブレークは平面（その後ろも同じ媒質が続く）
pub(crate) fn next_sag(d: &Surface) -> Option<impl Fn(f64, f64) -> f64 + '_> {
    next_ok(d.typ).then_some(move |x: f64, y: f64| match d.typ {
        SurfType::Fresnel | SurfType::Jones | SurfType::CoordBrk => 0.0,
        SurfType::CylFresnel => sag(d.c, d.k, &d.asph, y * y).0,
        SurfType::Toroidal => sag_toroidal(d.c, d.k, &d.asph, d.cx, x, y).0,
        SurfType::ZernikeStd | SurfType::ZernikeFringe | SurfType::GridSag => {
            crate::freeform::sag_xy(d, x, y).0
        }
        SurfType::Gradient9 => grad9_sag(d, x, y).0,
        SurfType::Gradium | SurfType::Standard => sag(d.c, d.k, &[], x * x + y * y).0,
        _ => sag(d.c, d.k, &d.asph, x * x + y * y).0,
    })
}

/// GRIN の媒質の次の面にできる面タイプか（[`next_sag`]）
pub(crate) fn next_ok(t: SurfType) -> bool {
    matches!(
        t,
        SurfType::Standard
            | SurfType::EvenAsph
            | SurfType::Binary2
            | SurfType::Toroidal
            | SurfType::ZernikeStd
            | SurfType::ZernikeFringe
            | SurfType::GridSag
            | SurfType::Fresnel
            | SurfType::CylFresnel
            | SurfType::Jones
            | SurfType::Gradium
            | SurfType::Gradient9
            | SurfType::CoordBrk
    )
}

/// 媒質の中を進んだ結果（媒質の面の局所座標）
pub(crate) struct Exit {
    pub p: V3,
    /// 単位の方向
    pub k: V3,
    pub opl: f64,
    /// 出口の屈折率
    pub n: f64,
}

/// Sharma の 1 歩（R, T = n dR/ds、刻み h = Δs / n。e0 は r での [`Grin::eval`]）。光路長は Simpson 則（段の 3 点の n²）
#[inline]
fn sharma(g: &Grin, lam: f64, r: V3, t: V3, h: f64, e0: (f64, V3)) -> Option<(V3, V3, f64)> {
    let (n0, d0) = e0;
    let a = scale(d0, h);
    let r1 = add(add(r, scale(t, 0.5 * h)), scale(a, h / 8.0));
    let (n1, d1) = g.eval_opt(lam, r1, false)?;
    let b = scale(d1, h);
    let r2 = add(add(r, scale(t, h)), scale(b, 0.5 * h));
    let (n2, d2) = g.eval_opt(lam, r2, false)?;
    let c = scale(d2, h);
    let rn = add(r, scale(add(t, scale(add(a, scale(b, 2.0)), 1.0 / 6.0)), h));
    let tn = add(t, scale(add(add(a, scale(b, 4.0)), c), 1.0 / 6.0));
    let opl = h / 6.0 * (n0 * n0 + 4.0 * n1 * n1 + n2 * n2);
    Some((rn, tn, opl))
}

/// 刻み h の 1 歩を、1 歩と半歩 2 つから Richardson で補正した値と、補正前の差（誤差の見積もり）
#[inline]
fn step(g: &Grin, lam: f64, r: V3, t: V3, h: f64, e0: (f64, V3)) -> Option<(V3, V3, f64, f64)> {
    let (ra, ta, oa) = sharma(g, lam, r, t, h, e0)?;
    let (r1, t1, o1) = sharma(g, lam, r, t, 0.5 * h, e0)?;
    let e1 = g.eval_opt(lam, r1, false)?;
    let (rb, tb, o2) = sharma(g, lam, r1, t1, 0.5 * h, e1)?;
    let ob = o1 + o2;
    let err = norm(sub(ra, rb)) + norm(sub(ta, tb)) / e0.0;
    Some((
        add(rb, scale(sub(rb, ra), 1.0 / 15.0)),
        add(tb, scale(sub(tb, ta), 1.0 / 15.0)),
        ob + (ob - oa) / 15.0,
        err,
    ))
}

/// 1 歩の補正前の誤差の許容（レンズの単位）。補正後の誤差はこれよりずっと小さく、玉 1 つを通した光線で
/// 1e-9 以下（カタログの玉 3 つで、許容 1e-14 の結果と比べて 6e-11〜9e-10）
const TOL: f64 = 1e-8;

/// 媒質の面の局所座標の点 p（面の上）から単位の方向 k に、媒質の中を進み、関数 f（次の面の手前と向こうで
/// 符号が変わる）の符号が変わる点（次の面との交点）まで追う。範囲の外に出る・交わらないときは None。
/// 刻みは誤差の見積もりで自動で変える（初めは Δt、上限は max_len / 4 と [`MAX_STEP`]）。交点は、歩の両端の
/// 位置と方向の 3 次の補間で見当をつけ、その刻みの歩を解き直して詰める
pub(crate) fn propagate(
    g: &Grin,
    lam: f64,
    p: V3,
    k: V3,
    f: impl Fn(V3) -> f64,
    max_len: f64,
) -> Option<Exit> {
    propagate_with(g, lam, p, k, f, max_len, true)
}

/// [`propagate`]。fast が false なら軸方向の分布の速い経路を使わない（試験で比べるため）
fn propagate_with(
    g: &Grin,
    lam: f64,
    p: V3,
    k: V3,
    f: impl Fn(V3) -> f64,
    max_len: f64,
    fast: bool,
) -> Option<Exit> {
    let mut e = g.eval(lam, p)?;
    let mut r = p;
    let mut t = scale(k, e.0);
    let mut opl = 0.0;
    let mut fr = f(r);
    if fr == 0.0 {
        return Some(Exit {
            p,
            k,
            opl: 0.0,
            n: e.0,
        });
    }
    let s0 = fr.signum();
    if fast {
        if let Some(x) = axial(g, lam, p, k, &f, fr, max_len) {
            return x;
        }
    }
    let hmax = MAX_STEP.min(0.25 * max_len).max(1e-9);
    let mut hs = g.step().min(hmax);
    let hmin = hmax * 1e-9;
    let mut len = 0.0;
    for _ in 0..1_000_000 {
        let h = hs / e.0;
        let (r2, t2, o, err) = step(g, lam, r, t, h, e)?;
        // 刻みの制御（5 次の誤差）
        let fac = if err > 0.0 {
            (0.9 * (TOL / err).powf(0.2)).clamp(0.2, 4.0)
        } else {
            4.0
        };
        if err > TOL && hs > hmin {
            hs *= fac;
            continue;
        }
        let f2 = f(r2);
        if f2 == 0.0 || f2.signum() == -s0 {
            return cross(g, lam, r, t, e, h, (fr, r2, t2, f2), &f)
                .map(|(x, o)| Exit { opl: opl + o, ..x });
        }
        r = r2;
        t = t2;
        opl += o;
        e = g.eval(lam, r)?;
        if f2.is_finite() {
            fr = f2;
        }
        len += hs;
        if len > max_len {
            return None;
        }
        hs = (hs * fac).min(hmax);
    }
    None
}

/// 8 点の Gauss–Legendre（[-1, 1]）
const GL8: [(f64, f64); 8] = [
    (-0.960_289_856_497_536_2, 0.101_228_536_290_376_26),
    (-0.796_666_477_413_626_7, 0.222_381_034_453_374_47),
    (-0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (-0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_47),
    (0.960_289_856_497_536_2, 0.101_228_536_290_376_26),
];

/// 軸方向の分布（公差の項のない Gradium）の速い経路。n k_x・n k_y が保たれるので、位置と光路長は z の 1 次元の
/// 積分 x(z) = ∫ T_x / T_z dz、OPL = ∫ n² / T_z dz になる（T_z = ±√(n² − T_x² − T_y²)）。z を区間 [`AX_PANEL`]
/// ごとに 8 点の Gauss–Legendre で積み、f の符号が変わる区間の中で根を割線法で詰める。
/// 当てはまらない（Gradium でない、z に沿って進まない、途中で折り返す）ときは None（一般の経路へ）
fn axial(
    g: &Grin,
    lam: f64,
    p: V3,
    k: V3,
    f: &impl Fn(V3) -> f64,
    fr: f64,
    max_len: f64,
) -> Option<Option<Exit>> {
    let GrinKind::Gradium {
        prof,
        disp,
        dz,
        dec,
        cap,
    } = &g.kind
    else {
        return None;
    };
    if dec.iter().any(|&v| v != 0.0) || k[2] == 0.0 {
        return None;
    }
    let n2 = |z: f64, strict: bool| gradium_n2(prof, disp, *dz, *cap, lam, z, strict).map(|v| v.0);
    let Some(m0) = n2(p[2], true) else {
        return Some(None);
    };
    let n0 = m0.sqrt();
    let (tx, ty) = (n0 * k[0], n0 * k[1]);
    let tt = tx * tx + ty * ty;
    let sg = k[2].signum();
    // [za, zb] の積分（x, y, OPL の増分）。折り返す（T_z が 0 になる）なら None
    let integ = |za: f64, zb: f64| -> Option<[f64; 3]> {
        let (c, hw) = (0.5 * (za + zb), 0.5 * (zb - za));
        let mut acc = [0.0; 3];
        for (x, w) in GL8 {
            let m = n2(c + hw * x, false)?;
            let q = m - tt;
            if !(q > 0.0) {
                return None;
            }
            let iz = sg / q.sqrt();
            acc[0] += w * tx * iz;
            acc[1] += w * ty * iz;
            acc[2] += w * m * iz;
        }
        Some(scale(acc, hw))
    };
    let at = |base: (V3, f64), za: f64, z: f64| -> Option<(V3, f64)> {
        let d = integ(za, z)?;
        Some(([base.0[0] + d[0], base.0[1] + d[1], z], base.1 + d[2]))
    };
    let mut cur = (p, 0.0);
    let mut fa = fr;
    let s0 = fr.signum();
    let npan = (max_len / AX_PANEL) as usize + 10;
    for _ in 0..npan {
        let za = cur.0[2];
        let zb = za + sg * AX_PANEL;
        let nb = at(cur, za, zb)?;
        let fb = f(nb.0);
        if fb == 0.0 || fb.signum() == -s0 {
            // [za, zb] の中の根（Illinois）
            let (mut a, mut b, mut ya, mut yb) = (za, zb, fa, fb);
            let mut best = nb;
            let mut side = 0i8;
            for _ in 0..100 {
                if yb == 0.0 || (b - a).abs() <= 1e-15 * (1.0 + b.abs()) {
                    break;
                }
                let c = (a * yb - b * ya) / (yb - ya);
                let nc = at(cur, za, c)?;
                let yc = f(nc.0);
                if !yc.is_finite() {
                    return Some(None);
                }
                best = nc;
                if yc.abs() < 1e-15 {
                    break;
                }
                if yc.signum() == yb.signum() {
                    b = c;
                    yb = yc;
                    if side == -1 {
                        ya *= 0.5;
                    }
                    side = -1;
                } else {
                    a = c;
                    ya = yc;
                    if side == 1 {
                        yb *= 0.5;
                    }
                    side = 1;
                }
            }
            let ze = best.0[2];
            let Some(me) = n2(ze, true) else {
                return Some(None);
            };
            let ne = me.sqrt();
            let tz = sg * (me - tt).max(0.0).sqrt();
            return Some(Some(Exit {
                p: best.0,
                k: scale([tx, ty, tz], 1.0 / ne),
                opl: best.1,
                n: ne,
            }));
        }
        cur = nb;
        if fb.is_finite() {
            fa = fb;
        }
    }
    Some(None)
}

/// 軸方向の分布の速い経路の z の区間
const AX_PANEL: f64 = 0.5;

/// 歩 (r, t) → (r2, t2)（刻み h）の中で f の符号が変わる点を求める。その点の状態と、歩の始めからの光路長
#[allow(clippy::too_many_arguments)]
fn cross(
    g: &Grin,
    lam: f64,
    r: V3,
    t: V3,
    e: (f64, V3),
    h: f64,
    end: (f64, V3, V3, f64),
    f: &impl Fn(V3) -> f64,
) -> Option<(Exit, f64)> {
    let (fr, r2, t2, f2) = end;
    // 3 次の Hermite 補間（dR/dτ = T）の上で f の根を挟み撃ち（Illinois）で求める
    let herm = |s: f64| -> V3 {
        let u = s / h;
        let (u2, u3) = (u * u, u * u * u);
        let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h10 = u3 - 2.0 * u2 + u;
        let h01 = -2.0 * u3 + 3.0 * u2;
        let h11 = u3 - u2;
        add(
            add(scale(r, h00), scale(t, h10 * h)),
            add(scale(r2, h01), scale(t2, h11 * h)),
        )
    };
    let root = |fun: &dyn Fn(f64) -> f64, mut a: f64, mut fa: f64, mut b: f64, mut fb: f64| {
        let mut side = 0i8;
        let mut c = b;
        for _ in 0..100 {
            if fb == 0.0 || (b - a).abs() <= 1e-15 * h {
                return b;
            }
            c = (a * fb - b * fa) / (fb - fa);
            let fc = fun(c);
            if !fc.is_finite() || fc.abs() < 1e-15 {
                return c;
            }
            if fc.signum() == fb.signum() {
                b = c;
                fb = fc;
                if side == -1 {
                    fa *= 0.5;
                }
                side = -1;
            } else {
                a = c;
                fa = fc;
                if side == 1 {
                    fb *= 0.5;
                }
                side = 1;
            }
        }
        c
    };
    let mut tau = root(&|s| f(herm(s)), 0.0, fr, h, f2);
    // 実際の歩で詰める（割線法。数回で 1e-14 に入る）
    let (mut prev, mut fprev): (f64, f64) = (h, f2);
    for _ in 0..30 {
        let (re, te, oe, _) = step(g, lam, r, t, tau, e)?;
        let fe = f(re);
        if !fe.is_finite() {
            return None;
        }
        if fe.abs() < 1e-14 || fe == fprev {
            let (ne, _) = g.eval(lam, re)?;
            return Some((
                Exit {
                    p: re,
                    k: normalize(te),
                    opl: 0.0,
                    n: ne,
                },
                oe,
            ));
        }
        let nt = tau - fe * (tau - prev) / (fe - fprev);
        prev = tau;
        fprev = fe;
        tau = nt;
    }
    None
}

// ------------------------------------------------------------------------------------------- データ

/// GRIN の材料のデータ（PROFILE.GRD・SGRIN.DAT・GRADIENT_9.DAT）
#[derive(Debug, Clone, Default)]
pub struct GrinData {
    pub profiles: HashMap<String, GradiumProfile>,
    pub sgrin: HashMap<String, Sgrin>,
    pub grad9: HashMap<String, Grad9Mat>,
}

fn nums(line: &str) -> Result<Vec<f64>> {
    line.split_whitespace()
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("GRIN のデータの数 {v:?}")))
        })
        .collect()
}

/// 空行を除いた行
fn lines(txt: &str) -> Vec<&str> {
    txt.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// PROFILE.GRD（13 行の組: 名前 分散の名前 z_max 密度 未使用、n_0 … n_11）
pub fn parse_grd(txt: &str, file: &str) -> Result<HashMap<String, GradiumProfile>> {
    let ls = lines(txt);
    let mut out = HashMap::new();
    for b in ls.chunks(13) {
        if b.len() < 13 {
            invalid!("GRD の組の行が足りない（{}）", b[0]);
        }
        let h: Vec<&str> = b[0].split_whitespace().collect();
        if h.len() < 3 {
            invalid!("GRD の見出しの行 {:?}", b[0]);
        }
        let zmax: f64 = h[2]
            .parse()
            .map_err(|_| Error::Invalid(format!("GRD の z_max {:?}", h[2])))?;
        let mut coef = Vec::with_capacity(12);
        for l in &b[1..] {
            coef.extend(nums(l)?);
        }
        if coef.len() != 12 || !(zmax > 0.0) {
            invalid!("GRD の組 {}", h[0]);
        }
        out.insert(
            h[0].to_string(),
            GradiumProfile {
                name: h[0].to_string(),
                family: h[1].to_string(),
                zmax,
                coef,
                file: file.to_string(),
            },
        );
    }
    Ok(out)
}

/// SGRIN.DAT（10 行の組: 名前、λmin λmax、λref、K_MAX L_MAX、K1j、K2j、K3j、L1j、L2j、L3j）
pub fn parse_sgrin(txt: &str) -> Result<HashMap<String, Sgrin>> {
    let ls = lines(txt);
    let mut out = HashMap::new();
    for b in ls.chunks(10) {
        if b.len() < 10 {
            invalid!("SGRIN の組の行が足りない（{}）", b[0]);
        }
        let name = b[0].split_whitespace().next().unwrap_or("").to_string();
        let r = nums(b[1])?;
        let lref = nums(b[2])?;
        let m = nums(b[3])?;
        if r.len() < 2 || lref.is_empty() || m.len() < 2 {
            invalid!("SGRIN の組 {name}");
        }
        let (km, lm) = (m[0] as usize, m[1] as usize);
        let row = |i: usize, n: usize| -> Result<Vec<f64>> {
            let v = nums(b[i])?;
            if v.len() != n {
                invalid!(
                    "SGRIN の組 {name} の {} 行目（{} 個、{n} 個のはず）",
                    i + 1,
                    v.len()
                );
            }
            Ok(v)
        };
        let s = Sgrin {
            name: name.clone(),
            lmin: r[0],
            lmax: r[1],
            lref: lref[0],
            k: [row(4, km)?, row(5, km)?, row(6, km)?],
            l: [row(7, lm)?, row(8, lm)?, row(9, lm)?],
        };
        out.insert(name, s);
    }
    Ok(out)
}

/// GRADIENT_9.DAT（6 行の組: 名前、B、C、K0、K1、K2）
pub fn parse_grad9(txt: &str) -> Result<HashMap<String, Grad9Mat>> {
    let ls = lines(txt);
    let mut out = HashMap::new();
    for b in ls.chunks(6) {
        if b.len() < 6 {
            invalid!("GRADIENT_9 の組の行が足りない（{}）", b[0]);
        }
        let v: Vec<f64> = b[1..]
            .iter()
            .map(|l| nums(l).map(|x| x.first().copied().unwrap_or(f64::NAN)))
            .collect::<Result<_>>()?;
        out.insert(
            b[0].to_string(),
            Grad9Mat {
                b: v[0],
                c: v[1],
                k0: v[2],
                k1: v[3],
                k2: v[4],
            },
        );
    }
    Ok(out)
}

fn find_file(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let want = name.to_uppercase();
    for d in dirs {
        let Ok(rd) = crate::vfs::read_dir(d) else {
            continue;
        };
        for e in rd {
            if e.file_name()
                .map(|n| n.to_string_lossy().to_uppercase())
                .unwrap_or_default()
                == want
            {
                return Some(e);
            }
        }
    }
    None
}

fn read(p: &Path) -> Result<String> {
    crate::glass::read_text(p).map_err(|e| Error::Io(format!("{}: {e}", p.display())))
}

impl GrinData {
    /// ガラスの検索パスから読む（ないファイルは空）。grd は Gradium の分布のファイル名（既定 PROFILE.GRD）
    pub fn load(dirs: &[PathBuf], grd: &str) -> Result<GrinData> {
        let mut g = GrinData::default();
        if let Some(p) = find_file(dirs, grd) {
            g.profiles = parse_grd(&read(&p)?, grd)?;
        }
        if let Some(p) = find_file(dirs, "SGRIN.DAT") {
            g.sgrin = parse_sgrin(&read(&p)?)?;
        }
        if let Some(p) = find_file(dirs, "GRADIENT_9.DAT") {
            g.grad9 = parse_grad9(&read(&p)?)?;
        }
        Ok(g)
    }

    /// Gradium の媒質（分布の名前 name）
    pub fn gradium(&self, name: &str, dz: f64, dec: [f64; 4], cap: u8, dt: f64) -> Result<Grin> {
        let prof = self.profiles.get(name).ok_or_else(|| {
            Error::Invalid(format!("Gradium の分布 {name} がない（PROFILE.GRD）"))
        })?;
        let disp = self.sgrin.get(&prof.family).ok_or_else(|| {
            Error::Invalid(format!(
                "Gradium の分布 {name} の分散 {} がない（SGRIN.DAT）",
                prof.family
            ))
        })?;
        if cap > 3 {
            invalid!("Gradium の端の扱い {cap}");
        }
        if dec[2] * dec[2] + dec[3] * dec[3] >= 1.0 {
            invalid!("Gradium の分布の軸の傾き {} {}", dec[2], dec[3]);
        }
        Ok(Grin {
            kind: GrinKind::Gradium {
                prof: prof.clone(),
                disp: disp.clone(),
                dz,
                dec,
                cap,
            },
            dt,
        })
    }

    /// Gradient 9 の媒質（材料 name）
    pub fn grad9(&self, name: &str, tan: [f64; 2], dt: f64) -> Result<Grin> {
        let mat = self.grad9.get(name).ok_or_else(|| {
            Error::Invalid(format!("Gradient 9 の材料 {name} がない（GRADIENT_9.DAT）"))
        })?;
        Ok(Grin {
            kind: GrinKind::Grad9 { mat: *mat, tan },
            dt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g9() -> Grin {
        Grin {
            kind: GrinKind::Grad9 {
                mat: Grad9Mat {
                    b: 1.6,
                    c: 0.01,
                    k0: 0.3,
                    k1: 0.004,
                    k2: 0.0002,
                },
                tan: [0.0; 2],
            },
            dt: 0.5,
        }
    }

    const GRD: &str =
        "LBTG1 LBTGSF 6.000 4.5 0\n1.75\n-0.12\n0.03\n-0.01\n0.004\n0\n0\n0\n0\n0\n0\n0\n";
    const SGRIN: &str = "LBTGSF\n0.4 1.0\n0.55\n3 2\n0.0331 0 -0.0331\n0.5 -0.2 0\n0 0 0\n0.0095 0.0003\n100 0\n1 0\n";

    fn gradium(dec: [f64; 4]) -> Grin {
        let d = GrinData {
            profiles: parse_grd(GRD, "LBTEST.GRD").unwrap(),
            sgrin: parse_sgrin(SGRIN).unwrap(),
            grad9: HashMap::new(),
        };
        d.gradium("LBTG1", 1.5, dec, 0, 0.1).unwrap()
    }

    /// Gradium の n² の z の微分は差分と合い、基準波長では分散の項が消える
    #[test]
    fn gradium_index_and_gradient() {
        let g = gradium([0.0; 4]);
        let (n, _) = g.eval(0.55, [0.0; 3]).unwrap();
        // u = 1.5 / 6 = 0.25
        let u: f64 = 0.25;
        let nr = 1.75 - 0.12 * u + 0.03 * u * u - 0.01 * u.powi(3) + 0.004 * u.powi(4);
        assert!((n - nr).abs() < 1e-15);
        assert_eq!(g.ref_index(), Some(nr));
        let g = gradium([0.1, -0.05, 0.01, 0.02]);
        let p = [0.4, -0.3, 1.2];
        let (n, d) = g.eval(0.48, p).unwrap();
        for i in 0..3 {
            let h = 1e-6;
            let (mut a, mut b) = (p, p);
            a[i] += h;
            b[i] -= h;
            let dn = (g.eval(0.48, a).unwrap().0 - g.eval(0.48, b).unwrap().0) / (2.0 * h);
            assert!((n * dn - d[i]).abs() < 1e-9, "{i}");
        }
        // 範囲の外（z′ + Δz < 0）は止める
        assert!(g.eval(0.55, [0.0, 0.0, -1.6]).is_none());
    }

    /// 軸方向の分布の速い経路は一般の経路（Runge–Kutta）と合う（曲がった出口の面）
    #[test]
    fn axial_path_matches_the_general_path() {
        let g = gradium([0.0; 4]);
        let (c2, t) = (-1.0 / 20.0, 3.0);
        let f = |q: V3| {
            let r2 = q[0] * q[0] + q[1] * q[1];
            q[2] - t - sag(c2, 0.0, &[], r2).0
        };
        for (p, k) in [
            ([0.0, 2.0, 0.3], normalize([0.0, -0.15, 1.0])),
            ([1.0, -1.5, 0.2], normalize([0.05, 0.1, 1.0])),
        ] {
            let a = propagate_with(&g, 0.4861327, p, k, f, 100.0, true).unwrap();
            let b = propagate_with(&g, 0.4861327, p, k, f, 100.0, false).unwrap();
            for i in 0..3 {
                assert!((a.p[i] - b.p[i]).abs() < 1e-9, "{:?} {:?}", a.p, b.p);
                assert!((a.k[i] - b.k[i]).abs() < 1e-9);
            }
            assert!((a.opl - b.opl).abs() < 1e-9);
            assert!(f(a.p).abs() < 1e-13);
        }
    }

    /// 径方向の分布の勾配は差分と合う
    #[test]
    fn gradient_matches_differences() {
        let g = g9();
        let p = [0.3, -0.2, 0.7];
        let (n, d) = g.eval(0.55, p).unwrap();
        for i in 0..3 {
            let h = 1e-6;
            let mut a = p;
            let mut b = p;
            a[i] += h;
            b[i] -= h;
            let dn = (g.eval(0.55, a).unwrap().0 - g.eval(0.55, b).unwrap().0) / (2.0 * h);
            assert!((n * dn - d[i]).abs() < 1e-9, "{i}: {} {}", n * dn, d[i]);
        }
    }

    /// 軸に平行に入った近軸光線は、径方向の分布の中で y cos(√A z) で進む（厳密解と比べる）
    #[test]
    fn selfoc_meridional_ray_follows_the_cosine() {
        let g = g9();
        let lam = 0.6;
        let (n0, a) = grad9_n0a(
            match &g.kind {
                GrinKind::Grad9 { mat, .. } => mat,
                _ => unreachable!(),
            },
            lam,
        );
        let y0 = 1e-4;
        let t = 3.0;
        let e = propagate(
            &g,
            lam,
            [0.0, y0, 0.0],
            [0.0, 0.0, 1.0],
            |q| q[2] - t,
            100.0,
        )
        .unwrap();
        let gg = a.sqrt();
        assert!((e.p[1] - y0 * (gg * t).cos()).abs() < 1e-10, "{}", e.p[1]);
        assert!((e.p[2] - t).abs() < 1e-13);
        // 光路長は n0 t に近い（y が小さい）
        assert!((e.opl - n0 * t).abs() < 1e-7, "{} {}", e.opl, n0 * t);
        // 近軸の受け渡しも同じ
        let (y, u, _) = g.paraxial(lam, y0, 0.0, 0.0, t, 1.0).unwrap();
        assert!((y - y0 * (gg * t).cos()).abs() < 1e-15);
        assert!((u + y0 * gg * (gg * t).sin()).abs() < 1e-15);
    }
}
