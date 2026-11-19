//! .zmx の多重構成を多重構成の表にする

use super::*;

/// .zmx の多重構成（MCE）を多重構成の表（[`lensbench_core::configs::Configs`]）にする。表せる演算子は THIC・CRVT・CONN と、
/// 偶数次非球面・座標ブレークの PRAM、材料の GLSS、系の値の WAVE・WLWT・PRWV・APER・XFIE・YFIE・FLWT。表せない演算子が構成で変われば、その演算子の名前を Err で返す（構成で変わらない
/// 演算子・光線追跡に効かない演算子は表に入れない）
pub(super) fn config_table(
    raw: &Raw,
    s: &System,
    n: usize,
    active: usize,
) -> lensbench_core::Result<lensbench_core::configs::Configs> {
    use lensbench_core::configs::{ConfigRow, Configs};
    use lensbench_core::param::{Kind, Param};
    let mut rows: Vec<(String, i64, i64, Vec<Option<String>>)> = Vec::new();
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
            rows[i].3[*c as usize - 1] = Some(v.clone());
        }
    }
    let mut out = Vec::new();
    for (op, p1, parm, raw_vals) in rows {
        let vals: Vec<Option<f64>> = raw_vals
            .iter()
            .map(|v| v.as_deref().and_then(|x| fv(x).ok()))
            // 経緯儀の角度（FLTP 4）は x・y の角度で入っているので角度（0）として読む
            .map(|v| v.map(|x| if op == "FLTP" && x == 4.0 { 0.0 } else { x }))
            .collect();
        // 変わるかは、数なら数で（1 と 1.0 は同じ）、数でなければ書いた文字で見る（GLSS などの名前の値）
        let same =
            |a: &(Option<String>, Option<f64>), b: &(Option<String>, Option<f64>)| match (a.1, b.1)
            {
                (Some(x), Some(y)) => x == y,
                _ => a.0 == b.0,
            };
        let both: Vec<(Option<String>, Option<f64>)> =
            raw_vals.iter().cloned().zip(vals.iter().copied()).collect();
        let varies = both.iter().any(|v| !same(v, &both[0]));
        let surf = usize::try_from(p1).ok().filter(|&k| k < s.surfaces.len());
        if let ("GLSS", Some(j), true) = (op.as_str(), surf, varies) {
            // 材料の行: 書いていない構成は今の材料。材料をピックアップする面（GLSS の無い面）も同じ行を持つ
            let now = s.surfaces[j].mat.name();
            let names: Vec<String> = raw_vals
                .iter()
                .map(|v| {
                    v.as_deref()
                        .map_or(now.clone(), |x| x.trim_matches('"').to_string())
                })
                .collect();
            for k in std::iter::once(j).chain(material_pickers(raw, j)) {
                let mut row = ConfigRow {
                    param: Param::new(k, Kind::Material),
                    values: Vec::new(),
                    names: Vec::new(),
                    picks: Vec::new(),
                };
                for n in &names {
                    let i = match row.names.iter().position(|x| x == n) {
                        Some(i) => i,
                        None => {
                            row.names.push(n.clone());
                            row.names.len() - 1
                        }
                    };
                    row.values.push(i as f64);
                }
                out.push(row);
            }
            continue;
        }
        let kind = match (op.as_str(), surf) {
            ("THIC", Some(_)) => Some(Kind::Thick),
            ("CRVT", Some(_)) => Some(Kind::Curv),
            ("CONN", Some(_)) => Some(Kind::Conic),
            ("SDIA", Some(_)) => Some(Kind::Semi),
            ("IGNR", Some(_)) => Some(Kind::Ignore),
            ("APMX", Some(_)) => Some(Kind::ApMax),
            // 面タイプのパラメータの番号からパラメータの種類（係数・座標ブレークの値）はピックアップと同じ表で
            ("PRAM", Some(k)) => u8::try_from(parm).ok().and_then(|i| {
                lensbench_core::pickup::kind_of_pick(
                    &s.surfaces[k],
                    lensbench_core::param::Kind::Parm(i),
                )
            }),
            _ => system_kind(&op, p1),
        };
        match kind {
            Some(kind) => {
                if !varies {
                    continue;
                }
                let p = if kind.is_global() {
                    Param::system(kind)
                } else {
                    Param::new(surf.expect("surface number checked above"), kind)
                };
                // 書いていない構成の値は今の系の値
                let now = s.get_param(p)?;
                out.push(ConfigRow {
                    param: p,
                    values: vals.iter().map(|v| v.unwrap_or(now)).collect(),
                    names: Vec::new(),
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

/// 面 j の材料をピックアップする面（連なっていても。途中に多重構成の GLSS がある面は除く）
fn material_pickers(raw: &Raw, j: usize) -> Vec<usize> {
    let glss = |k: usize| {
        raw.mce
            .iter()
            .any(|(op, p1, ..)| op == "GLSS" && *p1 == k as i64)
    };
    (0..raw.surfs.len())
        .filter(|&k| k != j && !glss(k))
        .filter(|&k| {
            let mut i = k;
            for _ in 0..raw.surfs.len() {
                match raw.surfs[i].glas_pickup {
                    Some(src) if src == j => return true,
                    Some(src) if src < raw.surfs.len() && !glss(src) => i = src,
                    _ => return false,
                }
            }
            false
        })
        .collect()
}

/// 系の値の演算子のパラメータの種類（番号 1 は波長・視野の番号、1 から）
fn system_kind(op: &str, p1: i64) -> Option<lensbench_core::param::Kind> {
    use lensbench_core::param::Kind;
    let i = u8::try_from(p1 - 1).ok();
    Some(match op {
        "WAVE" => Kind::Wave(i?),
        "WLWT" => Kind::WaveW(i?),
        "XFIE" => Kind::FieldX(i?),
        "YFIE" => Kind::FieldY(i?),
        "FLWT" => Kind::FieldW(i?),
        "PRWV" => Kind::Primary,
        "APER" => Kind::Aper,
        "SATP" => Kind::ApertureType,
        "STPS" => Kind::Stop,
        "FLTP" => Kind::FieldType,
        _ => return None,
    })
}
