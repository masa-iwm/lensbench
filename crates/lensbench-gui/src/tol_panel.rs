//! モンテカルロ公差のパネル: 公差の組・玉や群ごとの上書き・個別の公差・補償子・評価関数を並べ、serve の
//! `tolerance` で回して、評価関数の統計・累積分布・寄与の大きい項目（感度）・補償子の動きを出す。
//!
//! 設定は `lensbench tolerance --spec` と同じ JSON（[`TolPanel::spec`]）で、そのままファイルに保存・読込できる。
//! 補償子・評価関数・構成の欄は最適化のパネル（[`crate::optim_panel`]）と同じものを使う。
//! 走っている間は [`lensbench_engine::tolerance_progress`] で進み具合を読み、止めるのは
//! [`lensbench_engine::tolerance_cancel`]。

use egui::{RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints};
use serde_json::{json, Map, Value};

use crate::i18n::{n_, tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::{self as op, section, small_edit, text_of, OptPanel};
use crate::param_pick::{self as pp, Set};
use crate::theme;
use crate::widgets::sheet::{self, SheetState};

mod sheets;
use lensbench_design::tol_report::StatsInfo;
use lensbench_design::tolerance::ToleranceReport;
use sheets::{GroupSheet, LensSheet, TolSheet};

/// 公差の組の選択肢（"" は既定 = 全体の組）
const PRESETS: [(&str, &str); 5] = [
    ("commercial", n_("Commercial")),
    ("precision", n_("Precision")),
    ("high", n_("High")),
    ("none", n_("Off")),
    ("", n_("Default")),
];

const DISTS: [(&str, &str); 3] = [
    ("normal", n_("Normal (±2σ)")),
    ("uniform", n_("Uniform")),
    ("end", n_("End (± limits)")),
];

fn preset_label(k: &str) -> &'static str {
    PRESETS.iter().find(|x| x.0 == k).map_or("?", |x| tr(x.1))
}

fn dist_label(k: &str) -> &'static str {
    DISTS.iter().find(|x| x.0 == k).map_or("?", |x| tr(x.1))
}

/// 玉ごとの上書き（組を選ぶ）
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LensRow {
    pub name: String,
    pub preset: String,
}

/// 群・ブロックの公差（空は作らない）
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GroupRow {
    pub name: String,
    pub decenter: String,
    pub tilt: String,
    pub shift: String,
}

/// 個別の公差
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TolRow {
    /// パラメータ（カンマ区切りで複数: 同じずれを与える）
    pub param: String,
    /// 1 つでも params（並び）で書く
    pub multi: bool,
    pub pm: String,
    /// ± の代わりの範囲
    pub lo: String,
    pub hi: String,
    /// "" は既定の分布
    pub dist: String,
    pub relative: bool,
    /// パラメータごとにずれに掛ける数（カンマ区切り）
    pub scale: String,
    /// 結果での名前（空はパラメータの名前）
    pub label: String,
}

/// パネルの操作（app が serve に送る）
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Run(Value),
    Stop,
    Save(Value),
    Load,
    /// 結果の JSON を保存する
    SaveResult(Value),
    /// 試行ごとの補償子の CSV を保存する
    SaveCsv(String),
}

pub(crate) struct TolPanel {
    pub trials: usize,
    pub seed: u64,
    pub dist: String,
    /// 偏心・傾き・くさびの x・y を組にして円の中で引く
    pub radial: bool,
    /// 円の中の分布（uniform・normal・normal:σの数）
    pub radial_dist: String,
    /// くさびを玉の前の面に入れる（false は後ろの面。玉のもとの向きで、wedge_face）
    pub wedge_front: bool,
    pub preset: String,
    pub catalog: bool,
    pub sensitivity: bool,
    pub comp_iters: usize,
    /// メーカーごとのカタログの玉の公差（catalog_tols）
    pub catalog_tols: Vec<crate::tol_catalog::VendorTol>,
    /// 出す百分位（空白・カンマ区切り。空は既定）
    pub percentiles: String,
    /// 試行ごとのずれと評価を結果に入れる
    pub samples: bool,
    /// スレッド数（0 は自動）
    pub threads: usize,
    pub lenses: Vec<LensRow>,
    pub groups: Vec<GroupRow>,
    pub tols: Vec<TolRow>,
    /// 補償子（vars）・評価関数（merit・terms）・構成（configs）
    pub opt: OptPanel,
    pub running: bool,
    pub result: Option<std::sync::Arc<ToleranceReport>>,
    pub error: Option<String>,
    /// 結果の補償子の分布の図: 選んだ補償子（補償子、続いて傾きの大きさ）と、累積分布で出すか（false はヒストグラム）
    pub comp_sel: usize,
    pub comp_cdf: bool,
    /// 設定の JSON にあってパネルに欄のない項目（読んだまま保存・実行に使う）
    pub extra: Map<String, Value>,
    /// 玉・群・個別の公差の表の選択と書き換え中の文字
    pub lens_sheet: SheetState,
    pub group_sheet: SheetState,
    pub tol_sheet: SheetState,
}

impl Default for TolPanel {
    fn default() -> TolPanel {
        let mut opt = OptPanel::new();
        opt.merit = "wavefront".into();
        TolPanel {
            trials: 100,
            seed: 1,
            dist: "normal".into(),
            radial: false,
            radial_dist: "uniform".into(),
            wedge_front: false,
            preset: "commercial".into(),
            catalog: true,
            sensitivity: true,
            comp_iters: 10,
            catalog_tols: Vec::new(),
            percentiles: String::new(),
            samples: false,
            threads: 0,
            lenses: Vec::new(),
            groups: Vec::new(),
            tols: Vec::new(),
            opt,
            running: false,
            result: None,
            error: None,
            comp_sel: 0,
            comp_cdf: false,
            extra: Map::new(),
            lens_sheet: SheetState::default(),
            group_sheet: SheetState::default(),
            tol_sheet: SheetState::default(),
        }
    }
}

impl TolPanel {
    /// serve の tolerance の params（保存するファイルと同じ。configs も入る）
    pub(crate) fn spec(&self) -> Result<Value, String> {
        let op = self.opt.params()?;
        let mut m = self.extra.clone();
        m.insert("trials".into(), json!(self.trials));
        m.insert("seed".into(), json!(self.seed));
        m.insert("dist".into(), json!(self.dist));
        if self.radial {
            let d = self.radial_dist.trim();
            if d.is_empty() || d == "uniform" {
                m.insert("radial".into(), json!(true));
            } else {
                m.insert("radial".into(), json!(d));
            }
        } else {
            m.remove("radial");
        }
        m.insert("preset".into(), json!(self.preset));
        if self.wedge_front {
            m.insert("wedge_face".into(), json!("front"));
        } else {
            m.remove("wedge_face");
        }
        m.insert("catalog".into(), json!(self.catalog));
        m.insert("sensitivity".into(), json!(self.sensitivity));
        m.insert("comp_iters".into(), json!(self.comp_iters));
        // 空なら extra の分（欄で表せない読んだ値）があればそれを使う
        if let Some(v) = crate::tol_catalog::to_json(&self.catalog_tols)? {
            m.insert("catalog_tols".into(), v);
        }
        let pc = self
            .percentiles
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .map(|t| {
                t.parse::<f64>()
                    .ok()
                    .filter(|x| (0.0..=100.0).contains(x))
                    .ok_or_else(|| trf("Percentile {} is not a number from 0 to 100", &[&t]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if pc.is_empty() {
            m.remove("percentiles");
        } else {
            m.insert(
                "percentiles".into(),
                Value::Array(pc.into_iter().map(crate::tol_catalog::num_val).collect()),
            );
        }
        if self.samples {
            m.insert("samples".into(), json!(true));
        } else {
            m.remove("samples");
        }
        if self.threads > 0 {
            m.insert("threads".into(), json!(self.threads));
        } else {
            m.remove("threads");
        }
        // 欄のない上書き（読んだファイルの項目ごとの上書き）に欄の分を足す
        let mut lenses = self
            .extra
            .get("lenses")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for l in &self.lenses {
            let n = l.name.trim();
            if n.is_empty() || l.preset.is_empty() {
                continue;
            }
            lenses.insert(
                n.into(),
                if l.preset == "none" {
                    Value::Null
                } else {
                    json!(l.preset)
                },
            );
        }
        if !lenses.is_empty() {
            m.insert("lenses".into(), Value::Object(lenses));
        }
        let mut groups = Map::new();
        for g in &self.groups {
            let n = g.name.trim();
            if n.is_empty() {
                continue;
            }
            let mut o = Map::new();
            for (k, v) in [
                ("decenter", &g.decenter),
                ("tilt", &g.tilt),
                ("shift", &g.shift),
            ] {
                if let Some(x) = crate::widgets::parse_num(v, &format!("{n} {k}"))? {
                    o.insert(k.into(), json!(x));
                }
            }
            groups.insert(n.into(), Value::Object(o));
        }
        if !groups.is_empty() {
            m.insert("groups".into(), Value::Object(groups));
        }
        let mut tols = self
            .extra
            .get("tols")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for t in &self.tols {
            tols.extend(t.to_json()?);
        }
        if !tols.is_empty() {
            m.insert("tols".into(), json!(tols));
        }
        // 最適化のパネルの欄から
        let comps: Vec<Value> = op["vars"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                let mut o = json!({ "param": v["name"] });
                for k in ["lo", "hi"] {
                    if let Some(x) = v.get(k) {
                        o[k] = x.clone();
                    }
                }
                o
            })
            .collect();
        if !comps.is_empty() {
            m.insert("compensators".into(), json!(comps));
        }
        for k in ["merit", "rings", "terms", "configs"] {
            if let Some(v) = op.get(k).filter(|v| !v.is_null()) {
                m.insert(k.into(), v.clone());
            }
        }
        if let Some(r) = op.get("reference") {
            m.insert("reference".into(), r.clone());
        }
        Ok(Value::Object(m))
    }

    /// 保存したファイル（spec と同じ形）から
    pub(crate) fn set_spec(&mut self, v: &Value) -> Result<(), String> {
        let m = v.as_object().ok_or(tr("Settings are not an object"))?;
        let mut n = TolPanel::default();
        // 最適化のパネルの欄（vars・merit・terms・configs）
        let mut op = json!({});
        let vars: Vec<Value> = match m.get("compensators") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(x) => vec![x.clone()],
        }
        .into_iter()
        .map(|c| match c {
            Value::String(s) => match s.split_once('=') {
                Some((a, r)) => {
                    let (lo, hi) = r.split_once(':').unwrap_or((r, ""));
                    let f = |x: &str| {
                        x.trim()
                            .parse::<f64>()
                            .ok()
                            .map_or(Value::Null, |x| json!(x))
                    };
                    json!({ "name": a.trim(), "lo": f(lo), "hi": f(hi) })
                }
                None => json!(s),
            },
            Value::Object(o) => {
                let mut x = json!({ "name": o.get("param").cloned().unwrap_or(Value::Null) });
                for k in ["lo", "hi"] {
                    if let Some(y) = o.get(k) {
                        x[k] = y.clone();
                    }
                }
                x
            }
            x => x,
        })
        .collect();
        op["vars"] = json!(vars);
        for k in ["merit", "rings", "terms", "configs"] {
            if let Some(x) = m.get(k) {
                op[k] = x.clone();
            }
        }
        if m.get("merit").is_none() {
            op["merit"] = json!("wavefront");
        }
        if let Some(x) = m.get("reference") {
            op["reference"] = x.clone();
        }
        n.opt.set_params(&op)?;
        for (k, x) in m {
            match k.as_str() {
                "trials" => n.trials = x.as_u64().ok_or(tr("trials is not an integer"))? as usize,
                "seed" => n.seed = x.as_u64().ok_or(tr("seed is not an integer"))?,
                "dist" => n.dist = x.as_str().unwrap_or("normal").into(),
                "radial" => match x.as_str() {
                    Some(d) => {
                        n.radial = true;
                        n.radial_dist = d.into();
                    }
                    None => n.radial = x.as_bool().unwrap_or(false),
                },
                "preset" => n.preset = x.as_str().unwrap_or("commercial").into(),
                "wedge_face" => n.wedge_front = x.as_str() == Some("front"),
                "catalog" => n.catalog = x.as_bool().unwrap_or(true),
                "sensitivity" => n.sensitivity = x.as_bool().unwrap_or(true),
                "comp_iters" => n.comp_iters = x.as_u64().unwrap_or(10) as usize,
                // 欄で表せない書き方（範囲の式など）はそのまま残す
                "catalog_tols" => match crate::tol_catalog::from_json(x) {
                    Ok(v) => n.catalog_tols = v,
                    Err(_) => {
                        n.extra.insert(k.clone(), x.clone());
                    }
                },
                "percentiles" if x.as_array().is_some_and(|a| a.iter().all(Value::is_number)) => {
                    n.percentiles = x
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_f64)
                        .map(fmt_num)
                        .collect::<Vec<_>>()
                        .join(", ")
                }
                "samples" => n.samples = x.as_bool().unwrap_or(false),
                "threads" => n.threads = x.as_u64().unwrap_or(0) as usize,
                "lenses" => {
                    for (name, p) in x.as_object().into_iter().flatten() {
                        match p {
                            Value::String(s) => n.lenses.push(LensRow {
                                name: name.clone(),
                                preset: s.clone(),
                            }),
                            Value::Null => n.lenses.push(LensRow {
                                name: name.clone(),
                                preset: "none".into(),
                            }),
                            // 項目ごとの上書きは欄がないので、そのまま残す
                            _ => {
                                if let Some(o) = n
                                    .extra
                                    .entry("lenses")
                                    .or_insert_with(|| json!({}))
                                    .as_object_mut()
                                {
                                    o.insert(name.clone(), p.clone());
                                }
                            }
                        }
                    }
                }
                "groups" => {
                    for (name, g) in x.as_object().into_iter().flatten() {
                        let f = |k: &str| g.get(k).map(text_of).unwrap_or_default();
                        n.groups.push(GroupRow {
                            name: name.clone(),
                            decenter: f("decenter"),
                            tilt: f("tilt"),
                            shift: f("shift"),
                        });
                    }
                }
                "tols" => {
                    for t in x.as_array().into_iter().flatten() {
                        // 欄で表せない書き方はそのまま残す
                        match TolRow::from_json(t) {
                            Some(r) => n.tols.push(r),
                            _ => {
                                if let Some(a) = n
                                    .extra
                                    .entry("tols")
                                    .or_insert_with(|| json!([]))
                                    .as_array_mut()
                                {
                                    a.push(t.clone());
                                }
                            }
                        }
                    }
                }
                // $schema はエディタの検査用（保存するときに付け直す）
                "compensators" | "merit" | "rings" | "terms" | "configs" | "reference"
                | "$schema" => {}
                _ => {
                    n.extra.insert(k.clone(), x.clone());
                }
            }
        }
        // 欄に出ていない lenses・tols の項目は extra に残し、spec で欄の分と合わせて送る
        n.result = self.result.take();
        n.comp_sel = self.comp_sel;
        n.comp_cdf = self.comp_cdf;
        *self = n;
        Ok(())
    }
}

/// パネルを描く。progress は走っている間の (終えた数, 全体の数)
pub(crate) fn ui(
    ui: &mut Ui,
    p: &mut TolPanel,
    c: &op::Cands,
    progress: Option<(usize, usize)>,
) -> Vec<Action> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical().id_salt("tol_scroll").show(ui, |ui| {
        ui.horizontal(|ui| {
            if p.running {
                if ui.button(tr("Stop")).clicked() {
                    acts.push(Action::Stop);
                }
                ui.spinner();
            } else {
                // 欄の誤りがあれば押せなくして理由を出す
                let check = p.spec();
                let r = ui
                    .add_enabled(
                        check.is_ok(),
                        egui::Button::new(RichText::new(tr("Run tolerancing")).strong()),
                    )
                    .on_disabled_hover_text(check.clone().err().unwrap_or_default());
                if let Err(e) = &check {
                    ui.label(
                        RichText::new(format!("⚠ {e}"))
                            .small()
                            .color(ui.visuals().error_fg_color),
                    );
                }
                if r.clicked() {
                    if let Ok(v) = check {
                        p.error = None;
                        acts.push(Action::Run(v));
                    }
                }
            }
            ui.add_enabled_ui(!p.running, |ui| {
                if ui.button(tr("Save settings…")).clicked() {
                    match p.spec() {
                        Ok(v) => acts.push(Action::Save(v)),
                        Err(e) => p.error = Some(e),
                    }
                }
                if ui.button(tr("Load settings…")).clicked() {
                    acts.push(Action::Load);
                }
            });
        });
        if let Some(e) = &p.error {
            ui.colored_label(ui.visuals().error_fg_color, lensbench_core::i18n::tr(e));
        }
        if let Some((k, n)) = progress {
            let f = if n > 0 { k as f32 / n as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(f).text(format!("{k} / {n}")));
        } else if let Some(r) = p.result.clone() {
            egui::CollapsingHeader::new(RichText::new(tr("Result")).strong())
                .id_salt("tol_result")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let to_json = || serde_json::to_value(&*r).unwrap_or_default();
                        if ui.button(tr("Save results…")).clicked() {
                            acts.push(Action::SaveResult(to_json()));
                        }
                        let has_comp = !r.compensators.is_empty();
                        if ui
                            .add_enabled(has_comp, egui::Button::new(tr("Compensator CSV…")))
                            .on_hover_text(tr("Merit, compensators and tilt magnitudes [arcmin] for each trial"))
                            .clicked()
                        {
                            match lensbench_design::tolerance::compensators_csv(&to_json()) {
                                Ok(t) => acts.push(Action::SaveCsv(t)),
                                Err(e) => p.error = Some(String::from(e)),
                            }
                        }
                        if r.samples.is_none() {
                            ui.label(
                                RichText::new(tr("Turn on samples under Settings to include each trial's perturbations in the results"))
                                    .small()
                                    .color(weak),
                            );
                        }
                    });
                    result_ui(ui, &r, &mut p.comp_sel, &mut p.comp_cdf, weak)
                });
        }
        ui.separator();
        ui.add_enabled_ui(!p.running, |ui| {
            section(ui, "tol_set_sec", tr("Tolerance"), |ui| tol_ui(ui, p, c, weak));
            section(ui, "tol_comp_sec", tr("Compensators"), |ui| {
                op::vars_ui(ui, &mut p.opt, c, weak);
                ui.label(
                    RichText::new(tr("For each trial, compensators are re-fitted on the merit function (focus gap, group shift, etc.)."))
                        .small()
                        .color(weak),
                );
            });
            section(ui, "tol_merit_sec", tr("Merit function"), |ui| op::merit_ui(ui, &mut p.opt, false, weak));
            section(ui, "tol_run_sec", tr("Settings"), |ui| run_ui(ui, p, weak));
        });
    });
    acts
}

fn tol_ui(ui: &mut Ui, p: &mut TolPanel, c: &op::Cands, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Lens tolerance set"));
        for (k, l) in PRESETS.iter().take(4) {
            ui.selectable_value(&mut p.preset, k.to_string(), tr(l));
        }
    });
    ui.checkbox(
        &mut p.catalog,
        tr("Catalog lenses use catalog tolerances (catalog_tols in the settings; otherwise center thickness ±0.1 mm, focal length ±2%, centration 5′)"),
    );
    ui.label(
        RichText::new(tr(
            "A set covers radius, center thickness, wedge, index, dispersion, lens decenter and tilt, and the air gap after the lens (commercial: ±0.2%, ±0.15 mm, 3′, ±0.001, ±1%, ±0.1 mm, 3′, ±0.1 mm).",
        ))
        .small()
        .color(weak),
    );
    if p.catalog {
        egui::CollapsingHeader::new(tr("Catalog lens tolerances by vendor"))
            .id_salt("tol_catalog_tols")
            .default_open(!p.catalog_tols.is_empty())
            .show(ui, |ui| {
                if p.extra.contains_key("catalog_tols") {
                    ui.label(
                        RichText::new(tr("The loaded catalog_tols cannot be shown in these fields, so it is used as is (adding here replaces it)."))
                            .small()
                            .color(weak),
                    );
                }
                let before = p.catalog_tols.len();
                crate::tol_catalog::ui(ui, &mut p.catalog_tols, &PRESETS[..4], weak);
                if p.catalog_tols.len() != before {
                    p.extra.remove("catalog_tols");
                }
            });
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Distribution"));
        for (k, l) in DISTS {
            ui.selectable_value(&mut p.dist, k.to_string(), tr(l));
        }
    });
    // チェックの文が長いので、選択は次の行に字下げして置く（折り返しの中の入れ子で縦 1 文字に崩れない）
    ui.checkbox(
        &mut p.radial,
        tr("Pair x/y of decenters, tilts and wedges within a circle (magnitude at most the tolerance)"),
    );
    ui.horizontal_wrapped(|ui| {
        ui.add_space(24.0);
        let on = p.radial;
        if ui
            .add_enabled(
                on,
                egui::SelectableLabel::new(p.radial_dist == "uniform", tr("Uniform")),
            )
            .clicked()
        {
            p.radial_dist = "uniform".to_string();
        }
        if ui
            .add_enabled(
                on,
                egui::SelectableLabel::new(p.radial_dist == "normal", tr("Normal (radius 2σ)")),
            )
            .on_hover_text(tr(
                "Isotropic 2D normal truncated at the circle; 86% fall within 2σ",
            ))
            .clicked()
        {
            p.radial_dist = "normal".to_string();
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Wedge face"));
        ui.selectable_value(&mut p.wedge_front, false, tr("Back face"));
        ui.selectable_value(&mut p.wedge_front, true, tr("Front face"));
    })
    .response
    .on_hover_text(tr(
        "Front/back refer to the lens's own orientation (as in the catalog or .zmx). The same face is used for reversed lenses and lenses traversed twice via a mirror",
    ));
    // 玉ごとの組
    ui.label(RichText::new(tr("Set per lens")).strong());
    if !p.lenses.is_empty() {
        let mut ls = LensSheet {
            lenses: &mut p.lenses,
            units: pp::names(&c.sys, Set::Units),
        };
        sheet::show(ui, "tol_lenses", &mut p.lens_sheet, &mut ls);
    }
    ui.menu_button(tr("Add lens"), |ui| {
        for u in &c.units {
            if ui.button(u).clicked() {
                p.lenses.push(LensRow {
                    name: u.clone(),
                    preset: "none".into(),
                });
                ui.close_menu();
            }
        }
    });
    // 群・ブロック
    ui.label(
        RichText::new(tr(
            "Group/block tolerances (decenter mm, tilt deg, shift mm)",
        ))
        .strong(),
    );
    if !p.groups.is_empty() {
        let mut gs = GroupSheet {
            groups: &mut p.groups,
            units: pp::names(&c.sys, Set::Units),
        };
        sheet::show(ui, "tol_groups", &mut p.group_sheet, &mut gs);
    }
    ui.menu_button(tr("Add group"), |ui| {
        for u in &c.units {
            if ui.button(u).clicked() {
                p.groups.push(GroupRow {
                    name: u.clone(),
                    ..Default::default()
                });
                ui.close_menu();
            }
        }
    });
    // 個別の公差
    ui.label(RichText::new(tr("Individual tolerances (per surface)")).strong());
    if !p.tols.is_empty() {
        let mut ts = TolSheet {
            tols: &mut p.tols,
            names: pp::names(&c.sys, Set::Any),
        };
        sheet::show(ui, "tol_tols", &mut p.tol_sheet, &mut ts);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Add a tolerance")).clicked() {
            p.tols.push(TolRow::default());
        }
        if ui
            .button(tr("Add wavelength shift"))
            .on_hover_text(tr(
                "Source wavelength shift (± µm, all wavelengths shifted by the same amount; wave1 etc. for a single wavelength)",
            ))
            .clicked()
        {
            p.tols.push(TolRow {
                param: "wave".into(),
                pm: "0.01".into(),
                ..Default::default()
            });
        }
    });
    if !p.extra.is_empty() {
        ui.label(
            RichText::new(trf(
                "Settings without fields (used as is): {}",
                &[&p.extra.keys().cloned().collect::<Vec<_>>().join(", ")],
            ))
            .small()
            .color(weak),
        );
    }
}

fn run_ui(ui: &mut Ui, p: &mut TolPanel, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Trials"));
        ui.add(egui::DragValue::new(&mut p.trials).range(0..=1_000_000));
        ui.label(tr("Random seed"));
        ui.add(egui::DragValue::new(&mut p.seed));
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Compensator iterations"));
        ui.add(egui::DragValue::new(&mut p.comp_iters).range(1..=200));
        ui.checkbox(
            &mut p.sensitivity,
            tr("Also compute sensitivities (each tolerance at its limits)"),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Percentiles"));
        small_edit(ui, &mut p.percentiles, 120.0, "50, 80, 90, 95, 98");
        ui.label(tr("Threads"));
        ui.add(
            egui::DragValue::new(&mut p.threads)
                .range(0..=256)
                .custom_formatter(|v, _| {
                    if v < 0.5 {
                        tr("Auto").to_string()
                    } else {
                        format!("{v:.0}")
                    }
                }),
        );
    });
    ui.checkbox(
        &mut p.samples,
        tr("Include each trial's deltas and evaluation in the result (samples)"),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Config"));
        small_edit(ui, &mut p.opt.configs, 160.0, tr("Open system only"));
    });
    ui.label(
        RichText::new(tr("The same seed gives the same result (regardless of thread count). With several configurations, the same deviations are applied to all of them."))
            .small()
            .color(weak),
    );
}

fn f(v: Option<f64>) -> String {
    v.map_or("—".into(), fmt_num)
}

/// 補償子の分布の 1 つ: 名前・公称・統計・試行の値（追跡できた試行だけ）
struct CompDist<'a> {
    name: String,
    nominal: Option<f64>,
    stats: &'a StatsInfo,
    values: Vec<f64>,
}

/// 結果の補償子と傾きの大きさ（分）を並べる
fn comp_dists(r: &ToleranceReport) -> Vec<CompDist<'_>> {
    let vals = |v: &[Option<f64>]| -> Vec<f64> {
        v.iter()
            .flatten()
            .copied()
            .filter(|x| x.is_finite())
            .collect()
    };
    let comps = r.compensators.iter().map(|c| CompDist {
        name: c.name.clone(),
        nominal: c.nominal,
        stats: &c.stats,
        values: vals(&c.values),
    });
    let tilts = r.tilt_magnitudes.iter().map(|t| CompDist {
        name: format!("{} [′]", t.name),
        nominal: t.nominal,
        stats: &t.stats,
        values: vals(&t.values),
    });
    comps.chain(tilts).collect()
}

/// ヒストグラムの区間: (下端, 幅, 数)。区間の数は √n（5〜50）
fn histogram(v: &[f64]) -> Vec<(f64, f64, usize)> {
    let (lo, hi) = v
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
            (a.min(x), b.max(x))
        });
    if v.is_empty() || !lo.is_finite() {
        return Vec::new();
    }
    let nb = ((v.len() as f64).sqrt().round() as usize).clamp(5, 50);
    let w = if hi > lo {
        (hi - lo) / nb as f64
    } else {
        lo.abs().max(1e-12) * 1e-3
    };
    let mut cnt = vec![0usize; nb];
    for &x in v {
        let i = (((x - lo) / w) as usize).min(nb - 1);
        cnt[i] += 1;
    }
    cnt.into_iter()
        .enumerate()
        .map(|(i, c)| (lo + i as f64 * w, w, c))
        .collect()
}

/// 補償子の分布の表と図
fn comps_ui(
    ui: &mut Ui,
    r: &ToleranceReport,
    sel: &mut usize,
    cdf: &mut bool,
    weak: egui::Color32,
) {
    let ds = comp_dists(r);
    if ds.is_empty() {
        return;
    }
    ui.label(RichText::new(tr("Compensators")).strong());
    let pcts: Vec<String> = ds[0]
        .stats
        .percentiles
        .iter()
        .map(|x| format!("{}%", fmt_num(x.0)))
        .collect();
    egui::ScrollArea::horizontal()
        .id_salt("tol_comps_h")
        .show(ui, |ui| {
            egui::Grid::new("tol_comps").striped(true).show(ui, |ui| {
                for h in [
                    tr("Name"),
                    tr("Nominal"),
                    tr("Mean"),
                    tr("Std. dev."),
                    tr("Min"),
                    tr("Max"),
                ] {
                    ui.label(RichText::new(h).small().color(weak));
                }
                for p in &pcts {
                    ui.label(RichText::new(p).small().color(weak));
                }
                ui.end_row();
                for d in &ds {
                    ui.label(&d.name);
                    ui.label(f(d.nominal));
                    let st = d.stats;
                    for x in [st.mean, st.std, st.min, st.max] {
                        ui.label(f(x));
                    }
                    for x in &st.percentiles {
                        ui.label(f(x.1));
                    }
                    ui.end_row();
                }
            });
            crate::widgets::scroll_gap(ui);
        });
    if !r.tilt_magnitudes.is_empty() {
        ui.label(
            RichText::new(tr(
                "\".tilt [′]\" is the magnitude √(tx² + ty²) of the tilt pair tx, ty of the same lens, group or surface (arcmin).",
            ))
            .small()
            .color(weak),
        );
    }
    if ds.iter().all(|d| d.values.is_empty()) {
        return;
    }
    *sel = (*sel).min(ds.len() - 1);
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("tol_comp_sel")
            .selected_text(ds[*sel].name.as_str())
            .show_ui(ui, |ui| {
                for (i, d) in ds.iter().enumerate() {
                    ui.selectable_value(sel, i, d.name.as_str());
                }
            });
        ui.selectable_value(cdf, false, tr("Histogram"));
        ui.selectable_value(cdf, true, tr("Cumulative distribution"));
    });
    let d = &ds[*sel];
    let nom = d.nominal;
    let plot = Plot::new("tol_comp_plot")
        .height(180.0)
        .x_axis_label(d.name.clone())
        .allow_scroll(false);
    if *cdf {
        let mut v = d.values.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        let n = v.len() as f64;
        let pts: PlotPoints = v
            .iter()
            .enumerate()
            .map(|(i, &x)| [x, 100.0 * (i + 1) as f64 / n])
            .collect();
        plot.y_axis_label(tr("Cumulative [%]")).show(ui, |pu| {
            pu.line(Line::new(pts).name(tr("Cumulative distribution")));
            if let Some(x) = nom {
                pu.vline(egui_plot::VLine::new(x).name(tr("Nominal")));
            }
        });
    } else {
        let bars: Vec<egui_plot::Bar> = histogram(&d.values)
            .into_iter()
            .map(|(x0, w, c)| egui_plot::Bar::new(x0 + w / 2.0, c as f64).width(w))
            .collect();
        plot.y_axis_label(tr("Trials")).show(ui, |pu| {
            pu.bar_chart(egui_plot::BarChart::new(bars).name(tr("Histogram")));
            if let Some(x) = nom {
                pu.vline(egui_plot::VLine::new(x).name(tr("Nominal")));
            }
        });
    }
}

fn result_ui(
    ui: &mut Ui,
    r: &ToleranceReport,
    sel: &mut usize,
    as_cdf: &mut bool,
    weak: egui::Color32,
) {
    ui.label(trf(
        "{} trials ({} failed to trace), seed {}",
        &[&r.trials, &r.failed, &r.seed],
    ));
    if r.cancelled {
        ui.label(RichText::new(tr("Stopped (finished trials only)")).color(weak));
    }
    for e in &r.errors {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    // 統計の表（評価関数と項）
    let nom_merit = r
        .nominal
        .as_ref()
        .map(|e| e.merit)
        .filter(|x| x.is_finite());
    let mut rows: Vec<(String, Option<f64>, &StatsInfo)> =
        vec![(tr("Merit function").into(), nom_merit, &r.merit)];
    for t in &r.terms {
        // 項の単位を名前に添える（spot・ray は mm、wavefront・opd は波）
        let label = t.label.clone();
        let unit = term_unit(&label);
        rows.push((
            if unit.is_empty() {
                label
            } else {
                format!("{label} [{unit}]")
            },
            t.nominal.filter(|x| x.is_finite()),
            &t.stats,
        ));
    }
    let pcts: Vec<String> = r
        .merit
        .percentiles
        .iter()
        .map(|x| format!("{}%", fmt_num(x.0)))
        .collect();
    egui::ScrollArea::horizontal()
        .id_salt("tol_stats_h")
        .show(ui, |ui| {
            egui::Grid::new("tol_stats").striped(true).show(ui, |ui| {
                for h in ["", tr("Nominal"), tr("Mean"), tr("Std. dev."), tr("Max")] {
                    ui.label(RichText::new(h).small().color(weak));
                }
                for p in &pcts {
                    ui.label(RichText::new(p).small().color(weak));
                }
                ui.end_row();
                for (name, nom, st) in &rows {
                    ui.label(name);
                    ui.label(f(*nom));
                    ui.label(f(st.mean));
                    ui.label(f(st.std));
                    ui.label(f(st.max));
                    for x in &st.percentiles {
                        ui.label(f(x.1));
                    }
                    ui.end_row();
                }
            });
            crate::widgets::scroll_gap(ui);
        });
    if let Some(e) = r.rss_estimate {
        ui.label(
            RichText::new(trf(
                "Estimate from sensitivities (nominal + √Σ change²): {}",
                &[&fmt_num(e)],
            ))
            .small()
            .color(weak),
        );
    }
    // 累積分布
    let cdf = &r.cdf;
    if !cdf.is_empty() {
        let n = cdf.len() as f64;
        let pts: PlotPoints = cdf
            .iter()
            .enumerate()
            .map(|(i, &x)| [x, 100.0 * (i + 1) as f64 / n])
            .collect();
        Plot::new("tol_cdf")
            .height(180.0)
            .x_axis_label(tr("Merit function"))
            .y_axis_label(tr("Cumulative [%]"))
            .allow_scroll(false)
            .show(ui, |pu| {
                pu.line(Line::new(pts).name(tr("Cumulative distribution")));
                if let Some(x) = nom_merit {
                    pu.vline(egui_plot::VLine::new(x).name(tr("Nominal")));
                }
            });
    }
    comps_ui(ui, r, sel, as_cdf, weak);
    // 感度（悪い順）
    let sens = &r.sensitivity;
    if !sens.is_empty() {
        egui::CollapsingHeader::new(
            RichText::new(tr("Largest contributors (sensitivity, worst first)")).strong(),
        )
        .id_salt("tol_sens")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("tol_sens_g").striped(true).show(ui, |ui| {
                for h in [
                    tr("Tolerance"),
                    tr("Min"),
                    tr("Max"),
                    tr("Change (min)"),
                    tr("Change (max)"),
                ] {
                    ui.label(RichText::new(h).small().color(weak));
                }
                ui.end_row();
                for s in sens.iter().take(20) {
                    ui.label(item_name(&s.label));
                    ui.label(f(Some(s.lo)));
                    ui.label(f(Some(s.hi)));
                    ui.label(f(s.change_lo));
                    ui.label(f(s.change_hi));
                    ui.end_row();
                }
            });
            if sens.len() > 20 {
                ui.label(
                    RichText::new(trf("{} more items", &[&(sens.len() - 20)]))
                        .small()
                        .color(weak),
                );
            }
        });
    }
    let corr = &r.correlation;
    if corr.iter().any(|c| c.r.is_some()) {
        egui::CollapsingHeader::new(RichText::new(tr("Correlation with trials")).strong())
            .id_salt("tol_corr")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("tol_corr_g").striped(true).show(ui, |ui| {
                    for h in [
                        tr("Tolerance"),
                        tr("Correlation with deviation"),
                        tr("Correlation with |deviation|"),
                    ] {
                        ui.label(RichText::new(h).small().color(weak));
                    }
                    ui.end_row();
                    for c in corr.iter().take(15) {
                        ui.label(item_name(&c.label));
                        ui.label(f(c.r));
                        ui.label(f(c.r_abs));
                        ui.end_row();
                    }
                });
            });
    }
}

/// 公差の項目名（「L1 decenter x」など英語が元）を表示の言語で
fn item_name(label: &str) -> String {
    lensbench_core::i18n::tr_item(label)
}

/// 公差の項の名前（"spot #1"・"wavefront #2 (config 1)" など）から値の単位
fn term_unit(label: &str) -> &'static str {
    // ray は量（x・y の mm か l・m・n の方向余弦）で単位が違い、名前からは決まらないので添えない
    match label.split([' ', '#']).next().unwrap_or_default() {
        "spot" | "centroid" => "mm",
        "wavefront" | "opd" | "zernike" => tr("waves"),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 個別の公差の数は丸めずに欄に出す（表を通して保存しても細かい値が変わらない）
    #[test]
    fn tolerance_numbers_are_not_rounded() {
        let t = json!({"param": "3.t", "lo": -0.0123456789, "hi": 1.000000001, "pm": 0.1,
            "scale": [1.23456789e-7, 2.0]});
        let r = TolRow::from_json(&t).unwrap();
        assert_eq!(r.lo, "-0.0123456789");
        assert_eq!(r.hi, "1.000000001");
        assert_eq!(r.pm, "0.1");
        assert_eq!(r.scale, "0.000000123456789, 2");
    }

    #[test]
    fn term_units() {
        assert_eq!(term_unit("spot #1"), "mm");
        assert_eq!(term_unit("wavefront #2 (config 1)"), tr("waves"));
        assert_eq!(term_unit("mtf #3"), "");
        assert_eq!(term_unit("ray #4"), "");
    }

    #[test]
    fn spec_round_trip() {
        let mut p = TolPanel {
            trials: 50,
            preset: "precision".into(),
            ..Default::default()
        };
        p.lenses.push(LensRow {
            name: "L1".into(),
            preset: "none".into(),
        });
        p.lenses.push(LensRow {
            name: "L2".into(),
            preset: "high".into(),
        });
        p.groups.push(GroupRow {
            name: "G1".into(),
            decenter: "0.05".into(),
            ..Default::default()
        });
        p.tols.push(TolRow {
            param: "3.t".into(),
            pm: "0.02".into(),
            dist: "end".into(),
            ..Default::default()
        });
        p.radial = true;
        p.opt.vars.push(op::VarRow {
            name: "9.t".into(),
            lo: "-1".into(),
            ..Default::default()
        });
        let v = p.spec().unwrap();
        assert_eq!(v["lenses"], json!({"L1": null, "L2": "high"}));
        assert_eq!(v["groups"], json!({"G1": {"decenter": 0.05}}));
        assert_eq!(
            v["tols"],
            json!([{"param": "3.t", "pm": 0.02, "dist": "end"}])
        );
        assert_eq!(v["compensators"], json!([{"param": "9.t", "lo": -1.0}]));
        assert_eq!(v["merit"], json!("wavefront"));
        assert_eq!(v["radial"], json!(true));
        let mut q = TolPanel::default();
        q.set_spec(&v).unwrap();
        assert_eq!(q.spec().unwrap(), v);
        // 保存するファイル（先頭に $schema）を読み戻しても同じ。$schema は計算の設定に入れない
        let saved = lensbench_project::schema::pretty_with_schema(&v, "tolerance");
        let file: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            file["$schema"],
            lensbench_project::schema::url("tolerance").as_str()
        );
        let mut q = TolPanel::default();
        q.set_spec(&file).unwrap();
        assert_eq!(q.spec().unwrap(), v);
        // 円の中の正規分布
        let mut r = v.clone();
        r["radial"] = json!("normal:3");
        let mut q = TolPanel::default();
        q.set_spec(&r).unwrap();
        assert!(q.radial);
        assert_eq!(q.spec().unwrap(), r);
        // 欄のない項目は残す
        let mut w = v.clone();
        w["percentiles"] = json!([50, 99]);
        w["tols"]
            .as_array_mut()
            .unwrap()
            .push(json!({"params": ["1.r", "2.r"], "pm": 0.01, "relative": true}));
        q.set_spec(&w).unwrap();
        let back = q.spec().unwrap();
        assert_eq!(back["percentiles"], json!([50, 99]));
        // 複数のパラメータの公差も欄に出す（並びの順のまま）
        assert_eq!(back["tols"], w["tols"]);
        assert!(!q.extra.contains_key("tols"));
        assert_eq!(q.percentiles, "50, 99");
        // catalog_tols・samples・threads は欄に。欄で書けない catalog_tols はそのまま残す
        let mut w = v.clone();
        w["catalog_tols"] = json!({"Edmund": {"thickness": 0.05}, "*": "precision"});
        w["samples"] = json!(true);
        w["threads"] = json!(4);
        q.set_spec(&w).unwrap();
        assert_eq!(q.catalog_tols.len(), 2);
        assert!(q.samples);
        assert_eq!(q.threads, 4);
        assert_eq!(q.spec().unwrap(), w);
        w["catalog_tols"] = json!({"A": {"rules": [{"dia_lt": 5, "dia_le": 6}]}});
        q.set_spec(&w).unwrap();
        assert!(q.catalog_tols.is_empty());
        assert_eq!(q.spec().unwrap()["catalog_tols"], w["catalog_tols"]);
    }

    #[test]
    fn tol_rows_cover_items() {
        // 個別の公差の項目（lo・hi・params・scale・label・分布の書き方）は欄で読み書きして元に戻る
        let items = [
            json!({"param": "3.t", "lo": -0.01, "hi": 0.03}),
            json!({"params": ["1.r", "2.r"], "pm": 0.01, "relative": true, "scale": [1.0, -1.0], "label": "R pair"}),
            json!({"params": ["5.t"], "pm": 0.02, "dist": "normal:3"}),
            json!({"param": "4.tx", "pm": 0.1, "dist": "disc_x"}),
        ];
        for t in &items {
            let r = TolRow::from_json(t).unwrap();
            assert_eq!(r.to_json().unwrap().as_ref(), Some(t), "{t}");
        }
        // 欄で表せない書き方は None（そのまま残す）
        assert!(TolRow::from_json(&json!({"param": "3.t", "pm": "x"})).is_none());
        assert!(TolRow::from_json(&json!({"param": "3.t", "pm": 1, "other": 1})).is_none());
        // ± と範囲の両方、範囲の片方だけは誤り
        let mut r = TolRow::from_json(&items[0]).unwrap();
        r.pm = "0.1".into();
        assert!(r.to_json().is_err());
        r.pm.clear();
        r.hi.clear();
        assert!(r.to_json().is_err());
        assert_eq!(TolRow::default().to_json(), Ok(None));
    }

    #[test]
    fn comp_dists_and_histogram() {
        use lensbench_design::tol_report::{CompStats, TiltStats};
        let r = ToleranceReport {
            compensators: vec![CompStats {
                name: "M.tx".into(),
                values: vec![Some(0.1), None, Some(-0.2)],
                ..Default::default()
            }],
            tilt_magnitudes: vec![TiltStats {
                name: "M.tilt".into(),
                values: vec![Some(6.0), None, Some(12.0)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let d = comp_dists(&r);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].values, vec![0.1, -0.2]);
        assert_eq!(d[1].name, "M.tilt [′]");
        let v: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let h = histogram(&v);
        assert_eq!(h.len(), 10);
        assert_eq!(h.iter().map(|x| x.2).sum::<usize>(), 100);
        assert_eq!(h[9].2, 10);
        // 値がすべて同じでも区間を作る
        let h = histogram(&[2.0; 4]);
        assert_eq!(h.iter().map(|x| x.2).sum::<usize>(), 4);
        assert!(histogram(&[]).is_empty());
    }
}
