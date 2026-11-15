//! 同梱のプラグインの登録（CLI・GUI・serve・評価関数の `plugin` の項で共通）

use lensbench_core::plugin::Registry;

/// 同梱のプラグイン（解析）を登録した表に、今の拡張の表（[`lensbench_core::ext::current`]、exe や Python が
/// 足した分）のプラグインを重ねたもの
pub fn builtin() -> Registry {
    let mut r = Registry::new();
    lensbench_analysis::register(&mut r);
    for p in lensbench_core::ext::current().plugins().plugins() {
        r.register(p.clone());
    }
    r
}
