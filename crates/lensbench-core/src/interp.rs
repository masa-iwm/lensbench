//! 1 次元の補間（等間隔の Lagrange の重み、not-a-knot の 3 次スプライン）。解析・膜の表・偏光の格子はこれを使う。

/// 等間隔の節点 0, 1, …, K−1 の Lagrange 補間の、位置 u（節点の番号の単位）での重み
#[inline]
pub fn lagrange<const K: usize>(u: f64) -> [f64; K] {
    let d: [f64; K] = std::array::from_fn(|j| u - j as f64);
    // 左からの積と右からの積（Π_{j≠k}(u − j) を割り算なしで）
    let mut left = [1.0; K];
    let mut right = [1.0; K];
    for k in 1..K {
        left[k] = left[k - 1] * d[k - 1];
        right[K - 1 - k] = right[K - k] * d[K - k];
    }
    // 分母 Π_{j≠k}(k − j) = (−1)^(K−1−k) k! (K−1−k)!
    let fact = |n: usize| (1..=n).fold(1.0, |a, i| a * i as f64);
    std::array::from_fn(|k| {
        let den = fact(k) * fact(K - 1 - k);
        let den = if (K - 1 - k).is_multiple_of(2) {
            den
        } else {
            -den
        };
        left[k] * right[k] / den
    })
}

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
        let s = Self::not_a_knot_slopes(x, y)?;
        let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let sl: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
        let c = (0..n - 1)
            .map(|i| {
                let t = (s[i] + s[i + 1] - 2.0 * sl[i]) / (dx[i] * dx[i]);
                let q = (3.0 * sl[i] - 2.0 * s[i] - s[i + 1]) / dx[i];
                [t, q, s[i], y[i]]
            })
            .collect();
        Some(Spline { x: x.to_vec(), c })
    }

    /// not-a-knot の 3 次スプラインの節点の傾き（y について線形）。x が増えていない・点が 2 つ未満なら None
    pub fn not_a_knot_slopes(x: &[f64], y: &[f64]) -> Option<Vec<f64>> {
        let n = x.len();
        if n < 2 || y.len() != n || x.windows(2).any(|w| !(w[0] < w[1])) {
            return None;
        }
        let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let sl: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
        Some(if n == 2 {
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
            crate::linalg::solve_tridiag(&sub, &dia, &sup, &b)?
        })
    }

    pub fn eval(&self, v: f64) -> f64 {
        if v.is_nan() {
            return f64::NAN;
        }
        self.eval_at(self.locate(v), v)
    }

    /// v の区間の番号（[`Self::eval_at`] に渡す）
    pub(crate) fn locate(&self, v: f64) -> usize {
        let n = self.x.len();
        self.x[1..n - 1].partition_point(|&a| a <= v)
    }

    /// 区間 i（節が同じか、このスプラインの節がその前の部分である別のスプラインで [`Self::locate`] したもの）での値。
    /// 節が前の部分のときは最後の区間に丸める（区間を探し直したのと同じ）
    pub(crate) fn eval_at(&self, i: usize, v: f64) -> f64 {
        let i = i.min(self.c.len() - 1);
        let t = v - self.x[i];
        let [a, b, c, d] = self.c[i];
        ((a * t + b) * t + c) * t + d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lagrange_weights_reproduce_polynomials() {
        // K 点の重みは K−1 次までの多項式をそのまま再現する（節点の外でも）
        let p = |x: f64| 0.5 - 1.5 * x + 0.25 * x * x * x - 0.01 * x.powi(5);
        for &u in &[0.0, 0.3, 2.5, 4.99, -0.4] {
            let w = lagrange::<6>(u);
            let v: f64 = (0..6).map(|j| w[j] * p(j as f64)).sum();
            assert!((v - p(u)).abs() < 1e-12, "{u}: {v} {}", p(u));
            let w4 = lagrange::<4>(u);
            let c = |x: f64| 1.0 + x - 2.0 * x * x + 0.5 * x * x * x;
            let v4: f64 = (0..4).map(|j| w4[j] * c(j as f64)).sum();
            assert!((v4 - c(u)).abs() < 1e-12);
        }
        // 節点では 1 か 0
        assert_eq!(
            lagrange::<6>(2.0),
            [0.0, 0.0, 1.0, 0.0, 0.0, 0.0].map(|x: f64| x * 1.0)
        );
    }
}
