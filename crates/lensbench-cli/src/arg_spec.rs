//! 解析のサブコマンドの引数の一覧（名前・型・既定・説明）を clap の定義から作る（GUI の設定欄）。
//!
//! 形はプラグインの設定の項目（[`lensbench_core::plugin::ParamSpec`]）の JSON と同じ:
//! `{"name", "type", "default", "unit", "doc", "choices"?, "len"?, "repeat"?}`。`type` は `num`（実数）・`count`
//! （0 以上の整数）・`int`・`flag`（旗）・`str`・`choice`・`nums`（数のカンマ区切り）・`path`。`repeat` は
//! 旗を繰り返して複数渡せる引数（値は並び）。`min` は数の下限（0 で結果が空になるリング数など）。`required` は省けない引数、`conflicts` は一緒に使えない引数の名前の並び。

use std::any::TypeId;
use std::path::PathBuf;

use clap::ArgAction;
use lensbench_core::plugin::{ParamSpec, ParamType, Value as PVal};
use serde_json::{json, Value};

/// serve の analyze が系の読み方として決める引数（load で選ぶ）と、GUI に出さない出力の形
const FIXED: &[&str] = &[
    "glass_dir",
    "lens_catalog",
    "cfg",
    "state",
    "path",
    "json",
    "threads",
    "help",
    "version",
];

#[derive(Clone, Copy, PartialEq)]
enum Hint {
    /// 文字列で受けるが値は決まったもののどれか
    Choice(&'static [&'static str]),
    /// 文字列で受ける数のカンマ区切り
    Nums,
}

/// clap では文字列で受ける引数の型（サブコマンド（* は全部）, 引数の名前（- は _）, 型）
const HINTS: &[(&str, &str, Hint)] = &[
    ("*", "freqs", Hint::Nums),
    ("*", "kind", Hint::Choice(&["standard", "fringe"])),
    ("rms", "vs", Hint::Choice(&["field", "focus", "wavelength"])),
    ("*", "axis", Hint::Choice(&["y", "x"])),
    ("rms", "ref", Hint::Choice(&["centroid", "chief"])),
    ("fringe", "mode", Hint::Choice(&["pupil", "detector"])),
    (
        "fringe",
        "quantity",
        Hint::Choice(&[
            "intensity",
            "delta",
            "opd",
            "visibility",
            "sample",
            "reference",
        ]),
    ),
];

/// 下限のある数の引数（サブコマンド（* は全部）, 引数の名前, 下限）。0 では結果が空になるもの
const MINS: &[(&str, &str, f64)] = &[
    ("*", "rings", 1.0),
    ("*", "grid", 1.0),
    ("*", "freq_steps", 1.0),
    ("*", "pupil", 1.0),
    ("*", "image", 1.0),
    ("*", "points", 1.0),
    ("*", "steps", 1.0),
    ("*", "rays", 1.0),
];

/// サブコマンド `cmd` の引数（位置引数・系の読み方の引数を除く）。`cmd` が無ければ None
pub fn analysis_args(cmd: &str) -> Option<Vec<Value>> {
    let root = crate::cli_command();
    let sub = root.get_subcommands().find(|c| c.get_name() == cmd)?;
    let ext_choices = crate::ext::get(cmd).map(|e| e.choices).unwrap_or_default();
    let mut out = Vec::new();
    let mut prev_doc = String::new();
    for a in sub.get_arguments() {
        let id = a.get_id().as_str();
        if a.is_positional() || a.is_hide_set() || a.is_global_set() || FIXED.contains(&id) {
            continue;
        }
        let Some(long) = a.get_long() else { continue };
        let key = long.replace('-', "_");
        let vp = a.get_value_parser();
        let tid = vp.type_id();
        let repeat = matches!(a.get_action(), ArgAction::Append);
        let delim = a.get_value_delimiter().is_some();
        let flag = matches!(a.get_action(), ArgAction::SetTrue);
        let mut choices: Vec<String> = if flag {
            Vec::new()
        } else {
            a.get_possible_values()
                .iter()
                .filter(|p| !p.is_hide_set())
                .map(|p| p.get_name().to_string())
                .collect()
        };
        let is = |t: TypeId| tid == t;
        let hint = HINTS
            .iter()
            .find(|h| (h.0 == "*" || h.0 == cmd) && h.1 == key)
            .map(|h| h.2)
            .or_else(|| {
                ext_choices
                    .iter()
                    .find(|c| c.0 == key)
                    .map(|c| Hint::Choice(c.1))
            });
        if let Some(Hint::Choice(c)) = hint {
            choices = c.iter().map(|x| x.to_string()).collect();
        }
        let mut ty = if flag {
            "flag"
        } else if !choices.is_empty() {
            "choice"
        } else if is(TypeId::of::<f64>()) || is(TypeId::of::<f32>()) {
            if delim {
                "nums"
            } else {
                "num"
            }
        } else if is(TypeId::of::<usize>()) || is(TypeId::of::<u32>()) || is(TypeId::of::<u64>()) {
            if delim {
                "nums"
            } else {
                "count"
            }
        } else if is(TypeId::of::<i64>()) || is(TypeId::of::<i32>()) {
            "int"
        } else if is(TypeId::of::<PathBuf>()) {
            "path"
        } else {
            "str"
        };
        if hint == Some(Hint::Nums) {
            ty = "nums";
        }
        let defaults: Vec<String> = a
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let default = match (ty, defaults.as_slice()) {
            (_, []) => Value::Null,
            ("flag", _) => Value::Null,
            ("num" | "count" | "int", [d]) => d
                .parse::<f64>()
                .ok()
                .and_then(|x| serde_json::Number::from_f64(x).map(Value::Number))
                .map(|v| match (ty, v.as_f64()) {
                    ("count" | "int", Some(x)) => json!(x as i64),
                    _ => v,
                })
                .unwrap_or_else(|| json!(d)),
            (_, [d]) => json!(d),
            (_, v) => json!(v.join(",")),
        };
        // 説明を前の引数とまとめて書いたもの（「試料・参照の…」）は前の説明を使う
        let help = match a.get_help() {
            Some(h) => h.to_string().replace('\n', " "),
            None => prev_doc.clone(),
        };
        prev_doc = help.clone();
        let mut v = json!({
            // serve の analyze はキーの _ を - にするので、どちらで渡してもよい
            "name": key,
            "type": ty,
            "default": default,
            "unit": "",
            "doc": help,
        });
        if !choices.is_empty() {
            v["choices"] = json!(choices);
        }
        if matches!(ty, "num" | "count" | "int") {
            if let Some(m) = MINS
                .iter()
                .find(|m| (m.0 == "*" || m.0 == cmd) && m.1 == key)
            {
                v["min"] = json!(m.2);
            }
        }
        if repeat {
            v["repeat"] = json!(true);
        }
        if a.is_required_set() {
            v["required"] = json!(true);
        }
        let conflicts: Vec<String> = sub
            .get_arg_conflicts_with(a)
            .iter()
            .filter_map(|c| c.get_long())
            .map(|l| l.replace('-', "_"))
            .collect();
        if !conflicts.is_empty() {
            v["conflicts"] = json!(conflicts);
        }
        out.push(v);
    }
    Some(out)
}

/// 登録したプラグイン `name` の設定の項目（[`param_json`] の並び）。プラグインが無ければ None
pub fn plugin_params(name: &str) -> Option<Vec<Value>> {
    let reg = crate::builtin_plugins();
    let p = reg.get(name).ok()?;
    Some(p.params().iter().map(param_json).collect())
}

/// プラグインの設定の項目を JSON に（[`analysis_args`] と同じ形）
pub fn param_json(p: &ParamSpec) -> Value {
    let val = |v: &PVal| match v {
        PVal::Num(x) => json!(x),
        PVal::Str(s) => json!(s),
        PVal::Nums(v) => json!(v),
    };
    let mut v = json!({
        "name": p.name,
        "type": p.ty.as_str(),
        "default": p.default.as_ref().map(val).unwrap_or(Value::Null),
        "unit": p.unit,
        "doc": p.doc,
    });
    match &p.ty {
        ParamType::Choice(c) => v["choices"] = json!(c),
        ParamType::Nums { len: Some(n) } => v["len"] = json!(n),
        _ => {}
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtf_args_have_types() {
        let v = analysis_args("mtf").unwrap();
        let get = |n: &str| v.iter().find(|x| x["name"] == n).cloned();
        assert!(get("glass_dir").is_none() && get("file").is_none());
        assert!(get("field").is_some());
        assert!(analysis_args("no-such").is_none());
        // 一緒に使えない旗・省けない引数
        let c = get("fft").unwrap()["conflicts"].clone();
        assert!(c.as_array().unwrap().contains(&json!("interp")), "{c}");
        assert!(get("freqs").unwrap().get("required").is_none());
        let mf = analysis_args("mtf-focus").unwrap();
        let f = mf.iter().find(|x| x["name"] == "freqs").unwrap();
        assert_eq!(f["required"], true);
    }

    /// GUI に並べる解析はどれも引数の一覧が引け、型の補い（HINTS）はどれも実在の引数に当たる
    #[test]
    fn hints_match_real_args() {
        let cmds = [
            "spot",
            "mtf",
            "wavefront",
            "zernike",
            "rms",
            "psf",
            "geo-mtf",
            "huygens-mtf",
            "mtf-field",
            "mtf-focus",
            "fringe",
            "info",
            "elements",
            "ghost",
            "mech-check",
        ];
        let all: Vec<(String, Value)> = cmds
            .iter()
            .flat_map(|c| {
                analysis_args(c)
                    .unwrap_or_else(|| panic!("{c}"))
                    .into_iter()
                    .map(move |a| (c.to_string(), a))
            })
            .collect();
        for (c, n, _) in HINTS {
            assert!(
                all.iter()
                    .any(|(k, a)| (*c == "*" || c == k) && a["name"] == *n),
                "{c} {n}"
            );
        }
        for (_, a) in &all {
            let t = a["type"].as_str().unwrap();
            assert!(["num", "count", "int", "flag", "str", "choice", "nums", "path"].contains(&t));
            assert_eq!(t == "choice", a.get("choices").is_some(), "{a}");
        }
        let rms = analysis_args("rms").unwrap();
        let r = rms.iter().find(|a| a["name"] == "ref").unwrap();
        assert_eq!(r["default"], "centroid");
        // 説明を前の引数とまとめて書いたものは前の説明
        let fr = analysis_args("fringe").unwrap();
        let x = fr.iter().find(|a| a["name"] == "xtilt").unwrap();
        let y = fr.iter().find(|a| a["name"] == "ytilt").unwrap();
        assert_eq!(x["doc"], y["doc"]);
    }
}
