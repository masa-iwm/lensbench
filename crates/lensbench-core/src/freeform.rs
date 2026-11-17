//! 回転対称でない矢高の面: Zernike Standard Sag・Zernike Fringe Sag・Grid Sag。
//!
//! どれも「基準の面（曲率 c・円錐係数 k・偶数次非球面 r^2〜r^16）＋追加の矢高 z(x, y)」の形で、
//! 交点はニュートン法、屈折・反射の法線は (−∂z/∂x, −∂z/∂y, 1) で求める（[`crate::shape::Shape`]・[`isect_with`]）。
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
    /// (m, cos か) ごとにまとめた Jacobi 多項式 P_k^(0, m)(2ρ² − 1) の係数（項の係数と規格化を掛けたもの、k = (n − m)/2 の順）
    #[cfg_attr(feature = "serde", serde(skip))]
    groups: Vec<ZGroup>,
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
            return Err(format!("{} Zernike terms (at most {max})", d.coef.len()));
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
    let m = if n.is_multiple_of(2) {
        2 * k.div_ceil(2)
    } else {
        2 * (k / 2) + 1
    };
    // m > 0: j が偶数なら cos、奇数なら sin（Noll）
    (n as u32, m as u32, m == 0 || j.is_multiple_of(2))
}

/// 項 (n, m) の規格化の係数（Standard は √(n + 1)・√(2(n + 1))、Fringe は 1）
fn radial_norm(n: u32, m: u32, normalize: bool) -> f64 {
    if !normalize {
        1.0
    } else if m == 0 {
        ((n + 1) as f64).sqrt()
    } else {
        (2.0 * (n + 1) as f64).sqrt()
    }
}

/// (m, cos か) ごとにまとめた項: Σ_k a_k Q_k(x) の x = 2ρ² − 1 のべきの係数。Q_k = P_k^(0, m)（Jacobi 多項式）で、
/// R_(m+2k)^m(ρ) = ρ^m Q_k(2ρ² − 1)
#[derive(Debug, Clone, PartialEq)]
struct ZGroup {
    m: u32,
    cos: bool,
    /// x のべきの係数（低い次数から）
    px: Vec<f64>,
}

impl ZGroup {
    /// `a`: Q_k の係数（項の係数と規格化を掛けたもの、k = (n − m)/2 の順）。
    ///
    /// 係数は Q_k の 3 項漸化式で作る。ρ² のべきにすると係数が交互の符号で 1e5 台になり ρ → 1 で 3e-10 ずれたが、
    /// 区間の中心の x のべきなら 3e-13 に収まり（n ≤ 20）、光線ごとの評価は Horner（漸化式をそのまま回すより 3 倍ほど速い）
    fn new(m: u32, cos: bool, a: Vec<f64>) -> ZGroup {
        let b = m as f64;
        let n = a.len();
        let mut px = vec![0.0; n];
        // Q_0 = 1、Q_1 = 1 + (m + 2)(x − 1)/2
        let h = 0.5 * (b + 2.0);
        let (mut q0, mut q1) = (vec![1.0], vec![1.0 - h, h]);
        px[0] = a[0];
        if n > 1 {
            px[0] += a[1] * q1[0];
            px[1] += a[1] * q1[1];
        }
        for (k, &ak) in a.iter().enumerate().skip(2) {
            // 2k(k + m)(2k + m − 2) Q_k = (2k + m − 1)[(2k + m)(2k + m − 2) x − m²] Q_(k−1)
            //                              − 2(k − 1)(k + m − 1)(2k + m) Q_(k−2)
            let kf = k as f64;
            let c = 2.0 * kf + b;
            let c0 = 2.0 * kf * (kf + b) * (c - 2.0);
            let al = (c - 1.0) * c * (c - 2.0) / c0;
            let be = -(c - 1.0) * b * b / c0;
            let ga = 2.0 * (kf - 1.0) * (kf + b - 1.0) * c / c0;
            let mut q2 = vec![0.0; k + 1];
            for (i, &v) in q1.iter().enumerate() {
                q2[i + 1] += al * v;
                q2[i] += be * v;
            }
            for (i, &v) in q0.iter().enumerate() {
                q2[i] -= ga * v;
            }
            for (i, &v) in q2.iter().enumerate() {
                px[i] += ak * v;
            }
            (q0, q1) = (q1, q2);
        }
        ZGroup { m, cos, px }
    }

    /// Σ_k a_k Q_k(x) と d/dx
    fn eval(&self, x: f64) -> (f64, f64) {
        let (mut v, mut d) = (0.0, 0.0);
        for &c in self.px.iter().rev() {
            d = d * x + v;
            v = v * x + c;
        }
        (v, d)
    }
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
            let k = ((n - m) / 2) as usize;
            let e = match g.iter_mut().position(|e| e.0 == m && e.1 == cs) {
                Some(k) => &mut g[k].2,
                None => {
                    g.push((m, cs, Vec::new()));
                    &mut g.last_mut().expect("row pushed above").2
                }
            };
            if e.len() <= k {
                e.resize(k + 1, 0.0);
            }
            e[k] += a * radial_norm(n, m, !self.fringe);
        }
        g.sort_by_key(|e| (e.0, !e.1));
        self.mmax = g.iter().map(|e| e.0).max().unwrap_or(0);
        self.groups = g
            .into_iter()
            .map(|(m, cs, a)| ZGroup::new(m, cs, a))
            .collect();
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
        let k = ((n - m) / 2) as usize;
        let mut p = vec![0.0; k + 1];
        p[k] = radial_norm(n, m, !self.fringe);
        let g = [ZGroup::new(m, cs, p)];
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
    groups: &[ZGroup],
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
    // ふつうは固定長の配列（m ≤ 31 は Standard 231 項・Fringe 37 項まで）。項数の上限を超えた係数を
    // API から渡されても落ちないよう、超えるときだけ確保する
    let (mut br, mut bi) = ([0.0; 32], [0.0; 32]);
    let (mut vr, mut vi);
    let (wr, wi): (&mut [f64], &mut [f64]) = if mm < 32 {
        (&mut br, &mut bi)
    } else {
        vr = vec![0.0; mm + 1];
        vi = vec![0.0; mm + 1];
        (&mut vr, &mut vi)
    };
    wr[0] = 1.0;
    for m in 1..=mm {
        wr[m] = wr[m - 1] * u - wi[m - 1] * v;
        wi[m] = wr[m - 1] * v + wi[m - 1] * u;
    }
    let (mut z, mut zu, mut zv) = (0.0, 0.0, 0.0);
    for gr in groups {
        let (m, cs) = (gr.m as usize, gr.cos);
        // P(ρ²) = Σ a_k Q_k(2ρ² − 1) と dP/d(ρ²)
        let (pv, dq) = gr.eval(2.0 * r2 - 1.0);
        let dp = 2.0 * dq;
        // A = Re w^m（cos）か Im w^m（sin）、∂A/∂u・∂A/∂v
        let (a, au, av) = if m == 0 {
            (1.0, 0.0, 0.0)
        } else {
            let mf = m as f64;
            if cs {
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
            // 微分ではないので、矢高と傾きは厳密には合わない。OpticStudio との照合で G2 の方向が 3e-13 で一致）
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
    pub fn from_dat(text: &str) -> crate::Result<GridSag> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('!'));
        let head: Vec<f64> = lines
            .next()
            .ok_or("empty file")?
            .split_whitespace()
            .map(|v| v.parse::<f64>().map_err(|e| format!("{v}: {e}")))
            .collect::<Result<_, _>>()?;
        if head.len() < 4 {
            return Err("line 1 must be nx ny delx dely unitflag xdec ydec".into());
        }
        // 点の数は 1〜2^24 の整数（負・小数・巨大な値で確保があふれない）
        let count = |v: f64, what: &str| -> crate::Result<usize> {
            if v.fract() == 0.0 && (1.0..=16_777_216.0).contains(&v) {
                Ok(v as usize)
            } else {
                Err(format!("{what} {v} must be an integer 1 to 16777216").into())
            }
        };
        let (nx, ny) = (count(head[0], "nx")?, count(head[1], "ny")?);
        if nx * ny > 16_777_216 {
            return Err(format!("too many points {nx}×{ny} (up to 16777216)").into());
        }
        let unit = head.get(4).copied().unwrap_or(0.0) as i32;
        let sc = match unit {
            0 => 1.0,
            1 => 10.0,
            2 => 25.4,
            3 => 1000.0,
            u => return Err(format!("unit number {u}").into()),
        };
        let mut z = vec![[0.0; 4]; nx * ny];
        let mut nodata = vec![false; nx * ny];
        for r in 0..nx * ny {
            let l = lines
                .next()
                .ok_or_else(|| format!("too few points ({r} / {})", nx * ny))?;
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

/// 回転対称でない矢高の面タイプか（[`crate::shape::Shape::is_freeform`]。面の値によらない）
pub fn is_freeform(t: SurfType) -> bool {
    Surface {
        typ: t,
        ..Default::default()
    }
    .shape()
    .is_freeform()
}

/// 面の矢高と (∂z/∂x, ∂z/∂y)（面の局所座標）。面の外（基準の面の平方根が負、値のない格子点）は NaN。
/// 形どおり（[`crate::shape::Shape::sag`]、追跡と同じ形）: トロイダル面は x・y の曲線、Cylinder Fresnel は y だけの
/// 基板、Fresnel・理想レンズ・Jones などは頂点の平面（Fresnel の曲率は屈折の傾きだけで、面は平らな基板）。
/// 以前は回転対称でない面のほかを c・k・asph の偶数次非球面として求めていた
pub fn sag_xy(d: &Surface, x: f64, y: f64) -> (f64, f64, f64) {
    d.shape().sag(x, y)
}

/// 基準の二次曲面（c・k）との交点までの距離（頂点面上の pl から方向 kl に）。交わらなければ 0
fn conic_isect(c: f64, k: f64, pl: V3, kl: V3, sig: f64) -> f64 {
    if c == 0.0 {
        return 0.0;
    }
    let ff = c * (pl[0] * pl[0] + pl[1] * pl[1]);
    let gg = kl[2] - c * (pl[0] * kl[0] + pl[1] * kl[1]);
    let aa = c * (kl[0] * kl[0] + kl[1] * kl[1] + (1.0 + k) * kl[2] * kl[2]);
    let disc = gg * gg - aa * ff;
    if disc >= 0.0 {
        let s = crate::shape::conic_root(ff, gg, aa, disc, sig);
        if s.is_finite() {
            return s;
        }
    }
    0.0
}

/// 形 f(x, y) = (z, ∂z/∂x, ∂z/∂y) の面との交点までの距離（頂点面上の pl から方向 kl に）。
/// 初期値は二次曲面（c・k）との交点、その後ニュートン法。収束しなければ true も返す
pub(crate) fn isect_with(
    c: f64,
    k: f64,
    pl: V3,
    kl: V3,
    sig: f64,
    f: impl Fn(f64, f64) -> (f64, f64, f64),
) -> (f64, bool) {
    let mut s = conic_isect(c, k, pl, kl, sig);
    let mut ds = f64::NAN;
    for _ in 0..60 {
        let q = add(pl, scale(kl, s));
        let (z, zx, zy) = f(q[0], q[1]);
        ds = (q[2] - z) / (kl[2] - zx * kl[0] - zy * kl[1]);
        s -= ds;
        if !(ds.abs() > crate::trace::newton_tol(s, z)) {
            break;
        }
    }
    (s, !(ds.abs() <= 1e-10))
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
            assert!(e.contains("at most"), "{e}");
        }
    }

    #[test]
    fn radial_terms_are_accurate_up_to_order_20() {
        // 動径多項式 R_n^m（n ≤ 20、Standard の 231 項）の ρ^m を除いた部分を、整数の係数（u128 で厳密）の
        // double-double の Horner と比べる。ρ² のべきで f64 の Horner にすると ρ → 1 で 3e-10 ずれた（x = 2ρ² − 1 のべきは 3e-13）
        fn two_sum(a: f64, b: f64) -> (f64, f64) {
            let s = a + b;
            let bb = s - a;
            (s, (a - (s - bb)) + (b - bb))
        }
        let fact = |q: u32| -> u128 { (1..=q as u128).product() };
        let mut worst = 0.0f64;
        for j in 1..=231 {
            let (n, m, _) = zern_nm(j, false);
            let k = ((n - m) / 2) as usize;
            let mut a = vec![0.0; k + 1];
            a[k] = 1.0;
            for i in 0..=200 {
                let r2 = i as f64 / 200.0;
                let mut acc = (0.0f64, 0.0f64);
                for t in (0..=k).rev() {
                    let s = (k - t) as u32;
                    let c = (fact(n - s)
                        / (fact(s) * fact((n + m) / 2 - s) * fact((n - m) / 2 - s)))
                        as f64;
                    let c = if s.is_multiple_of(2) { c } else { -c };
                    // acc = acc * r2 + c（double-double）
                    let p = acc.0 * r2;
                    let e = acc.0.mul_add(r2, -p) + acc.1 * r2;
                    let (hi, lo) = two_sum(p, e);
                    let (s1, e1) = two_sum(hi, c);
                    acc = two_sum(s1, e1 + lo);
                }
                let exact = acc.0 + acc.1;
                let (v, _) = ZGroup::new(m, true, a.clone()).eval(2.0 * r2 - 1.0);
                // 値の大きさ（ρ → 0 で m が大きいと C(k + m, k) 倍になる）に対する誤差
                worst = worst.max((v - exact).abs() / exact.abs().max(1.0));
            }
        }
        assert!(worst < 5e-13, "{worst:e}");
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
            assert!((s - z.eval(x, y).0).abs() < 1e-12, "{s}");
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
