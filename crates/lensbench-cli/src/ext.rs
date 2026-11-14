//! 拡張の解析を CLI から使う: 足した解析（[`lensbench_engine::register`]）は同梱のものと同じく CLI のサブコマンドになる。
//! [`run_cli`] はサブコマンドを 1 つ実行して結果の JSON を返す（試験・組み込み用）。

use serde_json::Value;

/// コマンドラインの引数（先頭は exe の名前）でサブコマンドを 1 つ実行し、結果の JSON を返す（試験・組み込み用。
/// 標準出力には出さない。--save は書く）
pub fn run_cli<I, T>(argv: I) -> Result<Value, String>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = crate::parse_cli(argv).map_err(|e| e.render().to_string())?;
    Ok(crate::execute(cli.cmd)?.1)
}

#[cfg(test)]
mod tests {
    use lensbench_analysis::{Analysis, Cx, Meta};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    /// 試しの選択
    #[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "lowercase")]
    enum Mode {
        #[default]
        A,
        B,
    }

    /// 試しの拡張の設定
    #[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
    #[serde(default, deny_unknown_fields)]
    struct ToyParams {
        /// 視野の番号（1 始まり、省略で全視野）
        field: Option<usize>,
        /// 試しの選択
        mode: Mode,
        /// 設定のオブジェクト
        #[schemars(extend("x-kind" = "json"))]
        opt: Option<Value>,
    }

    /// 試しの拡張: 系を読み、視野の数と引数、別に読んだ系の面の数を返す
    struct Toy;

    impl Analysis for Toy {
        type Params = ToyParams;
        type Output = Value;
        fn meta(&self) -> Meta {
            Meta {
                name: "toy-ext",
                about: "試しの拡張",
                help: "",
            }
        }
        fn run(&self, cx: &Cx<'_>, p: &ToyParams) -> lensbench_core::Result<Value> {
            let f = lensbench_analysis::defs::fields(cx.system, p.field)?;
            let mut notes = Vec::new();
            let other = cx.source()?.variant("1", &mut notes)?;
            Ok(json!({
                "fields": f, "mode": p.mode, "other": other.surfaces.len(),
                "opt": p.opt, "notes": notes,
            }))
        }
    }

    #[test]
    fn ext_analysis_runs_in_cli_serve_and_arg_spec() {
        lensbench_engine::register(Toy).unwrap();
        let zmx = format!("{ROOT}/tests/refs/A0/system.zmx");
        let glass = format!("{ROOT}/tests/glass");
        // CLI
        let v = super::run_cli([
            "lensbench",
            "toy-ext",
            &zmx,
            "--glass-dir",
            &glass,
            "--field",
            "2",
            "--mode",
            "b",
            "--opt",
            r#"{"x": 1}"#,
        ])
        .unwrap();
        assert_eq!(v["fields"], json!([1]));
        assert_eq!(v["mode"], "b");
        assert_eq!(v["opt"], json!({ "x": 1 }));
        assert!(v["other"].as_u64().unwrap() > 3);
        // 引数の誤りは clap の誤り
        assert!(super::run_cli(["lensbench", "toy-ext", &zmx, "--nope"]).is_err());
        // GUI の引数の欄: 選択は choice、オブジェクトは json、系の読み方の引数は出ない
        let a = lensbench_engine::params::analysis_args("toy-ext").unwrap();
        let m = a.iter().find(|x| x["name"] == "mode").unwrap();
        assert_eq!(m["type"], "choice");
        assert_eq!(m["choices"], json!(["a", "b"]));
        let o = a.iter().find(|x| x["name"] == "opt").unwrap();
        assert_eq!(o["type"], "json");
        assert!(a.iter().all(|x| x["name"] != "glass_dir"));
        // serve の analyze: 読み込んだ系と読み方
        let sv = lensbench_engine::Session::new();
        sv.call("load", &json!({ "file": zmx, "glass_dir": [glass] }), None)
            .unwrap();
        let r = sv
            .call(
                "analyze",
                &json!({ "cmd": "toy-ext", "field": 1, "opt": { "x": [1, 2] } }),
                None,
            )
            .unwrap();
        assert_eq!(r["fields"], json!([0]));
        assert_eq!(r["opt"], json!({ "x": [1, 2] }));
        // 同梱の解析・CLI のサブコマンドの名前は足せない
        assert!(lensbench_engine::register(Named("spot")).is_err());
        assert!(lensbench_engine::register(Named("optimize")).is_err());
    }

    /// 拡張の解析が使えない名前の一覧（engine の RESERVED）は、登録簿の外の CLI のサブコマンドとちょうど同じ
    #[test]
    fn reserved_names_match_cli_subcommands() {
        use clap::CommandFactory;
        let builtin = lensbench_engine::builtin_analyses();
        let mut cli: Vec<String> = crate::Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .filter(|n| builtin.get(n).is_none())
            .collect();
        cli.sort();
        let mut r: Vec<String> = lensbench_engine::registry::RESERVED
            .iter()
            .map(|x| x.to_string())
            .collect();
        r.sort();
        assert_eq!(cli, r);
    }

    /// 名前だけ違う試しの拡張
    struct Named(&'static str);

    impl Analysis for Named {
        type Params = ToyParams;
        type Output = Value;
        fn meta(&self) -> Meta {
            Meta {
                name: self.0,
                about: "",
                help: "",
            }
        }
        fn run(&self, _: &Cx<'_>, _: &ToyParams) -> lensbench_core::Result<Value> {
            Ok(Value::Null)
        }
    }
}
