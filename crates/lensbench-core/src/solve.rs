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
use crate::system::{FieldCond, Launch, ObjSpec, RecSel, Trace};
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

impl Residual {
    /// 残差のもとになる光線の量（位置の目標は引く前）
    fn quantity(self) -> FieldCond {
        match self {
            Residual::Position { surf, .. } => FieldCond::Position { surf },
            Residual::DirSum { s1, s2 } => FieldCond::DirSum { s1, s2 },
            Residual::DirDiff { s1, s2 } => FieldCond::DirDiff { s1, s2 },
        }
    }
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
        let needs = match prob.residual {
            Residual::Position { surf, .. } => vec![surf],
            Residual::DirSum { s1, s2 } | Residual::DirDiff { s1, s2 } => vec![s1, s2],
        };
        for &j in &needs {
            if j == 0 || j > self.image() || self.surfaces[j].ignore {
                invalid!(
                    "residual surface {j} (surfaces not ignored, 1 to {})",
                    self.image()
                );
            }
        }
        if prob.unknown == Unknown::Object && !matches!(prob.object, ObjSpec::Point(_)) {
            invalid!("only a finite object can be an unknown");
        }
        let w = prob.wave;
        if w >= self.wavelengths.len() {
            invalid!("wavelength {w}");
        }
        let start = match (prob.unknown, prob.object) {
            (Unknown::Object, ObjSpec::Point(p)) => p,
            _ => prob.pupil,
        };
        let run = |u: [f64; 2], rec_all: bool| -> (Trace, [f64; 2]) {
            let (spec, pp) = match prob.unknown {
                Unknown::Object => (ObjSpec::Point(u), prob.pupil),
                Unknown::Pupil => (prob.object, u),
            };
            let (t, q) = self.ray_quantity(spec, pp, w, prob.residual.quantity(), rec_all);
            let r = match prob.residual {
                Residual::Position { target, .. } => [q[0] - target[0], q[1] - target[1]],
                _ => q,
            };
            (t, r)
        };
        let norm = |r: [f64; 2]| r[0].abs().max(r[1].abs());
        if !norm(run(start, false).1).is_finite() {
            invalid!("the starting ray doesn't pass ({:?})", prob.unknown);
        }
        let nw = crate::newton2::Newton2 {
            tol: prob.tol,
            max_it: prob.max_iter,
            diff: crate::newton2::Diff {
                h: 1e-6,
                rel: prob.unknown == Unknown::Object,
                central: true,
            },
            // 残差が減るまで歩幅を半分にする
            line: crate::newton2::Line::Decrease { min: 1e-4 },
        };
        let sol = nw.solve(|u| run(u, false).1, start, [0.0; 2]);
        let (u, r, it) = (sol.v, sol.r, sol.it);
        if sol.end == crate::newton2::End::Singular {
            return Err(Error::Invalid(format!(
                "ray solve: singular Jacobian (iteration {it}, residual {:e})",
                norm(r)
            )));
        }
        if !(norm(r) <= prob.tol) {
            invalid!(
                "the ray solve doesn't converge (iteration {it}, residual {:e}, tolerance {:e})",
                norm(r),
                prob.tol
            );
        }
        let t = run(u, true).0;
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

impl System {
    /// 物体 spec・瞳座標 pupil の光線（波長 w、ケラレ係数なし）の量 c: Position なら面 surf での位置 (x, y)（局所座標）、
    /// DirSum・DirDiff なら面 s1・s2 での方向余弦 (l, m) の和・差。止まった光線は NaN。
    /// 実像高・条件で定める視野（cond_spec）と [`System::solve_ray`] の残差はこれから作る
    pub(crate) fn ray_quantity(
        &self,
        spec: ObjSpec,
        pupil: [f64; 2],
        w: usize,
        c: FieldCond,
        rec_all: bool,
    ) -> (Trace, [f64; 2]) {
        let last = match c {
            FieldCond::Position { surf } => surf,
            FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => s1.max(s2),
        };
        // 記録は rec_all でなければ要る面だけ（面の多いシステムで反復ごとの記録の確保を減らす）
        let surfs = match c {
            FieldCond::Position { surf } => [surf, surf],
            FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => [s1, s2],
        };
        let l = Launch::Pupil {
            spec,
            signed: false,
            px: &[pupil[0]],
            py: &[pupil[1]],
            vig: Some([0.0; 5]),
            hxy: Some((0.0, 0.0)),
        };
        let rec = if rec_all {
            RecSel::All
        } else {
            RecSel::Surfs(&surfs)
        };
        let mut t = self.trace_launch(&l, w, Some(last), rec);
        t.hx_hy = (0.0, 0.0);
        if t.rays[0].status.surf().is_some() {
            return (t, [f64::NAN; 2]);
        }
        let q = match c {
            FieldCond::Position { surf } => t.rec_at(surf).map(|a| [a.xyz[0][0], a.xyz[0][1]]),
            FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => {
                t.rec_at(s1).zip(t.rec_at(s2)).map(|(a, b)| {
                    let (a, b) = (a.lmn[0], b.lmn[0]);
                    if matches!(c, FieldCond::DirSum { .. }) {
                        [a[0] + b[0], a[1] + b[1]]
                    } else {
                        [a[0] - b[0], a[1] - b[1]]
                    }
                })
            }
        };
        (t, q.unwrap_or([f64::NAN; 2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 途中の面で止まった光線の量は NaN（収束の判定・ヤコビアンに有限の値として混ざらない）
    #[test]
    fn ray_quantity_of_a_stopped_ray_is_nan() {
        // 半径 5 mm の凸面（入射瞳径 10 mm）。瞳座標 3 の光線は高さ 15 mm で面に当たらない
        let txt =
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
                   XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
                   SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.2\n  DISZ 10\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::test_zmx::parse_text(txt).unwrap();
        let s = crate::test_zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let spec = ObjSpec::Dir([0.0, 0.0, 1.0]);
        for c in [
            FieldCond::Position { surf: 2 },
            FieldCond::DirSum { s1: 1, s2: 2 },
            FieldCond::DirDiff { s1: 1, s2: 2 },
        ] {
            let (t, q) = s.ray_quantity(spec, [3.0, 0.0], 0, c, false);
            assert!(t.rays[0].status.surf().is_some(), "{c:?}");
            assert!(q[0].is_nan() && q[1].is_nan(), "{c:?}: {q:?}");
            // 届く光線は有限
            let (_, q) = s.ray_quantity(spec, [0.3, 0.0], 0, c, false);
            assert!(q[0].is_finite() && q[1].is_finite(), "{c:?}: {q:?}");
        }
    }
}
