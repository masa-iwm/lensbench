//! lensbench の解析。
//!
//! - [`aber`] は 収差図の計算（横収差・OPD の扇、像面湾曲、倍率色収差、色の焦点ずれ）
//! - [`gauss`] は 近軸ガウスビーム（各面のビーム径・ウエスト・広がり角）
//! - [`footprint`] は フットプリント（面の上の光束の範囲）とけられずに通る光線の割合
//! - [`analysis`]・[`psf`]・[`pop`]・[`sweep`]・[`fiber`]・[`raygrid`] は 実光線の解析（MTF・スポット・Zernike・波面・PSF・POP・
//!   焦点の掃引・ファイバ結合・光線の格子。core から移した。OPD と瞳の標本は core の `opd`）
//! - [`registry`] は 解析の約束（設定・結果の型と登録簿）。CLI・serve・GUI はここから作る
//! - [`defs`] は 同梱の解析の定義
//! - [`huygens`] は Huygens PSF・Huygens MTF の計算（core から外した解析の計算）
//! - [`ghost`]・[`interference`]・[`polgrid`] は ゴーストの経路・単色の干渉縞・偏光の格子（core から外した解析）
//! - [`plugins`] は 解析のプラグイン（`lensbench_core::plugin`、評価関数の項・照合が使う名前つき配列の形）

#![warn(unreachable_pub)]
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方（core と同じ）
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod aber;
pub mod analysis;
pub mod defs;
pub mod energy;
pub mod fiber;
pub mod footprint;
pub mod gauss;
pub mod ghost;
pub mod huygens;
pub mod illum;
pub mod imagesim;
pub mod interference;
mod nan;
pub mod plugins;
pub mod polgrid;
pub mod pop;
pub mod psf;
pub mod pupil_amp;
pub mod raygrid;
pub mod registry;
pub mod sweep;

pub use plugins::{register, Analysis as PluginAnalysis, Prepared};
pub use registry::{
    Analyses, Analysis, Cx, DynAnalysis, Meta, Param, ParamTy, Saved, Source, Typed, Variant,
};

/// ライブラリとして使うときの主な口（`use lensbench_analysis::prelude::*;`）: .zmx の読込（lensbench-formats の prelude）と
/// 解析（[`analysis`]・[`psf`]）
pub mod prelude {
    pub use crate::analysis::{self, Mtf, Wavefront};
    pub use crate::psf::{self, Psf, PsfOpts, PsfPlane};
    pub use lensbench_formats::prelude::*;
}
