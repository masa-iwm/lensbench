//! 検出器のブロック（[`crate::DetectorBlock`]）の種類の登録。
//!
//! 検出器は拡張（非公開版など）が足すもので、このクレートは中身を知らない。種類ごとに名前（ブロックの `"type"`）と、
//! 項目を確かめる関数・スキーマを登録する。登録していない種類のブロックは読むときに誤り。
//!
//! 検出器のブロックは最後のブロックだけに置け、面は足さない（像面に置く。前の空気のブロックが距離）。

use lensbench_core::ext::{self, Ext};

use serde_json::{Map, Value};

/// 検出器の種類
#[derive(Clone, Copy)]
pub struct DetectorKind {
    /// ブロックの `"type"`（組み込みのブロックの名前 [`crate::BUILTIN_BLOCKS`] とは別に）
    pub name: &'static str,
    /// 説明（スキーマの description）
    pub doc: &'static str,
    /// 項目（`"type"`・`"id"` を除く）を確かめる。誤りなら説明
    pub check: fn(&Map<String, Value>) -> lensbench_core::Result<()>,
    /// 項目（`"type"`・`"id"` を除く）の JSON スキーマ（無ければ何でも書ける）
    pub schema: Option<fn() -> Value>,
}

/// 登録した種類の表（拡張の表 [`Ext`] の欄、XCT-07）
#[derive(Clone, Default)]
pub struct DetectorKinds(Vec<DetectorKind>);

/// 表 e に種類を足す（同じ名前なら置き換える）。組み込みのブロックの名前は使えない
pub fn add(e: &mut Ext, k: DetectorKind) -> lensbench_core::Result<()> {
    if crate::BUILTIN_BLOCKS.contains(&k.name) || k.name.is_empty() {
        return Err(format!("detector kind name {:?} can't be used", k.name).into());
    }
    let v = &mut e.slot_mut::<DetectorKinds>().0;
    v.retain(|x| x.name != k.name);
    v.push(k);
    Ok(())
}

/// 種類をプロセスの既定の表に登録する（[`add`]）
pub fn register(k: DetectorKind) -> lensbench_core::Result<()> {
    ext::update_default(|e| add(e, k))
}

/// 名前の種類（今の計算の表から）
pub fn get(name: &str) -> Option<DetectorKind> {
    list().into_iter().find(|k| k.name == name)
}

/// 登録した種類（登録の順、今の計算の表から）
pub fn list() -> Vec<DetectorKind> {
    ext::current()
        .slot::<DetectorKinds>()
        .map(|k| k.0.clone())
        .unwrap_or_default()
}
