//! レンズカタログの差し込み口。

use std::path::Path;

/// カタログ中の 1 レンズ（面データは .zmx 断片のまま持つ）。
#[derive(Debug, Clone)]
pub struct CatalogLens {
    pub vendor: String,
    pub part: String,
    pub efl: f64,
    pub zmx: String,
}

/// 外部のレンズカタログを読む実装が満たすトレイト。
pub trait LensCatalogSource {
    /// この実装が読める拡張子（小文字、ドットなし）。
    fn extensions(&self) -> &[&str];
    /// ファイルを読み、レンズの一覧を返す。
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String>;
}
