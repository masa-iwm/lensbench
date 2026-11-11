//! パネルの中を 2 つに分ける（境をドラッグで動かす）。
//!
//! 上の欄を高さの割合で取ると、パネルが低いとき下の欄（面の設定など）が押し出されて見えなくなる。
//! ここでは残りの高さを 2 つに分け、どちらも最低の大きさを残す。横に広くて低いパネルでは左右に並べる。
//! 境はドラッグで動かせ（割合はパネルごとに覚える）、ダブルクリックで既定に戻す。

use egui::{CursorIcon, Id, Rect, Sense, Ui, UiBuilder};

/// 境の太さ
const GAP: f32 = 7.0;

/// 左右に並べるか（横に広く低い）
pub fn side_by_side(w: f32, h: f32) -> bool {
    w >= 700.0 && w >= 2.5 * h
}

/// 残りの領域を 2 つに分けて、前（上・左）と後（下・右）の Ui を返す。
/// `frac` は前の既定の割合（上下、左右）、`min` は各欄に残す最低の大きさ（領域が足りなければ半分ずつ）。
/// 境を動かすまでは既定の割合（呼ぶたびに変わってよい）、動かした後はその割合を覚える。
pub fn two(ui: &mut Ui, id: impl std::hash::Hash, frac: (f32, f32), min: f32) -> (Ui, Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let side = side_by_side(rect.width(), rect.height());
    let key = Id::new(("split", id, side));
    let len = (if side { rect.width() } else { rect.height() } - GAP).max(0.0);
    let lo = min.min(len * 0.5);
    let frac = if side { frac.1 } else { frac.0 };
    let f = ui.data_mut(|d| d.get_persisted::<f32>(key)).unwrap_or(frac);
    let mut a = (f * len).clamp(lo, len - lo);
    let hrect = |a: f32| {
        if side {
            Rect::from_min_max(
                egui::pos2(rect.left() + a, rect.top()),
                egui::pos2(rect.left() + a + GAP, rect.bottom()),
            )
        } else {
            Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + a),
                egui::pos2(rect.right(), rect.top() + a + GAP),
            )
        }
    };
    let resp = ui.interact(hrect(a), key.with("handle"), Sense::click_and_drag());
    if resp.dragged() {
        let d = resp.drag_delta();
        a = (a + if side { d.x } else { d.y }).clamp(lo, len - lo);
        if len > 0.0 {
            ui.data_mut(|m| m.insert_persisted(key, a / len));
        }
    }
    if resp.double_clicked() {
        a = (frac * len).clamp(lo, len - lo);
        ui.data_mut(|m| m.remove::<f32>(key));
    }
    let resp = resp.on_hover_text(crate::i18n::tr(
        "ドラッグで大きさを変える（ダブルクリックで元に戻す）",
    ));
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(if side {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
    }
    let v = ui.visuals();
    let stroke = if resp.hovered() || resp.dragged() {
        v.widgets.active.fg_stroke
    } else {
        v.widgets.noninteractive.bg_stroke
    };
    let h = hrect(a);
    if side {
        ui.painter().vline(h.center().x, h.y_range(), stroke);
    } else {
        ui.painter().hline(h.x_range(), h.center().y, stroke);
    }
    let (ra, rb) = if side {
        (
            Rect::from_min_max(rect.min, egui::pos2(h.left(), rect.bottom())),
            Rect::from_min_max(egui::pos2(h.right(), rect.top()), rect.max),
        )
    } else {
        (
            Rect::from_min_max(rect.min, egui::pos2(rect.right(), h.top())),
            Rect::from_min_max(egui::pos2(rect.left(), h.bottom()), rect.max),
        )
    };
    let child = |ui: &mut Ui, r: Rect, salt: &str| {
        let mut c = ui.new_child(
            UiBuilder::new()
                .id_salt((key, salt))
                .max_rect(r)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        c.set_clip_rect(r.intersect(ui.clip_rect()));
        c
    };
    (child(ui, ra, "a"), child(ui, rb, "b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_and_low_panels_go_side_by_side() {
        assert!(side_by_side(850.0, 210.0));
        assert!(!side_by_side(850.0, 500.0));
        assert!(!side_by_side(600.0, 100.0));
    }
}
