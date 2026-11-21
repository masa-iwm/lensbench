//! 解析グラフの操作（図と同じ拡大・移動・囲んで拡大・全体、目盛りの上でその軸だけ）と、カーソルの横軸の値での各曲線の値。

use egui::{Color32, Ui};
use egui_plot::{Plot, PlotBounds, PlotPoint, PlotResponse, PlotTransform, PlotUi};

/// カーソルの値を出す曲線
pub(crate) struct Series {
    pub name: String,
    pub color: Color32,
    pub pts: Vec<[f64; 2]>,
}

impl Series {
    pub(crate) fn new(name: impl Into<String>, color: Color32, pts: &[[f64; 2]]) -> Self {
        Series {
            name: name.into(),
            color,
            pts: pts.to_vec(),
        }
    }
}

/// 折れ線 pts の横軸 x での値（線形補間）。x が曲線の範囲の外なら None
pub(crate) fn interp(pts: &[[f64; 2]], x: f64) -> Option<f64> {
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

/// 次のフレームのグラフに当てる範囲の変更（egui_plot の範囲は描く関数の中でしか変えられないので、
/// 描いた後に読んだ操作を次のフレームで当てる）
#[derive(Clone, Copy)]
enum Pending {
    Bounds(PlotBounds),
    Fit,
}

fn pending_id(resp: &egui::Response) -> egui::Id {
    resp.id.with("plot_nav")
}

/// グラフ（egui_plot）を図（[`crate::widgets::pan_zoom`]）と同じ操作で描く: Ctrl（⌘）＋ホイールかピンチ＝ポインタの所で
/// 拡大縮小（素のホイールはパネルのスクロール）、左・中・右ドラッグと Space+左ドラッグ＝移動、
/// 右下の道具（＋・－・囲んで拡大・全体。右上は凡例）、Ctrl（⌘）+左ドラッグ＝囲んで拡大、ダブルクリック・Home＝全体。
/// `free_axes` なら横軸・縦軸の目盛りの上の Ctrl（⌘）＋ホイールでその軸だけ拡大縮小し、
/// 目盛りの上のドラッグでその軸だけ動かす（縦横比を固定した図は false）。egui_plot の既定の操作は切る
pub fn nav<R>(
    ui: &mut Ui,
    plot: Plot,
    free_axes: bool,
    body: impl FnOnce(&mut PlotUi) -> R,
) -> PlotResponse<R> {
    let plot = plot
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false);
    let scope = ui.scope(|ui| {
        plot.show(ui, |p| {
            let id = pending_id(p.response());
            let pending = p.ctx().data_mut(|d| {
                let v = d.get_temp::<Pending>(id);
                d.remove::<Pending>(id);
                v
            });
            match pending {
                Some(Pending::Bounds(b)) => p.set_plot_bounds(b),
                Some(Pending::Fit) => p.set_auto_bounds(true),
                None => {}
            }
            body(p)
        })
    });
    let (outer, r) = (scope.response.rect, scope.inner);
    // 道具は右下に（egui_plot の凡例は右上、カーソルの値は左上に出る）。拡大縮小は Ctrl（⌘）＋ホイールかピンチ
    let pz = crate::widgets::pan_zoom(ui, &r.response, true);
    let mut t = r.transform;
    let before = *t.bounds();
    let mut pending = if pz.reset { Some(Pending::Fit) } else { None };
    if pending.is_none() {
        if let Some(a) = pz.area {
            // 囲んだ枠をそのまま範囲に（縦横を別に決める。縦横比を固定した図は egui_plot が広げて合わせる）
            let p0 = t.value_from_position(a.left_bottom());
            let p1 = t.value_from_position(a.right_top());
            t.set_bounds(PlotBounds::from_min_max(
                [p0.x.min(p1.x), p0.y.min(p1.y)],
                [p0.x.max(p1.x), p0.y.max(p1.y)],
            ));
        } else {
            if pz.pan != egui::Vec2::ZERO {
                t.translate_bounds((-pz.pan.x as f64, -pz.pan.y as f64));
            }
            if let Some(at) = pz.at {
                t.zoom(egui::Vec2::splat(pz.zoom), at);
            }
        }
        if free_axes {
            axis_nav(ui, &r.response, outer, &mut t);
        }
        if *t.bounds() != before && t.bounds().is_valid() {
            pending = Some(Pending::Bounds(*t.bounds()));
        }
    }
    if let Some(pd) = pending {
        ui.ctx()
            .data_mut(|d| d.insert_temp(pending_id(&r.response), pd));
        ui.ctx().request_repaint();
    }
    crate::widgets::paint_zoom_tools(ui, &r.response);
    r
}

/// 目盛りの帯（グラフの枠の下＝横軸、左＝縦軸）の上のホイールとドラッグ: その軸だけ拡大縮小・移動
fn axis_nav(ui: &Ui, resp: &egui::Response, outer: egui::Rect, t: &mut PlotTransform) {
    let fr = resp.rect;
    let bands = [
        (
            0usize,
            egui::Rect::from_min_max(
                egui::pos2(fr.left(), fr.bottom()),
                egui::pos2(fr.right(), outer.bottom()),
            ),
        ),
        (
            1usize,
            egui::Rect::from_min_max(
                egui::pos2(outer.left(), fr.top()),
                egui::pos2(fr.left(), fr.bottom()),
            ),
        ),
    ];
    for (axis, band) in bands {
        if band.width() < 1.0 || band.height() < 1.0 {
            continue;
        }
        let r = ui.interact(band, resp.id.with(("axis_nav", axis)), egui::Sense::drag());
        let r = r.on_hover_cursor(if axis == 0 {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        });
        if r.dragged() {
            let d = r.drag_delta();
            let d = if axis == 0 {
                (-d.x as f64, 0.0)
            } else {
                (0.0, -d.y as f64)
            };
            t.translate_bounds(d);
        }
        if let Some(p) = r.hover_pos() {
            if let Some(f) = crate::widgets::take_wheel_zoom(ui) {
                let z = if axis == 0 {
                    egui::vec2(f, 1.0)
                } else {
                    egui::vec2(1.0, f)
                };
                // ポインタの位置の値を中心に（帯の外の座標は枠の端に寄せて読む）
                t.zoom(z, fr.clamp(p));
            }
        }
    }
}

/// グラフを描く。body で線を描き、カーソルの値を出す曲線を series に足す。x_unit はカーソルの値の横軸の単位
pub(crate) fn show(
    ui: &mut Ui,
    plot: Plot,
    x_unit: &str,
    body: impl FnOnce(&mut PlotUi, &mut Vec<Series>),
) {
    show_on(ui, plot, 0, x_unit, body);
}

/// [`show`] の縦軸版: 縦軸が元の値（視野・瞳の高さなど）のグラフ。カーソルは横線で、各曲線の縦軸 y での
/// 横軸の値を出す。y_unit はカーソルの値の縦軸の単位
pub(crate) fn show_by_y(
    ui: &mut Ui,
    plot: Plot,
    y_unit: &str,
    body: impl FnOnce(&mut PlotUi, &mut Vec<Series>),
) {
    show_on(ui, plot, 1, y_unit, body);
}

/// axis はカーソルの値の軸（0: 横、1: 縦）
fn show_on(
    ui: &mut Ui,
    plot: Plot,
    axis: usize,
    unit: &str,
    body: impl FnOnce(&mut PlotUi, &mut Vec<Series>),
) {
    let weak = crate::theme::weak_text(ui.visuals());
    let mut series = Vec::new();
    // 既定の近くの点の座標の札は出さない（カーソルの値の一覧と重なる）
    let r = nav(ui, plot.label_formatter(|_, _| String::new()), true, |p| {
        body(p, &mut series);
        // カーソルの線は線の項目にしない（自動の範囲に入ってしまう）。グラフの外にカーソルがあれば出さない
        p.response()
            .hovered()
            .then(|| p.pointer_coordinate())
            .flatten()
            .map(|q: PlotPoint| if axis == 0 { q.x } else { q.y })
    });
    if let Some(v) = r.inner.filter(|_| !series.is_empty()) {
        let fr = *r.transform.frame();
        let seg = if axis == 0 {
            let sx = r.transform.position_from_point_x(v);
            [egui::pos2(sx, fr.top()), egui::pos2(sx, fr.bottom())]
        } else {
            let sy = r.transform.position_from_point_y(v);
            [egui::pos2(fr.left(), sy), egui::pos2(fr.right(), sy)]
        };
        ui.painter_at(fr)
            .line_segment(seg, egui::Stroke::new(1.0_f32, weak));
        if axis == 1 {
            // 縦軸で引くので、点の横と縦を入れ替えた曲線で読む
            for s in &mut series {
                s.pts.iter_mut().for_each(|p| p.swap(0, 1));
            }
        }
        readout(ui, fr, if axis == 0 { "x" } else { "y" }, v, unit, &series);
    }
}

/// カーソルの軸の値と、各曲線の値をグラフの左上に出す
fn readout(ui: &Ui, rect: egui::Rect, axis: &str, x: f64, x_unit: &str, series: &[Series]) {
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
    let mut lines = vec![(format!("{axis} = {} {x_unit}", fmt(x)), text)];
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
