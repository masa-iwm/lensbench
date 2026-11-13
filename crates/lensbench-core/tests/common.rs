//! 結合テストが共通に使う基準データの場所（公開 #216 BLD-26）。
//! 基準（tests/refs）・テスト用ガラス（tests/glass）は LB_REFS_DIR・LB_GLASS_DIR で差し替えられる
//! （別の基準・ガラスで同じ照合をかけるため。lensbench_verify::zemax_refs と同じ約束）。

use std::path::PathBuf;

/// リポジトリの根
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 基準データのフォルダ（LB_REFS_DIR で差し替え）
pub fn refs_dir() -> PathBuf {
    std::env::var_os("LB_REFS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("tests/refs"))
}

/// ガラスカタログのフォルダ（LB_GLASS_DIR で差し替え、パス区切りで複数可）
pub fn glass_dirs() -> Vec<PathBuf> {
    match std::env::var_os("LB_GLASS_DIR") {
        Some(v) => std::env::split_paths(&v).collect(),
        None => vec![root().join("tests/glass")],
    }
}
