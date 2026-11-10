//! 未完のカタログ品（厚さ・径の足りない品）を補う窓（公開 #70）。
//!
//! 未完の品を足そうとすると開き、品ページを 1 件だけ開く（Windows はアプリ内のブラウザ [`crate::webview`]、
//! ほかは既定のブラウザ）。アプリ内のブラウザはページの文字を送ってくるので、仕様表（`Thickness (mm):` など）を
//! 読んで欄に入れる。CAD 欄から STEP を落とせば測って入れる。既定のブラウザのときは、ページの文字を写して
//! 貼り付けるか、落とした STEP を選ぶ。ユーザーが値を確かめて確定すると、serve の `catalog_fill` で
//! 補足のカタログ（設定のフォルダの `catalog-fill/<メーカー>.json`）に普通の品として保存する。
//!
//! ページは選んだ品の 1 件だけを開き、裏でまとめて取りに行かない。ボットの判定・captcha はそのまま見せる。

use egui::{Color32, RichText, Ui};
use lensbench_project::catalog_fill::{read_page, PageRead, StepSize};
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::catalog::LensRow;
use crate::i18n::{tr, trf};

/// 補足のカタログを置くフォルダ（ネイティブは設定のフォルダの `catalog-fill`）
#[cfg(not(target_arch = "wasm32"))]
static FILL_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 補足のカタログのフォルダを決める（起動時に 1 回）
#[cfg(not(target_arch = "wasm32"))]
pub fn set_fill_dir(settings_dir: Option<PathBuf>) {
    if let Some(d) = settings_dir {
        let _ = FILL_DIR.set(d.join("catalog-fill"));
    }
}

/// 補足のカタログのフォルダ（レンズのフォルダに足す）
pub fn fill_dir() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    return Some(PathBuf::from(crate::web_store::USER_CATALOGS));
    #[cfg(not(target_arch = "wasm32"))]
    FILL_DIR.get().cloned()
}

/// メーカーの補足のカタログのファイル
pub fn fill_file(vendor: &str) -> Option<PathBuf> {
    let safe: String = vendor
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.is_empty() {
        "vendor".into()
    } else {
        safe
    };
    #[cfg(target_arch = "wasm32")]
    return fill_dir().map(|d| d.join(format!("fill-{safe}.json")));
    #[cfg(not(target_arch = "wasm32"))]
    fill_dir().map(|d| d.join(format!("{safe}.json")))
}

/// 径の欄の文字（`25`・`25 x 35`）を値（数か [幅, 高さ]）に
pub fn parse_diameter(s: &str) -> Option<Value> {
    let nums: Vec<f64> = s
        .split(|c: char| c == 'x' || c == 'X' || c == '×' || c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok())
        .collect::<Option<_>>()?;
    match nums.as_slice() {
        [d] if *d > 0.0 && d.is_finite() => Some(json!(d)),
        [a, b] if *a > 0.0 && *b > 0.0 && a.is_finite() && b.is_finite() => Some(json!([a, b])),
        _ => None,
    }
}

/// 値（数か [幅, 高さ]）を欄の文字に
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Array(a) => a
            .iter()
            .map(|x| crate::json_view::fmt_num(x.as_f64().unwrap_or(f64::NAN)))
            .collect::<Vec<_>>()
            .join(" x "),
        x => x
            .as_f64()
            .map(crate::json_view::fmt_num)
            .unwrap_or_default(),
    }
}

fn parse_thickness(s: &str) -> Option<f64> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|t| *t > 0.0 && t.is_finite())
}

/// 品ページのアプリ内のブラウザを起こす（hidden は見せずに）
#[cfg(windows)]
fn spawn(r: &LensRow, hidden: bool) -> Result<crate::webview::Child, String> {
    let dl = std::env::temp_dir()
        .join("lensbench-gui")
        .join("downloads")
        .join(
            r.part
                .replace(|c: char| !c.is_alphanumeric() && c != '-', "_"),
        );
    let title = format!("{} {} — lensbench", r.vendor, r.part);
    crate::webview::Child::spawn(&r.url, &title, &dl, hidden)
}

/// 裏の読み取りの決め事: 行に止まってから開くまで、前に開いてから次を開くまで、値を待つ長さ
#[cfg_attr(not(windows), allow(dead_code))]
pub const AUTO_DWELL: std::time::Duration = std::time::Duration::from_millis(1500);
#[cfg_attr(not(windows), allow(dead_code))]
pub const AUTO_GAP: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg_attr(not(windows), allow(dead_code))]
pub const AUTO_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// ページの読み取りから、足りない値がすべて一通りに決まるなら catalog_fill の値（厚さ・径）。
/// 同じ値の欄に違う値が並ぶ・決められない欄だけ・ありえない大きさなら None（窓に出してユーザーが決める）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn sure_values(missing: &[String], r: &PageRead) -> Option<(Option<f64>, Option<Value>)> {
    let mut t = None;
    let mut d = None;
    for m in missing {
        let vals: Vec<&Value> = r
            .found
            .iter()
            .filter(|f| f.name == m.as_str())
            .filter_map(|f| f.value.as_ref())
            .collect();
        let first = vals.first()?;
        if vals.iter().any(|v| v != first) {
            return None;
        }
        let ok = |x: f64, hi: f64| x > 0.0 && x <= hi;
        match m.as_str() {
            "thickness" => {
                let x = r.thickness.filter(|x| ok(*x, 200.0))?;
                t = Some(x);
            }
            "diameter" => {
                let v = r.diameter.clone()?;
                let good = match &v {
                    Value::Array(a) => a.iter().all(|x| x.as_f64().is_some_and(|x| ok(x, 1000.0))),
                    x => x.as_f64().is_some_and(|x| ok(x, 1000.0)),
                };
                if !good {
                    return None;
                }
                d = Some(v);
            }
            _ => return None,
        }
    }
    Some((t, d))
}

/// catalog_fill の params
#[cfg_attr(not(windows), allow(dead_code))]
fn fill_params(r: &LensRow, t: Option<f64>, d: Option<&Value>, from: &str) -> Option<Value> {
    let out = fill_file(&r.vendor)?;
    let mut p = json!({
        "catalog": r.source.clone()?,
        "item": r.part,
        "out": out.display().to_string(),
        "from": from,
    });
    if let Some(t) = t {
        p["thickness"] = json!(t);
    }
    if let Some(d) = d {
        p["diameter"] = d.clone();
    }
    Some(p)
}

/// 補う窓へ引き継ぐもの: 品・見せていないブラウザ・最後に届いたページ（URL と文字）
#[cfg(windows)]
pub type Handover = (LensRow, crate::webview::Child, Option<(String, String)>);

/// 裏の読み取りの結果
#[cfg_attr(not(windows), allow(dead_code))]
pub enum AutoOut {
    /// 値が決まったので保存する（catalog_fill の params）
    Save(Value),
    /// 決まらないので補う窓に出す
    #[cfg(windows)]
    Show(Box<Handover>),
}

/// 行を選んだら、その品のページだけを見えない窓で開いて仕様表を読み、値が一通りに決まればそのまま保存する
/// （Windows）。矢印キーで流したときに続けて開かないよう、行に [`AUTO_DWELL`] 止まってから開き、
/// 次を開くまで [`AUTO_GAP`] 空ける。1 度に 1 件、同じ品はこの起動の間は 1 度だけ。決まらない
/// （欄が無い・食い違う・ボットの判定や captcha）ときは [`AUTO_WAIT`] の後に窓を見せて補う窓へ
#[derive(Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct AutoFill {
    /// 選んでいる未完の品（メーカー・型番）と選んだ時刻
    sel: Option<(String, web_time::Instant)>,
    /// 開いた品
    tried: std::collections::HashSet<String>,
    last_open: Option<web_time::Instant>,
    #[cfg(windows)]
    job: Option<AutoJob>,
}

#[cfg(windows)]
struct AutoJob {
    row: LensRow,
    child: crate::webview::Child,
    started: web_time::Instant,
    /// 最後に届いたページの文字
    text: Option<(String, String)>,
}

impl AutoFill {
    /// 毎フレーム: 選んでいる行（未完の品なら）を見て、開く・読む・保存する
    pub fn tick(&mut self, ctx: &egui::Context, sel: Option<&LensRow>) -> Option<AutoOut> {
        let now = web_time::Instant::now();
        let row = sel.filter(|r| r.is_incomplete() && !r.url.is_empty() && r.source.is_some());
        let key = row.map(|r| format!("{}\t{}", r.vendor, r.part));
        if key.as_deref() != self.sel.as_ref().map(|(k, _)| k.as_str()) {
            self.sel = key.map(|k| (k, now));
        }
        #[cfg(windows)]
        {
            if let Some(out) = self.poll(now) {
                return Some(out);
            }
            if self.job.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
                return None;
            }
            let (Some(r), Some((k, t0))) = (row, self.sel.clone()) else {
                return None;
            };
            if self.tried.contains(&k) || fill_file(&r.vendor).is_none() {
                return None;
            }
            let ready = (t0 + AUTO_DWELL).max(self.last_open.map_or(now, |t| t + AUTO_GAP));
            if now < ready {
                ctx.request_repaint_after(ready - now);
                return None;
            }
            self.tried.insert(k);
            self.last_open = Some(now);
            if let Ok(child) = spawn(r, true) {
                self.job = Some(AutoJob {
                    row: r.clone(),
                    child,
                    started: now,
                    text: None,
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
        #[cfg(not(windows))]
        let _ = (ctx, row);
        None
    }

    /// 補う窓を開いた品は裏で読まない（読んでいれば止める）
    pub fn forget(&mut self, part: &str) {
        #[cfg(windows)]
        if self.job.as_ref().is_some_and(|j| j.row.part == part) {
            self.job = None;
        }
        #[cfg(not(windows))]
        let _ = part;
    }

    #[cfg(windows)]
    fn poll(&mut self, now: web_time::Instant) -> Option<AutoOut> {
        let job = self.job.as_mut()?;
        let mut closed = false;
        for m in job.child.poll() {
            match m {
                crate::webview::Msg::Page { url, text } => {
                    let miss = job.row.missing();
                    if let Some((t, d)) = sure_values(&miss, &read_page(&text)) {
                        let p = fill_params(&job.row, t, d.as_ref(), "page");
                        self.job = None;
                        return p.map(AutoOut::Save);
                    }
                    job.text = Some((url, text));
                }
                crate::webview::Msg::Closed => closed = true,
                crate::webview::Msg::Download { .. } => {}
            }
        }
        if closed {
            self.job = None;
            return None;
        }
        if now.duration_since(job.started) >= AUTO_WAIT {
            let j = self.job.take()?;
            return Some(AutoOut::Show(Box::new((j.row, j.child, j.text))));
        }
        None
    }
}

/// 窓の状態
#[derive(Default)]
pub struct FillWin {
    pub open: bool,
    row: Option<LensRow>,
    thickness: String,
    diameter: String,
    /// 欄の値の出どころ（page・step・manual）
    from: &'static str,
    /// 貼り付けたページの文字
    paste: String,
    /// 仕様表の読み取り（ページの URL と結果）
    read: Option<(String, PageRead)>,
    /// 測った STEP（ファイルと結果）
    step: Option<(String, Result<StepSize, String>)>,
    /// 最後の操作の結果（誤りなら true）
    note: Option<(String, bool)>,
    /// 確定を頼んで返事を待っている
    pub pending: bool,
    /// 品ページを開いたか（開くのは 1 回だけ）
    opened: bool,
    /// アプリ内のブラウザ
    #[cfg(windows)]
    child: Option<crate::webview::Child>,
}

impl FillWin {
    /// 未完の品を選んで開く
    pub fn start(&mut self, r: LensRow) {
        *self = FillWin {
            open: true,
            from: "manual",
            ..Default::default()
        };
        // 分かっている値は欄に入れておく
        if let Some(v) = r.raw["specs"].get("diameter") {
            self.diameter = value_text(v);
        }
        if let Some(v) = r.raw["specs"].get("thickness") {
            self.thickness = value_text(v);
        }
        self.row = Some(r);
    }

    /// 裏の読み取り（[`AutoFill`]）で値が決まらなかった品を、開いているページのまま引き継ぐ
    #[cfg(windows)]
    pub fn adopt(
        &mut self,
        r: LensRow,
        mut child: crate::webview::Child,
        text: Option<(String, String)>,
    ) {
        self.start(r);
        self.opened = true;
        child.show();
        self.child = Some(child);
        if let Some((url, t)) = text {
            self.take_page(url, &t);
        }
        self.note = Some((
            tr("ページから値が決まらなかったので、窓に出しました").to_string(),
            false,
        ));
    }

    /// serve の `catalog_fill` の結果（この窓の品でなければ何もしない。裏の読み取りの保存も同じ結果で来る）
    pub fn done(&mut self, v: &Value) {
        if self.row.as_ref().map(|r| r.part.as_str()) != v["item"].as_str() {
            return;
        }
        self.pending = false;
        self.note = Some((
            trf(
                "{} に保存しました。一覧では普通の品として並びます",
                &[&v["catalog"].as_str().unwrap_or_default()],
            ),
            false,
        ));
        self.row = None;
        self.open = false;
        #[cfg(windows)]
        {
            self.child = None;
        }
    }

    pub fn failed(&mut self, e: &str) {
        self.pending = false;
        self.note = Some((e.to_string(), true));
    }

    fn missing(&self) -> Vec<String> {
        self.row.as_ref().map(|r| r.missing()).unwrap_or_default()
    }

    /// 品ページを開く（Windows はアプリ内のブラウザ、開けなければ既定のブラウザ）
    fn open_page(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.row else { return };
        if r.url.is_empty() {
            return;
        }
        self.opened = true;
        #[cfg(windows)]
        {
            match spawn(r, false) {
                Ok(c) => {
                    self.child = Some(c);
                    return;
                }
                Err(e) => {
                    self.note = Some((
                        trf(
                            "アプリ内のブラウザを開けないので既定のブラウザで開きます: {}",
                            &[&e],
                        ),
                        true,
                    ));
                }
            }
        }
        ctx.open_url(egui::OpenUrl::new_tab(&r.url));
    }

    /// ページの文字を読んで、空の欄に入れる
    fn take_page(&mut self, url: String, text: &str) {
        let r = read_page(text);
        let mut filled = false;
        if self.thickness.trim().is_empty() || self.from == "page" {
            if let Some(t) = r.thickness {
                self.thickness = crate::json_view::fmt_num(t);
                filled = true;
            }
        }
        if self.diameter.trim().is_empty() || self.from == "page" {
            if let Some(d) = &r.diameter {
                self.diameter = value_text(d);
                filled = true;
            }
        }
        if filled && self.from != "step" {
            self.from = "page";
        }
        self.read = Some((url, r));
    }

    /// STEP を測って、空の欄に入れる
    #[cfg(not(target_arch = "wasm32"))]
    fn take_step(&mut self, path: PathBuf) {
        let name = path.display().to_string();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let res = if ext == "step" || ext == "stp" {
            lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| lensbench_project::catalog_fill::read_step(&t))
        } else {
            Err(tr("STEP（.step・.stp）でないファイル").to_string())
        };
        if let Ok(s) = &res {
            let miss = self.missing();
            if miss.iter().any(|m| m == "thickness") && self.thickness.trim().is_empty() {
                self.thickness = crate::json_view::fmt_num(s.thickness);
                self.from = "step";
            }
            if miss.iter().any(|m| m == "diameter") && self.diameter.trim().is_empty() {
                self.diameter = value_text(&s.diameter);
                self.from = "step";
            }
        }
        self.step = Some((name, res));
    }

    /// 窓を描く。確定したら serve の `catalog_fill` の params を返す
    pub fn window(&mut self, ctx: &egui::Context) -> Option<Value> {
        let r = self.row.clone()?;
        if !self.opened {
            self.open_page(ctx);
        }
        #[cfg(windows)]
        if let Some(c) = &mut self.child {
            let mut closed = false;
            for m in c.poll() {
                match m {
                    crate::webview::Msg::Page { url, text } => self.take_page(url, &text),
                    crate::webview::Msg::Download {
                        path: Some(p),
                        ok: true,
                    } => self.take_step(p),
                    crate::webview::Msg::Download { .. } => {}
                    crate::webview::Msg::Closed => closed = true,
                }
            }
            if closed {
                self.child = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        let mut open = self.open;
        let mut req = None;
        egui::Window::new(trf("未完の品を補う — {} {}", &[&r.vendor, &r.part]))
            .id(egui::Id::new("catalog_fill"))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                req = self.body(ui, &r);
            });
        if !open {
            self.open = false;
            self.row = None;
            #[cfg(windows)]
            {
                self.child = None;
            }
        }
        req
    }

    fn body(&mut self, ui: &mut Ui, r: &LensRow) -> Option<Value> {
        let warn = ui.visuals().warn_fg_color;
        if let Some(t) = r.raw["title"].as_str() {
            ui.label(t);
        }
        ui.colored_label(
            warn,
            trf(
                "足りない値: {}。値を入れて確定すると置けるようになります。",
                &[&r.missing_text()],
            ),
        );
        if let Some(n) = r.raw["notes"].as_array() {
            for x in n.iter().filter_map(Value::as_str) {
                ui.weak(x);
            }
        }
        ui.horizontal_wrapped(|ui| {
            if !r.url.is_empty() {
                if ui.button(tr("🌐 品ページを開き直す")).clicked() {
                    #[cfg(windows)]
                    {
                        self.child = None;
                    }
                    self.open_page(ui.ctx());
                }
                ui.hyperlink_to(tr("既定のブラウザで開く"), &r.url);
            }
        });
        #[cfg(windows)]
        ui.weak(if self.child.is_some() {
            tr("品ページの仕様表を読んでいます。CAD 欄から STEP を落とすと測ります。ボットの判定や captcha が出たら、ページの指示に従ってください。")
        } else {
            tr("アプリ内のブラウザは閉じています。ページの文字を写して下に貼り付けるか、STEP を選んでください。")
        });
        #[cfg(not(windows))]
        ui.weak(tr("品ページを既定のブラウザで開きました。ページの文字を全部選んで写し、下に貼り付けるか、CAD 欄から落とした STEP を選んでください。"));
        ui.separator();
        ui.collapsing(tr("ページの文字を貼り付ける"), |ui| {
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.paste)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .hint_text("Thickness (mm): 3.00"),
                    );
                });
            if ui.button(tr("仕様表を読む")).clicked() {
                let t = std::mem::take(&mut self.paste);
                self.take_page(tr("貼り付けた文字").to_string(), &t);
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        if ui.button(tr("📂 STEP を選んで測る…")).clicked() {
            if let Some(p) = crate::files::open("fill_step", true, || {
                crate::files::FileDialog::new().add_filter("STEP", &["step", "stp", "STEP", "STP"])
            }) {
                self.take_step(p);
            }
        }
        // 読んだ値（仕様表と STEP を並べて見比べる）
        if let Some((url, pr)) = &self.read {
            ui.label(RichText::new(trf("仕様表（{}）", &[url])).strong());
            if pr.found.is_empty() {
                ui.colored_label(warn, tr("厚さ・径の欄が見つかりません"));
            }
            for f in &pr.found {
                let v = f
                    .value
                    .as_ref()
                    .map_or(tr("（決められない）").to_string(), |v| {
                        format!("{} mm", value_text(v))
                    });
                ui.label(format!("{}: {}  →  {v}", f.label, f.raw));
            }
        }
        if let Some((name, res)) = &self.step {
            ui.label(RichText::new(trf("STEP（{}）", &[name])).strong());
            match res {
                Ok(s) => {
                    ui.label(trf(
                        "厚さ {} mm、径 {} mm",
                        &[
                            &crate::json_view::fmt_num(s.thickness),
                            &value_text(&s.diameter),
                        ],
                    ));
                    ui.weak(&s.note);
                    // 仕様表と食い違えば知らせる（マウント込みの STEP など）
                    if let Some(t) = self.read.as_ref().and_then(|(_, p)| p.thickness) {
                        if (t - s.thickness).abs() > 0.05 {
                            ui.colored_label(
                                warn,
                                trf("仕様表の厚さ {} mm と違います（マウント込みの形かもしれません）", &[&crate::json_view::fmt_num(t)]),
                            );
                        }
                    }
                }
                Err(e) => {
                    ui.colored_label(Color32::RED, e);
                }
            }
        }
        ui.separator();
        let miss = self.missing();
        let mut changed = false;
        egui::Grid::new("fill_grid").num_columns(2).show(ui, |ui| {
            if miss.iter().any(|m| m == "thickness") {
                ui.label(tr("厚さ [mm]"));
                changed |= ui.text_edit_singleline(&mut self.thickness).changed();
                ui.end_row();
            }
            if miss.iter().any(|m| m == "diameter") {
                ui.label(tr("径 [mm]"));
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut self.diameter).hint_text("25 / 25 x 35"))
                    .changed();
                ui.end_row();
            }
        });
        if changed {
            self.from = "manual";
        }
        let t = parse_thickness(&self.thickness);
        let d = parse_diameter(&self.diameter);
        let ok = miss.iter().all(|m| match m.as_str() {
            "thickness" => t.is_some(),
            "diameter" => d.is_some(),
            _ => false,
        });
        let out = fill_file(&r.vendor);
        let mut req = None;
        ui.horizontal(|ui| {
            let can = ok && !self.pending && out.is_some() && r.source.is_some();
            if ui
                .add_enabled(can, egui::Button::new(tr("✔ 確定して保存")))
                .on_hover_text(tr("値を確かめてから確定してください。補足のカタログに保存し、元のカタログは書き換えません"))
                .clicked()
            {
                req = fill_params(r, t, d.as_ref(), self.from);
                self.pending = req.is_some();
            }
            if self.pending {
                ui.spinner();
            }
            ui.weak(match self.from {
                "page" => tr("値: 仕様表から"),
                "step" => tr("値: STEP から"),
                _ => tr("値: 入力"),
            });
        });
        if out.is_none() {
            ui.colored_label(
                Color32::RED,
                tr("補足のカタログを置く設定のフォルダがありません"),
            );
        }
        if let Some((n, err)) = &self.note {
            ui.colored_label(
                if *err {
                    Color32::RED
                } else {
                    ui.visuals().text_color()
                },
                n,
            );
        }
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diameter_text() {
        assert_eq!(parse_diameter("25"), Some(json!(25.0)));
        assert_eq!(parse_diameter(" 25 x 35 "), Some(json!([25.0, 35.0])));
        assert_eq!(parse_diameter("25×35"), Some(json!([25.0, 35.0])));
        assert_eq!(parse_diameter(""), None);
        assert_eq!(parse_diameter("-1"), None);
        assert_eq!(parse_diameter("abc"), None);
        assert_eq!(value_text(&json!([25.0, 35.0])), "25 x 35");
        assert_eq!(value_text(&json!(12.5)), "12.5");
    }

    fn row(missing: &[&str]) -> LensRow {
        let raw = json!({
            "vendor": "V", "part": "F-1", "incomplete": true, "missing": missing,
            "url": "", "specs": { "diameter": 25 }, "source": { "catalog": "/x/V.json", "item": "F-1" },
        });
        let mut r = crate::catalog::parse_lenses(&json!({ "lenses": [raw] })).remove(0);
        r.url.clear();
        r
    }

    #[test]
    fn page_text_fills_empty_fields() {
        let mut w = FillWin::default();
        w.start(row(&["thickness"]));
        assert_eq!(w.diameter, "25");
        assert!(w.thickness.is_empty());
        w.take_page("u".into(), "Thickness (mm):\n3.00 ±0.10\n");
        assert_eq!(w.thickness, "3");
        assert_eq!(w.from, "page");
        // 値の無いページでは欄を変えない
        w.thickness = "2".into();
        w.from = "manual";
        w.take_page("u".into(), "Access denied");
        assert_eq!(w.thickness, "2");
        assert!(w.read.as_ref().unwrap().1.found.is_empty());
    }

    #[test]
    fn sure_values_only_when_unambiguous() {
        let m = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let r = read_page("Thickness (mm):\n3.00\nDiameter (mm):\n25.00\n");
        assert_eq!(
            sure_values(&m(&["thickness", "diameter"]), &r),
            Some((Some(3.0), Some(json!(25.0))))
        );
        // 欄が無い（ボットの判定のページなど）
        assert_eq!(
            sure_values(&m(&["thickness"]), &read_page("Verify you are human")),
            None
        );
        // 同じ値の欄に違う値
        let r = read_page("Thickness (mm): 3.00\nSubstrate Thickness (mm): 2.00\n");
        assert_eq!(sure_values(&m(&["thickness"]), &r), None);
        // 決められない欄だけ
        let r = read_page("Substrate Thickness (mm):\n2.00 & 3.00\n");
        assert_eq!(sure_values(&m(&["thickness"]), &r), None);
        // ありえない大きさ
        let r = read_page("Thickness (mm): 3000\n");
        assert_eq!(sure_values(&m(&["thickness"]), &r), None);
    }

    #[test]
    fn auto_fill_waits_on_the_row() {
        // 選んだばかりの行は開かない（Windows 以外は開かない）
        let ctx = egui::Context::default();
        let mut a = AutoFill::default();
        let mut r = row(&["thickness"]);
        r.url = "https://example.com/p/1".into();
        assert!(a.tick(&ctx, Some(&r)).is_none());
        assert!(a.tried.is_empty());
    }

    #[test]
    fn incomplete_row_is_not_placed() {
        let mut b = crate::catalog::Browser::default();
        let r = row(&["thickness"]);
        assert!(r.is_incomplete() && !r.placeable());
        b.add_pick(&r);
        assert_eq!(b.combo.len(), 0);
        assert_eq!(b.fill_req.as_ref().map(|x| x.part.as_str()), Some("F-1"));
    }
}
