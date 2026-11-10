//! 解析の図の続き: 波面図と干渉縞・回折 PSF・Zernike 係数・フィールド MTF・スルーフォーカス MTF・RMS の掃引・
//! 幾何 MTF。スポット図と回折 MTF は plots.rs。拡張の解析の図は登録した関数（[`crate::ext`]）で描く。
//!
//! 図はすべて serve の analyze の JSON から描く（波面は `map` を足して頼む: [`figure_params`]）。
//! 干渉縞は波面の格子から GUI で求める（参照球との干渉、OpticStudio の Interferogram の式）。

use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_plot::{Bar, BarChart, Legend, Line, LineStyle, Plot, PlotImage, PlotPoint};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::plots::fmt_sig;
use crate::theme;

/// 図を描ける解析（スポット・回折 MTF を含む）
pub fn has_figure(cmd: &str) -> bool {
    if let Some(a) = crate::ext::get(cmd) {
        return a.figure.is_some();
    }
    matches!(
        cmd,
        "spot"
            | "mtf"
            | "huygens-mtf"
            | "wavefront"
            | "zernike"
            | "psf"
            | "geo-mtf"
            | "mtf-field"
            | "mtf-focus"
            | "rms"
            | "fringe"
            | "pop"
    )
}

/// 図に要る引数（figure のとき）と、省けない引数の既定を足す（利用者が与えた引数はそのまま）。
/// cutoff は主波長の遮断周波数 [cycles/mm]
pub fn figure_params(cmd: &str, m: &mut Map<String, Value>, cutoff: Option<f64>, figure: bool) {
    if let Some(f) = crate::ext::get(cmd).and_then(|a| a.figure_params) {
        f(m, figure);
        return;
    }
    match cmd {
        "wavefront" if figure => {
            m.entry("map").or_insert(json!(true));
        }
        "fringe" | "pop" if figure => {
            m.entry("grid").or_insert(json!(true));
        }
        // 周波数が要る解析は遮断周波数の 1/10・1/4・1/2（無ければ 10・20・40）
        "mtf-field" | "mtf-focus" => {
            m.entry("freqs")
                .or_insert_with(|| json!(default_freqs(cutoff)));
        }
        _ => {}
    }
}

/// 周波数を頼まれていないときの既定（有効数字 2 桁）
pub fn default_freqs(cutoff: Option<f64>) -> Vec<f64> {
    match cutoff {
        Some(c) if c.is_finite() && c > 0.0 => [0.1, 0.25, 0.5]
            .iter()
            .map(|k| fmt_sig(c * k, 2).parse().unwrap_or(c * k))
            .collect(),
        _ => vec![10.0, 20.0, 40.0],
    }
}

/// 図を描く。描けない結果なら false（呼び手が表で見せる）
pub fn show(ui: &mut Ui, id: u64, cmd: &str, v: &Value, prev: Option<&Value>) -> bool {
    if let Some(a) = crate::ext::get(cmd) {
        return a.figure.is_some_and(|f| f(ui, id, v));
    }
    match cmd {
        "wavefront" => wavefront(ui, id, v),
        "psf" => psf(ui, id, v),
        "zernike" => zernike(ui, id, v),
        "mtf-field" => mtf_field(ui, id, v, prev),
        "mtf-focus" => mtf_focus(ui, id, v, prev),
        "rms" => rms(ui, id, v, prev),
        "geo-mtf" => {
            crate::plots::mtf_curves(ui, v, prev, id, "geometric_mtf");
            true
        }
        "fringe" => interference(ui, id, v),
        "pop" => pop(ui, id, v),
        _ => false,
    }
}

// ------------------------------------------------------------------------------------- 色の表

/// 色の表
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmap {
    /// 青→白→赤（波面）
    CoolWarm,
    /// 紺→緑→黄（PSF・強度）
    Viridis,
    /// 黒→白（干渉縞）
    Gray,
}

fn lerp_stops(stops: &[[u8; 3]], t: f64) -> Color32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let x = t * (stops.len() - 1) as f64;
    let i = (x.floor() as usize).min(stops.len() - 2);
    let f = x - i as f64;
    let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (stops[i], stops[i + 1]);
    Color32::from_rgb(m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]))
}

/// t（0〜1）の色
pub fn cmap(c: Cmap, t: f64) -> Color32 {
    match c {
        Cmap::CoolWarm => lerp_stops(
            &[
                [0x3B, 0x4C, 0xC0],
                [0x8D, 0xB0, 0xFE],
                [0xDD, 0xDD, 0xDD],
                [0xF4, 0x9A, 0x7B],
                [0xB4, 0x04, 0x26],
            ],
            t,
        ),
        Cmap::Viridis => lerp_stops(
            &[
                [0x44, 0x01, 0x54],
                [0x3B, 0x52, 0x8B],
                [0x21, 0x91, 0x8C],
                [0x5E, 0xC9, 0x62],
                [0xFD, 0xE7, 0x25],
            ],
            t,
        ),
        Cmap::Gray => {
            let g = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
            Color32::from_gray(g)
        }
    }
}

/// 2 次元の値の格子（行 0 が上 = +y）。NaN は透明
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f64>,
    /// 図の座標の範囲 [x0, x1, y0, y1]（画素の端）
    pub extent: [f64; 4],
}

impl Grid {
    /// 有限な値の最小・最大
    pub fn range(&self) -> Option<(f64, f64)> {
        let mut r: Option<(f64, f64)> = None;
        for &x in self.v.iter().filter(|x| x.is_finite()) {
            r = Some(r.map_or((x, x), |(a, b)| (a.min(x), b.max(x))));
        }
        r
    }

    /// 図の座標 (x, y) の値
    pub fn at(&self, x: f64, y: f64) -> Option<f64> {
        let [x0, x1, y0, y1] = self.extent;
        let ix = ((x - x0) / (x1 - x0) * self.w as f64).floor();
        let iy = ((y1 - y) / (y1 - y0) * self.h as f64).floor();
        if ix < 0.0 || iy < 0.0 || ix >= self.w as f64 || iy >= self.h as f64 {
            return None;
        }
        let v = self.v[iy as usize * self.w + ix as usize];
        v.is_finite().then_some(v)
    }

    fn image(&self, c: Cmap, lo: f64, hi: f64) -> ColorImage {
        let span = if hi > lo { hi - lo } else { 1.0 };
        let px: Vec<Color32> = self
            .v
            .iter()
            .map(|&x| {
                if x.is_finite() {
                    cmap(c, (x - lo) / span)
                } else {
                    Color32::TRANSPARENT
                }
            })
            .collect();
        ColorImage {
            size: [self.w, self.h],
            pixels: px,
        }
    }

    /// 値を k 倍の細かさに双一次で補間する（4 つの隣が有限な所だけ。干渉縞を滑らかに）
    pub fn upsample(&self, k: usize) -> Grid {
        if k <= 1 || self.w < 2 || self.h < 2 {
            return self.clone();
        }
        let (w, h) = (self.w * k, self.h * k);
        let mut v = vec![f64::NAN; w * h];
        let g = |x: usize, y: usize| self.v[y * self.w + x];
        for oy in 0..h {
            // 画素の中心の位置（元の画素の単位）
            let fy = ((oy as f64 + 0.5) / k as f64 - 0.5).clamp(0.0, (self.h - 1) as f64);
            let y0 = (fy.floor() as usize).min(self.h - 2);
            let ty = fy - y0 as f64;
            for ox in 0..w {
                let fx = ((ox as f64 + 0.5) / k as f64 - 0.5).clamp(0.0, (self.w - 1) as f64);
                let x0 = (fx.floor() as usize).min(self.w - 2);
                let tx = fx - x0 as f64;
                let near = g(
                    (fx.round() as usize).min(self.w - 1),
                    (fy.round() as usize).min(self.h - 1),
                );
                if !near.is_finite() {
                    continue;
                }
                let q = [g(x0, y0), g(x0 + 1, y0), g(x0, y0 + 1), g(x0 + 1, y0 + 1)];
                v[oy * w + ox] = if q.iter().all(|x| x.is_finite()) {
                    (q[0] * (1.0 - tx) + q[1] * tx) * (1.0 - ty)
                        + (q[2] * (1.0 - tx) + q[3] * tx) * ty
                } else {
                    near
                };
            }
        }
        Grid {
            w,
            h,
            v,
            extent: self.extent,
        }
    }
}

/// 格子を色の図で描く（軸・拡大・ホバーで値）。lo・hi は色の範囲
#[allow(clippy::too_many_arguments)]
pub fn heatmap(
    ui: &mut Ui,
    salt: impl std::hash::Hash,
    g: &Grid,
    c: Cmap,
    lo: f64,
    hi: f64,
    labels: (&str, &str, &str),
    height: f32,
) {
    let id = ui.make_persistent_id(&salt);
    // 図の画像は値が変わったときだけ作り直す
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for x in &g.v {
            x.to_bits().hash(&mut h);
        }
        (g.w, g.h, c as u8, lo.to_bits(), hi.to_bits()).hash(&mut h);
        h.finish()
    };
    let tex = ui
        .ctx()
        .data_mut(|d| d.get_temp::<(u64, TextureHandle)>(id));
    let tex = match tex {
        Some((k, t)) if k == key => t,
        _ => {
            let t = ui.ctx().load_texture(
                format!("fig-{:?}", id),
                g.image(c, lo, hi),
                TextureOptions::NEAREST,
            );
            ui.ctx().data_mut(|d| d.insert_temp(id, (key, t.clone())));
            t
        }
    };
    let [x0, x1, y0, y1] = g.extent;
    let (xl, yl, vl) = labels;
    let vl = vl.to_string();
    ui.horizontal(|ui| {
        Plot::new(("heat", id))
            .data_aspect(1.0)
            .height(height)
            .width((ui.available_width() - 72.0).max(80.0))
            .x_axis_label(xl)
            .y_axis_label(yl)
            .allow_scroll(false)
            .show_grid(false)
            .label_formatter(|_, p| match g.at(p.x, p.y) {
                Some(z) => format!(
                    "x {}\ny {}\n{} {}",
                    fmt_sig(p.x, 4),
                    fmt_sig(p.y, 4),
                    vl,
                    fmt_sig(z, 4)
                ),
                None => format!("x {}\ny {}", fmt_sig(p.x, 4), fmt_sig(p.y, 4)),
            })
            .show(ui, |p| {
                p.image(PlotImage::new(
                    tex.id(),
                    PlotPoint::new((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                    [(x1 - x0) as f32, (y1 - y0) as f32],
                ));
            });
        colorbar(ui, c, lo, hi, height);
    });
}

/// 色の帯と上下の値
fn colorbar(ui: &mut Ui, c: Cmap, lo: f64, hi: f64, height: f32) {
    ui.vertical(|ui| {
        let vis = ui.visuals().clone();
        ui.label(RichText::new(fmt_sig(hi, 3)).small().monospace());
        let bh = (height - 40.0).max(20.0);
        let (r, painter) = ui.allocate_painter(egui::vec2(12.0, bh), egui::Sense::hover());
        let n = 32;
        for k in 0..n {
            let t0 = k as f32 / n as f32;
            let t1 = (k + 1) as f32 / n as f32;
            let rr = egui::Rect::from_min_max(
                egui::pos2(r.rect.left(), r.rect.bottom() - t1 * bh),
                egui::pos2(r.rect.right(), r.rect.bottom() - t0 * bh),
            );
            painter.rect_filled(rr, 0.0, cmap(c, (t0 + t1) as f64 / 2.0));
        }
        painter.rect_stroke(
            r.rect,
            0.0,
            (0.5, vis.widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
        ui.label(RichText::new(fmt_sig(lo, 3)).small().monospace());
    });
}

/// 図の高さ（パネルの残りから）
pub fn fig_height(ui: &Ui, reserve: f32) -> f32 {
    (ui.available_height() - reserve).clamp(160.0, 560.0)
}

/// 数の並び（数でない要素は NaN）
pub fn f64s(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
        .unwrap_or_default()
}

/// 視野の選び（全視野の結果のとき）。選んだ項目の位置を返す
pub fn pick_field(ui: &mut Ui, id: u64, what: &str, items: &[Value]) -> usize {
    let key = ui.make_persistent_id(("fig-field", id, what));
    let mut k: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(key, 0usize));
    if k >= items.len() {
        k = 0;
    }
    if items.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (i, it) in items.iter().enumerate() {
                let f = it["field"].as_u64().unwrap_or(i as u64 + 1);
                ui.selectable_value(&mut k, i, trf("視野 {}", &[&f]));
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(key, k));
    }
    k
}

/// 小さい薄い字
pub fn weak(ui: &Ui, s: impl Into<String>) -> RichText {
    RichText::new(s.into())
        .small()
        .color(theme::weak_text(ui.visuals()))
}

// ------------------------------------------------------------------------------------- 波面・干渉縞

/// 波面の JSON（1 視野）から格子を作る（行 0 = +y）
pub fn wavefront_grid(item: &Value) -> Option<Grid> {
    let p = f64s(&item["p"]);
    let rows = item["map"].as_array()?;
    let n = p.len();
    if n < 2 || rows.len() != n {
        return None;
    }
    let mut v = Vec::with_capacity(n * n);
    for iy in (0..n).rev() {
        let r = f64s(&rows[iy]);
        if r.len() != n {
            return None;
        }
        v.extend(r);
    }
    let d = p[1] - p[0];
    Some(Grid {
        w: n,
        h: n,
        v,
        extent: [
            p[0] - d / 2.0,
            p[n - 1] + d / 2.0,
            p[0] - d / 2.0,
            p[n - 1] + d / 2.0,
        ],
    })
}

/// 干渉縞の設定（パネルごと、保存しない）
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FringeCfg {
    pub on: bool,
    /// OPD にかける数（ダブルパスは 2）
    pub scale: f64,
    /// 傾きの縞の数（瞳の直径あたり）
    pub xtilt: f64,
    pub ytilt: f64,
}

impl Default for FringeCfg {
    fn default() -> Self {
        FringeCfg {
            on: false,
            scale: 1.0,
            xtilt: 0.0,
            ytilt: 0.0,
        }
    }
}

/// 参照球との干渉縞: Δ = scale·W + (xtilt·px + ytilt·py)/2、I = (1 + cos 2πΔ)/2（OpticStudio の Interferogram）
pub fn interferogram(g: &Grid, f: &FringeCfg) -> Grid {
    let [x0, x1, y0, y1] = g.extent;
    let mut out = g.clone();
    for iy in 0..g.h {
        let py = y1 - (iy as f64 + 0.5) / g.h as f64 * (y1 - y0);
        for ix in 0..g.w {
            let px = x0 + (ix as f64 + 0.5) / g.w as f64 * (x1 - x0);
            let w = g.v[iy * g.w + ix];
            out.v[iy * g.w + ix] = if w.is_finite() {
                let d = f.scale * w + (f.xtilt * px + f.ytilt * py) / 2.0;
                0.5 * (1.0 + (std::f64::consts::TAU * d).cos())
            } else {
                f64::NAN
            };
        }
    }
    out
}

fn wavefront(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let Some(items) = v["wavefront"].as_array() else {
        return false;
    };
    if items.is_empty() || items[0]["map"].is_null() {
        return false;
    }
    let k = pick_field(ui, id, "wavefront", items);
    let it = &items[k];
    let Some(g) = wavefront_grid(it) else {
        ui.weak(tr("波面の格子が読めない"));
        return true;
    };
    let key = ui.make_persistent_id(("fringe", id));
    let mut f: FringeCfg = ui.ctx().data_mut(|d| *d.get_temp_mut_or_default(key));
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut f.on, false, tr("波面"));
        ui.selectable_value(&mut f.on, true, tr("干渉縞"))
            .on_hover_text(tr(
                "参照球との干渉（I = (1 + cos 2πΔ)/2、Δ = 倍率·W + 傾き）",
            ));
        if f.on {
            ui.label(RichText::new(tr("倍率")).small());
            ui.add(
                egui::DragValue::new(&mut f.scale)
                    .speed(0.05)
                    .range(0.0..=10.0),
            )
            .on_hover_text(tr("OPD にかける数（ダブルパスは 2）"));
            ui.label(RichText::new(tr("傾き x")).small());
            ui.add(
                egui::DragValue::new(&mut f.xtilt)
                    .speed(0.1)
                    .range(-200.0..=200.0),
            )
            .on_hover_text(tr("瞳の直径あたりの縞の数"));
            ui.label(RichText::new(tr("傾き y")).small());
            ui.add(
                egui::DragValue::new(&mut f.ytilt)
                    .speed(0.1)
                    .range(-200.0..=200.0),
            );
        }
        ui.label(weak(
            ui,
            format!(
                "P-V {} · RMS {} {}",
                fmt_sig(it["pv"].as_f64().unwrap_or(f64::NAN), 4),
                fmt_sig(it["rms"].as_f64().unwrap_or(f64::NAN), 4),
                tr("波")
            ),
        ));
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, f));
    let h = fig_height(ui, 8.0);
    let lab = (tr("瞳 x"), tr("瞳 y"));
    if f.on {
        let fine = g.upsample((256 / g.w.max(1)).clamp(1, 8));
        let ig = interferogram(&fine, &f);
        heatmap(
            ui,
            ("ifg", id),
            &ig,
            Cmap::Gray,
            0.0,
            1.0,
            (lab.0, lab.1, tr("強度")),
            h,
        );
    } else {
        let (lo, hi) = g.range().unwrap_or((0.0, 1.0));
        heatmap(
            ui,
            ("wf", id),
            &g,
            Cmap::CoolWarm,
            lo,
            hi,
            (lab.0, lab.1, tr("波")),
            h,
        );
    }
    true
}

// ------------------------------------------------------------------------------------- PSF

/// PSF の JSON（field・x・y・i の並び）から視野ごとの格子を作る。座標は µm
pub fn psf_grids(v: &Value) -> Vec<(u64, Grid)> {
    type Pts = Vec<(f64, f64, f64)>;
    let mut out: Vec<(u64, Pts)> = Vec::new();
    for p in v["psf"].as_array().into_iter().flatten() {
        let f = p["field"].as_u64().unwrap_or(1);
        let q = (
            p["x"].as_f64().unwrap_or(f64::NAN),
            p["y"].as_f64().unwrap_or(f64::NAN),
            p["i"].as_f64().unwrap_or(f64::NAN),
        );
        match out.iter_mut().find(|(g, _)| *g == f) {
            Some((_, pts)) => pts.push(q),
            None => out.push((f, vec![q])),
        }
    }
    // 単位: lens（mm）なら µm に。アフォーカル（角度）はそのまま
    let k = if v["unit"].as_str() == Some("lens") {
        1e3
    } else {
        1.0
    };
    out.into_iter()
        .filter_map(|(f, pts)| {
            let uniq = |sel: fn(&(f64, f64, f64)) -> f64| {
                let mut a: Vec<f64> = pts.iter().map(sel).filter(|x| x.is_finite()).collect();
                a.sort_by(|p, q| p.total_cmp(q));
                a.dedup_by(|p, q| (*p - *q).abs() <= 1e-12 * q.abs().max(1e-300));
                a
            };
            let xs = uniq(|p| p.0);
            let ys = uniq(|p| p.1);
            let (w, h) = (xs.len(), ys.len());
            if w < 2 || h < 2 {
                return None;
            }
            let idx = |a: &[f64], x: f64| {
                a.binary_search_by(|p| p.total_cmp(&x))
                    .unwrap_or_else(|i| i.min(a.len() - 1))
            };
            let mut g = vec![f64::NAN; w * h];
            for &(x, y, i) in &pts {
                if x.is_finite() && y.is_finite() {
                    let (ix, iy) = (idx(&xs, x), idx(&ys, y));
                    g[(h - 1 - iy) * w + ix] = i;
                }
            }
            let dx = (xs[w - 1] - xs[0]) / (w - 1) as f64;
            let dy = (ys[h - 1] - ys[0]) / (h - 1) as f64;
            Some((
                f,
                Grid {
                    w,
                    h,
                    v: g,
                    extent: [
                        (xs[0] - dx / 2.0) * k,
                        (xs[w - 1] + dx / 2.0) * k,
                        (ys[0] - dy / 2.0) * k,
                        (ys[h - 1] + dy / 2.0) * k,
                    ],
                },
            ))
        })
        .collect()
}

/// 値がピークの rel 倍を超える所を 1.5 倍に広げた正方形に切り出す（PSF の像の格子は広いので）
pub fn crop_to_signal(g: &Grid, rel: f64) -> Grid {
    let Some((_, peak)) = g.range() else {
        return g.clone();
    };
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
    for iy in 0..g.h {
        for ix in 0..g.w {
            let v = g.v[iy * g.w + ix];
            if v.is_finite() && v > peak * rel {
                x0 = x0.min(ix);
                x1 = x1.max(ix);
                y0 = y0.min(iy);
                y1 = y1.max(iy);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return g.clone();
    }
    // 中心と半幅（画素）。正方形にして 1.5 倍、最小 8 画素
    let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
    let half = (((x1 - x0).max(y1 - y0) as f64 + 1.0) * 0.75).max(4.0);
    let lo = |c: f64, n: usize| ((c - half).floor().max(0.0) as usize).min(n - 1);
    let hi = |c: f64, n: usize| ((c + half).ceil() as usize).min(n - 1);
    let (ax, bx, ay, by) = (lo(cx, g.w), hi(cx, g.w), lo(cy, g.h), hi(cy, g.h));
    let (w, h) = (bx - ax + 1, by - ay + 1);
    let mut v = Vec::with_capacity(w * h);
    for iy in ay..=by {
        v.extend_from_slice(&g.v[iy * g.w + ax..=iy * g.w + bx]);
    }
    let [ex0, ex1, ey0, ey1] = g.extent;
    let (dx, dy) = ((ex1 - ex0) / g.w as f64, (ey1 - ey0) / g.h as f64);
    Grid {
        w,
        h,
        v,
        extent: [
            ex0 + ax as f64 * dx,
            ex0 + (bx + 1) as f64 * dx,
            ey1 - (by + 1) as f64 * dy,
            ey1 - ay as f64 * dy,
        ],
    }
}

fn psf(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let grids = psf_grids(v);
    if grids.is_empty() {
        return false;
    }
    let items: Vec<Value> = grids.iter().map(|(f, _)| json!({ "field": f })).collect();
    let k = pick_field(ui, id, "psf", &items);
    let g = &crop_to_signal(&grids[k].1, 1e-2);
    let key = ui.make_persistent_id(("psf-log", id));
    let mut log: bool = ui.ctx().data_mut(|d| *d.get_temp_mut_or(key, false));
    let peak = g.range().map(|r| r.1).unwrap_or(1.0);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut log, tr("対数（log10）"));
        ui.label(weak(
            ui,
            trf("ピーク {}（収差のない瞳の中心 = 1）", &[&fmt_sig(peak, 4)]),
        ));
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, log));
    let unit = if v["unit"].as_str() == Some("lens") {
        "µm".to_string()
    } else {
        v["unit"].as_str().unwrap_or("").to_string()
    };
    let (xl, yl) = (format!("x [{unit}]"), format!("y [{unit}]"));
    let h = fig_height(ui, 8.0);
    if log {
        let mut lg = g.clone();
        let floor = (peak * 1e-6).max(1e-300);
        for x in lg.v.iter_mut() {
            if x.is_finite() {
                *x = x.max(floor).log10();
            }
        }
        let hi = peak.max(floor).log10();
        heatmap(
            ui,
            ("psf", id, true),
            &lg,
            Cmap::Viridis,
            hi - 6.0,
            hi,
            (&xl, &yl, "log10 I"),
            h,
        );
    } else {
        heatmap(
            ui,
            ("psf", id, false),
            g,
            Cmap::Viridis,
            0.0,
            peak,
            (&xl, &yl, "I"),
            h,
        );
    }
    true
}

// ------------------------------------------------------------------------------------- Zernike

/// Zernike 係数の棒（視野ごとに色、項の番号は 1 始まり）
fn zernike(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let Some(items) = v["zernike"].as_array() else {
        return false;
    };
    let nf = items.len().max(1);
    let wbar = 0.8 / nf as f64;
    let kind = v["kind"].as_str().unwrap_or("standard").to_string();
    let detail: Vec<String> = items
        .iter()
        .map(|it| {
            trf(
                "視野 {}: 当てはめの残り RMS {} · Strehl {}",
                &[
                    &it["field"].as_u64().unwrap_or(0),
                    &fmt_sig(it["rms_fit"].as_f64().unwrap_or(f64::NAN), 3),
                    &fmt_sig(it["strehl"].as_f64().unwrap_or(f64::NAN), 3),
                ],
            )
        })
        .collect();
    ui.label(weak(ui, trf("{} の係数 [波]", &[&kind])))
        .on_hover_text(detail.join("\n"));
    let charts: Vec<BarChart> = items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let f = it["field"].as_u64().unwrap_or(k as u64 + 1);
            let off = (k as f64 - (nf as f64 - 1.0) / 2.0) * wbar;
            let bars: Vec<Bar> = f64s(&it["coef"])
                .iter()
                .enumerate()
                .map(|(j, &c)| {
                    Bar::new((j + 1) as f64 + off, c)
                        .width(wbar * 0.95)
                        .name(format!("Z{}", j + 1))
                })
                .collect();
            BarChart::new(bars)
                .color(theme::series(f.saturating_sub(1) as usize))
                .name(trf("視野 {}", &[&f]))
        })
        .collect();
    Plot::new(("zern", id))
        .legend(Legend::default())
        .x_axis_label(tr("項（Noll / Fringe の番号）"))
        .y_axis_label(tr("係数 [波]"))
        .allow_scroll(false)
        .show(ui, |p| {
            for c in charts {
                p.bar_chart(c);
            }
        });
    true
}

// ------------------------------------------------------------------------------------- MTF の掃引・RMS

/// フィールド MTF: 周波数ごとに色、子午は実線・球欠は破線
fn mtf_field(ui: &mut Ui, id: u64, v: &Value, prev: Option<&Value>) -> bool {
    let Some(_) = v["field_mtf"].as_array() else {
        return false;
    };
    let unit = v["unit"].as_str().unwrap_or("cycles/mm").to_string();
    let series = |v: &Value| -> Vec<(f64, &'static str, Vec<[f64; 2]>)> {
        let mut out: Vec<(f64, &'static str, Vec<[f64; 2]>)> = Vec::new();
        for p in v["field_mtf"].as_array().into_iter().flatten() {
            let fr = p["freq"].as_f64().unwrap_or(f64::NAN);
            let x = p["h"].as_f64().unwrap_or(f64::NAN);
            for k in ["tangential", "sagittal"] {
                let Some(y) = p[k].as_f64() else { continue };
                match out.iter_mut().find(|(f, kk, _)| *f == fr && *kk == k) {
                    Some((_, _, pts)) => pts.push([x, y]),
                    None => out.push((fr, k, vec![[x, y]])),
                }
            }
        }
        out
    };
    let now = series(v);
    let before = prev.map(series).unwrap_or_default();
    let freqs: Vec<f64> = {
        let mut f: Vec<f64> = now.iter().map(|s| s.0).collect();
        f.dedup();
        f
    };
    let col = |fr: f64| theme::series(freqs.iter().position(|&x| x == fr).unwrap_or(0));
    Plot::new(("mtf-field", id))
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(tr("視野（正規化）"))
        .y_axis_label("MTF")
        .allow_scroll(false)
        .show(ui, |p| {
            for (fr, _, pts) in before {
                p.line(
                    Line::new(pts)
                        .color(col(fr).gamma_multiply(0.35))
                        .style(LineStyle::Dotted { spacing: 4.0 })
                        .width(1.0_f32),
                );
            }
            for (fr, k, pts) in now {
                let t = k == "tangential";
                let name = format!(
                    "{} {} {}",
                    fmt_sig(fr, 3),
                    unit,
                    if t { tr("子午") } else { tr("球欠") }
                );
                p.line(
                    Line::new(pts)
                        .color(col(fr))
                        .style(if t {
                            LineStyle::Solid
                        } else {
                            LineStyle::Dashed { length: 6.0 }
                        })
                        .width(1.6_f32)
                        .name(name),
                );
            }
        });
    true
}

/// スルーフォーカス MTF: 横軸は像面のずらし、（視野・周波数）ごとに色
fn mtf_focus(ui: &mut Ui, id: u64, v: &Value, prev: Option<&Value>) -> bool {
    let Some(_) = v["through_focus"].as_array() else {
        return false;
    };
    let unit = v["unit"].as_str().unwrap_or("cycles/mm").to_string();
    type S = Vec<((u64, f64), &'static str, Vec<[f64; 2]>)>;
    let series = |v: &Value| -> S {
        let mut out: S = Vec::new();
        for p in v["through_focus"].as_array().into_iter().flatten() {
            let key = (
                p["field"].as_u64().unwrap_or(1),
                p["freq"].as_f64().unwrap_or(f64::NAN),
            );
            let x = p["dz"].as_f64().unwrap_or(f64::NAN);
            for k in ["tangential", "sagittal"] {
                let Some(y) = p[k].as_f64() else { continue };
                match out.iter_mut().find(|(q, kk, _)| *q == key && *kk == k) {
                    Some((_, _, pts)) => pts.push([x, y]),
                    None => out.push((key, k, vec![[x, y]])),
                }
            }
        }
        out
    };
    // 周波数は 1 つずつ見る（色は視野、子午は実線・球欠は破線）
    let mut freqs: Vec<f64> = Vec::new();
    for p in v["through_focus"].as_array().into_iter().flatten() {
        let f = p["freq"].as_f64().unwrap_or(f64::NAN);
        if !freqs.contains(&f) {
            freqs.push(f);
        }
    }
    let fk = ui.make_persistent_id(("tf-freq", id));
    let mut sel: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(fk, 0usize));
    if sel >= freqs.len() {
        sel = 0;
    }
    if freqs.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (i, f) in freqs.iter().enumerate() {
                ui.selectable_value(&mut sel, i, format!("{} {}", fmt_sig(*f, 3), unit));
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(fk, sel));
    }
    let fsel = freqs.get(sel).copied().unwrap_or(f64::NAN);
    let keep = |s: S| -> S { s.into_iter().filter(|x| x.0 .1 == fsel).collect() };
    let now = keep(series(v));
    let before = keep(prev.map(series).unwrap_or_default());
    let col = |q: (u64, f64)| theme::series(q.0.saturating_sub(1) as usize);
    Plot::new(("mtf-focus", id))
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(tr("像面のずらし [レンズの単位]"))
        .y_axis_label("MTF")
        .allow_scroll(false)
        .show(ui, |p| {
            for (q, _, pts) in before {
                p.line(
                    Line::new(pts)
                        .color(col(q).gamma_multiply(0.35))
                        .style(LineStyle::Dotted { spacing: 4.0 })
                        .width(1.0_f32),
                );
            }
            for (q, k, pts) in now {
                let t = k == "tangential";
                let name = if t {
                    trf("視野 {} 子午", &[&q.0])
                } else {
                    trf("視野 {} 球欠", &[&q.0])
                };
                p.line(
                    Line::new(pts)
                        .color(col(q))
                        .style(if t {
                            LineStyle::Solid
                        } else {
                            LineStyle::Dashed { length: 6.0 }
                        })
                        .width(1.6_f32)
                        .name(name),
                );
            }
        });
    true
}

/// RMS の掃引（視野・フォーカス・波長）。量はスポット [µm]・波面 [波]・Strehl から選ぶ
fn rms(ui: &mut Ui, id: u64, v: &Value, prev: Option<&Value>) -> bool {
    let Some(_) = v["rms"].as_array() else {
        return false;
    };
    let vs = v["vs"].as_str().unwrap_or("field").to_string();
    let (xk, xl) = match vs.as_str() {
        "focus" => ("dz", tr("像面のずらし [レンズの単位]").to_string()),
        "wavelength" => ("wavelength", tr("波長 [µm]").to_string()),
        _ => ("h", tr("視野（正規化）").to_string()),
    };
    let key = ui.make_persistent_id(("rms-q", id));
    let mut q: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(key, 0usize));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut q, 0, tr("スポット"));
        ui.selectable_value(&mut q, 1, tr("波面"));
        ui.selectable_value(&mut q, 2, "Strehl");
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, q));
    let lens_um = v["unit"].as_str() == Some("lens");
    let (yk, yl, k) = match q {
        1 => ("wfe", tr("RMS 波面 [波]").to_string(), 1.0),
        2 => ("strehl", "Strehl".to_string(), 1.0),
        _ => (
            "spot",
            if lens_um {
                tr("RMS スポット [µm]").to_string()
            } else {
                tr("RMS スポット").to_string()
            },
            if lens_um { 1e3 } else { 1.0 },
        ),
    };
    let series = |v: &Value| -> Vec<(u64, Vec<[f64; 2]>)> {
        let mut out: Vec<(u64, Vec<[f64; 2]>)> = Vec::new();
        for p in v["rms"].as_array().into_iter().flatten() {
            let f = p["field"].as_u64().unwrap_or(0);
            let x = p[xk].as_f64().unwrap_or(f64::NAN);
            let Some(y) = p[yk].as_f64() else { continue };
            match out.iter_mut().find(|(g, _)| *g == f) {
                Some((_, pts)) => pts.push([x, y * k]),
                None => out.push((f, vec![[x, y * k]])),
            }
        }
        out
    };
    let now = series(v);
    let before = prev
        .filter(|p| p["vs"].as_str() == Some(vs.as_str()))
        .map(series)
        .unwrap_or_default();
    Plot::new(("rms", id))
        .legend(Legend::default())
        .include_y(0.0)
        .x_axis_label(xl)
        .y_axis_label(yl)
        .allow_scroll(false)
        .show(ui, |p| {
            for (f, pts) in before {
                p.line(
                    Line::new(pts)
                        .color(theme::series(f.saturating_sub(1) as usize).gamma_multiply(0.35))
                        .style(LineStyle::Dotted { spacing: 4.0 })
                        .width(1.0_f32),
                );
            }
            for (f, pts) in now {
                let mut l = Line::new(pts)
                    .color(theme::series(f.saturating_sub(1) as usize))
                    .width(1.6_f32);
                if f > 0 {
                    l = l.name(trf("視野 {}", &[&f]));
                }
                p.line(l);
            }
        });
    true
}

// ------------------------------------------------------------------------------------- 干渉（fringe・拡張の干渉の図）

/// 干渉の JSON（fringe と同じ形: nx・ny・grid（番号 j·nx + i、j = 0 が下）・mode・pupil_min・pupil_step・
/// top_left_pixel_center_mm・pixel_mm）から格子を作る（行 0 = 上）。座標は mm（瞳上の縞は正規化瞳座標）
pub fn interference_grid(v: &Value) -> Option<Grid> {
    let nx = v["nx"].as_u64()? as usize;
    let ny = v["ny"].as_u64()? as usize;
    let g = f64s(&v["grid"]);
    if nx == 0 || ny == 0 || g.len() != nx * ny {
        return None;
    }
    let mut out = Vec::with_capacity(nx * ny);
    for j in (0..ny).rev() {
        out.extend_from_slice(&g[j * nx..(j + 1) * nx]);
    }
    // 左上の画素の中心と間隔
    let (x0, ytop, d) = if v["mode"].as_str() == Some("pupil") {
        let p0 = v["pupil_min"].as_f64()?;
        let d = v["pupil_step"].as_f64()?;
        (p0, p0 + (ny as f64 - 1.0) * d, d)
    } else {
        let tl = &v["top_left_pixel_center_mm"];
        (tl[0].as_f64()?, tl[1].as_f64()?, v["pixel_mm"].as_f64()?)
    };
    Some(Grid {
        w: nx,
        h: ny,
        v: out,
        extent: [
            x0 - d / 2.0,
            x0 + (nx as f64 - 0.5) * d,
            ytop - (ny as f64 - 0.5) * d,
            ytop + d / 2.0,
        ],
    })
}

/// 量の色の表（符号のある量は青→赤、縞の強度は白黒、ほかは紺→黄）。gray_intensity は quantity が intensity
/// のとき白黒にするか（fringe）。quantity を書かない結果は強度で白黒
fn interference_cmap(v: &Value, gray_intensity: bool) -> Cmap {
    let q = v["quantity"].as_str().unwrap_or("intensity");
    match q {
        "opd" | "gd" | "delta" | "aolp" | "chi" => Cmap::CoolWarm,
        "intensity" if gray_intensity || v["quantity"].is_null() => Cmap::Gray,
        _ => Cmap::Viridis,
    }
}

/// 簡易 POP の出力の照度の格子（`grid_irradiance`、番号 j·nx + i、j は +y の向き）。上の行から並べ直す
pub fn pop_grid(v: &Value) -> Option<Grid> {
    let xs = f64s(&v["x"]);
    let ys = f64s(&v["y"]);
    let g = f64s(&v["grid_irradiance"]);
    let (nx, ny) = (xs.len(), ys.len());
    if nx < 2 || ny < 2 || g.len() != nx * ny {
        return None;
    }
    let mut out = Vec::with_capacity(nx * ny);
    for j in (0..ny).rev() {
        out.extend_from_slice(&g[j * nx..(j + 1) * nx]);
    }
    let dx = (xs[nx - 1] - xs[0]) / (nx - 1) as f64;
    let dy = (ys[ny - 1] - ys[0]) / (ny - 1) as f64;
    Some(Grid {
        w: nx,
        h: ny,
        v: out,
        extent: [
            xs[0] - dx / 2.0,
            xs[nx - 1] + dx / 2.0,
            ys[0] - dy / 2.0,
            ys[ny - 1] + dy / 2.0,
        ],
    })
}

/// 簡易 POP: 出力の照度の図と、中心を通る x・y の断面
fn pop(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let xs = f64s(&v["x"]);
    let ys = f64s(&v["y"]);
    let ix = f64s(&v["irradiance_x"]);
    let iy = f64s(&v["irradiance_y"]);
    if xs.len() != ix.len() || ys.len() != iy.len() || xs.is_empty() {
        return false;
    }
    let n = |x: &Value| fmt_sig(x.as_f64().unwrap_or(f64::NAN), 4);
    let o = &v["output"];
    ui.label(weak(
        ui,
        trf(
            "面 {} の後 {} mm · {} · パワー {} · RMS 幅 {} mm",
            &[
                &v["surface"],
                &n(&v["distance"]),
                &v["method"].as_str().unwrap_or_default(),
                &n(&o["power"]),
                &o["rms_width"]
                    .as_array()
                    .map(|a| a.iter().map(n).collect::<Vec<_>>().join(" × "))
                    .unwrap_or_else(|| n(&o["rms_width"])),
            ],
        ),
    ));
    let h = (ui.available_height() * 0.55).clamp(160.0, 520.0);
    if let Some(g) = pop_grid(v) {
        let (lo, hi) = g.range().unwrap_or((0.0, 1.0));
        heatmap(
            ui,
            ("pop", id),
            &g,
            Cmap::Viridis,
            lo.min(0.0),
            hi,
            ("x [mm]", "y [mm]", tr("照度")),
            h,
        );
    }
    Plot::new(("pop-cut", id))
        .legend(Legend::default())
        .include_y(0.0)
        .x_axis_label(tr("位置 [mm]"))
        .y_axis_label(tr("照度 [W/mm²]"))
        .allow_scroll(false)
        .show(ui, |p| {
            let pts = |a: &[f64], b: &[f64]| -> Vec<[f64; 2]> {
                a.iter().zip(b).map(|(x, y)| [*x, *y]).collect()
            };
            p.line(
                Line::new(pts(&xs, &ix))
                    .color(theme::series(0))
                    .name(tr("x の断面")),
            );
            p.line(
                Line::new(pts(&ys, &iy))
                    .color(theme::series(1))
                    .style(LineStyle::Dashed { length: 6.0 })
                    .name(tr("y の断面")),
            );
        });
    true
}

fn interference(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let n = |k: &str| fmt_sig(v[k].as_f64().unwrap_or(f64::NAN), 3);
    let q = v["quantity"].as_str().unwrap_or("intensity");
    let info = if v["mode"].as_str() == Some("pupil") {
        trf(
            "{} · 瞳上 · P-V {} · RMS {} 波",
            &[&q, &n("pv_waves"), &n("rms_waves")],
        )
    } else {
        trf(
            "{} · 像面上 · 可視度の平均 {}",
            &[&q, &n("visibility_mean")],
        )
    };
    interference_view(ui, ("fringe", id), v, &info, true)
}

/// 干渉の図（[`interference_grid`] の形の JSON）。info は図の上の説明、gray_intensity は強度を白黒にするか。
/// 格子が無ければ false
pub fn interference_view(
    ui: &mut Ui,
    salt: impl std::hash::Hash,
    v: &Value,
    info: &str,
    gray_intensity: bool,
) -> bool {
    let Some(g) = interference_grid(v) else {
        return false;
    };
    let q = v["quantity"].as_str().unwrap_or("intensity").to_string();
    ui.label(weak(ui, info)).on_hover_text(tr(
        "量・構成・遅延などは「⚙ 設定」の欄で（CLI のサブコマンドと同じ引数）",
    ));
    let (lo, hi) = g.range().unwrap_or((0.0, 1.0));
    let c = interference_cmap(v, gray_intensity);
    let (lo, hi) = if c == Cmap::CoolWarm {
        let m = lo.abs().max(hi.abs());
        (-m, m)
    } else if c == Cmap::Gray && lo >= 0.0 {
        (0.0, hi.max(1e-300))
    } else {
        (lo, hi)
    };
    let pupil = v["mode"].as_str() == Some("pupil");
    let (xl, yl) = if pupil {
        (tr("瞳 x"), tr("瞳 y"))
    } else {
        ("x [mm]", "y [mm]")
    };
    let h = fig_height(ui, 8.0);
    heatmap(ui, ("ifr", salt), &g, c, lo, hi, (xl, yl, &q), h);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_and_freqs() {
        let mut m = Map::new();
        figure_params("wavefront", &mut m, None, true);
        assert_eq!(m["map"], json!(true));
        let mut m = Map::new();
        figure_params("wavefront", &mut m, None, false);
        assert!(m.is_empty());
        figure_params("mtf-focus", &mut m, None, false);
        assert_eq!(m["freqs"], json!([10.0, 20.0, 40.0]));
        let mut m = Map::new();
        m.insert("freqs".into(), json!([5]));
        figure_params("mtf-field", &mut m, Some(300.0), true);
        assert_eq!(m["freqs"], json!([5]));
        assert_eq!(default_freqs(Some(325.4)), vec![33.0, 81.0, 160.0]);
        assert_eq!(default_freqs(None), vec![10.0, 20.0, 40.0]);
        assert!(has_figure("fringe") && !has_figure("info"));
        let mut m = Map::new();
        figure_params("fringe", &mut m, None, true);
        assert_eq!(m["grid"], json!(true));
    }

    #[test]
    fn wavefront_and_fringes() {
        let item = json!({
            "p": [-1.0, 0.0, 1.0],
            "map": [[null, 0.0, null], [0.0, 0.25, 0.0], [null, 0.5, null]],
        });
        let g = wavefront_grid(&item).unwrap();
        // 行 0 は +y（map の最後の行）
        assert_eq!(g.v[1], 0.5);
        assert!(g.v[0].is_nan());
        assert_eq!(g.extent, [-1.5, 1.5, -1.5, 1.5]);
        assert_eq!(g.at(0.0, 1.0), Some(0.5));
        assert_eq!(g.range(), Some((0.0, 0.5)));
        let ig = interferogram(&g, &FringeCfg::default());
        // W = 0.25 波 → cos(π/2) = 0 → 0.5、W = 0.5 → 0
        assert!((ig.v[4] - 0.5).abs() < 1e-12);
        assert!(ig.v[1].abs() < 1e-12);
        assert!(ig.v[0].is_nan());
        let up = g.upsample(2);
        assert_eq!((up.w, up.h), (6, 6));
        assert!(up.v.iter().any(|x| x.is_finite()));
    }

    #[test]
    fn pop_grid_rows_top_first() {
        let v = json!({ "x": [-1.0, 0.0, 1.0], "y": [-0.5, 0.5],
            "grid_irradiance": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] });
        let g = pop_grid(&v).unwrap();
        assert_eq!((g.w, g.h), (3, 2));
        assert_eq!(g.v, vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
        assert_eq!(g.extent, [-1.5, 1.5, -1.0, 1.0]);
        assert!(pop_grid(&json!({ "x": [0.0], "y": [0.0] })).is_none());
    }

    #[test]
    fn psf_grid() {
        let mut pts = Vec::new();
        for (iy, y) in [-1e-3, 0.0, 1e-3].iter().enumerate() {
            for (ix, x) in [-1e-3, 0.0, 1e-3].iter().enumerate() {
                pts.push(json!({ "field": 1, "x": x, "y": y, "i": (iy * 3 + ix) as f64 }));
            }
        }
        let g = psf_grids(&json!({ "unit": "lens", "psf": pts }));
        assert_eq!(g.len(), 1);
        let g = &g[0].1;
        assert_eq!((g.w, g.h), (3, 3));
        // 行 0 は +y（iy = 2）
        assert_eq!(g.v[0], 6.0);
        assert!((g.extent[1] - 1.5).abs() < 1e-9);
    }

    #[test]
    fn crop() {
        let mut v = vec![0.0; 100];
        v[5 * 10 + 5] = 1.0;
        let g = Grid {
            w: 10,
            h: 10,
            v,
            extent: [0.0, 10.0, 0.0, 10.0],
        };
        let c = crop_to_signal(&g, 1e-3);
        assert_eq!((c.w, c.h), (9, 9));
        // 値の位置は切り出しても同じ座標
        assert_eq!(c.at(5.5, 4.5), Some(1.0));
        assert_eq!(g.at(5.5, 4.5), Some(1.0));
    }

    #[test]
    fn interference_grids() {
        // 2×2、j = 0 が下（y の小さい側）
        let v = json!({
            "mode": "detector", "quantity": "intensity", "nx": 2, "ny": 2,
            "top_left_pixel_center_mm": [-0.5, 0.5], "pixel_mm": 1.0,
            "grid": [1.0, 2.0, 3.0, null],
        });
        let g = interference_grid(&v).unwrap();
        assert_eq!(g.extent, [-1.0, 1.0, -1.0, 1.0]);
        assert_eq!(g.at(-0.5, 0.5), Some(3.0));
        assert_eq!(g.at(0.5, 0.5), None);
        assert_eq!(g.at(0.5, -0.5), Some(2.0));
        let p = json!({
            "mode": "pupil", "nx": 2, "ny": 2, "pupil_min": -0.5, "pupil_step": 1.0,
            "grid": [0.0, 0.0, 1.0, 1.0],
        });
        let g = interference_grid(&p).unwrap();
        assert_eq!(g.v, vec![1.0, 1.0, 0.0, 0.0]);
        assert_eq!(g.extent, [-1.0, 1.0, -1.0, 1.0]);
        assert!(interference_grid(&json!({ "nx": 2, "ny": 2 })).is_none());
    }

    #[test]
    fn colors() {
        assert_eq!(cmap(Cmap::Gray, 1.0), Color32::from_gray(255));
        assert_eq!(
            cmap(Cmap::Viridis, 0.0),
            Color32::from_rgb(0x44, 0x01, 0x54)
        );
        assert_eq!(
            cmap(Cmap::CoolWarm, f64::NAN),
            Color32::from_rgb(0x3B, 0x4C, 0xC0)
        );
    }
}
