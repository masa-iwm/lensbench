//! 小さな線形代数。

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

/// 対称正定値の n×n を Cholesky で解く（正定値でなければ None）
pub fn cholesky_solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<Vec<f64>> {
    for c in 0..n {
        let mut d = a[c * n + c];
        for k in 0..c {
            d -= a[c * n + k] * a[c * n + k];
        }
        if !(d > 0.0) {
            return None;
        }
        let d = d.sqrt();
        a[c * n + c] = d;
        for rr in c + 1..n {
            let mut v = a[rr * n + c];
            for k in 0..c {
                v -= a[rr * n + k] * a[c * n + k];
            }
            a[rr * n + c] = v / d;
        }
    }
    let mut y = b.to_vec();
    for i in 0..n {
        for k in 0..i {
            y[i] -= a[i * n + k] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    for i in (0..n).rev() {
        for k in i + 1..n {
            y[i] -= a[k * n + i] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    Some(y)
}
