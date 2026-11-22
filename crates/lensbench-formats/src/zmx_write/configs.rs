//! .zmx の書き出しの多重構成の表の行（演算子・行の番号・書けない行の注意）

use lensbench_core::surface::SurfType;
use lensbench_core::system::System;

/// 多重構成の表の行の .zmx の演算子: (演算子, 番号 1, PARM の番号, 値の換算)。書けない行は None
pub(super) type ConfigOp = (&'static str, usize, Option<u8>, fn(f64) -> f64);
pub(super) fn config_op(s: &System, p: lensbench_core::param::Param) -> Option<ConfigOp> {
    use lensbench_core::param::Kind;
    let id: fn(f64) -> f64 = |v| v;
    let n = |i: u8| i as usize + 1;
    match p.kind {
        Kind::Wave(i) => return Some(("WAVE", n(i), None, id)),
        Kind::WaveW(i) => return Some(("WLWT", n(i), None, id)),
        Kind::FieldX(i) => return Some(("XFIE", n(i), None, id)),
        Kind::FieldY(i) => return Some(("YFIE", n(i), None, id)),
        Kind::FieldW(i) => return Some(("FLWT", n(i), None, id)),
        Kind::FieldVig(i, j) => {
            return Some((
                ["FVDX", "FVDY", "FVCX", "FVCY", "FTAN"][j as usize],
                n(i),
                None,
                id,
            ))
        }
        Kind::Primary => return Some(("PRWV", 0, None, id)),
        Kind::Aper => return Some(("APER", 0, None, id)),
        Kind::ApertureType => return Some(("SATP", 0, None, id)),
        Kind::Stop => return Some(("STPS", 0, None, id)),
        Kind::FieldType => return Some(("FLTP", 0, None, id)),
        Kind::Temp => return Some(("TEMP", 0, None, id)),
        Kind::Pres => return Some(("PRES", 0, None, id)),
        Kind::Afocal => return Some(("AFOC", 0, None, id)),
        Kind::ApodType => return Some(("APDT", 0, None, id)),
        Kind::Apod => return Some(("APDF", 0, None, id)),
        Kind::Aiming => return Some(("RAAM", 0, None, id)),
        Kind::Ropd => return Some(("OPDR", 0, None, id)),
        Kind::AimPupil(i) => {
            return Some((
                ["PSHX", "PSHY", "PSHZ", "PSCX", "PSCY"][(i as usize).min(4)],
                0,
                None,
                id,
            ))
        }
        _ => {}
    }
    let d = s.surfaces.get(p.surface()?)?;
    Some(match p.kind {
        Kind::Thick => ("THIC", p.index(), None, id),
        Kind::Semi => ("SDIA", p.index(), None, id),
        Kind::Ignore => ("IGNR", p.index(), None, id),
        Kind::ApMax => ("APMX", p.index(), None, id),
        Kind::ApMin => ("APMN", p.index(), None, id),
        Kind::ApDx => ("APDX", p.index(), None, id),
        Kind::ApDy => ("APDY", p.index(), None, id),
        Kind::ApType => ("APTP", p.index(), None, id),
        Kind::BeforeOrder => ("CBOR", p.index(), None, id),
        Kind::AfterOrder => ("CAOR", p.index(), None, id),
        Kind::ModelNd => ("MIND", p.index(), None, id),
        Kind::ModelVd => ("MABB", p.index(), None, id),
        Kind::ModelDpgf => ("MDPG", p.index(), None, id),
        Kind::Tce => ("TCEX", p.index(), None, id),
        Kind::MechSemi => ("MCSD", p.index(), None, id),
        Kind::ChipZone => ("CHZN", p.index(), None, id),
        Kind::CurvSolve(i) => (["CSP1", "CSP2"][i as usize], p.index(), None, id),
        Kind::ThickSolve(i) => (["TSP1", "TSP2", "TSP3"][i as usize], p.index(), None, id),
        Kind::ParmSolve(n, i) => (["PSP1", "PSP2", "PSP3"][i as usize], p.index(), Some(n), id),
        Kind::Before(i) => (
            ["CBDX", "CBDY", "CBTX", "CBTY", "CBTZ"][i as usize],
            p.index(),
            None,
            id,
        ),
        Kind::After(i) => (
            ["CADX", "CADY", "CATX", "CATY", "CATZ"][i as usize],
            p.index(),
            None,
            id,
        ),
        Kind::Curv => ("CRVT", p.index(), None, id),
        Kind::Radius => ("CRVT", p.index(), None, |r| {
            if r == 0.0 {
                0.0
            } else {
                1.0 / r
            }
        }),
        Kind::Conic => ("CONN", p.index(), None, id),
        Kind::Asph(i) if d.typ() == SurfType::EvenAsph => ("PRAM", p.index(), Some(i + 1), id),
        k if d.typ() == SurfType::CoordBrk => {
            let i = k.tilt_index()?;
            ("PRAM", p.index(), Some(i as u8 + 1), id)
        }
        _ => return None,
    })
}

/// 多重構成の表の .zmx に書く行の番号（1 始まり、パラメータごと）。書けない行は数えない
pub(super) fn config_row_numbers(
    s: &System,
    c: &lensbench_core::configs::Configs,
) -> Vec<(lensbench_core::param::Param, usize)> {
    let mut out = Vec::new();
    for r in &c.rows {
        if r.has_names() || config_op(s, r.param).is_some() {
            out.push((r.param, out.len() + 1));
        }
    }
    out
}

/// 構成のピックアップを .zmx に書けない行（材料・膜の名前の行、半径の行は曲率の行になるので倍率・オフセットが違う）
pub(super) fn config_pick_unwritable(r: &lensbench_core::configs::ConfigRow) -> bool {
    r.is_material() || r.is_coating() || r.param.kind == lensbench_core::param::Kind::Radius
}

/// 多重構成の表のうち .zmx に書けない行の注意
pub(super) fn config_notes(s: &System) -> Vec<String> {
    let Some(c) = &s.configs else {
        return Vec::new();
    };
    if s.units.iter().any(|u| u.moved()) {
        return vec![
            "the multi-configuration table of a system with moved lenses or groups can't be written to .zmx (the surface numbers change)".to_string(),
        ];
    }
    let mut out = Vec::new();
    for r in &c.rows {
        if config_op(s, r.param).is_none() {
            out.push(format!(
                "multi-configuration row {} can't be written to .zmx",
                r.param.name()
            ));
        }
        let nums = config_row_numbers(s, c);
        let own = nums.iter().find(|(x, _)| *x == r.param).map(|(_, n)| *n);
        let lost_row = r.picks.iter().any(|q| {
            q.row.is_some_and(|p| {
                let src = nums.iter().find(|(x, _)| *x == p).map(|(_, n)| *n);
                src.is_none() || src >= own || q.from > q.k
            })
        });
        if !r.picks.is_empty() && (config_pick_unwritable(r) || lost_row) {
            out.push(format!(
                "only the values of the configuration pickups of multi-configuration row {} were written",
                r.param.name()
            ));
        }
    }
    out
}
