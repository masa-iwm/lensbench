//! パネルが共有する小さな部品。文字の欄・表のセルの数の読み、補完の並べ方、JSON の数の読み、図の拡大・移動の操作

use egui::{Pos2, Response, Ui, Vec2};
use serde_json::Value;

use crate::i18n::trf;

pub(crate) mod cell;
pub(crate) mod choices;
pub(crate) mod menu;
pub(crate) mod name_edit;
pub(crate) mod num;
pub(crate) mod scale_bar;
pub(crate) mod sheet;
pub(crate) use num::Num;
#[cfg(test)]
mod ui_tests;

/// 自前で描く図（レイアウト・スポット・ガラスマップなど）に名前を付ける（AccessKit の木に画像として出て、
/// 読み上げ・画面の試験が図を探せる）。name は図の種類と、何の図かが分かる短い文を作る関数（AccessKit が
/// 要るときだけ呼ばれ、要らない exe では文字列を作らない）。name は egui の Context を握ったまま呼ばれるので、中で
/// ui・ctx（data・style など）を読まない（読むと止まる）
pub(crate) fn name_figure(r: &egui::Response, name: impl Fn() -> String) {
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, r.enabled(), name()));
}

/// 見出しの無い入力欄（薄い字の例だけのもの）に名前を付ける（AccessKit の木に出て、読み上げ・画面の試験が欄を
/// 探せる）。egui の TextEdit は薄い字を木に出さないので、その文を名前にし、値は欄の文字のまま
pub(crate) fn name_field(r: &egui::Response, name: &str, text: &str) {
    r.widget_info(|| {
        let mut w = egui::WidgetInfo::labeled(egui::Role::TextInput, r.enabled(), name);
        w.current_text_value = Some(text.to_owned());
        w
    });
}

/// 折り返す行（`horizontal_wrapped`）で、幅 w の塊が行の残りに入らなければ先に折り返す。ComboBox や組にした
/// 部品は行の端で折り返されず、パネルの右で切れるので、その前に呼ぶ
pub(crate) fn wrap_before(ui: &mut egui::Ui, w: f32) {
    // 折り返す行の available_width は折り返した後の幅（行の全幅）を返すので、行の残りはカーソルから測る
    // 行の先頭（左端）では折り返さない（塊がパネルより広いと空の行ができる）
    let at_start = ui.cursor().left() <= ui.max_rect().left() + 0.5;
    if !at_start && ui.max_rect().right() - ui.cursor().left() < w {
        ui.end_row();
    }
}

/// [`wrap_before`] の、ComboBox n 個（とその前の名前 label_w）の幅
pub(crate) fn combos_width(ui: &egui::Ui, n: usize, label_w: f32) -> f32 {
    let s = ui.spacing();
    n as f32 * (s.combo_width + s.item_spacing.x) + label_w
}

/// 文字の欄の数。空は None、数でない・有限でない（inf・nan）は誤り（`what` は欄の名前）
pub(crate) fn parse_num(s: &str, what: &str) -> Result<Option<f64>, String> {
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

/// 横に流す表の下の余白（横のスクロールバーが最後の行に重ならない）。ScrollArea::horizontal の中、表の後に置く
pub(crate) fn scroll_gap(ui: &mut egui::Ui) {
    let s = ui.spacing().scroll;
    ui.add_space(s.bar_width + s.bar_inner_margin + s.bar_outer_margin + 2.0);
}

/// 候補の並べ方: 前方一致なら Some(true)、名前か説明の部分一致なら Some(false)、合わなければ None。
/// 名前・説明・打った文字は同じ大小（小文字か大文字）に揃えて渡す（補完つきの欄で共通）
pub(crate) fn match_rank(name: &str, desc: &str, q: &str) -> Option<bool> {
    if name.starts_with(q) {
        Some(true)
    } else if name.contains(q) || desc.contains(q) {
        Some(false)
    } else {
        None
    }
}

/// JSON の数（inf は `{"$f": "inf"}`）。数でなければ None
pub(crate) fn json_num(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        return Some(x);
    }
    match v.get("$f").and_then(Value::as_str) {
        Some("inf") => Some(f64::INFINITY),
        Some("-inf") => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// 図・グラフの上の拡大・移動の操作（1 フレーム分）。図（レイアウト・スポット・ガラスマップ）も解析のグラフも同じ操作:
/// Ctrl（Mac は ⌘）＋ホイールかピンチ＝ポインタの所で拡大縮小（素のホイールはパネルのスクロール）、中・右ドラッグと
/// Space+左ドラッグ＝移動、ダブルクリック・Home＝全体。右下の道具（ポインタが図の上にあるとき。右上はグラフの凡例）＝
/// ＋・－（押すたびに [`ZOOM_STEP`] 倍）・囲んで拡大・全体。囲んで拡大は Ctrl（⌘）＋左ドラッグでも
///
/// 図は `pan` を先に足し、それから `at` を中心に `zoom` 倍する（囲んで拡大は、囲んだ所の中心を図の中心へ動かして拡大する）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PanZoom {
    /// 拡大の倍率（1 は変えない、1 より大きいと拡大）
    pub zoom: f32,
    /// 拡大の中心（画面の座標）
    pub at: Option<Pos2>,
    /// 移動（画面の px）
    pub pan: Vec2,
    /// 全体表示に戻す
    pub reset: bool,
    /// 囲んで拡大で囲んだ枠（画面）。縦横を別に決められる図（ガラスマップ）は pan・zoom の代わりにこれを使う
    pub area: Option<egui::Rect>,
}

/// ＋・－のボタン 1 回の倍率
pub(crate) const ZOOM_STEP: f32 = 1.25;

/// 図の右上の道具
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Tool {
    In,
    Out,
    Area,
    Fit,
}

/// 囲んで拡大の状態（図ごとに egui の一時データ）
#[derive(Debug, Clone, Copy, Default)]
struct BoxZoom {
    /// 「囲んで拡大」を押した（次の左ドラッグで囲む。1 回囲んだら戻る）
    armed: bool,
    /// 囲んでいる間: 始めた所と今の所
    drag: Option<(Pos2, Pos2)>,
    /// 囲む途中で Esc を押した（ボタンを離すまでのドラッグは何もしない）
    cancelled: bool,
}

fn box_id(resp: &Response) -> egui::Id {
    resp.id.with("box_zoom")
}

fn box_state(ui: &Ui, resp: &Response) -> BoxZoom {
    ui.data(|d| d.get_temp(box_id(resp)).unwrap_or_default())
}

/// 道具のボタンの場所（右下から左へ。右上はグラフの凡例）。図が狭すぎるときは出さない
fn tool_rects(rect: egui::Rect) -> Vec<(Tool, egui::Rect)> {
    const H: f32 = 22.0;
    const GAP: f32 = 2.0;
    let widths = [
        (Tool::In, H),
        (Tool::Out, H),
        (Tool::Area, H),
        (Tool::Fit, 44.0),
    ];
    let total: f32 = widths.iter().map(|w| w.1).sum::<f32>() + GAP * 3.0;
    if rect.width() < total + 16.0 || rect.height() < H + 16.0 {
        return Vec::new();
    }
    let mut x = rect.right() - 8.0 - total;
    widths
        .iter()
        .map(|&(t, w)| {
            let r =
                egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - 8.0 - H), egui::vec2(w, H));
            x += w + GAP;
            (t, r)
        })
        .collect()
}

/// 道具を出すか（ポインタが図の上にあるか、囲んで拡大を待っているとき）
fn tools_shown(ui: &Ui, resp: &Response, st: &BoxZoom) -> bool {
    st.armed || st.drag.is_some() || ui.rect_contains_pointer(resp.rect)
}

/// 囲んで拡大のドラッグ中か（図は左ドラッグのほかの役（選ぶ・動かす・回す・移動）をしない）
pub(crate) fn box_zooming(ui: &Ui, resp: &Response) -> bool {
    box_state(ui, resp).drag.is_some()
}

/// 移動のドラッグ中か（中・右ドラッグ、Space+左ドラッグ、`left_pans` なら左ドラッグも。囲んで拡大の間は除く）
pub(crate) fn pan_zoom_pans(ui: &Ui, resp: &Response, left_pans: bool) -> bool {
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    resp.dragged_by(egui::PointerButton::Middle)
        || resp.dragged_by(egui::PointerButton::Secondary)
        || (resp.dragged_by(egui::PointerButton::Primary)
            && (left_pans || space)
            && !box_zooming(ui, resp))
}

/// 囲んだ所 b を図の場所 rect いっぱいに広げる操作（b の中心を rect の中心へ動かし、rect の中心で拡大）
pub(crate) fn zoom_to_box(rect: egui::Rect, b: egui::Rect) -> Option<PanZoom> {
    if b.width() < 4.0 || b.height() < 4.0 {
        return None;
    }
    Some(PanZoom {
        zoom: (rect.width() / b.width()).min(rect.height() / b.height()),
        at: Some(rect.center()),
        pan: rect.center() - b.center(),
        reset: false,
        area: Some(b),
    })
}

/// `resp`（click_and_drag で取った図の場所）の上の操作を読む。`left_pans` は左ドラッグでも移動する図
/// （ガラスマップ・グラフなど、左ドラッグにほかの役が無いもの）。素のホイールは外のスクロールに残し、Ctrl（⌘）＋ホイール・
/// ピンチで拡大したら外に渡さない。右下の道具と囲んだ枠は、図を描いた後に [`paint_zoom_tools`] で描く
pub(crate) fn pan_zoom(ui: &Ui, resp: &Response, left_pans: bool) -> PanZoom {
    let mut out = PanZoom {
        zoom: 1.0,
        at: None,
        pan: Vec2::ZERO,
        reset: false,
        area: None,
    };
    let rect = resp.rect;
    let mut st = box_state(ui, resp);
    if tools_shown(ui, resp, &st) {
        for (t, b) in tool_rects(rect) {
            let tip = tool_tip(t);
            let r = ui.interact(b, resp.id.with(("zoom_tool", t)), egui::Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
            let r = r.on_hover_text(tip);
            if r.clicked() {
                match t {
                    Tool::In | Tool::Out => {
                        out.zoom = if t == Tool::In {
                            ZOOM_STEP
                        } else {
                            1.0 / ZOOM_STEP
                        };
                        out.at = Some(rect.center());
                    }
                    Tool::Area => st.armed = !st.armed,
                    Tool::Fit => out.reset = true,
                }
            }
        }
    }
    if (st.armed || st.drag.is_some()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.armed = false;
        st.cancelled = st.drag.is_some();
    }
    // 囲んで拡大（「囲んで拡大」を押した後か Ctrl・⌘ を押しての左ドラッグ）
    let started = resp.drag_started_by(egui::PointerButton::Primary)
        && (st.armed || ui.input(|i| i.modifiers.command));
    if started {
        let p0 = ui
            .input(|i| i.pointer.press_origin())
            .or(resp.interact_pointer_pos());
        st.drag = p0.map(|p| (p, resp.interact_pointer_pos().unwrap_or(p)));
    } else if let Some((p0, p1)) = st.drag {
        if resp.dragged_by(egui::PointerButton::Primary) {
            st.drag = Some((p0, resp.interact_pointer_pos().unwrap_or(p1)));
        } else {
            let p1 = resp.interact_pointer_pos().unwrap_or(p1);
            let cancelled = st.cancelled;
            st = BoxZoom::default();
            if let Some(z) =
                zoom_to_box(rect, egui::Rect::from_two_pos(p0, p1)).filter(|_| !cancelled)
            {
                out = z;
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(box_id(resp), st));
    if st.drag.is_none() && pan_zoom_pans(ui, resp, left_pans) {
        out.pan = resp.drag_delta();
    }
    out.reset |=
        resp.double_clicked() || (resp.hovered() && ui.input(|i| i.key_pressed(egui::Key::Home)));
    if let Some(p) = resp.hover_pos() {
        if let Some(f) = take_wheel_zoom(ui) {
            out.zoom = f;
            out.at = Some(p);
        }
    }
    if (st.armed || st.drag.is_some()) && ui.rect_contains_pointer(rect) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    out
}

/// このフレームの Ctrl（⌘）＋ホイールとピンチの拡大の倍率（素のホイールは読まず外のスクロールに残す）。拡大したら外に渡さない
pub(crate) fn take_wheel_zoom(ui: &Ui) -> Option<f32> {
    // Ctrl（⌘）＋ホイールとピンチは egui が zoom_delta にする（そのときスクロールの量は 0）
    let f = ui.input(|i| i.zoom_delta());
    if (f - 1.0).abs() <= 1e-6 {
        return None;
    }
    ui.ctx().input_mut(|i| {
        i.smooth_scroll_delta = Vec2::ZERO;
    });
    Some(f)
}

fn tool_tip(t: Tool) -> &'static str {
    match t {
        Tool::In => crate::i18n::tr("Zoom in"),
        Tool::Out => crate::i18n::tr("Zoom out"),
        Tool::Area => crate::i18n::tr("Zoom to area: drag a box (also Ctrl+drag, ⌘+drag on Mac)"),
        Tool::Fit => crate::i18n::tr("Fit all (also Home, or double-click an empty spot)"),
    }
}

/// [`pan_zoom`] の右上の道具と、囲んでいる枠を描く（図を描いた後に呼ぶ）
pub(crate) fn paint_zoom_tools(ui: &Ui, resp: &Response) {
    let st = box_state(ui, resp);
    let p = ui.painter_at(resp.rect);
    if let Some((p0, p1)) = st.drag.filter(|_| !st.cancelled) {
        let b = egui::Rect::from_two_pos(p0, p1);
        let c = ui.visuals().selection.bg_fill;
        p.rect(
            b,
            0.0,
            c.gamma_multiply(0.15),
            egui::Stroke::new(1.0_f32, c),
            egui::StrokeKind::Inside,
        );
    }
    if !tools_shown(ui, resp, &st) {
        return;
    }
    for (t, b) in tool_rects(resp.rect) {
        let id = resp.id.with(("zoom_tool", t));
        let w = match ui.ctx().read_response(id) {
            Some(r) => ui
                .style()
                .interact_selectable(&r, t == Tool::Area && st.armed),
            None => ui.visuals().widgets.inactive,
        };
        p.rect(
            b,
            4.0,
            w.weak_bg_fill,
            w.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let font = egui::FontId::proportional(if t == Tool::Fit { 12.0 } else { 14.0 });
        let col = w.text_color();
        match t {
            Tool::Area => {
                // 点線の四角（字形に依らない）
                let r = b.shrink(6.0);
                let pts = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                p.extend(egui::Shape::dashed_line(
                    &pts,
                    egui::Stroke::new(1.0_f32, col),
                    2.0,
                    2.0,
                ));
            }
            Tool::In | Tool::Out => {
                // ＋・－（字形に依らない）
                let c = b.center();
                let h = 5.0;
                let s = egui::Stroke::new(1.5_f32, col);
                p.line_segment([c - egui::vec2(h, 0.0), c + egui::vec2(h, 0.0)], s);
                if t == Tool::In {
                    p.line_segment([c - egui::vec2(0.0, h), c + egui::vec2(0.0, h)], s);
                }
            }
            Tool::Fit => {
                p.text(
                    b.center(),
                    egui::Align2::CENTER_CENTER,
                    crate::i18n::tr("Fit"),
                    font,
                    col,
                );
            }
        }
    }
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
    fn match_rank_orders() {
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
