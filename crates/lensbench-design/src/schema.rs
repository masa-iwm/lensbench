//! JSON スキーマ: 差し替え探索の設定（`swap`）。プロジェクト・系ファイル・ユーザーカタログは
//! [`lensbench_project::schema`]。リポジトリの `docs/schema/swap.schema.json` はこれで作り、テスト
//! （`tests/schema.rs`）で型とずれていないかを確かめる（作り直すには `LENSBENCH_UPDATE_SCHEMA=1 cargo test
//! -p lensbench-design --test it schema::`）。

use crate::swap::SwapSpec;
use lensbench_project::schema::{finish, gen};
use serde_json::Value;

/// このクレートのスキーマの名前（`docs/schema/<名前>.schema.json`）
pub const NAMES: [&str; 1] = ["swap"];

/// 名前のスキーマ。知らない名前なら None
pub fn schema(name: &str) -> Option<Value> {
    let (mut v, title) = match name {
        "swap" => (gen::<SwapSpec>(), "lensbench 差し替え探索の設定"),
        _ => return None,
    };
    finish(&mut v, name, title);
    Some(v)
}

/// このクレートのすべてのスキーマ（名前と値）
pub fn all() -> Vec<(&'static str, Value)> {
    NAMES
        .iter()
        .map(|n| (*n, schema(n).expect("NAMES の名前")))
        .collect()
}
