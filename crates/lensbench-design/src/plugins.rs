//! 同梱のプラグインの登録（CLI・GUI・serve・評価関数の `plugin` の項で共通）

use lensbench_core::plugin::{Config, Instance, Registry};
use lensbench_core::Result;
use std::sync::OnceLock;

/// 同梱のプラグイン（解析）の表（1 回だけ作る）
fn bundled() -> &'static Registry {
    static B: OnceLock<Registry> = OnceLock::new();
    B.get_or_init(|| {
        let mut r = Registry::new();
        lensbench_analysis::register(&mut r);
        r
    })
}

/// 同梱のプラグイン（解析）を登録した表に、今の拡張の表（[`lensbench_core::ext::current`]、exe や Python が
/// 足した分）のプラグインを重ねたもの
pub fn builtin() -> Registry {
    let mut r = bundled().clone();
    for p in lensbench_core::ext::current().plugins().plugins() {
        r.register(p.clone());
    }
    r
}

/// 名前のプラグインを設定で用意する（今の拡張の表に足したものが先、無ければ同梱のもの）。評価関数の項を作るたびに
/// 表を作り直さない
pub fn prepare(name: &str, config: &Config) -> Result<Box<dyn Instance>> {
    let e = lensbench_core::ext::current();
    if e.plugins().get(name).is_ok() {
        e.plugins().prepare(name, config)
    } else {
        bundled().prepare(name, config)
    }
}
