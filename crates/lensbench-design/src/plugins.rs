//! 同梱のプラグインの登録（CLI・GUI・serve・評価関数の `plugin` の項で共通）

use lensbench_core::plugin::{self, Registry};
use lensbench_core::RwLockExt;

/// 同梱のプラグイン（解析）を登録した表に、プロセスで共通の登録（[`plugin::global`]、
/// exe や Python が足した分）を重ねたもの
pub fn builtin() -> Registry {
    let mut r = Registry::new();
    lensbench_analysis::register(&mut r);
    let g = plugin::global().read_ok();
    for p in g.plugins() {
        r.register(p.clone());
    }
    r
}
