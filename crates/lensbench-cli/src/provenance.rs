//! --save の JSON に刻む版と系の hash（provenance）

use lensbench_core::System;
use serde_json::{json, Value};

thread_local! {
    /// このスレッドで最後に読んだ系の hash（--save の JSON の provenance に刻む）
    static LOADED_HASH: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// --save の provenance に刻む系（serve の登録簿の解析は load を通らないので、書く前にここで）
pub(crate) fn set(s: &System) {
    let h = lensbench_project::sysfile::system_hash(s).ok();
    LOADED_HASH.with(|x| *x.borrow_mut() = h);
}

/// 刻む系を忘れる（解析ごとに読み直す前に）
pub(crate) fn clear() {
    LOADED_HASH.with(|x| *x.borrow_mut() = None);
}

/// 結果に刻む版と系の hash
pub(crate) fn value() -> Value {
    let h = LOADED_HASH.with(|x| x.borrow().clone());
    json!({ "lensbench": lensbench_core::VERSION, "system_hash": h })
}
