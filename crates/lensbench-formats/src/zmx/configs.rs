//! .zmx の多重構成を多重構成の表にする

use super::*;

/// .zmx の多重構成（MCE）を多重構成の表（[`lensbench_core::configs::Configs`]）にする。表せる演算子は THIC・CRVT・CONN と、
/// 偶数次非球面・座標ブレークの PRAM、材料の GLSS、膜の COTN、モデルガラスの MIND・MABB・MDPG（モデルガラスでない面の行は
/// OpticStudio も使わないので表に入れない）、熱膨張係数の TCEX、系の値の WAVE・WLWT・PRWV・APER・SATP・STPS・FLTP・XFIE・YFIE・FLWT・FVDX・FVDY・FVCX・FVCY・FVAN・FTAN・TEMP・PRES・AFOC・APDT・APDF・RAAM・OPDR。表せない演算子が構成で変われば、その演算子の名前を Err で返す（構成で変わらない
/// 演算子・光線追跡に効かない演算子は表に入れない）
pub(super) fn config_table(
    raw: &Raw,
    s: &System,
    links: (
        &[lensbench_core::solves::Solve],
        &[lensbench_core::pickup::Pickup],
    ),
    n: usize,
    active: usize,
    notes: &mut Vec<String>,
) -> lensbench_core::Result<lensbench_core::configs::Configs> {
    use lensbench_core::configs::{ConfigPick, ConfigRow, Configs};
    use lensbench_core::param::{Kind, Param};
    type Row = (String, i64, i64, Vec<Option<String>>, Vec<ConfigPick>);
    let mut rows: Vec<Row> = Vec::new();
    for (op, p1, c, v, rest) in &raw.mce {
        let parm = if op == "PRAM" || op.starts_with("PSP") {
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
                rows.push((op.clone(), *p1, parm, vec![None; n], Vec::new()));
                rows.len() - 1
            }
        };
        if (1..=n as i64).contains(c) {
            rows[i].3[*c as usize - 1] = Some(v.clone());
            rows[i].4.retain(|q| q.k != *c as usize);
            if let Some(q) = config_pick(rest, *c, n)? {
                rows[i].4.push(q);
            }
        }
    }
    let mut out = Vec::new();
    for (op, p1, parm, raw_vals, picks) in rows {
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
        // 構成のピックアップのある行は、値が同じでもつながりを残す
        let changes = both.iter().any(|v| !same(v, &both[0]));
        let varies = changes || !picks.is_empty();
        let surf = usize::try_from(p1).ok().filter(|&k| k < s.surfaces.len());
        if let ("COTN", Some(j), true) = (op.as_str(), surf, varies) {
            // 膜の行: 書いていない構成は今の膜
            let now = s.surfaces[j].coat.clone();
            let mut row = ConfigRow {
                param: Param::new(j, Kind::Coating),
                values: Vec::new(),
                names: Vec::new(),
                picks: Vec::new(),
            };
            for v in &raw_vals {
                let n = v
                    .as_deref()
                    .map_or(now.clone(), |x| x.trim_matches('"').trim().to_string());
                let i = match row.names.iter().position(|x| *x == n) {
                    Some(i) => i,
                    None => {
                        row.names.push(n);
                        row.names.len() - 1
                    }
                };
                row.values.push(i as f64);
            }
            out.push(row);
            continue;
        }
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
            // ソルブ・ピックアップの条件の値。ソルブもピックアップも無い面の行は OpticStudio でも効かないので表に入れない
            (o, Some(k)) if solve_kind(o, parm).is_some() => {
                let kind = solve_kind(o, parm);
                let p = Param::new(k, kind.expect("checked in the guard"));
                let Ok(now) = lensbench_core::param::solve_value(links.0, links.1, p) else {
                    if changes {
                        notes.push(format!(
                            "multi-configuration row {op} {p1}: surface {k} has no solve or pickup it sets (OpticStudio does not use it either); not kept in the table"
                        ));
                    }
                    continue;
                };
                if varies {
                    out.retain(|r| r.param != p);
                    out.push(numeric_row(p, &vals, now, picks));
                }
                continue;
            }
            // モデルガラスでない面の行は OpticStudio も使わない（材料はカタログのまま）
            ("MIND" | "MABB" | "MDPG", Some(k))
                if !matches!(
                    s.surfaces[k].mat,
                    lensbench_core::material::Material::Model { .. }
                ) =>
            {
                continue
            }
            // 開口の最小の半径・偏心・種類、面の前・後の傾き偏心の値と順序
            (o, Some(_)) if surface_table_kind(o).is_some() => surface_table_kind(o),
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
                // 書いていない構成の値は今の系の値（全部の構成に値があれば使わない。今の構成で開口の無い面の半径の行など）。
                // 同じパラメータの行（FVAN と FTAN）は後の行が勝つ
                let now = match s.get_param(p) {
                    Err(_) if vals.iter().all(Option::is_some) => 0.0,
                    v => v?,
                };
                out.retain(|r| r.param != p);
                out.push(numeric_row(p, &vals, now, picks));
            }
            None if changes
                && !matches!(op.as_str(), "MCOM" | "CWGT" | "MOFF" | "LTTL" | "PUCN") =>
            {
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

/// 数の行。書いていない構成の値は今の系の値 now。構成のピックアップの値は元の構成から求め直す
/// （ファイルの値は OpticStudio が求めた値で、同じになる）
fn numeric_row(
    p: lensbench_core::param::Param,
    vals: &[Option<f64>],
    now: f64,
    picks: Vec<lensbench_core::configs::ConfigPick>,
) -> lensbench_core::configs::ConfigRow {
    let mut r = lensbench_core::configs::ConfigRow {
        param: p,
        values: vals.iter().map(|v| v.unwrap_or(now)).collect(),
        names: Vec::new(),
        picks,
    };
    r.resolve_picks();
    r
}

/// 多重構成の行の構成 c のセルの構成のピックアップ（[`super::parse`] で自分の行からのものだけにした欄）
fn config_pick(
    rest: &[String],
    c: i64,
    n: usize,
) -> lensbench_core::Result<Option<lensbench_core::configs::ConfigPick>> {
    let Some(t) = rest
        .first()
        .and_then(|t| t.parse::<i64>().ok())
        .filter(|&t| t >= 2)
    else {
        return Ok(None);
    };
    let from = usize::try_from(t - 1).unwrap_or(0);
    if from == 0 || from > n || from as i64 == c {
        invalid!(
            "configuration {c}: configuration pickup from {from} (1 to {n}, another configuration)"
        );
    }
    let num = |i: usize| rest.get(i).map_or(Ok(0.0), |x| fv(x));
    Ok(Some(lensbench_core::configs::ConfigPick {
        k: c as usize,
        from,
        scale: num(5)?,
        offset: num(6)?,
    }))
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

/// 面の多重構成の表だけの値の演算子（APMN・APDX・APDY・APTP・CBOR・CAOR・MIND・MABB・MDPG・TCEX、面の前後の傾き偏心 CBDX〜CBTZ・CADX〜CATZ。
/// 値の位置は dx dy tx ty tz の順）の種類（[`lensbench_core::param::set_surface_table_value`] で入れる種類）
fn surface_table_kind(op: &str) -> Option<lensbench_core::param::Kind> {
    use lensbench_core::param::Kind;
    match op {
        "APMN" => return Some(Kind::ApMin),
        "APDX" => return Some(Kind::ApDx),
        "APDY" => return Some(Kind::ApDy),
        "APTP" => return Some(Kind::ApType),
        "MIND" => return Some(Kind::ModelNd),
        "MABB" => return Some(Kind::ModelVd),
        "MDPG" => return Some(Kind::ModelDpgf),
        "TCEX" => return Some(Kind::Tce),
        "CBOR" => return Some(Kind::BeforeOrder),
        "CAOR" => return Some(Kind::AfterOrder),
        _ => {}
    }
    let i = ["DX", "DY", "TX", "TY", "TZ"]
        .iter()
        .position(|t| op.get(2..) == Some(*t))? as u8;
    match op.get(..2)? {
        "CB" => Some(Kind::Before(i)),
        "CA" => Some(Kind::After(i)),
        _ => None,
    }
}

/// ソルブ・ピックアップの条件の値の演算子（CSP1・CSP2・TSP1〜TSP3・PSP1〜PSP3、PSP は PARM の番号 parm）の種類
fn solve_kind(op: &str, parm: i64) -> Option<lensbench_core::param::Kind> {
    use lensbench_core::param::Kind;
    let i = op.get(3..)?.parse::<u8>().ok()?.checked_sub(1)?;
    match op.get(..3)? {
        "CSP" if i < 2 => Some(Kind::CurvSolve(i)),
        "TSP" if i < 3 => Some(Kind::ThickSolve(i)),
        "PSP" if i < 3 => Some(Kind::ParmSolve(u8::try_from(parm).ok()?, i)),
        _ => None,
    }
}

/// 読む構成のソルブ・ピックアップの条件の値（CSP・TSP・PSP）を rx のソルブ・ピックアップに入れる（.zmx を今の構成でない
/// 構成で読むとき。rx のソルブ・ピックアップを作った後に呼ぶ）
pub(super) fn apply_solve_values(ops: &Ops, rx: &mut lensbench_core::system::Prescription) {
    use lensbench_core::param::Param;
    for ((op, p1), v) in ops {
        // PSP の鍵は「PSPi:PARM の番号」（[`super::values::config_ops`]）
        let (name, parm) = match op.split_once(':') {
            Some((o, n)) => (o, n.parse::<i64>().unwrap_or(-1)),
            None => (op.as_str(), 0),
        };
        let (Some(k), Ok(j), Ok(v)) = (solve_kind(name, parm), usize::try_from(*p1), fv(v)) else {
            continue;
        };
        lensbench_core::param::set_solve_value(rx, Param::new(j, k), v);
    }
}

/// 読む構成の多重構成の値のうち、面・系の値を作るときに見ない演算子（温度・気圧・アフォーカル・アポダイゼーション・光線
/// エイミング・OPD の基準と、[`surface_table_kind`] の面の値）を rx に入れる（.zmx を今の構成でない構成で読むとき）
pub(super) fn apply_config_values(ops: &Ops, rx: &mut lensbench_core::system::Prescription) {
    use lensbench_core::pupil::Apodization;
    use lensbench_core::system::Aiming;
    let num = |op: &str, p1: i64| ops.get(&(op.to_string(), p1)).and_then(|v| fv(v).ok());
    if let Some(v) = num("TEMP", 0) {
        rx.temperature = v;
    }
    if let Some(v) = num("PRES", 0) {
        rx.pressure = v;
    }
    if let Some(v) = num("AFOC", 0) {
        rx.afocal = v != 0.0;
    }
    if num("APDT", 0).is_some() || num("APDF", 0).is_some() {
        let t = num("APDT", 0).map_or(rx.apodization.os_code(), |v| v as i64);
        let g = num("APDF", 0).unwrap_or(rx.apodization.factor());
        rx.apodization = Apodization::from_os_code(t, g);
    }
    if let Some(v) = num("RAAM", 0) {
        rx.aiming = Aiming::from_os_code(v as i64);
    }
    if let Some(v) = num("OPDR", 0) {
        rx.ropd = v as i64;
    }
    for ((op, p1), v) in ops {
        let (Some(k), Ok(j), Ok(v)) = (surface_table_kind(op), usize::try_from(*p1), fv(v)) else {
            continue;
        };
        if let Some(d) = rx.surfaces.get_mut(j) {
            lensbench_core::param::set_surface_table_value(d, k, v);
        }
    }
    for ((op, p1), v) in ops {
        if let (true, Some(d)) = (
            op == "COTN",
            usize::try_from(*p1)
                .ok()
                .and_then(|j| rx.surfaces.get_mut(j)),
        ) {
            d.coat = v.trim_matches('"').trim().to_string();
        }
    }
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
        "FVDX" => Kind::FieldVig(i?, 0),
        "FVDY" => Kind::FieldVig(i?, 1),
        "FVCX" => Kind::FieldVig(i?, 2),
        "FVCY" => Kind::FieldVig(i?, 3),
        // FTAN（視野の Tangential Angle）。FVAN（同じ値）は読むときに FTAN にしている
        "FTAN" => Kind::FieldVig(i?, 4),
        "PRWV" => Kind::Primary,
        "APER" => Kind::Aper,
        "SATP" => Kind::ApertureType,
        "STPS" => Kind::Stop,
        "FLTP" => Kind::FieldType,
        "TEMP" => Kind::Temp,
        "PRES" => Kind::Pres,
        "AFOC" => Kind::Afocal,
        "APDT" => Kind::ApodType,
        "APDF" => Kind::Apod,
        "RAAM" => Kind::Aiming,
        "OPDR" => Kind::Ropd,
        _ => return None,
    })
}
