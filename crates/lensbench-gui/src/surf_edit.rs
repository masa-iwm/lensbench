//! 面の詳細設定: 面の表で選んだ面の、表に無い項目を編集する欄（表の「詳細設定」で出す）。
//!
//! 種類・曲率半径・厚さ・硝材・膜・半径・コーニック・面タイプのパラメータ・ソルブ・ピックアップ・絞り・無視は
//! 面の表で変える（OpticStudio の Lens Data のように。屈折率のずれ dn・分散の変化 disp・複屈折は硝材のセルの [...]）。
//! ここは表に出ない面タイプの非球面と、
//! 開口・面の前後の傾き偏心・面タイプの形のデータ（Zernike・Grid Sag・Binary 2・Cylinder Fresnel・GRIN・利用者定義の面）を JSON で。
//! 値は serve の `surface_data` で読み、「全項目（JSON）」では面のデータをそのまま書き換えられる。書き換えは [`Act`] で GUI に返す。

use std::collections::HashMap;

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{n_, tr, trf};
use crate::lens_data::Act;
use crate::theme;

/// 面データの typ（serde の名前）と .zmx の名前
const TYP_NAMES: &[(&str, &str)] = &[
    ("Standard", "STANDARD"),
    ("EvenAsph", "EVENASPH"),
    ("CoordBrk", "COORDBRK"),
    ("Jones", "JONESMAT"),
    ("Paraxial", "PARAXIAL"),
    ("Toroidal", "TOROIDAL"),
    ("ZernikeStd", "SZERNSAG"),
    ("ZernikeFringe", "FZERNSAG"),
    ("GridSag", "GRID_SAG"),
    ("Fresnel", "FRESNELS"),
    ("CylFresnel", "CFRESNEL"),
    ("Binary2", "BINARY_2"),
    ("OddAsph", "ODDASPHE"),
    ("TiltSurf", "TILTSURF"),
    ("Irregular", "IRREGULA"),
    ("Gradium", "GRINSUR8"),
    ("Gradient9", "GRINSUR9"),
    ("User", "USER"),
];

/// 面データの typ を .zmx の名前に
pub(crate) fn zmx_type(typ: &str) -> &'static str {
    TYP_NAMES
        .iter()
        .find(|x| x.0 == typ)
        .map(|x| x.1)
        .unwrap_or("STANDARD")
}

/// JSON で書き換える面のデータの項目（名前、表示名）
const JSON_ITEMS: &[(&str, &str)] = &[
    ("ap", n_("Aperture")),
    ("before", n_("Tilt/decenter before")),
    ("after", n_("Tilt/decenter after")),
    ("zern", n_("Zernike terms")),
    ("grid", n_("Grid Sag grid")),
    ("bin2", n_("Binary 2 phase")),
    ("cylf", "Cylinder Fresnel"),
    ("grin", n_("Gradient index (GRIN)")),
    ("user", n_("User-defined surface")),
];

/// 面の設定の状態
#[derive(Debug, Clone, Default)]
pub(crate) struct Inspector {
    /// 読んだ面と、serve の surface_data の結果
    pub data: Option<(usize, Value)>,
    /// 頼んだ面（結果を待っている）
    pub asked: Option<usize>,
    /// 入力欄の文字（項目ごと。値を読み直したら捨てる）
    bufs: HashMap<String, String>,
    /// 全項目（JSON）の文字
    all: Option<String>,
}

impl Inspector {
    /// 面 j の値を読み直す（結果は [`Inspector::set`]）
    pub(crate) fn stale(&mut self) {
        self.data = None;
        self.asked = None;
    }

    pub(crate) fn set(&mut self, j: usize, v: Value) {
        self.data = Some((j, v));
        self.asked = None;
        self.bufs.clear();
        self.all = None;
    }
}

/// 複屈折材の板の欄（入口: mode・結晶軸、出口）。変えたら面のデータの bire の値（ne は材料「名前-E」から作り直す）
pub(crate) fn bire_ui(ui: &mut Ui, dj: usize, cur: &Value) -> Option<Value> {
    let kind = match cur {
        Value::String(s) if s == "Out" => 2,
        Value::Object(o) if o.contains_key("In") => 1,
        _ => 0,
    };
    let inn = &cur["In"];
    let mut mode = inn["mode"].as_u64().unwrap_or(2);
    let mut axis: [f64; 3] = std::array::from_fn(|i| inn["axis"][i].as_f64().unwrap_or(0.0));
    if kind != 1 {
        axis = [0.0, 1.0, 0.0];
    }
    let mut k = kind;
    let mut out = None;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for (i, l) in [tr("Off"), tr("In"), tr("Out")].iter().enumerate() {
                ui.selectable_value(&mut k, i, *l);
            }
        });
        let mut changed = k != kind;
        if k == 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("Mode"));
                for (m, l) in [
                    (2, tr("Wave plate (2)")),
                    (0, tr("Ordinary only (0)")),
                    (3, tr("Extraordinary path, wave plate (3)")),
                    (1, tr("Extraordinary only (1)")),
                ] {
                    changed |= ui.selectable_value(&mut mode, m, l).changed();
                }
                ui.label(tr("Crystal axis"));
                for (i, a) in axis.iter_mut().enumerate() {
                    changed |= ui
                        .add(
                            crate::widgets::Num::new(a)
                                .max_decimals(6)
                                .prefix(["x ", "y ", "z "][i]),
                        )
                        .accessible_name(format!("{} {}", tr("Crystal axis"), ["x", "y", "z"][i]))
                        .changed();
                }
            });
            ui.label(
                RichText::new(trf(
                    "The material after surface {} is the ordinary ray and material-E the extraordinary ray (must be in the catalog). Use with an Out surface.",
                    &[&dj],
                ))
                .small()
                .color(theme::weak_text(ui.visuals())),
            );
        }
        if changed {
            out = Some(match k {
                1 => json!({ "In": { "mode": mode, "axis": axis } }),
                2 => json!("Out"),
                _ => Value::Null,
            });
        }
    });
    out
}

/// 書き換え用の数の文字（丸めない）
pub(crate) fn full(x: f64) -> String {
    if x.is_infinite() {
        if x > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        format!("{x}")
    }
}

fn nums_text(v: &Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| crate::widgets::json_num(x).map(full).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// 「1, 2.5, -3」を数の並びに
pub(crate) fn parse_nums(s: &str) -> Result<Vec<f64>, String> {
    s.split([',', ' ', ';'])
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            x.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| trf("{} is not a number", &[&x.trim()]))
        })
        .collect()
}

/// 入力欄（Enter・フォーカスを外したときに、今の値と違えば新しい文字を返す）
fn text_field(
    ui: &mut Ui,
    bufs: &mut HashMap<String, String>,
    key: &str,
    name: &str,
    cur: String,
    width: f32,
) -> Option<String> {
    let buf = bufs.entry(key.to_string()).or_insert_with(|| cur.clone());
    let r = ui
        .add(egui::TextEdit::singleline(buf).desired_width(width))
        .accessible_name(name);
    if r.lost_focus() && buf.trim() != cur.trim() {
        return Some(buf.clone());
    }
    None
}

/// 面 j の設定の欄。read_only は変えられない理由（ロック・組合せの要素の設定で変える面など）
pub(crate) fn ui(ui: &mut Ui, st: &mut Inspector, j: usize, read_only: Option<String>) -> Vec<Act> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    let Some((dj, v)) = st.data.clone().filter(|(dj, _)| *dj == j) else {
        ui.weak(tr("Reading the surface…"));
        return acts;
    };
    let d = &v["data"];
    let typ = d["typ"].as_str().unwrap_or("Standard");
    let ztyp = zmx_type(typ);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(trf("Surface {} details", &[&dj])).strong());
        ui.label(RichText::new(ztyp).small().color(weak));
        if let Some(why) = &read_only {
            ui.label(RichText::new(why).color(ui.visuals().warn_fg_color));
        }
    });
    if read_only.is_some() {
        ui.disable();
    }
    let mut fields = Map::new();
    let mut data = Map::new();
    let mut err: Option<String> = None;
    let bufs = &mut st.bufs;
    egui::Grid::new(("surf_edit", dj))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            let label = |ui: &mut Ui, t: &str| {
                ui.label(RichText::new(t).color(weak));
            };
            // 非球面の係数（表のパラメータの列に出ない面タイプだけ）
            let in_table = crate::lens_data::par_count(ztyp) > 0;
            let asph_used = matches!(
                typ,
                "ZernikeStd"
                    | "ZernikeFringe"
                    | "GridSag"
                    | "User"
                    | "OddAsph"
                    | "Irregular"
                    | "TiltSurf"
            );
            if !in_table && (asph_used || d["asph"].as_array().is_some_and(|a| !a.is_empty())) {
                let name = tr("Asphere r², r⁴, …");
                label(ui, name);
                if let Some(t) = text_field(ui, bufs, "asph", name, nums_text(&d["asph"]), 260.0) {
                    match parse_nums(&t) {
                        Ok(a) => {
                            fields.insert("asph".into(), json!(a));
                        }
                        Err(e) => err = Some(e),
                    }
                }
                ui.end_row();
            }
            // JSON の項目（値のあるものと、その面タイプで使うもの）
            for (key, name) in JSON_ITEMS {
                let used = match *key {
                    "zern" => matches!(typ, "ZernikeStd" | "ZernikeFringe"),
                    "grid" => typ == "GridSag",
                    "bin2" => typ == "Binary2",
                    "cylf" => typ == "CylFresnel",
                    "grin" => matches!(typ, "Gradium" | "Gradient9"),
                    "user" => typ == "User",
                    _ => true,
                };
                if !used && d[*key].is_null() {
                    continue;
                }
                label(ui, tr(name));
                let cur = serde_json::to_string(&d[*key]).unwrap_or_default();
                if let Some(t) = text_field(ui, bufs, key, tr(name), cur, 320.0) {
                    match serde_json::from_str::<Value>(&t) {
                        Ok(x) => {
                            data.insert(key.to_string(), x);
                        }
                        Err(e) => err = Some(format!("{}: {e}", tr(name))),
                    }
                }
                ui.end_row();
            }
        });
    ui.label(
        RichText::new(tr(
            "Press Enter or click outside to apply. Aperture is \"None\" or {\"Circular\": {\"min\": 0, \"max\": 5, \"dx\": 0, \"dy\": 0}}; tilt/decenter is {\"d\": [dx, dy, tx, ty, tz], \"order\": 0} (after: \"None\", \"Reverse\" or {\"Tilt\": …})",
        ))
        .small()
        .color(weak),
    );

    // 全項目（JSON）
    ui.separator();
    egui::CollapsingHeader::new(tr("All fields (JSON)"))
        .id_salt(("surf_all", dj))
        .show(ui, |ui| {
            let mut pretty = d.clone();
            if let Some(o) = pretty.as_object_mut() {
                o.remove("stop");
                o.remove("mirror");
            }
            let text = st
                .all
                .get_or_insert_with(|| serde_json::to_string_pretty(&pretty).unwrap_or_default());
            ui.add(
                egui::TextEdit::multiline(text)
                    .code_editor()
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            )
            .accessible_name(tr("All fields (JSON)"));
            let (apply, reset) = ui
                .horizontal(|ui| {
                    (
                        ui.button(tr("Apply")).clicked(),
                        ui.button(tr("Revert")).clicked(),
                    )
                })
                .inner;
            if apply {
                match serde_json::from_str::<Value>(text) {
                    Ok(Value::Object(m)) => {
                        for (k, x) in m {
                            if pretty.get(&k) != Some(&x) {
                                data.insert(k, x);
                            }
                        }
                    }
                    Ok(_) => err = Some(tr("All fields must be {field: value}").into()),
                    Err(e) => err = Some(e.to_string()),
                }
            }
            if reset {
                st.all = None;
            }
        });

    if let Some(e) = err {
        acts.push(Act::Refused(e));
    }
    if !fields.is_empty() {
        acts.push(Act::Fields { j: dj, fields });
    }
    if !data.is_empty() {
        acts.push(Act::Data { j: dj, data });
    }
    acts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_lists() {
        assert_eq!(zmx_type("ZernikeStd"), "SZERNSAG");
        assert_eq!(zmx_type("Jones"), "JONESMAT");
        // 表の種類の候補はどれも面データの typ にある
        for (t, _) in crate::lens_data::TYPE_NAMES {
            assert!(TYP_NAMES.iter().any(|x| x.1 == *t), "{t}");
        }
        assert_eq!(parse_nums("1, 2.5 -3").unwrap(), vec![1.0, 2.5, -3.0]);
        assert!(parse_nums("1, x").is_err());
        assert_eq!(nums_text(&json!([0.0, 1e-6])), "0, 0.000001");
        assert_eq!(full(0.1 + 0.2), "0.30000000000000004");
    }
}
