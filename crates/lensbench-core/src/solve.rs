//! 物点や瞳座標を未知数にした実光線の解き。既存の瞳のエイミングとは別。
//!
//! 未知数 2 つ（物点の x・y、または瞳座標 px・py）を、残差 2 つが 0 になるよう Newton 法で解く。残差は次から選ぶ:
//! - 面 s での光線の位置 x・y（局所座標）と目標の差
//! - 面 s1 と s2 での方向余弦 l・m（局所座標、面の後の向き）の和、または差
//!
//! 例: ダブルパスで主光線を面 A と面 B で逆向きにする（法線入射で戻る）物点は、未知数を物点、残差を A・B の
//! 方向余弦の和にして解く。戻り光が面 s の点 (x, y) を通る物点は、残差を面 s の位置にして解く。
//!
//! 光線は系の設定どおりに追跡する（瞳座標は系の光線エイミングを通す。ケラレ係数はかけない）。
//! ヤコビアンは中心差分で求める。

use crate::error::{Error, Result};
use crate::invalid;
use crate::system::{ObjSpec, RecSurf, Trace};
use crate::System;

/// 未知数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unknown {
    /// 物点の x・y [mm]（物体面の局所座標。有限物体）
    Object,
    /// 瞳座標 px・py（正規化）
    Pupil,
}

/// 残差（2 つ）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Residual {
    /// 面 surf での光線の位置 (x, y)（局所座標）− target
    Position { surf: usize, target: [f64; 2] },
    /// 面 s1 と s2 での方向余弦 (l, m) の和
    DirSum { s1: usize, s2: usize },
    /// 面 s1 と s2 での方向余弦 (l, m) の差（s1 − s2）
    DirDiff { s1: usize, s2: usize },
}

/// 解く問題
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayProblem {
    pub unknown: Unknown,
    pub residual: Residual,
    /// 物点 [mm]（有限物体）か、無限遠物体の方向（物体空間の単位ベクトル）。未知数が物点なら初期値
    pub object: ObjSpec,
    /// 瞳座標。未知数が瞳座標なら初期値
    pub pupil: [f64; 2],
    /// 波長の番号
    pub wave: usize,
    /// 残差の許容（位置は mm、方向余弦はそのまま）
    pub tol: f64,
    pub max_iter: usize,
}

impl RayProblem {
    pub fn new(unknown: Unknown, residual: Residual, object: ObjSpec, wave: usize) -> RayProblem {
        RayProblem {
            unknown,
            residual,
            object,
            pupil: [0.0, 0.0],
            wave,
            tol: 1e-12,
            max_iter: 50,
        }
    }
}

/// 解いた結果
#[derive(Debug, Clone)]
pub struct RaySolution {
    pub object: ObjSpec,
    pub pupil: [f64; 2],
    pub residual: [f64; 2],
    pub iterations: usize,
    /// 解いた光線の追跡（面ごとの記録つき、1 本）
    pub trace: Trace,
}

impl System {
    /// 未知数 2 つ・残差 2 つの実光線を Newton 法で解く
    pub fn solve_ray(&self, prob: &RayProblem) -> Result<RaySolution> {
        let (needs, last) = match prob.residual {
            Residual::Position { surf, .. } => (vec![surf], surf),
            Residual::DirSum { s1, s2 } | Residual::DirDiff { s1, s2 } => {
                (vec![s1, s2], s1.max(s2))
            }
        };
        for &j in &needs {
            if j == 0 || j > self.image() || self.surfaces[j].ignore {
                invalid!("残差の面 {j}（無視しない面 1〜{}）", self.image());
            }
        }
        if prob.unknown == Unknown::Object && !matches!(prob.object, ObjSpec::Point(_)) {
            invalid!("物点を未知数にするのは有限物体だけ");
        }
        let w = prob.wave;
        if w >= self.wavelengths.len() {
            invalid!("波長 {w}");
        }
        let start = match (prob.unknown, prob.object) {
            (Unknown::Object, ObjSpec::Point(p)) => p,
            _ => prob.pupil,
        };
        let run = |u: [f64; 2]| -> (Trace, [f64; 2]) {
            let (spec, pp) = match prob.unknown {
                Unknown::Object => (ObjSpec::Point(u), prob.pupil),
                Unknown::Pupil => (prob.object, u),
            };
            let t = self.trace_spec(
                spec,
                &[pp[0]],
                &[pp[1]],
                w,
                crate::TraceOpts::new()
                    .vig([0.0; 5])
                    .upto(last)
                    .record()
                    .hxy((0.0, 0.0)),
            );
            let at = |j: usize| -> &RecSurf { t.rec_at(j).unwrap() };
            let bad = t.rays[0].err != 0;
            let r = match prob.residual {
                Residual::Position { surf, target } => {
                    let x = at(surf).xyz[0];
                    [x[0] - target[0], x[1] - target[1]]
                }
                Residual::DirSum { s1, s2 } => {
                    let (a, b) = (at(s1).lmn[0], at(s2).lmn[0]);
                    [a[0] + b[0], a[1] + b[1]]
                }
                Residual::DirDiff { s1, s2 } => {
                    let (a, b) = (at(s1).lmn[0], at(s2).lmn[0]);
                    [a[0] - b[0], a[1] - b[1]]
                }
            };
            (t, if bad { [f64::NAN; 2] } else { r })
        };
        let norm = |r: [f64; 2]| r[0].abs().max(r[1].abs());
        let mut u = start;
        let (mut t, mut r) = run(u);
        if !norm(r).is_finite() {
            invalid!("初期値の光線が通らない（{:?}）", prob.unknown);
        }
        let mut it = 0;
        while it < prob.max_iter && !(norm(r) <= prob.tol) {
            it += 1;
            let h = match prob.unknown {
                Unknown::Object => 1e-6 * u[0].abs().max(u[1].abs()).max(1.0),
                Unknown::Pupil => 1e-6,
            };
            let mut jac = [[0.0; 2]; 2];
            for c in 0..2 {
                let mut a = u;
                let mut b = u;
                a[c] += h;
                b[c] -= h;
                let (ra, rb) = (run(a).1, run(b).1);
                jac[0][c] = (ra[0] - rb[0]) / (2.0 * h);
                jac[1][c] = (ra[1] - rb[1]) / (2.0 * h);
            }
            let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
            let du = [
                (jac[1][1] * r[0] - jac[0][1] * r[1]) / det,
                (-jac[1][0] * r[0] + jac[0][0] * r[1]) / det,
            ];
            if !(du[0].is_finite() && du[1].is_finite()) {
                return Err(Error::Invalid(format!(
                    "光線の解き: ヤコビアンが特異（反復 {it}、残差 {:e}）",
                    norm(r)
                )));
            }
            // 残差が減るまで歩幅を半分にする
            let mut step = 1.0;
            loop {
                let v = [u[0] - step * du[0], u[1] - step * du[1]];
                let (t2, r2) = run(v);
                if norm(r2) < norm(r) || step < 1e-4 {
                    if norm(r2).is_finite() {
                        u = v;
                        t = t2;
                        r = r2;
                    }
                    break;
                }
                step *= 0.5;
            }
            if step < 1e-4 {
                break;
            }
        }
        if !(norm(r) <= prob.tol) {
            invalid!(
                "光線の解きが収束しない（反復 {it}、残差 {:e}、許容 {:e}）",
                norm(r),
                prob.tol
            );
        }
        let (object, pupil) = match prob.unknown {
            Unknown::Object => (ObjSpec::Point(u), prob.pupil),
            Unknown::Pupil => (prob.object, u),
        };
        Ok(RaySolution {
            object,
            pupil,
            residual: r,
            iterations: it,
            trace: t,
        })
    }
}
