//! 解析の図（の一部）: スポット図（視野ごとの光線の点）と MTF の曲線。前の結果（値を変える前）も薄く重ねる。
//! ほかの解析の図は figures.rs。

use egui::{Color32, RichText, Sense, Ui};
use egui_plot::{Line, LineStyle, Plot};
use lensbench_analysis::defs::{GeoMtfOut, MtfOut, MtfRow, SpotOut};

use crate::figures::Out;

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::theme;

/// スポット図の光線の点（1 視野・1 波長）
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SpotRays {
    /// 視野の番号（1 始まり）
    pub field: usize,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 像面の局所座標 [レンズの単位]（通った光線だけ）
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

/// 瞳の点（同心円。中心と、k 番目の輪に 6k 点）。最初の点は主光線（0, 0）
pub(crate) fn pupil_grid(rings: usize) -> (Vec<f64>, Vec<f64>) {
    let mut px = vec![0.0];
    let mut py = vec![0.0];
    for k in 1..=rings {
        let r = k as f64 / rings as f64;
        let n = 6 * k;
        for j in 0..n {
            let a = std::f64::consts::TAU * j as f64 / n as f64;
            px.push(r * a.cos());
            py.push(r * a.sin());
        }
    }
    (px, py)
}

/// trace の結果（配列）から通った光線の点を取る
pub(crate) fn rays_from(field: usize, wave: usize, arrays: &[(String, Vec<f64>)]) -> SpotRays {
    let get = |k: &str| {
        arrays
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    };
    let (x, y, err, vig) = (get("x"), get("y"), get("err"), get("vig"));
    let mut r = SpotRays {
        field,
        wave,
        ..Default::default()
    };
    for i in 0..x.len().min(y.len()) {
        let ok =
            err.get(i).copied().unwrap_or(0.0) == 0.0 && vig.get(i).copied().unwrap_or(0.0) == 0.0;
        if ok && x[i].is_finite() && y[i].is_finite() {
            r.x.push(x[i]);
            r.y.push(y[i]);
        }
    }
    r
}

/// 図に要る系の値
pub(crate) struct SysInfo<'a> {
    pub waves_um: &'a [f64],
    pub primary: usize,
    pub wfno: Option<f64>,
}

fn spot_rms(v: &SpotOut, field: usize) -> Option<f64> {
    let r = v.spot.iter().find(|s| s.field == field)?.rms_centroid;
    r.is_finite().then_some(r)
}

fn delta_text(vis: &egui::Visuals, now: f64, prev: Option<f64>) -> Option<(String, Color32)> {
    let p = prev?;
    let d = now - p;
    if d.abs() < 1e-12 * now.abs().max(1e-300) {
        return None;
    }
    let col = if d < 0.0 {
        theme::better(vis)
    } else {
        theme::worse(vis)
    };
    Some((
        format!("{}{}", if d > 0.0 { "+" } else { "" }, fmt_sig(d, 3)),
        col,
    ))
}

/// 有効数字 n 桁
pub fn fmt_sig(x: f64, n: i32) -> String {
    if x == 0.0 || !x.is_finite() {
        return fmt_num(x);
    }
    let e = x.abs().log10().floor() as i32;
    let k = 10f64.powi(n - 1 - e);
    fmt_num((x * k).round() / k)
}

/// スポット図の拡大・移動を 1 つの枠の操作で更新する。`s0` は倍率 1 の px/µm
fn spot_view(
    ui: &Ui,
    resp: &egui::Response,
    view: (f32, egui::Vec2),
    s0: f32,
) -> (f32, egui::Vec2) {
    let pz = crate::widgets::pan_zoom(ui, resp, true);
    if pz.reset {
        return (1.0, egui::Vec2::ZERO);
    }
    let (mut z, mut off) = view;
    // 画面の px の差を µm に（y は上が正）
    let to_um = |v: egui::Vec2, z: f32| egui::vec2(v.x, -v.y) / (s0 * z);
    off -= to_um(pz.pan, z);
    if let Some(p) = pz.at {
        let d = p - resp.rect.center();
        let under = off + to_um(d, z);
        z = (z * pz.zoom).clamp(1.0, 1e4);
        off = under - to_um(d, z);
    }
    if z <= 1.0 + 1e-4 {
        off = egui::Vec2::ZERO;
    }
    (z, off)
}

/// スポット図。点は µm、中心は各視野の主波長の重心（無ければ全体の重心）
/// 視野の番号（昇順）・視野ごとの中心・全視野で共通の縮尺（中心からの最大の距離 mm）
fn spot_scale(rays: &[SpotRays], primary: usize) -> (Vec<usize>, Vec<(f64, f64)>, f64) {
    let mut fields: Vec<usize> = rays.iter().map(|r| r.field).collect();
    fields.sort();
    fields.dedup();
    // 視野ごとの中心と、全視野で共通の縮尺
    let centers: Vec<(f64, f64)> = fields
        .iter()
        .map(|&f| {
            let pick = |only_primary: bool| {
                let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
                for r in rays
                    .iter()
                    .filter(|r| r.field == f && (!only_primary || r.wave == primary))
                {
                    for i in 0..r.x.len() {
                        sx += r.x[i];
                        sy += r.y[i];
                        n += 1;
                    }
                }
                (n > 0).then(|| (sx / n as f64, sy / n as f64))
            };
            pick(true).or_else(|| pick(false)).unwrap_or((0.0, 0.0))
        })
        .collect();
    let mut rmax: f64 = 0.0;
    for r in rays {
        let c = centers[fields.iter().position(|&f| f == r.field).unwrap()];
        for i in 0..r.x.len() {
            rmax = rmax.max(((r.x[i] - c.0).powi(2) + (r.y[i] - c.1).powi(2)).sqrt());
        }
    }
    (fields, centers, rmax)
}

/// スポット図。rays_gen は光線の世代（届くたびに変わる。中心・縮尺はそのときだけ求める）
pub(crate) fn spot(
    ui: &mut Ui,
    rays: &[SpotRays],
    rays_gen: u64,
    result: Option<&SpotOut>,
    prev: Option<&SpotOut>,
    sys: &SysInfo,
    st: &mut crate::figures::FigState,
) {
    let scale = crate::figures::memo(ui, ("spot-scale", sys.primary), rays_gen, || {
        spot_scale(rays, sys.primary)
    });
    let (fields, centers, rmax) = (&scale.0, &scale.1, scale.2);
    if fields.is_empty() {
        ui.weak(tr("Tracing rays…"));
        return;
    }
    let airy_um = sys.wfno.map(|f| {
        1.22 * sys
            .waves_um
            .get(sys.primary.saturating_sub(1))
            .copied()
            .unwrap_or(0.55)
            * f
    });
    // 枠の幅（µm）は OpticStudio の Scale と同じ: 指定（spot_bar）があればその長さ、無ければ点と Airy 円が入る
    // きりのよい長さ（1・2・5×10^n）。枠の下の縮尺は倍率 1 のとき枠の幅
    let fit_um = (rmax * 1e3).max(airy_um.unwrap_or(0.0)).max(1e-3) * 2.3;
    let box_um = if st.spot_bar > 0.0 {
        st.spot_bar
    } else {
        crate::widgets::scale_bar::nice_up(fit_um * 1e-3).map_or(fit_um, |mm| mm * 1e3)
    };
    let half_um = box_um / 2.0;
    let n = fields.len();
    let avail = ui.available_width();
    let cols = ((avail / 150.0).floor() as usize).clamp(1, n.max(1));
    let cell = ((avail - 8.0 * cols as f32) / cols as f32).clamp(90.0, 260.0);
    let vis = ui.visuals().clone();
    // 拡大・移動（全視野で共通。倍率と中心のずれ µm）。ホイール＝拡大、右・中ドラッグ＝移動、
    // ダブルクリック・Home＝全体
    let mut view: (f32, egui::Vec2) = (st.spot_zoom, egui::Vec2::from(st.spot_pan));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(trf(
                "Half width of a box {} µm",
                &[&fmt_sig(half_um / view.0 as f64, 3)],
            ))
            .small()
            .color(theme::weak_text(&vis)),
        );
        // 縮尺の長さ（0 は自動）
        ui.label(
            RichText::new(tr("Scale bar"))
                .small()
                .color(theme::weak_text(&vis)),
        );
        ui.add(
            egui::DragValue::new(&mut st.spot_bar)
                .speed(1.0)
                .range(0.0..=1e6)
                .custom_formatter(|v, _| {
                    if v <= 0.0 {
                        tr("Auto").to_string()
                    } else {
                        format!("{} µm", fmt_sig(v, 6))
                    }
                })
                // 「40 µm」も「40」も読む。数でなければ（「自動」など）0＝自動
                .custom_parser(|t| {
                    Some(
                        t.trim()
                            .trim_end_matches("µm")
                            .trim()
                            .parse::<f64>()
                            .unwrap_or(0.0),
                    )
                }),
        )
        .on_hover_text(tr("Width of each box and length of the scale bar below it (0: automatic), like Scale in OpticStudio"));
        if let Some(a) = airy_um {
            ui.label(
                RichText::new(trf("Dashed circle: Airy radius {} µm", &[&fmt_sig(a, 3)]))
                    .small()
                    .color(theme::weak_text(&vis)),
            );
        }
        // 波長の凡例
        let mut waves: Vec<usize> = rays.iter().map(|r| r.wave).collect();
        waves.sort();
        waves.dedup();
        for w in waves {
            let um = sys.waves_um.get(w - 1).copied().unwrap_or(0.55);
            ui.label(
                RichText::new(format!("● {} µm", fmt_num(um)))
                    .small()
                    .color(theme::wave_color(um)),
            );
        }
    });
    egui::Grid::new(ui.next_auto_id())
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (k, &f) in fields.iter().enumerate() {
                ui.vertical(|ui| {
                    let (resp, painter) =
                        ui.allocate_painter(egui::vec2(cell, cell), Sense::click_and_drag());
                    // 名前の関数は Context を握ったまま呼ばれるので、中で ui・ctx を読まない（読むと止まる）
                    let fname = field_namer(ui)(f);
                    crate::widgets::name_figure(&resp, || trf("Spot diagram, {}", &[&fname]));
                    let rect = resp.rect;
                    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
                    let painter = painter.with_clip_rect(rect.intersect(ui.clip_rect()));
                    let s0 = (cell / 2.0 - 4.0) / half_um as f32;
                    view = spot_view(ui, &resp, view, s0);
                    let s = s0 * view.0;
                    // 拡大の中心（重心から view.1 µm ずれた所）を枠の中央に
                    let c = rect.center() - egui::vec2(view.1.x, -view.1.y) * s;
                    let grid = vis.widgets.noninteractive.bg_stroke.color;
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 4.0, c.y),
                            egui::pos2(rect.right() - 4.0, c.y),
                        ],
                        (0.5, grid),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(c.x, rect.top() + 4.0),
                            egui::pos2(c.x, rect.bottom() - 4.0),
                        ],
                        (0.5, grid),
                    );
                    if let Some(a) = airy_um {
                        let pts: Vec<egui::Pos2> = (0..=48)
                            .map(|j| {
                                let t = std::f32::consts::TAU * j as f32 / 48.0;
                                c + egui::vec2(t.cos(), t.sin()) * a as f32 * s
                            })
                            .collect();
                        painter.add(egui::Shape::dashed_line(
                            &pts,
                            (1.0, theme::weak_text(&vis).gamma_multiply(0.7)),
                            3.0,
                            3.0,
                        ));
                    }
                    let cc = centers[k];
                    for r in rays.iter().filter(|r| r.field == f) {
                        let col = theme::wave_color(
                            sys.waves_um.get(r.wave - 1).copied().unwrap_or(0.55),
                        );
                        for i in 0..r.x.len() {
                            let p = c + egui::vec2(
                                ((r.x[i] - cc.0) * 1e3) as f32 * s,
                                // 像の y は上が正
                                -((r.y[i] - cc.1) * 1e3) as f32 * s,
                            );
                            painter.circle_filled(p, 1.3, col);
                        }
                    }
                    // 枠の下に縮尺（OpticStudio と同じく倍率 1 では枠の幅、拡大したら見えている幅までのきりのよい長さ）
                    let (br, bp) = ui.allocate_painter(egui::vec2(cell, 18.0), Sense::hover());
                    let at = br.rect.left_bottom() + egui::vec2(4.0, -1.0);
                    let al = egui::Align2::LEFT_BOTTOM;
                    let weak = theme::weak_text(&vis);
                    if (view.0 - 1.0).abs() < 1e-4 {
                        crate::widgets::scale_bar::draw_len(
                            &bp,
                            at,
                            al,
                            s as f64 * 1e3,
                            box_um * 1e-3,
                            weak,
                        );
                    } else {
                        crate::widgets::scale_bar::draw(
                            &bp,
                            at,
                            al,
                            s as f64 * 1e3,
                            cell - 8.0,
                            weak,
                        );
                    }
                    // 指した所の座標（重心から µm）
                    if let Some(p) = resp.hover_pos() {
                        let d = (p - c) / s;
                        resp.clone().on_hover_text_at_pointer(format!(
                            "x {} µm, y {} µm",
                            fmt_sig(d.x as f64, 3),
                            fmt_sig(-d.y as f64, 3)
                        ));
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(fname.as_str()).color(theme::weak_text(&vis)));
                        if let Some(rms) = result.and_then(|r| spot_rms(r, f)) {
                            let um = rms * 1e3;
                            ui.label(
                                RichText::new(format!("RMS {} µm", fmt_sig(um, 3))).monospace(),
                            );
                            if let Some((t, col)) = delta_text(
                                &vis,
                                um,
                                prev.and_then(|p| spot_rms(p, f)).map(|x| x * 1e3),
                            ) {
                                ui.label(RichText::new(t).small().color(col))
                                    .on_hover_text(tr("Difference from before the change"));
                            }
                        }
                    });
                });
                if (k + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
    (st.spot_zoom, st.spot_pan) = (view.0, view.1.into());
}

/// 視野の座標の文字の並び（画面の一時データの鍵。解析パネルが描く前に入れる）
pub(crate) const FIELD_LABELS: &str = "lensbench-field-labels";

/// 視野の名前「視野 2 (0, 14)」を作る関数（座標が分かれば添える）。視野の番号は 1 から
pub(crate) fn field_namer(ui: &Ui) -> impl Fn(usize) -> String {
    let coords: Vec<String> = ui
        .data(|d| d.get_temp(egui::Id::new(FIELD_LABELS)))
        .unwrap_or_default();
    move |f| match f.checked_sub(1).and_then(|i| coords.get(i)) {
        Some(c) => trf("Field {} {}", &[&f, c]),
        None => trf("Field {}", &[&f]),
    }
}

/// MTF の曲線（視野ごとに色、子午は実線・球欠は破線）。前の結果は薄い点線。描けなければ false
pub(crate) fn mtf(
    ui: &mut Ui,
    out: Out<'_>,
    prev: Option<Out<'_>>,
    id: u64,
    gen: u64,
    show_prev: &mut bool,
) -> bool {
    let Some(now) = out.get::<MtfOut>(ui, ("mtf", id, "now"), gen) else {
        return false;
    };
    let before = prev.and_then(|p| p.get::<MtfOut>(ui, ("mtf", id, "prev"), gen));
    let before = before.as_ref().map(|b| &b.mtf[..]);
    mtf_curves(ui, &now.mtf, &now.unit, before, id, gen, "mtf", show_prev);
    true
}

/// 幾何 MTF の曲線（[`mtf`] と同じ描き方）。描けなければ false
pub(crate) fn geo_mtf(
    ui: &mut Ui,
    out: Out<'_>,
    prev: Option<Out<'_>>,
    id: u64,
    gen: u64,
    show_prev: &mut bool,
) -> bool {
    let Some(now) = out.get::<GeoMtfOut>(ui, ("geo-mtf", id, "now"), gen) else {
        return false;
    };
    let before = prev.and_then(|p| p.get::<GeoMtfOut>(ui, ("geo-mtf", id, "prev"), gen));
    let before = before.as_ref().map(|b| &b.geometric_mtf[..]);
    mtf_curves(
        ui,
        &now.geometric_mtf,
        &now.unit,
        before,
        id,
        gen,
        "geometric_mtf",
        show_prev,
    );
    true
}

/// MTF の曲線（key は図の名前: 回折 MTF は mtf、幾何 MTF は geometric_mtf）。gen は結果の世代
/// （曲線はそのときだけ作る）
#[allow(clippy::too_many_arguments)]
fn mtf_curves(
    ui: &mut Ui,
    rows: &[MtfRow],
    unit: &str,
    prev: Option<&[MtfRow]>,
    id: u64,
    gen: u64,
    key: &str,
    show_prev: &mut bool,
) {
    let unit = unit.to_string();
    let lines = |rows: &[MtfRow]| -> Vec<(usize, &'static str, Vec<[f64; 2]>)> {
        let mut out = Vec::new();
        for c in rows {
            for (k, a) in [("tangential", &c.tangential), ("sagittal", &c.sagittal)] {
                // JSON に書けない値（NaN）は点にしない
                let pts: Vec<[f64; 2]> = c
                    .freqs
                    .iter()
                    .zip(a)
                    .filter(|(_, y)| y.is_finite())
                    .map(|(x, y)| [*x, *y])
                    .collect();
                out.push((c.field, k, pts));
            }
        }
        out
    };
    let both = crate::figures::memo(ui, ("mtf-lines", key, id), gen, || {
        (lines(rows), prev.map(lines).unwrap_or_default())
    });
    let (now, prev_lines) = (&both.0, &both.1);
    // 前の結果（点線）は出すかを選べる。出すときも今の結果の周波数の範囲で切る（軸を引き伸ばさない）
    let xmax = now
        .iter()
        .flat_map(|(_, _, p)| p.iter().map(|q| q[0]))
        .filter(|x| x.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    let before: Vec<(usize, &str, Vec<[f64; 2]>)> = if *show_prev {
        prev_lines
            .iter()
            .map(|(f, k, p)| {
                (
                    *f,
                    *k,
                    p.iter()
                        .copied()
                        .filter(|q| q[0] <= xmax * (1.0 + 1e-9))
                        .collect(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut fields: Vec<usize> = now.iter().map(|x| x.0).collect();
    fields.dedup();
    let has_prev = prev.is_some();
    // 凡例は図の外（下）に、線の見本と視野の値で。順は視野 → 子午・球欠に固定。グラフの上の道具の行の分も引く
    // （引かないと凡例がパネルの下に押し出されて見えない）
    let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    let legend_h = crate::plot_tools::bar_height(ui) + row * if has_prev { 2.0 } else { 1.0 };
    let plot = Plot::new((key, id))
        .include_y(0.0)
        .include_y(1.0)
        .include_x(0.0)
        .height({
            let h = ui.available_height() - legend_h;
            if h.is_finite() {
                h.max(80.0)
            } else {
                300.0
            }
        })
        .x_axis_label(unit.clone())
        .y_axis_label("MTF");
    let field_label = field_namer(ui);
    crate::plot_tools::show(ui, plot, &unit, |p, reg| {
        for (f, _k, pts) in before {
            p.line(
                Line::new(pts)
                    .color(theme::series(f.saturating_sub(1)).gamma_multiply(0.35))
                    .style(LineStyle::Dotted { spacing: 4.0 })
                    .width(1.0_f32),
            );
        }
        for &(f, k, ref pts) in now.iter() {
            let t = k == "tangential";
            let name = if t {
                trf("Field {} tangential", &[&f])
            } else {
                trf("Field {} sagittal", &[&f])
            };
            let color = theme::series(f.saturating_sub(1));
            reg.push(crate::plot_tools::Series::new(name.clone(), color, pts));
            p.line(
                Line::new(pts.clone())
                    .color(color)
                    .style(if t {
                        LineStyle::Solid
                    } else {
                        LineStyle::Dashed { length: 6.0 }
                    })
                    .width(1.6_f32),
            );
        }
    });
    let weak = theme::weak_text(ui.visuals());
    let sample = |ui: &mut Ui, color: Color32, dashed: bool, dotted: bool| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 10.0), Sense::hover());
        let y = r.center().y;
        let st = egui::Stroke::new(1.6_f32, color);
        let p = ui.painter();
        if dashed || dotted {
            let (on, off) = if dotted { (1.5, 3.0) } else { (5.0, 3.0) };
            let mut x = r.left();
            while x < r.right() {
                let x2 = (x + on).min(r.right());
                p.line_segment([egui::pos2(x, y), egui::pos2(x2, y)], st);
                x += on + off;
            }
        } else {
            p.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], st);
        }
    };
    ui.horizontal_wrapped(|ui| {
        sample(ui, weak, false, false);
        ui.label(RichText::new(tr("tangential")).small().color(weak));
        sample(ui, weak, true, false);
        ui.label(RichText::new(tr("sagittal")).small().color(weak));
        ui.add_space(8.0);
        for f in &fields {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
            ui.painter()
                .rect_filled(r, 2.0, theme::series(f.saturating_sub(1)));
            ui.label(RichText::new(field_label(*f)).small());
        }
    });
    if has_prev {
        ui.horizontal(|ui| {
            sample(ui, weak, false, true);
            ui.checkbox(
                show_prev,
                RichText::new(tr("Previous result (before the system change)")).small(),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_and_rays() {
        let (px, py) = pupil_grid(3);
        assert_eq!(px.len(), 1 + 6 + 12 + 18);
        assert_eq!((px[0], py[0]), (0.0, 0.0));
        assert!(px
            .iter()
            .zip(&py)
            .all(|(x, y)| x * x + y * y <= 1.0 + 1e-12));
        let a = vec![
            ("x".to_string(), vec![0.0, 1.0, 2.0]),
            ("y".to_string(), vec![0.0, 1.0, f64::NAN]),
            ("err".to_string(), vec![0.0, 0.0, 0.0]),
            ("vig".to_string(), vec![0.0, 1.0, 0.0]),
        ];
        let r = rays_from(2, 1, &a);
        assert_eq!(r.x, vec![0.0]);
        assert_eq!(r.field, 2);
    }

    #[test]
    fn sig() {
        assert_eq!(fmt_sig(0.0123456, 3), "0.0123");
        assert_eq!(fmt_sig(-1234.5, 2), "-1200");
    }
}
