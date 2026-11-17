//! パネルが共有する小さな部品。文字の欄・表のセルの数の読み、補完の並べ方、JSON の数の読み、図の拡大・移動の操作

use egui::{Pos2, Response, Ui, Vec2};
use serde_json::Value;

use crate::i18n::trf;

pub mod cell;
pub mod choices;
pub mod menu;
#[cfg(test)]
mod ui_tests;

/// 折り返す行（`horizontal_wrapped`）で、幅 w の塊が行の残りに入らなければ先に折り返す。ComboBox や組にした
/// 部品は行の端で折り返されず、パネルの右で切れるので、その前に呼ぶ
pub fn wrap_before(ui: &mut egui::Ui, w: f32) {
    // 折り返す行の available_width は折り返した後の幅（行の全幅）を返すので、行の残りはカーソルから測る
    // 行の先頭（左端）では折り返さない（塊がパネルより広いと空の行ができる）
    let at_start = ui.cursor().left() <= ui.max_rect().left() + 0.5;
    if !at_start && ui.max_rect().right() - ui.cursor().left() < w {
        ui.end_row();
    }
}

/// [`wrap_before`] の、ComboBox n 個（とその前の名前 label_w）の幅
pub fn combos_width(ui: &egui::Ui, n: usize, label_w: f32) -> f32 {
    let s = ui.spacing();
    n as f32 * (s.combo_width + s.item_spacing.x) + label_w
}

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
        .ok_or_else(|| trf("Value of {} is not a number: {}", &[&what, &t]))
}

/// 表のセルの数。inf・infinity（符号つきも）は無限大、空・数でない・nan は誤り（多重構成の表など）
pub fn parse_cell(s: &str) -> Result<f64, String> {
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "inf" | "infinity" | "+inf" | "+infinity" => Ok(f64::INFINITY),
        "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
        _ => t
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| trf("Not a number: {}", &[&t])),
    }
}

/// 候補の並べ方: 前方一致なら Some(true)、名前か説明の部分一致なら Some(false)、合わなければ None。
/// 名前・説明・打った文字は同じ大小（小文字か大文字）に揃えて渡す（補完つきの欄で共通）
pub fn match_rank(name: &str, desc: &str, q: &str) -> Option<bool> {
    if name.starts_with(q) {
        Some(true)
    } else if name.contains(q) || desc.contains(q) {
        Some(false)
    } else {
        None
    }
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
/// 拡大縮小、中・右ドラッグと Space+左ドラッグ＝移動、ダブルクリック・Home＝全体
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

/// 移動のドラッグ中か（中・右ドラッグ、Space+左ドラッグ、`left_pans` なら左ドラッグも）
pub fn pan_zoom_pans(ui: &Ui, resp: &Response, left_pans: bool) -> bool {
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    resp.dragged_by(egui::PointerButton::Middle)
        || resp.dragged_by(egui::PointerButton::Secondary)
        || (resp.dragged_by(egui::PointerButton::Primary) && (left_pans || space))
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
    if pan_zoom_pans(ui, resp, left_pans) {
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
    fn parse_cell_reads_inf() {
        assert_eq!(parse_cell(" inf "), Ok(f64::INFINITY));
        assert_eq!(parse_cell("-Infinity"), Ok(f64::NEG_INFINITY));
        assert_eq!(parse_cell("2.5"), Ok(2.5));
        for s in ["", "nan", "x"] {
            assert!(parse_cell(s).is_err(), "{s}");
        }
        assert_eq!(match_rank("n-bk7", "schott", "n-b"), Some(true));
        assert_eq!(match_rank("n-bk7", "schott", "bk"), Some(false));
        assert_eq!(match_rank("n-bk7", "schott", "sch"), Some(false));
        assert_eq!(match_rank("n-bk7", "schott", "zz"), None);
    }

    #[test]
    fn json_num_reads_inf() {
        assert_eq!(json_num(&json!(3)), Some(3.0));
        assert_eq!(json_num(&json!({"$f": "inf"})), Some(f64::INFINITY));
        assert_eq!(json_num(&json!({"$f": "-inf"})), Some(f64::NEG_INFINITY));
        assert_eq!(json_num(&json!("x")), None);
    }
}

/// 見るだけの図の拡大（対数）と移動（パネルの状態に持つ）
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Nav {
    pub zoom_log: f32,
    pub pan: egui::Vec2,
}
