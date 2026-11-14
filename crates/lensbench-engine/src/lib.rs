//! lensbench の実行の層: 解析の登録簿（同梱の解析と拡張の解析）・系の読み方・serve の場（[`Session`]）。
//!
//! CLI（`lensbench-cli`）・GUI・Web 版・Python はここを使う（CLI の引数の解釈・要求を運ぶ道は CLI が持つ）。

pub mod analyze;
pub mod browse;
pub mod defs;
pub mod load;
pub mod params;
pub mod provenance;
pub mod registry;
pub mod session;
pub mod source;
pub mod zoom;

pub use browse::{
    set_cache_dir as set_lens_list_cache_dir, set_prepared as set_prepared_lens_list,
};
pub use load::{builtin_plugins, SysSpec};
pub use registry::{analyses, builtin_analyses, register};
pub use session::{
    browse, call, call_arrays, global, note_panic, optimize_cancel, optimize_progress,
    preview_layout, range_system, swap_cancel, swap_progress, tolerance_cancel, tolerance_progress,
    user_catalog_edit, Arrays, Bin, CallError, Data, Files, Session,
};
