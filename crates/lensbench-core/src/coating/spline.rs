//! 膜の表・モデルの補間に使う 3 次スプライン

/// 3 次スプライン（両端 not-a-knot、範囲外は端の多項式で外挿。scipy の CubicSpline と同じ）
#[derive(Debug, Clone)]
pub struct Spline {
    x: Vec<f64>,
    /// 区間ごとの係数（t = x − x_i の 3, 2, 1, 0 次）
    c: Vec<[f64; 4]>,
}

impl Spline {
    pub fn not_a_knot(x: &[f64], y: &[f64]) -> Option<Spline> {
        let n = x.len();
        if n < 2 || y.len() != n || x.windows(2).any(|w| !(w[0] < w[1])) {
            return None;
        }
        let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let sl: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
        // 各点の傾き s
        let s: Vec<f64> = if n == 2 {
            vec![sl[0], sl[0]]
        } else if n == 3 {
            // 放物線
            let a = (sl[1] - sl[0]) / (x[2] - x[0]);
            (0..3)
                .map(|i| sl[0] + a * (2.0 * x[i] - x[0] - x[1]))
                .collect()
        } else {
            // 三重対角の連立（行 0 と n−1 が not-a-knot）
            let mut sub = vec![0.0; n];
            let mut dia = vec![0.0; n];
            let mut sup = vec![0.0; n];
            let mut b = vec![0.0; n];
            for i in 1..n - 1 {
                sub[i] = dx[i];
                dia[i] = 2.0 * (dx[i - 1] + dx[i]);
                sup[i] = dx[i - 1];
                b[i] = 3.0 * (dx[i] * sl[i - 1] + dx[i - 1] * sl[i]);
            }
            let d0 = x[2] - x[0];
            dia[0] = dx[1];
            sup[0] = d0;
            b[0] = ((dx[0] + 2.0 * d0) * dx[1] * sl[0] + dx[0] * dx[0] * sl[1]) / d0;
            let d1 = x[n - 1] - x[n - 3];
            dia[n - 1] = dx[n - 3];
            sub[n - 1] = d1;
            b[n - 1] = (dx[n - 2] * dx[n - 2] * sl[n - 3]
                + (2.0 * d1 + dx[n - 2]) * dx[n - 3] * sl[n - 2])
                / d1;
            solve_tridiag(&sub, &dia, &sup, &b)?
        };
        let c = (0..n - 1)
            .map(|i| {
                let t = (s[i] + s[i + 1] - 2.0 * sl[i]) / (dx[i] * dx[i]);
                let q = (3.0 * sl[i] - 2.0 * s[i] - s[i + 1]) / dx[i];
                [t, q, s[i], y[i]]
            })
            .collect();
        Some(Spline { x: x.to_vec(), c })
    }

    pub fn eval(&self, v: f64) -> f64 {
        if v.is_nan() {
            return f64::NAN;
        }
        self.eval_at(self.locate(v), v)
    }

    /// v の区間の番号（[`Self::eval_at`] に渡す）
    pub(super) fn locate(&self, v: f64) -> usize {
        let n = self.x.len();
        self.x[1..n - 1].partition_point(|&a| a <= v)
    }

    /// 区間 i（節が同じか、このスプラインの節がその前の部分である別のスプラインで [`Self::locate`] したもの）での値。
    /// 節が前の部分のときは最後の区間に丸める（区間を探し直したのと同じ）
    pub(super) fn eval_at(&self, i: usize, v: f64) -> f64 {
        let i = i.min(self.c.len() - 1);
        let t = v - self.x[i];
        let [a, b, c, d] = self.c[i];
        ((a * t + b) * t + c) * t + d
    }
}

/// 三重対角の連立方程式（部分ピボット付きの消去。行 0 と n−1 の対角が小さくてもよいように）
fn solve_tridiag(sub: &[f64], dia: &[f64], sup: &[f64], b: &[f64]) -> Option<Vec<f64>> {
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
