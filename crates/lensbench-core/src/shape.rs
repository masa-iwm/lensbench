//! 面の形（[`Shape`]）: 矢高と勾配、法線、交点の求め方、近軸の曲率（KRN-05）。
//!
//! 形に依る計算はここに集める。[`Surface::shape`] は今の平たい [`Surface`] の値から形を借りて作る見方で、
//! 面の種類ごとに `asph` などの値の意味（偶数次非球面の係数、Odd Asphere の β、Tilted の tan、Irregular の偏心・
//! 傾き・Zernike 風の項）をここで分ける。面の働き（屈折・反射・理想レンズ・Jones など）は [`crate::action`]。
//!
//! 追跡の内側（[`crate::batch`]）は面ごとに [`SurfKind`] を前計算し、回転対称の面は光線の組のまま、
//! それ以外は光線ごとに [`Shape::isect`]・[`Shape::sag`] を呼ぶ（どれも enum の静的な分岐）。
//!
//! 結果は面の種類ごとに書いていた以前の計算と同じビットにしている。そのため、形に効かない残りの値で求め方が
//! 変わる従来の扱い（[`Shape::isect_kind`] の `filled`、[`Shape::Tilted`] の `c`・`k`、[`Shape::Zernike`] の
//! `stray_grid`）もそのまま残す。値の置き場を enum に替えるとき（KRN-05 の続き）に見直す

use crate::action::Action;
use crate::freeform::{isect_with, GridSag, ZernSag};
use crate::grin::GrinKind;
use crate::math::*;
use crate::surface::{SurfType, Surface};

/// 偶数次非球面: z = c r² / (1 + √(1 − (1 + k) c² r²)) + Σ a[i] r^{2(i+1)}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Base<'a> {
    pub c: f64,
    pub k: f64,
    /// r², r⁴, … の係数
    pub a: &'a [f64],
}

impl Base<'_> {
    /// 0 でない非球面の項があるか
    pub fn has_terms(&self) -> bool {
        self.a.iter().any(|&v| v != 0.0)
    }
}

/// Irregular（IRREGULA）の形: 偏心・傾けた標準面（c・k）に Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y
/// （ρ は `radius` で正規化、ρ_y は `angle` だけ回した y）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Irregular {
    pub c: f64,
    pub k: f64,
    /// 偏心 x・y
    pub decenter: [f64; 2],
    /// 傾き x・y [deg]
    pub tilt: [f64; 2],
    pub zs: f64,
    pub za: f64,
    pub zc: f64,
    /// 項を回す角度 [deg]
    pub angle: f64,
    /// 正規化半径（面の半直径）
    pub radius: f64,
}

/// 面の形（[`Surface::shape`]）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape<'a> {
    /// 頂点の平面（Fresnel・Jones・理想レンズ・利用者定義の面、座標ブレーク）
    Plane,
    /// 二次曲面（c = 0 は平面）。標準面・Gradium
    Conic {
        c: f64,
        k: f64,
    },
    /// 偶数次非球面。Even Asphere・Binary 2
    EvenAsphere(Base<'a>),
    /// Odd Asphere: 二次曲面（c・k）+ Σ b[i] r^{i+1}
    OddAsphere {
        c: f64,
        k: f64,
        b: &'a [f64],
    },
    /// トロイダル面: YZ 面の曲線（c・k・y², y⁴, … の係数 a）を、頂点から 1/cx 離れた Y 軸に平行な軸の周りに回した面
    /// （cx = 0 は円柱面）
    Toroid {
        c: f64,
        k: f64,
        a: &'a [f64],
        cx: f64,
    },
    /// Zernike Sag: 偶数次非球面 + Zernike の項（Standard か Fringe かは [`ZernSag::fringe`]）。
    /// `stray_grid` は Grid Sag から種類を変えた面に残った格子（形には効かず、従来どおり近軸の曲率にだけ効く）
    Zernike {
        base: Base<'a>,
        terms: Option<&'a ZernSag>,
        stray_grid: Option<&'a GridSag>,
    },
    /// Grid Sag: 偶数次非球面 + Zernike（Standard）の項 + 格子の補間
    GridSag {
        base: Base<'a>,
        terms: Option<&'a ZernSag>,
        grid: Option<&'a GridSag>,
    },
    /// Tilted: 平面 z = x tan α + y tan β。`c`・`k` は形に効かない残りの値で、従来どおり交点のニュートン法の
    /// 初期値（二次曲面との交点）にだけ使う
    Tilted {
        tan: [f64; 2],
        c: f64,
        k: f64,
    },
    /// 標準面 + x tan α + y tan β（Gradient 9 の形。傾きは面の後ろの屈折率分布の値）
    ConicTilted {
        c: f64,
        k: f64,
        tan: [f64; 2],
    },
    /// y だけの偶数次非球面（Cylinder Fresnel の基板）
    Cylinder(Base<'a>),
    Irregular(Irregular),
}

impl Surface {
    /// 面の形（今の値から作る見方）
    pub fn shape(&self) -> Shape<'_> {
        let (c, k) = (self.c, self.k);
        let base = Base {
            c,
            k,
            a: &self.asph,
        };
        let p = |i: usize| self.asph.get(i).copied().unwrap_or(0.0);
        match self.typ {
            SurfType::Standard | SurfType::Gradium => Shape::Conic { c, k },
            SurfType::EvenAsph | SurfType::Binary2 => Shape::EvenAsphere(base),
            SurfType::Toroidal => Shape::Toroid {
                c,
                k,
                a: &self.asph,
                cx: self.cx,
            },
            SurfType::ZernikeStd | SurfType::ZernikeFringe => Shape::Zernike {
                base,
                terms: self.zern.as_deref(),
                stray_grid: self.grid.as_deref(),
            },
            SurfType::GridSag => Shape::GridSag {
                base,
                terms: self.zern.as_deref(),
                grid: self.grid.as_deref(),
            },
            SurfType::OddAsph => Shape::OddAsphere {
                c,
                k,
                b: &self.asph,
            },
            SurfType::TiltSurf => Shape::Tilted {
                tan: [p(0), p(1)],
                c,
                k,
            },
            SurfType::Gradient9 => Shape::ConicTilted {
                c,
                k,
                tan: match self.grin.as_deref().map(|g| &g.kind) {
                    Some(GrinKind::Grad9 { tan, .. }) => *tan,
                    _ => [0.0; 2],
                },
            },
            SurfType::CylFresnel => Shape::Cylinder(base),
            SurfType::Irregular => Shape::Irregular(Irregular {
                c,
                k,
                decenter: [p(0), p(1)],
                tilt: [p(2), p(3)],
                zs: p(4),
                za: p(5),
                zc: p(6),
                angle: p(7),
                radius: self.semi,
            }),
            SurfType::Fresnel
            | SurfType::Jones
            | SurfType::Paraxial
            | SurfType::User
            | SurfType::CoordBrk => Shape::Plane,
        }
    }

    /// 座標ブレーク（面ではなく、座標系だけを変える要素）
    pub fn is_coord_break(&self) -> bool {
        self.typ == SurfType::CoordBrk
    }

    /// 形のパラメータ（曲率・円錐係数・非球面の係数・Zernike の項）を変数・微分にできる面: 標準面・偶数次非球面と
    /// 回転対称でない面（[`Shape::is_freeform`]）。微分（[`crate::fermat`] の `shape_dz`）は `asph` を偶数次非球面の
    /// 係数として求める（Odd Asphere・Tilted・Irregular の `asph` は別の意味。従来どおり）
    pub(crate) fn shape_params(&self) -> bool {
        matches!(self.typ, SurfType::Standard | SurfType::EvenAsph) || self.shape().is_freeform()
    }
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

/// 二次曲面との交点の根（頂点面上の点からの距離）: aa s² − 2 gg s + ff = 0 の根 ff / (gg + σ √disc)（σ はその空間の
/// 進む向き: 鏡で反射するたびに反転、[`crate::trace::space_signs`]）。OpticStudio と同じく、光線が空間の向きと逆
/// （gg σ < 0、90° を超える視野の光線が面 1 へ向かうなど）でも同じ式の根を採る（以前は gg の符号で頂点に近い根を
/// 採っていた）。桁落ちしないよう gg σ < 0 では同じ根を (gg − σ √disc) / aa で求める
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

/// 偶数次非球面の矢高と (∂z/∂x, ∂z/∂y)
#[inline]
fn even_xy(c: f64, k: f64, a: &[f64], x: f64, y: f64) -> (f64, f64, f64) {
    let (z, dz) = sag(c, k, a, x * x + y * y);
    (z, 2.0 * dz * x, 2.0 * dz * y)
}

/// 近軸の曲率 c + 2 a₁（r² の項は近軸の曲率に入る）
#[inline]
fn r2_curv(c: f64, a: &[f64]) -> f64 {
    c + 2.0 * a.first().copied().unwrap_or(0.0)
}

impl Irregular {
    /// 偏心・傾ける前の座標の矢高と勾配: 標準面 + Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y。OpticStudio 26.1 と照合
    /// （自作の系で面の交点 1e-15 mm）
    fn local(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let (zs, za, zc, ang) = (self.zs, self.za, self.zc, self.angle.to_radians());
        let (z0, dz) = sag(self.c, self.k, &[], x * x + y * y);
        let (mut z, mut zx, mut zy) = (z0, 2.0 * dz * x, 2.0 * dz * y);
        let r = self.radius;
        if r > 0.0 && (zs != 0.0 || za != 0.0 || zc != 0.0) {
            let (sa, ca) = ang.sin_cos();
            let v = (-x * sa + y * ca) / r;
            let rho2 = (x * x + y * y) / (r * r);
            let (r2x, r2y) = (2.0 * x / (r * r), 2.0 * y / (r * r));
            let (vx, vy) = (-sa / r, ca / r);
            z += zs * rho2 * rho2 + za * v * v + zc * rho2 * v;
            zx += zs * 2.0 * rho2 * r2x + za * 2.0 * v * vx + zc * (r2x * v + rho2 * vx);
            zy += zs * 2.0 * rho2 * r2y + za * 2.0 * v * vy + zc * (r2y * v + rho2 * vy);
        }
        (z, zx, zy)
    }

    /// 面の座標の矢高と勾配。形は偏心 (dx, dy) の後に Rx(tx) Ry(ty) で傾けたもの: 点 P = d + Rx(tx) Ry(ty) Q
    /// （Q は [`Irregular::local`] の座標）。z を Q_z = f(Q_x, Q_y) のニュートン法で解く
    pub fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let (dx, dy) = (self.decenter[0], self.decenter[1]);
        let (tx, ty) = (self.tilt[0].to_radians(), self.tilt[1].to_radians());
        let (sx, cx) = tx.sin_cos();
        let (sy, cy) = ty.sin_cos();
        // M = Rx(tx) Ry(ty)（行優先）、Q = Mᵀ (P − d)
        let m = [
            [cy, 0.0, sy],
            [sx * sy, cx, -sx * cy],
            [-cx * sy, sx, cx * cy],
        ];
        let q = |px: f64, py: f64, pz: f64| -> [f64; 3] {
            [
                m[0][0] * px + m[1][0] * py + m[2][0] * pz,
                m[0][1] * px + m[1][1] * py + m[2][1] * pz,
                m[0][2] * px + m[1][2] * py + m[2][2] * pz,
            ]
        };
        let (px, py) = (x - dx, y - dy);
        let mut z = self.local(px, py).0;
        for _ in 0..50 {
            let qq = q(px, py, z);
            let (f, fx, fy) = self.local(qq[0], qq[1]);
            // g(z) = Q_z − f(Q_x, Q_y)。∂Q/∂z は M の 3 行目
            let g = qq[2] - f;
            let gz = m[2][2] - fx * m[2][0] - fy * m[2][1];
            if !(g.is_finite() && gz.is_finite()) || gz == 0.0 {
                return (f64::NAN, f64::NAN, f64::NAN);
            }
            let step = g / gz;
            z -= step;
            if step.abs() <= 1e-15 * (1.0 + z.abs()) {
                break;
            }
        }
        let qq = q(px, py, z);
        let (_, fx, fy) = self.local(qq[0], qq[1]);
        let gz = m[2][2] - fx * m[2][0] - fy * m[2][1];
        let gx = m[0][2] - fx * m[0][0] - fy * m[0][1];
        let gy = m[1][2] - fx * m[1][0] - fy * m[1][1];
        (z, -gx / gz, -gy / gz)
    }
}

/// 偶数次非球面 + Zernike の項（+ 格子）の近軸の曲率（YZ 面）。[`Shape::paraxial_curv`]
fn zern_grid_curv(base: Base, terms: Option<&ZernSag>, grid: Option<&GridSag>) -> f64 {
    let mut c = r2_curv(base.c, base.a);
    if let Some(z) = terms {
        let a4 = z.coef.get(3).copied().unwrap_or(0.0);
        let k = if z.fringe { 4.0 } else { 4.0 * 3f64.sqrt() };
        c += k * a4 / (z.norm * z.norm);
    }
    if let Some(g) = grid.filter(|g| !g.linear) {
        let h = 1e-5;
        let a = g.eval(0.0, h).map_or(0.0, |v| v.2);
        let b = g.eval(0.0, -h).map_or(0.0, |v| v.2);
        c += (a - b) / (2.0 * h);
    }
    c
}

/// トロイダル面との交点までの距離（頂点面上の pl から方向 kl に）。収束しなければ true も返す。
/// 初期値は y だけの二次曲面（円柱面）との交点、その後ニュートン法
fn toroidal_isect(c: f64, kc: f64, a: &[f64], cx: f64, pl: V3, kl: V3, sig: f64) -> (f64, bool) {
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
    if cx != 0.0 || a.iter().any(|&v| v != 0.0) || s == 0.0 && c != 0.0 {
        let mut ds = f64::NAN;
        for _ in 0..60 {
            let q = add(pl, scale(kl, s));
            let (z, zx, zy) = sag_toroidal(c, kc, a, cx, q[0], q[1]);
            ds = (q[2] - z) / (kl[2] - zx * kl[0] - zy * kl[1]);
            s -= ds;
            if !(ds.abs() > crate::trace::newton_tol(s, z)) {
                break;
            }
        }
        bad = !(ds.abs() <= 1e-10);
    }
    (s, bad)
}

impl Shape<'_> {
    /// 傾きの項が 0 の [`Shape::ConicTilted`] なら曲率 c（交点・法線は標準面と同じ求め方）
    fn untilted_c(&self) -> Option<f64> {
        match *self {
            Shape::ConicTilted { c, tan, .. } if tan == [0.0; 2] => Some(c),
            _ => None,
        }
    }

    /// 面の局所座標 (x, y) での矢高と (∂z/∂x, ∂z/∂y)。面の外（平方根が負、値のない格子点）は NaN
    pub fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        match *self {
            Shape::Plane => (0.0, 0.0, 0.0),
            Shape::Conic { c, k } => even_xy(c, k, &[], x, y),
            Shape::EvenAsphere(b) => even_xy(b.c, b.k, b.a, x, y),
            Shape::Cylinder(b) => {
                let (z, dz) = sag(b.c, b.k, b.a, y * y);
                (z, 0.0, 2.0 * dz * y)
            }
            Shape::Toroid { c, k, a, cx } => sag_toroidal(c, k, a, cx, x, y),
            Shape::ConicTilted { c, k, tan } => {
                let (z, dz) = sag(c, k, &[], x * x + y * y);
                (
                    z + x * tan[0] + y * tan[1],
                    2.0 * dz * x + tan[0],
                    2.0 * dz * y + tan[1],
                )
            }
            Shape::Tilted { tan: [a, b], .. } => (a * x + b * y, a, b),
            Shape::Irregular(ir) => ir.sag(x, y),
            Shape::OddAsphere { c, k, b } => {
                let r2 = x * x + y * y;
                let (z0, dz) = sag(c, k, &[], r2);
                let (mut z, mut zx, mut zy) = (z0, 2.0 * dz * x, 2.0 * dz * y);
                // Σ β_i r^i（r = 0 の勾配は 0。β_1 の円錐の頂点）
                let r = r2.sqrt();
                let (mut a, mut ar) = (0.0, 0.0);
                for (i, &bi) in b.iter().enumerate() {
                    if bi != 0.0 {
                        let n = (i + 1) as i32;
                        a += bi * r.powi(n);
                        ar += n as f64 * bi * r.powi(n - 1);
                    }
                }
                z += a;
                if r > 0.0 {
                    zx += ar * x / r;
                    zy += ar * y / r;
                }
                (z, zx, zy)
            }
            Shape::Zernike { base, terms, .. } => {
                let (mut z, mut zx, mut zy) = even_xy(base.c, base.k, base.a, x, y);
                if let Some(zs) = terms {
                    let (a, ax, ay) = zs.eval(x, y);
                    z += a;
                    zx += ax;
                    zy += ay;
                }
                (z, zx, zy)
            }
            Shape::GridSag { base, terms, grid } => {
                let (mut z, mut zx, mut zy) = even_xy(base.c, base.k, base.a, x, y);
                // Grid Sag にも Zernike（Standard）の項がある（外挿する）
                if let Some(zs) = terms {
                    let (a, ax, ay) = zs.eval(x, y);
                    z += a;
                    zx += ax;
                    zy += ay;
                }
                if let Some(g) = grid {
                    match g.eval(x, y) {
                        Some((a, ax, ay)) => {
                            z += a;
                            zx += ax;
                            zy += ay;
                        }
                        None => return (f64::NAN, f64::NAN, f64::NAN),
                    }
                }
                (z, zx, zy)
            }
        }
    }

    /// 回転対称でない矢高の面か（Zernike・Grid Sag・Odd Asphere・Tilted・Irregular。瞳を実光線で求める、
    /// 交点・法線を一般の矢高で求めるなど）
    pub fn is_freeform(&self) -> bool {
        matches!(
            self,
            Shape::Zernike { .. }
                | Shape::GridSag { .. }
                | Shape::OddAsphere { .. }
                | Shape::Tilted { .. }
                | Shape::Irregular(_)
        )
    }

    /// 近軸の曲率（YZ 面。トロイダル面も YZ 面で、OpticStudio と同じ）。OpticStudio 26.1 の EFFL に合わせたもの:
    /// - 偶数次非球面・トロイダル面: c + 2 a₁（r²・y² の項は近軸の曲率に入る）
    /// - Odd Asphere: c + 2 β₂、Tilted: 0、Irregular: c
    /// - Zernike Sag: 基準の c + 2 a₁ に、Zernike の項のうち Z4（デフォーカス）の分だけを足す
    ///   （Standard Z4 = √3 (2ρ² − 1) → 4√3 A4 / R²、Fringe Z4 = 2ρ² − 1 → 4 A4 / R²。偏心は見ない）
    /// - Grid Sag: 双三次なら頂点での ∂²z/∂y²（格子の補間の分を含む。勾配 ∂z/∂y の中心差分）、双一次は基準の面だけ。
    ///   OpticStudio は頂点の ∂²z/∂y² を矢高の差分（刻み 1e-2 mm 程度）で求めるらしく、円錐係数・r^4 以上の項・
    ///   双一次の折れ目の分だけ違う（EFFL で相対 1e-9〜1e-8、頂点が格子点に乗る双一次では大きく違う。
    ///   OpticStudio との照合）。ここでは差分を使わない厳密な値にしている
    pub fn paraxial_curv(&self) -> f64 {
        match *self {
            Shape::Plane | Shape::Tilted { .. } => 0.0,
            Shape::Conic { c, .. } | Shape::ConicTilted { c, .. } => r2_curv(c, &[]),
            Shape::EvenAsphere(b) | Shape::Cylinder(b) => r2_curv(b.c, b.a),
            Shape::Toroid { c, a, .. } => r2_curv(c, a),
            // r² の項（β_2）を足す
            Shape::OddAsphere { c, b, .. } => c + 2.0 * b.get(1).copied().unwrap_or(0.0),
            Shape::Irregular(ir) => ir.c,
            Shape::Zernike {
                base,
                terms,
                stray_grid,
            } => zern_grid_curv(base, terms, stray_grid),
            Shape::GridSag { base, terms, grid } => zern_grid_curv(base, terms, grid),
        }
    }

    /// 交点の求め方。`filled` は面の `asph`・`cx` に値が入っているか（0 でも）で、従来どおり、入っていれば
    /// c = 0 の回転対称の面でも [`Isect::Std`] で求める（[`Isect::Plane`] と違うのは 0 の符号だけ）
    pub(crate) fn isect_kind(&self, filled: bool) -> Isect {
        if let Some(c) = self.untilted_c() {
            return Shape::Conic { c, k: 0.0 }.isect_kind(filled);
        }
        match *self {
            Shape::Plane => Isect::Plane,
            Shape::Conic { c, .. } => {
                if c != 0.0 || filled {
                    Isect::Std { asph: false }
                } else {
                    Isect::Plane
                }
            }
            Shape::EvenAsphere(b) => {
                if b.c != 0.0 || filled {
                    Isect::Std {
                        asph: b.has_terms(),
                    }
                } else {
                    Isect::Plane
                }
            }
            Shape::Toroid { c, .. } => {
                if c != 0.0 || filled {
                    Isect::Newton
                } else {
                    Isect::Plane
                }
            }
            Shape::Cylinder(b) => {
                if b.c != 0.0 || b.has_terms() {
                    Isect::Newton
                } else {
                    Isect::Plane
                }
            }
            Shape::ConicTilted { .. }
            | Shape::Zernike { .. }
            | Shape::GridSag { .. }
            | Shape::OddAsphere { .. }
            | Shape::Tilted { .. }
            | Shape::Irregular(_) => Isect::Newton,
        }
    }

    /// 法線の求め方（屈折を別の面の傾きで決める Fresnel は [`surf_kind`] で上書きする）
    pub(crate) fn norm_kind(&self) -> Norm {
        if let Some(c) = self.untilted_c() {
            return Shape::Conic { c, k: 0.0 }.norm_kind();
        }
        match *self {
            Shape::Plane => Norm::Flat,
            Shape::Conic { c, .. } => {
                if c == 0.0 {
                    Norm::Flat
                } else {
                    Norm::Conic
                }
            }
            Shape::EvenAsphere(b) => {
                if b.has_terms() {
                    Norm::Asph
                } else if b.c == 0.0 {
                    Norm::Flat
                } else {
                    Norm::Conic
                }
            }
            _ => Norm::Grad,
        }
    }

    /// 局所座標の点 p（面の上）での法線（規格化前、+z 寄り）。[`Shape::norm_kind`] の求め方で、追跡の内側と同じ式
    pub(crate) fn normal(&self, p: V3) -> V3 {
        match (self.norm_kind(), *self) {
            (Norm::Flat, _) => [0.0, 0.0, 1.0],
            (Norm::Conic, Shape::Conic { c, k } | Shape::ConicTilted { c, k, .. })
            | (Norm::Conic, Shape::EvenAsphere(Base { c, k, .. })) => {
                [-c * p[0], -c * p[1], 1.0 - c * (1.0 + k) * p[2]]
            }
            (Norm::Asph, Shape::EvenAsphere(b)) => {
                let (_, dz) = sag(b.c, b.k, b.a, p[0] * p[0] + p[1] * p[1]);
                [-2.0 * dz * p[0], -2.0 * dz * p[1], 1.0]
            }
            _ => {
                let (_, zx, zy) = self.sag(p[0], p[1]);
                [-zx, -zy, 1.0]
            }
        }
    }

    /// 交点までの距離（頂点面上の pl から方向 kl に、σ はその空間の進む向き）。収束しなければ true も返す。
    /// [`Isect::Newton`] の面のためのもの（回転対称の面は追跡の内側で光線の組のまま求める）。初期値は基準の
    /// 二次曲面との交点、その後ニュートン法
    pub(crate) fn isect(&self, pl: V3, kl: V3, sig: f64) -> (f64, bool) {
        match *self {
            Shape::Toroid { c, k, a, cx } => toroidal_isect(c, k, a, cx, pl, kl, sig),
            // 基板は y だけの関数なので初期値は頂点の平面
            Shape::Cylinder(_) | Shape::Plane => {
                isect_with(0.0, 0.0, pl, kl, sig, |x, y| self.sag(x, y))
            }
            Shape::Conic { c, k }
            | Shape::ConicTilted { c, k, .. }
            | Shape::OddAsphere { c, k, .. }
            | Shape::Tilted { c, k, .. }
            | Shape::EvenAsphere(Base { c, k, .. })
            | Shape::Zernike {
                base: Base { c, k, .. },
                ..
            }
            | Shape::GridSag {
                base: Base { c, k, .. },
                ..
            }
            | Shape::Irregular(Irregular { c, k, .. }) => {
                isect_with(c, k, pl, kl, sig, |x, y| self.sag(x, y))
            }
        }
    }
}

/// 交点の求め方（面ごとに前計算する。[`surf_kind`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Isect {
    /// 頂点の平面（Fresnel もここ）
    Plane,
    /// 回転対称の曲面（asph: 非球面項があれば Newton で詰める）。光線の組のまま求める
    Std { asph: bool },
    /// 光線ごとに [`Shape::isect`]（トロイダル面・自由曲面・Cylinder Fresnel の基板・形を傾けた Gradient 9）
    Newton,
}

/// 法線の求め方（Paraxial・座標ブレークは面の働きで別に扱う）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Norm {
    Flat,
    Conic,
    Asph,
    /// 光線ごとに矢高の勾配 [`Shape::sag`] から
    Grad,
    /// Fresnel・Cylinder Fresnel の屈折の面の傾き（[`crate::action::FresnelBend::normal`]）
    Fresnel,
}

/// 追跡の内側で面ごとに分ける種類（光線ごと・面ごとの判定を系の更新時の 1 回にする）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfKind {
    pub isect: Isect,
    pub norm: Norm,
    /// Fresnel・Cylinder Fresnel（光路長の補正がある）
    pub fresnel: bool,
    pub act: crate::action::Act,
}

/// 面の交点・法線の求め方と働き
pub(crate) fn surf_kind(d: &Surface) -> SurfKind {
    let shape = d.shape();
    let action = d.action();
    let fresnel = matches!(action, Action::Fresnel(_));
    let isect = shape.isect_kind(!d.asph.is_empty() || d.cx != 0.0);
    let norm = if fresnel {
        Norm::Fresnel
    } else if matches!(action, Action::Jones(_)) && d.c != 0.0 {
        // 従来どおり、曲率の残った Jones の面（.zmx の読込では誤り）は交点を頂点の平面で、法線を曲率で求める
        Norm::Conic
    } else {
        shape.norm_kind()
    };
    SurfKind {
        isect,
        norm,
        fresnel,
        act: crate::action::Act::of(d, &action),
    }
}

/// 面 d の局所座標の点 pl での単位法線（+z 寄り。追跡の内側と同じ式。理想レンズ・Jones・座標ブレークなど平らな面は (0, 0, 1)）
pub(crate) fn local_normal(d: &Surface, pl: V3) -> V3 {
    let action = d.action();
    if d.is_coord_break()
        || matches!(
            action,
            Action::Paraxial { .. } | Action::Jones(_) | Action::User(_)
        )
    {
        return [0.0, 0.0, 1.0];
    }
    let v = match action {
        Action::Fresnel(b) => b.normal(pl[0], pl[1]),
        _ => d.shape().normal(pl),
    };
    normalize(v)
}
