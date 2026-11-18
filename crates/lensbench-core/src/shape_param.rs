//! 形のパラメータ（c・R・k・`asph`・Zernike の項）による矢高と勾配の微分（[`Shape::dsag`]）。
//!
//! 光路長の微分（[`crate::fermat`]）・着地点の微分（[`crate::landing`]）・パラメータの検査（[`crate::param`]）は
//! ここを通す。`asph` の意味は形ごとに違う（偶数次非球面の r², r⁴, … の係数、Odd Asphere の r¹, r², … の β、
//! Tilted の tan α・tan β、Irregular の偏心 x・y、傾き x・y、Zs・Za・Zc、角度）ので、微分も形ごとに求める。
//! 形を変えないパラメータ（Tilted の c・k、Zernike Sag・Grid Sag でない面に残った Zernike の項など）は
//! [`Shape::takes`] が false で、変数・微分にできない（[`crate::System::check_param`] で断る）

use crate::param::Kind;
use crate::shape::{sag, Irregular, Shape};

/// 回転対称の矢高 z(u)（u = r²）の、形のパラメータ（c・R・k・偶数次の係数 a）による微分 ∂z/∂p
pub(crate) fn dsag(c: f64, k: f64, kind: Kind, r2: f64) -> f64 {
    let root = (1.0 - (1.0 + k) * c * c * r2).max(0.0).sqrt();
    match kind {
        Kind::Curv | Kind::Radius => {
            let dc = r2 / (1.0 + root)
                + if c != 0.0 {
                    c * c * r2 * r2 * (1.0 + k) / ((1.0 + root) * (1.0 + root) * root)
                } else {
                    0.0
                };
            if kind == Kind::Radius {
                // dc/dR = −c²
                -c * c * dc
            } else {
                dc
            }
        }
        Kind::Conic => c * c * c * r2 * r2 / (2.0 * root * (1.0 + root) * (1.0 + root)),
        Kind::Asph(i) => r2.powi(i as i32 + 1),
        _ => 0.0,
    }
}

/// 回転対称の矢高の dz/du の、形のパラメータによる微分 ∂²z/∂u∂p
pub(crate) fn dsag_u(c: f64, k: f64, kind: Kind, u: f64) -> f64 {
    let root = (1.0 - (1.0 + k) * c * c * u).max(0.0).sqrt();
    match kind {
        Kind::Curv | Kind::Radius => {
            let v = 0.5 / root + c * c * (1.0 + k) * u / (2.0 * root * root * root);
            if kind == Kind::Radius {
                -c * c * v
            } else {
                v
            }
        }
        Kind::Conic => c * c * c * u / (4.0 * root * root * root),
        Kind::Asph(i) => (i as f64 + 1.0) * u.powi(i as i32),
        _ => 0.0,
    }
}

/// 回転対称の矢高の d²z/du²（偶数次非球面）
pub(crate) fn sag_uu(c: f64, k: f64, asph: &[f64], u: f64) -> f64 {
    let a = (1.0 + k) * c * c;
    let root = (1.0 - a * u).max(0.0).sqrt();
    let mut v = if c != 0.0 {
        c * a / (4.0 * root * root * root)
    } else {
        0.0
    };
    for (i, &ai) in asph.iter().enumerate() {
        let p = (i + 1) as i32;
        if ai != 0.0 && p >= 2 {
            v += ai * (p * (p - 1)) as f64 * u.powi(p - 2);
        }
    }
    v
}

impl Shape<'_> {
    /// 形のパラメータ kind（c・R・k・`asph` の i 番目・Zernike の項）がこの形を変え、[`Shape::dsag`] で微分できるか。
    /// - 二次曲面: c・k。偶数次非球面・Zernike Sag・Grid Sag: c・k・a（Zernike の項は面に項があるとき）
    /// - Odd Asphere: c・k・β（`asph` の i 番目が r^(i+1) の係数）
    /// - Tilted: tan α・tan β（`asph` の 0・1 番目）だけ。c・k は形に効かない
    /// - Irregular: c・k と `asph` の 0〜7 番目（偏心 x・y、傾き x・y、Zs・Za・Zc、角度）
    /// - そのほか（トロイダル面・Cylinder Fresnel の基板・Gradient 9 の形・平面）: なし（変数にできない面）
    pub fn takes(&self, kind: Kind) -> bool {
        match (*self, kind) {
            (
                Shape::Conic { .. }
                | Shape::EvenAsphere(_)
                | Shape::OddAsphere { .. }
                | Shape::Zernike { .. }
                | Shape::GridSag { .. }
                | Shape::Irregular(_),
                Kind::Curv | Kind::Radius | Kind::Conic,
            ) => true,
            (
                Shape::EvenAsphere(_)
                | Shape::OddAsphere { .. }
                | Shape::Zernike { .. }
                | Shape::GridSag { .. },
                Kind::Asph(_),
            ) => true,
            (Shape::Tilted { .. }, Kind::Asph(i)) => i < 2,
            (Shape::Irregular(_), Kind::Asph(i)) => i < 8,
            (Shape::Zernike { terms, .. } | Shape::GridSag { terms, .. }, Kind::Zern(_)) => {
                terms.is_some()
            }
            (Shape::User { .. }, Kind::Curv | Kind::Radius | Kind::Conic) => true,
            (Shape::User { shape, .. }, Kind::User(k)) => shape.dsag(k.name(), 0.0, 0.0).is_some(),
            _ => false,
        }
    }

    /// 矢高と勾配 (z, ∂z/∂x, ∂z/∂y)（[`Shape::sag`]）の、形のパラメータ kind による微分（面の局所座標の x・y を
    /// 固定）。角度のパラメータ（Irregular の傾き・角度）は度あたり。形を変えないパラメータ（[`Shape::takes`] が
    /// false）は None
    pub fn dsag(&self, kind: Kind, x: f64, y: f64) -> Option<[f64; 3]> {
        if !self.takes(kind) {
            return None;
        }
        let rot = |c: f64, k: f64| {
            let u = x * x + y * y;
            let dz1 = dsag_u(c, k, kind, u);
            [dsag(c, k, kind, u), 2.0 * dz1 * x, 2.0 * dz1 * y]
        };
        Some(match (*self, kind) {
            (
                Shape::Zernike { terms: Some(z), .. } | Shape::GridSag { terms: Some(z), .. },
                Kind::Zern(j),
            ) => {
                let (a, ax, ay) = z.term(j as usize, x, y);
                [a, ax, ay]
            }
            // z = x tan α + y tan β
            (Shape::Tilted { .. }, Kind::Asph(0)) => [x, 1.0, 0.0],
            (Shape::Tilted { .. }, _) => [y, 0.0, 1.0],
            // β r^n（n = i + 1。r = 0 の勾配は 0、[`Shape::sag`] と同じ）
            (Shape::OddAsphere { .. }, Kind::Asph(i)) => {
                let n = i as i32 + 1;
                let r = (x * x + y * y).sqrt();
                if r > 0.0 {
                    let ar = n as f64 * r.powi(n - 1) / r;
                    [r.powi(n), ar * x, ar * y]
                } else {
                    [r.powi(n), 0.0, 0.0]
                }
            }
            (Shape::Irregular(ir), _) => ir.dsag(kind, x, y),
            (Shape::User { shape, .. }, Kind::User(k)) => shape.dsag(k.name(), x, y)?,
            (
                Shape::Conic { c, k } | Shape::OddAsphere { c, k, .. } | Shape::User { c, k, .. },
                _,
            ) => rot(c, k),
            (
                Shape::EvenAsphere(b)
                | Shape::Zernike { base: b, .. }
                | Shape::GridSag { base: b, .. },
                _,
            ) => rot(b.c, b.k),
            _ => unreachable!("takes で確かめた形のパラメータ"),
        })
    }
}

impl Irregular {
    /// 項 Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y の部品: ρ²・ρ_y と、その x・y による微分
    /// （ρ² = (x² + y²)/R²、ρ_y = (−x sin θ + y cos θ)/R）。正規化半径 R が 0 以下なら None（項は効かない）
    fn parts(&self, x: f64, y: f64) -> Option<Parts> {
        let r = self.radius;
        if !(r > 0.0) {
            return None;
        }
        let (sa, ca) = self.angle.to_radians().sin_cos();
        Some(Parts {
            rho2: (x * x + y * y) / (r * r),
            r2x: 2.0 * x / (r * r),
            r2y: 2.0 * y / (r * r),
            r2xx: 2.0 / (r * r),
            v: (-x * sa + y * ca) / r,
            vx: -sa / r,
            vy: ca / r,
            // ρ_y の角度 θ [rad] による微分
            va: (-x * ca - y * sa) / r,
            vxa: -ca / r,
            vya: -sa / r,
        })
    }

    /// 偏心・傾ける前の座標の矢高 f の 2 階微分 (f_xx, f_xy, f_yy)（[`Irregular::local`] の）
    fn local_hess(&self, x: f64, y: f64) -> [f64; 3] {
        let u = x * x + y * y;
        let s1 = sag(self.c, self.k, &[], u).1;
        let s2 = sag_uu(self.c, self.k, &[], u);
        let mut h = [
            2.0 * s1 + 4.0 * s2 * x * x,
            4.0 * s2 * x * y,
            2.0 * s1 + 4.0 * s2 * y * y,
        ];
        let (zs, za, zc) = (self.zs, self.za, self.zc);
        if zs != 0.0 || za != 0.0 || zc != 0.0 {
            if let Some(p) = self.parts(x, y) {
                h[0] += zs * (2.0 * p.r2x * p.r2x + 2.0 * p.rho2 * p.r2xx)
                    + za * 2.0 * p.vx * p.vx
                    + zc * (p.r2xx * p.v + 2.0 * p.r2x * p.vx);
                h[1] += zs * 2.0 * p.r2x * p.r2y
                    + za * 2.0 * p.vx * p.vy
                    + zc * (p.r2x * p.vy + p.r2y * p.vx);
                h[2] += zs * (2.0 * p.r2y * p.r2y + 2.0 * p.rho2 * p.r2xx)
                    + za * 2.0 * p.vy * p.vy
                    + zc * (p.r2xx * p.v + 2.0 * p.r2y * p.vy);
            }
        }
        h
    }

    /// 偏心・傾ける前の座標の (f, f_x, f_y) の、形の値（c・R・k、Zs・Za・Zc、角度 [deg]）による微分
    fn local_dp(&self, kind: Kind, x: f64, y: f64) -> [f64; 3] {
        match kind {
            Kind::Curv | Kind::Radius | Kind::Conic => {
                let u = x * x + y * y;
                let du = dsag_u(self.c, self.k, kind, u);
                [dsag(self.c, self.k, kind, u), 2.0 * du * x, 2.0 * du * y]
            }
            Kind::Asph(i @ 4..=7) => {
                let Some(p) = self.parts(x, y) else {
                    return [0.0; 3];
                };
                match i {
                    4 => [p.rho2 * p.rho2, 2.0 * p.rho2 * p.r2x, 2.0 * p.rho2 * p.r2y],
                    5 => [p.v * p.v, 2.0 * p.v * p.vx, 2.0 * p.v * p.vy],
                    6 => [
                        p.rho2 * p.v,
                        p.r2x * p.v + p.rho2 * p.vx,
                        p.r2y * p.v + p.rho2 * p.vy,
                    ],
                    _ => {
                        let (za, zc, deg) = (self.za, self.zc, 1f64.to_radians());
                        [
                            deg * (za * 2.0 * p.v * p.va + zc * p.rho2 * p.va),
                            deg * (za * 2.0 * (p.va * p.vx + p.v * p.vxa)
                                + zc * (p.r2x * p.va + p.rho2 * p.vxa)),
                            deg * (za * 2.0 * (p.va * p.vy + p.v * p.vya)
                                + zc * (p.r2y * p.va + p.rho2 * p.vya)),
                        ]
                    }
                }
            }
            _ => [0.0; 3],
        }
    }

    /// 矢高と勾配の、形のパラメータによる微分（[`Shape::dsag`]）。面は g(P) = Q_z − f(Q_x, Q_y) = 0
    /// （Q = Mᵀ (P − d)、[`Irregular::sag`]）で決まるので、陰関数として微分する:
    /// dz/dp = −g_p / G_z、勾配 −G_x/G_z・−G_y/G_z（G = ∂g/∂P）は G の全微分（z の動きを含む）から
    fn dsag(&self, kind: Kind, x: f64, y: f64) -> [f64; 3] {
        let [m, mtx, mty] = self.rot();
        let z = self.sag(x, y).0;
        let p = [x - self.decenter[0], y - self.decenter[1], z];
        // Q_i = Σ_r m[r][i] P_r
        let q = |m: &[[f64; 3]; 3], v: [f64; 3]| -> [f64; 3] {
            [0, 1, 2].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
        };
        let qq = q(&m, p);
        let (_, fx, fy) = self.local(qq[0], qq[1]);
        let [hxx, hxy, hyy] = self.local_hess(qq[0], qq[1]);
        // パラメータによる Q の動き dq（P は固定）、M の動き dm、f・f_x・f_y の動き fp（Q は固定）
        let deg = 1f64.to_radians();
        let zero = [[0.0; 3]; 3];
        let (dq, dm, fp) = match kind {
            // 偏心: ∂Q/∂d_x = −(M の 0 行目)
            Kind::Asph(s @ 0..=1) => {
                let s = s as usize;
                ([-m[s][0], -m[s][1], -m[s][2]], zero, [0.0; 3])
            }
            Kind::Asph(s @ 2..=3) => {
                let dm = if s == 2 { mtx } else { mty }.map(|r| r.map(|v| v * deg));
                (q(&dm, p), dm, [0.0; 3])
            }
            _ => ([0.0; 3], zero, self.local_dp(kind, qq[0], qq[1])),
        };
        let g = [0, 1, 2].map(|s| m[s][2] - fx * m[s][0] - fy * m[s][1]);
        let gp = dq[2] - fx * dq[0] - fy * dq[1] - fp[0];
        let dz = -gp / g[2];
        // f_x・f_y の動き: パラメータ（Q の動きと f の値）の分と、z の動き（Q が M の 2 行目の向きに動く）の分
        let ex = hxx * dq[0] + hxy * dq[1] + fp[1];
        let ey = hxy * dq[0] + hyy * dq[1] + fp[2];
        let ezx = hxx * m[2][0] + hxy * m[2][1];
        let ezy = hxy * m[2][0] + hyy * m[2][1];
        let dg = [0, 1, 2].map(|s| {
            dm[s][2]
                - fx * dm[s][0]
                - fy * dm[s][1]
                - ex * m[s][0]
                - ey * m[s][1]
                - (ezx * m[s][0] + ezy * m[s][1]) * dz
        });
        let g2 = g[2] * g[2];
        [
            dz,
            -(dg[0] * g[2] - g[0] * dg[2]) / g2,
            -(dg[1] * g[2] - g[1] * dg[2]) / g2,
        ]
    }
}

/// [`Irregular::parts`] の値
struct Parts {
    rho2: f64,
    r2x: f64,
    r2y: f64,
    r2xx: f64,
    v: f64,
    vx: f64,
    vy: f64,
    va: f64,
    vxa: f64,
    vya: f64,
}
