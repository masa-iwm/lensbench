//! 自作の玉（2 面の単玉）・平板だけが持つ欄（[`CustomLens`]）

use egui::RichText;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::i18n::tr;
use crate::theme;

/// 自作の玉（2 面の単玉）・平板の欄（[`Pick::custom`]）。3 面以上・貼り合わせの玉は [`Pick::surfs`] に面ごとに持つ。
/// 半径は [`Pick::semi`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CustomLens {
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
    pub(crate) const NONE: CustomLens = CustomLens {
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

/// 半径の JSON（自動なら "auto"）
pub(super) fn semi_json(semi: f64, auto: bool) -> Value {
    if auto {
        json!("auto")
    } else {
        json!(semi)
    }
}

/// 自作の玉・平板の半径の欄と自動の切り替え。0 は径を書かない（開口なし。.zmx には自動の半直径で書く）ので、0 まで下げられる
pub(super) fn semi_edit(ui: &mut egui::Ui, semi: &mut f64, auto: &mut bool) {
    ui.add_enabled_ui(!*auto, |ui| {
        let weak = theme::weak_text(ui.visuals());
        let lbl = ui.label(RichText::new(tr("Semi-diameter")).color(weak));
        ui.add(
            crate::widgets::Num::new(semi)
                .max_decimals(4)
                .range(0.0..=f64::INFINITY)
                .suffix(" mm"),
        )
        .on_hover_text(tr(
            "0 leaves the semi-diameter unwritten (no aperture on the surfaces)",
        ))
        .labelled_by(lbl.id);
    });
    ui.checkbox(auto, tr("Auto")).on_hover_text(tr(
        "Compute the semi-diameter from rays of all fields and wavelengths",
    ));
}
