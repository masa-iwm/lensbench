//! 拡張の解析: この CLI を組み込んだ exe（非公開版など）が足す解析（解析の登録簿の [`lensbench_analysis::Analysis`]）。
//!
//! 足した解析は、同梱のものと同じく CLI のサブコマンド・`serve` の `analyze`・`watch`・GUI の解析（GUI 側の見せ方の登録は
//! `lensbench_gui::ext`）で使える。引数は設定の型のスキーマから作り、系の読み方（別の構成・状態の系、プロジェクトファイル）は
//! [`lensbench_analysis::Cx::source`] で引く。exe の入口で [`register`] してから [`crate::main`] を呼ぶ。

use std::sync::{Arc, RwLock};

use lensbench_analysis::{Analyses, Analysis};
use serde_json::Value;

static EXTRA: RwLock<Option<Analyses>> = RwLock::new(None);

/// 同梱の解析と足した解析を並べた登録簿（足すと作り直す）
static ALL: RwLock<Option<Arc<Analyses>>> = RwLock::new(None);

/// 解析を足す（同じ名前は置き換える）。同梱のサブコマンド・解析と同じ名前は誤り
pub fn register<A: Analysis>(a: A) -> Result<(), String> {
    let name = a.meta().name;
    if crate::builtin_names().iter().any(|n| n == name) {
        return Err(format!("解析 {name} は同梱のものと同じ名前"));
    }
    EXTRA
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Analyses::default)
        .add(a);
    *ALL.write().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// 解析の登録簿（同梱の解析と、足した拡張の解析）
pub(crate) fn analyses() -> Arc<Analyses> {
    if let Some(a) = ALL.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return a;
    }
    let mut a = crate::builtin_analyses();
    if let Some(x) = EXTRA.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        for d in x.iter() {
            a.add_dyn(d.clone());
        }
    }
    let a = Arc::new(a);
    *ALL.write().unwrap_or_else(|e| e.into_inner()) = Some(a.clone());
    a
}

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
        super::register(Toy).unwrap();
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
        let a = crate::arg_spec::analysis_args("toy-ext").unwrap();
        let m = a.iter().find(|x| x["name"] == "mode").unwrap();
        assert_eq!(m["type"], "choice");
        assert_eq!(m["choices"], json!(["a", "b"]));
        let o = a.iter().find(|x| x["name"] == "opt").unwrap();
        assert_eq!(o["type"], "json");
        assert!(a.iter().all(|x| x["name"] != "glass_dir"));
        // serve の analyze: 読み込んだ系と読み方
        crate::serve::call("load", &json!({ "file": zmx, "glass_dir": [glass] }), None).unwrap();
        let r = crate::serve::call(
            "analyze",
            &json!({ "cmd": "toy-ext", "field": 1, "opt": { "x": [1, 2] } }),
            None,
        )
        .unwrap();
        assert_eq!(r["fields"], json!([0]));
        assert_eq!(r["opt"], json!({ "x": [1, 2] }));
        // 同梱の名前は足せない
        assert!(super::register(Named("spot")).is_err());
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
