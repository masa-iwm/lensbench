//! パネルが共有する小さな部品（GPN-08）。文字の欄の数の読み、JSON の数の読み、図の拡大・移動の操作（GPN-09）

use egui::{Pos2, Response, Ui, Vec2};
use serde_json::Value;

use crate::i18n::trf;

/// 文字の欄の数。空は None、数でない・有限でない（inf・nan）は誤り（`what` は欄の名前）
pub fn parse_num(s: &str, what: &str) -> Result<Option<f64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .map(Some)
        .ok_or_else(|| trf("{} の値 {} が数でない", &[&what, &t]))
}

/// JSON の数（inf は `{"$f": "inf"}`）。数でなければ None
pub fn json_num(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        return Some(x);
    }
    match v.get("$f").and_then(Value::as_str) {
        Some("inf") => Some(f64::INFINITY),
        Some("-inf") => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// 図の上の拡大・移動の操作（1 フレーム分）。図ごとに同じ操作にそろえる: ホイール（ピンチ）＝ポインタの所で
/// 拡大縮小、中・右ドラッグと Space+左ドラッグ＝移動、ダブルクリック・Home＝全体（GPN-09）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanZoom {
    /// 拡大の倍率（1 は変えない、1 より大きいと拡大）
    pub zoom: f32,
    /// 拡大の中心（画面の座標）
    pub at: Option<Pos2>,
    /// 移動（画面の px）
    pub pan: Vec2,
    /// 全体表示に戻す
    pub reset: bool,
}

/// `resp`（click_and_drag で取った図の場所）の上の操作を読む。`left_pans` は左ドラッグでも移動する図
/// （ガラスマップなど、左ドラッグにほかの役が無いもの）。ホイールを使ったら外のスクロールに渡さない
pub fn pan_zoom(ui: &Ui, resp: &Response, left_pans: bool) -> PanZoom {
    let mut out = PanZoom {
        zoom: 1.0,
        at: None,
        pan: Vec2::ZERO,
        reset: false,
    };
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    let pan_btn = resp.dragged_by(egui::PointerButton::Middle)
        || resp.dragged_by(egui::PointerButton::Secondary)
        || (resp.dragged_by(egui::PointerButton::Primary) && (left_pans || space));
    if pan_btn {
        out.pan = resp.drag_delta();
    }
    out.reset =
        resp.double_clicked() || (resp.hovered() && ui.input(|i| i.key_pressed(egui::Key::Home)));
    if let Some(p) = resp.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let f = (scroll * 0.003).exp() * pinch;
        if (f - 1.0).abs() > 1e-6 {
            out.zoom = f;
            out.at = Some(p);
            ui.ctx().input_mut(|i| {
                i.smooth_scroll_delta = Vec2::ZERO;
                i.raw_scroll_delta = Vec2::ZERO;
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_num_rejects_non_finite() {
        assert_eq!(parse_num(" 2.5 ", "x"), Ok(Some(2.5)));
        assert_eq!(parse_num("  ", "x"), Ok(None));
        for s in ["inf", "-inf", "nan", "NaN", "abc"] {
            assert!(parse_num(s, "x").is_err(), "{s}");
        }
    }

    #[test]
    fn json_num_reads_inf() {
        assert_eq!(json_num(&json!(3)), Some(3.0));
        assert_eq!(json_num(&json!({"$f": "inf"})), Some(f64::INFINITY));
        assert_eq!(json_num(&json!({"$f": "-inf"})), Some(f64::NEG_INFINITY));
        assert_eq!(json_num(&json!("x")), None);
    }
}
