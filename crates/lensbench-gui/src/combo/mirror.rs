//! 鏡だけが持つ欄（[`MirrorData`]）

use serde::{Deserialize, Serialize};

/// 鏡だけが持つ欄（[`Pick::mirror`]）。膜は BS と同じ [`Pick::coat`]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MirrorData {
    /// 曲率半径 [mm]（0 は平面）
    pub radius: f64,
    /// コーニック
    pub conic: f64,
    /// 往路を戻る（ダブルパス・干渉計の腕の端）
    pub back: bool,
}

impl MirrorData {
    pub const DEFAULT: MirrorData = MirrorData {
        radius: 0.0,
        conic: 0.0,
        back: false,
    };
}

impl super::Pick {
    /// 行で書いた鏡（Zernike・Grid Sag など、.zmx を分けた鏡）の行。欄（半径・半直径・膜）は使わない
    pub fn mirror_row(&self) -> Option<&serde_json::Value> {
        self.extra.get("row").filter(|r| !r.is_null())
    }
}
