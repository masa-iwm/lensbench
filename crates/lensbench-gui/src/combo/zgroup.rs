//! .zmx の玉・取り込みの欄（[`ZmxSrc`]）と、取り込みの中の群（[`ZGroup`]、プロジェクトの zmx の groups）

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use super::StatePlace;

/// .zmx の玉・取り込みだけが持つ欄（[`Pick::zmx`]）。ファイルの場所は、ユーザーカタログの項目と同じ
/// [`Pick::path`] に持つ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZmxSrc {
    /// .zmx の玉: 何番目の玉か（1 始まり）
    pub lens: usize,
    /// マルチコンフィグの番号（1 始まり）
    pub config: usize,
    /// 取り込み: 面 from〜to（0 は既定: 面 1〜像面の手前）
    pub range: [usize; 2],
    /// 取り込み: 玉モードの群（面 from〜to、名前・役割）
    pub zgroups: Vec<ZGroup>,
    /// 取り込み: 状態ごとのマルチコンフィグの番号（状態の名前 → 番号。無ければ config）
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub state_config: BTreeMap<String, usize>,
}

impl ZmxSrc {
    pub const DEFAULT: ZmxSrc = ZmxSrc {
        lens: 1,
        config: 1,
        range: [0, 0],
        zgroups: Vec::new(),
        state_config: BTreeMap::new(),
    };
}

impl Default for ZmxSrc {
    fn default() -> Self {
        Self::DEFAULT
    }
}

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
