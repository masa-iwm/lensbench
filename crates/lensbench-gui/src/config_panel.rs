//! 多重構成のエディタ（OpticStudio の Multi-Configuration Editor に当たる表）。
//!
//! 行 = パラメータ（serve の `set` と同じ名前: `3.t`・`2.c`・`G1.shift` など）、列 = 構成。表は serve の
//! `config_table` から作り、変更は serve の `edit` の操作（`set_config_count`・`set_config_row`・`add_config_row`・
//! `remove_config_row`・`set_config`）で送る。.zmx の多重構成は `config_diff` で表に取り込む。

use egui::{Key, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};
use crate::theme;

/// serve の config_table の結果
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    /// 構成の数（表が無ければ 1）
    pub n: usize,
    /// 今の構成（1 始まり）
    pub active: usize,
    /// (パラメータの名前, 構成ごとの値)
    pub rows: Vec<(String, Vec<f64>)>,
    /// .zmx の多重構成の数と、読んだ構成
    pub zmx_cfgs: usize,
    pub zmx_cfg: usize,
}

pub fn parse(v: &Value) -> Table {
    let us = |k: &str| v[k].as_u64().unwrap_or(1).max(1) as usize;
    Table {
        n: us("n"),
        active: us("active"),
        rows: v["rows"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let vals = r["values"]
                            .as_array()
                            .map(|x| x.iter().map(|y| y.as_f64().unwrap_or(f64::NAN)).collect())
                            .unwrap_or_default();
                        (r["param"].as_str().unwrap_or_default().to_string(), vals)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        zmx_cfgs: us("zmx_cfgs"),
        zmx_cfg: us("zmx_cfg"),
    }
}

/// パネルの操作
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// serve の edit の操作
    Ops(Vec<Value>),
    /// .zmx の多重構成を表に取り込む（serve の config_diff）
    Import,
    /// 断った理由
    Refused(String),
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub table: Option<Table>,
    /// 書き換え中のセル（行、構成 0 始まり、文字、入力欄にフォーカスを渡したか）
    edit: Option<(usize, usize, String, bool)>,
    /// 足す行のパラメータの名前
    new_param: String,
    /// 取り込みで表にできなかった違い
    pub notes: Vec<String>,
}

/// 丸めない数の文字
fn full(x: f64) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "inf".into()
    }
}

/// 表の数（見る用、有効 6 桁）
fn short(x: f64) -> String {
    if !x.is_finite() {
        return "inf".into();
    }
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// 値の文字を数に（inf も）
fn parse_num(t: &str) -> Result<f64, String> {
    let t = t.trim();
    match t.to_ascii_lowercase().as_str() {
        "inf" | "infinity" => Ok(f64::INFINITY),
        "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
        _ => t.parse().map_err(|_| trf("数でない: {}", &[&t])),
    }
}

/// 行 r の構成 k の値を v にした set_config_row
fn set_cell(row: &(String, Vec<f64>), k: usize, v: f64) -> Value {
    let mut vals = row.1.clone();
    if let Some(x) = vals.get_mut(k) {
        *x = v;
    }
    json!({ "op": "set_config_row", "param": row.0, "values": vals })
}

pub fn ui(ui: &mut Ui, st: &mut State, combo: bool) -> Vec<Act> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let Some(t) = st.table.clone() else {
        ui.add_space(8.0);
        ui.label(tr("系を開くと、多重構成の表が出ます。"));
        return acts;
    };
    if combo {
        ui.label(
            RichText::new(tr(
                "組合せの系は作り直すと表が消えます。組合せの構成はプロジェクトの状態（states）で扱います。表を使うには系ファイルに保存して開いてください。",
            ))
            .small()
            .color(weak),
        );
    }
    // 構成の数・今の構成
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("構成")).color(weak));
        for k in 1..=t.n {
            if ui
                .selectable_label(k == t.active, k.to_string())
                .on_hover_text(tr("この構成の値を系に入れる（解析・図・面の表が追従）"))
                .clicked()
                && k != t.active
            {
                acts.push(Act::Ops(vec![json!({ "op": "set_config", "k": k })]));
            }
        }
        ui.separator();
        if ui
            .button("＋")
            .on_hover_text(tr("構成を足す（値は今の構成と同じ）"))
            .clicked()
        {
            acts.push(Act::Ops(vec![
                json!({ "op": "set_config_count", "n": t.n + 1 }),
            ]));
        }
        if ui
            .add_enabled(t.n > 1, egui::Button::new("−"))
            .on_hover_text(tr("最後の構成を消す（1 つにすると表も消える）"))
            .clicked()
        {
            let mut ops = Vec::new();
            if t.active == t.n {
                ops.push(json!({ "op": "set_config", "k": t.n - 1 }));
            }
            ops.push(json!({ "op": "set_config_count", "n": t.n - 1 }));
            acts.push(Act::Ops(ops));
        }
    });
    if t.zmx_cfgs > 1 && t.rows.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(trf(
                "この .zmx は {} 個の構成を持ち、構成 {} を読んでいます。",
                &[&t.zmx_cfgs, &t.zmx_cfg],
            ));
            if ui
                .button(tr("表に取り込む"))
                .on_hover_text(tr(
                    "各構成を読み、構成で違う値（曲率・コーニック・厚さ・非球面・傾き偏心・dn・disp）を表の行にする",
                ))
                .clicked()
            {
                acts.push(Act::Import);
            }
        });
    }
    for n in &st.notes {
        ui.label(RichText::new(n).small().color(ui.visuals().warn_fg_color));
    }
    ui.add_space(4.0);

    // 表
    let mut commit: Option<(usize, usize, String)> = None;
    let mut remove: Option<usize> = None;
    egui::ScrollArea::horizontal()
        .id_salt("mce_h")
        .show(ui, |ui| {
            // 文字を選べるラベルはクリックを取ってしまう（セルの空き・行のクリックが行に届かない）
            ui.style_mut().interaction.selectable_labels = false;
            let mut table = TableBuilder::new(ui)
                .id_salt("mce")
                .auto_shrink([false, true])
                .striped(true)
                .resizable(true)
                .column(Column::initial(90.0).at_least(60.0));
            for _ in 0..t.n {
                table = table.column(Column::initial(90.0).at_least(50.0));
            }
            table
                .column(Column::exact(24.0))
                .header(22.0, |mut h| {
                    h.col(|ui| {
                        ui.strong(tr("パラメータ"));
                    });
                    for k in 1..=t.n {
                        h.col(|ui| {
                            let rt = RichText::new(trf("構成 {}", &[&k])).strong();
                            ui.label(if k == t.active { rt.color(accent) } else { rt });
                        });
                    }
                    h.col(|_| {});
                })
                .body(|body| {
                    body.rows(20.0, t.rows.len(), |mut row| {
                        let i = row.index();
                        let (name, vals) = &t.rows[i];
                        row.col(|ui| {
                            ui.label(RichText::new(name).monospace());
                        });
                        for k in 0..t.n {
                            row.col(|ui| {
                                if let Some(e) = st.edit.as_mut().filter(|e| e.0 == i && e.1 == k) {
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut e.2)
                                            .desired_width(f32::INFINITY)
                                            .horizontal_align(egui::Align::RIGHT),
                                    );
                                    if !e.3 {
                                        // 開いたら全部を選んで、打てばそのまま置き換わる
                                        resp.request_focus();
                                        if let Some(mut s) =
                                            egui::TextEdit::load_state(ui.ctx(), resp.id)
                                        {
                                            let all = egui::text::CCursorRange::two(
                                                egui::text::CCursor::new(0),
                                                egui::text::CCursor::new(e.2.chars().count()),
                                            );
                                            s.cursor.set_char_range(Some(all));
                                            s.store(ui.ctx(), resp.id);
                                        }
                                        e.3 = true;
                                    }
                                    if ui.input(|x| x.key_pressed(Key::Escape)) {
                                        st.edit = None;
                                    } else if resp.lost_focus() {
                                        commit = Some((i, k, e.2.clone()));
                                    }
                                    return;
                                }
                                let v = vals.get(k).copied().unwrap_or(f64::NAN);
                                let mut rt = RichText::new(short(v)).monospace();
                                if k + 1 == t.active {
                                    rt = rt.color(accent);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.label(rt),
                                );
                                // セルのどこをクリックしても書き換える
                                let resp = ui
                                    .interact(
                                        ui.max_rect(),
                                        ui.id().with(("mce", i, k)),
                                        Sense::click(),
                                    )
                                    .on_hover_text(full(v));
                                if resp.double_clicked() || resp.clicked() {
                                    st.edit = Some((i, k, full(v), false));
                                }
                            });
                        }
                        row.col(|ui| {
                            if ui
                                .small_button("✖")
                                .on_hover_text(tr("行を消す（系の値は今のまま）"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    });
                });
        });
    if let Some((i, k, text)) = commit {
        st.edit = None;
        if let Some(r) = t.rows.get(i) {
            match parse_num(&text) {
                // set_config_row は今の構成の値を系に入れ直す（ほかの構成の値を変えても系はそのまま）
                Ok(v) if r.1.get(k) != Some(&v) => acts.push(Act::Ops(vec![set_cell(r, k, v)])),
                Ok(_) => {}
                Err(e) => acts.push(Act::Refused(e)),
            }
        }
    }
    if let Some(i) = remove {
        acts.push(Act::Ops(vec![
            json!({ "op": "remove_config_row", "param": t.rows[i].0 }),
        ]));
    }
    // 行を足す
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut st.new_param)
                .hint_text("3.t・2.c・STOP.k・G1.shift")
                .desired_width(160.0),
        );
        let enter = resp.lost_focus() && ui.input(|x| x.key_pressed(Key::Enter));
        let add = ui
            .add_enabled(
                !st.new_param.trim().is_empty(),
                egui::Button::new(tr("行を足す")),
            )
            .on_hover_text(tr(
                "パラメータを表に足す（名前は Python・serve の set と同じ。どの構成も今の値から）",
            ))
            .clicked();
        if (add || enter) && !st.new_param.trim().is_empty() {
            let mut ops = Vec::new();
            if t.n < 2 {
                ops.push(json!({ "op": "set_config_count", "n": 2 }));
            }
            ops.push(json!({ "op": "add_config_row", "param": st.new_param.trim() }));
            acts.push(Act::Ops(ops));
            st.new_param.clear();
        }
    });
    if t.n < 2 && t.zmx_cfgs < 2 {
        ui.label(
            RichText::new(tr(
                "構成は 1 つです。＋ で構成を足すか、行を足すと構成 2 つの表になります。",
            ))
            .small()
            .color(weak),
        );
    }
    acts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_cell() {
        let t = parse(&json!({
            "n": 2, "active": 2, "zmx_cfgs": 2, "zmx_cfg": 2,
            "rows": [{ "param": "2.t", "values": [90.0, 80.0] }],
        }));
        assert_eq!((t.n, t.active, t.zmx_cfgs), (2, 2, 2));
        assert_eq!(t.rows[0], ("2.t".to_string(), vec![90.0, 80.0]));
        assert_eq!(
            set_cell(&t.rows[0], 0, 85.0),
            json!({ "op": "set_config_row", "param": "2.t", "values": [85.0, 80.0] })
        );
        assert_eq!(parse_num(" inf ").unwrap(), f64::INFINITY);
        assert!(parse_num("x").is_err());
        assert_eq!(short(0.1 + 0.2), "0.3");
        assert_eq!(short(-0.0), "0");
    }
}
