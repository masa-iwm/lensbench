//! 小さな線形代数（密行列）。解法はここに 1 つずつ置き、解析・最適化・膜の当てはめはこれを使う。
//!
//! - [`Lu`]: 正方行列の LU（部分ピボット）。特異なら None
//! - [`solve_or_zero`]: 正方行列の消去で、ピボットが 0 の未知数は 0（傾きが決まらないとき）
//! - [`Cholesky`]: 対称正定値（正規方程式など）。正定値でなければ None
//! - [`Qr`]・[`lstsq`]・[`lstsq_cols`]: 最小二乗の Householder QR。0 の列（階数が落ちる）の未知数は 0
//! - [`solve_tridiag`]: 三重対角（スプラインの節点の傾き）。特異なら None

/// 小さな正方行列の LU 分解（部分ピボット、行優先）
pub struct Lu {
    a: Vec<f64>,
    piv: Vec<usize>,
    n: usize,
}

impl Lu {
    pub fn new(mut a: Vec<f64>, n: usize) -> Option<Lu> {
        let scale = a.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        if !(scale > 0.0) || !scale.is_finite() {
            return None;
        }
        let mut piv: Vec<usize> = (0..n).collect();
        for k in 0..n {
            let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
            if !(a[p * n + k].abs() > 1e-14 * scale) {
                return None;
            }
            if p != k {
                for c in 0..n {
                    a.swap(k * n + c, p * n + c);
                }
                piv.swap(k, p);
            }
            for i in k + 1..n {
                let f = a[i * n + k] / a[k * n + k];
                a[i * n + k] = f;
                for c in k + 1..n {
                    a[i * n + c] -= f * a[k * n + c];
                }
            }
        }
        Some(Lu { a, piv, n })
    }

    pub fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = self.n;
        let mut x: Vec<f64> = self.piv.iter().map(|&i| b[i]).collect();
        for i in 0..n {
            for c in 0..i {
                x[i] -= self.a[i * n + c] * x[c];
            }
        }
        for i in (0..n).rev() {
            for c in i + 1..n {
                x[i] -= self.a[i * n + c] * x[c];
            }
            x[i] /= self.a[i * n + i];
        }
        x
    }
}

/// 小さな連立一次方程式（部分ピボットの消去）。ピボットが 0 の未知数は 0 にする（傾きが決まらないとき）
pub fn solve_or_zero(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    let scale = a
        .iter()
        .flatten()
        .fold(0.0f64, |x, y| x.max(y.abs()))
        .max(1e-300);
    let mut skip = vec![false; n];
    for c in 0..n {
        let p = (c..n)
            .max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))
            .expect("non-empty range");
        if !(a[p][c].abs() > 1e-14 * scale) {
            skip[c] = true;
            continue;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for c in (0..n).rev() {
        if skip[c] {
            continue;
        }
        let mut t = b[c];
        for k in c + 1..n {
            t -= a[c][k] * x[k];
        }
        x[c] = t / a[c][c];
    }
    x
}

/// 対称正定値 g（n×n、行優先）の Cholesky 分解 g = L Lᵀ
#[derive(Debug, Clone)]
pub struct Cholesky {
    /// L（下三角、行優先）
    l: Vec<f64>,
    n: usize,
}

impl Cholesky {
    /// 分解する。対角の残り d が d ≤ 1e-14 · g_ii（正定値でない・条件数が 1e14 ほどを超える）なら None
    pub fn new(g: &[f64], n: usize) -> Option<Cholesky> {
        let mut l = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..=i {
                let s: f64 = (0..j).map(|k| l[i * n + k] * l[j * n + k]).sum();
                if i == j {
                    let d = g[i * n + i] - s;
                    if !(d > 1e-14 * g[i * n + i].abs()) {
                        return None;
                    }
                    l[i * n + i] = d.sqrt();
                } else {
                    l[i * n + j] = (g[i * n + j] - s) / l[j * n + j];
                }
            }
        }
        Some(Cholesky { l, n })
    }

    /// g x = b を解く
    pub fn solve(&self, b: &[f64]) -> Vec<f64> {
        let (l, n) = (&self.l, self.n);
        let mut y = vec![0.0; n];
        for i in 0..n {
            let s: f64 = (0..i).map(|k| l[i * n + k] * y[k]).sum();
            y[i] = (b[i] - s) / l[i * n + i];
        }
        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let s: f64 = (i + 1..n).map(|k| l[k * n + i] * x[k]).sum();
            x[i] = (y[i] - s) / l[i * n + i];
        }
        x
    }
}

/// 内積（4 本に分けて足す。足す順は長さだけで決まる）
#[inline]
#[doc(hidden)]
pub fn dot4(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut s = [0.0f64; 4];
    let (ca, ra) = a.as_chunks::<4>();
    let (cb, rb) = b.as_chunks::<4>();
    for (x, y) in ca.iter().zip(cb) {
        for k in 0..4 {
            s[k] += x[k] * y[k];
        }
    }
    let mut t = (s[0] + s[1]) + (s[2] + s[3]);
    for (x, y) in ra.iter().zip(rb) {
        t += x * y;
    }
    t
}

/// [`dot4`] の a[0]・b[0] を a0・b0 に置き換えた値（足す順は同じ）
fn dot4_with(a: &[f64], b: &[f64], a0: Option<f64>, b0: Option<f64>) -> f64 {
    let n = a.len().min(b.len());
    let at = |i: usize| if i == 0 { a0.unwrap_or(a[0]) } else { a[i] };
    let bt = |i: usize| if i == 0 { b0.unwrap_or(b[0]) } else { b[i] };
    let mut s = [0.0f64; 4];
    let full = n / 4 * 4;
    for c in (0..full).step_by(4) {
        for k in 0..4 {
            s[k] += at(c + k) * bt(c + k);
        }
    }
    let mut t = (s[0] + s[1]) + (s[2] + s[3]);
    for i in full..n {
        t += at(i) * bt(i);
    }
    t
}

/// m × n の A の Householder QR。右辺 b ごとに最小二乗 min ‖A x − b‖ を解く（[`Qr::solve`]）
#[derive(Debug, Clone)]
pub struct Qr {
    /// 列優先（列 j が q[j*m..(j+1)*m]）。対角より上は R、列 k の行 k.. は反射のベクトル v
    q: Vec<f64>,
    m: usize,
    n: usize,
    /// 列 k の反射の |v|²（反射しない列は 0）
    vn: Vec<f64>,
    /// R の対角（0 の列は階数が落ちた列）
    rdiag: Vec<f64>,
}

impl Qr {
    /// 列優先の A（列 j が q[j*m..(j+1)*m]）を分解する。大きければ列ごとに並列（値は同じ）
    pub fn from_cols(mut q: Vec<f64>, m: usize) -> Qr {
        use rayon::prelude::*;
        let n = q.len().checked_div(m).unwrap_or(0);
        let (mut vn, mut rdiag) = (vec![0.0; n], vec![0.0; n]);
        let par = m * n >= 1 << 16 && !crate::par::sequential();
        for k in 0..n.min(m) {
            let (done, rest) = q.split_at_mut((k + 1) * m);
            let ck = &mut done[k * m..];
            let nrm = dot4(&ck[k..], &ck[k..]).sqrt();
            if nrm == 0.0 {
                continue;
            }
            let c0 = ck[k];
            let alpha = if c0 > 0.0 { -nrm } else { nrm };
            // v = 列 k の行 k.. − alpha e_k。v の行 k+1.. は列の値そのものなので、列の行 k を v[0] にして列に持つ
            let v0 = c0 - alpha;
            let w = dot4_with(&ck[k..], &ck[k..], Some(v0), Some(v0));
            if w == 0.0 {
                rdiag[k] = alpha;
                continue;
            }
            // 列 k 自身に反射を掛けた行 k の値
            let s0 = dot4_with(&ck[k..], &ck[k..], Some(c0), Some(v0)) * 2.0 / w;
            rdiag[k] = c0 - s0 * v0;
            ck[k] = v0;
            let v = &ck[k..];
            let refl = |col: &mut [f64]| {
                let s = dot4(&col[k..], v) * 2.0 / w;
                for (c, vi) in col[k..].iter_mut().zip(v) {
                    *c -= s * vi;
                }
            };
            // 残りの列は互いに独立（列ごとに同じ計算なので並列でも同じ値）
            if par {
                rest.par_chunks_mut(m).for_each(crate::par::scoped(refl));
            } else {
                rest.chunks_mut(m).for_each(refl);
            }
            vn[k] = w;
        }
        Qr { q, m, n, vn, rdiag }
    }

    /// 行優先の A（m × n）を分解する
    pub fn from_rows(a: &[f64], m: usize, n: usize) -> Qr {
        let mut q = vec![0.0; m * n];
        for i in 0..m {
            for j in 0..n {
                q[j * m + i] = a[i * n + j];
            }
        }
        Qr::from_cols(q, m)
    }

    /// 列の数
    pub fn cols(&self) -> usize {
        self.n
    }

    /// R の対角（0 の列は階数が落ちた列）
    pub fn rdiag(&self) -> &[f64] {
        &self.rdiag
    }

    /// Qᵀ b
    fn qt(&self, b: &[f64]) -> Vec<f64> {
        let m = self.m;
        let mut y = b.to_vec();
        for k in 0..self.n.min(m) {
            let w = self.vn[k];
            if w == 0.0 {
                continue;
            }
            let v = &self.q[k * m + k..(k + 1) * m];
            let s = dot4(&y[k..], v) * 2.0 / w;
            for (c, vi) in y[k..].iter_mut().zip(v) {
                *c -= s * vi;
            }
        }
        y
    }

    /// 最小二乗 min ‖A x − b‖ の x。R の対角が 0 の列の未知数は 0
    pub fn solve(&self, b: &[f64]) -> Vec<f64> {
        let (m, n) = (self.m, self.n);
        let y = self.qt(b);
        let mut x = vec![0.0; n];
        for k in (0..n.min(m)).rev() {
            let s: f64 = (k + 1..n).map(|j| self.q[j * m + k] * x[j]).sum();
            x[k] = if self.rdiag[k] != 0.0 {
                (y[k] - s) / self.rdiag[k]
            } else {
                0.0
            };
        }
        x
    }

    /// 最小二乗の残差のノルム ‖A x − b‖（Qᵀb の行 n.. のノルム）
    pub fn residual(&self, b: &[f64]) -> f64 {
        let y = self.qt(b);
        y[self.n.min(self.m)..]
            .iter()
            .map(|x| x * x)
            .sum::<f64>()
            .sqrt()
    }
}

/// 最小二乗 A x ≈ b（A は行の並び）の Householder QR。0 の列の未知数は 0
pub fn lstsq(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let m = a.len();
    if m == 0 {
        return Vec::new();
    }
    let n = a[0].len();
    let mut q = vec![0.0; m * n];
    for (i, row) in a.iter().enumerate() {
        for (j, &v) in row.iter().enumerate() {
            q[j * m + i] = v;
        }
    }
    lstsq_cols(q, m, b)
}

/// 最小二乗 A x ≈ b（A は列優先、列 j が q[j*m..(j+1)*m]）の Householder QR。0 の列の未知数は 0
pub fn lstsq_cols(q: Vec<f64>, m: usize, b: &[f64]) -> Vec<f64> {
    if m == 0 {
        return Vec::new();
    }
    Qr::from_cols(q, m).solve(b)
}

/// 三重対角の連立方程式（部分ピボット付きの消去。行 0 と n−1 の対角が小さくてもよいように）
pub fn solve_tridiag(sub: &[f64], dia: &[f64], sup: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    // 一般の帯行列として小さな密行列で解く（点数は 100 程度まで）
    let n = dia.len();
    let mut a = vec![vec![0.0; n + 1]; n];
    for i in 0..n {
        a[i][i] = dia[i];
        if i > 0 {
            a[i][i - 1] = sub[i];
        }
        if i + 1 < n {
            a[i][i + 1] = sup[i];
        }
        a[i][n] = b[i];
    }
    // 行 0 は (dia0, sup0)、行 n−1 は (sub, dia) の 2 要素。ピボット付きガウス消去
    for col in 0..n {
        let lim = (col + 3).min(n);
        let p = (col..lim).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[p][col] == 0.0 {
            return None;
        }
        a.swap(col, p);
        for r in col + 1..lim {
            let f = a[r][col] / a[col][col];
            if f != 0.0 {
                for c in col..=n {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut v = a[i][n];
        for c in i + 1..n {
            v -= a[i][c] * x[c];
        }
        x[i] = v / a[i][i];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_solves_least_squares_and_gives_the_residual() {
        // 直線 y = 1 + 2x の点に ±0.5 の誤差: 解は 1・2、残差は √(4 × 0.25) = 1
        let rows: Vec<f64> = (0..4).flat_map(|i| [1.0, i as f64]).collect();
        let b: Vec<f64> = (0..4)
            .map(|i| 1.0 + 2.0 * i as f64 + if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let qr = Qr::from_rows(&rows, 4, 2);
        let x = qr.solve(&b);
        // 誤差 (+,−,+,−) は 1・x に直交しないので、最小二乗の解を正規方程式で確かめる
        let g = [4.0, 6.0, 6.0, 14.0];
        let atb = [
            b.iter().sum::<f64>(),
            b.iter().enumerate().map(|(i, v)| i as f64 * v).sum(),
        ];
        let want = Lu::new(g.to_vec(), 2).unwrap().solve(&atb);
        assert!((x[0] - want[0]).abs() < 1e-12 && (x[1] - want[1]).abs() < 1e-12);
        let r: f64 = (0..4)
            .map(|i| (x[0] + x[1] * i as f64 - b[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((qr.residual(&b) - r).abs() < 1e-12);
        // 0 の列の未知数は 0
        let z = lstsq(
            &[vec![1.0, 0.0], vec![1.0, 0.0], vec![1.0, 0.0]],
            &[1.0, 2.0, 3.0],
        );
        assert_eq!(z, vec![2.0, 0.0]);
    }

    #[test]
    fn cholesky_and_tridiagonal_solve() {
        let g = [4.0, 2.0, 0.0, 2.0, 5.0, 1.0, 0.0, 1.0, 3.0];
        let x = Cholesky::new(&g, 3).unwrap().solve(&[2.0, 8.0, 4.0]);
        for i in 0..3 {
            let r: f64 = (0..3).map(|j| g[i * 3 + j] * x[j]).sum();
            assert!((r - [2.0, 8.0, 4.0][i]).abs() < 1e-12);
        }
        assert!(Cholesky::new(&[1.0, 2.0, 2.0, 1.0], 2).is_none());
        // 同じ行列を三重対角で
        let t = solve_tridiag(
            &[0.0, 2.0, 1.0],
            &[4.0, 5.0, 3.0],
            &[2.0, 1.0, 0.0],
            &[2.0, 8.0, 4.0],
        )
        .unwrap();
        for i in 0..3 {
            assert!((t[i] - x[i]).abs() < 1e-12);
        }
    }
}
