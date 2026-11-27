//! 解析の図の続き: 波面図と干渉縞・回折 PSF・Zernike 係数・フィールド MTF・スルーフォーカス MTF・RMS の掃引・
//! 幾何 MTF。スポット図と回折 MTF は plots.rs。拡張の解析の図は登録した関数（[`crate::ext`]）で描く。
//!
//! 図はすべて serve の analyze の JSON から描く（波面は `map` を足して頼む: [`figure_params`]）。
//! 干渉縞は波面の格子から GUI で求める（参照球との干渉、OpticStudio の Interferogram の式）。

use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_plot::{Bar, BarChart, Legend, Line, LineStyle, Plot, PlotImage, PlotPoint};
use lensbench_analysis::defs::{
    MtfFieldOut, MtfFocusOut, PsfOut, RmsOut, WavefrontOut, WavefrontRow, ZernikeOut,
};
use lensbench_analysis::Typed;
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::plots::fmt_sig;
use crate::theme;

/// 図を描ける解析（スポット・回折 MTF を含む）
pub(crate) fn has_figure(cmd: &str) -> bool {
    if let Some(a) = crate::ext::get(cmd) {
        return a.figure.is_some();
    }
    if crate::figures_aber::CMDS.contains(&cmd) {
        return true;
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
            | "interference"
            | "pop"
            | "image-sim"
            | "nsc"
    )
}

/// 図に要る引数（figure のとき）と、省けない引数の既定を足す（利用者が与えた引数はそのまま）。
/// cutoff は主波長の遮断周波数 [cycles/mm]
pub(crate) fn figure_params(
    cmd: &str,
    m: &mut Map<String, Value>,
    cutoff: Option<f64>,
    figure: bool,
) {
    if let Some(f) = crate::ext::get(cmd).and_then(|a| a.figure_params) {
        f(m, figure);
        return;
    }
    match cmd {
        "wavefront" if figure => {
            m.entry("map").or_insert(json!(true));
        }
        "interference" | "pop" | "image-sim" if figure => {
            m.entry("grid").or_insert(json!(true));
        }
        // レイアウト図に重ねる枝
        "nsc" if figure => {
            m.entry("layout_rays").or_insert(json!(NSC_LAYOUT_RAYS));
            // 経路の表の行ごとの順次の系（ゴーストの系は一時フォルダに書いて開く）
            m.entry("sequential").or_insert(json!(true));
            if let Some(d) = crate::nsc_view::ghost_dir() {
                m.entry("out").or_insert(json!(d.display().to_string()));
            }
        }
        // 周波数が要る解析は遮断周波数の 1/10・1/4・1/2（無ければ 10・20・40）
        "mtf-field" | "mtf-focus" => {
            m.entry("freqs")
                .or_insert_with(|| json!(default_freqs(cutoff)));
        }
        _ => {}
    }
}

/// 非順次の図で、レイアウト図に枝を描くために追い直す光源の光線の数
const NSC_LAYOUT_RAYS: usize = 100;

/// 周波数を頼まれていないときの既定（有効数字 2 桁）
pub(crate) fn default_freqs(cutoff: Option<f64>) -> Vec<f64> {
    match cutoff {
        Some(c) if c.is_finite() && c > 0.0 => [0.1, 0.25, 0.5]
            .iter()
            .map(|k| fmt_sig(c * k, 2).parse().unwrap_or(c * k))
            .collect(),
        _ => vec![10.0, 20.0, 40.0],
    }
}

/// 結果の世代 gen ごとに 1 度だけ作る（図の格子など。毎フレーム JSON から作り直さない）
pub(crate) fn memo<T: Send + Sync + 'static>(
    ui: &Ui,
    key: impl std::hash::Hash + std::fmt::Debug,
    gen: u64,
    f: impl FnOnce() -> T,
) -> std::sync::Arc<T> {
    let id = ui.make_persistent_id(("memo", key));
    if let Some((g, v)) = ui
        .ctx()
        .data(|d| d.get_temp::<(u64, std::sync::Arc<T>)>(id))
    {
        if g == gen {
            return v;
        }
    }
    let v = std::sync::Arc::new(f());
    ui.ctx().data_mut(|d| d.insert_temp(id, (gen, v.clone())));
    v
}

/// 解析の結果: JSON と、同じプロセスの計算からなら型のままの結果（[`Typed`]）
#[derive(Clone, Copy)]
pub(crate) struct Out<'a> {
    pub v: &'a Value,
    pub t: Option<&'a Typed>,
}

impl<'a> Out<'a> {
    pub(crate) fn new(v: &'a Value, t: Option<&'a Typed>) -> Out<'a> {
        Out { v, t }
    }

    /// 結果の型 T の値（型のままの結果があれば写さずに。Web 版は JSON から読む）。世代 gen ごとに 1 度だけ作る。
    /// 読めなければ None（呼び手は表で見せる）
    pub(crate) fn get<T: serde::de::DeserializeOwned + Send + Sync + 'static>(
        &self,
        ui: &Ui,
        key: impl std::hash::Hash + std::fmt::Debug,
        gen: u64,
    ) -> Option<std::sync::Arc<T>> {
        let (v, t) = (self.v, self.t);
        let m = memo(ui, ("out", key), gen, || {
            t.and_then(|t| t.get_arc::<T>()).or_else(|| {
                serde_json::from_value::<T>(v.clone())
                    .ok()
                    .map(std::sync::Arc::new)
            })
        });
        (*m).clone()
    }
}

/// 図を描く。描けない結果なら false（呼び手が表で見せる）。gen は結果の世代（届くたびに変わる）
pub(crate) fn show(
    ui: &mut Ui,
    id: u64,
    gen: u64,
    cmd: &str,
    out: Out<'_>,
    prev: Option<Out<'_>>,
    st: &mut FigState,
) -> bool {
    let v = out.v;
    if let Some(a) = crate::ext::get(cmd) {
        return a.figure.is_some_and(|f| f(ui, id, v));
    }
    if let Some(shown) = crate::figures_aber::show(ui, id, gen, cmd, out) {
        return shown;
    }
    // 前の結果は今の結果と同じ世代で読み直す（前の結果は今の結果が届いたときに決まる）
    macro_rules! typed {
        ($t:ty) => {{
            let now = out.get::<$t>(ui, (id, cmd, "now"), gen);
            let before = prev.and_then(|p| p.get::<$t>(ui, (id, cmd, "prev"), gen));
            (now, before)
        }};
    }
    match cmd {
        "wavefront" => match typed!(WavefrontOut).0 {
            Some(w) => wavefront(ui, id, gen, &w, st),
            None => false,
        },
        "psf" => match typed!(PsfOut).0 {
            Some(p) => psf(ui, id, gen, &p, st),
            None => false,
        },
        "zernike" => match typed!(ZernikeOut).0 {
            Some(z) => zernike(ui, id, &z),
            None => false,
        },
        "mtf-field" => match typed!(MtfFieldOut) {
            (Some(m), p) => mtf_field(ui, id, &m, p.as_deref()),
            _ => false,
        },
        "mtf-focus" => match typed!(MtfFocusOut) {
            (Some(m), p) => mtf_focus(ui, id, &m, p.as_deref(), st),
            _ => false,
        },
        "rms" => match typed!(RmsOut) {
            (Some(r), p) => rms(ui, id, &r, p.as_deref(), st),
            _ => false,
        },
        "geo-mtf" => {
            crate::plots::geo_mtf(ui, out, prev, id, gen, &mut st.mtf_prev);
            true
        }
        "mech-check" => crate::mech_table::show(ui, id, v),
        "interference" => interference(ui, id, v),
        "pop" => pop(ui, id, gen, v),
        "image-sim" => image_sim(ui, id, gen, v),
        "nsc" => crate::nsc_view::show(ui, id, gen, v, &mut st.nsc),
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
        ColorImage::new([self.w, self.h], px)
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
    salt: impl std::hash::Hash + std::fmt::Debug,
    g: &Grid,
    c: Cmap,
    lo: f64,
    hi: f64,
    labels: (&str, &str, &str),
    height: f32,
) {
    let id = ui.make_persistent_id(&salt);
    // 図の画像は値が変わったときだけ作り直す
    // 毎フレーム全画素を通すので、SipHash ではなく 64 ビットずつ混ぜる軽いもの
    let key = {
        let mix = |h: u64, x: u64| (h.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        let mut h = 0u64;
        for x in &g.v {
            h = mix(h, x.to_bits());
        }
        for x in [g.w as u64, g.h as u64, c as u64, lo.to_bits(), hi.to_bits()] {
            h = mix(h, x);
        }
        h
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
        let plot = Plot::new(("heat", id))
            .data_aspect(1.0)
            .height(height)
            .width((ui.available_width() - 72.0).max(80.0))
            .x_axis_label(xl)
            .y_axis_label(yl)
            .show_grid(false)
            .label_formatter(|h| {
                let p = match h {
                    egui_plot::HoverPosition::NearDataPoint { position, .. }
                    | egui_plot::HoverPosition::Elsewhere { position } => position,
                };
                Some(match g.at(p.x, p.y) {
                    Some(z) => format!(
                        "x {}\ny {}\n{} {}",
                        fmt_sig(p.x, 4),
                        fmt_sig(p.y, 4),
                        vl,
                        fmt_sig(z, 4)
                    ),
                    None => format!("x {}\ny {}", fmt_sig(p.x, 4), fmt_sig(p.y, 4)),
                })
            });
        // 縦横比を固定した図なので、目盛りの上でその軸だけの拡大はしない
        crate::plot_tools::nav(ui, plot, false, |p| {
            p.image(PlotImage::new(
                "",
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

/// 解析パネルの図の見せ方（パネルごと。[`crate::layout::AnaCfg`] に入れて並びと一緒に保存する）
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct FigState {
    /// 全視野の結果で選んだ視野の位置（図の種類ごと: wavefront・psf）
    pub pick: std::collections::BTreeMap<String, usize>,
    /// 波面の図の干渉縞
    pub fringe: FringeCfg,
    /// PSF を対数で
    pub psf_log: bool,
    /// through-focus の MTF で選んだ周波数の位置
    pub tf_freq: usize,
    /// RMS の図の量（0 スポット・1 波面・2 Strehl）
    pub rms_q: usize,
    /// MTF の図に前の結果（点線）を出す
    pub mtf_prev: bool,
    /// スポット図の倍率と、中心のずれ [µm]
    pub spot_zoom: f32,
    pub spot_pan: [f32; 2],
    /// スポット図の縮尺の長さ [µm]（0 は自動: 枠に合わせたきりのよい長さ）
    pub spot_bar: f64,
    /// 非順次の図の見せ方
    pub nsc: crate::nsc_view::NscFig,
}

impl Default for FigState {
    fn default() -> Self {
        FigState {
            pick: Default::default(),
            fringe: FringeCfg::default(),
            psf_log: false,
            tf_freq: 0,
            rms_q: 0,
            mtf_prev: true,
            spot_zoom: 1.0,
            spot_pan: [0.0; 2],
            spot_bar: 0.0,
            nsc: Default::default(),
        }
    }
}

/// 視野の選び（全視野の結果のとき）。選んだ項目の位置を返す。選びは画面の一時データに持つ
/// （拡張の解析の図から。内蔵の図は [`FigState::pick`]）
pub fn pick_field(ui: &mut Ui, id: u64, what: &str, items: &[Value]) -> usize {
    let key = ui.make_persistent_id(("fig-field", id, what));
    let mut k: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(key, 0usize));
    let fields: Vec<usize> = items
        .iter()
        .enumerate()
        .map(|(i, it)| it["field"].as_u64().map_or(i + 1, |f| f as usize))
        .collect();
    pick_in(ui, &mut k, &fields);
    ui.ctx().data_mut(|d| d.insert_temp(key, k));
    k
}

/// 視野の選び（k を書き換える。範囲の外なら 0 に戻す）。fields は項目ごとの視野の番号
fn pick_in(ui: &mut Ui, k: &mut usize, fields: &[usize]) {
    let fname = crate::plots::field_namer(ui);
    if *k >= fields.len() {
        *k = 0;
    }
    if fields.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (i, &f) in fields.iter().enumerate() {
                ui.selectable_value(k, i, fname(f));
            }
        });
    }
}

/// 内蔵の図の視野の選び（[`FigState::pick`] に持つ）
fn pick_state(ui: &mut Ui, st: &mut FigState, what: &str, fields: &[usize]) -> usize {
    let k = st.pick.entry(what.to_string()).or_insert(0);
    pick_in(ui, k, fields);
    *k
}

/// 小さい薄い字
pub fn weak(ui: &Ui, s: impl Into<String>) -> RichText {
    RichText::new(s.into())
        .small()
        .color(theme::weak_text(ui.visuals()))
}

// ------------------------------------------------------------------------------------- 波面・干渉縞

/// 波面の結果（1 視野）から格子を作る（行 0 = +y）
pub(crate) fn wavefront_grid(item: &WavefrontRow) -> Option<Grid> {
    let p = item.p.as_deref()?;
    let rows = item.map.as_deref()?;
    let n = p.len();
    if n < 2 || rows.len() != n {
        return None;
    }
    let mut v = Vec::with_capacity(n * n);
    for iy in (0..n).rev() {
        let r = &rows[iy];
        if r.len() != n {
            return None;
        }
        v.extend_from_slice(r);
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

/// 干渉縞の設定（パネルごと。[`FigState`] に入れて並びと一緒に保存する）
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct FringeCfg {
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
pub(crate) fn interferogram(g: &Grid, f: &FringeCfg) -> Grid {
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

fn wavefront(ui: &mut Ui, id: u64, gen: u64, w: &WavefrontOut, st: &mut FigState) -> bool {
    let items = &w.wavefront;
    if items.is_empty() || items[0].map.is_none() {
        return false;
    }
    let fields: Vec<usize> = items.iter().map(|it| it.field).collect();
    let k = pick_state(ui, st, "wavefront", &fields);
    let it = &items[k];
    let Some(g) = &*memo(ui, ("wf-grid", id, k), gen, || wavefront_grid(it)) else {
        ui.weak(tr("Cannot read the wavefront grid"));
        return true;
    };
    let mut f = st.fringe;
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut f.on, false, tr("Wavefront"));
        ui.selectable_value(&mut f.on, true, tr("Fringes"))
            .on_hover_text(tr(
                "Interference with the reference sphere (I = (1 + cos 2πΔ)/2, Δ = scale·W + tilt)",
            ));
        if f.on {
            let lbl = ui.label(RichText::new(tr("Scale")).small());
            ui.add(crate::widgets::Num::new(&mut f.scale).range(0.0..=10.0))
                .on_hover_text(tr("Factor applied to the OPD (2 for double pass)"))
                .labelled_by(lbl.id);
            let lbl = ui.label(RichText::new(tr("Tilt x")).small());
            ui.add(crate::widgets::Num::new(&mut f.xtilt).range(-200.0..=200.0))
                .on_hover_text(tr("Fringes across the pupil diameter"))
                .labelled_by(lbl.id);
            let lbl = ui.label(RichText::new(tr("Tilt y")).small());
            ui.add(crate::widgets::Num::new(&mut f.ytilt).range(-200.0..=200.0))
                .labelled_by(lbl.id);
        }
        ui.label(weak(
            ui,
            format!(
                "P-V {} · RMS {} {}",
                fmt_sig(it.pv, 4),
                fmt_sig(it.rms, 4),
                tr("waves")
            ),
        ));
    });
    st.fringe = f;
    let h = fig_height(ui, 8.0);
    let lab = (tr("Pupil x"), tr("Pupil y"));
    if f.on {
        // 縞の設定は鍵でなく世代に混ぜる（スライダーの値ごとに格子を残さない）
        let fk = [f.scale, f.xtilt, f.ytilt].map(f64::to_bits);
        let stamp = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (gen, fk).hash(&mut h);
            h.finish()
        };
        let ig = memo(ui, ("wf-ifg", id, k), stamp, || {
            interferogram(&g.upsample((256 / g.w.max(1)).clamp(1, 8)), &f)
        });
        heatmap(
            ui,
            ("ifg", id),
            &ig,
            Cmap::Gray,
            0.0,
            1.0,
            (lab.0, lab.1, tr("Intensity")),
            h,
        );
    } else {
        let (lo, hi) = g.range().unwrap_or((0.0, 1.0));
        heatmap(
            ui,
            ("wf", id),
            g,
            Cmap::CoolWarm,
            lo,
            hi,
            (lab.0, lab.1, tr("waves")),
            h,
        );
    }
    true
}

// ------------------------------------------------------------------------------------- PSF

/// PSF の結果（field・x・y・i の並び）から視野ごとの格子を作る。座標は µm
pub(crate) fn psf_grids(v: &PsfOut) -> Vec<(u64, Grid)> {
    type Pts = Vec<(f64, f64, f64)>;
    let mut out: Vec<(u64, Pts)> = Vec::new();
    for p in &v.psf {
        let f = p.field as u64;
        let q = (p.x, p.y, p.i);
        match out.iter_mut().find(|(g, _)| *g == f) {
            Some((_, pts)) => pts.push(q),
            None => out.push((f, vec![q])),
        }
    }
    // 単位: lens（mm）なら µm に。アフォーカル（角度）はそのまま
    let k = if lens_mm(&v.unit) { 1e3 } else { 1.0 };
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
pub(crate) fn crop_to_signal(g: &Grid, rel: f64) -> Grid {
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

fn psf(ui: &mut Ui, id: u64, gen: u64, v: &PsfOut, st: &mut FigState) -> bool {
    let grids = memo(ui, ("psf-grids", id), gen, || psf_grids(v));
    if grids.is_empty() {
        return false;
    }
    let fields: Vec<usize> = grids.iter().map(|(f, _)| *f as usize).collect();
    let k = pick_state(ui, st, "psf", &fields);
    let g = &*memo(ui, ("psf-crop", id, k), gen, || {
        crop_to_signal(&grids[k].1, 1e-2)
    });
    let mut log = st.psf_log;
    let peak = g.range().map(|r| r.1).unwrap_or(1.0);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut log, tr("Log (log10)"));
        ui.label(weak(
            ui,
            trf("Peak {} (aberration-free center = 1)", &[&fmt_sig(peak, 4)]),
        ));
    });
    st.psf_log = log;
    let unit = if lens_mm(&v.unit) {
        "µm".to_string()
    } else {
        v.unit.clone()
    };
    let (xl, yl) = (format!("x [{unit}]"), format!("y [{unit}]"));
    let h = fig_height(ui, 8.0);
    if log {
        let floor = (peak * 1e-6).max(1e-300);
        let lg = memo(ui, ("psf-log", id, k), gen, || {
            let mut lg = g.clone();
            for x in lg.v.iter_mut() {
                if x.is_finite() {
                    *x = x.max(floor).log10();
                }
            }
            lg
        });
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
fn zernike(ui: &mut Ui, id: u64, v: &ZernikeOut) -> bool {
    let fname = crate::plots::field_namer(ui);
    let items = &v.zernike;
    let nf = items.len().max(1);
    let wbar = 0.8 / nf as f64;
    let kind = serde_json::to_value(v.kind)
        .ok()
        .and_then(|k| k.as_str().map(str::to_string))
        .unwrap_or_else(|| "standard".into());
    let detail: Vec<String> = items
        .iter()
        .map(|it| {
            trf(
                "Field {}: fit residual RMS {} · Strehl {}",
                &[&it.field, &fmt_sig(it.rms_fit, 3), &fmt_sig(it.strehl, 3)],
            )
        })
        .collect();
    ui.label(weak(ui, trf("{} coefficients [waves]", &[&kind])))
        .on_hover_text(detail.join("\n"));
    let charts: Vec<BarChart> = items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let f = it.field as u64;
            let off = (k as f64 - (nf as f64 - 1.0) / 2.0) * wbar;
            let bars: Vec<Bar> = it
                .coef
                .iter()
                .enumerate()
                .map(|(j, &c)| {
                    Bar::new((j + 1) as f64 + off, c)
                        .width(wbar * 0.95)
                        .name(format!("Z{}", j + 1))
                })
                .collect();
            BarChart::new(fname(f as usize), bars)
                .color(theme::series(f.saturating_sub(1) as usize))
        })
        .collect();
    let plot = Plot::new(("zern", id))
        .legend(Legend::default())
        .x_axis_label(tr("Term (Noll / Fringe index)"))
        .y_axis_label(tr("Coefficient [waves]"));
    crate::plot_tools::nav(ui, plot, true, |p| {
        for c in charts {
            p.bar_chart(c);
        }
    });
    true
}

// ------------------------------------------------------------------------------------- MTF の掃引・RMS

/// フィールド MTF: 周波数ごとに色、子午は実線・球欠は破線
fn mtf_field(ui: &mut Ui, id: u64, v: &MtfFieldOut, prev: Option<&MtfFieldOut>) -> bool {
    let unit = v.unit.clone();
    let series = |v: &MtfFieldOut| -> Vec<(f64, &'static str, Vec<[f64; 2]>)> {
        let mut out: Vec<(f64, &'static str, Vec<[f64; 2]>)> = Vec::new();
        for p in &v.field_mtf {
            let (fr, x) = (p.freq, p.h);
            for (k, y) in [("tangential", p.tangential), ("sagittal", p.sagittal)] {
                // JSON に書けない値（NaN）は点にしない
                if !y.is_finite() {
                    continue;
                }
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
    let plot = Plot::new(("mtf-field", id))
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(tr("Field (normalized)"))
        .y_axis_label("MTF");
    crate::plot_tools::show(ui, plot, "", |p, reg| {
        for (fr, _, pts) in before {
            p.line(
                Line::new("", pts)
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
                if t { tr("tangential") } else { tr("sagittal") }
            );
            reg.push(crate::plot_tools::Series::new(name.clone(), col(fr), &pts));
            p.line(
                Line::new(name, pts)
                    .color(col(fr))
                    .style(if t {
                        LineStyle::Solid
                    } else {
                        LineStyle::Dashed { length: 6.0 }
                    })
                    .width(1.6_f32),
            );
        }
    });
    true
}

/// スルーフォーカス MTF: 横軸は像面のずらし、（視野・周波数）ごとに色
fn mtf_focus(
    ui: &mut Ui,
    id: u64,
    v: &MtfFocusOut,
    prev: Option<&MtfFocusOut>,
    st: &mut FigState,
) -> bool {
    let unit = v.unit.clone();
    type S = Vec<((u64, f64), &'static str, Vec<[f64; 2]>)>;
    let series = |v: &MtfFocusOut| -> S {
        let mut out: S = Vec::new();
        for p in &v.through_focus {
            let key = (p.field as u64, p.freq);
            let x = p.dz;
            for (k, y) in [("tangential", p.tangential), ("sagittal", p.sagittal)] {
                if !y.is_finite() {
                    continue;
                }
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
    for p in &v.through_focus {
        let f = p.freq;
        if !freqs.contains(&f) {
            freqs.push(f);
        }
    }
    let mut sel = st.tf_freq;
    if sel >= freqs.len() {
        sel = 0;
    }
    if freqs.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (i, f) in freqs.iter().enumerate() {
                ui.selectable_value(&mut sel, i, format!("{} {}", fmt_sig(*f, 3), unit));
            }
        });
    }
    st.tf_freq = sel;
    let fsel = freqs.get(sel).copied().unwrap_or(f64::NAN);
    let keep = |s: S| -> S { s.into_iter().filter(|x| x.0 .1 == fsel).collect() };
    let now = keep(series(v));
    let before = keep(prev.map(series).unwrap_or_default());
    let col = |q: (u64, f64)| theme::series(q.0.saturating_sub(1) as usize);
    let plot = Plot::new(("mtf-focus", id))
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(tr("Image plane shift [lens units]"))
        .y_axis_label("MTF");
    crate::plot_tools::show(ui, plot, "", |p, reg| {
        for (q, _, pts) in before {
            p.line(
                Line::new("", pts)
                    .color(col(q).gamma_multiply(0.35))
                    .style(LineStyle::Dotted { spacing: 4.0 })
                    .width(1.0_f32),
            );
        }
        for (q, k, pts) in now {
            let t = k == "tangential";
            let name = if t {
                trf("Field {} tangential", &[&q.0])
            } else {
                trf("Field {} sagittal", &[&q.0])
            };
            reg.push(crate::plot_tools::Series::new(name.clone(), col(q), &pts));
            p.line(
                Line::new(name, pts)
                    .color(col(q))
                    .style(if t {
                        LineStyle::Solid
                    } else {
                        LineStyle::Dashed { length: 6.0 }
                    })
                    .width(1.6_f32),
            );
        }
    });
    true
}

/// RMS の掃引（視野・フォーカス・波長）。量はスポット [µm]・波面 [波]・Strehl から選ぶ
fn rms(ui: &mut Ui, id: u64, v: &RmsOut, prev: Option<&RmsOut>, st: &mut FigState) -> bool {
    let fname = crate::plots::field_namer(ui);
    let vs_of = |v: &RmsOut| {
        serde_json::to_value(v.vs)
            .ok()
            .and_then(|x| x.as_str().map(str::to_string))
            .unwrap_or_else(|| "field".into())
    };
    let vs = vs_of(v);
    let (xk, xl) = match vs.as_str() {
        "focus" => ("dz", tr("Image plane shift [lens units]").to_string()),
        "wavelength" => ("wavelength", tr("Wavelengths [µm]").to_string()),
        _ => ("h", tr("Field (normalized)").to_string()),
    };
    let mut q = st.rms_q;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut q, 0, tr("Spot"));
        ui.selectable_value(&mut q, 1, tr("Wavefront"));
        ui.selectable_value(&mut q, 2, "Strehl");
    });
    st.rms_q = q;
    let lens_um = lens_mm(&v.unit);
    let (yk, yl, k) = match q {
        1 => ("wfe", tr("RMS wavefront [waves]").to_string(), 1.0),
        2 => ("strehl", "Strehl".to_string(), 1.0),
        _ => (
            "spot",
            if lens_um {
                tr("RMS spot [µm]").to_string()
            } else {
                tr("RMS spot").to_string()
            },
            if lens_um { 1e3 } else { 1.0 },
        ),
    };
    let series = |v: &RmsOut| -> Vec<(u64, Vec<[f64; 2]>)> {
        let mut out: Vec<(u64, Vec<[f64; 2]>)> = Vec::new();
        for p in &v.rms {
            let f = p.field.unwrap_or(0) as u64;
            let x = match xk {
                "dz" => p.dz,
                "wavelength" => p.wavelength,
                _ => p.h,
            }
            .unwrap_or(f64::NAN);
            let y = match yk {
                "wfe" => p.wfe,
                "strehl" => p.strehl,
                _ => p.spot,
            };
            if !y.is_finite() {
                continue;
            }
            match out.iter_mut().find(|(g, _)| *g == f) {
                Some((_, pts)) => pts.push([x, y * k]),
                None => out.push((f, vec![[x, y * k]])),
            }
        }
        out
    };
    let now = series(v);
    let before = prev
        .filter(|p| vs_of(p) == vs)
        .map(series)
        .unwrap_or_default();
    let plot = Plot::new(("rms", id))
        .legend(Legend::default())
        .include_y(0.0)
        .x_axis_label(xl)
        .y_axis_label(yl);
    crate::plot_tools::show(ui, plot, "", |p, reg| {
        for (f, pts) in before {
            p.line(
                Line::new("", pts)
                    .color(theme::series(f.saturating_sub(1) as usize).gamma_multiply(0.35))
                    .style(LineStyle::Dotted { spacing: 4.0 })
                    .width(1.0_f32),
            );
        }
        for (f, pts) in now {
            reg.push(crate::plot_tools::Series::new(
                if f > 0 {
                    fname(f as usize)
                } else {
                    String::new()
                },
                theme::series(f.saturating_sub(1) as usize),
                &pts,
            ));
            let name = if f > 0 {
                fname(f as usize)
            } else {
                String::new()
            };
            let l = Line::new(name, pts)
                .color(theme::series(f.saturating_sub(1) as usize))
                .width(1.6_f32);
            p.line(l);
        }
    });
    true
}

// ------------------------------------------------------------------------------------- 干渉（interference・拡張の干渉の図）

/// 干渉の JSON（interference と同じ形: nx・ny・grid（番号 j·nx + i、j = 0 が下）・mode・pupil_min・pupil_step・
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
/// のとき白黒にするか（interference）。quantity を書かない結果は強度で白黒
fn interference_cmap(v: &Value, gray_intensity: bool) -> Cmap {
    let q = v["quantity"].as_str().unwrap_or("intensity");
    match q {
        "opd" | "gd" | "delta" | "aolp" | "chi" => Cmap::CoolWarm,
        "intensity" if gray_intensity || v["quantity"].is_null() => Cmap::Gray,
        _ => Cmap::Viridis,
    }
}

/// 簡易 POP の出力の照度の格子（`grid_irradiance`、番号 j·nx + i、j は +y の向き）。上の行から並べ直す
pub(crate) fn pop_grid(v: &Value) -> Option<Grid> {
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
fn pop(ui: &mut Ui, id: u64, gen: u64, v: &Value) -> bool {
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
            "After surface {}: {} mm · {} · power {} · RMS width {} mm",
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
    if let Some(g) = &*memo(ui, ("pop-grid", id), gen, || pop_grid(v)) {
        let (lo, hi) = g.range().unwrap_or((0.0, 1.0));
        heatmap(
            ui,
            ("pop", id),
            g,
            Cmap::Viridis,
            lo.min(0.0),
            hi,
            ("x [mm]", "y [mm]", tr("Irradiance")),
            h,
        );
    }
    let plot = Plot::new(("pop-cut", id))
        .legend(Legend::default())
        .include_y(0.0)
        .x_axis_label(tr("Position [mm]"))
        .y_axis_label(tr("Irradiance [W/mm²]"));
    crate::plot_tools::show(ui, plot, "mm", |p, reg| {
        let pts = |a: &[f64], b: &[f64]| -> Vec<[f64; 2]> {
            a.iter().zip(b).map(|(x, y)| [*x, *y]).collect()
        };
        reg.push(crate::plot_tools::Series::new(
            tr("x section"),
            theme::series(0),
            &pts(&xs, &ix),
        ));
        reg.push(crate::plot_tools::Series::new(
            tr("y section"),
            theme::series(1),
            &pts(&ys, &iy),
        ));
        p.line(Line::new(tr("x section"), pts(&xs, &ix)).color(theme::series(0)));
        p.line(
            Line::new(tr("y section"), pts(&ys, &iy))
                .color(theme::series(1))
                .style(LineStyle::Dashed { length: 6.0 }),
        );
    });
    true
}

fn interference(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let n = |k: &str| fmt_sig(v[k].as_f64().unwrap_or(f64::NAN), 3);
    let q = v["quantity"].as_str().unwrap_or("intensity");
    let info = if v["mode"].as_str() == Some("pupil") {
        trf(
            "{} · pupil · P-V {} · RMS {} waves",
            &[&q, &n("pv_waves"), &n("rms_waves")],
        )
    } else {
        trf(
            "{} · detector · mean visibility {}",
            &[&q, &n("visibility_mean")],
        )
    };
    interference_view(ui, ("interference", id), v, &info, true)
}

/// 干渉の図（[`interference_grid`] の形の JSON）。info は図の上の説明、gray_intensity は強度を白黒にするか。
/// 格子が無ければ false
pub fn interference_view(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    v: &Value,
    info: &str,
    gray_intensity: bool,
) -> bool {
    let Some(g) = interference_grid(v) else {
        return false;
    };
    let q = v["quantity"].as_str().unwrap_or("intensity").to_string();
    ui.label(weak(ui, info)).on_hover_text(tr(
        "Set the quantity, configurations, delay and so on in ⚙ Settings (same arguments as the CLI subcommand)",
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
        (tr("Pupil x"), tr("Pupil y"))
    } else {
        ("x [mm]", "y [mm]")
    };
    let h = fig_height(ui, 8.0);
    heatmap(ui, ("ifr", salt), &g, c, lo, hi, (xl, yl, &q), h);
    true
}

/// 位置が mm か（アフォーカルの角度でない。前の結果は mm を "lens" と書いた）
fn lens_mm(unit: &str) -> bool {
    matches!(unit, "mm" | "lens")
}

#[cfg(test)]
#[path = "figures_tests.rs"]
mod tests;

/// 像の模擬の像（検出器の画素、1 で白。グレーは値も読める、RGB はそのまま色に）
fn image_sim(ui: &mut Ui, id: u64, gen: u64, v: &Value) -> bool {
    let (w, h) = (
        v["width"].as_u64().unwrap_or(0) as usize,
        v["height"].as_u64().unwrap_or(0) as usize,
    );
    let ch: Vec<Vec<f64>> = v["image"]
        .as_array()
        .map(|a| a.iter().map(f64s).collect())
        .unwrap_or_default();
    if w == 0 || ch.is_empty() || ch.iter().any(|c| c.len() != w * h) {
        return false;
    }
    let px = v["pixel"].as_f64().unwrap_or(1.0);
    let c = f64s(&v["center"]);
    let (cx, cy) = (
        c.first().copied().unwrap_or(0.0),
        c.get(1).copied().unwrap_or(0.0),
    );
    let (hw, hh) = (w as f64 * px / 2.0, h as f64 * px / 2.0);
    let extent = [cx - hw, cx + hw, cy - hh, cy + hh];
    let height = (ui.available_height() * 0.8).clamp(200.0, 720.0);
    if ch.len() == 1 {
        let g = Grid {
            w,
            h,
            v: ch[0].clone(),
            extent,
        };
        heatmap(
            ui,
            ("image-sim", id),
            &g,
            Cmap::Gray,
            0.0,
            1.0,
            ("x", "y", tr("Brightness")),
            height,
        );
        return true;
    }
    let tex = memo(ui, ("image-sim-rgb", id), gen, || {
        let u = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let pixels = (0..w * h)
            .map(|k| Color32::from_rgb(u(ch[0][k]), u(ch[1][k]), u(ch[2][k])))
            .collect();
        ui.ctx().load_texture(
            format!("image-sim-{id}"),
            ColorImage::new([w, h], pixels),
            TextureOptions::NEAREST,
        )
    });
    let plot = Plot::new(("image-sim-plot", id))
        .data_aspect(1.0)
        .height(height)
        .show_grid(false);
    crate::plot_tools::nav(ui, plot, false, |p| {
        p.image(PlotImage::new(
            "",
            tex.id(),
            PlotPoint::new(cx, cy),
            [(2.0 * hw) as f32, (2.0 * hh) as f32],
        ));
    });
    true
}
