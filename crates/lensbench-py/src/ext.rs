//! 別の拡張モジュール（非公開版など）が、この Python モジュールに関数・クラスを足すための口。
//!
//! 入口の `#[pymodule]` で [`crate::init_module`] を呼んだ後に、自分の関数・クラスをモジュールに足す。系は
//! [`PySystem`]（`lb.System`）で受け、[`PySystem::sys`] で今の系を得る。
//!
//! `lb.run_many` などの模擬の組の関数に、Python を通さず Rust のまま回せるもの（[`NativeTask`]）を渡せるようにするには、
//! [`register_native_task`] で Python の値から取り出す関数を登録する。

use std::sync::Arc;

use lensbench_core::System;
use pyo3::prelude::*;

pub use crate::plugin::{arrays_to_py, PyPlugin};
pub use crate::{
    err, field_sel, obj_out, obj_spec, parse_changes, wave_index, FieldArg, ObjArg, PySystem,
};

/// A simulation run in Rust on the systems of sets (without the GIL)
pub trait NativeTask: Send + Sync {
    /// Run on the system `sample` of a set and return the set's result as JSON (a string). `batch_seed` is the set's seed
    fn run_json(&self, sample: &System, batch_seed: u64) -> lensbench_core::Result<String>;
}

/// A function that takes a [`NativeTask`] out of a Python value (None if the value is not one)
pub type NativeTaskFrom = fn(&Bound<'_, PyAny>) -> Option<Arc<dyn NativeTask>>;

/// The registered functions that take out a [`NativeTask`] (a slot of the extension table `lensbench_core::ext::Ext`)
#[derive(Clone, Default)]
pub struct NativeTasks(Vec<NativeTaskFrom>);

/// Add a function that takes out a [`NativeTask`] (to the process default extension table)
pub fn register_native_task(f: NativeTaskFrom) {
    lensbench_core::ext::update_default(|e| e.slot_mut::<NativeTasks>().0.push(f));
}

/// Take a [`NativeTask`] out of a value with the registered functions (None if none match)
pub(crate) fn native_task(v: &Bound<'_, PyAny>) -> Option<Arc<dyn NativeTask>> {
    let e = lensbench_core::ext::current();
    e.slot::<NativeTasks>()?.0.iter().find_map(|f| f(v))
}
