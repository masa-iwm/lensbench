//! 組合せの最後の検出器（[`DetectorPick`]）

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 組合せの最後の検出器のブロック: 種類と、欄の文字列（項目の名前 → 欄に書いた文字列、空は既定）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectorPick {
    pub kind: String,
    pub vals: BTreeMap<String, String>,
}

impl DetectorPick {
    /// プロジェクトのブロック（{"type": kind, 項目…}）。種類が登録されていなければ欄の値をそのまま入れ子に
    pub fn block(&self) -> Result<Value, String> {
        let mut v = match crate::ext::detector(&self.kind) {
            Some(d) => d.object.json(&self.vals, false)?,
            None => crate::argform::nest(
                &self
                    .vals
                    .iter()
                    .filter(|(_, t)| !t.trim().is_empty())
                    .map(|(k, t)| {
                        let x = serde_json::from_str(t).unwrap_or_else(|_| json!(t));
                        (k.clone(), x)
                    })
                    .collect(),
            ),
        };
        if !v.is_object() {
            v = json!({});
        }
        v["type"] = json!(self.kind);
        Ok(v)
    }
}
