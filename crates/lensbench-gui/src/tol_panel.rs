//! モンテカルロ公差のパネル: 公差の組・玉や群ごとの上書き・個別の公差・補償子・評価関数を並べ、serve の
//! `tolerance` で回して、評価関数の統計・累積分布・寄与の大きい項目（感度）・補償子の動きを出す。
//!
//! 設定は `lensbench tolerance --spec` と同じ JSON（[`TolPanel::spec`]）で、そのままファイルに保存・読込できる。
//! 補償子・評価関数・構成の欄は最適化のパネル（[`crate::optim_panel`]）と同じものを使う。
//! 走っている間は [`lensbench_cli::serve::tolerance_progress`] で進み具合を読み、止めるのは
//! [`lensbench_cli::serve::tolerance_cancel`]。

use egui::{RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::{self as op, section, small_edit, text_of, OptPanel};
use crate::theme;

/// 公差の組の選択肢（"" は既定 = 全体の組）
const PRESETS: [(&str, &str); 5] = [
    ("commercial", "一般"),
    ("precision", "精密"),
    ("high", "高精度"),
    ("none", "なし"),
    ("", "既定"),
];

const DISTS: [(&str, &str); 3] = [
    ("normal", "正規（±2σ）"),
    ("uniform", "一様"),
    ("end", "端（±の端）"),
];

fn preset_label(k: &str) -> &'static str {
    PRESETS.iter().find(|x| x.0 == k).map_or("?", |x| tr(x.1))
}

fn dist_label(k: &str) -> &'static str {
    DISTS.iter().find(|x| x.0 == k).map_or("?", |x| tr(x.1))
}

/// 玉ごとの上書き（組を選ぶ）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensRow {
    pub name: String,
    pub preset: String,
}

/// 群・ブロックの公差（空は作らない）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupRow {
    pub name: String,
    pub decenter: String,
    pub tilt: String,
    pub shift: String,
}

/// 個別の公差
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TolRow {
    pub param: String,
    pub pm: String,
    /// "" は既定の分布
    pub dist: String,
    pub relative: bool,
}

/// パネルの操作（app が serve に送る）
pub enum Action {
    Run(Value),
    Stop,
    Save(Value),
    Load,
    /// 結果の JSON を保存する
    SaveResult(Value),
    /// 試行ごとの補償子の CSV を保存する
    SaveCsv(String),
}

pub struct TolPanel {
    pub trials: usize,
    pub seed: u64,
    pub dist: String,
    /// 偏心・傾き・くさびの x・y を組にして円の中で引く
    pub radial: bool,
    /// 円の中の分布（uniform・normal・normal:σの数）
    pub radial_dist: String,
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
    pub result: Option<Value>,
    pub error: Option<String>,
    /// 結果の補償子の分布の図: 選んだ補償子（補償子、続いて傾きの大きさ）と、累積分布で出すか（false はヒストグラム）
    pub comp_sel: usize,
    pub comp_cdf: bool,
    /// 設定の JSON にあってパネルに欄のない項目（読んだまま保存・実行に使う）
    pub extra: Map<String, Value>,
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
        }
    }
}

fn num(s: &str, what: &str) -> Result<Option<f64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<f64>()
        .map(Some)
        .map_err(|_| trf("{} の値 {} が数でない", &[&what, &t]))
}

impl TolPanel {
    /// serve の tolerance の params（保存するファイルと同じ。configs も入る）
    pub fn spec(&self) -> Result<Value, String> {
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
                    .ok_or_else(|| trf("百分位 {} が 0〜100 の数でない", &[&t]))
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
                if let Some(x) = num(v, &format!("{n} {k}"))? {
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
            let p = t.param.trim();
            if p.is_empty() {
                continue;
            }
            let pm = num(&t.pm, p)?.ok_or_else(|| trf("{} の ± がない", &[&p]))?;
            let mut o = json!({ "param": p, "pm": pm });
            if !t.dist.is_empty() {
                o["dist"] = json!(t.dist);
            }
            if t.relative {
                o["relative"] = json!(true);
            }
            tols.push(o);
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
            m.insert("ref".into(), r.clone());
        }
        Ok(Value::Object(m))
    }

    /// 保存したファイル（spec と同じ形）から
    pub fn set_spec(&mut self, v: &Value) -> Result<(), String> {
        let m = v.as_object().ok_or(tr("設定がオブジェクトでない"))?;
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
        if let Some(x) = m.get("ref") {
            op["reference"] = x.clone();
        }
        n.opt.set_params(&op)?;
        for (k, x) in m {
            match k.as_str() {
                "trials" => n.trials = x.as_u64().ok_or(tr("trials が整数でない"))? as usize,
                "seed" => n.seed = x.as_u64().ok_or(tr("seed が整数でない"))?,
                "dist" => n.dist = x.as_str().unwrap_or("normal").into(),
                "radial" => match x.as_str() {
                    Some(d) => {
                        n.radial = true;
                        n.radial_dist = d.into();
                    }
                    None => n.radial = x.as_bool().unwrap_or(false),
                },
                "preset" => n.preset = x.as_str().unwrap_or("commercial").into(),
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
                        // 1 つのパラメータの ± だけ欄に出す。ほかはそのまま残す
                        match (t["param"].as_str(), t["pm"].as_f64()) {
                            (Some(p), Some(pm)) if t.get("params").is_none() => {
                                n.tols.push(TolRow {
                                    param: p.into(),
                                    pm: fmt_num(pm),
                                    dist: t["dist"].as_str().unwrap_or("").into(),
                                    relative: t["relative"].as_bool().unwrap_or(false),
                                })
                            }
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
                "compensators" | "merit" | "rings" | "terms" | "configs" | "ref" => {}
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
pub fn ui(
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
                if ui.button(tr("止める")).clicked() {
                    acts.push(Action::Stop);
                }
                ui.spinner();
            } else if ui
                .button(RichText::new(tr("公差を回す")).strong())
                .clicked()
            {
                match p.spec() {
                    Ok(v) => {
                        p.error = None;
                        acts.push(Action::Run(v));
                    }
                    Err(e) => p.error = Some(e),
                }
            }
            ui.add_enabled_ui(!p.running, |ui| {
                if ui.button(tr("設定を保存…")).clicked() {
                    match p.spec() {
                        Ok(v) => acts.push(Action::Save(v)),
                        Err(e) => p.error = Some(e),
                    }
                }
                if ui.button(tr("設定を読む…")).clicked() {
                    acts.push(Action::Load);
                }
            });
        });
        if let Some(e) = &p.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some((k, n)) = progress {
            let f = if n > 0 { k as f32 / n as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(f).text(format!("{k} / {n}")));
        } else if let Some(r) = p.result.clone() {
            egui::CollapsingHeader::new(RichText::new(tr("結果")).strong())
                .id_salt("tol_result")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(tr("結果を保存…")).clicked() {
                            acts.push(Action::SaveResult(r.clone()));
                        }
                        let has_comp = r["compensators"].as_array().is_some_and(|a| !a.is_empty());
                        if ui
                            .add_enabled(has_comp, egui::Button::new(tr("補償子の CSV…")))
                            .on_hover_text(tr("試行ごとの評価関数・補償子・傾きの大きさ [分]"))
                            .clicked()
                        {
                            match lensbench_project::tolerance::compensators_csv(&r) {
                                Ok(t) => acts.push(Action::SaveCsv(t)),
                                Err(e) => p.error = Some(e),
                            }
                        }
                        if r.get("samples").is_none() {
                            ui.label(
                                RichText::new(tr("試行ごとのずれは「設定」の samples を入れて回すと結果に入ります"))
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
            section(ui, "tol_set_sec", tr("公差"), |ui| tol_ui(ui, p, c, weak));
            section(ui, "tol_comp_sec", tr("補償子"), |ui| {
                op::vars_ui(ui, &mut p.opt, c, weak);
                ui.label(
                    RichText::new(tr("試行ごとに、補償子を評価関数で合わせ直します（ピントの間隔・群の移動など）。"))
                        .small()
                        .color(weak),
                );
            });
            section(ui, "tol_merit_sec", tr("評価関数"), |ui| op::merit_ui(ui, &mut p.opt, weak));
            section(ui, "tol_run_sec", tr("設定"), |ui| run_ui(ui, p, weak));
        });
    });
    acts
}

fn tol_ui(ui: &mut Ui, p: &mut TolPanel, c: &op::Cands, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("玉の公差の組"));
        for (k, l) in PRESETS.iter().take(4) {
            ui.selectable_value(&mut p.preset, k.to_string(), tr(l));
        }
    });
    ui.checkbox(
        &mut p.catalog,
        tr("カタログの玉はカタログの玉の公差（設定の catalog_tols、なければ中心厚 ±0.1 mm・焦点距離 ±2%・芯取り 5′）"),
    );
    ui.label(
        RichText::new(tr(
            "組は曲率半径・中心厚・くさび・屈折率・分散・玉の偏心と傾き・玉の後の間隔（一般: ±0.2%・±0.15 mm・3′・±0.001・±1%・±0.1 mm・3′・±0.1 mm）。",
        ))
        .small()
        .color(weak),
    );
    if p.catalog {
        egui::CollapsingHeader::new(tr("メーカーごとのカタログの玉の公差"))
            .id_salt("tol_catalog_tols")
            .default_open(!p.catalog_tols.is_empty())
            .show(ui, |ui| {
                if p.extra.contains_key("catalog_tols") {
                    ui.label(
                        RichText::new(tr("読んだ catalog_tols は欄で表せないので、そのまま使います（ここで足すと置き換えます）。"))
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
    ui.horizontal(|ui| {
        ui.label(tr("分布"));
        for (k, l) in DISTS {
            ui.selectable_value(&mut p.dist, k.to_string(), tr(l));
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(
            &mut p.radial,
            tr("偏心・傾き・くさびは x・y を組にして円の中で（大きさが公差以下）"),
        );
        ui.add_enabled_ui(p.radial, |ui| {
            ui.selectable_value(&mut p.radial_dist, "uniform".to_string(), tr("一様"));
            ui.selectable_value(
                &mut p.radial_dist,
                "normal".to_string(),
                tr("正規（半径 2σ）"),
            )
            .on_hover_text(tr(
                "等方の 2 次元正規分布を円で切ったもの。半径 2σ の中は 86%",
            ));
        });
    });
    // 玉ごとの組
    ui.label(RichText::new(tr("玉ごとの組")).strong());
    let mut del = None;
    for (i, l) in p.lenses.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            small_edit(ui, &mut l.name, 90.0, "lens1");
            egui::ComboBox::from_id_salt(("tol_lens_preset", i))
                .selected_text(preset_label(&l.preset))
                .width(80.0)
                .show_ui(ui, |ui| {
                    for (k, lb) in PRESETS {
                        ui.selectable_value(&mut l.preset, k.to_string(), tr(lb));
                    }
                });
            if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                del = Some(i);
            }
        });
    }
    if let Some(i) = del {
        p.lenses.remove(i);
    }
    ui.menu_button(tr("玉を足す"), |ui| {
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
    ui.label(RichText::new(tr("群・ブロックの公差（偏心 mm・傾き deg・移動 mm）")).strong());
    let mut del = None;
    egui::Grid::new("tol_groups")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            for (i, g) in p.groups.iter_mut().enumerate() {
                small_edit(ui, &mut g.name, 70.0, "G1");
                small_edit(ui, &mut g.decenter, 55.0, tr("偏心"));
                small_edit(ui, &mut g.tilt, 55.0, tr("傾き"));
                small_edit(ui, &mut g.shift, 55.0, tr("移動"));
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.groups.remove(i);
    }
    ui.menu_button(tr("群を足す"), |ui| {
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
    ui.label(RichText::new(tr("個別の公差（面ごと）")).strong());
    let mut del = None;
    egui::Grid::new("tol_tols")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            for (i, t) in p.tols.iter_mut().enumerate() {
                small_edit(ui, &mut t.param, 80.0, "3.t");
                small_edit(ui, &mut t.pm, 60.0, "±");
                egui::ComboBox::from_id_salt(("tol_dist", i))
                    .selected_text(if t.dist.is_empty() {
                        tr("既定")
                    } else {
                        dist_label(&t.dist)
                    })
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut t.dist, String::new(), tr("既定"));
                        for (k, l) in DISTS {
                            ui.selectable_value(&mut t.dist, k.to_string(), tr(l));
                        }
                    });
                ui.checkbox(&mut t.relative, tr("比"))
                    .on_hover_text(tr("± を今の値に対する比にする（0.01 = ±1%）"));
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.tols.remove(i);
    }
    if ui.button(tr("個別の公差を足す")).clicked() {
        p.tols.push(TolRow::default());
    }
    if !p.extra.is_empty() {
        ui.label(
            RichText::new(trf(
                "欄のない設定（そのまま使う）: {}",
                &[&p.extra.keys().cloned().collect::<Vec<_>>().join(", ")],
            ))
            .small()
            .color(weak),
        );
    }
}

fn run_ui(ui: &mut Ui, p: &mut TolPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("試行の数"));
        ui.add(egui::DragValue::new(&mut p.trials).range(0..=1_000_000));
        ui.label(tr("乱数の種"));
        ui.add(egui::DragValue::new(&mut p.seed));
    });
    ui.horizontal(|ui| {
        ui.label(tr("補償子の反復"));
        ui.add(egui::DragValue::new(&mut p.comp_iters).range(1..=200));
        ui.checkbox(&mut p.sensitivity, tr("感度（公差ごとに ± の端）も求める"));
    });
    ui.horizontal(|ui| {
        ui.label(tr("百分位"));
        small_edit(ui, &mut p.percentiles, 120.0, "50, 80, 90, 95, 98");
        ui.label(tr("スレッド数"));
        ui.add(
            egui::DragValue::new(&mut p.threads)
                .range(0..=256)
                .custom_formatter(|v, _| {
                    if v < 0.5 {
                        tr("自動").to_string()
                    } else {
                        format!("{v:.0}")
                    }
                }),
        );
    });
    ui.checkbox(
        &mut p.samples,
        tr("試行ごとのずれと評価を結果に入れる（samples）"),
    );
    ui.horizontal(|ui| {
        ui.label(tr("構成"));
        small_edit(ui, &mut p.opt.configs, 160.0, tr("開いている系だけ"));
    });
    ui.label(
        RichText::new(tr("同じ種なら同じ結果になります（スレッド数によらない）。構成を並べると、同じずれをすべての構成に与えます。"))
            .small()
            .color(weak),
    );
}

fn f(v: &Value) -> String {
    v.as_f64().map_or("—".into(), fmt_num)
}

/// 補償子の分布の 1 つ: 名前・公称・統計・試行の値（追跡できた試行だけ）
struct CompDist {
    name: String,
    nominal: Value,
    stats: Value,
    values: Vec<f64>,
}

/// 結果の補償子と傾きの大きさ（分）を並べる
fn comp_dists(r: &Value) -> Vec<CompDist> {
    let mut out = Vec::new();
    let vals = |v: &Value| -> Vec<f64> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .filter(|x| x.is_finite())
            .collect()
    };
    for c in r["compensators"].as_array().into_iter().flatten() {
        out.push(CompDist {
            name: text_of(&c["name"]),
            nominal: c["nominal"].clone(),
            stats: c["stats"].clone(),
            values: vals(&c["values"]),
        });
    }
    for t in r["tilt_magnitudes"].as_array().into_iter().flatten() {
        out.push(CompDist {
            name: format!("{} [′]", text_of(&t["name"])),
            nominal: t["nominal"].clone(),
            stats: t["stats"].clone(),
            values: vals(&t["values"]),
        });
    }
    out
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
fn comps_ui(ui: &mut Ui, r: &Value, sel: &mut usize, cdf: &mut bool, weak: egui::Color32) {
    let ds = comp_dists(r);
    if ds.is_empty() {
        return;
    }
    ui.label(RichText::new(tr("補償子")).strong());
    let pcts: Vec<String> = ds[0].stats["percentiles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| format!("{}%", f(&x[0])))
        .collect();
    egui::ScrollArea::horizontal()
        .id_salt("tol_comps_h")
        .show(ui, |ui| {
            egui::Grid::new("tol_comps").striped(true).show(ui, |ui| {
                for h in [
                    tr("名前"),
                    tr("公称"),
                    tr("平均"),
                    tr("標準偏差"),
                    tr("最小"),
                    tr("最大"),
                ] {
                    ui.label(RichText::new(h).small().color(weak));
                }
                for p in &pcts {
                    ui.label(RichText::new(p).small().color(weak));
                }
                ui.end_row();
                for d in &ds {
                    ui.label(&d.name);
                    ui.label(f(&d.nominal));
                    for k in ["mean", "std", "min", "max"] {
                        ui.label(f(&d.stats[k]));
                    }
                    for x in d.stats["percentiles"].as_array().into_iter().flatten() {
                        ui.label(f(&x[1]));
                    }
                    ui.end_row();
                }
            });
        });
    if r["tilt_magnitudes"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        ui.label(
            RichText::new(tr(
                "「.tilt [′]」は同じ玉・群・面の傾き tx・ty の組の大きさ √(tx² + ty²)（分）。",
            ))
            .small()
            .color(weak),
        );
    }
    if ds.iter().all(|d| d.values.is_empty()) {
        return;
    }
    *sel = (*sel).min(ds.len() - 1);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("tol_comp_sel")
            .selected_text(ds[*sel].name.as_str())
            .show_ui(ui, |ui| {
                for (i, d) in ds.iter().enumerate() {
                    ui.selectable_value(sel, i, d.name.as_str());
                }
            });
        ui.selectable_value(cdf, false, tr("ヒストグラム"));
        ui.selectable_value(cdf, true, tr("累積分布"));
    });
    let d = &ds[*sel];
    let nom = d.nominal.as_f64();
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
        plot.y_axis_label(tr("累積 [%]")).show(ui, |pu| {
            pu.line(Line::new(pts).name(tr("累積分布")));
            if let Some(x) = nom {
                pu.vline(egui_plot::VLine::new(x).name(tr("公称")));
            }
        });
    } else {
        let bars: Vec<egui_plot::Bar> = histogram(&d.values)
            .into_iter()
            .map(|(x0, w, c)| egui_plot::Bar::new(x0 + w / 2.0, c as f64).width(w))
            .collect();
        plot.y_axis_label(tr("試行の数")).show(ui, |pu| {
            pu.bar_chart(egui_plot::BarChart::new(bars).name(tr("ヒストグラム")));
            if let Some(x) = nom {
                pu.vline(egui_plot::VLine::new(x).name(tr("公称")));
            }
        });
    }
}

fn result_ui(ui: &mut Ui, r: &Value, sel: &mut usize, as_cdf: &mut bool, weak: egui::Color32) {
    ui.label(trf(
        "試行 {}（追跡できなかった {}）、種 {}",
        &[&r["trials"], &r["failed"], &r["seed"]],
    ));
    if r["cancelled"].as_bool() == Some(true) {
        ui.label(RichText::new(tr("途中で止めた（終わった試行だけ）")).color(weak));
    }
    for e in r["errors"].as_array().into_iter().flatten() {
        ui.colored_label(ui.visuals().warn_fg_color, text_of(e));
    }
    // 統計の表（評価関数と項）
    let mut rows: Vec<(String, Value, Value)> = vec![(
        tr("評価関数").into(),
        r["nominal"]["merit"].clone(),
        r["merit"].clone(),
    )];
    for t in r["terms"].as_array().into_iter().flatten() {
        rows.push((
            text_of(&t["label"]),
            t["nominal"].clone(),
            t["stats"].clone(),
        ));
    }
    let pcts: Vec<String> = r["merit"]["percentiles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| format!("{}%", f(&x[0])))
        .collect();
    egui::ScrollArea::horizontal()
        .id_salt("tol_stats_h")
        .show(ui, |ui| {
            egui::Grid::new("tol_stats").striped(true).show(ui, |ui| {
                for h in ["", tr("公称"), tr("平均"), tr("標準偏差"), tr("最大")] {
                    ui.label(RichText::new(h).small().color(weak));
                }
                for p in &pcts {
                    ui.label(RichText::new(p).small().color(weak));
                }
                ui.end_row();
                for (name, nom, st) in &rows {
                    ui.label(name);
                    ui.label(f(nom));
                    ui.label(f(&st["mean"]));
                    ui.label(f(&st["std"]));
                    ui.label(f(&st["max"]));
                    for x in st["percentiles"].as_array().into_iter().flatten() {
                        ui.label(f(&x[1]));
                    }
                    ui.end_row();
                }
            });
        });
    if let Some(e) = r["rss_estimate"].as_f64() {
        ui.label(
            RichText::new(trf(
                "感度からの見積り（公称 + √Σ 変化²）: {}",
                &[&fmt_num(e)],
            ))
            .small()
            .color(weak),
        );
    }
    // 累積分布
    let cdf: Vec<f64> = r["cdf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    if !cdf.is_empty() {
        let n = cdf.len() as f64;
        let pts: PlotPoints = cdf
            .iter()
            .enumerate()
            .map(|(i, &x)| [x, 100.0 * (i + 1) as f64 / n])
            .collect();
        let nom = r["nominal"]["merit"].as_f64();
        Plot::new("tol_cdf")
            .height(180.0)
            .x_axis_label(tr("評価関数"))
            .y_axis_label(tr("累積 [%]"))
            .allow_scroll(false)
            .show(ui, |pu| {
                pu.line(Line::new(pts).name(tr("累積分布")));
                if let Some(x) = nom {
                    pu.vline(egui_plot::VLine::new(x).name(tr("公称")));
                }
            });
    }
    comps_ui(ui, r, sel, as_cdf, weak);
    // 感度（悪い順）
    let sens = r["sensitivity"].as_array().cloned().unwrap_or_default();
    if !sens.is_empty() {
        egui::CollapsingHeader::new(RichText::new(tr("寄与の大きい項目（感度、悪い順）")).strong())
            .id_salt("tol_sens")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("tol_sens_g").striped(true).show(ui, |ui| {
                    for h in [
                        tr("公差"),
                        tr("下限"),
                        tr("上限"),
                        tr("変化（下限）"),
                        tr("変化（上限）"),
                    ] {
                        ui.label(RichText::new(h).small().color(weak));
                    }
                    ui.end_row();
                    for s in sens.iter().take(20) {
                        ui.label(text_of(&s["label"]));
                        ui.label(f(&s["lo"]));
                        ui.label(f(&s["hi"]));
                        ui.label(f(&s["change_lo"]));
                        ui.label(f(&s["change_hi"]));
                        ui.end_row();
                    }
                });
                if sens.len() > 20 {
                    ui.label(
                        RichText::new(trf("ほか {} 項目", &[&(sens.len() - 20)]))
                            .small()
                            .color(weak),
                    );
                }
            });
    }
    let corr = r["correlation"].as_array().cloned().unwrap_or_default();
    if corr.iter().any(|c| c["r"].is_number()) {
        egui::CollapsingHeader::new(RichText::new(tr("試行との相関")).strong())
            .id_salt("tol_corr")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("tol_corr_g").striped(true).show(ui, |ui| {
                    for h in [tr("公差"), tr("ずれとの相関"), tr("|ずれ| との相関")] {
                        ui.label(RichText::new(h).small().color(weak));
                    }
                    ui.end_row();
                    for c in corr.iter().take(15) {
                        ui.label(text_of(&c["label"]));
                        ui.label(f(&c["r"]));
                        ui.label(f(&c["r_abs"]));
                        ui.end_row();
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            relative: false,
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
        // 欄のない公差（複数のパラメータ）が前、欄の分が後
        assert_eq!(back["tols"].as_array().unwrap().len(), 2);
        assert_eq!(back["tols"][1]["param"], json!("3.t"));
        assert!(q.extra.contains_key("tols"));
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
    fn comp_dists_and_histogram() {
        let r = json!({
            "compensators": [{"name": "M.tx", "nominal": 0.0, "stats": {}, "values": [0.1, null, -0.2]}],
            "tilt_magnitudes": [{"name": "M.tilt", "nominal": 0.0, "stats": {}, "values": [6.0, null, 12.0]}],
        });
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
