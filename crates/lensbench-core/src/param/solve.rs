//! ソルブ・ピックアップの条件の値（多重構成の表の CSP・TSP・PSP の行、[`Kind::CurvSolve`]・[`Kind::ThickSolve`]・
//! [`Kind::ParmSolve`]）。値の番号は OpticStudio のソルブの窓の上からの順（OpticStudio 26.1 で確認）: 周辺光線の高さは
//! 高さ・瞳の帯、F ナンバーは F ナンバー、ピックアップは元の面・倍率・オフセット（曲率のピックアップは元の面・倍率）

use super::{Kind, Param};
use crate::error::Result;
use crate::invalid;
use crate::pickup::Pickup;
use crate::solves::{Solve, SolveKind};
use crate::system::{Prescription, System};

/// p が指すもの: 面のソルブか、面へのピックアップ
enum Target {
    Solve(usize),
    Pick(usize),
}

/// p の量（曲率・厚さ・面タイプのパラメータ）と値の番号
fn what(p: Param) -> (Kind, usize) {
    match p.kind {
        Kind::CurvSolve(i) => (Kind::Curv, i as usize),
        Kind::ThickSolve(i) => (Kind::Thick, i as usize),
        Kind::ParmSolve(n, i) => (Kind::Parm(n), i as usize),
        _ => unreachable!("not a solve parameter"),
    }
}

fn find(solves: &[Solve], pickups: &[Pickup], p: Param) -> Result<Target> {
    let j = p.index();
    let (w, _) = what(p);
    // 面タイプのパラメータのソルブはピックアップだけ
    let solve = (!matches!(w, Kind::Parm(_)))
        .then(|| {
            solves
                .iter()
                .position(|q| q.surf == j && q.kind.param_kind() == w)
        })
        .flatten();
    if let Some(i) = solve {
        return Ok(Target::Solve(i));
    }
    if let Some(i) = pickups.iter().position(|q| q.dst == j && q.what == w) {
        return Ok(Target::Pick(i));
    }
    invalid!(
        "{}: surface has no solve or pickup for this value",
        p.name()
    )
}

/// ソルブの条件の値（OpticStudio の番号の順）
fn solve_values(k: &SolveKind) -> Vec<f64> {
    use SolveKind as S;
    match *k {
        S::MarginalHeight { height } => vec![height, 0.0],
        S::MarginalHeightZone { height, zone } => vec![height, zone],
        S::ChiefHeight { height } => vec![height],
        S::Opd { opd, zone } => vec![opd, zone],
        S::Edge { thickness, radius } => vec![thickness, radius],
        S::Position { other, length } => vec![other as f64, length],
        S::Compensator { other, sum } => vec![other as f64, sum],
        S::CenterOfCurvature { other }
        | S::ConcentricSurface { other }
        | S::ConcentricRadius { other } => vec![other as f64],
        S::MarginalAngle { angle } | S::ChiefAngle { angle } => vec![angle],
        S::ElementPower { power } => vec![power],
        S::FNumber { fnum } => vec![fnum],
        _ => Vec::new(),
    }
}

/// 値の番号 i が面の番号か
fn is_surface(k: &SolveKind, i: usize) -> bool {
    i == 0 && k.other_surface().is_some()
}

/// ソルブ k の値 i を v にしたもの
fn with_value(k: &SolveKind, i: usize, v: f64) -> SolveKind {
    use SolveKind as S;
    let mut a = solve_values(k);
    a[i] = v;
    let o = a[0] as usize;
    match *k {
        S::MarginalHeight { .. } | S::MarginalHeightZone { .. } if a[1] == 0.0 => {
            S::MarginalHeight { height: a[0] }
        }
        S::MarginalHeight { .. } | S::MarginalHeightZone { .. } => S::MarginalHeightZone {
            height: a[0],
            zone: a[1],
        },
        S::ChiefHeight { .. } => S::ChiefHeight { height: a[0] },
        S::Opd { .. } => S::Opd {
            opd: a[0],
            zone: a[1],
        },
        S::Edge { .. } => S::Edge {
            thickness: a[0],
            radius: a[1],
        },
        S::Position { .. } => S::Position {
            other: o,
            length: a[1],
        },
        S::Compensator { .. } => S::Compensator {
            other: o,
            sum: a[1],
        },
        S::CenterOfCurvature { .. } => S::CenterOfCurvature { other: o },
        S::ConcentricSurface { .. } => S::ConcentricSurface { other: o },
        S::ConcentricRadius { .. } => S::ConcentricRadius { other: o },
        S::MarginalAngle { .. } => S::MarginalAngle { angle: a[0] },
        S::ChiefAngle { .. } => S::ChiefAngle { angle: a[0] },
        S::ElementPower { .. } => S::ElementPower { power: a[0] },
        S::FNumber { .. } => S::FNumber { fnum: a[0] },
        k => k,
    }
}

/// ピックアップの値（元の面・倍率・オフセット。曲率はオフセットを持たない）
fn pick_values(q: &Pickup) -> Vec<f64> {
    if q.what == Kind::Curv {
        vec![q.src as f64, q.scale]
    } else {
        vec![q.src as f64, q.scale, q.offset]
    }
}

/// p の今の値
pub(super) fn get(s: &System, p: Param) -> Result<f64> {
    solve_value(&s.solves, &s.pickups, p)
}

/// ソルブ solves・ピックアップ pickups の、p（[`Kind::CurvSolve`] など）の値。p が指すソルブ・ピックアップが無ければ誤り
pub fn solve_value(solves: &[Solve], pickups: &[Pickup], p: Param) -> Result<f64> {
    let (_, i) = what(p);
    let v = match find(solves, pickups, p)? {
        Target::Solve(k) => solve_values(&solves[k].kind),
        Target::Pick(k) => pick_values(&pickups[k]),
    };
    match v.get(i) {
        Some(&x) => Ok(x),
        None => invalid!("{}: the solve or pickup has no value {}", p.name(), i + 1),
    }
}

/// p を v にできるか（面の番号の値は今の面の番号の範囲の整数、ピックアップの元は別の面）
pub(super) fn check(s: &System, p: Param, v: f64) -> Result<()> {
    get(s, p)?;
    let (_, i) = what(p);
    let surf = match find(&s.solves, &s.pickups, p)? {
        Target::Solve(k) => is_surface(&s.solves[k].kind, i),
        Target::Pick(_) => i == 0,
    };
    let ns = s.surfaces.len() as f64;
    if surf && (v.fract() != 0.0 || v < 0.0 || v >= ns || v == p.index() as f64) {
        invalid!(
            "{}: surface {v} (another surface, 0 to {})",
            p.name(),
            ns - 1.0
        );
    }
    if !v.is_finite() {
        invalid!("{} = {v}", p.name());
    }
    Ok(())
}

/// p を v にする（[`check`] で確かめた値）。ソルブ・ピックアップが無ければ何もせず false
pub fn set_solve_value(rx: &mut Prescription, p: Param, v: f64) -> bool {
    let (_, i) = what(p);
    match find(&rx.solves, &rx.pickups, p) {
        Ok(Target::Solve(k)) if i < solve_values(&rx.solves[k].kind).len() => {
            rx.solves[k].kind = with_value(&rx.solves[k].kind, i, v);
        }
        Ok(Target::Pick(k)) if i < pick_values(&rx.pickups[k]).len() => {
            let q = &mut rx.pickups[k];
            match i {
                0 => q.src = v as usize,
                1 => q.scale = v,
                _ => q.offset = v,
            }
        }
        _ => return false,
    }
    true
}
