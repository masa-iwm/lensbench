//! 回転対称でない矢高の面: Zernike Standard Sag・Zernike Fringe Sag・Grid Sag。
//!
//! どれも「基準の面（曲率 c・円錐係数 k・偶数次非球面 r^2〜r^16）＋追加の矢高 z(x, y)」の形で、
//! 交点はニュートン法、屈折・反射の法線は (−∂z/∂x, −∂z/∂y, 1) で求める（[`sag_xy`]・[`isect`]）。
//! ほかの面タイプ（のフレネル面・回折面など）も、交点用の形と屈折用の傾きを分けて同じ道具で扱える
//! （[`isect_with`]）。
//!
//! - Zernike Standard Sag: 項は Noll の順で規格化したもの（Z1 = 1, Z2 = 2ρ cos φ, Z3 = 2ρ sin φ,
//!   Z4 = √3 (2ρ² − 1), Z5 = √6 ρ² sin 2φ, Z6 = √6 ρ² cos 2φ, …、231 項まで）。
//! - Zernike Fringe Sag: 規格化しない Fringe（University of Arizona）の順の 37 項。
//! - ρ = r / 正規化半径、φ は +x から +y へ測る角。外挿しないときは ρ > 1 で Zernike の項を 0 にする。
//! - Grid Sag: 長方形の格子の値（矢高と ∂z/∂x・∂z/∂y・∂²z/∂x∂y）を双三次（または双一次）で補間する。

use crate::math::*;
use crate::surface::{SurfType, Surface};
use crate::trace::sag;

/// Zernike の矢高の項
///
/// serde では内部の `groups`・`mmax` を書き出さず、読み直したら `coef` から作り直す。
/// 項の数が [`ZernSag::max_terms`] を超えるものは読み直しでエラーにする
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(try_from = "ZernSagData"))]
pub struct ZernSag {
    /// Fringe の順（規格化なし）か、Standard（Noll の順、規格化）か
    pub fringe: bool,
    /// 正規化半径 [mm]
    pub norm: f64,
    /// 項 1, 2, … の係数 [mm]
    pub coef: Vec<f64>,
    /// 正規化半径の外でも Zernike の項を使うか
    pub extrapolate: bool,
    /// Zernike の項の中心の偏心（Zern Decenter X・Y）[mm]。ρ・φ は (x − dx, y − dy) から
    pub dx: f64,
    pub dy: f64,
    /// (m, cos か) ごとにまとめた ρ² の多項式（係数を掛けて足したもの、低い次数から）
    #[cfg_attr(feature = "serde", serde(skip))]
    groups: Vec<(u32, bool, Vec<f64>)>,
    /// m の最大
    #[cfg_attr(feature = "serde", serde(skip))]
    mmax: u32,
}

/// [`ZernSag`] の serde の読み直し用（内部の多項式なし）。順番は ZernSag の書き出しと同じ
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename = "ZernSag"))]
struct ZernSagData {
    /// Fringe の順（規格化なし）か、Standard（Noll の順、規格化）か
    fringe: bool,
    /// 正規化半径 [mm]
    norm: f64,
    /// 項 1, 2, … の係数 [mm]
    coef: Vec<f64>,
    /// 正規化半径の外でも Zernike の項を使うか
    extrapolate: bool,
    /// Zernike の項の中心の偏心（Zern Decenter X・Y）[mm]
    dx: f64,
    dy: f64,
}

#[cfg(feature = "serde")]
impl TryFrom<ZernSagData> for ZernSag {
    type Error = String;
    fn try_from(d: ZernSagData) -> Result<ZernSag, String> {
        let max = ZernSag::max_terms(d.fringe);
        if d.coef.len() > max {
            return Err(format!("Zernike の項が {} 個（上限 {max}）", d.coef.len()));
        }
        let mut z = ZernSag::new(d.fringe, d.norm, d.coef, d.extrapolate);
        z.dx = d.dx;
        z.dy = d.dy;
        Ok(z)
    }
}

/// 項 j（1 から）の (n, m, cos か)。Standard は Noll、Fringe は Fringe の順
pub fn zern_nm(j: usize, fringe: bool) -> (u32, u32, bool) {
    assert!(j >= 1);
    if fringe {
        // (n + m)/2 = g の組ごとに m = g, g−1, …, 0（n = 2g − m）、m > 0 は cos・sin の順。
        // g ≤ 5 の 36 項のあとの 37 項目だけは (12, 0)
        if j == 37 {
            return (12, 0, true);
        }
        let mut k = 1;
        let mut g = 0u32;
        loop {
            for m in (0..=g).rev() {
                let n = 2 * g - m;
                if m == 0 {
                    if k == j {
                        return (n, 0, true);
                    }
                    k += 1;
                } else {
                    if k == j {
                        return (n, m, true);
                    }
                    if k + 1 == j {
                        return (n, m, false);
                    }
                    k += 2;
                }
            }
            g += 1;
        }
    }
    let mut n = 0usize;
    while (n + 1) * (n + 2) / 2 < j {
        n += 1;
    }
    let k = j - n * (n + 1) / 2 - 1;
    let m = if n % 2 == 0 {
        2 * k.div_ceil(2)
    } else {
        2 * (k / 2) + 1
    };
    // m > 0: j が偶数なら cos、奇数なら sin（Noll）
    (n as u32, m as u32, m == 0 || j % 2 == 0)
}

/// R_n^m(ρ) = ρ^m P(ρ²) の P の係数（低い次数から、規格化の係数を掛けて）
fn radial_poly(n: u32, m: u32, normalize: bool) -> Vec<f64> {
    let h = ((n - m) / 2) as usize;
    let fact = |k: u32| -> f64 { (1..=k).map(|v| v as f64).product() };
    let nrm = if normalize {
        if m == 0 {
            ((n + 1) as f64).sqrt()
        } else {
            (2.0 * (n + 1) as f64).sqrt()
        }
    } else {
        1.0
    };
    let mut p = vec![0.0; h + 1];
    for s in 0..=h as u32 {
        // ρ^(n−2s) = ρ^m (ρ²)^(h−s)
        let c = fact(n - s) / (fact(s) * fact((n + m) / 2 - s) * fact((n - m) / 2 - s));
        let c = if s % 2 == 0 { c } else { -c };
        p[h - s as usize] = c * nrm;
    }
    p
}

impl ZernSag {
    pub fn new(fringe: bool, norm: f64, coef: Vec<f64>, extrapolate: bool) -> ZernSag {
        let mut z = ZernSag {
            fringe,
            norm,
            coef,
            extrapolate,
            dx: 0.0,
            dy: 0.0,
            groups: Vec::new(),
            mmax: 0,
        };
        z.rebuild();
        z
    }

    /// 係数を変えたあとに呼ぶ（まとめた多項式を作り直す）
    pub fn rebuild(&mut self) {
        let mut g: Vec<(u32, bool, Vec<f64>)> = Vec::new();
        for (i, &a) in self.coef.iter().enumerate() {
            if a == 0.0 {
                continue;
            }
            let (n, m, cs) = zern_nm(i + 1, self.fringe);
            let p = radial_poly(n, m, !self.fringe);
            let e = match g.iter_mut().position(|e| e.0 == m && e.1 == cs) {
                Some(k) => &mut g[k].2,
                None => {
                    g.push((m, cs, Vec::new()));
                    &mut g.last_mut().unwrap().2
                }
            };
            if e.len() < p.len() {
                e.resize(p.len(), 0.0);
            }
            for (t, v) in p.iter().enumerate() {
                e[t] += a * v;
            }
        }
        g.sort_by_key(|e| (e.0, !e.1));
        self.mmax = g.iter().map(|e| e.0).max().unwrap_or(0);
        self.groups = g;
    }

    /// Zernike の項の和と (∂/∂x, ∂/∂y)
    pub fn eval(&self, x: f64, y: f64) -> (f64, f64, f64) {
        if self.groups.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        eval_groups(
            &self.groups,
            self.mmax,
            self.norm,
            self.extrapolate,
            x - self.dx,
            y - self.dy,
        )
    }

    /// 項 j（1 から）だけの値（係数 1）と (∂/∂x, ∂/∂y)。感度 dz/dA_j に使う
    pub fn term(&self, j: usize, x: f64, y: f64) -> (f64, f64, f64) {
        let (n, m, cs) = zern_nm(j, self.fringe);
        let g = [(m, cs, radial_poly(n, m, !self.fringe))];
        eval_groups(&g, m, self.norm, self.extrapolate, x - self.dx, y - self.dy)
    }

    /// 項の数の上限（Standard 231、Fringe 37。OpticStudio と同じ）
    pub fn max_terms(fringe: bool) -> usize {
        if fringe {
            37
        } else {
            231
        }
    }
}

/// Σ_(m, cos/sin) A(u, v) P(ρ²) と (∂/∂x, ∂/∂y)
fn eval_groups(
    groups: &[(u32, bool, Vec<f64>)],
    mmax: u32,
    norm: f64,
    extrapolate: bool,
    x: f64,
    y: f64,
) -> (f64, f64, f64) {
    if !(norm > 0.0) {
        return (0.0, 0.0, 0.0);
    }
    let (u, v) = (x / norm, y / norm);
    let r2 = u * u + v * v;
    if !extrapolate && r2 > 1.0 {
        return (0.0, 0.0, 0.0);
    }
    // w^m = (u + i v)^m の実部・虚部（m = 0..=mmax）
    let mm = mmax as usize;
    let mut wr = [0.0; 32];
    let mut wi = [0.0; 32];
    wr[0] = 1.0;
    for m in 1..=mm {
        wr[m] = wr[m - 1] * u - wi[m - 1] * v;
        wi[m] = wr[m - 1] * v + wi[m - 1] * u;
    }
    let (mut z, mut zu, mut zv) = (0.0, 0.0, 0.0);
    for (m, cs, p) in groups {
        let m = *m as usize;
        // P(ρ²) と P'(ρ²)
        let (mut pv, mut dp) = (0.0, 0.0);
        for &c in p.iter().rev() {
            dp = dp * r2 + pv;
            pv = pv * r2 + c;
        }
        // A = Re w^m（cos）か Im w^m（sin）、∂A/∂u・∂A/∂v
        let (a, au, av) = if m == 0 {
            (1.0, 0.0, 0.0)
        } else {
            let mf = m as f64;
            if *cs {
                (wr[m], mf * wr[m - 1], -mf * wi[m - 1])
            } else {
                (wi[m], mf * wi[m - 1], mf * wr[m - 1])
            }
        };
        z += pv * a;
        zu += 2.0 * u * dp * a + pv * au;
        zv += 2.0 * v * dp * a + pv * av;
    }
    (z, zu / norm, zv / norm)
}

/// Grid Sag の格子
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GridSag {
    pub nx: usize,
    pub ny: usize,
    /// 格子の間隔 [mm]
    pub dx: f64,
    pub dy: f64,
    /// 格子の中心の偏心 [mm]
    pub xdec: f64,
    pub ydec: f64,
    /// 点ごとの (z, ∂z/∂x, ∂z/∂y, ∂²z/∂x∂y)。並びは y の小さい行から、行の中は x の小さい点から
    /// （iy * nx + ix）。単位は mm
    pub z: Vec<[f64; 4]>,
    /// 値のない点
    pub nodata: Vec<bool>,
    /// 双一次で補間する（false は双三次）。双一次では傾きも格子点の傾きの双一次補間
    pub linear: bool,
}

impl GridSag {
    /// 格子の値と (∂/∂x, ∂/∂y)。格子の外は 0（OpticStudio と同じ）、値のない点に掛かる升目は None
    pub fn eval(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let (nx, ny) = (self.nx, self.ny);
        if nx < 2 || ny < 2 {
            return None;
        }
        let fx = (x - self.xdec) / self.dx + (nx - 1) as f64 / 2.0;
        let fy = (y - self.ydec) / self.dy + (ny - 1) as f64 / 2.0;
        let lim = |f: f64, n: usize| f >= -1e-12 && f <= (n - 1) as f64 + 1e-12;
        if !(lim(fx, nx) && lim(fy, ny)) {
            return Some((0.0, 0.0, 0.0));
        }
        let ix = (fx.floor().max(0.0) as usize).min(nx - 2);
        let iy = (fy.floor().max(0.0) as usize).min(ny - 2);
        let (t, s) = (fx - ix as f64, fy - iy as f64);
        let at = |i: usize, j: usize| j * nx + i;
        let idx = [
            at(ix, iy),
            at(ix + 1, iy),
            at(ix, iy + 1),
            at(ix + 1, iy + 1),
        ];
        if idx.iter().any(|&k| self.nodata[k]) {
            return None;
        }
        let p = idx.map(|k| self.z[k]);
        if self.linear {
            // 双一次: 矢高は z の、傾きは格子点の ∂z/∂x・∂z/∂y の双一次補間（OpticStudio 26.1 と同じ。矢高の
            // 微分ではないので、矢高と傾きは厳密には合わない。非公開側の照合データ G2 で方向が 3e-13 で一致）
            let bl = |c: usize| {
                p[0][c] * (1.0 - t) * (1.0 - s)
                    + p[1][c] * t * (1.0 - s)
                    + p[2][c] * (1.0 - t) * s
                    + p[3][c] * t * s
            };
            return Some((bl(0), bl(1), bl(2)));
        }
        // 3 次のエルミート基底 h0(0)=1, h1(1)=1, g0'(0)=1, g1'(1)=1 と微分
        let herm = |t: f64| {
            let (t2, t3) = (t * t, t * t * t);
            (
                [2.0 * t3 - 3.0 * t2 + 1.0, -2.0 * t3 + 3.0 * t2],
                [t3 - 2.0 * t2 + t, t3 - t2],
                [6.0 * t2 - 6.0 * t, -6.0 * t2 + 6.0 * t],
                [3.0 * t2 - 4.0 * t + 1.0, 3.0 * t2 - 2.0 * t],
            )
        };
        let (hx, gx, dhx, dgx) = herm(t);
        let (hy, gy, dhy, dgy) = herm(s);
        let (dx, dy) = (self.dx, self.dy);
        let (mut z, mut zt, mut zs) = (0.0, 0.0, 0.0);
        for b in 0..2 {
            for a in 0..2 {
                let q = p[b * 2 + a];
                let (f, fx, fy, fxy) = (q[0], q[1] * dx, q[2] * dy, q[3] * dx * dy);
                z += f * hx[a] * hy[b]
                    + fx * gx[a] * hy[b]
                    + fy * hx[a] * gy[b]
                    + fxy * gx[a] * gy[b];
                zt += f * dhx[a] * hy[b]
                    + fx * dgx[a] * hy[b]
                    + fy * dhx[a] * gy[b]
                    + fxy * dgx[a] * gy[b];
                zs += f * hx[a] * dhy[b]
                    + fx * gx[a] * dhy[b]
                    + fy * hx[a] * dgy[b]
                    + fxy * gx[a] * dgy[b];
            }
        }
        Some((z, zt / dx, zs / dy))
    }

    /// OpticStudio の .DAT（1 行目 `nx ny delx dely unitflag xdec ydec`、続いて点ごとに
    /// `z dz/dx dz/dy d2z/dxdy nodata`。上の行（+y）から、行の中は −x から）を読む。
    /// 単位（0 mm・1 cm・2 inch・3 m）は mm に直す
    pub fn from_dat(text: &str) -> Result<GridSag, String> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('!'));
        let head: Vec<f64> = lines
            .next()
            .ok_or("空のファイル")?
            .split_whitespace()
            .map(|v| v.parse::<f64>().map_err(|e| format!("{v}: {e}")))
            .collect::<Result<_, _>>()?;
        if head.len() < 4 {
            return Err("1 行目は nx ny delx dely unitflag xdec ydec".into());
        }
        let (nx, ny) = (head[0] as usize, head[1] as usize);
        let unit = head.get(4).copied().unwrap_or(0.0) as i32;
        let sc = match unit {
            0 => 1.0,
            1 => 10.0,
            2 => 25.4,
            3 => 1000.0,
            u => return Err(format!("単位の番号 {u}")),
        };
        let mut z = vec![[0.0; 4]; nx * ny];
        let mut nodata = vec![false; nx * ny];
        for r in 0..nx * ny {
            let l = lines
                .next()
                .ok_or_else(|| format!("点が足りない（{r} / {}）", nx * ny))?;
            let v: Vec<f64> = l
                .split_whitespace()
                .map(|v| v.parse::<f64>().map_err(|e| format!("{v}: {e}")))
                .collect::<Result<_, _>>()?;
            let (row, ix) = (r / nx, r % nx);
            let k = (ny - 1 - row) * nx + ix;
            let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
            z[k] = [g(0) * sc, g(1), g(2), g(3) / sc];
            nodata[k] = g(4) != 0.0;
        }
        let mut g = GridSag {
            nx,
            ny,
            dx: head[2] * sc,
            dy: head[3] * sc,
            xdec: head.get(5).copied().unwrap_or(0.0) * sc,
            ydec: head.get(6).copied().unwrap_or(0.0) * sc,
            z,
            nodata,
            linear: false,
        };
        if g.z
            .iter()
            .all(|p| p[1] == 0.0 && p[2] == 0.0 && p[3] == 0.0)
        {
            g.fill_derivs();
        }
        Ok(g)
    }

    /// 微分を矢高の差分で求める（.DAT に微分がないとき。OpticStudio と同じく numpy.gradient の形:
    /// 内側は中心差分、端は片側の 1 次差分。∂²z/∂x∂y は ∂z/∂x の y 方向の差分）
    pub fn fill_derivs(&mut self) {
        let (nx, ny) = (self.nx, self.ny);
        let grad = |f: &dyn Fn(usize) -> f64, n: usize, h: f64, i: usize| -> f64 {
            if n < 2 {
                0.0
            } else if i == 0 {
                (f(1) - f(0)) / h
            } else if i == n - 1 {
                (f(n - 1) - f(n - 2)) / h
            } else {
                (f(i + 1) - f(i - 1)) / (2.0 * h)
            }
        };
        let z: Vec<f64> = self.z.iter().map(|p| p[0]).collect();
        let mut zx = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                zx[j * nx + i] = grad(&|k| z[j * nx + k], nx, self.dx, i);
                self.z[j * nx + i][2] = grad(&|k| z[k * nx + i], ny, self.dy, j);
            }
        }
        for j in 0..ny {
            for i in 0..nx {
                self.z[j * nx + i][1] = zx[j * nx + i];
                self.z[j * nx + i][3] = grad(&|k| zx[k * nx + i], ny, self.dy, j);
            }
        }
    }
}

/// 回転対称でない矢高の面か（交点・法線を [`sag_xy`] で求める面）
pub fn is_freeform(t: SurfType) -> bool {
    matches!(
        t,
        SurfType::ZernikeStd
            | SurfType::ZernikeFringe
            | SurfType::GridSag
            | SurfType::OddAsph
            | SurfType::TiltSurf
            | SurfType::Irregular
    )
}

/// 偶数次非球面の項（`asph`）を基準の面に使う面タイプか（Odd Asphere・Tilted・Irregular の `asph` は別の意味）
fn even_terms(t: SurfType) -> bool {
    !matches!(
        t,
        SurfType::OddAsph | SurfType::TiltSurf | SurfType::Irregular
    )
}

/// Irregular の形（偏心・傾ける前の座標）の矢高と勾配: 標準面 + Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y。ρ は半直径で正規化、
/// ρ_y は角度だけ回した y。OpticStudio 26.1 と照合（自作の系で面の交点 1e-15 mm）
fn irregular_local(d: &Surface, x: f64, y: f64) -> (f64, f64, f64) {
    let p = |i: usize| d.asph.get(i).copied().unwrap_or(0.0);
    let (zs, za, zc, ang) = (p(4), p(5), p(6), p(7).to_radians());
    let (z0, dz) = sag(d.c, d.k, &[], x * x + y * y);
    let (mut z, mut zx, mut zy) = (z0, 2.0 * dz * x, 2.0 * dz * y);
    let r = d.semi;
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

/// Irregular の面の矢高と勾配（面の座標）。形は偏心 (dx, dy) の後に Rx(tx) Ry(ty) で傾けたもの:
/// 点 P = d + Rx(tx) Ry(ty) Q（Q は [`irregular_local`] の座標）。z を Q_z = f(Q_x, Q_y) のニュートン法で解く
fn irregular_xy(d: &Surface, x: f64, y: f64) -> (f64, f64, f64) {
    let p = |i: usize| d.asph.get(i).copied().unwrap_or(0.0);
    let (dx, dy, tx, ty) = (p(0), p(1), p(2).to_radians(), p(3).to_radians());
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
    let mut z = irregular_local(d, px, py).0;
    for _ in 0..50 {
        let qq = q(px, py, z);
        let (f, fx, fy) = irregular_local(d, qq[0], qq[1]);
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
    let (_, fx, fy) = irregular_local(d, qq[0], qq[1]);
    let gz = m[2][2] - fx * m[2][0] - fy * m[2][1];
    let gx = m[0][2] - fx * m[0][0] - fy * m[0][1];
    let gy = m[1][2] - fx * m[1][0] - fy * m[1][1];
    (z, -gx / gz, -gy / gz)
}

/// 面の矢高と (∂z/∂x, ∂z/∂y)（面の局所座標）。面の外（基準の面の平方根が負、値のない格子点）は NaN
pub fn sag_xy(d: &Surface, x: f64, y: f64) -> (f64, f64, f64) {
    let r2 = x * x + y * y;
    match d.typ {
        SurfType::TiltSurf => {
            let a = d.asph.first().copied().unwrap_or(0.0);
            let b = d.asph.get(1).copied().unwrap_or(0.0);
            return (a * x + b * y, a, b);
        }
        SurfType::Irregular => return irregular_xy(d, x, y),
        _ => {}
    }
    let asph: &[f64] = if even_terms(d.typ) { &d.asph } else { &[] };
    let (z0, dz) = sag(d.c, d.k, asph, r2);
    let (mut z, mut zx, mut zy) = (z0, 2.0 * dz * x, 2.0 * dz * y);
    match d.typ {
        SurfType::ZernikeStd | SurfType::ZernikeFringe => {
            if let Some(zs) = &d.zern {
                let (a, ax, ay) = zs.eval(x, y);
                z += a;
                zx += ax;
                zy += ay;
            }
        }
        SurfType::GridSag => {
            // Grid Sag にも Zernike（Standard）の項がある（外挿する）
            if let Some(zs) = &d.zern {
                let (a, ax, ay) = zs.eval(x, y);
                z += a;
                zx += ax;
                zy += ay;
            }
            if let Some(g) = &d.grid {
                match g.eval(x, y) {
                    Some((a, ax, ay)) => {
                        z += a;
                        zx += ax;
                        zy += ay;
                    }
                    None => return (f64::NAN, f64::NAN, f64::NAN),
                }
            }
        }
        SurfType::OddAsph => {
            // Σ β_i r^i（r = 0 の勾配は 0。β_1 の円錐の頂点）
            let r = r2.sqrt();
            let (mut a, mut ar) = (0.0, 0.0);
            for (i, &b) in d.asph.iter().enumerate() {
                if b != 0.0 {
                    let n = (i + 1) as i32;
                    a += b * r.powi(n);
                    ar += n as f64 * b * r.powi(n - 1);
                }
            }
            z += a;
            if r > 0.0 {
                zx += ar * x / r;
                zy += ar * y / r;
            }
        }
        _ => {}
    }
    (z, zx, zy)
}

/// 近軸の曲率（YZ 面）。OpticStudio 26.1 の EFFL に合わせたもの（非公開側の照合データの Z・G 系）:
/// - Zernike Sag: 基準の c + 2 α1 に、Zernike の項のうち Z4（デフォーカス）の分だけを足す
///   （Standard Z4 = √3 (2ρ² − 1) → 4√3 A4 / R²、Fringe Z4 = 2ρ² − 1 → 4 A4 / R²。偏心は見ない）
/// - Grid Sag: 双三次なら頂点での ∂²z/∂y²（格子の補間の分を含む。勾配 ∂z/∂y の中心差分）、双一次は基準の面だけ。
///   OpticStudio は頂点の ∂²z/∂y² を矢高の差分（刻み 1e-2 mm 程度）で求めるらしく、円錐係数・r^4 以上の項・
///   双一次の折れ目の分だけ違う（EFFL で相対 1e-9〜1e-8、頂点が格子点に乗る双一次では大きく違う。
///   非公開側の照合データ）。ここでは差分を使わない厳密な値にしている
pub fn paraxial_curv(d: &Surface) -> f64 {
    match d.typ {
        // r² の項（β_2）を足す
        SurfType::OddAsph => return d.c + 2.0 * d.asph.get(1).copied().unwrap_or(0.0),
        SurfType::TiltSurf => return 0.0,
        SurfType::Irregular => return d.c,
        _ => {}
    }
    let a1 = d.asph.first().copied().unwrap_or(0.0);
    let mut c = d.c + 2.0 * a1;
    if let Some(z) = &d.zern {
        let a4 = z.coef.get(3).copied().unwrap_or(0.0);
        let k = if z.fringe { 4.0 } else { 4.0 * 3f64.sqrt() };
        c += k * a4 / (z.norm * z.norm);
    }
    if let Some(g) = d.grid.as_ref().filter(|g| !g.linear) {
        let h = 1e-5;
        let a = g.eval(0.0, h).map_or(0.0, |v| v.2);
        let b = g.eval(0.0, -h).map_or(0.0, |v| v.2);
        c += (a - b) / (2.0 * h);
    }
    c
}

/// 基準の二次曲面（c・k）との交点までの距離（頂点面上の pl から方向 kl に）。交わらなければ 0
fn conic_isect(c: f64, k: f64, pl: V3, kl: V3) -> f64 {
    if c == 0.0 {
        return 0.0;
    }
    let ff = c * (pl[0] * pl[0] + pl[1] * pl[1]);
    let gg = kl[2] - c * (pl[0] * kl[0] + pl[1] * kl[1]);
    let aa = c * (kl[0] * kl[0] + kl[1] * kl[1] + (1.0 + k) * kl[2] * kl[2]);
    let disc = gg * gg - aa * ff;
    if disc >= 0.0 {
        let s = ff / (gg + gg.signum() * disc.sqrt());
        if s.is_finite() {
            return s;
        }
    }
    0.0
}

/// 形 f(x, y) = (z, ∂z/∂x, ∂z/∂y) の面との交点までの距離（頂点面上の pl から方向 kl に）。
/// 初期値は二次曲面（c・k）との交点、その後ニュートン法。収束しなければ true も返す
pub fn isect_with(
    c: f64,
    k: f64,
    pl: V3,
    kl: V3,
    f: impl Fn(f64, f64) -> (f64, f64, f64),
) -> (f64, bool) {
    let mut s = conic_isect(c, k, pl, kl);
    let mut ds = f64::NAN;
    for _ in 0..60 {
        let q = add(pl, scale(kl, s));
        let (z, zx, zy) = f(q[0], q[1]);
        ds = (q[2] - z) / (kl[2] - zx * kl[0] - zy * kl[1]);
        s -= ds;
        if !(ds.abs() > 1e-14) {
            break;
        }
    }
    (s, !(ds.abs() <= 1e-10))
}

/// [`is_freeform`] の面との交点までの距離
pub fn isect(d: &Surface, pl: V3, kl: V3) -> (f64, bool) {
    isect_with(d.c, d.k, pl, kl, |x, y| sag_xy(d, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "serde")]
    #[test]
    fn serde_rebuilds_groups_from_coef() {
        // 書き出しに内部の多項式は入らず、読み直すと coef から作り直す
        let mut z = ZernSag::new(
            false,
            5.0,
            vec![0.0, 1e-3, 0.0, 2e-3, 0.0, 0.0, 0.0, 0.0, 0.0, 3e-4],
            false,
        );
        z.dx = 0.1;
        let txt = serde_json::to_string(&z).unwrap();
        assert!(!txt.contains("groups") && !txt.contains("mmax"), "{txt}");
        let back: ZernSag = serde_json::from_str(&txt).unwrap();
        assert_eq!(back, z);
        assert_eq!(back.eval(1.0, 2.0), z.eval(1.0, 2.0));
        // 食い違った内部の値を与えても使わない（余分なフィールドは無視して作り直す）
        let bad = txt.replacen('{', r#"{"groups":[[40,true,[1.0]]],"mmax":40,"#, 1);
        let back: ZernSag = serde_json::from_str(&bad).unwrap();
        assert_eq!(back, z);
        assert_eq!(back.eval(1.0, 2.0), z.eval(1.0, 2.0));
        // 項の数が上限を超えるものは断る
        for (fringe, n) in [(true, 38), (false, 232)] {
            let big = ZernSag {
                coef: vec![1e-6; n],
                fringe,
                ..z.clone()
            };
            let e = serde_json::from_str::<ZernSag>(&serde_json::to_string(&big).unwrap())
                .unwrap_err()
                .to_string();
            assert!(e.contains("上限"), "{e}");
        }
    }

    #[test]
    fn noll_and_fringe_order() {
        let noll: Vec<_> = (1..=11).map(|j| zern_nm(j, false)).collect();
        assert_eq!(
            noll,
            vec![
                (0, 0, true),
                (1, 1, true),
                (1, 1, false),
                (2, 0, true),
                (2, 2, false),
                (2, 2, true),
                (3, 1, false),
                (3, 1, true),
                (3, 3, false),
                (3, 3, true),
                (4, 0, true),
            ]
        );
        let fr: Vec<_> = [1, 2, 3, 4, 5, 6, 7, 9, 10, 16, 17, 25, 36, 37]
            .iter()
            .map(|&j| zern_nm(j, true))
            .collect();
        assert_eq!(
            fr,
            vec![
                (0, 0, true),
                (1, 1, true),
                (1, 1, false),
                (2, 0, true),
                (2, 2, true),
                (2, 2, false),
                (3, 1, true),
                (4, 0, true),
                (3, 3, true),
                (6, 0, true),
                (4, 4, true),
                (8, 0, true),
                (10, 0, true),
                (12, 0, true),
            ]
        );
    }

    #[test]
    fn zernike_values_and_gradient() {
        // Noll Z4 = √3 (2ρ² − 1)、Z5 = √6 ρ² sin 2φ、Z8 = √8 (3ρ³ − 2ρ) cos φ
        let (x, y, r) = (0.7, -1.3, 2.5);
        let (u, v) = (x / r, y / r);
        let rho2: f64 = u * u + v * v;
        let one = |j: usize| {
            let mut c = vec![0.0; j];
            c[j - 1] = 1.0;
            ZernSag::new(false, r, c, true)
        };
        let phi = v.atan2(u);
        let rho = rho2.sqrt();
        assert!((one(4).eval(x, y).0 - 3f64.sqrt() * (2.0 * rho2 - 1.0)).abs() < 1e-14);
        assert!((one(5).eval(x, y).0 - 6f64.sqrt() * rho2 * (2.0 * phi).sin()).abs() < 1e-14);
        assert!(
            (one(8).eval(x, y).0 - 8f64.sqrt() * (3.0 * rho.powi(3) - 2.0 * rho) * phi.cos()).abs()
                < 1e-14
        );
        // 勾配を差分と比べる（231 項・Fringe 37 項）
        for z in [
            ZernSag::new(
                false,
                r,
                (1..=231).map(|i| (i as f64 * 0.37).sin()).collect(),
                true,
            ),
            ZernSag::new(
                true,
                r,
                (1..=37).map(|i| (i as f64 * 0.91).cos()).collect(),
                true,
            ),
        ] {
            let h = 1e-6;
            let (_, zx, zy) = z.eval(x, y);
            let nx = (z.eval(x + h, y).0 - z.eval(x - h, y).0) / (2.0 * h);
            let ny = (z.eval(x, y + h).0 - z.eval(x, y - h).0) / (2.0 * h);
            assert!((zx - nx).abs() < 1e-6 * (1.0 + zx.abs()), "{zx} {nx}");
            assert!((zy - ny).abs() < 1e-6 * (1.0 + zy.abs()), "{zy} {ny}");
            let s: f64 = z
                .coef
                .iter()
                .enumerate()
                .map(|(i, &a)| a * z.term(i + 1, x, y).0)
                .sum();
            assert!((s - z.eval(x, y).0).abs() < 1e-10, "{s}");
        }
    }

    #[test]
    fn grid_bicubic_reproduces_cubic() {
        // 双三次の多項式は、正しい微分を与えれば双三次の補間でそのまま戻る
        let f = |x: f64, y: f64| {
            (
                0.1 * x * x * x - 0.2 * x * y * y + 0.05 * x * y + 0.3 * y,
                0.3 * x * x - 0.2 * y * y + 0.05 * y,
                -0.4 * x * y + 0.05 * x + 0.3,
                -0.4 * y + 0.05,
            )
        };
        let (nx, ny, dx, dy, xd, yd) = (7, 5, 0.5, 0.8, 0.1, -0.2);
        let mut text = format!("{nx} {ny} {dx} {dy} 0 {xd} {yd}\n");
        for row in 0..ny {
            for ix in 0..nx {
                let x = (ix as f64 - 3.0) * dx + xd;
                let y = ((ny - 1 - row) as f64 - 2.0) * dy + yd;
                let (z, zx, zy, zxy) = f(x, y);
                text += &format!("{z} {zx} {zy} {zxy} 0\n");
            }
        }
        let g = GridSag::from_dat(&text).unwrap();
        for (x, y) in [(0.33, 0.21), (-1.2, -1.5), (1.4, 1.3), (0.1, -0.2)] {
            let (z, zx, zy) = g.eval(x, y).unwrap();
            let e = f(x, y);
            assert!(
                (z - e.0).abs() < 1e-13 && (zx - e.1).abs() < 1e-12 && (zy - e.2).abs() < 1e-12
            );
        }
        assert_eq!(g.eval(1.8, 0.0), Some((0.0, 0.0, 0.0)));
    }
}
