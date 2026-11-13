//! 解析の結果（JSON）を見る。値の並んだオブジェクトの並び（視野ごとのスポットなど）は表に、ほかは木にする。
//! 図が入るまでの、どの解析にも使える表示。

use crate::i18n::{tr, trf};
use egui::{CollapsingHeader, RichText, Ui};
use serde_json::Value;

/// 長い数の並びは先頭だけ示す
const MAX_NUMS: usize = 12;
/// 表にする行の上限（超えた分は数だけ示す）
const MAX_ROWS: usize = 200;

/// 数を有効 6 桁くらいで（整数はそのまま）
pub fn fmt_num(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else if x != 0.0 && (x.abs() < 1e-3 || x.abs() >= 1e6) {
        format!("{x:.5e}")
    } else {
        let s = format!("{x:.6}");
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    }
}

/// 1 行で書ける値（数・文字列・真偽・null、短い数の並び）
fn inline(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("—".into()),
        Value::Bool(b) => Some(if *b { tr("はい") } else { tr("いいえ") }.into()),
        Value::Number(n) => Some(n.as_f64().map(fmt_num).unwrap_or_else(|| n.to_string())),
        Value::String(s) => Some(s.clone()),
        Value::Array(a) if a.iter().all(|x| x.is_number() || x.is_null()) => {
            let mut s: Vec<String> = a
                .iter()
                .take(MAX_NUMS)
                .map(|x| inline(x).unwrap())
                .collect();
            if a.len() > MAX_NUMS {
                s.push(trf("…（全 {} 個）", &[&a.len()]));
            }
            Some(format!("[{}]", s.join(", ")))
        }
        _ => None,
    }
}

/// オブジェクトの並びで、どれも 1 行で書ける値だけを持つなら、その列の名前（出てくる順）
fn table_columns(a: &[Value]) -> Option<Vec<String>> {
    if a.is_empty() {
        return None;
    }
    let mut cols: Vec<String> = Vec::new();
    for x in a {
        let o = x.as_object()?;
        for (k, v) in o {
            inline(v)?;
            if !cols.iter().any(|c| c == k) {
                cols.push(k.clone());
            }
        }
    }
    Some(cols)
}

pub fn show(ui: &mut Ui, v: &Value) {
    show_value(ui, tr("結果"), v, 0);
}

fn show_value(ui: &mut Ui, name: &str, v: &Value, depth: usize) {
    if let Some(s) = inline(v) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            ui.label(s);
        });
        return;
    }
    let id = ui.id().with((name, depth));
    match v {
        Value::Array(a) => {
            CollapsingHeader::new(trf("{}（{} 個）", &[&name, &a.len()]))
                .id_salt(id)
                .default_open(depth < 2)
                .show(ui, |ui| {
                    if let Some(cols) = table_columns(a) {
                        table(ui, id, &cols, a);
                    } else {
                        for (i, x) in a.iter().enumerate().take(MAX_ROWS) {
                            show_value(ui, &format!("{}", i + 1), x, depth + 1);
                        }
                        if a.len() > MAX_ROWS {
                            ui.label(trf("…（全 {} 個）", &[&a.len()]));
                        }
                    }
                });
        }
        Value::Object(o) => {
            CollapsingHeader::new(name)
                .id_salt(id)
                .default_open(depth < 2)
                .show(ui, |ui| {
                    for (k, x) in o {
                        show_value(ui, k, x, depth + 1);
                    }
                });
        }
        _ => {}
    }
}

/// 一様なオブジェクトの並びを表に。見える行だけ描くので行の上限は無い。見出しのクリックで並べ替え、
/// 数は右寄せ（GPN-16）
fn table(ui: &mut Ui, id: egui::Id, cols: &[String], rows: &[Value]) {
    use egui_extras::{Column, TableBuilder};
    // 並べ替え（列の番号、逆順）
    let sort_id = id.with("sort");
    let mut sort: Option<(usize, bool)> = ui.data(|d| d.get_temp(sort_id)).unwrap_or(None);
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some((c, desc)) = sort.filter(|s| s.0 < cols.len()) {
        let key = &cols[c];
        order.sort_by(|&a, &b| {
            let (x, y) = (rows[a].get(key), rows[b].get(key));
            let o = match (x.and_then(Value::as_f64), y.and_then(Value::as_f64)) {
                (Some(p), Some(q)) => p.total_cmp(&q),
                _ => {
                    let t = |v: Option<&Value>| v.and_then(inline).unwrap_or_default();
                    t(x).cmp(&t(y))
                }
            };
            if desc {
                o.reverse()
            } else {
                o
            }
        });
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        let mut t = TableBuilder::new(ui)
            .id_salt(id.with("table"))
            .striped(true)
            .resizable(true)
            .max_scroll_height(420.0)
            .column(Column::auto().at_least(30.0));
        for _ in cols {
            t = t.column(Column::auto().at_least(50.0));
        }
        t.header(row_h, |mut h| {
            h.col(|ui| {
                ui.strong("#");
            });
            for (k, c) in cols.iter().enumerate() {
                h.col(|ui| {
                    let mark = match sort {
                        Some((s, false)) if s == k => " ▲",
                        Some((s, true)) if s == k => " ▼",
                        _ => "",
                    };
                    if ui
                        .add(
                            egui::Label::new(RichText::new(format!("{c}{mark}")).strong())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(tr("クリックで並べ替え"))
                        .clicked()
                    {
                        sort = match sort {
                            Some((s, false)) if s == k => Some((k, true)),
                            Some((s, true)) if s == k => None,
                            _ => Some((k, false)),
                        };
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, order.len(), |mut row| {
                let i = order[row.index()];
                let r = &rows[i];
                row.col(|ui| {
                    ui.label(format!("{}", i + 1));
                });
                for c in cols {
                    row.col(|ui| {
                        let v = r.get(c);
                        let text = v.and_then(inline).unwrap_or_default();
                        if v.is_some_and(Value::is_number) {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(text);
                                },
                            );
                        } else {
                            ui.label(text);
                        }
                    });
                }
            });
        });
    });
    ui.data_mut(|d| d.insert_temp(sort_id, sort));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_are_short() {
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(33.124650689903426), "33.124651");
        assert_eq!(fmt_num(4e-10), "4.00000e-10");
    }

    #[test]
    fn flat_objects_become_a_table() {
        let a = json!([{"field": 1, "rms": 0.01}, {"field": 2, "rms": 0.02, "geo": 0.1}]);
        assert_eq!(
            table_columns(a.as_array().unwrap()),
            Some(vec!["field".into(), "rms".into(), "geo".into()])
        );
        let b = json!([{"field": 1, "rays": [{"x": 1}]}]);
        assert_eq!(table_columns(b.as_array().unwrap()), None);
    }
}
