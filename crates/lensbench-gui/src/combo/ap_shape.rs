//! 開口・遮蔽の形（[`ApShape`]）

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
