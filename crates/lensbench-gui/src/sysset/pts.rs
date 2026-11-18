//! 系の設定の視野・波長の点（[`super`]）

use serde::{Deserialize, Serialize};

/// 視野の点（x, y と重み）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct FieldPt {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "one")]
    pub w: f64,
    /// ケラレ係数 [VDX, VDY, VCX, VCY, VAN]（プロジェクトの fields.vignetting。開いた .zmx の値を持つ）
    #[serde(default, skip_serializing_if = "no_vig")]
    pub vig: [f64; 5],
}

fn no_vig(v: &[f64; 5]) -> bool {
    v.iter().all(|&x| x == 0.0)
}

impl FieldPt {
    pub(crate) fn y(y: f64) -> FieldPt {
        FieldPt::xyw(0.0, y, 1.0)
    }

    pub(crate) fn xyw(x: f64, y: f64, w: f64) -> FieldPt {
        FieldPt {
            x,
            y,
            w,
            vig: [0.0; 5],
        }
    }

    /// ケラレ係数があるか
    pub(crate) fn has_vig(&self) -> bool {
        !no_vig(&self.vig)
    }
}

/// 波長 [µm] と重み
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct WavePt {
    pub um: f64,
    #[serde(default = "one")]
    pub w: f64,
}

impl WavePt {
    pub(crate) fn um(um: f64) -> WavePt {
        WavePt { um, w: 1.0 }
    }
}

fn one() -> f64 {
    1.0
}
