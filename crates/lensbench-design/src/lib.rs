//! 設計の評価と探索の設定・結果の JSON: 評価関数の項（[`merit`]・[`plugin_term`]）、モンテカルロ公差
//! （[`tolerance`]）、カタログレンズの差し替え探索（[`swap`]）。計算は core（`optim`・`tolerance`）、
//! 系はプロジェクト（lensbench-project）から作る。評価関数の `plugin` の項が同梱の解析を呼ぶので、解析
//! （lensbench-analysis）への依存はこのクレートに置き、プロジェクトのクレートは解析に依存しない（XCT-11）

pub mod merit;
pub mod plugin_term;
pub mod plugins;
pub mod schema;
pub mod swap;
pub mod tolerance;
