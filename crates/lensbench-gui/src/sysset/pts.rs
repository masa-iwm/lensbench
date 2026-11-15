//! 系の設定の視野・波長の点（[`super`]）

use serde::{Deserialize, Serialize};

/// 視野の点（x, y と重み）。古い保存の数（視野角 y）も読む
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "FieldIn")]
pub struct FieldPt {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    /// ケラレ係数 [VDX, VDY, VCX, VCY, VAN]（プロジェクトの fields.vignetting。開いた .zmx の値を持つ）
    #[serde(skip_serializing_if = "no_vig")]
    pub vig: [f64; 5],
}

fn no_vig(v: &[f64; 5]) -> bool {
    v.iter().all(|&x| x == 0.0)
}

impl FieldPt {
    pub fn y(y: f64) -> FieldPt {
        FieldPt::xyw(0.0, y, 1.0)
    }

    pub fn xyw(x: f64, y: f64, w: f64) -> FieldPt {
        FieldPt {
            x,
            y,
            w,
            vig: [0.0; 5],
        }
    }

    /// ケラレ係数があるか
    pub fn has_vig(&self) -> bool {
        !no_vig(&self.vig)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FieldIn {
    Y(f64),
    Pt {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default = "one")]
        w: f64,
        #[serde(default)]
        vig: [f64; 5],
    },
}

impl From<FieldIn> for FieldPt {
    fn from(v: FieldIn) -> FieldPt {
        match v {
            FieldIn::Y(y) => FieldPt::y(y),
            FieldIn::Pt { x, y, w, vig } => FieldPt { x, y, w, vig },
        }
    }
}

/// 波長 [µm] と重み。古い保存の数（波長だけ）も読む
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "WaveIn")]
pub struct WavePt {
    pub um: f64,
    pub w: f64,
}

impl WavePt {
    pub fn um(um: f64) -> WavePt {
        WavePt { um, w: 1.0 }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WaveIn {
    Um(f64),
    Pt {
        um: f64,
        #[serde(default = "one")]
        w: f64,
    },
}

impl From<WaveIn> for WavePt {
    fn from(v: WaveIn) -> WavePt {
        match v {
            WaveIn::Um(um) => WavePt::um(um),
            WaveIn::Pt { um, w } => WavePt { um, w },
        }
    }
}

fn one() -> f64 {
    1.0
}
