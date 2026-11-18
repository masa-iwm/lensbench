//! 組合せに置く機構部品（[`MechPick`]）

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 組合せに置く機構部品: カタログの品を、要素（ブロック id）の面に置く
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MechPick {
    pub name: String,
    /// ユーザーカタログのファイル（絶対パス）と品の名前
    pub catalog: String,
    pub item: String,
    /// 置く要素のブロック id（空なら系の面 1）と、その要素の面（0 始まり）
    pub at: String,
    pub face: usize,
    /// 置いた面の局所座標でのずれ [mm] と傾き [deg]
    pub offset: [f64; 3],
    pub tilt: [f64; 2],
    /// 保持する要素のブロック id
    pub holds: Vec<String>,
    /// 部品の座を保持する要素の前面に合わせて置く（置く要素・面の代わり）
    pub seat: bool,
    /// 前の機構部品（名前）の後ろの端につなぐ（置く要素・面・座の代わり。z はそこからのずれ）
    pub after: String,
    /// 裏返して置く（光が部品の z の大きい端から入る）
    pub reversed: bool,
}

impl MechPick {
    /// プロジェクトの mechanics の項目
    pub(crate) fn json(&self) -> Value {
        let at = if self.at.is_empty() {
            json!({ "surface": 1 })
        } else {
            json!({ "block": self.at, "surface": self.face })
        };
        if !self.after.is_empty() {
            let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item,
                "after": self.after, "offset": self.offset });
            if !self.holds.is_empty() {
                v["holds"] = json!(self.holds);
            }
            if self.reversed {
                v["reversed"] = json!(true);
            }
            return v;
        }
        let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item, "at": at,
            "offset": self.offset, "tilt": self.tilt });
        if !self.holds.is_empty() {
            v["holds"] = json!(self.holds);
            if self.seat {
                v["seat"] = json!(true);
            }
        }
        if self.reversed {
            v["reversed"] = json!(true);
        }
        v
    }
}
