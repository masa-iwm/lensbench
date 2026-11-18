//! 別の拡張モジュール（非公開版など）が、この Python モジュールに関数・クラスを足すための口。
//!
//! 入口の `#[pymodule]` で [`crate::init_module`] を呼んだ後に、自分の関数・クラスをモジュールに足す。系は
//! [`PySystem`]（`lb.System`）で受け、[`PySystem::sys`] で今の系を得る。
//!
//! `lb.run_many` などの模擬の組の関数に、Python を通さず Rust のまま回せるもの（[`NativeTask`]）を渡せるようにするには、
//! [`register_native_task`] で Python の値から取り出す関数を登録する。
//!
//! 外のクレートは [`PyExtensions`] で `lensbench_engine::Extensions` に関数・クラスを足す関数を足し、入口の
//! `#[pymodule]` から [`module_with`] を呼ぶ（計算の登録と同じ値 1 つ）。

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

/// A function that adds functions and classes to the module (a slot of the extension table)
pub type ModuleFn = fn(&Bound<'_, PyModule>) -> PyResult<()>;

/// The registered [`ModuleFn`]s
#[derive(Clone, Default)]
struct ModuleFns(Vec<ModuleFn>);

/// Add Python functions to `lensbench_engine::Extensions` (they go into the extension table on install)
pub trait PyExtensions {
    /// A function that adds functions and classes to the module ([`module_with`] calls it after [`crate::init_module`])
    fn py_module(self, f: ModuleFn) -> Self;
    /// A function that takes a [`NativeTask`] out of a Python value
    fn py_native_task(self, f: NativeTaskFrom) -> Self;
}

impl PyExtensions for lensbench_engine::Extensions {
    fn py_module(self, f: ModuleFn) -> Self {
        // 同じ関数は 1 つだけ（同じ拡張を 2 回 install しても並べない）
        self.with_ext(move |e| {
            let v = &mut e.slot_mut::<ModuleFns>().0;
            if !v.iter().any(|g| *g as usize == f as usize) {
                v.push(f);
            }
        })
    }
    fn py_native_task(self, f: NativeTaskFrom) -> Self {
        self.with_ext(move |e| {
            let v = &mut e.slot_mut::<NativeTasks>().0;
            if !v.iter().any(|g| *g as usize == f as usize) {
                v.push(f);
            }
        })
    }
}

/// Whether the table `e` has any function added to the module (for checking an extension)
pub fn has_module_fns(e: &lensbench_core::ext::Ext) -> bool {
    e.slot::<ModuleFns>().is_some_and(|m| !m.0.is_empty())
}

/// Install `ext`, add the bundled functions and classes ([`crate::init_module`]) and then the extension's ([`PyExtensions::py_module`]).
/// Call it from the `#[pymodule]` of an extension module
pub fn module_with(m: &Bound<'_, PyModule>, ext: lensbench_engine::Extensions) -> PyResult<()> {
    ext.install().map_err(err)?;
    crate::init_module(m)?;
    let fns = lensbench_core::ext::current()
        .slot::<ModuleFns>()
        .map(|f| f.0.clone())
        .unwrap_or_default();
    for f in fns {
        f(m)?;
    }
    Ok(())
}

/// Take a [`NativeTask`] out of a value with the registered functions (None if none match)
pub(crate) fn native_task(v: &Bound<'_, PyAny>) -> Option<Arc<dyn NativeTask>> {
    let e = lensbench_core::ext::current();
    e.slot::<NativeTasks>()?.0.iter().find_map(|f| f(v))
}
