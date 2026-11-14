//! 検出器のブロック（[`crate::DetectorBlock`]）の種類の登録。
//!
//! 検出器は拡張（非公開版など）が足すもので、このクレートは中身を知らない。種類ごとに名前（ブロックの `"type"`）と、
//! 項目を確かめる関数・スキーマを登録する。登録していない種類のブロックは読むときに誤り。
//!
//! 検出器のブロックは最後のブロックだけに置け、面は足さない（像面に置く。前の空気のブロックが距離）。

use lensbench_core::RwLockExt;
use std::sync::RwLock;

use serde_json::{Map, Value};

/// 検出器の種類
#[derive(Clone, Copy)]
pub struct DetectorKind {
    /// ブロックの `"type"`（組み込みのブロックの名前 [`crate::BUILTIN_BLOCKS`] とは別に）
    pub name: &'static str,
    /// 説明（スキーマの description）
    pub doc: &'static str,
    /// 項目（`"type"`・`"id"` を除く）を確かめる。誤りなら説明
    pub check: fn(&Map<String, Value>) -> Result<(), String>,
    /// 項目（`"type"`・`"id"` を除く）の JSON スキーマ（無ければ何でも書ける）
    pub schema: Option<fn() -> Value>,
}

static KINDS: RwLock<Vec<DetectorKind>> = RwLock::new(Vec::new());

/// 種類を登録する（同じ名前なら置き換える）。組み込みのブロックの名前は登録できない
pub fn register(k: DetectorKind) -> Result<(), String> {
    if crate::BUILTIN_BLOCKS.contains(&k.name) || k.name.is_empty() {
        return Err(format!("検出器の種類の名前 {:?} は使えない", k.name));
    }
    let mut v = KINDS.write_ok();
    v.retain(|x| x.name != k.name);
    v.push(k);
    Ok(())
}

/// 名前の種類
pub fn get(name: &str) -> Option<DetectorKind> {
    KINDS.read_ok().iter().find(|k| k.name == name).copied()
}

/// 登録した種類（登録の順）
pub fn list() -> Vec<DetectorKind> {
    KINDS.read_ok().clone()
}
