//! キーボード（Shift+F10）で開く右クリックの欄（レンズデータ・多重構成で共通）。
//! 中身は右クリックと同じ関数で描き、選んだ・Esc・外のクリックで閉じる

use egui::{Id, Key, Pos2, Ui};

/// 右クリックの欄を開くキー（Shift+F10）が押された
pub(crate) fn menu_key(ui: &Ui) -> bool {
    ui.input(|i| i.modifiers.shift && i.key_pressed(Key::F10))
}

/// `pos` に浮かせた欄を出す。`add` は中身を描き、何かを選んだら true を返す。
/// 戻り値は欄を閉じるとき true（選んだ・Esc・外のクリック。`fresh`（開いたフレーム）の外のクリックは数えない）
pub(crate) fn float_menu(
    ui: &Ui,
    id: Id,
    pos: Pos2,
    max_width: f32,
    fresh: bool,
    add: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let mut done = false;
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_max_width(max_width);
                done = add(ui);
            });
        });
    done || ui.input(|i| i.key_pressed(Key::Escape))
        || (!fresh && area.response.clicked_elsewhere())
}
