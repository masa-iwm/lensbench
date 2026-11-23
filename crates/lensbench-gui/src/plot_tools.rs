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
/// 拡大縮小（素のホイールはパネルのスクロール）、左・中ドラッグと Space+左ドラッグ＝移動、
/// 右下の道具（＋・－・囲んで拡大・全体。右上は凡例）、Ctrl（⌘）+左ドラッグ＝囲んで拡大、ダブルクリック・Home＝全体、
/// 右クリック＝メニュー（軸の範囲を数で決めるなど）。
/// `free_axes` なら横軸・縦軸の目盛りの上の Ctrl（⌘）＋ホイールでその軸だけ拡大縮小し、
/// 目盛りの上のドラッグでその軸だけ動かす（縦横比を固定した図は false）。egui_plot の既定の操作は切る
pub fn nav<R>(
    ui: &mut Ui,
    plot: Plot,
    free_axes: bool,
    body: impl FnOnce(&mut PlotUi) -> R,
) -> PlotResponse<R> {
    nav_with(ui, plot, free_axes, body).0
}

/// [`nav`] と同じに描き、右クリックのメニューで「データをコピー」を選んだら true も返す
pub(crate) fn nav_with<R>(
    ui: &mut Ui,
    plot: Plot,
    free_axes: bool,
    body: impl FnOnce(&mut PlotUi) -> R,
) -> (PlotResponse<R>, bool) {
    let plot = plot
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        // egui_plot 0.38 の目盛りのドラッグでの拡大（既定で入る）も切る（目盛りの上の操作は axis_nav）
        .allow_axis_zoom_drag(false);
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
    let opts = crate::widgets::NavOpts {
        left_pans: true,
        graph: true,
        ..crate::widgets::NavOpts::FIGURE
    };
    let pz = crate::widgets::pan_zoom(ui, &r.response, opts);
    let mut t = r.transform;
    let before = *t.bounds();
    let mut shown = before;
    let mut pending = if crate::widgets::fig_nav::history(ui, &r.response, &pz, &mut shown) {
        Some(Pending::Bounds(shown))
    } else if pz.reset {
        Some(Pending::Fit)
    } else {
        None
    };
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
                let z = if free_axes {
                    pz.zoom_xy
                } else {
                    egui::Vec2::splat(pz.zoom)
                };
                t.zoom(z, at);
            }
        }
        if free_axes {
            axis_nav(ui, &r.response, outer, &mut t);
        }
        if *t.bounds() != before && t.bounds().is_valid() {
            pending = Some(Pending::Bounds(*t.bounds()));
        }
    }
    if pz.menu == Some(crate::widgets::FigMenu::AxisRange) {
        AxisEdit::open(ui, &r.response, before);
    }
    if let Some(b) = AxisEdit::show(ui, &r.response) {
        crate::widgets::fig_nav::history_push(ui, &r.response, &before);
        pending = Some(Pending::Bounds(b));
    }
    if let Some(pd) = pending {
        ui.ctx()
            .data_mut(|d| d.insert_temp(pending_id(&r.response), pd));
        ui.ctx().request_repaint();
    }
    crate::widgets::paint_zoom_tools(ui, &r.response);
    (r, pz.menu == Some(crate::widgets::FigMenu::CopyData))
}

/// 軸の範囲を数で決める小窓（右クリックのメニューの「軸の範囲…」。グラフごとに egui の一時データ）
#[derive(Clone, Default)]
struct AxisEdit {
    /// 横の最小・最大、縦の最小・最大（打った文字）
    v: [String; 4],
    /// 読めない・最小が最大以上
    bad: bool,
}

impl AxisEdit {
    fn id(resp: &egui::Response) -> egui::Id {
        resp.id.with("axis_edit")
    }

    fn open(ui: &Ui, resp: &egui::Response, b: PlotBounds) {
        let e = AxisEdit {
            v: [b.min()[0], b.max()[0], b.min()[1], b.max()[1]].map(fmt),
            bad: false,
        };
        ui.data_mut(|d| d.insert_temp(Self::id(resp), e));
    }

    /// 開いていれば小窓を出し、「適用」で読めた範囲を返す
    fn show(ui: &Ui, resp: &egui::Response) -> Option<PlotBounds> {
        let id = Self::id(resp);
        let mut e: AxisEdit = ui.data(|d| d.get_temp(id))?;
        let (mut open, mut out, mut close) = (true, None, false);
        egui::Window::new(crate::i18n::tr("Axis range"))
            .id(id.with("win"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(resp.rect.center())
            .show(ui.ctx(), |ui| {
                egui::Grid::new(id.with("grid")).show(ui, |ui| {
                    ui.label("");
                    ui.label(crate::i18n::tr("Min"));
                    ui.label(crate::i18n::tr("Max"));
                    ui.end_row();
                    for (k, name) in ["Horizontal axis", "Vertical axis"].into_iter().enumerate() {
                        ui.label(crate::i18n::tr(name));
                        for j in 0..2 {
                            let i = 2 * k + j;
                            let te = egui::TextEdit::singleline(&mut e.v[i]).desired_width(90.0);
                            let r = ui.add(te);
                            let what = if j == 0 { "Min" } else { "Max" };
                            let label =
                                format!("{} {}", crate::i18n::tr(name), crate::i18n::tr(what));
                            crate::widgets::name_field(&r, &label, &e.v[i]);
                        }
                        ui.end_row();
                    }
                });
                if e.bad {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        crate::i18n::tr("Enter numbers with min < max"),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Apply")).clicked() {
                        let n: Vec<Option<f64>> =
                            e.v.iter()
                                .map(|s| {
                                    s.trim()
                                        .replace(',', ".")
                                        .parse()
                                        .ok()
                                        .filter(|v: &f64| v.is_finite())
                                })
                                .collect();
                        match (n[0], n[1], n[2], n[3]) {
                            (Some(x0), Some(x1), Some(y0), Some(y1)) if x0 < x1 && y0 < y1 => {
                                out = Some(PlotBounds::from_min_max([x0, y0], [x1, y1]));
                                e.bad = false;
                            }
                            _ => e.bad = true,
                        }
                    }
                    close = ui.button(crate::i18n::tr("Close")).clicked();
                });
            });
        ui.data_mut(|d| {
            if open && !close {
                d.insert_temp(id, e);
            } else {
                d.remove::<AxisEdit>(id);
            }
        });
        out
    }
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
    let (r, copy) = nav_with(ui, plot.label_formatter(|_| None), true, |p| {
        body(p, &mut series);
        // カーソルの線は線の項目にしない（自動の範囲に入ってしまう）。グラフの外にカーソルがあれば出さない
        p.response()
            .hovered()
            .then(|| p.pointer_coordinate())
            .flatten()
            .map(|q: PlotPoint| if axis == 0 { q.x } else { q.y })
    });
    if copy {
        ui.ctx().copy_text(csv(&series));
    }
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

/// 曲線の点の CSV（1 行 1 点: 曲線の名前・横・縦）
fn csv(series: &[Series]) -> String {
    let mut s = String::from("series,x,y\n");
    for c in series {
        let name = if c.name.contains([',', '"', '\n']) {
            format!("\"{}\"", c.name.replace('"', "\"\""))
        } else {
            c.name.clone()
        };
        for [x, y] in &c.pts {
            s.push_str(&format!("{name},{x},{y}\n"));
        }
    }
    s
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

    /// 右クリックのメニューの「軸の範囲…」: 小窓に今の範囲が入り、打って「適用」すると次のフレームの範囲になる。
    /// 「データをコピー」は曲線の点を CSV で写す
    #[test]
    fn context_menu_sets_axis_range_and_copies_data() {
        use egui_kittest::kittest::Queryable;
        let _g = crate::serve_lock();
        crate::i18n::set(crate::i18n::Lang::En);
        let mut h = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui_state(
                |ui, st: &mut (Option<PlotBounds>, egui::Rect)| {
                    let plot = Plot::new("cm").width(400.0).height(300.0);
                    show(ui, plot, "", |p, s| {
                        let pts = [[0.0, 0.0], [10.0, 5.0]];
                        p.line(egui_plot::Line::new("L", pts.to_vec()));
                        s.push(Series::new("L", Color32::RED, &pts));
                        st.0 = Some(p.plot_bounds());
                        st.1 = p.response().rect;
                    });
                },
                (None, egui::Rect::NOTHING),
            );
        h.run_steps(3);
        let right_click = |h: &mut egui_kittest::Harness<'_, (Option<PlotBounds>, egui::Rect)>| {
            let p = h.state().1.center();
            h.hover_at(p);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(1);
            }
            h.run_steps(2);
        };
        right_click(&mut h);
        h.get_by_label("Axis range…").click_accesskit();
        h.run_steps(3);
        for (name, v) in [
            ("Horizontal axis Min", "2"),
            ("Horizontal axis Max", "4"),
            ("Vertical axis Min", "-1"),
            ("Vertical axis Max", "1"),
        ] {
            h.get_by_label(name).focus();
            h.run_steps(1);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(name).type_text(v);
            h.run_steps(1);
        }
        h.get_by_label("Apply").click_accesskit();
        h.run_steps(4);
        let b = h.state().0.unwrap();
        assert_eq!((b.min(), b.max()), ([2.0, -1.0], [4.0, 1.0]));
        // 最小が最大以上なら当てずに誤りを出す
        h.get_by_label("Vertical axis Max").focus();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertical axis Max").type_text("-5");
        h.run_steps(1);
        h.get_by_label("Apply").click_accesskit();
        h.run_steps(3);
        h.get_by_label("Enter numbers with min < max");
        assert_eq!(h.state().0.unwrap().max(), [4.0, 1.0]);
        h.get_by_label("Close").click_accesskit();
        h.run_steps(2);
        assert!(h.query_by_label("Apply").is_none());
        // データをコピー
        right_click(&mut h);
        h.get_by_label("Copy data (CSV)").click_accesskit();
        h.run_steps(1);
        let copied = h.output().platform_output.commands.iter().any(|c| {
            matches!(c, egui::OutputCommand::CopyText(t) if t.starts_with("series,x,y\nL,0,0\n"))
        });
        assert!(copied, "{:?}", h.output().platform_output.commands);
    }

    #[test]
    fn csv_has_one_row_per_point_and_quotes_names() {
        let s = [
            Series::new("a", Color32::RED, &[[0.0, 1.0], [2.0, 3.5]]),
            Series::new("x, \"y\"", Color32::RED, &[[1.0, -1.0]]),
        ];
        assert_eq!(
            csv(&s),
            "series,x,y\na,0,1\na,2,3.5\n\"x, \"\"y\"\"\",1,-1\n"
        );
    }
}
