//! 別の拡張モジュール（非公開版など）が、この Python モジュールに関数・クラスを足すための口。
//!
//! 入口の `#[pymodule]` で [`crate::init_module`] を呼んだ後に、自分の関数・クラスをモジュールに足す。系は
//! [`PySystem`]（`lb.System`）で受け、[`PySystem::sys`] で今の系を得る。
//!
//! `lb.run_many` などの模擬の組の関数に、Python を通さず Rust のまま回せるもの（[`NativeTask`]）を渡せるようにするには、
//! [`register_native_task`] で Python の値から取り出す関数を登録する。

use std::sync::{Arc, RwLock};

use lensbench_core::System;
use pyo3::prelude::*;

pub use crate::plugin::{arrays_to_py, PyPlugin};
pub use crate::{
    err, field_sel, obj_out, obj_spec, parse_changes, wave_index, FieldArg, ObjArg, PySystem,
};

/// 組の系で Rust のまま回す模擬（GIL を持たない）
pub trait NativeTask: Send + Sync {
    /// 組の系 `sample` で回し、組の結果の JSON（文字列）を返す。`batch_seed` は組の種
    fn run_json(&self, sample: &System, batch_seed: u64) -> lensbench_core::Result<String>;
}

/// Python の値から [`NativeTask`] を取り出す関数（その値でなければ None）
pub type NativeTaskFrom = fn(&Bound<'_, PyAny>) -> Option<Arc<dyn NativeTask>>;

static NATIVE: RwLock<Vec<NativeTaskFrom>> = RwLock::new(Vec::new());

/// [`NativeTask`] を取り出す関数を足す
pub fn register_native_task(f: NativeTaskFrom) {
    NATIVE.write().unwrap_or_else(|e| e.into_inner()).push(f);
}

/// 登録した関数で値から [`NativeTask`] を取り出す（どれも合わなければ None）
pub(crate) fn native_task(v: &Bound<'_, PyAny>) -> Option<Arc<dyn NativeTask>> {
    let fs = NATIVE.read().unwrap_or_else(|e| e.into_inner()).clone();
    fs.iter().find_map(|f| f(v))
}
