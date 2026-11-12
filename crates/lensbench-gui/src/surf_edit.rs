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

use crate::i18n::{tr, trf};
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
    ("Gradium", "GRINSUR8"),
    ("Gradient9", "GRINSUR9"),
    ("User", "USER"),
];

/// 面データの typ を .zmx の名前に
pub fn zmx_type(typ: &str) -> &'static str {
    TYP_NAMES
        .iter()
        .find(|x| x.0 == typ)
        .map(|x| x.1)
        .unwrap_or("STANDARD")
}

/// JSON で書き換える面のデータの項目（名前、表示名）
const JSON_ITEMS: &[(&str, &str)] = &[
    ("ap", "開口"),
    ("before", "前の傾き偏心"),
    ("after", "後の傾き偏心"),
    ("zern", "Zernike の項"),
    ("grid", "Grid Sag の格子"),
    ("bin2", "Binary 2 の位相"),
    ("cylf", "Cylinder Fresnel"),
    ("grin", "屈折率分布（GRIN）"),
    ("user", "利用者定義の面"),
];

/// 面の設定の状態
#[derive(Debug, Clone, Default)]
pub struct Inspector {
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
    pub fn stale(&mut self) {
        self.data = None;
        self.asked = None;
    }

    pub fn set(&mut self, j: usize, v: Value) {
        self.data = Some((j, v));
        self.asked = None;
        self.bufs.clear();
        self.all = None;
    }
}

/// 複屈折材の板の欄（入口: mode・結晶軸、出口）。変えたら面のデータの bire の値（ne は材料「名前-E」から作り直す）
pub fn bire_ui(ui: &mut Ui, dj: usize, cur: &Value) -> Option<Value> {
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
            for (i, l) in [tr("なし"), tr("入口"), tr("出口")].iter().enumerate() {
                ui.selectable_value(&mut k, i, *l);
            }
        });
        let mut changed = k != kind;
        if k == 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("モード"));
                for (m, l) in [(2, tr("波長板（2）")), (0, tr("常光だけ（0）"))] {
                    changed |= ui.selectable_value(&mut mode, m, l).changed();
                }
                ui.label(tr("結晶軸"));
                for (i, a) in axis.iter_mut().enumerate() {
                    changed |= ui
                        .add(
                            egui::DragValue::new(a)
                                .speed(0.01)
                                .max_decimals(6)
                                .prefix(["x ", "y ", "z "][i]),
                        )
                        .changed();
                }
            });
            ui.label(
                RichText::new(trf(
                    "面 {} の後の材料が常光、材料名-E が異常光（カタログに要る）。出口の面と組で使う",
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

/// JSON の数（inf は {"$f": "inf"}）
fn num(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        return Some(x);
    }
    match v.get("$f").and_then(Value::as_str) {
        Some("inf") => Some(f64::INFINITY),
        Some("-inf") => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// 書き換え用の数の文字（丸めない）
pub fn full(x: f64) -> String {
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
                .map(|x| num(x).map(full).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// 「1, 2.5, -3」を数の並びに
pub fn parse_nums(s: &str) -> Result<Vec<f64>, String> {
    s.split([',', ' ', ';'])
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            x.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| trf("{} は数ではない", &[&x.trim()]))
        })
        .collect()
}

/// 入力欄（Enter・フォーカスを外したときに、今の値と違えば新しい文字を返す）
fn text_field(
    ui: &mut Ui,
    bufs: &mut HashMap<String, String>,
    key: &str,
    cur: String,
    width: f32,
) -> Option<String> {
    let buf = bufs.entry(key.to_string()).or_insert_with(|| cur.clone());
    let r = ui.add(egui::TextEdit::singleline(buf).desired_width(width));
    if r.lost_focus() && buf.trim() != cur.trim() {
        return Some(buf.clone());
    }
    None
}

/// 面 j の設定の欄。read_only は変えられない理由（ロック・組合せの要素の設定で変える面など）
pub fn ui(ui: &mut Ui, st: &mut Inspector, j: usize, read_only: Option<String>) -> Vec<Act> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    let Some((dj, v)) = st.data.clone().filter(|(dj, _)| *dj == j) else {
        ui.weak(tr("面の値を読んでいます…"));
        return acts;
    };
    let d = &v["data"];
    let typ = d["typ"].as_str().unwrap_or("Standard");
    let ztyp = zmx_type(typ);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(trf("面 {} の詳細設定", &[&dj])).strong());
        ui.label(RichText::new(ztyp).small().color(weak));
        if let Some(why) = &read_only {
            ui.label(RichText::new(why).color(ui.visuals().warn_fg_color));
        }
    });
    ui.label(
        RichText::new(tr(
            "種類・曲率半径・厚さ・硝材・膜・半径・コーニック・パラメータは表で。ソルブ・ピックアップはセルの右端の [...]、屈折率のずれ・分散・複屈折は硝材のセルの [...]、絞り・無視・挿入は右クリック（Shift+F10）で。",
        ))
        .small()
        .color(weak),
    );
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
                let name = tr("非球面 r², r⁴, …");
                label(ui, name);
                if let Some(t) = text_field(ui, bufs, "asph", nums_text(&d["asph"]), 260.0) {
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
                if let Some(t) = text_field(ui, bufs, key, cur, 320.0) {
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
            "Enter・欄の外のクリックで決める。開口は \"None\" か {\"Circular\": {\"min\": 0, \"max\": 5, \"dx\": 0, \"dy\": 0}}、傾き偏心は {\"d\": [dx, dy, tx, ty, tz], \"order\": 0}（後は \"None\"・\"Reverse\"・{\"Tilt\": …}）",
        ))
        .small()
        .color(weak),
    );

    // 全項目（JSON）
    ui.separator();
    egui::CollapsingHeader::new(tr("全項目（JSON）"))
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
            );
            let (apply, reset) = ui
                .horizontal(|ui| {
                    (
                        ui.button(tr("適用")).clicked(),
                        ui.button(tr("元に戻す")).clicked(),
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
                    Ok(_) => err = Some(tr("全項目は {項目: 値}").into()),
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
