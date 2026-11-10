//! 解析グラフの操作（#126・#127）: 拡大・縮小・全体表示のボタンと、カーソルの横軸の値での各曲線の値。
//!
//! グラフ（egui_plot）は、ドラッグで移動、Ctrl（Mac は ⌘）＋ホイールかピンチで拡大縮小、
//! 右ドラッグで囲んだ所を拡大、ダブルクリックで全体表示。ホイールだけはパネルのスクロールに使う。

use egui::{Color32, RichText, Ui};
use egui_plot::{Plot, PlotPoint, PlotUi};

use crate::i18n::tr;

/// カーソルの値を出す曲線
pub struct Series {
    pub name: String,
    pub color: Color32,
    pub pts: Vec<[f64; 2]>,
}

impl Series {
    pub fn new(name: impl Into<String>, color: Color32, pts: &[[f64; 2]]) -> Self {
        Series {
            name: name.into(),
            color,
            pts: pts.to_vec(),
        }
    }
}

/// 折れ線 pts の横軸 x での値（線形補間）。x が曲線の範囲の外なら None
pub fn interp(pts: &[[f64; 2]], x: f64) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    pts.windows(2).find_map(|w| {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        if !(lo <= x && x <= hi) || !y0.is_finite() || !y1.is_finite() {
            return None;
        }
        if x1 == x0 {
            return Some(y0);
        }
        Some(y0 + (y1 - y0) * (x - x0) / (x1 - x0))
    })
}

#[derive(Clone, Copy)]
enum Cmd {
    Zoom(f32),
    Reset,
}

/// グラフを描く。body で線を描き、カーソルの値を出す曲線を series に足す。x_unit はカーソルの値の横軸の単位
pub fn show(
    ui: &mut Ui,
    plot: Plot,
    x_unit: &str,
    body: impl FnOnce(&mut PlotUi, &mut Vec<Series>),
) {
    let weak = crate::theme::weak_text(ui.visuals());
    let mut cmd = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if ui.small_button("＋").on_hover_text(tr("拡大")).clicked() {
            cmd = Some(Cmd::Zoom(1.25));
        }
        if ui.small_button("－").on_hover_text(tr("縮小")).clicked() {
            cmd = Some(Cmd::Zoom(0.8));
        }
        if ui
            .small_button(tr("全体"))
            .on_hover_text(tr("全体を表示（ダブルクリックでも）"))
            .clicked()
        {
            cmd = Some(Cmd::Reset);
        }
        ui.label(RichText::new("ⓘ").small().color(weak)).on_hover_text(tr(
            "ドラッグで移動、Ctrl（Mac は ⌘）＋ホイールかピンチで拡大縮小、右ドラッグで囲んだ所を拡大、ダブルクリックで全体",
        ));
    });
    let mut series = Vec::new();
    // 既定の近くの点の座標の札は出さない（カーソルの値の一覧と重なる）
    let r = plot
        .allow_scroll(false)
        .label_formatter(|_, _| String::new())
        .show(ui, |p| {
            body(p, &mut series);
            match cmd {
                Some(Cmd::Zoom(f)) => {
                    let c = p.plot_bounds().center();
                    p.zoom_bounds(egui::Vec2::splat(f), c);
                }
                Some(Cmd::Reset) => p.set_auto_bounds(true),
                None => {}
            }
            // 縦線は線の項目にしない（自動の範囲に入ってしまう）。グラフの外にカーソルがあれば出さない
            p.response()
                .hovered()
                .then(|| p.pointer_coordinate())
                .flatten()
                .map(|q: PlotPoint| q.x)
        });
    if let Some(x) = r.inner.filter(|_| !series.is_empty()) {
        let sx = r.transform.position_from_point_x(x);
        let fr = *r.transform.frame();
        ui.painter_at(fr).line_segment(
            [egui::pos2(sx, fr.top()), egui::pos2(sx, fr.bottom())],
            egui::Stroke::new(1.0_f32, weak),
        );
        readout(ui, fr, x, x_unit, &series);
    }
}

/// カーソルの横軸の値と、各曲線の値をグラフの左上に出す
fn readout(ui: &Ui, rect: egui::Rect, x: f64, x_unit: &str, series: &[Series]) {
    let vals: Vec<(&Series, f64)> = series
        .iter()
        .filter_map(|s| interp(&s.pts, x).map(|y| (s, y)))
        .collect();
    if vals.is_empty() {
        return;
    }
    let painter = ui.painter_at(rect);
    let font = egui::FontId::monospace(11.0);
    let text = ui.visuals().text_color();
    let mut lines = vec![(format!("x = {} {x_unit}", fmt(x)), text)];
    for (s, y) in vals {
        lines.push((format!("{}  {}", fmt(y), s.name), s.color));
    }
    let h = 14.0;
    let w = lines
        .iter()
        .map(|(t, _)| t.chars().count() as f32 * 6.8)
        .fold(0.0, f32::max);
    let origin = rect.left_top() + egui::vec2(8.0, 8.0);
    let bg = egui::Rect::from_min_size(
        origin - egui::vec2(4.0, 2.0),
        egui::vec2(w + 8.0, h * lines.len() as f32 + 4.0),
    );
    painter.rect_filled(bg, 4.0, ui.visuals().extreme_bg_color.gamma_multiply(0.9));
    for (i, (t, c)) in lines.into_iter().enumerate() {
        painter.text(
            origin + egui::vec2(0.0, h * i as f32),
            egui::Align2::LEFT_TOP,
            t,
            font.clone(),
            c,
        );
    }
}

/// 有効数字 5 桁ほど
fn fmt(v: f64) -> String {
    if v == 0.0 || (1e-3..1e5).contains(&v.abs()) {
        let d = (4 - v.abs().log10().floor() as i32).clamp(0, 6) as usize;
        format!("{v:.d$}")
    } else {
        format!("{v:.4e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interp_both_orders_and_outside() {
        let p = [[0.0, 1.0], [10.0, 0.0], [20.0, 0.5]];
        assert_eq!(interp(&p, 5.0), Some(0.5));
        assert_eq!(interp(&p, 15.0), Some(0.25));
        assert_eq!(interp(&p, 25.0), None);
        let q = [[1.0, 0.0], [0.0, 2.0]];
        assert_eq!(interp(&q, 0.25), Some(1.5));
        assert_eq!(fmt(0.123456), "0.12346");
        assert_eq!(fmt(123.456), "123.46");
    }
}
