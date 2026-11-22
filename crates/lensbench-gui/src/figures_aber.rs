//! 収差図の図: 横収差・OPD の扇、像面湾曲と歪曲、縦収差、倍率色収差、色の焦点ずれ、グリッド歪曲、ザイデル係数。
//! フットプリント・相対照度・けられの図と、近軸ガウスビームの輪郭も。
//!
//! グラフはどれも [`crate::plot_tools::show`] を通す（解析グラフの操作をそろえる）。色は波長の色（[`theme::wave_color`]、
//! 波長は結果の画面が [`WAVES_UM`] に置く）。

use egui::{Color32, Ui};
use egui_plot::{Bar, BarChart, HLine, Legend, Line, LineStyle, Plot, PlotPoints};
use egui_plot::{MarkerShape, PlotBounds, PlotPoint, Points, Text, VLine};
use lensbench_analysis::defs::{
    FanOut, FieldCurvOut, FootprintOut, GaussOut, GridOut, LatOut, LongOut, RelIllumOut, SeidelOut,
    SeidelRow, ShiftOut, VignettingOut,
};

use crate::figures::{weak, Out};
use crate::i18n::{tr, trf};
use crate::plot_tools::{self, Series};
use crate::plots::fmt_sig;
use crate::theme;

/// 結果の画面が置く、系の波長 [µm] の並び（ui の一時データの鍵）
pub(crate) const WAVES_UM: &str = "lensbench-waves-um";

/// 収差図の解析
pub(crate) const CMDS: &[&str] = &[
    "ray-fan",
    "opd-fan",
    "pupil-fan",
    "field-curvature",
    "longitudinal",
    "lateral-color",
    "focal-shift",
    "grid-distortion",
    "seidel",
    "footprint",
    "relative-illumination",
    "vignetting",
    "gaussian-beam",
];

/// 収差図を描く。cmd が収差図でなければ None、描けない結果なら Some(false)
pub(crate) fn show(ui: &mut Ui, id: u64, gen: u64, cmd: &str, out: Out<'_>) -> Option<bool> {
    macro_rules! get {
        ($t:ty) => {
            out.get::<$t>(ui, (id, cmd, "now"), gen)
        };
    }
    Some(match cmd {
        "ray-fan" | "opd-fan" | "pupil-fan" => get!(FanOut).is_some_and(|v| fans(ui, id, cmd, &v)),
        "field-curvature" => get!(FieldCurvOut).is_some_and(|v| field_curv(ui, id, &v)),
        "longitudinal" => get!(LongOut).is_some_and(|v| longitudinal(ui, id, &v)),
        "lateral-color" => get!(LatOut).is_some_and(|v| lateral(ui, id, &v)),
        "focal-shift" => get!(ShiftOut).is_some_and(|v| focal_shift(ui, id, &v)),
        "grid-distortion" => get!(GridOut).is_some_and(|v| grid(ui, id, &v)),
        "seidel" => get!(SeidelOut).is_some_and(|v| seidel(ui, id, &v)),
        "footprint" => get!(FootprintOut).is_some_and(|v| footprint(ui, id, &v)),
        "relative-illumination" => get!(RelIllumOut).is_some_and(|v| rel_illum(ui, id, &v)),
        "vignetting" => get!(VignettingOut).is_some_and(|v| vignetting(ui, id, &v)),
        "gaussian-beam" => get!(GaussOut).is_some_and(|v| gauss(ui, id, &v)),
        _ => return None,
    })
}

/// 波長の色と名前（結果の画面が置いた系の波長から）
struct Waves(Vec<f64>);

impl Waves {
    fn of(ui: &Ui) -> Waves {
        Waves(
            ui.data(|d| d.get_temp(egui::Id::new(WAVES_UM)))
                .unwrap_or_default(),
        )
    }
    /// 波長 w（1 始まり）の色と名前
    fn style(&self, w: usize) -> (Color32, String) {
        match w.checked_sub(1).and_then(|i| self.0.get(i)) {
            Some(&u) => (theme::wave_color(u), format!("{} µm", fmt_sig(u, 4))),
            None => (
                theme::series(w.saturating_sub(1)),
                trf("Wavelength {}", &[&w]),
            ),
        }
    }
}

/// 波長の凡例の行（waves は 1 始まりの番号）。note は後ろに添える説明
fn wave_legend(ui: &mut Ui, ws: &Waves, waves: &[usize], note: Option<String>) {
    ui.horizontal_wrapped(|ui| {
        for &w in waves {
            let (c, name) = ws.style(w);
            ui.label(egui::RichText::new(format!("● {name}")).small().color(c));
        }
        if let Some(n) = note {
            ui.label(weak(ui, n).small());
        }
    });
}

/// 有限な点だけの折れ線（NaN で線を切る）
fn segments(x: &[f64], y: &[f64]) -> Vec<Vec<[f64; 2]>> {
    let mut out = vec![Vec::new()];
    for (&a, &b) in x.iter().zip(y) {
        if a.is_finite() && b.is_finite() {
            out.last_mut().expect("not empty").push([a, b]);
        } else if !out.last().expect("not empty").is_empty() {
            out.push(Vec::new());
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

/// 切れた線をつないだ点（カーソルの値を読む曲線）
fn flat(segs: &[Vec<[f64; 2]>]) -> Vec<[f64; 2]> {
    segs.iter().flatten().copied().collect()
}

/// 折れ線を描く（NaN で切れた線は同じ名前の続き）
fn draw(
    p: &mut egui_plot::PlotUi,
    pts: Vec<Vec<[f64; 2]>>,
    col: Color32,
    dashed: bool,
    name: &str,
) {
    for (k, s) in pts.into_iter().enumerate() {
        let mut l = Line::new(PlotPoints::from(s))
            .color(col)
            .width(1.6_f32)
            .style(if dashed {
                LineStyle::Dashed { length: 6.0 }
            } else {
                LineStyle::Solid
            });
        if k == 0 {
            l = l.name(name);
        }
        p.line(l);
    }
}

/// 図の高さ（パネルの残りを rows 段に分ける。最低 min）
fn row_height(ui: &Ui, rows: usize, min: f32) -> f32 {
    let bar = ui.spacing().interact_size.y * 2.0 + ui.spacing().item_spacing.y * 2.0;
    let h = (ui.available_height() / rows.max(1) as f32) - bar;
    if h.is_finite() {
        h.max(min)
    } else {
        300.0
    }
}

// ------------------------------------------------------------------------------------- 扇

/// 横収差・OPD の扇: 視野ごとに 1 段、左が子午（値 対 Py）・右が球欠（値 対 Px）。縦軸は全部の段でそろえる
fn fans(ui: &mut Ui, id: u64, cmd: &str, v: &FanOut) -> bool {
    let ws = Waves::of(ui);
    if v.fans.is_empty() {
        return false;
    }
    let fname = crate::plots::field_namer(ui);
    let mut fields: Vec<usize> = v.fans.iter().map(|f| f.field).collect();
    fields.dedup();
    let ymax = v
        .fans
        .iter()
        .flat_map(|f| f.tangential.iter().chain(&f.sagittal))
        .filter(|x| x.is_finite())
        .fold(0.0_f64, |m, x| m.max(x.abs()));
    let ymax = if ymax > 0.0 { ymax } else { 1.0 };
    let unit = v.unit.clone();
    let ylab = if cmd == "opd-fan" {
        trf("OPD [{}]", &[&unit])
    } else if cmd == "pupil-fan" {
        trf("Pupil aberration [{}]", &[&unit])
    } else {
        trf("Ray error [{}]", &[&unit])
    };
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let mut wl: Vec<usize> = v.fans.iter().map(|f| f.wave).collect();
    wl.sort();
    wl.dedup();
    wave_legend(ui, &ws, &wl, None);
    let h = row_height(ui, fields.len(), 150.0);
    for f in fields {
        ui.label(fname(f));
        ui.columns(2, |cols| {
            for (k, ui) in cols.iter_mut().enumerate() {
                let tan = k == 0;
                let plot = Plot::new((cmd, id, f, k))
                    .height(h)
                    .include_x(-1.0)
                    .include_x(1.0)
                    .include_y(-ymax)
                    .include_y(ymax)
                    .x_axis_label(if tan {
                        tr("Py (tangential)")
                    } else {
                        tr("Px (sagittal)")
                    })
                    .y_axis_label(ylab.clone());
                plot_tools::show(ui, plot, "", |p, reg| {
                    for r in v.fans.iter().filter(|r| r.field == f) {
                        let (col, name) = ws.style(r.wave);
                        let y = if tan { &r.tangential } else { &r.sagittal };
                        let segs = segments(&r.p, y);
                        let all: Vec<[f64; 2]> = segs.iter().flatten().copied().collect();
                        reg.push(Series::new(name.clone(), col, &all));
                        draw(p, segs, col, false, &name);
                    }
                });
            }
        });
    }
    true
}

// --------------------------------------------------------------------------- 像面湾曲・歪曲

/// 像面湾曲（左: 焦点の z 対 視野、子午は実線・球欠は破線）と歪曲（右: % 対 視野）
fn field_curv(ui: &mut Ui, id: u64, v: &FieldCurvOut) -> bool {
    let ws = Waves::of(ui);
    let kind = match v.distortion_kind {
        lensbench_analysis::defs::DistKind::FTan => "f tan θ",
        lensbench_analysis::defs::DistKind::FTheta => "f θ",
    };
    ui.label(weak(
        ui,
        trf(
            "Maximum distortion {} % ({})",
            &[&fmt_sig(v.max_distortion, 4), &kind],
        ),
    ));
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let wl: Vec<usize> = v.waves.iter().map(|w| w.wave).collect();
    wave_legend(
        ui,
        &ws,
        &wl,
        Some(tr("Solid: tangential, dashed: sagittal").to_string()),
    );
    let h = row_height(ui, 1, 200.0);
    let unit = v.unit.clone();
    ui.columns(2, |cols| {
        let plot = Plot::new(("field-curvature", id, 0))
            .height(h)
            .include_x(0.0)
            .include_y(0.0)
            .x_axis_label(trf("Focus from the image surface [{}]", &[&unit]))
            .y_axis_label(tr("Field"));
        plot_tools::show_by_y(&mut cols[0], plot, "", |p, reg| {
            for w in &v.waves {
                let (col, name) = ws.style(w.wave);
                let t = segments(&w.tangential, &v.field_value);
                let s = segments(&w.sagittal, &v.field_value);
                reg.push(Series::new(format!("{name} T"), col, &flat(&t)));
                reg.push(Series::new(format!("{name} S"), col, &flat(&s)));
                draw(p, t, col, false, &format!("{name} T"));
                draw(p, s, col, true, &format!("{name} S"));
            }
        });
        let plot = Plot::new(("field-curvature", id, 1))
            .height(h)
            .include_x(0.0)
            .include_y(0.0)
            .x_axis_label(tr("Distortion [%]"))
            .y_axis_label(tr("Field"));
        plot_tools::show_by_y(&mut cols[1], plot, "", |p, reg| {
            for w in &v.waves {
                let (col, name) = ws.style(w.wave);
                let segs = segments(&w.distortion, &v.field_value);
                reg.push(Series::new(name.clone(), col, &flat(&segs)));
                draw(p, segs, col, false, &name);
            }
        });
    });
    true
}

// ------------------------------------------------------------------------------------- 縦収差

/// 縦収差: 横軸は像面からの距離、縦軸は正規化瞳の高さ
fn longitudinal(ui: &mut Ui, id: u64, v: &LongOut) -> bool {
    let ws = Waves::of(ui);
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let wl: Vec<usize> = v.waves.iter().map(|w| w.wave).collect();
    wave_legend(ui, &ws, &wl, None);
    let h = row_height(ui, 1, 200.0);
    let plot = Plot::new(("longitudinal", id))
        .height(h)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(trf("Longitudinal aberration [{}]", &[&v.unit]))
        .y_axis_label(tr("Pupil height (normalized)"));
    plot_tools::show_by_y(ui, plot, "", |p, reg| {
        for w in &v.waves {
            let (col, name) = ws.style(w.wave);
            let segs = segments(&w.z, &v.zones);
            reg.push(Series::new(name.clone(), col, &flat(&segs)));
            draw(p, segs, col, false, &name);
        }
    });
    true
}

// --------------------------------------------------------------------------------- 倍率色収差

/// 倍率色収差: 横軸は視野、縦軸は主波長との像高の差。エアリー半径を破線で
fn lateral(ui: &mut Ui, id: u64, v: &LatOut) -> bool {
    let ws = Waves::of(ui);
    if v.airy.is_finite() {
        ui.label(weak(
            ui,
            trf(
                "Dashed lines: Airy radius ±{} {}",
                &[&fmt_sig(v.airy, 3), &v.unit],
            ),
        ));
    }
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let wl: Vec<usize> = v.waves.iter().map(|w| w.wave).collect();
    wave_legend(ui, &ws, &wl, None);
    let h = row_height(ui, 1, 200.0);
    let weak_col = theme::weak_text(ui.visuals());
    let plot = Plot::new(("lateral-color", id))
        .height(h)
        .include_x(0.0)
        .include_y(0.0)
        .x_axis_label(tr("Field"))
        .y_axis_label(trf("Lateral color [{}]", &[&v.unit]));
    plot_tools::show(ui, plot, "", |p, reg| {
        if v.airy.is_finite() {
            for s in [1.0, -1.0] {
                p.hline(
                    HLine::new(s * v.airy)
                        .color(weak_col)
                        .style(LineStyle::Dashed { length: 4.0 }),
                );
            }
        }
        for w in &v.waves {
            let (col, name) = ws.style(w.wave);
            let segs = segments(&v.field_value, &w.d);
            let all: Vec<[f64; 2]> = segs.iter().flatten().copied().collect();
            reg.push(Series::new(name.clone(), col, &all));
            draw(p, segs, col, false, &name);
        }
    });
    true
}

// ------------------------------------------------------------------------------ 色の焦点ずれ

/// 色の焦点ずれ: 横軸は波長、縦軸は主波長の近軸焦点からのずれ
fn focal_shift(ui: &mut Ui, id: u64, v: &ShiftOut) -> bool {
    ui.label(weak(
        ui,
        trf(
            "Maximum focal shift range {} {} · diffraction-limited depth {} {}",
            &[
                &fmt_sig(v.range, 4),
                &v.unit,
                &fmt_sig(v.diffraction_depth, 4),
                &v.unit,
            ],
        ),
    ));
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let h = row_height(ui, 1, 200.0);
    let col = theme::series(0);
    let plot = Plot::new(("focal-shift", id))
        .height(h)
        .include_y(0.0)
        .x_axis_label(tr("Wavelength [µm]"))
        .y_axis_label(trf("Focal shift [{}]", &[&v.unit]));
    plot_tools::show(ui, plot, "µm", |p, reg| {
        let segs = segments(&v.wavelengths, &v.shift);
        let all: Vec<[f64; 2]> = segs.iter().flatten().copied().collect();
        let name = tr("Focal shift").to_string();
        reg.push(Series::new(name.clone(), col, &all));
        draw(p, segs, col, false, &name);
    });
    true
}

// ------------------------------------------------------------------------------- グリッド歪曲

/// グリッド歪曲: 理想の格子（薄い線）と実際の主光線の点を結んだ格子
fn grid(ui: &mut Ui, id: u64, v: &GridOut) -> bool {
    let n = v.grid;
    if n < 2 || v.points.len() != n * n {
        return false;
    }
    ui.label(weak(
        ui,
        trf(
            "Maximum distortion {} % at field ({}, {})",
            &[
                &fmt_sig(v.max_distortion, 4),
                &fmt_sig(v.max_at[0], 3),
                &fmt_sig(v.max_at[1], 3),
            ],
        ),
    ));
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let h = row_height(ui, 1, 240.0);
    let (col, _) = Waves::of(ui).style(v.wave);
    let ideal_col = theme::weak_text(ui.visuals());
    let at = |i: usize, j: usize| &v.points[j * n + i];
    let plot = Plot::new(("grid-distortion", id))
        .height(h)
        .data_aspect(1.0)
        .legend(Legend::default())
        .x_axis_label(format!("x [{}]", v.unit))
        .y_axis_label(format!("y [{}]", v.unit));
    plot_tools::show(ui, plot, "", |p, _| {
        for (pick, c, name) in [
            (0usize, ideal_col, tr("Ideal (paraxial)").to_string()),
            (1, col, tr("Real chief rays").to_string()),
        ] {
            let pt = |g: &lensbench_analysis::defs::GridPoint| {
                if pick == 0 {
                    g.ideal
                } else {
                    g.real
                }
            };
            let mut first = true;
            for a in 0..n {
                for row in [true, false] {
                    let line: Vec<[f64; 2]> = (0..n)
                        .map(|b| pt(if row { at(b, a) } else { at(a, b) }))
                        .collect();
                    let mut l = Line::new(PlotPoints::from(line))
                        .color(c)
                        .width(if pick == 0 { 1.0_f32 } else { 1.4 });
                    if first {
                        l = l.name(name.clone());
                        first = false;
                    }
                    p.line(l);
                }
            }
        }
    });
    true
}

// ------------------------------------------------------------------------------- ザイデル係数

/// ザイデル係数の棒（面ごとに SPHA・COMA・ASTI・FCUR・DIST・CLA・CTR、最後の組が和）
fn seidel(ui: &mut Ui, id: u64, v: &SeidelOut) -> bool {
    const NAMES: [&str; 7] = ["SPHA", "COMA", "ASTI", "FCUR", "DIST", "CLA", "CTR"];
    let vals = |r: &SeidelRow| [r.spha, r.coma, r.asti, r.fcur, r.dist, r.cla, r.ctr];
    let rows: Vec<&SeidelRow> = v.surfaces.iter().chain(std::iter::once(&v.sum)).collect();
    let n = rows.len();
    let wbar = 0.8 / NAMES.len() as f64;
    let s = &v.sum;
    ui.label(weak(
        ui,
        trf(
            "Sum: SPHA {} · COMA {} · ASTI {} · FCUR {} · DIST {} · CLA {} · CTR {} [{}]",
            &[
                &fmt_sig(s.spha, 4),
                &fmt_sig(s.coma, 4),
                &fmt_sig(s.asti, 4),
                &fmt_sig(s.fcur, 4),
                &fmt_sig(s.dist, 4),
                &fmt_sig(s.cla, 4),
                &fmt_sig(s.ctr, 4),
                &v.unit,
            ],
        ),
    ));
    // 選んだ棒の札（x、半分の幅、値、文字）。egui_plot の札は棒の上端の右に付くので、上端が図の外（高い棒・
    // 拡大した時）や右端の棒でははみ出る。自前で図の中に描く
    let mut hits: Vec<(f64, f64, f64, String)> = Vec::new();
    let charts: Vec<BarChart> = NAMES
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let off = (k as f64 - (NAMES.len() as f64 - 1.0) / 2.0) * wbar;
            let bars: Vec<Bar> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let label = if i + 1 == n {
                        tr("Sum").to_string()
                    } else {
                        trf("Surface {}", &[&r.surf])
                    };
                    let (x, y) = ((i + 1) as f64 + off, vals(r)[k]);
                    let text = format!("{label} {name}  {}", fmt_sig(y, 4));
                    hits.push((x, wbar / 2.0, y, text));
                    Bar::new(x, y)
                        .width(wbar * 0.95)
                        .name(format!("{label} {name}"))
                })
                .collect();
            BarChart::new(bars)
                .color(theme::series(k))
                .name(*name)
                .element_formatter(Box::new(|_, _| String::new()))
        })
        .collect();
    // 凡例はグラフの外（中に置くと右の面・和の棒に重なる）
    ui.horizontal_wrapped(|ui| {
        for (k, name) in NAMES.iter().enumerate() {
            ui.label(
                egui::RichText::new(format!("■ {name}"))
                    .small()
                    .color(theme::series(k)),
            );
        }
    });
    let h = row_height(ui, 1, 220.0);
    let surfs: Vec<String> = v.surfaces.iter().map(|r| r.surf.to_string()).collect();
    let sum = tr("Sum").to_string();
    let all: Vec<f64> = rows
        .iter()
        .flat_map(|r| vals(r))
        .filter(|x| x.is_finite())
        .collect();
    let (lo, hi) = all
        .iter()
        .fold((0.0_f64, 0.0_f64), |(a, b), &x| (a.min(x), b.max(x)));
    let pad = (hi - lo).max(1e-12) * 0.12;
    let plot = Plot::new(("seidel", id))
        .height(h)
        .include_y(hi + pad)
        .include_y(lo - pad)
        .x_axis_formatter(move |m, _| {
            let i = m.value.round();
            if (m.value - i).abs() > 1e-6 || i < 1.0 {
                return String::new();
            }
            let i = i as usize;
            if i <= surfs.len() {
                surfs[i - 1].clone()
            } else if i == surfs.len() + 1 {
                sum.clone()
            } else {
                String::new()
            }
        })
        .x_axis_label(tr("Surface"))
        .y_axis_label(trf("Seidel sum [{}]", &[&v.unit]));
    let color = ui.visuals().text_color();
    plot_tools::show(ui, plot, "", |p, _| {
        for c in charts {
            p.bar_chart(c);
        }
        let at = p.pointer_coordinate().filter(|_| p.response().hovered());
        let hit = at.and_then(|m| hits.iter().find(|h| (m.x - h.0).abs() <= h.1));
        if let Some((x, _, y, text)) = hit {
            let (pos, anchor) = bar_label_at(*x, *y, &p.plot_bounds());
            p.text(Text::new(pos, egui::RichText::new(text).color(color)).anchor(anchor));
        }
    });
    true
}

/// 棒 (x, 値 y) の札の位置と向き。図の範囲 b の中に収める: 上向きの棒は上端の上（図の上端に近ければ
/// 下）、下向きは逆。左半分の棒は右へ、右半分は左へ伸ばす。端から少し内側に置く（自動の範囲を広げない）
fn bar_label_at(x: f64, y: f64, b: &PlotBounds) -> (PlotPoint, egui::Align2) {
    use egui::Align2;
    let ([x0, y0], [x1, y1]) = (b.min(), b.max());
    let (rx, ry) = ((x1 - x0) * 0.08, (y1 - y0) * 0.08);
    let yc = y.clamp(y0 + ry, (y1 - ry).max(y0 + ry));
    let above = if y >= 0.0 {
        y <= y1 - 2.0 * ry
    } else {
        y < y0 + 2.0 * ry
    };
    let right = x < (x0 + x1) / 2.0;
    let xc = x.clamp(x0 + rx, (x1 - rx).max(x0 + rx));
    let anchor = match (above, right) {
        (true, true) => Align2::LEFT_BOTTOM,
        (true, false) => Align2::RIGHT_BOTTOM,
        (false, true) => Align2::LEFT_TOP,
        (false, false) => Align2::RIGHT_TOP,
    };
    (PlotPoint::new(xc, yc), anchor)
}

// ------------------------------------------------------------------------------- フットプリント

/// フットプリント: 面の開口の輪郭（薄い線）と、視野ごとの色の光線の交点・瞳の縁
fn footprint(ui: &mut Ui, id: u64, v: &FootprintOut) -> bool {
    let fname = crate::plots::field_namer(ui);
    ui.label(weak(
        ui,
        trf(
            "Surface {} · semi-diameter {} {} · largest beam radius {} {} ({} % of the semi-diameter)",
            &[
                &v.surface,
                &fmt_sig(v.semi, 4),
                &v.unit,
                &fmt_sig(v.max_radius, 4),
                &v.unit,
                &fmt_sig(100.0 * v.fill, 3),
            ],
        ),
    ));
    let detail: Vec<String> = v
        .rows
        .iter()
        .map(|r| {
            trf(
                "{} · wavelength {}: radius {} · through {} % · clipped here {} %",
                &[
                    &fname(r.field),
                    &r.wave,
                    &fmt_sig(r.max_radius, 4),
                    &fmt_sig(100.0 * r.through, 3),
                    &fmt_sig(100.0 * r.clipped, 3),
                ],
            )
        })
        .collect();
    ui.label(weak(ui, tr("Per field (hover)")))
        .on_hover_text(detail.join("\n"));
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let h = row_height(ui, 1, 240.0);
    let ap_col = theme::weak_text(ui.visuals());
    let plot = Plot::new(("footprint", id))
        .height(h)
        .data_aspect(1.0)
        .legend(Legend::default())
        .x_axis_label(format!("x [{}]", v.unit))
        .y_axis_label(format!("y [{}]", v.unit));
    plot_tools::show(ui, plot, "", |p, _| {
        for (k, a) in v.aperture.iter().enumerate() {
            let mut l = Line::new(PlotPoints::from(a.clone()))
                .color(ap_col)
                .width(1.2_f32);
            if k == 0 {
                l = l.name(tr("Aperture").to_string());
            }
            p.line(l);
        }
        let mut named: Vec<usize> = Vec::new();
        for r in &v.rows {
            let col = theme::series(r.field.saturating_sub(1));
            let mut pts = Points::new(PlotPoints::from(r.points.clone()))
                .color(col.gamma_multiply(0.8))
                .radius(1.5_f32)
                .shape(MarkerShape::Circle);
            if !named.contains(&r.field) {
                pts = pts.name(fname(r.field));
                named.push(r.field);
            }
            p.points(pts);
            let ex: Vec<f64> = r.edge.iter().map(|q| q[0].unwrap_or(f64::NAN)).collect();
            let ey: Vec<f64> = r.edge.iter().map(|q| q[1].unwrap_or(f64::NAN)).collect();
            for s in segments(&ex, &ey) {
                p.line(Line::new(PlotPoints::from(s)).color(col).width(1.2_f32));
            }
        }
    });
    true
}

// --------------------------------------------------------------------------- 相対照度・けられ

/// 相対照度（と実効 F 値をカーソルの値に）
fn rel_illum(ui: &mut Ui, id: u64, v: &RelIllumOut) -> bool {
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    if v.transmission.is_some() {
        ui.label(weak(
            ui,
            tr("Includes the coating and Fresnel transmission relative to the axis"),
        ));
    }
    let h = row_height(ui, 1, 200.0);
    let col = theme::series(0);
    let plot = Plot::new(("relative-illumination", id))
        .height(h)
        .include_y(0.0)
        .include_y(1.0)
        .include_x(0.0)
        .x_axis_label(tr("Field"))
        .y_axis_label(tr("Relative illumination"));
    plot_tools::show(ui, plot, "", |p, reg| {
        let segs = segments(&v.field_value, &v.relative);
        let all: Vec<[f64; 2]> = segs.iter().flatten().copied().collect();
        let name = tr("Relative illumination").to_string();
        reg.push(Series::new(name.clone(), col, &all));
        draw(p, segs, col, false, &name);
    });
    true
}

/// けられずに通る光線の割合
fn vignetting(ui: &mut Ui, id: u64, v: &VignettingOut) -> bool {
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let h = row_height(ui, 1, 200.0);
    let col = theme::series(1);
    let plot = Plot::new(("vignetting", id))
        .height(h)
        .include_y(0.0)
        .include_y(1.0)
        .include_x(0.0)
        .x_axis_label(tr("Field"))
        .y_axis_label(tr("Unvignetted rays"));
    plot_tools::show(ui, plot, "", |p, reg| {
        let segs = segments(&v.field_value, &v.unvignetted);
        let all: Vec<[f64; 2]> = segs.iter().flatten().copied().collect();
        let name = tr("Unvignetted rays").to_string();
        reg.push(Series::new(name.clone(), col, &all));
        draw(p, segs, col, false, &name);
    });
    true
}

// ------------------------------------------------------------------------------ ガウスビーム

/// 近軸ガウスビーム: 光軸に沿ったビームの半径（±）と、面の位置の縦線
fn gauss(ui: &mut Ui, id: u64, v: &GaussOut) -> bool {
    if let Some(last) = v.rows.last() {
        ui.label(weak(
            ui,
            trf(
                "Image: beam radius {} {} · waist {} {} · waist to image {} {} · divergence {} mrad",
                &[
                    &fmt_sig(last.size, 4),
                    &v.unit,
                    &fmt_sig(last.waist, 4),
                    &v.unit,
                    &fmt_sig(last.position, 4),
                    &v.unit,
                    &fmt_sig(last.divergence * 1e3, 4),
                ],
            ),
        ));
    }
    if !v.warnings.is_empty() {
        ui.label(weak(ui, v.warnings.join("\n")));
    }
    let h = row_height(ui, 1, 200.0);
    let col = theme::series(0);
    let plot = Plot::new(("gaussian-beam", id))
        .height(h)
        .include_y(0.0)
        .x_axis_label(trf("Position along the axis [{}]", &[&v.unit]))
        .y_axis_label(trf("Beam radius [{}]", &[&v.unit]));
    let weak_col = ui.visuals().weak_text_color().gamma_multiply(0.5);
    plot_tools::show(ui, plot, &v.unit, |p, reg| {
        for r in &v.rows {
            if r.z.is_finite() {
                p.vline(VLine::new(r.z).color(weak_col).width(0.6_f32));
            }
        }
        let up: Vec<Vec<[f64; 2]>> = v.profile.clone();
        let down: Vec<Vec<[f64; 2]>> = v
            .profile
            .iter()
            .map(|s| s.iter().map(|q| [q[0], -q[1]]).collect())
            .collect();
        // M² の混合モードは基本モードの √M² 倍（破線）
        if v.m2 > 1.0 {
            let m = v.m2.sqrt();
            let mix = |sign: f64| -> Vec<Vec<[f64; 2]>> {
                v.profile
                    .iter()
                    .map(|s| s.iter().map(|q| [q[0], sign * m * q[1]]).collect())
                    .collect()
            };
            let name = tr("Mixed-mode beam radius").to_string();
            let c2 = theme::series(1);
            reg.push(Series::new(name.clone(), c2, &flat(&mix(1.0))));
            draw(p, mix(1.0), c2, true, &name);
            draw(p, mix(-1.0), c2, true, "");
        }
        let name = tr("Beam radius").to_string();
        reg.push(Series::new(name.clone(), col, &flat(&up)));
        draw(p, up, col, false, &name);
        draw(p, down, col, false, "");
    });
    true
}

#[cfg(test)]
mod tests {
    /// 棒の札は図の中: 上端が図の上より高い棒は上端の内側で下向きに、右半分の棒は左へ伸ばす
    #[test]
    fn bar_label_stays_inside() {
        use egui::Align2;
        let b = super::PlotBounds::from_min_max([2.0, 0.0], [5.5, 0.4]);
        let (p, a) = super::bar_label_at(3.1, 0.9, &b);
        assert!(p.y < 0.4 && p.y > 0.0, "{p:?}");
        assert_eq!(a, Align2::LEFT_TOP);
        let (p, a) = super::bar_label_at(5.4, 0.05, &b);
        assert!(p.x < 5.5, "{p:?}");
        assert_eq!(a, Align2::RIGHT_BOTTOM);
        let (p, a) = super::bar_label_at(2.5, -0.3, &b);
        assert!(p.y >= 0.0, "{p:?}");
        assert_eq!(a, Align2::LEFT_BOTTOM);
        // 図の外の右の棒も内側へ
        let (p, _) = super::bar_label_at(9.0, 0.1, &b);
        assert!(p.x < 5.5);
    }

    use super::*;

    #[test]
    fn segments_split_at_nan() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0];
        let y = [1.0, f64::NAN, 2.0, 3.0, f64::NAN];
        assert_eq!(
            segments(&x, &y),
            vec![vec![[0.0, 1.0]], vec![[2.0, 2.0], [3.0, 3.0]]]
        );
    }
}
