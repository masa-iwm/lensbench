//! 引数・設定の一覧（名前・型・既定・説明）から作る入力欄。
//!
//! 一覧は serve の `analysis_args`（CLI の解析の引数、clap の定義から）か、プラグインの設定の項目
//! （`plugins` の `params`、[`lensbench_core::plugin::ParamSpec`]）と同じ形の JSON。欄の値は文字列で持ち
//! （並びの保存に入る）、空なら省く（解析・プラグインの既定）。計算のときに型に合わせて JSON の値にする。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::theme;

/// 1 つの引数・設定の項目
#[derive(Debug, Clone, PartialEq)]
pub struct ArgSpec {
    pub name: String,
    /// num・count・int・flag・str・choice・nums・path
    pub ty: String,
    pub default: Value,
    pub unit: String,
    pub doc: String,
    pub choices: Vec<String>,
    /// nums の個数の上限（2 なら 1 個で x・y 共通）
    pub len: Option<usize>,
    /// 旗を繰り返して複数渡す（値はカンマ区切り）
    pub repeat: bool,
}

impl ArgSpec {
    pub fn parse(v: &Value) -> Option<ArgSpec> {
        Some(ArgSpec {
            name: v["name"].as_str()?.to_string(),
            ty: v["type"].as_str().unwrap_or("str").to_string(),
            default: v["default"].clone(),
            unit: v["unit"].as_str().unwrap_or_default().to_string(),
            doc: v["doc"].as_str().unwrap_or_default().to_string(),
            choices: v["choices"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            len: v["len"].as_u64().map(|n| n as usize),
            repeat: v["repeat"].as_bool().unwrap_or(false),
        })
    }

    pub fn parse_list(v: &Value) -> Vec<ArgSpec> {
        v.as_array()
            .map(|a| a.iter().filter_map(ArgSpec::parse).collect())
            .unwrap_or_default()
    }

    /// 既定値を欄に書く形で（無ければ空）
    pub fn default_text(&self) -> String {
        value_text(&self.default)
    }

    /// 欄の文字列を JSON の値に（空なら None）
    pub fn value(&self, t: &str) -> Result<Option<Value>, String> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(None);
        }
        let bad = || trf("{}: 値 {} が読めない", &[&self.name, &t]);
        let num = |s: &str| -> Result<f64, String> {
            s.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(bad)
        };
        Ok(Some(match self.ty.as_str() {
            "flag" => match t {
                "true" => json!(true),
                "false" => json!(false),
                // プラグインの設定の旗は 0 か 1
                _ => match t.parse::<f64>() {
                    Ok(1.0) => json!(true),
                    Ok(0.0) => json!(false),
                    _ => return Err(bad()),
                },
            },
            "num" => json!(num(t)?),
            "count" => {
                let x = num(t)?;
                if x < 0.0 || x.fract() != 0.0 {
                    return Err(trf("{} は 0 以上の整数", &[&self.name]));
                }
                json!(x as u64)
            }
            "int" => {
                let x = num(t)?;
                if x.fract() != 0.0 {
                    return Err(trf("{} は整数", &[&self.name]));
                }
                json!(x as i64)
            }
            "nums" => {
                let v: Vec<f64> = split(t).map(num).collect::<Result<_, _>>()?;
                if v.is_empty() {
                    return Ok(None);
                }
                match self.len {
                    Some(n) if v.len() > n => {
                        return Err(trf("{} は {} 個まで", &[&self.name, &n]))
                    }
                    // x・y 共通（設定の JSON は 2 個が要る）
                    Some(2) if v.len() == 1 => Value::Array(vec![num_json(v[0]); 2]),
                    _ => Value::Array(v.into_iter().map(num_json).collect()),
                }
            }
            _ if self.repeat => json!(split(t).map(str::to_string).collect::<Vec<_>>()),
            _ => json!(t),
        }))
    }
}

/// 解析 `cmd` の引数の一覧（CLI の定義から。プロセスの中で 1 度だけ作る）
pub fn analysis_specs(cmd: &str) -> Vec<ArgSpec> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<ArgSpec>>>> = OnceLock::new();
    let mut c = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    c.entry(cmd.to_string())
        .or_insert_with(|| {
            lensbench_cli::arg_spec::analysis_args(cmd)
                .map(|v| ArgSpec::parse_list(&Value::Array(v)))
                .unwrap_or_default()
        })
        .clone()
}

/// 設定のオブジェクトの欄の項目（拡張の解析の [`crate::ext::ObjectArg`]）
pub fn object_specs(o: &crate::ext::ObjectArg) -> Vec<ArgSpec> {
    ArgSpec::parse_list(&Value::Array((o.specs)()))
}

/// 数を JSON に。整数の値は整数で（設定の JSON の整数の項目、画素の数など、が読めるように）
fn num_json(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// カンマ・空白で区切る
fn split(t: &str) -> impl Iterator<Item = &str> {
    t.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
}

/// JSON の値を欄の文字列に
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Bool(b) => b.to_string(),
        x => x.to_string(),
    }
}

/// 欄の値を名前 → JSON の値に（空の欄は省く）
pub fn to_map(
    specs: &[ArgSpec],
    vals: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, String> {
    let mut m = Map::new();
    for (k, t) in vals {
        // 一覧に無い名前（解析の種類を変えた前の欄など）は渡さない
        let Some(s) = specs.iter().find(|s| &s.name == k) else {
            continue;
        };
        if let Some(v) = s.value(t)? {
            // 既定が false の旗の false は省くのと同じ（CLI の旗は false を受けない）
            let def_true = s.default == json!(true) || s.default.as_f64() == Some(1.0);
            if v != json!(false) || s.ty != "flag" || def_true {
                m.insert(k.clone(), v);
            }
        }
    }
    Ok(m)
}

/// 「.」でつないだ名前を入れ子のオブジェクトに（`mla.pitch` → {"mla": {"pitch": …}}）
pub fn nest(m: &Map<String, Value>) -> Value {
    let mut root = Map::new();
    for (k, v) in m {
        let parts: Vec<&str> = k.split('.').collect();
        let mut cur = &mut root;
        for p in &parts[..parts.len() - 1] {
            cur = cur
                .entry(p.to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("入れ子はオブジェクト");
        }
        cur.insert(parts[parts.len() - 1].to_string(), v.clone());
    }
    Value::Object(root)
}

/// 入れ子のオブジェクトを「.」でつないだ名前 → 欄の文字列に（[`nest`] の逆）
pub fn flatten(v: &Value) -> BTreeMap<String, String> {
    fn walk(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    let name = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&name, x, out);
                }
            }
            Value::Null => {}
            x => {
                out.insert(prefix.to_string(), value_text(x));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk("", v, &mut out);
    out
}

/// 欄を並べる。`skip` の名前は出さない。値を変えたら true（文字の欄は入力を終えたとき）
pub fn form(
    ui: &mut Ui,
    salt: impl std::hash::Hash + Copy,
    specs: &[&ArgSpec],
    vals: &mut BTreeMap<String, String>,
) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let mut changed = false;
    egui::Grid::new(("argform", salt))
        .num_columns(3)
        .spacing([8.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for s in specs {
                let set = vals.get(&s.name).is_some_and(|t| !t.trim().is_empty());
                let mut label = RichText::new(&s.name).monospace();
                if set {
                    label = label.strong();
                }
                let tip = if s.unit.is_empty() {
                    s.doc.clone()
                } else {
                    format!("{} [{}]", s.doc, s.unit)
                };
                if tip.is_empty() {
                    ui.label(label);
                } else {
                    ui.label(label).on_hover_text(tip);
                }
                ui.horizontal(|ui| {
                    let t = vals.entry(s.name.clone()).or_default();
                    match s.ty.as_str() {
                        "flag" => {
                            let def = s.default == json!(true) || s.default.as_f64() == Some(1.0);
                            let mut b = match s.value(t) {
                                Ok(Some(v)) => v == json!(true),
                                _ => def,
                            };
                            if ui.checkbox(&mut b, "").changed() {
                                *t = if b == def {
                                    String::new()
                                } else {
                                    b.to_string()
                                };
                                changed = true;
                            }
                        }
                        "choice" => {
                            let def = s.default_text();
                            let shown = if t.trim().is_empty() {
                                if def.is_empty() {
                                    tr("（既定）").to_string()
                                } else {
                                    trf("{}（既定）", &[&def])
                                }
                            } else {
                                t.clone()
                            };
                            egui::ComboBox::from_id_salt(("argchoice", salt, &s.name))
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(t.is_empty(), tr("（既定）")).clicked()
                                    {
                                        t.clear();
                                        changed = true;
                                    }
                                    for c in &s.choices {
                                        if ui.selectable_label(t == c, c).clicked() {
                                            *t = if *c == def { String::new() } else { c.clone() };
                                            changed = true;
                                        }
                                    }
                                });
                        }
                        _ => {
                            let hint = match (s.default_text(), s.ty.as_str()) {
                                (d, _) if !d.is_empty() => d,
                                (_, "nums") => tr("a, b, …").to_string(),
                                _ => tr("自動").to_string(),
                            };
                            let w = if s.ty == "path" || s.ty == "str" || s.ty == "nums" {
                                180.0
                            } else {
                                90.0
                            };
                            let r = ui.add(
                                egui::TextEdit::singleline(t)
                                    .hint_text(hint)
                                    .desired_width(w),
                            );
                            let bad = s.value(t).is_err();
                            if bad {
                                ui.colored_label(ui.visuals().error_fg_color, "⚠")
                                    .on_hover_text(s.value(t).err().unwrap_or_default());
                            }
                            if r.lost_focus() && !bad {
                                changed = true;
                            }
                            if s.ty == "path" {
                                let start = ui
                                    .small_button("…")
                                    .on_hover_text(tr("ファイルを選ぶ"))
                                    .clicked();
                                // 欄ごとに区別する（ui.id() は兄弟の Ui で同じになる）
                                let key = format!(
                                    "arg-path-{:?}",
                                    egui::Id::new(("argpath", salt, &s.name))
                                );
                                if let Some(p) =
                                    crate::files::open(&key, start, crate::files::FileDialog::new)
                                {
                                    *t = p.display().to_string();
                                    changed = true;
                                }
                            }
                        }
                    }
                    if !s.unit.is_empty() {
                        ui.label(RichText::new(&s.unit).small().color(weak));
                    }
                });
                let reset = ui.add_enabled(
                    set,
                    egui::Button::new(RichText::new("↺").small()).frame(false),
                );
                if reset.on_hover_text(tr("既定に戻す")).clicked() {
                    vals.remove(&s.name);
                    changed = true;
                }
                ui.end_row();
            }
        });
    // 空の欄は持たない（並びの保存を小さく）
    vals.retain(|_, t| !t.trim().is_empty());
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, ty: &str, len: Option<usize>) -> ArgSpec {
        ArgSpec {
            name: name.into(),
            ty: ty.into(),
            default: Value::Null,
            unit: String::new(),
            doc: String::new(),
            choices: Vec::new(),
            len,
            repeat: false,
        }
    }

    #[test]
    fn values_by_type() {
        assert_eq!(
            spec("a", "num", None).value(" 2.5 ").unwrap(),
            Some(json!(2.5))
        );
        assert_eq!(spec("a", "count", None).value("3").unwrap(), Some(json!(3)));
        assert!(spec("a", "count", None).value("-1").is_err());
        assert!(spec("a", "num", None).value("x").is_err());
        assert_eq!(spec("a", "num", None).value("").unwrap(), None);
        assert_eq!(
            spec("a", "nums", None).value("10, 20.5 40").unwrap(),
            Some(json!([10, 20.5, 40]))
        );
        // 2 個の並びは 1 個で x・y 共通
        assert_eq!(
            spec("a", "nums", Some(2)).value("0.2").unwrap(),
            Some(json!([0.2, 0.2]))
        );
        assert!(spec("a", "nums", Some(2)).value("1,2,3").is_err());
        assert_eq!(
            spec("a", "flag", None).value("true").unwrap(),
            Some(json!(true))
        );
        assert_eq!(spec("a", "str", None).value("x").unwrap(), Some(json!("x")));
    }

    #[test]
    fn map_skips_unknown_and_false() {
        let specs = vec![spec("n", "count", None), spec("f", "flag", None)];
        let vals: BTreeMap<String, String> = [("n", "4"), ("f", "false"), ("old", "1")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let m = to_map(&specs, &vals).unwrap();
        assert_eq!(Value::Object(m), json!({ "n": 4 }));
    }

    #[test]
    fn nest_and_flatten_round_trip() {
        let mut m = Map::new();
        m.insert("mla.pitch".into(), json!([0.2, 0.2]));
        m.insert("detector.noise.shot".into(), json!(true));
        m.insert("reference".into(), json!("lenslet"));
        let v = nest(&m);
        assert_eq!(
            v,
            json!({ "mla": { "pitch": [0.2, 0.2] }, "detector": { "noise": { "shot": true } }, "reference": "lenslet" })
        );
        let f = flatten(&v);
        assert_eq!(f["mla.pitch"], "0.2, 0.2");
        assert_eq!(f["detector.noise.shot"], "true");
        assert_eq!(f["reference"], "lenslet");
    }
}
