//! 表のセルのその場の入力欄（レンズデータ・多重構成・組合せの寸法で共通）。
//! 開いた最初のフレームでフォーカスを取り、Enter・F2・ダブルクリックで始めたら全部を選び、打った文字から始めたら
//! 後ろにカーソルを置く。Tab・↑↓ は欄の外へフォーカスを移さず、呼んだ側が決めて隣のセルへ移る

use egui::{Key, Response, Ui};

use crate::i18n::trf;

/// 入力欄が受けたキー（このフレーム）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellKeys {
    pub tab: bool,
    pub shift: bool,
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub esc: bool,
}

impl CellKeys {
    pub fn read(ui: &Ui) -> CellKeys {
        ui.input(|i| CellKeys {
            tab: i.key_pressed(Key::Tab),
            shift: i.modifiers.shift,
            up: i.key_pressed(Key::ArrowUp),
            down: i.key_pressed(Key::ArrowDown),
            enter: i.key_pressed(Key::Enter),
            esc: i.key_pressed(Key::Escape),
        })
    }

    /// 決めて移る先（行・列の差）。Tab は横（Shift で前）、arrows なら ↑↓ で縦
    pub fn nav(&self, arrows: bool) -> Option<(i32, i32)> {
        if self.tab {
            Some((0, if self.shift { -1 } else { 1 }))
        } else if arrows && self.up {
            Some((-1, 0))
        } else if arrows && self.down {
            Some((1, 0))
        } else {
            None
        }
    }
}

/// 入力欄を開いた最初のフレーム: フォーカスを取り、select_all なら全部を選ぶ（でなければ後ろにカーソル）
pub fn start(ui: &Ui, resp: &Response, text: &str, select_all: bool) {
    resp.request_focus();
    if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        let n = text.chars().count();
        let from = if select_all { 0 } else { n };
        s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(from),
            egui::text::CCursor::new(n),
        )));
        s.store(ui.ctx(), resp.id);
    }
}

/// 決められなかった理由を、欄の赤枠と下の吹き出しで出す（Esc でやめる）
pub fn show_error(ui: &Ui, resp: &Response, why: &str) {
    ui.painter().rect_stroke(
        resp.rect.expand(1.0),
        2.0,
        egui::Stroke::new(1.5_f32, ui.visuals().error_fg_color),
        egui::StrokeKind::Outside,
    );
    egui::Area::new(resp.id.with("why"))
        .order(egui::Order::Tooltip)
        .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 2.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    trf("⚠ {} (Esc to cancel)", &[&why]),
                );
            });
        });
}

/// 表のセルの入力欄（セルの幅いっぱい、right なら右寄せ）。started は開いた後の最初のフレームを済ませたか、
/// error は前に決められなかった理由。キーは欄を描く前に読む
pub fn input(
    ui: &mut Ui,
    text: &mut String,
    started: &mut bool,
    select_all: bool,
    right: bool,
    error: Option<&str>,
) -> (Response, CellKeys) {
    let keys = CellKeys::read(ui);
    let te = egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .lock_focus(true);
    let te = if right {
        te.horizontal_align(egui::Align::RIGHT)
    } else {
        te
    };
    let resp = ui.add(te);
    if let Some(why) = error {
        show_error(ui, &resp, why);
    }
    if !*started {
        start(ui, &resp, text, select_all);
        *started = true;
    }
    (resp, keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_follows_keys() {
        let k = |tab, shift, up, down| CellKeys {
            tab,
            shift,
            up,
            down,
            ..Default::default()
        };
        assert_eq!(k(true, false, false, false).nav(true), Some((0, 1)));
        assert_eq!(k(true, true, false, false).nav(false), Some((0, -1)));
        assert_eq!(k(false, false, true, false).nav(true), Some((-1, 0)));
        assert_eq!(k(false, false, true, false).nav(false), None);
        assert_eq!(k(false, false, false, true).nav(true), Some((1, 0)));
    }

    /// 開いた最初のフレームで全部を選ぶ・後ろにカーソルを置く
    #[test]
    fn start_selects() {
        let ctx = egui::Context::default();
        for all in [true, false] {
            let mut text = "12.5".to_string();
            let mut started = false;
            let mut id = None;
            for _ in 0..2 {
                let _ = ctx.run(Default::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (r, _) = input(ui, &mut text, &mut started, all, true, None);
                        id = Some(r.id);
                    });
                });
            }
            let s = egui::TextEdit::load_state(&ctx, id.unwrap()).unwrap();
            let r = s.cursor.char_range().unwrap();
            let (a, b) = (
                r.primary.index.min(r.secondary.index),
                r.primary.index.max(r.secondary.index),
            );
            assert_eq!((a, b), if all { (0, 4) } else { (4, 4) });
            assert!(started);
        }
    }
}
