//! 開口・遮蔽の形（[`ApShape`]）と開口の要素だけが持つ欄（[`ApData`]、GPN-04）

use super::Pick;
use crate::i18n::tr;
use serde::{Deserialize, Serialize};

/// 開口・遮蔽の形
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApShape {
    /// 円の開口（内径があれば輪帯）
    #[default]
    Circle,
    /// 矩形の開口
    Rect,
    /// 楕円の開口
    Ellipse,
    /// 円の遮蔽（内径〜外径を遮る）
    Obscuration,
    /// スパイダー（中心から外へ伸びる幅の腕を遮る。.zmx の SPID）
    Spider,
}

impl ApShape {
    pub fn label(self) -> &'static str {
        match self {
            ApShape::Circle => tr("Circular aperture"),
            ApShape::Rect => tr("Rectangular aperture"),
            ApShape::Ellipse => tr("Elliptical aperture"),
            ApShape::Obscuration => tr("Circular obscuration"),
            ApShape::Spider => tr("Spider"),
        }
    }
}

/// 開口の要素の形と寸法（[`Pick::ap`]）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApData {
    pub shape: ApShape,
    /// 矩形・楕円の x・y の半幅 [mm]
    pub half: [f64; 2],
    /// 円の開口・遮蔽の内径（半径）[mm]
    pub min: f64,
    /// スパイダーの腕の幅（全幅）[mm]
    pub spider_width: f64,
    /// スパイダーの腕の本数（360° を等分、最初の腕は +x）
    pub spider_arms: u32,
}

impl ApData {
    pub const DEFAULT: ApData = ApData {
        shape: ApShape::Circle,
        half: [5.0, 5.0],
        min: 0.0,
        spider_width: 1.0,
        spider_arms: 4,
    };
}

impl Default for ApData {
    fn default() -> Self {
        ApData::DEFAULT
    }
}

impl Pick {
    /// 開口の欄（開口の要素でなければ既定）
    pub fn ap(&self) -> &ApData {
        self.ap.as_deref().unwrap_or(&ApData::DEFAULT)
    }

    /// 開口の欄を変える（無ければ作る。開口の要素だけで呼ぶ）
    pub fn ap_mut(&mut self) -> &mut ApData {
        self.ap.get_or_insert_with(Box::default)
    }
}
