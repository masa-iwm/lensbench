//! 偏光追跡を粗い瞳格子で行い、全光線へ補間して Jones 行列を配る。
//!
//! 伝搬の位相 exp(−ik·OPL) を除いた Jones 行列（[`crate::system::System::trace_pol_launch`] の phase = false）は
//! 瞳の上で滑らかなので、正規化瞳座標の [−1, 1]² の正規の格子（nx × ny 点）で偏光追跡し、その値を 2 次元の
//! 3 次スプライン（not-a-knot の 1 次元スプラインのテンソル積）で補間する。全光線の偏光追跡より速い
//! （偏光追跡は普通の追跡の約 10 倍かかる）。
//!
//! 格子点の扱い:
//! - 追跡が止まった・けられた格子点（瞳の縁の外側など）は、格子の上で最も近い有効な点の値で埋める。
//! - 瞳の内側（px² + py² ≤ 1）で欠けた格子点は「欠け」として返す（[`PolGrid::missing`]）。欠けた点から
//!   [`BAD_MARGIN`] 点の範囲のセルにある光線は NaN にする（黙って埋めない）。
//!
//! 値は格子点ごとに 6 つの複素数: Jones 行列 [xx, xy, yx, yy]（行 = 出射の x・y、列 = 入射の x・y）と
//! z 成分 [z_x, z_y]（列 = 入射の x・y）。

use crate::coating::{Coatings, C64};
use crate::system::{Launch, System};
use crate::{invalid, Result};

/// 格子点ごとの値の数（Jones 4 + z 成分 2）
pub const NV: usize = 6;

/// 欠けた格子点から何点の範囲のセルの光線を補間しないか（スプラインは大域なので、埋めた値の影響は
/// 1 点離れるごとに約 0.27 倍に減る。2 点で 4×4 の近傍の外側にもう 1 点の余裕）
pub const BAD_MARGIN: usize = 2;

/// 補間に使う 1 軸の格子の点数の下限（not-a-knot のスプラインに 4 点要る）
pub const MIN_POINTS: usize = 4;

/// 粗い瞳格子での偏光追跡の結果と、補間の係数
#[derive(Debug, Clone)]
pub struct PolGrid {
    pub nx: usize,
    pub ny: usize,
    /// 格子点の値（j * nx + i の順、i が px の向き）
    f: Vec<[C64; NV]>,
    /// ∂f/∂px・∂f/∂py・∂²f/∂px∂py（スプラインの節点での微分）
    fx: Vec<[C64; NV]>,
    fy: Vec<[C64; NV]>,
    fxy: Vec<[C64; NV]>,
    /// 瞳の内側で欠けた格子点（j * nx + i）
    missing: Vec<usize>,
    /// 補間に使えない格子点（欠けた点。missing と同じ集合を引きやすい形で）
    bad: Vec<bool>,
}

/// 1 軸の格子の座標（[−1, 1] を n − 1 等分）
pub fn axis(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| -1.0 + 2.0 * i as f64 / (n - 1) as f64)
        .collect()
}

impl PolGrid {
    /// 光線の始め方 l（[`Launch::Pupil`]。px・py は無視して格子に置き換える）の物体・ケラレ係数で、
    /// 波長 w の偏光追跡を nx × ny の格子で行う（伝搬の位相なし）
    pub fn trace(
        sys: &System,
        l: &Launch,
        w: usize,
        coat: &Coatings,
        nx: usize,
        ny: usize,
    ) -> Result<PolGrid> {
        if nx < MIN_POINTS || ny < MIN_POINTS {
            invalid!("偏光の格子は各軸 {MIN_POINTS} 点以上（{nx} × {ny}）");
        }
        let (ax, ay) = (axis(nx), axis(ny));
        let mut gx = Vec::with_capacity(nx * ny);
        let mut gy = Vec::with_capacity(nx * ny);
        for &y in &ay {
            for &x in &ax {
                gx.push(x);
                gy.push(y);
            }
        }
        let gl = match *l {
            Launch::Pupil {
                spec,
                signed,
                vig,
                hxy,
                ..
            } => Launch::Pupil {
                spec,
                signed,
                px: &gx,
                py: &gy,
                vig,
                hxy,
            },
            _ => invalid!("偏光の格子は瞳座標（normalized・signed）で始める光線だけ"),
        };
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        let y = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
        let p = sys.trace_pol_launch(&gl, w, &[x, y], coat, false)?;
        let n = nx * ny;
        let mut vals = vec![[C64::new(0.0, 0.0); NV]; n];
        let mut good = vec![false; n];
        for k in 0..n {
            let e = &p.e[k];
            let (ex, ey) = (e[0], e[1]);
            let v = [ex[0], ey[0], ex[1], ey[1], ex[2], ey[2]];
            if p.trace.ok(k) && v.iter().all(|c| c.re.is_finite() && c.im.is_finite()) {
                vals[k] = v;
                good[k] = true;
            }
        }
        if !good.iter().any(|&g| g) {
            invalid!("偏光の格子の点がどれも通らない");
        }
        let inside = |k: usize| gx[k] * gx[k] + gy[k] * gy[k] <= 1.0 + 1e-12;
        let missing: Vec<usize> = (0..n).filter(|&k| !good[k] && inside(k)).collect();
        let mut bad = vec![false; n];
        missing.iter().for_each(|&k| bad[k] = true);
        fill_nearest(&mut vals, &good, nx, ny);
        let (hx, hy) = (2.0 / (nx - 1) as f64, 2.0 / (ny - 1) as f64);
        let sx = SlopeOp::new(nx, hx);
        let sy = SlopeOp::new(ny, hy);
        let fx = sx.along_x(&vals, nx, ny);
        let fy = sy.along_y(&vals, nx, ny);
        let fxy = sx.along_x(&fy, nx, ny);
        Ok(PolGrid {
            nx,
            ny,
            f: vals,
            fx,
            fy,
            fxy,
            missing,
            bad,
        })
    }

    /// 瞳の内側で欠けた格子点の座標 (px, py)
    pub fn missing(&self) -> Vec<[f64; 2]> {
        let (ax, ay) = (axis(self.nx), axis(self.ny));
        self.missing
            .iter()
            .map(|&k| [ax[k % self.nx], ay[k / self.nx]])
            .collect()
    }

    /// 正規化瞳座標 (px, py) での補間値。格子の外（|p| > 1）は端の多項式を延ばす。セルから
    /// [`BAD_MARGIN`] 点の範囲に欠けた格子点があれば None
    pub fn eval(&self, px: f64, py: f64) -> Option<[C64; NV]> {
        if !(px.is_finite() && py.is_finite()) {
            return None;
        }
        let (nx, ny) = (self.nx, self.ny);
        let (hx, hy) = (2.0 / (nx - 1) as f64, 2.0 / (ny - 1) as f64);
        let cell = |p: f64, h: f64, n: usize| -> (usize, f64) {
            let s = (p + 1.0) / h;
            let i = (s.floor().max(0.0) as usize).min(n - 2);
            (i, s - i as f64)
        };
        let (i, t) = cell(px, hx, nx);
        let (j, u) = cell(py, hy, ny);
        if !self.missing.is_empty() {
            let m = BAD_MARGIN;
            let (i0, i1) = (i.saturating_sub(m), (i + 1 + m).min(nx - 1));
            let (j0, j1) = (j.saturating_sub(m), (j + 1 + m).min(ny - 1));
            for jj in j0..=j1 {
                for ii in i0..=i1 {
                    if self.bad[jj * nx + ii] {
                        return None;
                    }
                }
            }
        }
        // 3 次 Hermite の基底（値と、h を掛けた微分）
        let h = |s: f64| {
            let (s2, s3) = (s * s, s * s * s);
            (
                [2.0 * s3 - 3.0 * s2 + 1.0, -2.0 * s3 + 3.0 * s2],
                [s3 - 2.0 * s2 + s, s3 - s2],
            )
        };
        let (ax, bx) = h(t);
        let (ay, by) = h(u);
        let mut out = [C64::new(0.0, 0.0); NV];
        for (b, jj) in [j, j + 1].into_iter().enumerate() {
            for (a, ii) in [i, i + 1].into_iter().enumerate() {
                let k = jj * nx + ii;
                let w0 = ax[a] * ay[b];
                let w1 = bx[a] * ay[b] * hx;
                let w2 = ax[a] * by[b] * hy;
                let w3 = bx[a] * by[b] * hx * hy;
                for c in 0..NV {
                    out[c] += self.f[k][c] * w0
                        + self.fx[k][c] * w1
                        + self.fy[k][c] * w2
                        + self.fxy[k][c] * w3;
                }
            }
        }
        Some(out)
    }
}

/// 有効でない点を、格子の上で最も近い（添字の距離の 2 乗が最小、同じなら添字の小さい）有効な点の値で埋める
fn fill_nearest(v: &mut [[C64; NV]], good: &[bool], nx: usize, ny: usize) {
    let gk: Vec<usize> = (0..nx * ny).filter(|&k| good[k]).collect();
    for k in 0..nx * ny {
        if good[k] {
            continue;
        }
        let (i, j) = ((k % nx) as i64, (k / nx) as i64);
        let mut best = (i64::MAX, 0);
        for &q in &gk {
            let (a, b) = ((q % nx) as i64 - i, (q / nx) as i64 - j);
            let d = a * a + b * b;
            if d < best.0 {
                best = (d, q);
            }
        }
        v[k] = v[best.1];
    }
}

/// 等間隔 n 点の値から not-a-knot の 3 次スプラインの節点の傾きを出す線形の演算子（n × n の行列）
struct SlopeOp {
    n: usize,
    m: Vec<f64>,
}

impl SlopeOp {
    fn new(n: usize, h: f64) -> SlopeOp {
        // 2 階微分 M: 内側 M[i−1] + 4M[i] + M[i+1] = 6(f[i+1] − 2f[i] + f[i−1])/h²、
        // 両端 not-a-knot: M0 − 2M1 + M2 = 0、M[n−3] − 2M[n−2] + M[n−1] = 0
        let mut a = vec![0.0; n * n];
        let mut r = vec![0.0; n * n]; // 右辺の f に掛かる係数
        a[0] = 1.0;
        a[1] = -2.0;
        a[2] = 1.0;
        for i in 1..n - 1 {
            a[i * n + i - 1] = 1.0;
            a[i * n + i] = 4.0;
            a[i * n + i + 1] = 1.0;
            let c = 6.0 / (h * h);
            r[i * n + i - 1] = c;
            r[i * n + i] = -2.0 * c;
            r[i * n + i + 1] = c;
        }
        let l = n - 1;
        a[l * n + l - 2] = 1.0;
        a[l * n + l - 1] = -2.0;
        a[l * n + l] = 1.0;
        // M = A⁻¹ R f
        let mf = solve_mat(&a, &r, n);
        // 傾き d[i] = (f[i+1] − f[i])/h − h(2M[i] + M[i+1])/6（i < n−1）、
        // d[n−1] = (f[n−1] − f[n−2])/h + h(M[n−2] + 2M[n−1])/6
        let mut m = vec![0.0; n * n];
        for i in 0..n {
            let row = &mut m[i * n..(i + 1) * n];
            if i < n - 1 {
                row[i + 1] += 1.0 / h;
                row[i] -= 1.0 / h;
                for c in 0..n {
                    row[c] -= h * (2.0 * mf[i * n + c] + mf[(i + 1) * n + c]) / 6.0;
                }
            } else {
                row[i] += 1.0 / h;
                row[i - 1] -= 1.0 / h;
                for c in 0..n {
                    row[c] += h * (mf[(i - 1) * n + c] + 2.0 * mf[i * n + c]) / 6.0;
                }
            }
        }
        SlopeOp { n, m }
    }

    fn apply(&self, src: &[[C64; NV]], dst: &mut [[C64; NV]], idx: impl Fn(usize) -> usize) {
        let n = self.n;
        for i in 0..n {
            let mut acc = [C64::new(0.0, 0.0); NV];
            for c in 0..n {
                let w = self.m[i * n + c];
                if w != 0.0 {
                    let s = &src[idx(c)];
                    for q in 0..NV {
                        acc[q] += s[q] * w;
                    }
                }
            }
            dst[idx(i)] = acc;
        }
    }

    /// 各行（py 一定）に沿った傾き
    fn along_x(&self, v: &[[C64; NV]], nx: usize, ny: usize) -> Vec<[C64; NV]> {
        let mut d = vec![[C64::new(0.0, 0.0); NV]; nx * ny];
        for j in 0..ny {
            self.apply(v, &mut d, |i| j * nx + i);
        }
        d
    }

    /// 各列（px 一定）に沿った傾き
    fn along_y(&self, v: &[[C64; NV]], nx: usize, ny: usize) -> Vec<[C64; NV]> {
        let mut d = vec![[C64::new(0.0, 0.0); NV]; nx * ny];
        for i in 0..nx {
            self.apply(v, &mut d, |j| j * nx + i);
        }
        d
    }
}

/// A X = B（n × n、行優先）を部分ピボットの Gauss 消去で解く
fn solve_mat(a: &[f64], b: &[f64], n: usize) -> Vec<f64> {
    let mut a = a.to_vec();
    let mut x = b.to_vec();
    for col in 0..n {
        let p = (col..n)
            .max_by(|&r, &s| a[r * n + col].abs().total_cmp(&a[s * n + col].abs()))
            .unwrap();
        if p != col {
            for c in 0..n {
                a.swap(p * n + c, col * n + c);
                x.swap(p * n + c, col * n + c);
            }
        }
        let d = a[col * n + col];
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r * n + col] / d;
            if f == 0.0 {
                continue;
            }
            for c in 0..n {
                a[r * n + c] -= f * a[col * n + c];
                x[r * n + c] -= f * x[col * n + c];
            }
        }
    }
    for r in 0..n {
        let d = a[r * n + r];
        for c in 0..n {
            x[r * n + c] /= d;
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3 次の多項式は not-a-knot のスプラインで厳密に再現する
    #[test]
    fn slope_exact_for_cubic() {
        let n = 7;
        let h = 2.0 / (n - 1) as f64;
        let op = SlopeOp::new(n, h);
        let xs = axis(n);
        let f = |x: f64| 0.3 - 1.2 * x + 0.7 * x * x + 2.1 * x * x * x;
        let df = |x: f64| -1.2 + 1.4 * x + 6.3 * x * x;
        for i in 0..n {
            let d: f64 = (0..n).map(|c| op.m[i * n + c] * f(xs[c])).sum();
            assert!((d - df(xs[i])).abs() < 1e-12, "{i}: {d} {}", df(xs[i]));
        }
    }

    /// 双 3 次の多項式は補間で厳密に再現する（格子点の間・格子の外の延長でも）
    #[test]
    fn eval_exact_for_bicubic() {
        let (nx, ny) = (6, 5);
        let g = |x: f64, y: f64| {
            C64::new(
                1.0 + x - 0.5 * y + 0.3 * x * y * y + 0.8 * x * x * x * y,
                0.2 * y * y * y - x * x,
            )
        };
        let (ax, ay) = (axis(nx), axis(ny));
        let mut f = Vec::new();
        for &y in &ay {
            for &x in &ax {
                f.push([g(x, y); NV]);
            }
        }
        let (hx, hy) = (2.0 / (nx - 1) as f64, 2.0 / (ny - 1) as f64);
        let (sx, sy) = (SlopeOp::new(nx, hx), SlopeOp::new(ny, hy));
        let fy = sy.along_y(&f, nx, ny);
        let pg = PolGrid {
            nx,
            ny,
            fx: sx.along_x(&f, nx, ny),
            fxy: sx.along_x(&fy, nx, ny),
            fy,
            f,
            missing: vec![],
            bad: vec![false; nx * ny],
        };
        for &(x, y) in &[
            (0.13, -0.77),
            (-0.99, 0.5),
            (1.0, 1.0),
            (0.0, 0.0),
            (1.02, -1.01),
        ] {
            let v = pg.eval(x, y).unwrap()[3];
            assert!((v - g(x, y)).norm() < 1e-12, "({x}, {y}): {v} {}", g(x, y));
        }
    }

    #[test]
    fn fill_nearest_picks_closest() {
        let (nx, ny) = (4, 4);
        let mut v = vec![[C64::new(0.0, 0.0); NV]; 16];
        let mut good = vec![false; 16];
        v[5][0] = C64::new(1.0, 0.0);
        good[5] = true;
        v[15][0] = C64::new(2.0, 0.0);
        good[15] = true;
        fill_nearest(&mut v, &good, nx, ny);
        assert_eq!(v[0][0].re, 1.0);
        assert_eq!(v[14][0].re, 2.0);
        assert_eq!(v[10][0].re, 1.0); // (2, 2) は 5 にも 15 にも距離² 2 → 添字の小さい 5
    }
}
