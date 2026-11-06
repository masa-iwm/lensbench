//! lensbench のコア: 順次光線追跡、.zmx 読込、解析。
//!
//! 外部のレンズカタログ（ZMF など）の読込は [`catalog::LensCatalogSource`] を実装した
//! 別クレートから差し込む。公開版にはその実装を含めない。

pub mod catalog;

/// ライブラリの版。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
