//! core の上のファイルの形式。
//!
//! - [`zmx_write`] は 系を .zmx（OpticStudio のレンズファイル）に書き出す（core から外した）。
//!   読込は core の `crate::zmx`（系のソルブ・ZPL と一緒に core に残す）

#![warn(unreachable_pub)]
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方（core と同じ）
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod zmx;
pub mod zmx_write;

/// ライブラリとして使うときの主な口（`use lensbench_formats::prelude::*;`）: core の prelude に .zmx の読込を足したもの
pub mod prelude {
    pub use crate::zmx::{self, LoadOptions};
    pub use lensbench_core::prelude::*;
}
