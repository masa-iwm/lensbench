//! .zmx の多重構成を多重構成の表にする

use super::*;

/// .zmx の多重構成（MCE）を多重構成の表（[`lensbench_core::configs::Configs`]）にする。表せる演算子は THIC・CRVT・CONN と、
/// 偶数次非球面・座標ブレークの PRAM。表せない演算子が構成で変われば、その演算子の名前を Err で返す（構成で変わらない
/// 演算子・光線追跡に効かない演算子は表に入れない）
pub(super) fn config_table(
    raw: &Raw,
    s: &System,
    n: usize,
    active: usize,
) -> lensbench_core::Result<lensbench_core::configs::Configs> {
    use lensbench_core::configs::{ConfigRow, Configs};
    use lensbench_core::param::{Kind, Param};
    let mut rows: Vec<(String, i64, i64, Vec<Option<f64>>)> = Vec::new();
    for (op, p1, c, v, rest) in &raw.mce {
        let parm = if op == "PRAM" {
            rest.get(1)
                .and_then(|x| x.parse::<i64>().ok())
                .unwrap_or(-1)
        } else {
            0
        };
        let i = match rows
            .iter()
            .position(|r| r.0 == *op && r.1 == *p1 && r.2 == parm)
        {
            Some(i) => i,
            None => {
                rows.push((op.clone(), *p1, parm, vec![None; n]));
                rows.len() - 1
            }
        };
        if (1..=n as i64).contains(c) {
            rows[i].3[*c as usize - 1] = fv(v).ok();
        }
    }
    let mut out = Vec::new();
    for (op, p1, parm, vals) in rows {
        let first = vals.first().copied().flatten();
        let varies = vals.iter().any(|v| *v != first);
        let surf = usize::try_from(p1).ok().filter(|&k| k < s.surfaces.len());
        let kind = match (op.as_str(), surf) {
            ("THIC", Some(_)) => Some(Kind::Thick),
            ("CRVT", Some(_)) => Some(Kind::Curv),
            ("CONN", Some(_)) => Some(Kind::Conic),
            // 面タイプのパラメータの番号からパラメータの種類（係数・座標ブレークの値）はピックアップと同じ表で
            ("PRAM", Some(k)) => u8::try_from(parm).ok().and_then(|i| {
                lensbench_core::pickup::kind_of_pick(
                    &s.surfaces[k],
                    lensbench_core::param::Kind::Parm(i),
                )
            }),
            _ => None,
        };
        match kind {
            Some(kind) => {
                if !varies {
                    continue;
                }
                let p = Param::new(surf.expect("surface number checked above"), kind);
                // 書いていない構成の値は今の系の値
                let now = s.get_param(p)?;
                out.push(ConfigRow {
                    param: p,
                    values: vals.iter().map(|v| v.unwrap_or(now)).collect(),
                    picks: Vec::new(),
                });
            }
            None if varies && !matches!(op.as_str(), "MCOM" | "CWGT" | "MOFF" | "LTTL") => {
                return Err(format!("{op} {p1}").into());
            }
            None => {}
        }
    }
    Ok(Configs {
        n,
        active,
        rows: out,
    })
}
