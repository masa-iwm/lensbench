//! 解析の登録簿: 同梱の解析（[`lensbench_analysis::Analyses::builtin`] と [`crate::defs`]）と、exe の入口で足した拡張の解析
//! （非公開版など、[`register`]）。足した解析は同梱のものと同じく CLI のサブコマンド・serve の `analyze`・`watch`・GUI の解析
//! （GUI 側の見せ方の登録は `lensbench_gui::ext`）で使える。設定は設定の型のスキーマから作り、系の読み方（別の構成・状態の系、
//! プロジェクトファイル）は [`lensbench_analysis::Cx::source`] で引く。

use lensbench_core::ext::Ext;
use std::sync::Arc;

use lensbench_analysis::{Analyses, Analysis, DynAnalysis};

/// CLI のサブコマンド（解析の登録簿の外のもの）の名前。拡張の解析はこの名前を使えない（CLI の試験で clap の定義と突き合わせる）
pub const RESERVED: &[&str] = &[
    "convert",
    "make-thermal",
    "zmx",
    "optimize",
    "tolerance",
    "zoom",
    "swap",
    "catalog-add",
    "catalog-search",
    "catalog-step",
    "cad-step",
    "catalog-list",
    "coatings",
    "coat-kinks",
    "layout",
    "coat-model",
    "coat-compact",
    "coat-design",
    "serve",
    "watch",
    "run",
    "plugins",
    "verify",
];

/// 足した拡張の解析（拡張の表 [`lensbench_core::ext::Ext`] の欄）
#[derive(Clone, Default)]
pub struct ExtraAnalyses(Analyses);

/// 同梱の解析の登録簿（拡張の解析は除く）
pub fn builtin_analyses() -> Analyses {
    static B: std::sync::OnceLock<Analyses> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        let mut a = Analyses::builtin().clone();
        crate::defs::register(&mut a);
        a
    })
    .clone()
}

/// 同梱の解析・CLI のサブコマンドと同じ名前でないか
fn check_name(name: &str) -> lensbench_core::Result<()> {
    if RESERVED.contains(&name) || builtin_analyses().get(name).is_some() {
        return Err(format!("analysis {name} has the same name as a built-in one").into());
    }
    Ok(())
}

/// 表 e に型を消した解析を足す（同じ名前は置き換える）
pub fn add_dyn(e: &mut Ext, a: Arc<dyn DynAnalysis>) -> lensbench_core::Result<()> {
    check_name(a.meta().name)?;
    e.slot_mut::<ExtraAnalyses>().0.add_dyn(a);
    Ok(())
}

/// 表 e に解析を足す（同じ名前は置き換える）。同梱の解析・CLI のサブコマンドと同じ名前は誤り
pub fn add<A: Analysis>(e: &mut Ext, a: A) -> lensbench_core::Result<()> {
    check_name(a.meta().name)?;
    e.slot_mut::<ExtraAnalyses>().0.add(a);
    Ok(())
}

/// 解析をプロセスの既定の表に足す（[`add`]）
pub fn register<A: Analysis>(a: A) -> lensbench_core::Result<()> {
    lensbench_core::ext::update_default(|e| add(e, a))
}

/// 解析の登録簿（同梱の解析と、今の計算の表に足した拡張の解析）
pub fn analyses() -> Arc<Analyses> {
    let mut a = builtin_analyses();
    if let Some(x) = lensbench_core::ext::current().slot::<ExtraAnalyses>() {
        for d in x.0.iter() {
            a.add_dyn(d.clone());
        }
    }
    Arc::new(a)
}

#[cfg(test)]
mod plugin_defaults {
    use lensbench_core::plugin::{Registry, Value};
    use serde_json::Value as Json;

    /// 同じ値か（プラグインの値と登録簿の JSON の既定）
    fn same(p: &Value, j: &Json) -> bool {
        match (p, j) {
            (Value::Num(x), Json::Number(n)) => n.as_f64() == Some(*x),
            (Value::Bool(x), Json::Bool(y)) => x == y,
            (Value::Str(x), Json::String(y)) => x == y,
            // 番号を文字でも受ける項目（interference の beam1 = "1" など）
            (Value::Num(x), Json::String(y)) => y.parse::<f64>().ok() == Some(*x),
            (Value::Nums(x), Json::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| b.as_f64() == Some(*a))
            }
            _ => false,
        }
    }

    /// 同梱プラグインの設定の既定は、同じ解析の登録簿の既定と揃える
    #[test]
    fn plugin_defaults_match_registry() {
        let mut r = Registry::default();
        lensbench_analysis::plugins::register(&mut r);
        let reg = super::builtin_analyses();
        let mut checked = 0;
        for p in r.plugins() {
            let name = match p.name() {
                "huygens_psf" => "psf".to_string(),
                "interferogram" => "interference".to_string(),
                n => n.replace('_', "-"),
            };
            let Some(a) = reg.get(&name) else {
                panic!("plugin {} has no registry analysis {name}", p.name());
            };
            let defs: std::collections::BTreeMap<_, _> = a
                .params()
                .into_iter()
                .map(|x| (x.name, x.default))
                .collect();
            for s in p.params() {
                let (Some(d), Some(j)) = (&s.default, defs.get(&s.name)) else {
                    continue;
                };
                assert!(
                    same(d, j),
                    "{}.{}: plugin {d:?} / registry {j}",
                    p.name(),
                    s.name
                );
                checked += 1;
            }
        }
        assert!(checked >= 20, "only {checked} defaults compared");
    }
}
