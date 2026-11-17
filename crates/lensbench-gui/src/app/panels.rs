//! パネルの状態（文書モデルの 3 段目）。各パネルの型はそれぞれのファイルにあり、ここはその置き場だけ

use super::*;

/// パネルの状態
pub(super) struct Panels {
    /// 面の表: 状態、面の値、玉・群、木
    pub(super) ld: lens_data::LensData,
    /// 組合せのレイアウト図の選択・拡大・ドラッグ
    pub(super) combo_view: combo::View,
    /// 組合せの状態の表（組合せでは多重構成の表の代わり）
    pub(super) states: crate::state_table::View,
    /// 組合せの要素をユーザーカタログに保存する窓
    pub(super) cat_save: CatSave,
    /// 未完のカタログ品を補う窓
    pub(super) fill: crate::fill::FillWin,
    /// 最適化のパネル
    pub(super) opt: crate::optim_panel::OptPanel,
    /// 差し替え探索のパネル
    pub(super) swap: crate::swap_panel::SwapPanel,
    /// モンテカルロ公差のパネル
    pub(super) tol: crate::tol_panel::TolPanel,
    /// ズームのパネル
    pub(super) zoom: crate::zoom_panel::ZoomPanel,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            ld: Default::default(),
            combo_view: Default::default(),
            states: Default::default(),
            cat_save: CatSave::default(),
            fill: Default::default(),
            opt: crate::optim_panel::OptPanel::new(),
            swap: Default::default(),
            tol: Default::default(),
            zoom: Default::default(),
        }
    }
}
