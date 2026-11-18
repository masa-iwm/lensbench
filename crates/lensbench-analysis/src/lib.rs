//! lensbench の解析。
//!
//! - [`registry`] は 解析の約束（設定・結果の型と登録簿）。CLI・serve・GUI はここから作る
//! - [`defs`] は 同梱の解析の定義
//! - [`huygens`] は Huygens PSF・Huygens MTF の計算（core から外した解析の計算）
//! - [`ghost`]・[`interference`]・[`polgrid`] は ゴーストの経路・単色の干渉縞・偏光の格子（core から外した解析）
//! - [`plugins`] は 解析のプラグイン（`lensbench_core::plugin`、評価関数の項・照合が使う名前つき配列の形）

#![warn(unreachable_pub)]
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方（core と同じ）
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod defs;
pub mod ghost;
pub mod huygens;
pub mod interference;
mod nan;
pub mod plugins;
pub mod polgrid;
pub mod pupil_amp;
pub mod registry;

pub use plugins::{register, Analysis as PluginAnalysis, Prepared};
pub use registry::{
    Analyses, Analysis, Cx, DynAnalysis, Meta, Param, ParamTy, Saved, Source, Typed, Variant,
};
