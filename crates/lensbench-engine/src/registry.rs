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

/// 足した拡張の解析（拡張の表 [`lensbench_core::ext::Ext`] の欄、XCT-07）
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
pub fn add_dyn(e: &mut Ext, a: Arc<dyn DynAnalysis>) -> Result<(), String> {
    check_name(a.meta().name)?;
    e.slot_mut::<ExtraAnalyses>().0.add_dyn(a);
    Ok(())
}

/// 表 e に解析を足す（同じ名前は置き換える）。同梱の解析・CLI のサブコマンドと同じ名前は誤り
pub fn add<A: Analysis>(e: &mut Ext, a: A) -> Result<(), String> {
    check_name(a.meta().name)?;
    e.slot_mut::<ExtraAnalyses>().0.add(a);
    Ok(())
}

/// 解析をプロセスの既定の表に足す（[`add`]）
pub fn register<A: Analysis>(a: A) -> Result<(), String> {
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
