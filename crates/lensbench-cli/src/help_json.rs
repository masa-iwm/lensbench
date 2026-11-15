//! `lensbench help --json`（隠し）: clap の定義（解析の登録簿のサブコマンドも）を JSON で出す（見直し CLI-29）。
//!
//! マニュアルの CLI のページ（`scripts/gen_docs.py`）はこの JSON から作る。`--help` の文字を読み解かない。

use clap::{ArgAction, Command};
use serde_json::{json, Value};

/// `argv`（exe の名前を除く）が `help --json` か
pub fn wants(argv: &[std::ffi::OsString]) -> bool {
    argv.len() == 2 && argv[0] == "help" && argv[1] == "--json"
}

fn text(s: Option<&clap::builder::StyledStr>) -> Value {
    s.map_or(Value::Null, |x| Value::from(x.to_string()))
}

fn arg_json(a: &clap::Arg) -> Value {
    let flag = matches!(
        a.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    );
    let multiple = matches!(a.get_action(), ArgAction::Append)
        || a.get_num_args().is_some_and(|n| n.max_values() > 1);
    let value_name = a
        .get_value_names()
        .and_then(|v| v.first())
        .map(|x| x.to_string())
        .unwrap_or_else(|| a.get_id().as_str().to_string());
    json!({
        "id": a.get_id().as_str(),
        "long": a.get_long(),
        "short": a.get_short().map(|c| c.to_string()),
        "positional": a.is_positional(),
        "flag": flag,
        "value_name": if flag { Value::Null } else { Value::from(value_name) },
        "required": a.is_required_set(),
        "multiple": multiple,
        "global": a.is_global_set(),
        "default": a.get_default_values().iter().map(|x| x.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "choices": a.get_possible_values().iter().filter(|p| !p.is_hide_set()).map(|p| p.get_name().to_string()).collect::<Vec<_>>(),
        "help": text(a.get_help()),
        "long_help": text(a.get_long_help()),
    })
}

fn command_json(c: &mut Command) -> Value {
    let usage = c.render_usage().to_string();
    let usage = usage.strip_prefix("Usage: ").unwrap_or(&usage).to_string();
    let args: Vec<Value> = c
        .get_arguments()
        .filter(|a| {
            !a.is_hide_set()
                && !matches!(
                    a.get_action(),
                    ArgAction::Help
                        | ArgAction::HelpShort
                        | ArgAction::HelpLong
                        | ArgAction::Version
                )
        })
        .map(arg_json)
        .collect();
    // 一覧の順は --help と同じ（display_order、同じなら定義の順）
    let mut subs: Vec<&mut Command> = c
        .get_subcommands_mut()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
        .collect();
    subs.sort_by_key(|s| s.get_display_order());
    let subs: Vec<Value> = subs.into_iter().map(command_json).collect();
    json!({
        "name": c.get_name(),
        "about": text(c.get_about()),
        "long_about": text(c.get_long_about()),
        "usage": usage,
        "args": args,
        "subcommands": subs,
    })
}

/// help を `f` で訳す（about・long_about・引数の help・long_help、入れ子のサブコマンドも。XCT-20）
pub fn localize(mut c: clap::Command, f: &dyn Fn(&str) -> String) -> clap::Command {
    if let Some(t) = c.get_about().map(|x| f(&x.to_string())) {
        c = c.about(t);
    }
    if let Some(t) = c.get_long_about().map(|x| f(&x.to_string())) {
        c = c.long_about(t);
    }
    let ids: Vec<clap::Id> = c.get_arguments().map(|a| a.get_id().clone()).collect();
    for id in ids {
        c = c.mut_arg(id, |mut a| {
            if let Some(t) = a.get_help().map(|x| f(&x.to_string())) {
                a = a.help(t);
            }
            if let Some(t) = a.get_long_help().map(|x| f(&x.to_string())) {
                a = a.long_help(t);
            }
            a
        });
    }
    let subs: Vec<String> = c
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for n in subs {
        c = c.mut_subcommand(n, |s| localize(s, f));
    }
    c
}

/// `lensbench plugins` の doc（英語が元）を文字の出力では表示の言語に訳す。JSON は英語のまま（XCT-20）
pub fn localize_docs(mut v: Value, json: bool) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    match x {
                        Value::String(s) if k == "doc" => *s = lensbench_core::i18n::tr_doc(s),
                        _ => walk(x),
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    if !json {
        walk(&mut v);
    }
    v
}

/// clap の定義の JSON（版・終了コードの表も）。マニュアルの CLI のページは日本語なので help を表で訳す（訳した後の日本語はそのまま）
pub fn help_json(c: Command) -> Value {
    let mut c = localize(c, &lensbench_core::i18n::translate_doc);
    c.build();
    let mut v = command_json(&mut c);
    // exe の入口で足した解析（非公開版など、lensbench_engine::register）の印。非公開版のマニュアルは足した分だけ載せる
    let builtin = lensbench_engine::builtin_analyses();
    let all = crate::analyses();
    for s in v["subcommands"].as_array_mut().into_iter().flatten() {
        let n = s["name"].as_str().unwrap_or_default();
        s["ext"] = json!(all.get(n).is_some() && builtin.get(n).is_none());
    }
    v["version"] = json!(lensbench_core::VERSION);
    v["exit_codes"] = Value::from(
        crate::error::EXIT_CODES
            .iter()
            .map(|(code, m)| json!({ "code": code, "meaning": lensbench_core::i18n::translate_doc(m) }))
            .collect::<Vec<_>>(),
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_every_visible_subcommand_with_args() {
        let v = help_json(crate::cli_command());
        let subs = v["subcommands"].as_array().unwrap();
        let names: Vec<&str> = subs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        // 手書きのサブコマンドと登録簿の解析の両方
        for n in ["convert", "optimize", "serve", "spot", "mtf"] {
            assert!(names.contains(&n), "{n} が無い: {names:?}");
        }
        assert!(!names.contains(&"help"));
        assert_eq!(
            subs.iter().find(|s| s["name"] == "spot").unwrap()["ext"],
            json!(false)
        );
        let spot = subs.iter().find(|s| s["name"] == "spot").unwrap();
        let glass = spot["args"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["long"] == "glass-dir")
            .unwrap();
        assert_eq!(glass["multiple"], json!(true));
        assert!(glass["help"]
            .as_str()
            .unwrap()
            .contains("LENSBENCH_GLASSCAT"));
        let file = spot["args"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "file")
            .unwrap();
        assert_eq!(file["positional"], json!(true));
        assert!(spot["usage"]
            .as_str()
            .unwrap()
            .starts_with("lensbench spot"));
        assert!(v["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["long"] == "threads" && a["global"] == json!(true)));
        assert_eq!(v["exit_codes"][0]["code"], json!(0));
        // 書き出し先の --out は手書きのサブコマンドも登録簿の解析も -o でも渡せる（CLI-14）
        for s in subs {
            for a in s["args"].as_array().unwrap() {
                if a["long"] == "out" {
                    assert_eq!(a["short"], json!("o"), "{}", s["name"]);
                }
            }
        }
    }

    #[test]
    fn wants_only_help_json() {
        let a = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert!(wants(&a(&["help", "--json"])));
        assert!(!wants(&a(&["help"])));
        assert!(!wants(&a(&["spot", "--json"])));
    }
}
