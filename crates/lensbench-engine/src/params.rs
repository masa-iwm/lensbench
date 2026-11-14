//! 解析・プラグインの設定の項目の一覧（名前・型・既定・説明。GUI の設定欄、serve の `analysis_args`）。
//!
//! 形はプラグインの設定の項目（[`lensbench_core::plugin::ParamSpec`]）の JSON と同じ:
//! `{"name", "type", "default", "unit", "doc", "choices"?, "len"?, "min"?, "required"?}`。`type` は `num`（実数）・`count`
//! （0 以上の整数）・`int`・`flag`（旗）・`str`・`choice`・`nums`（数の並び）・`path`・`paths`・`json`。

use lensbench_core::plugin::{ParamSpec, ParamType, Value as PVal};
use serde_json::{json, Value};

/// analyze の解析 `cmd`（解析の登録簿の解析と膜の境界 coatings）の設定の項目。`cmd` が無ければ None
pub fn analysis_args(cmd: &str) -> Option<Vec<Value>> {
    if cmd == "coatings" {
        // 膜の境界は登録簿の外（serve の analyze が読み込んだ系で直に求める）。受けるのは table だけ
        return Some(vec![json!({
            "name": "table",
            "type": "path",
            "default": null,
            "unit": "",
            "doc": "膜の実測表（LBCT 形式、複数可）",
            "repeat": true,
        })]);
    }
    let a = crate::analyses().get(cmd).cloned()?;
    Some(a.params().iter().map(|p| p.to_json()).collect())
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
        // 求め方は選択肢、省けない引数
        assert_eq!(
            get("method").unwrap()["choices"],
            json!(["auto", "interp", "direct", "fft"])
        );
        assert!(get("freqs").unwrap().get("required").is_none());
        let mf = analysis_args("mtf-focus").unwrap();
        let f = mf.iter().find(|x| x["name"] == "freqs").unwrap();
        assert_eq!(f["required"], true);
    }

    /// GUI に並べる解析（layout::ANALYSES と同じ並び）はどれも設定の項目が引け、型は決まったもののどれか
    #[test]
    fn gui_analyses_have_args() {
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
            "pop",
            "rays",
            "coatings",
        ];
        for c in cmds {
            for a in analysis_args(c).unwrap_or_else(|| panic!("{c}")) {
                let t = a["type"].as_str().unwrap();
                assert!(
                    [
                        "num", "count", "int", "flag", "str", "choice", "nums", "path", "paths",
                        "json"
                    ]
                    .contains(&t),
                    "{c} {a}"
                );
                assert_eq!(t == "choice", a.get("choices").is_some(), "{a}");
            }
        }
        let c = analysis_args("coatings").unwrap();
        assert_eq!(c[0]["name"], "table");
        assert_eq!(c[0]["repeat"], true);
        let rms = analysis_args("rms").unwrap();
        let r = rms.iter().find(|a| a["name"] == "reference").unwrap();
        assert_eq!(r["default"], "centroid");
    }
}
