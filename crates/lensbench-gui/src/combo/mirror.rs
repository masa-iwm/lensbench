//! 鏡だけが持つ欄（[`MirrorData`]）

use serde::{Deserialize, Serialize};

use super::Pick;

/// 鏡だけが持つ欄（[`Pick::mirror`]）。曲率半径は自作の玉と同じ [`Pick::r1`]、膜は BS と同じ [`Pick::coat`]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MirrorData {
    /// コーニック
    pub conic: f64,
    /// 往路を戻る（ダブルパス・干渉計の腕の端）
    pub back: bool,
}

impl MirrorData {
    pub const DEFAULT: MirrorData = MirrorData {
        conic: 0.0,
        back: false,
    };
}

impl Pick {
    /// 鏡の欄（鏡でなければ既定）
    pub fn mirror(&self) -> &MirrorData {
        static DEFAULT: MirrorData = MirrorData::DEFAULT;
        self.mirror.as_deref().unwrap_or(&DEFAULT)
    }

    /// 鏡の欄を変える（無ければ作る。鏡の要素だけで呼ぶ）
    pub fn mirror_mut(&mut self) -> &mut MirrorData {
        self.mirror.get_or_insert_with(Box::default)
    }
}
