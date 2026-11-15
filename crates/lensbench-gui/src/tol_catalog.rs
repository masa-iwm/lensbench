//! 公差の設定の `catalog_tols`（メーカーごとのカタログの玉の公差）の欄。メーカーごとに、玉の公差の項目の値と、
//! 外径・焦点距離・貼り合わせの条件つきの上書き（`rules`、前から順に、条件が合うものを当てる）を書く。
//! 形は lensbench_design::tolerance::catalog_rule。

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{n_, tr, trf};
use crate::json_view::fmt_num;

/// 玉の公差の項目（名前、表示名、単位）
pub const FIELDS: [(&str, &str, &str); 9] = [
    ("radius_rel", n_("Radius"), n_("Ratio")),
    ("focal_rel", n_("Focal length"), n_("Ratio")),
    ("thickness", n_("Center thickness"), "mm"),
    ("wedge", n_("Wedge"), "°"),
    ("dn", n_("Index"), ""),
    ("disp", n_("Dispersion"), n_("Ratio")),
    ("decenter", n_("Decenter"), "mm"),
    ("tilt", n_("Tilt"), "°"),
    ("space", n_("Space after"), "mm"),
];

/// 下限・上限の比べ方
const LO_OPS: [&str; 2] = ["ge", "gt"];
const HI_OPS: [&str; 2] = ["lt", "le"];

/// 条件つきの上書き（rules の 1 つ）。空の欄は条件・項目なし
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rule {
    /// 外径 [mm] の下限（比べ方 ge・gt）と上限（lt・le）
    pub dia_lo: (String, String),
    pub dia_hi: (String, String),
    /// 焦点距離の絶対値 [mm] の下限・上限
    pub efl_lo: (String, String),
    pub efl_hi: (String, String),
    /// 貼り合わせ: None は問わない
    pub cemented: Option<bool>,
    pub fields: [String; 9],
}

/// メーカー 1 つ
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VendorTol {
    /// メーカー名（ブロックのメーカー名に含まれれば合う。"*" はほか）
    pub vendor: String,
    /// 組の名前（commercial など）か "none"（公差なし）。空なら下の項目
    pub preset: String,
    pub fields: [String; 9],
    pub rules: Vec<Rule>,
}

fn num_text(v: &Value) -> String {
    v.as_f64().map(fmt_num).unwrap_or_default()
}

fn put_fields(o: &mut Map<String, Value>, f: &[String; 9], what: &str) -> Result<(), String> {
    for (i, (k, _, _)) in FIELDS.iter().enumerate() {
        if let Some(x) = crate::widgets::parse_num(&f[i], &format!("{what} {k}"))? {
            o.insert(k.to_string(), num_val(x));
        }
    }
    Ok(())
}

/// 整数の値は整数で書く（読んだファイルと同じ形に戻す）
pub fn num_val(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// 欄から catalog_tols の JSON（メーカーが無ければ None）
pub fn to_json(v: &[VendorTol]) -> Result<Option<Value>, String> {
    let mut out = Map::new();
    for t in v {
        let name = t.vendor.trim();
        if name.is_empty() {
            continue;
        }
        if t.preset == "none" {
            out.insert(name.into(), Value::Null);
            continue;
        }
        if !t.preset.is_empty() {
            out.insert(name.into(), json!(t.preset));
            continue;
        }
        let mut o = Map::new();
        put_fields(&mut o, &t.fields, name)?;
        let mut rules = Vec::new();
        for (i, r) in t.rules.iter().enumerate() {
            let what = format!("{name} rules[{i}]");
            let mut ro = Map::new();
            for (q, (op, val)) in [
                ("dia", &r.dia_lo),
                ("dia", &r.dia_hi),
                ("efl", &r.efl_lo),
                ("efl", &r.efl_hi),
            ] {
                if let Some(x) = crate::widgets::parse_num(val, &format!("{what} {q}"))? {
                    ro.insert(format!("{q}_{op}"), num_val(x));
                }
            }
            if let Some(c) = r.cemented {
                ro.insert("cemented".into(), json!(c));
            }
            put_fields(&mut ro, &r.fields, &what)?;
            rules.push(Value::Object(ro));
        }
        if !rules.is_empty() {
            o.insert("rules".into(), Value::Array(rules));
        }
        out.insert(name.into(), Value::Object(o));
    }
    Ok((!out.is_empty()).then_some(Value::Object(out)))
}

fn fields_of(m: &Map<String, Value>, what: &str) -> Result<[String; 9], String> {
    let mut f: [String; 9] = Default::default();
    for (k, x) in m {
        let i = FIELDS
            .iter()
            .position(|(n, _, _)| n == k)
            .ok_or_else(|| format!("{what}: {k}"))?;
        if !x.is_number() {
            return Err(format!("{what}: {k}"));
        }
        f[i] = num_text(x);
    }
    Ok(f)
}

/// catalog_tols の JSON から欄へ。欄で書けない形（同じ量の同じ向きの条件が 2 つなど）は Err（設定にそのまま残す）
pub fn from_json(v: &Value) -> Result<Vec<VendorTol>, String> {
    let m = v.as_object().ok_or("catalog_tols")?;
    let mut out = Vec::new();
    for (name, x) in m {
        let mut t = VendorTol {
            vendor: name.clone(),
            ..Default::default()
        };
        match x {
            Value::Null => t.preset = "none".into(),
            Value::String(s) => t.preset = s.clone(),
            Value::Object(o) => {
                let mut plain = o.clone();
                let rules = plain.remove("rules");
                t.fields = fields_of(&plain, name)?;
                for r in rules
                    .as_ref()
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let ro = r.as_object().ok_or(name.as_str())?;
                    let mut rule = Rule::default();
                    let mut rest = Map::new();
                    for (k, y) in ro {
                        if k == "cemented" {
                            rule.cemented = Some(y.as_bool().ok_or(k.as_str())?);
                            continue;
                        }
                        let slot = match k.split_once('_') {
                            Some(("dia", op)) if LO_OPS.contains(&op) => {
                                Some((&mut rule.dia_lo, op))
                            }
                            Some(("dia", op)) if HI_OPS.contains(&op) => {
                                Some((&mut rule.dia_hi, op))
                            }
                            Some(("efl", op)) if LO_OPS.contains(&op) => {
                                Some((&mut rule.efl_lo, op))
                            }
                            Some(("efl", op)) if HI_OPS.contains(&op) => {
                                Some((&mut rule.efl_hi, op))
                            }
                            _ => None,
                        };
                        match slot {
                            Some((s, op)) => {
                                if !s.1.is_empty() {
                                    return Err(format!("{name}: {k}"));
                                }
                                *s = (op.to_string(), num_text(y));
                            }
                            None => {
                                rest.insert(k.clone(), y.clone());
                            }
                        }
                    }
                    rule.fields = fields_of(&rest, name)?;
                    t.rules.push(rule);
                }
            }
            _ => return Err(name.clone()),
        }
        out.push(t);
    }
    Ok(out)
}

fn cell(ui: &mut Ui, s: &mut String) {
    ui.add(
        egui::TextEdit::singleline(s)
            .desired_width(42.0)
            .hint_text("—"),
    );
}

fn bound(ui: &mut Ui, id: impl std::hash::Hash, b: &mut (String, String), ops: &[&str; 2]) {
    if b.0.is_empty() {
        b.0 = ops[0].into();
    }
    let sym = |o: &str| match o {
        "ge" => "≥",
        "gt" => ">",
        "lt" => "<",
        _ => "≤",
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(sym(&b.0))
        .width(28.0)
        .show_ui(ui, |ui| {
            for o in ops {
                ui.selectable_value(&mut b.0, o.to_string(), sym(o));
            }
        });
    cell(ui, &mut b.1);
}

/// 欄を描く。presets は組の（名前, 表示名）
pub fn ui(
    ui: &mut Ui,
    v: &mut Vec<VendorTol>,
    presets: &[(&str, &'static str)],
    weak: egui::Color32,
) {
    ui.label(
        RichText::new(tr(
            "Catalog lens tolerances by vendor (catalog_tols). A name matches if the block's vendor contains it (case-insensitive; * is any other). Empty cells use the defaults. Condition rows apply in order; each match overrides the values.",
        ))
        .small()
        .color(weak),
    );
    let mut del = None;
    for (i, t) in v.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut t.vendor)
                    .desired_width(90.0)
                    .hint_text(tr("Vendor")),
            );
            egui::ComboBox::from_id_salt(("tc_preset", i))
                .selected_text(if t.preset.is_empty() {
                    tr("By item").to_string()
                } else {
                    presets
                        .iter()
                        .find(|p| p.0 == t.preset)
                        .map_or(t.preset.clone(), |p| tr(p.1).to_string())
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut t.preset, String::new(), tr("By item"));
                    for (k, l) in presets.iter().filter(|p| !p.0.is_empty()) {
                        ui.selectable_value(&mut t.preset, k.to_string(), tr(l));
                    }
                });
            if ui.small_button("✖").on_hover_text(tr("Delete")).clicked() {
                del = Some(i);
            }
        });
        if !t.preset.is_empty() {
            continue;
        }
        let mut rdel = None;
        egui::Grid::new(("tc_grid", i))
            .num_columns(FIELDS.len() + 2)
            .spacing([4.0, 2.0])
            .show(ui, |ui| {
                ui.label("");
                for (_, l, u) in FIELDS {
                    ui.label(
                        RichText::new(if u.is_empty() {
                            tr(l).to_string()
                        } else {
                            format!("{} [{}]", tr(l), tr(u))
                        })
                        .small(),
                    );
                }
                ui.end_row();
                ui.label(tr("Default"));
                for f in t.fields.iter_mut() {
                    cell(ui, f);
                }
                ui.end_row();
                for (j, r) in t.rules.iter_mut().enumerate() {
                    ui.label(trf("Condition {}", &[&(j + 1)]));
                    for f in r.fields.iter_mut() {
                        cell(ui, f);
                    }
                    if ui.small_button("✖").clicked() {
                        rdel = Some(j);
                    }
                    ui.end_row();
                }
            });
        for (j, r) in t.rules.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(trf("Condition {}", &[&(j + 1)]))
                        .small()
                        .color(weak),
                );
                ui.label(tr("Diameter"));
                bound(ui, ("tc_dl", i, j), &mut r.dia_lo, &LO_OPS);
                bound(ui, ("tc_dh", i, j), &mut r.dia_hi, &HI_OPS);
                ui.label(tr("|Focal length|"));
                bound(ui, ("tc_el", i, j), &mut r.efl_lo, &LO_OPS);
                bound(ui, ("tc_eh", i, j), &mut r.efl_hi, &HI_OPS);
                egui::ComboBox::from_id_salt(("tc_cem", i, j))
                    .selected_text(match r.cemented {
                        None => tr("Any lens type"),
                        Some(true) => tr("Cemented"),
                        Some(false) => tr("Singlet"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut r.cemented, None, tr("Any lens type"));
                        ui.selectable_value(&mut r.cemented, Some(true), tr("Cemented"));
                        ui.selectable_value(&mut r.cemented, Some(false), tr("Singlet"));
                    });
            });
        }
        if let Some(j) = rdel {
            t.rules.remove(j);
        }
        if ui.small_button(tr("＋ Condition")).clicked() {
            t.rules.push(Rule::default());
        }
    }
    if let Some(i) = del {
        v.remove(i);
    }
    if ui.button(tr("＋ Vendor")).clicked() {
        v.push(VendorTol::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rules_apply() {
        let v = json!({
            "Edmund": { "thickness": 0.1, "focal_rel": 0.01,
                "rules": [{ "dia_lt": 10, "focal_rel": 0.02 }, { "dia_ge": 12, "dia_le": 25, "cemented": true, "wedge": 0.05 }] },
            "Thorlabs": "precision",
            "*": null,
        });
        let t = from_json(&v).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(to_json(&t).unwrap().unwrap(), v);
        // 公差の計算が読める（条件が当たる）
        let base = lensbench_design::montecarlo::LensTol::preset("commercial").unwrap();
        let g = lensbench_design::tolerance::LensGeom {
            dia: 8.0,
            efl: 50.0,
            glasses: 1,
        };
        let r = lensbench_design::tolerance::catalog_rule(&v["Edmund"], base, g, "Edmund")
            .unwrap()
            .unwrap();
        assert_eq!((r.thickness, r.focal_rel), (0.1, 0.02));
        // 同じ量の同じ向きの条件が 2 つは欄で書けない
        assert!(from_json(&json!({ "A": { "rules": [{ "dia_lt": 5, "dia_le": 6 }] } })).is_err());
        // 数でない値は誤り
        let bad = vec![VendorTol {
            vendor: "X".into(),
            fields: std::array::from_fn(|i| if i == 0 { "abc".into() } else { String::new() }),
            ..Default::default()
        }];
        assert!(to_json(&bad).is_err());
    }
}
