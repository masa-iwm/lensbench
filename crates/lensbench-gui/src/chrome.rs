//! アプリ版の窓の枠（#138）: OS のタイトルバーを消し、上の帯をタイトルバーにする。
//!
//! - 上の帯の空き所: ドラッグで窓を動かす、ダブルクリックで最大化・戻す
//! - 帯の右端: 最小化・最大化・閉じる
//! - 窓の縁: ドラッグで大きさを変える（OS の枠が無いので自前で `BeginResize` を送る）
//!
//! 設定の「OS のタイトルバーを使う」で元に戻せる。Web 版はブラウザの中なので関係しない。

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use egui::{CursorIcon, ResizeDirection, Sense, Ui, ViewportCommand};

/// 自前の枠を使うか（アプリ版で、OS のタイトルバーを使う設定でないとき）
pub fn custom(os_titlebar: bool) -> bool {
    cfg!(not(target_arch = "wasm32")) && !os_titlebar
}

/// 窓のボタンの絵
#[derive(Clone, Copy)]
enum Glyph {
    Close,
    Maximize,
    Restore,
    Minimize,
}

/// 線で描く窓のボタン（フォントに記号が無くても出るように）
fn glyph_button(ui: &mut Ui, g: Glyph, tip: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(40.0, 28.0), Sense::click());
    let r = r.on_hover_text(tip);
    let vis = ui.visuals();
    let mut fg = vis.text_color();
    if r.hovered() {
        let bg = match g {
            Glyph::Close => {
                fg = egui::Color32::WHITE;
                egui::Color32::from_rgb(196, 43, 28)
            }
            _ => vis.widgets.hovered.weak_bg_fill,
        };
        ui.painter().rect_filled(rect, 4.0, bg);
    }
    let c = rect.center();
    let h = 5.0;
    let st = egui::Stroke::new(1.2_f32, fg);
    let p = ui.painter();
    match g {
        Glyph::Close => {
            p.line_segment([c + egui::vec2(-h, -h), c + egui::vec2(h, h)], st);
            p.line_segment([c + egui::vec2(-h, h), c + egui::vec2(h, -h)], st);
        }
        Glyph::Maximize => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(2.0 * h, 2.0 * h)),
                0.0,
                st,
                egui::StrokeKind::Middle,
            );
        }
        Glyph::Restore => {
            let s = egui::vec2(1.6 * h, 1.6 * h);
            p.rect_stroke(
                egui::Rect::from_center_size(c + egui::vec2(-1.5, 1.5), s),
                0.0,
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([c + egui::vec2(-h + 1.5, -h), c + egui::vec2(h, -h)], st);
            p.line_segment([c + egui::vec2(h, -h), c + egui::vec2(h, h - 1.5)], st);
        }
        Glyph::Minimize => {
            p.line_segment([c + egui::vec2(-h, 0.0), c + egui::vec2(h, 0.0)], st);
        }
    }
    r
}

/// 帯の右端の最小化・最大化・閉じる（右から左へ並べる帯の中で呼ぶ）
pub fn window_buttons(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if glyph_button(ui, Glyph::Close, crate::i18n::tr("閉じる")).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    let (g, tip) = if max {
        (Glyph::Restore, crate::i18n::tr("元のサイズに戻す"))
    } else {
        (Glyph::Maximize, crate::i18n::tr("最大化"))
    };
    if glyph_button(ui, g, tip).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
    if glyph_button(ui, Glyph::Minimize, crate::i18n::tr("最小化")).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    ui.separator();
}

/// 帯の残りの空き所を、窓を動かす所にする（ダブルクリックで最大化・戻す）
pub fn drag_area(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let r = ui.interact(rect, ui.id().with("title_drag"), Sense::click_and_drag());
    if r.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    } else if r.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// 縁からこの幅 [pt] の所で大きさを変える
const EDGE: f32 = 5.0;

/// 窓の中の位置 p が縁のどこか（角を先に見る）
pub fn edge_at(screen: egui::Rect, p: egui::Pos2) -> Option<ResizeDirection> {
    if !screen.contains(p) {
        return None;
    }
    let l = p.x - screen.left() < EDGE;
    let r = screen.right() - p.x < EDGE;
    let t = p.y - screen.top() < EDGE;
    let b = screen.bottom() - p.y < EDGE;
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, _, _, _) => ResizeDirection::West,
        (_, true, _, _) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

/// 窓の縁のカーソルと、押したら大きさを変え始める。最大化中はしない
pub fn resize_edges(ctx: &egui::Context) {
    let (max, pos, pressed, screen) = ctx.input(|i| {
        (
            i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false),
            i.pointer.hover_pos(),
            i.pointer.primary_pressed(),
            i.screen_rect(),
        )
    });
    if max {
        return;
    }
    let Some(d) = pos.and_then(|p| edge_at(screen, p)) else {
        return;
    };
    ctx.set_cursor_icon(match d {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    });
    if pressed {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners() {
        let s = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        assert_eq!(
            edge_at(s, egui::pos2(1.0, 1.0)),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            edge_at(s, egui::pos2(798.0, 599.0)),
            Some(ResizeDirection::SouthEast)
        );
        assert_eq!(
            edge_at(s, egui::pos2(400.0, 2.0)),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            edge_at(s, egui::pos2(799.0, 300.0)),
            Some(ResizeDirection::East)
        );
        assert_eq!(edge_at(s, egui::pos2(400.0, 300.0)), None);
        assert!(!custom(true));
    }
}
