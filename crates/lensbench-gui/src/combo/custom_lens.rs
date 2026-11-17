//! 自作の玉（2 面の単玉）・平板だけが持つ欄（[`CustomLens`]）

use serde::{Deserialize, Serialize};

use super::Pick;

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

impl Pick {
    /// 自作の玉・平板の欄（ほかの種類では [`CustomLens::NONE`]）
    pub fn custom(&self) -> &CustomLens {
        static NONE: CustomLens = CustomLens::NONE;
        self.custom.as_deref().unwrap_or(&NONE)
    }

    /// 自作の玉・平板の欄を変える（無ければ新しい玉の値で作る。自作の玉・平板だけで呼ぶ）
    pub fn custom_mut(&mut self) -> &mut CustomLens {
        self.custom.get_or_insert_with(Box::default)
    }
}
