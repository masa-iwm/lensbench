//! 解析の結果（JSON）を見る。値の並んだオブジェクトの並び（視野ごとのスポットなど）は表に、ほかは木にする。
//! 図が入るまでの、どの解析にも使える表示。

use crate::i18n::{tr, trf};
use egui::{CollapsingHeader, Grid, RichText, Ui};
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

fn table(ui: &mut Ui, id: egui::Id, cols: &[String], rows: &[Value]) {
    egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        Grid::new(id.with("grid"))
            .striped(true)
            .min_col_width(40.0)
            .show(ui, |ui| {
                ui.label(RichText::new("#").strong());
                for c in cols {
                    ui.label(RichText::new(c).strong());
                }
                ui.end_row();
                for (i, r) in rows.iter().enumerate().take(MAX_ROWS) {
                    ui.label(format!("{}", i + 1));
                    for c in cols {
                        ui.label(r.get(c).and_then(inline).unwrap_or_default());
                    }
                    ui.end_row();
                }
            });
    });
    if rows.len() > MAX_ROWS {
        ui.label(trf("…（全 {} 行）", &[&rows.len()]));
    }
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
