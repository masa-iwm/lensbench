//! カタログの品の検索と補完（PRJ-05。以前は lensbench-project の `catalog_spec`・`catalog_fill`）。
//!
//! - [`spec`] は 仕様での絞り込み（[`spec::SpecFilter`]）・一覧の検索（[`spec::CatalogQuery`]）・ほかのカタログの品へ
//!   仕様とリンクを足す（[`spec::apply_overlays`]）。品の種類・名前の決まった仕様の型はユーザーカタログの形の一部なので
//!   lensbench-project の `catalog_spec` にあり、ここから同じ名前で使える
//! - [`fill`] は 未完の品を品ページの仕様表・STEP の寸法で補う
//! - [`step`] は 品の STEP を測る（厚さ・径・枠・機構部品の寸法、殻の置き方。文字の読みは lensbench-step）

#![warn(unreachable_pub)]
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方（core と同じ）
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod fill;
pub mod spec;
pub mod step;
