//! lensbench の解析。
//!
//! - [`registry`] は 解析の約束（設定・結果の型と登録簿）。CLI・serve・GUI はここから作る
//! - [`defs`] は 同梱の解析の定義
//! - [`plugins`] は 解析のプラグイン（`lensbench_core::plugin`、評価関数の項・照合が使う名前つき配列の形）

pub mod defs;
pub mod plugins;
pub mod registry;

pub use plugins::{register, Analysis as PluginAnalysis, Prepared};
pub use registry::{
    Analyses, Analysis, Cx, DynAnalysis, Meta, Param, ParamTy, Saved, Source, Variant,
};
