//! core の上のファイルの形式。
//!
//! - [`zmx_write`] は 系を .zmx（OpticStudio のレンズファイル）に書き出す（core から外した、XCT-04）。
//!   読込は core の `lensbench_core::zmx`（系のソルブ・ZPL と一緒に core に残す）
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方（core と同じ）
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod zmx_write;
