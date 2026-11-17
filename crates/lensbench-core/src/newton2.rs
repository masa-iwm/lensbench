//! 未知数 2 つ・残差 2 つのニュートン法（光線の狙い・視野の条件・実光線の解き）。
//!
//! ヤコビアンは残差の差分（前進か中心）で、刻みは決まった値か未知数の大きさに比べた値。残差が有限でなくなった・
//! ヤコビアンが解けない・歩幅を縮めても進めないときは止め、その理由を返す。N 本の光線を同じ手順でまとめて解ける
//! （残差の関数は N 本を 1 度に求める。光線ごとの手順・結果は 1 本ずつと同じビット）。

use crate::math::solve2;

/// 差分の刻み
#[derive(Debug, Clone, Copy)]
pub(crate) struct Diff {
    /// 刻み（rel なら max(|v₀|, |v₁|, 1) 倍）
    pub h: f64,
    pub rel: bool,
    /// 中心差分（でなければ前進差分）
    pub central: bool,
}

impl Diff {
    pub(crate) const fn fwd(h: f64) -> Diff {
        Diff {
            h,
            rel: false,
            central: false,
        }
    }
}

/// 歩幅の縮め方
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Line {
    /// 縮めない
    Full,
    /// 残差が有限になるまで半分に（min より小さくなったら止める）
    Finite { min: f64 },
    /// 残差が減るまで半分に（min より小さくなったら、有限ならその点を採って止める）
    Decrease { min: f64 },
}

/// 解き方
#[derive(Debug, Clone, Copy)]
pub(crate) struct Newton2 {
    /// 収束: max(|r₀|, |r₁|) <= tol（光線ごとに [`Newton2::solve_lanes`] の tol で変えられる）
    pub tol: f64,
    pub max_it: usize,
    pub diff: Diff,
    pub line: Line,
}

/// 止まった理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Converged,
    /// 反復の上限まで回った
    MaxIter,
    /// 残差が有限でない（光線が届かない）
    NotFinite,
    /// ヤコビアンが解けない
    Singular,
    /// 歩幅を縮めても進めない
    Stalled,
}

/// 解（v は最後の点、r はその残差、it は反復の回数）
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sol {
    pub v: [f64; 2],
    pub r: [f64; 2],
    pub it: usize,
    pub end: End,
}

impl Sol {
    pub(crate) fn ok(&self) -> bool {
        self.end == End::Converged
    }
    /// 残差の大きさ（有限でなければ無限大）
    pub(crate) fn err(&self) -> f64 {
        let e = self.r[0].abs().max(self.r[1].abs());
        if e.is_finite() {
            e
        } else {
            f64::INFINITY
        }
    }
}

#[inline]
fn norm(r: [f64; 2]) -> f64 {
    r[0].abs().max(r[1].abs())
}

#[inline]
fn finite(r: [f64; 2]) -> bool {
    r[0].is_finite() && r[1].is_finite()
}

impl Newton2 {
    /// 1 本を解く: f(v) = tgt（残差は f(v) − tgt、ヤコビアンは f の差分）
    pub(crate) fn solve(
        &self,
        mut f: impl FnMut([f64; 2]) -> [f64; 2],
        v0: [f64; 2],
        tgt: [f64; 2],
    ) -> Sol {
        let [s] = self.solve_lanes(|v: &[[f64; 2]; 1]| [f(v[0])], [v0], [tgt], [self.tol]);
        s
    }

    /// N 本を同じ手順でまとめて解く（f は N 本の値を 1 度に求める。止まった光線の値も渡すが、結果は使わない）。
    /// tgt・tol は光線ごとの目標と収束の幅
    pub(crate) fn solve_lanes<const N: usize>(
        &self,
        mut f: impl FnMut(&[[f64; 2]; N]) -> [[f64; 2]; N],
        v0: [[f64; 2]; N],
        tgt: [[f64; 2]; N],
        tol: [f64; N],
    ) -> [Sol; N] {
        let res = |q: [[f64; 2]; N]| -> [[f64; 2]; N] {
            std::array::from_fn(|l| [q[l][0] - tgt[l][0], q[l][1] - tgt[l][1]])
        };
        let mut v = v0;
        // q は f の値、r は残差
        let mut q = f(&v);
        let mut r = res(q);
        let mut it = [0usize; N];
        let mut end: [Option<End>; N] = [None; N];
        for _ in 0..self.max_it {
            for l in 0..N {
                if end[l].is_some() {
                    continue;
                }
                if !finite(r[l]) {
                    end[l] = Some(End::NotFinite);
                } else if norm(r[l]) <= tol[l] {
                    end[l] = Some(End::Converged);
                }
            }
            if end.iter().all(|e| e.is_some()) {
                break;
            }
            // ヤコビアン（行優先 [∂r₀/∂v₀ ∂r₀/∂v₁; ∂r₁/∂v₀ ∂r₁/∂v₁]）
            let h: [f64; N] = std::array::from_fn(|l| {
                if self.diff.rel {
                    self.diff.h * v[l][0].abs().max(v[l][1].abs()).max(1.0)
                } else {
                    self.diff.h
                }
            });
            let shifted = |c: usize, s: f64| -> [[f64; 2]; N] {
                std::array::from_fn(|l| {
                    let mut a = v[l];
                    if s > 0.0 {
                        a[c] += h[l];
                    } else {
                        a[c] -= h[l];
                    }
                    a
                })
            };
            let jac: [[f64; 4]; N] = if self.diff.central {
                let (xa, xb) = (f(&shifted(0, 1.0)), f(&shifted(0, -1.0)));
                let (ya, yb) = (f(&shifted(1, 1.0)), f(&shifted(1, -1.0)));
                std::array::from_fn(|l| {
                    let d = 2.0 * h[l];
                    [
                        (xa[l][0] - xb[l][0]) / d,
                        (ya[l][0] - yb[l][0]) / d,
                        (xa[l][1] - xb[l][1]) / d,
                        (ya[l][1] - yb[l][1]) / d,
                    ]
                })
            } else {
                let (qx, qy) = (f(&shifted(0, 1.0)), f(&shifted(1, 1.0)));
                std::array::from_fn(|l| {
                    let q = q[l];
                    [
                        (qx[l][0] - q[0]) / h[l],
                        (qy[l][0] - q[0]) / h[l],
                        (qx[l][1] - q[1]) / h[l],
                        (qy[l][1] - q[1]) / h[l],
                    ]
                })
            };
            let mut dv = [[0.0; 2]; N];
            for l in 0..N {
                if end[l].is_some() {
                    continue;
                }
                it[l] += 1;
                dv[l] = solve2(jac[l], r[l]);
                if !finite(dv[l]) {
                    end[l] = Some(End::Singular);
                }
            }
            // 歩幅 t を光線ごとに縮めながら次の点を探す（決まった光線は done）
            let mut t = [1.0f64; N];
            let mut done: [bool; N] = std::array::from_fn(|l| end[l].is_some());
            while !done.iter().all(|&d| d) {
                let vn: [[f64; 2]; N] =
                    std::array::from_fn(|l| [v[l][0] - t[l] * dv[l][0], v[l][1] - t[l] * dv[l][1]]);
                let qn = f(&vn);
                let rn = res(qn);
                for l in 0..N {
                    if done[l] {
                        continue;
                    }
                    let accept = match self.line {
                        Line::Full => true,
                        Line::Finite { .. } => finite(rn[l]),
                        Line::Decrease { min } => norm(rn[l]) < norm(r[l]) || t[l] < min,
                    };
                    if accept {
                        done[l] = true;
                        match self.line {
                            Line::Decrease { min } => {
                                if finite(rn[l]) {
                                    v[l] = vn[l];
                                    q[l] = qn[l];
                                    r[l] = rn[l];
                                }
                                if t[l] < min {
                                    end[l] = Some(End::Stalled);
                                }
                            }
                            _ => {
                                v[l] = vn[l];
                                q[l] = qn[l];
                                r[l] = rn[l];
                            }
                        }
                        continue;
                    }
                    t[l] *= 0.5;
                    if let Line::Finite { min } = self.line {
                        if t[l] < min {
                            done[l] = true;
                            end[l] = Some(End::Stalled);
                        }
                    }
                }
            }
        }
        std::array::from_fn(|l| {
            let e = end[l].unwrap_or_else(|| {
                if !finite(r[l]) {
                    End::NotFinite
                } else if norm(r[l]) <= tol[l] {
                    End::Converged
                } else {
                    End::MaxIter
                }
            });
            Sol {
                v: v[l],
                r: r[l],
                it: it[l],
                end: e,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_smooth_system_and_reports_why_it_stops() {
        let n = Newton2 {
            tol: 1e-13,
            max_it: 40,
            diff: Diff::fwd(1e-7),
            line: Line::Full,
        };
        // (x² − 2, x y − 1) の根 (√2, 1/√2)
        let f = |v: [f64; 2]| [v[0] * v[0] - 2.0, v[0] * v[1] - 1.0];
        let s = n.solve(f, [1.0, 1.0], [0.0; 2]);
        assert!(s.ok(), "{s:?}");
        assert!((s.v[0] - 2f64.sqrt()).abs() < 1e-12);
        assert!((s.v[1] - 0.5f64.sqrt()).abs() < 1e-12);
        // 残差が有限でない
        let s = n.solve(|_| [f64::NAN, 0.0], [0.0, 0.0], [0.0; 2]);
        assert_eq!(s.end, End::NotFinite);
        // ヤコビアンが解けない（残差が v によらない）
        let s = n.solve(|_| [1.0, 1.0], [0.0, 0.0], [0.0; 2]);
        assert_eq!(s.end, End::Singular);
        // まとめて解いても 1 本ずつと同じビット
        let v0 = [[1.0, 1.0], [2.0, 0.3], [0.5, 3.0]];
        let fl = |v: &[[f64; 2]; 3]| v.map(f);
        let all = n.solve_lanes(fl, v0, [[0.0; 2]; 3], [n.tol; 3]);
        for l in 0..3 {
            let one = n.solve(f, v0[l], [0.0; 2]);
            assert_eq!(all[l].v.map(f64::to_bits), one.v.map(f64::to_bits));
            assert_eq!(all[l].it, one.it);
        }
    }

    #[test]
    fn line_search_halves_until_the_residual_is_finite() {
        let n = Newton2 {
            tol: 1e-12,
            max_it: 60,
            diff: Diff::fwd(1e-7),
            line: Line::Finite { min: 1e-6 },
        };
        // x > 3 で光線が届かない（NaN）。根は x = 2、y = 1
        let f = |v: [f64; 2]| {
            if v[0] > 3.0 {
                [f64::NAN; 2]
            } else {
                [(v[0] - 2.0).atan(), v[1] - 1.0]
            }
        };
        // 初めの一歩は x = 3.69（届かない）なので半分に縮める
        let s = n.solve(f, [0.5, 0.0], [0.0; 2]);
        assert!(s.ok(), "{s:?}");
        assert!((s.v[0] - 2.0).abs() < 1e-12);
    }
}
