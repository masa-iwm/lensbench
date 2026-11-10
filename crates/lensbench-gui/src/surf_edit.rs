//! 面の設定: 面の表で選んだ面の、すべての項目を編集する欄。
//!
//! 値は serve の `surface_data`（面のデータを全部の項目で、とソルブ・ピックアップ）で読む。よく使う項目は入力欄で、
//! 開口・面の前後の傾き偏心・面タイプの形のデータ（Zernike・Grid Sag・Binary 2・Cylinder Fresnel・GRIN・利用者定義の面）は JSON で、
//! 「全項目（JSON）」では面のデータをそのまま書き換えられる。書き換えは [`Act`] で GUI に返す。

use std::collections::HashMap;

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::lens_data::Act;
use crate::theme;

/// 面タイプ（.zmx の名前）
pub const TYPES: &[&str] = &[
    "STANDARD", "EVENASPH", "COORDBRK", "JONESMAT", "PARAXIAL", "TOROIDAL", "SZERNSAG", "FZERNSAG",
    "GRID_SAG", "FRESNELS", "CFRESNEL", "BINARY_2", "GRINSUR8", "GRINSUR9", "USER",
];

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
    /// 足すソルブ（種類、値、param、別の面）
    solve: (String, String, String, String),
    /// 足すピックアップ（量、元の面、倍率、オフセット）
    pick: (String, String, String, String),
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
fn bire_ui(ui: &mut Ui, dj: usize, cur: &Value) -> Option<Value> {
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

/// 面 j の設定の欄。read_only は変えられない理由（ロック・組合せの要素の設定で変える面など）、combo は組合せの系
/// （絞り・ソルブ・ピックアップは組合せの要素の surface_ops になる）
pub fn ui(
    ui: &mut Ui,
    st: &mut Inspector,
    j: usize,
    read_only: Option<String>,
    combo: bool,
) -> Vec<Act> {
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
        ui.label(RichText::new(trf("面 {} の設定", &[&dj])).strong());
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
    let mut ops: Vec<Value> = Vec::new();
    let mut err: Option<String> = None;
    let bufs = &mut st.bufs;
    egui::Grid::new(("surf_edit", dj))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            let label = |ui: &mut Ui, t: &str| {
                ui.label(RichText::new(t).color(weak));
            };
            // 種類
            label(ui, tr("種類"));
            let mut sel = ztyp.to_string();
            egui::ComboBox::from_id_salt(("surf_type", dj))
                .selected_text(&sel)
                .show_ui(ui, |ui| {
                    for t in TYPES {
                        ui.selectable_value(&mut sel, t.to_string(), *t);
                    }
                });
            if sel != ztyp {
                fields.insert("type".into(), json!(sel));
            }
            ui.end_row();
            // 絞り・無視
            label(ui, tr("絞り・無視"));
            ui.horizontal(|ui| {
                let stop = d["stop"] == true;
                let mut s = stop;
                // 組合せでは外すこともできる（前の絞り・絞りの要素に戻す）
                let r = ui.add_enabled(!stop || combo, egui::Checkbox::new(&mut s, tr("絞り")));
                if r.changed() {
                    if s {
                        ops.push(json!({ "op": "set_stop", "surface": dj }));
                    } else {
                        ops.push(json!({ "op": "unset_stop", "surface": dj }));
                    }
                }
                let mut ig = d["ignore"] == true;
                if ui.checkbox(&mut ig, tr("この面を無視")).changed() {
                    fields.insert("ignore".into(), json!(ig));
                }
            });
            ui.end_row();
            for (key, name, cur) in [
                (
                    "comment",
                    tr("コメント"),
                    d["comment"].as_str().unwrap_or_default().to_string(),
                ),
                (
                    "material",
                    tr("硝材"),
                    d["mat"].as_str().unwrap_or_default().to_string(),
                ),
                (
                    "coating",
                    tr("膜"),
                    d["coat"].as_str().unwrap_or_default().to_string(),
                ),
            ] {
                if key == "material" && typ == "CoordBrk" {
                    continue;
                }
                label(ui, name);
                if let Some(t) = text_field(ui, bufs, key, cur, 200.0) {
                    fields.insert(key.into(), json!(t.trim()));
                }
                ui.end_row();
            }
            // 数
            let c = num(&d["c"]).unwrap_or(0.0);
            let r = if c == 0.0 { f64::INFINITY } else { 1.0 / c };
            let mut items: Vec<(&str, &str, f64)> = Vec::new();
            if typ != "CoordBrk" {
                items.push(("r", tr("曲率半径"), r));
            }
            items.push(("t", tr("厚さ"), num(&d["t"]).unwrap_or(f64::NAN)));
            if typ != "CoordBrk" {
                items.push(("semi", tr("半径"), num(&d["semi"]).unwrap_or(0.0)));
                items.push(("k", tr("コーニック"), num(&d["k"]).unwrap_or(0.0)));
            }
            if typ == "Toroidal" {
                items.push(("cx", tr("x の曲率（回転）"), num(&d["cx"]).unwrap_or(0.0)));
            }
            if typ == "Paraxial" {
                items.push(("focal", tr("焦点距離"), num(&d["focal"]).unwrap_or(0.0)));
            }
            if typ != "CoordBrk" && d["mat"] != "MIRROR" {
                items.push(("dn", tr("屈折率のずれ dn"), num(&d["dn"]).unwrap_or(0.0)));
                items.push((
                    "disp",
                    tr("分散の変化 disp"),
                    num(&d["disp"]).unwrap_or(0.0),
                ));
            }
            if typ != "CoordBrk" {
                // 半径の自動（光線から求める）
                label(ui, tr("半径を自動"));
                let mut auto = d["semi_auto"] == true;
                if ui
                    .checkbox(&mut auto, "")
                    .on_hover_text(tr("半径を全視野・全波長の光線から求める"))
                    .changed()
                {
                    fields.insert("semi_auto".into(), json!(auto));
                }
                ui.end_row();
            }
            for (key, name, x) in items {
                label(ui, name);
                let cur = full(x);
                if let Some(t) = text_field(ui, bufs, key, cur, 120.0) {
                    let t = t.trim();
                    let val = if key == "r"
                        && (t.is_empty()
                            || t.eq_ignore_ascii_case("inf")
                            || t.eq_ignore_ascii_case("infinity"))
                    {
                        Some(0.0)
                    } else {
                        t.parse::<f64>().ok().filter(|v| v.is_finite())
                    };
                    match val {
                        Some(v) => {
                            fields.insert(key.into(), json!(v));
                        }
                        None => err = Some(trf("{} は数ではない", &[&t])),
                    }
                }
                ui.end_row();
            }
            // 並びの数
            let mut lists: Vec<(&str, String, Value, Option<usize>)> = Vec::new();
            if typ != "CoordBrk" {
                lists.push((
                    "asph",
                    tr("非球面 r², r⁴, …").into(),
                    d["asph"].clone(),
                    None,
                ));
            }
            if typ == "CoordBrk" {
                lists.push((
                    "cb",
                    tr("偏心 x y・傾き x y z").into(),
                    d["cb"].clone(),
                    Some(5),
                ));
            }
            if typ == "Jones" {
                lists.push((
                    "jones",
                    tr("Jones 行列（実・虚 ×4）").into(),
                    d["jones"].clone(),
                    Some(8),
                ));
            }
            for (key, name, cur, n) in lists {
                label(ui, &name);
                if let Some(t) = text_field(ui, bufs, key, nums_text(&cur), 260.0) {
                    match parse_nums(&t) {
                        Ok(a) if n.map_or(true, |n| a.len() == n) => {
                            fields.insert(key.into(), json!(a));
                        }
                        Ok(a) => {
                            err = Some(trf(
                                "{} は {} 個（{} 個）",
                                &[&name, &n.unwrap_or(0), &a.len()],
                            ))
                        }
                        Err(e) => err = Some(e),
                    }
                }
                ui.end_row();
            }
            if typ == "CoordBrk" {
                label(ui, tr("順序"));
                let mut o = d["cb_order"].as_u64().unwrap_or(0) != 0;
                if ui
                    .checkbox(&mut o, tr("傾けてから偏心（順序 1）"))
                    .changed()
                {
                    fields.insert("cb_order".into(), json!(if o { 1 } else { 0 }));
                }
                ui.end_row();
            }
            // 複屈折材の板の入口・出口（開いた系の標準面）
            if (!combo && typ == "Standard") || !d["bire"].is_null() {
                label(ui, tr("複屈折"));
                if let Some(b) = bire_ui(ui, dj, &d["bire"]) {
                    data.insert("bire".into(), b);
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

    // ソルブ・ピックアップ（組合せでは要素の surface_ops に）
    {
        ui.separator();
        ui.label(RichText::new(tr("ソルブ")).strong());
        for s in v["solves"].as_array().cloned().unwrap_or_default() {
            ui.horizontal(|ui| {
                let what = s["what"].as_str().unwrap_or("t");
                ui.label(format!(
                    "{} : {} {}",
                    what,
                    s["kind"].as_str().unwrap_or_default(),
                    fmt_num(num(&s["value"]).unwrap_or(0.0))
                ));
                if let Some(o) = s["other"].as_u64() {
                    ui.label(trf("（面 {}）", &[&o]));
                }
                if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
                    ops.push(json!({ "op": "remove_solve", "surface": dj, "what": what }));
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            let (kind, val, param, other) = &mut st.solve;
            if kind.is_empty() {
                *kind = "marginal_height".into();
            }
            egui::ComboBox::from_id_salt(("solve_kind", dj))
                .selected_text(kind.as_str())
                .show_ui(ui, |ui| {
                    for k in SOLVE_KINDS {
                        ui.selectable_value(kind, k.to_string(), *k);
                    }
                });
            ui.label(RichText::new(tr("値")).color(weak));
            ui.add(egui::TextEdit::singleline(val).desired_width(60.0));
            ui.label(RichText::new("param").color(weak));
            ui.add(egui::TextEdit::singleline(param).desired_width(50.0));
            ui.label(RichText::new(tr("別の面")).color(weak));
            ui.add(egui::TextEdit::singleline(other).desired_width(40.0));
            if ui.button(tr("＋ ソルブを足す")).clicked() {
                let mut op = json!({ "op": "add_solve", "surface": dj, "kind": kind });
                let f = |s: &str| s.trim().parse::<f64>().ok();
                if let Some(x) = f(val) {
                    op["value"] = json!(x);
                }
                if let Some(x) = f(param) {
                    op["param"] = json!(x);
                }
                if let Ok(o) = other.trim().parse::<usize>() {
                    op["other"] = json!(o);
                }
                ops.push(op);
            }
        });
        ui.label(RichText::new(tr("ピックアップ")).strong());
        for p in v["pickups"].as_array().cloned().unwrap_or_default() {
            ui.horizontal(|ui| {
                let what = p["what"].as_str().unwrap_or_default().to_string();
                ui.label(format!(
                    "{what} = {} × {} {} + {}",
                    fmt_num(num(&p["scale"]).unwrap_or(1.0)),
                    trf("面 {}", &[&p["src"]]),
                    p["from"].as_str().unwrap_or(&what),
                    fmt_num(num(&p["offset"]).unwrap_or(0.0)),
                ));
                if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
                    ops.push(json!({ "op": "remove_pickup", "dst": dj, "what": what }));
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            let (what, src, scale, offset) = &mut st.pick;
            ui.label(RichText::new(tr("量")).color(weak));
            ui.add(
                egui::TextEdit::singleline(what)
                    .hint_text("c r t k p1 semi …")
                    .desired_width(60.0),
            );
            ui.label(RichText::new(tr("元の面")).color(weak));
            ui.add(egui::TextEdit::singleline(src).desired_width(40.0));
            ui.label(RichText::new(tr("倍率")).color(weak));
            ui.add(
                egui::TextEdit::singleline(scale)
                    .hint_text("1")
                    .desired_width(50.0),
            );
            ui.label(RichText::new(tr("オフセット")).color(weak));
            ui.add(
                egui::TextEdit::singleline(offset)
                    .hint_text("0")
                    .desired_width(50.0),
            );
            if ui.button(tr("＋ ピックアップを足す")).clicked() {
                match src.trim().parse::<usize>() {
                    Ok(s) if !what.trim().is_empty() => {
                        let f = |x: &str, d: f64| x.trim().parse::<f64>().unwrap_or(d);
                        ops.push(json!({
                            "op": "add_pickup", "dst": dj, "src": s, "what": what.trim(),
                            "scale": f(scale, 1.0), "offset": f(offset, 0.0),
                        }));
                    }
                    _ => err = Some(tr("ピックアップには量と元の面の番号が要る").into()),
                }
            }
        });
    }

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
    if !ops.is_empty() {
        acts.push(Act::Ops(ops));
    }
    acts
}

/// ソルブの種類（serve の add_solve の kind）
const SOLVE_KINDS: &[&str] = &[
    "marginal_height",
    "marginal_height_zone",
    "chief_height",
    "pupil_position",
    "edge",
    "opd",
    "position",
    "compensator",
    "center_of_curvature",
    "marginal_angle",
    "chief_angle",
    "marginal_normal",
    "chief_normal",
    "aplanatic",
    "element_power",
    "concentric_surface",
    "concentric_radius",
    "f_number",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_lists() {
        assert_eq!(zmx_type("ZernikeStd"), "SZERNSAG");
        assert_eq!(zmx_type("Jones"), "JONESMAT");
        assert_eq!(TYPES.len(), TYP_NAMES.len());
        assert_eq!(parse_nums("1, 2.5 -3").unwrap(), vec![1.0, 2.5, -3.0]);
        assert!(parse_nums("1, x").is_err());
        assert_eq!(nums_text(&json!([0.0, 1e-6])), "0, 0.000001");
        assert_eq!(full(0.1 + 0.2), "0.30000000000000004");
        // 計算側のソルブの種類と同じ
        assert_eq!(SOLVE_KINDS, lensbench_core::edit::SOLVE_KINDS);
    }
}
