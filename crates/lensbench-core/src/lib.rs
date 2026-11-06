//! lensbench のコア: 順次光線追跡、.zmx 読込、解析。
//!
//! 外部のレンズカタログ（ZMF など）の読込は [`catalog::LensCatalogSource`] を実装した
//! 別クレートから差し込む。公開版にはその実装を含めない。
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方。
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]

pub mod error;
pub mod math;

pub mod analysis;
pub mod catalog;
pub mod glass;
pub mod report;
pub mod surface;
pub mod system;
pub mod trace;
pub mod zmx;

pub use error::{Error, Result};
pub use system::{FieldSel, System};

/// ライブラリの版。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
