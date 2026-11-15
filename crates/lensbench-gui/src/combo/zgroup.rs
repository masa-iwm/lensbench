//! .zmx の取り込みの中の群（[`ZGroup`]、プロジェクトの zmx の groups）

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use super::StatePlace;

/// .zmx の取り込みの中の群（プロジェクトの zmx の groups）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ZGroup {
    pub from: usize,
    pub to: usize,
    pub name: String,
    pub role: String,
    /// 状態が指す id（開いたプロジェクトの id。空なら名前）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// 状態ごとの移動 [mm]・偏心・傾き（開いたプロジェクトの状態）
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub state_shift: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub state_place: BTreeMap<String, StatePlace>,
}

impl ZGroup {
    /// 状態が指す id
    pub fn key(&self) -> &str {
        if self.id.is_empty() {
            self.name.trim()
        } else {
            &self.id
        }
    }

    /// プロジェクトの zmx の groups の項目（面の範囲が無ければ None）
    pub fn json(&self) -> Option<Value> {
        if self.from == 0 || self.to < self.from {
            return None;
        }
        let mut v = json!({ "from": self.from, "to": self.to });
        if !self.key().is_empty() {
            v["id"] = json!(self.key());
        }
        if !self.name.trim().is_empty() {
            v["name"] = json!(self.name.trim());
        }
        if !self.role.is_empty() {
            v["role"] = json!(self.role);
        }
        Some(v)
    }

    /// 状態 n の値（状態の set の項目。無ければ None）
    pub fn state_set(&self, n: &str) -> Option<Map<String, Value>> {
        let mut m = Map::new();
        if let Some(x) = self.state_shift.get(n) {
            m.insert("shift".into(), json!(x));
        }
        if let Some(p) = self.state_place.get(n) {
            m.insert("decenter".into(), json!(p.decenter));
            m.insert("tilt".into(), json!(p.tilt));
        }
        (!m.is_empty() && !self.key().is_empty()).then_some(m)
    }
}
