//! 解析の登録簿: 同梱の解析（[`lensbench_analysis::Analyses::builtin`] と [`crate::defs`]）と、exe の入口で足した拡張の解析
//! （非公開版など、[`register`]）。足した解析は同梱のものと同じく CLI のサブコマンド・serve の `analyze`・`watch`・GUI の解析
//! （GUI 側の見せ方の登録は `lensbench_gui::ext`）で使える。設定は設定の型のスキーマから作り、系の読み方（別の構成・状態の系、
//! プロジェクトファイル）は [`lensbench_analysis::Cx::source`] で引く。

use lensbench_core::RwLockExt;
use std::sync::{Arc, RwLock};

use lensbench_analysis::{Analyses, Analysis};

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
    "cad-upload",
    "coat-upload",
    "coat-fetch",
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

static EXTRA: RwLock<Option<Analyses>> = RwLock::new(None);

/// 同梱の解析と足した解析を並べた登録簿（足すと作り直す）
static ALL: RwLock<Option<Arc<Analyses>>> = RwLock::new(None);

/// 同梱の解析の登録簿（拡張の解析は除く）
pub fn builtin_analyses() -> Analyses {
    let mut a = Analyses::builtin().clone();
    crate::defs::register(&mut a);
    a
}

/// 解析を足す（同じ名前は置き換える）。同梱の解析・CLI のサブコマンドと同じ名前は誤り
pub fn register<A: Analysis>(a: A) -> Result<(), String> {
    let name = a.meta().name;
    if RESERVED.contains(&name) || builtin_analyses().get(name).is_some() {
        return Err(format!("解析 {name} は同梱のものと同じ名前"));
    }
    EXTRA
        .write_ok()
        .get_or_insert_with(Analyses::default)
        .add(a);
    *ALL.write_ok() = None;
    Ok(())
}

/// 解析の登録簿（同梱の解析と、足した拡張の解析）
pub fn analyses() -> Arc<Analyses> {
    if let Some(a) = ALL.read_ok().clone() {
        return a;
    }
    let mut a = builtin_analyses();
    if let Some(x) = EXTRA.read_ok().as_ref() {
        for d in x.iter() {
            a.add_dyn(d.clone());
        }
    }
    let a = Arc::new(a);
    *ALL.write_ok() = Some(a.clone());
    a
}
