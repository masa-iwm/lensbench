//! 設計の評価と探索の設定・結果の JSON: 評価関数の項（[`merit`]・[`plugin_term`]）、モンテカルロ公差
//! （設定と結果の JSON は [`tolerance`]、計算は [`montecarlo`]）、カタログレンズの差し替え探索（[`swap`]）、
//! 膜のモデルの当てはめ（[`coat_fit`]）と膜の設計（[`coat_design`]）。最適化の計算は core の `optim`、
//! 系はプロジェクト（lensbench-project）から作る。core から外した設計の計算（公差・膜の当てはめと設計）もここに
//! 置き、core を光線追跡の核に寄せる（XCT-04）。評価関数の `plugin` の項が同梱の解析を呼ぶので、解析
//! （lensbench-analysis）への依存はこのクレートに置き、プロジェクトのクレートは解析に依存しない（XCT-11）

// NaN を弾く比較（!(x > 0.0)）を core と同じく使う
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod coat_design;
pub mod coat_fit;
pub mod merit;
pub mod montecarlo;
pub mod plugin_term;
pub mod plugins;
pub mod schema;
pub mod swap;
pub mod tolerance;
