//! `optimize --zmx-merit`: .zmx に保存したメリットファンクションを最適化の項にする

use crate::{overlay_systems, warn_notes, AppError, Common, ConfigSel};
use lensbench_core::zmx::merit::MeritRow;
use lensbench_core::System;
use serde_json::Value;

/// .zmx から読んだメリットファンクション
pub(crate) struct ZmxMerit {
    /// 写した項
    pub terms: Vec<Value>,
    /// 項が 2 つ以上の構成を使うときは構成 1〜n の系（1 つなら空）
    pub systems: Vec<System>,
    /// --out の .zmx にそのまま書き戻す行（写せなかった行も含む。レンズ単位が mm でなければ空）
    pub rows: Vec<MeritRow>,
}

/// .zmx の評価関数を写した項。写せない行は標準エラーへ
pub(crate) fn terms(c: &Common, s: &System) -> Result<ZmxMerit, AppError> {
    let is_zmx = c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("zmx"));
    if !is_zmx {
        return Err(AppError::usage("--zmx-merit needs a .zmx file"));
    }
    let mf = lensbench_core::zmx::merit::merit_function(&c.file)?;
    let z = lensbench_design::merit_zmx::terms_from_zmx(&mf, s.primary + 1, s.image());
    warn_notes(z.notes);
    // 書き出す .zmx は mm なので、ほかの単位の行は長さの目標が合わない
    let rows = if mf.lens_unit_mm == 1.0 {
        mf.rows
    } else {
        Vec::new()
    };
    let systems = if z.configs <= 1 {
        vec![]
    } else {
        let sel = ConfigSel {
            cfgs: Some(
                (1..=z.configs)
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ..Default::default()
        };
        overlay_systems(c, &sel)?
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    };
    // 項にできないもの（構成・視野の番号が系に無いなど）は行の番号つきで飛ばす
    let check: &[System] = if systems.is_empty() {
        std::slice::from_ref(s)
    } else {
        &systems
    };
    let mut terms = Vec::new();
    let mut notes = Vec::new();
    for (t, row) in z.terms.into_iter().zip(z.sources) {
        match lensbench_design::merit::term_from_json(&t, check) {
            Ok(_) => terms.push(t),
            Err(e) => notes.push(format!(".zmx merit function row {row}: {e}; skipped")),
        }
    }
    warn_notes(notes);
    if terms.is_empty() {
        return Err(AppError::usage(format!(
            "{}: no merit function terms to use",
            c.file.display()
        )));
    }
    Ok(ZmxMerit {
        terms,
        systems,
        rows,
    })
}

/// --out の .zmx に書くメリットファンクションの行: .zmx から読んだ行と、--term・--terms の項を写した行
pub(crate) fn rows_to_write(
    zmx_rows: Vec<MeritRow>,
    terms: &[Value],
    systems: &[System],
) -> Vec<MeritRow> {
    let (more, notes) = lensbench_design::merit_zmx::rows_from_terms(terms, systems, &zmx_rows);
    warn_notes(notes);
    // 読めない行は番号（OPGT などの参照）を保つために BLNK にして元の文字をコメントに残す
    let mut rows: Vec<MeritRow> = zmx_rows
        .into_iter()
        .map(|r| {
            if r.readable() {
                r
            } else {
                MeritRow::blank(&r.text())
            }
        })
        .collect();
    rows.extend(more);
    if systems.len() > 1 {
        warn_notes(vec![
            "the .zmx is written from configuration 1; the multi-configuration table keeps the values read".into(),
        ]);
    }
    rows
}
