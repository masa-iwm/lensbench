//! 自作の玉（2 面の単玉）・平板だけが持つ欄（[`CustomLens`]）

use serde::{Deserialize, Serialize};

/// 自作の玉（2 面の単玉）・平板の欄（[`Pick::custom`]）。3 面以上・貼り合わせの玉は [`Pick::surfs`] に面ごとに持つ。
/// 半径は [`Pick::semi`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomLens {
    /// 前・後の面の曲率半径 [mm]（0 は平面。平板は 0）
    pub r1: f64,
    pub r2: f64,
    /// 中心厚 [mm]
    pub t: f64,
    pub glass: String,
    /// 半径を光線から求める
    pub semi_auto: bool,
}

impl CustomLens {
    /// ほかの種類の要素が読むときの値（硝材は空）
    pub const NONE: CustomLens = CustomLens {
        r1: 0.0,
        r2: 0.0,
        t: 0.0,
        glass: String::new(),
        semi_auto: false,
    };
}

impl Default for CustomLens {
    /// 新しい自作の玉の値
    fn default() -> Self {
        CustomLens {
            r1: 50.0,
            r2: -50.0,
            t: 4.0,
            glass: "N-BK7".into(),
            semi_auto: false,
        }
    }
}
