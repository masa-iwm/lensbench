//! 解析の結果（JSON）を見る。値の並んだオブジェクトの並び（視野ごとのスポットなど）は表に、ほかは木にする。
//! 図が入るまでの、どの解析にも使える表示。

use crate::i18n::{n_, tr, trf};
use egui::{CollapsingHeader, RichText, Ui};
use serde_json::Value;

/// 長い数の並びは先頭だけ示す
const MAX_NUMS: usize = 12;
/// 表にする行の上限（超えた分は数だけ示す）
const MAX_ROWS: usize = 200;

/// 数を有効 6 桁くらいで（整数はそのまま）
pub(crate) fn fmt_num(x: f64) -> String {
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

/// 表の列の名前と、結果の `unit`（位置・長さの単位、波など）で数える列か。ここに無い鍵はそのまま出す
const COLS: &[(&str, &str, bool)] = &[
    ("field", n_("Field #"), false),
    ("wave", n_("Wave #"), false),
    ("rms_centroid", n_("RMS (centroid)"), true),
    ("rms_chief", n_("RMS (chief ray)"), true),
    ("geo", n_("GEO radius"), true),
    ("centroid", n_("Centroid"), true),
    ("chief", n_("Chief ray"), true),
    ("pv", n_("PV"), true),
    ("pv_centroid", n_("PV (centroid)"), true),
    ("rms", n_("RMS"), true),
    ("rms_fit", n_("RMS of fit"), true),
    ("max_fit", n_("Max fit error"), true),
    ("strehl", n_("Strehl ratio"), false),
];

/// 値の見せ方: 単位の倍率（mm の結果は µm で出す）と見出しの単位
#[derive(Clone, Copy)]
struct Units<'a> {
    unit: Option<&'a str>,
}

impl Units<'_> {
    /// 鍵の名前・単位・倍率。単位を持たない鍵は倍率 1
    fn of(&self, key: &str) -> (String, f64) {
        let Some(&(_, name, in_unit)) = COLS.iter().find(|c| c.0 == key) else {
            return (key.to_string(), 1.0);
        };
        let name = tr(name);
        match (in_unit, self.unit) {
            (true, Some("mm")) => (format!("{name} [µm]"), 1e3),
            (true, Some("waves")) => (format!("{name} [{}]", tr("waves")), 1.0),
            (true, Some(u)) => (format!("{name} [{u}]"), 1.0),
            _ => (name.to_string(), 1.0),
        }
    }
}

/// 単位を持つ値（位置・長さ）に倍率を掛ける（数と数の並び）。計算の丸めの残り（1e-12 より小さい）は 0 にする。
/// 単位の無い値（係数・比など）は小さくてもそのまま
fn scaled(v: &Value, k: f64) -> Value {
    if k == 1.0 {
        return v.clone();
    }
    match v {
        Value::Number(n) => n.as_f64().map_or(v.clone(), |x| {
            Value::from(if x.abs() < 1e-12 { 0.0 } else { x * k })
        }),
        Value::Array(a) => Value::Array(a.iter().map(|x| scaled(x, k)).collect()),
        _ => v.clone(),
    }
}

/// 1 行で書ける値（数・文字列・真偽・null、短い数の並び）
fn inline(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("—".into()),
        Value::Bool(b) => Some(if *b { tr("yes") } else { tr("no") }.into()),
        Value::Number(n) => Some(n.as_f64().map(fmt_num).unwrap_or_else(|| n.to_string())),
        Value::String(s) => Some(s.clone()),
        Value::Array(a) if a.iter().all(|x| x.is_number() || x.is_null()) => {
            let mut s: Vec<String> = a
                .iter()
                .take(MAX_NUMS)
                .map(|x| inline(x).unwrap())
                .collect();
            if a.len() > MAX_NUMS {
                s.push(trf("… ({} in all)", &[&a.len()]));
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

pub(crate) fn show(ui: &mut Ui, v: &Value) {
    let u = Units {
        unit: v["unit"].as_str(),
    };
    show_value(ui, u, tr("Result"), v, 0);
}

fn show_value(ui: &mut Ui, u: Units, name: &str, v: &Value, depth: usize) {
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
            CollapsingHeader::new(trf("{} ({})", &[&name, &a.len()]))
                .id_salt(id)
                .default_open(depth < 2)
                .show(ui, |ui| {
                    if let Some(cols) = table_columns(a) {
                        table(ui, u, id, &cols, a);
                    } else {
                        for (i, x) in a.iter().enumerate().take(MAX_ROWS) {
                            show_value(ui, u, &format!("{}", i + 1), x, depth + 1);
                        }
                        if a.len() > MAX_ROWS {
                            ui.label(trf("… ({} in all)", &[&a.len()]));
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
                        // 名前・単位は 1 行の値（数・数の並び）にだけ。中に並びやオブジェクトを持つ鍵
                        // （rms の結果の rms など）は、中の値の単位がそろわないのでそのまま
                        if inline(x).is_some() {
                            let (name, f) = u.of(k);
                            show_value(ui, u, &name, &scaled(x, f), depth + 1);
                        } else {
                            show_value(ui, u, k, x, depth + 1);
                        }
                    }
                });
        }
        _ => {}
    }
}

/// 一様なオブジェクトの並びを表に。見える行だけ描くので行の上限は無い。見出しのクリックで並べ替え、
/// 数は右寄せ
fn table(ui: &mut Ui, u: Units, id: egui::Id, cols: &[String], rows: &[Value]) {
    let heads: Vec<(String, f64)> = cols.iter().map(|c| u.of(c)).collect();
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
                let head = &heads[k].0;
                h.col(|ui| {
                    let mark = match sort {
                        Some((s, false)) if s == k => " ▲",
                        Some((s, true)) if s == k => " ▼",
                        _ => "",
                    };
                    if ui
                        .add(
                            egui::Label::new(RichText::new(format!("{head}{mark}")).strong())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(format!("{c}\n{}", tr("Click to sort")))
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
                for (c, (_, f)) in cols.iter().zip(&heads) {
                    row.col(|ui| {
                        let v = r.get(c).map(|x| scaled(x, *f));
                        let v = v.as_ref();
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
        crate::widgets::scroll_gap(ui);
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
    fn spot_columns_in_um() {
        let u = Units { unit: Some("mm") };
        let (name, f) = u.of("rms_centroid");
        assert!(name.ends_with("[µm]"), "{name}");
        assert_eq!(f, 1e3);
        assert_eq!(u.of("field").1, 1.0);
        assert_eq!(u.of("whatever"), ("whatever".to_string(), 1.0));
        assert_eq!(scaled(&json!([0.001, 0.002]), 1e3), json!([1.0, 2.0]));
        assert_eq!(inline(&scaled(&json!(-3.4e-17), 1e3)).as_deref(), Some("0"));
        // 単位の無い値は小さくても 0 にしない
        assert_eq!(inline(&json!(4e-13)), Some("4.00000e-13".into()));
        let w = Units {
            unit: Some("waves"),
        };
        assert_eq!(w.of("rms").1, 1.0);
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
