//! 操作の履歴のパネル: 元に戻す・やり直す段を 1 本の時間の並びで見せ、行のクリックでその段まで一度に戻す・進める
//! （編集メニューの「ここまで元に戻す」と同じ段）

use super::super::*;
use super::Step;
use crate::theme;

impl Core {
    pub(in crate::app) fn history_ui(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        let undo = self.undo_list(true);
        let redo = self.undo_list(false);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.can_undo(true), egui::Button::new(tr("Undo (Ctrl+Z)")))
                .clicked()
            {
                self.msgs.push(Msg::Undo(true, 1));
            }
            if ui
                .add_enabled(self.can_undo(false), egui::Button::new(tr("Redo (Ctrl+Y)")))
                .clicked()
            {
                self.msgs.push(Msg::Undo(false, 1));
            }
        });
        ui.label(
            RichText::new(tr(
                "Click a step to go back (or forward) to just before it. Newest at the top; redo steps above Now",
            ))
            .small()
            .color(weak),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("history_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // やり直す段（先のものほど上）。行を押すとその段まで進める
                for (i, l) in redo.iter().enumerate().rev() {
                    if let Some(n) = step_row(ui, l, i, false, weak) {
                        self.msgs.push(Msg::Undo(false, n));
                    }
                }
                ui.label(RichText::new(tr("▶ Now")).strong());
                // 元に戻す段（新しい順）。行を押すとその段を戻す（その段の前へ）
                for (i, l) in undo.iter().enumerate() {
                    if let Some(n) = step_row(ui, l, i, true, weak) {
                        self.msgs.push(Msg::Undo(true, n));
                    }
                }
                if undo.is_empty() && redo.is_empty() {
                    ui.label(RichText::new(tr("No steps yet")).color(weak));
                }
            });
    }
}

/// 段の 1 行（やり直す段は薄く）。押したら戻す・進める段の数
fn step_row(ui: &mut Ui, l: &Step, i: usize, back: bool, weak: Color32) -> Option<usize> {
    // 文書を替えた段（新規・開く）の前で区切る（その先は前の文書）
    if back && l.doc && i > 0 {
        ui.separator();
    }
    let mut t = RichText::new(format!("{}   {}", l.title, l.age()));
    if !back {
        t = t.color(weak).italics();
    }
    let r = ui.selectable_label(false, t);
    let r = if l.details.len() > 1 {
        r.on_hover_text(l.tip())
    } else {
        r
    };
    if !back && l.doc && i > 0 {
        ui.separator();
    }
    r.clicked().then_some(i + 1)
}
