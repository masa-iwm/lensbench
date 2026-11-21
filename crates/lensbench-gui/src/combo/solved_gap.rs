//! ソルブ・ピックアップで決まるレイアウト図の寸法（図・欄・キーでは変えない）

use crate::i18n::trf;
use crate::theme;
use egui::{RichText, Ui};
use std::sync::Arc;

/// ソルブで決まる寸法（[`super::View::solved`]）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SolvedGap {
    /// 寸法（経路に沿った要素の後の空気、[`super::STOP_GAP`] は絞りから最初の要素）
    pub gap: usize,
    /// 系の今の値（ソルブが決めた厚み）[mm]
    pub value: f64,
    /// 表の印（ソルブ 'S'・ピックアップ 'P'）と説明（レンズデータの表と同じ）
    pub mark: char,
    pub what: String,
}

impl SolvedGap {
    /// 寸法 g がソルブで決まっていれば、その寸法
    pub(crate) fn find(all: &[SolvedGap], g: usize) -> Option<&SolvedGap> {
        all.iter().find(|x| x.gap == g)
    }

    /// 変えられない理由
    pub(crate) fn reason(&self) -> String {
        trf(
            "{} decides this gap. To change it, remove it in the Lens Data table (right-click the thickness cell)",
            &[&self.what],
        )
    }
}

/// フレームごとの寸法の写し（要素の欄・システムの設定は View を持たないので、egui の文脈から引く）
fn key() -> egui::Id {
    egui::Id::new("lensbench-solved-gaps")
}

impl SolvedGap {
    /// 今のフレームの寸法を置く（App がフレームの始めに）
    pub(crate) fn store(ctx: &egui::Context, all: Vec<SolvedGap>) {
        ctx.data_mut(|d| d.insert_temp(key(), Arc::new(all)));
    }

    /// 今のフレームの寸法 g（置いていなければ無い）
    pub(crate) fn current(ctx: &egui::Context, g: usize) -> Option<SolvedGap> {
        let all = ctx.data(|d| d.get_temp::<Arc<Vec<SolvedGap>>>(key()))?;
        SolvedGap::find(&all, g).cloned()
    }

    /// 入力欄の代わりに、今の値（印付き）と変えられない理由を出す
    pub(crate) fn show(&self, ui: &mut Ui, label: &str) {
        let weak = theme::weak_text(ui.visuals());
        ui.label(RichText::new(label).color(weak));
        ui.label(format!(
            "{} mm {}",
            crate::json_view::fmt_num((self.value * 1e4).round() / 1e4),
            self.mark
        ))
        .on_hover_text(self.reason());
    }
}

impl super::View {
    /// 要素 k の前の間隔がソルブで決まる（光軸に沿って動かすと、後の間隔だけ変わって後ろの要素がずれる）
    pub(crate) fn before_solved(&self, k: usize) -> bool {
        let g = if k == 0 { super::STOP_GAP } else { k - 1 };
        SolvedGap::find(&self.solved, g).is_some()
    }
}
