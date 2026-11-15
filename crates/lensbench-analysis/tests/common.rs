//! 結合テストが共通に使うリポジトリの根（core の tests/common.rs と同じ約束、公開 #216 BLD-26）。

use std::path::PathBuf;

/// リポジトリの根
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
