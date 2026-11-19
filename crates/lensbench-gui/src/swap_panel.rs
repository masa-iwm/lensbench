//! カタログレンズの差し替え探索のパネル: 差し替える所（玉）と候補の絞り込み、組ごとに最適化する間隔、
//! 設定を並べ、serve の `swap` で回して、評価関数の小さい順の組を表に出す。組合せの系なら、選んだ組を組合せに入れられる。
//!
//! 設定は `lensbench swap --spec` と同じ JSON（[`SwapPanel::spec`]）で、そのままファイルに保存・読込できる。
//! 走っている間は [`lensbench_engine::swap_progress`] で進み具合を読み、止めるのは
//! [`lensbench_engine::swap_cancel`]。
//!
//! 組合せの系では、空気に id を付けたプロジェクト（[`combo_project`]）を serve に渡す: 絞りの後の空気が `d0`、
//! 要素 k（1 始まり）の後の空気が `dk`。

use egui::{RichText, Ui};
use serde_json::{json, Value};

use lensbench_design::swap::{SwapCand, SwapReply, SwapResult};

use crate::combo::{Combo, ElemKind, Pick};
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::{section, small_edit, term_rows, terms_json, terms_ui, text_of, TermRow};
use crate::param_pick::{self as pp, Set};
use crate::theme;
use crate::widgets::sheet::{self, Cell, Sheet, SheetState};

/// 差し替える所の行
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SlotRow {
    /// 玉のブロックの id
    pub block: String,
    pub vendor: String,
    pub efl: [String; 2],
    pub dia: [String; 2],
    pub flip: bool,
}

/// 組ごとに最適化する変数の行
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
}

/// パネルの操作（app が serve に送る）
#[derive(Clone, Debug)]
pub(crate) enum Action {
    /// 組の数と候補だけ数える（spec）
    Count(Value),
    /// 回す（spec）
    Run(Value),
    Stop,
    /// 結果の行 i の組を組合せに入れる
    Insert(usize),
    /// 入れる前の組合せに戻す
    Undo,
    Save(Value),
    Load,
}

/// 候補（差し替えられる玉・間隔）
#[derive(Default)]
pub(crate) struct Cands {
    /// 玉のブロック (id, 説明)
    pub blocks: Vec<(String, String)>,
    /// id のある空気 (id, 説明)
    pub gaps: Vec<(String, String)>,
    /// プロジェクトの状態の名前（複数の状態で同時に評価するときに選ぶ）
    pub states: Vec<String>,
    /// 名前の欄の補完の候補
    pub sys: std::rc::Rc<crate::param_pick::Sys>,
}

pub(crate) struct SwapPanel {
    pub slots: Vec<SlotRow>,
    pub vars: Vec<VarRow>,
    pub merit: String,
    pub reference: String,
    pub rings: usize,
    pub iters: usize,
    /// 系の焦点距離の目標（空は目標なし）
    pub efl: String,
    pub max_count: usize,
    /// 粗いふるい（prescreen）を使う、その リング数・残す組の数
    pub pre: bool,
    pub pre_rings: usize,
    pub pre_keep: usize,
    /// 評価関数に足す項（optimize と同じ）
    pub terms: Vec<TermRow>,
    pub new_term: String,
    /// 出発点（1 行に 1 つ、「d1.t=10, d2.t=5」）
    pub starts: String,
    /// 同時に評価する状態（空は今の状態だけ）と、状態ごとに解く変数（カンマ区切り）
    pub states: Vec<String>,
    pub state_vars: String,
    /// 有効径の条件
    pub clear_aperture: bool,
    /// ヤコビアン（fd・auto）
    pub jac: String,
    /// 表に出す組の数
    pub top: usize,
    pub new_var: String,
    pub running: bool,
    /// serve の swap の結果（count_only なら組の数と候補だけ）
    pub result: Option<std::sync::Arc<SwapReply>>,
    pub error: Option<String>,
    /// 組合せに入れた行（表で印を付ける）
    pub inserted: Option<usize>,
    /// 入れる前の組合せ（元に戻す）
    pub undo: Option<Combo>,
    /// レンズ・変数・項の表の選択と書き換え中の文字
    pub slots_sheet: SheetState,
    pub vars_sheet: SheetState,
    pub terms_sheet: SheetState,
}

impl Default for SwapPanel {
    fn default() -> SwapPanel {
        SwapPanel {
            slots: Vec::new(),
            vars: Vec::new(),
            merit: "spot".into(),
            reference: "centroid".into(),
            rings: 3,
            iters: 30,
            efl: String::new(),
            max_count: 100_000,
            pre: false,
            pre_rings: 2,
            pre_keep: 1000,
            terms: Vec::new(),
            new_term: "paraxial".into(),
            starts: String::new(),
            states: Vec::new(),
            state_vars: String::new(),
            clear_aperture: false,
            jac: "forward".into(),
            top: 50,
            new_var: String::new(),
            running: false,
            result: None,
            error: None,
            inserted: None,
            undo: None,
            slots_sheet: SheetState::default(),
            vars_sheet: SheetState::default(),
            terms_sheet: SheetState::default(),
        }
    }
}

/// 数の欄の文字列（整数の値は小数点なし）
fn num_text(v: &Value) -> String {
    match v.as_f64() {
        Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", x as i64),
        Some(x) => x.to_string(),
        None => text_of(v),
    }
}

impl SwapPanel {
    /// `lensbench swap --spec` の JSON（保存するファイルと同じ）
    pub(crate) fn spec(&self) -> Result<Value, String> {
        let mut slots = Vec::new();
        for s in &self.slots {
            if s.block.trim().is_empty() {
                continue;
            }
            let b = s.block.trim();
            let mut f = json!({
                "efl": [crate::widgets::parse_num(&s.efl[0], b)?, crate::widgets::parse_num(&s.efl[1], b)?],
                "diameter": [crate::widgets::parse_num(&s.dia[0], b)?, crate::widgets::parse_num(&s.dia[1], b)?],
            });
            if !s.vendor.trim().is_empty() {
                f["vendor"] = json!(s.vendor.trim());
            }
            slots.push(json!({ "block": b, "filter": f, "flip": s.flip }));
        }
        let mut vars = Vec::new();
        for v in &self.vars {
            let n = v.name.trim();
            if n.is_empty() {
                continue;
            }
            let lo = crate::widgets::parse_num(&v.lo, n)?;
            let hi = crate::widgets::parse_num(&v.hi, n)?;
            vars.push(json!(if lo.is_none() && hi.is_none() {
                n.to_string()
            } else {
                let s = |x: Option<f64>| x.map(|x| x.to_string()).unwrap_or_default();
                format!("{n}={}:{}", s(lo), s(hi))
            }));
        }
        let mut p = json!({
            "slots": slots,
            "vars": vars,
            "merit": self.merit,
            "reference": self.reference,
            "rings": self.rings,
            "iters": self.iters,
            "max_count": self.max_count,
        });
        if self.pre {
            p["prescreen"] = json!({ "rings": self.pre_rings, "keep": self.pre_keep });
        }
        if let Some(e) = crate::widgets::parse_num(&self.efl, tr("Target focal length"))? {
            p["efl"] = json!(e);
        }
        let terms = terms_json(&self.terms)?;
        if !terms.is_empty() {
            p["terms"] = json!(terms);
        }
        let starts = parse_starts(&self.starts)?;
        if !starts.is_empty() {
            p["starts"] = json!(starts);
        }
        if !self.states.is_empty() {
            p["states"] = json!(self.states);
            let sv: Vec<&str> = self
                .state_vars
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .collect();
            if !sv.is_empty() {
                p["state_vars"] = json!(sv);
            }
        }
        if self.clear_aperture {
            p["clear_aperture"] = json!(true);
        }
        if self.jac != "forward" {
            p["jac"] = json!(self.jac);
        }
        Ok(p)
    }

    /// 保存したファイル（spec と同じ形）から
    pub(crate) fn set_spec(&mut self, p: &Value) -> Result<(), String> {
        let o = p
            .as_object()
            .ok_or_else(|| tr("Swap search settings are not an object").to_string())?;
        let mut n = SwapPanel {
            top: self.top,
            ..Default::default()
        };
        for (k, v) in o {
            match k.as_str() {
                // エディタの検査用（保存するときに付け直す）
                "$schema" => {}
                "slots" => {
                    for s in v.as_array().into_iter().flatten() {
                        let f = &s["filter"];
                        let pair = |a: &Value| [num_text(&a[0]), num_text(&a[1])];
                        n.slots.push(SlotRow {
                            block: s["block"].as_str().unwrap_or_default().into(),
                            vendor: f["vendor"].as_str().unwrap_or_default().into(),
                            efl: pair(&f["efl"]),
                            dia: pair(&f["diameter"]),
                            flip: s["flip"].as_bool().unwrap_or(false),
                        });
                    }
                }
                "vars" => {
                    for x in v.as_array().into_iter().flatten() {
                        // オブジェクトの形 {name, min, max} も読む
                        if x.is_object() {
                            n.vars.push(VarRow {
                                name: x["name"].as_str().unwrap_or_default().trim().into(),
                                lo: num_text(&x["min"]),
                                hi: num_text(&x["max"]),
                            });
                            continue;
                        }
                        let s = x.as_str().unwrap_or_default();
                        let (name, r) = s.split_once('=').unwrap_or((s, ""));
                        let (lo, hi) = r.split_once(':').unwrap_or(("", ""));
                        n.vars.push(VarRow {
                            name: name.trim().into(),
                            lo: lo.trim().into(),
                            hi: hi.trim().into(),
                        });
                    }
                }
                "merit" => n.merit = text_of(v),
                "reference" => n.reference = text_of(v),
                "rings" => n.rings = v.as_u64().unwrap_or(3) as usize,
                "iters" => n.iters = v.as_u64().unwrap_or(30) as usize,
                "efl" => n.efl = num_text(v),
                "max_count" => n.max_count = v.as_u64().unwrap_or(100_000) as usize,
                "prescreen" => {
                    n.pre = !v.is_null();
                    n.pre_rings = v["rings"].as_u64().unwrap_or(2) as usize;
                    n.pre_keep = v["keep"].as_u64().unwrap_or(1000) as usize;
                }
                "terms" => n.terms = term_rows(v)?,
                "starts" => n.starts = starts_text(v),
                "states" => {
                    n.states = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                }
                "state_vars" => {
                    n.state_vars = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| match x {
                            Value::String(s) => Some(s.clone()),
                            // オブジェクトの形は短い形 "名前=下限:上限" にして欄に出す
                            Value::Object(_) => {
                                let name = x["name"].as_str()?.trim();
                                let (lo, hi) = (num_text(&x["min"]), num_text(&x["max"]));
                                Some(if lo.is_empty() && hi.is_empty() {
                                    name.to_string()
                                } else {
                                    format!("{name}={lo}:{hi}")
                                })
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                }
                "clear_aperture" => n.clear_aperture = v.as_bool().unwrap_or(false),
                "jac" => n.jac = v.as_str().unwrap_or("forward").into(),
                // GUI では今の系の状態を使う
                "state" => {}
                k => return Err(trf("Unknown key in swap search settings: {}", &[&k])),
            }
        }
        *self = n;
        Ok(())
    }
}

/// 出発点の欄（1 行に 1 つ、「d1.t=10, d2.t=5」）から {名前: 値} の並びへ
pub(crate) fn parse_starts(s: &str) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    let mut out = Vec::new();
    for line in s.lines().flat_map(|l| l.split(';')).map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let mut o = serde_json::Map::new();
        for kv in line.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| trf("Start {} must be name=value", &[&kv]))?;
            let x: f64 = v
                .trim()
                .parse()
                .map_err(|_| trf("The value of start {} is not a number", &[&kv]))?;
            o.insert(k.trim().to_string(), json!(x));
        }
        out.push(o);
    }
    Ok(out)
}

fn starts_text(v: &Value) -> String {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, x)| format!("{k}={}", num_text(x)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 組合せのプロジェクトの空気に id（d0・d1…）を付けたもの、玉の候補、間隔の候補
pub(crate) fn combo_project(c: &Combo) -> (Value, Cands) {
    let mut p = c.project();
    let ids = c.ids();
    let mut cands = Cands {
        ..Default::default()
    };
    // project() の並び: 前の絞りがあれば [絞り, 空気]、続いて要素ごとに [要素, 空気]（要素の後の空気が dk）。
    // 要素が絞り（type stop）でも 1 つの要素として数える
    let n = c.picks.len();
    let off = if c.lead_stop() { 2 } else { 0 };
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    if off == 2 {
        gaps.push((1, 0));
    }
    gaps.extend((1..=n).map(|k| (off + 2 * k - 1, k)));
    if let Some(bs) = p["blocks"].as_array_mut() {
        for (at, k) in gaps {
            let Some(b) = bs.get_mut(at).filter(|b| b["type"] == "air") else {
                continue;
            };
            let id = format!("d{k}");
            b["id"] = json!(id);
            let desc = if k == 0 {
                tr("Stop → first element").to_string()
            } else if k == n {
                trf("{} → image", &[&ids[k - 1]])
            } else {
                trf("after {}", &[&ids[k - 1]])
            };
            cands.gaps.push((id, desc));
        }
    }
    for (pk, id) in c.picks.iter().zip(&ids) {
        if pk.kind == ElemKind::Catalog && pk.path.is_empty() {
            cands
                .blocks
                .push((id.clone(), format!("{} {}", pk.vendor, pk.part)));
        }
    }
    (p, cands)
}

/// 結果の行の組を組合せに入れる（玉の型番・向き、変数の間隔 dK.t）。`ids` は回したときの組合せの id。
/// 入れられなければ理由
pub(crate) fn insert_row(c: &mut Combo, ids: &[String], row: &SwapResult) -> Result<(), String> {
    let mut n = c.clone();
    for l in &row.lenses {
        let b = &l.block;
        let k = ids
            .iter()
            .position(|x| x == b)
            .filter(|&k| k < n.picks.len())
            .ok_or_else(|| {
                trf(
                    "{} is not in the combination (run again after changing the combination)",
                    &[&b],
                )
            })?;
        // カタログの玉に替える。前の玉のファイル（.zmx・ユーザーカタログ）・面の上書き・面の操作は持ち越さない
        let pk = &mut n.picks[k];
        *pk = Pick {
            vendor: l.vendor.clone(),
            part: l.part.clone(),
            reversed: l.reversed,
            stop_front: pk.stop_front,
            ..pk.retyped(ElemKind::Catalog)
        };
    }
    for v in row.fit.iter().flat_map(|f| &f.vars) {
        let name = &v.name;
        let x = v.x.unwrap_or(f64::NAN);
        let k = name
            .strip_prefix('d')
            .and_then(|s| s.strip_suffix(".t"))
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&k| k <= n.picks.len() && x.is_finite())
            .ok_or_else(|| {
                trf(
                    "Variable {} cannot be put into the combination (only gaps dK.t)",
                    &[&name],
                )
            })?;
        if k == 0 {
            n.stop_gap = x;
        } else {
            n.picks[k - 1].air = x;
        }
    }
    *c = n;
    Ok(())
}

/// パネルを描く。`progress` は走っている間の (終えた組, 組の数)
pub(crate) fn ui(
    ui: &mut Ui,
    p: &mut SwapPanel,
    c: &Cands,
    progress: Option<(usize, usize)>,
) -> Vec<Action> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical()
        .id_salt("swap_scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if p.running {
                    if ui.button(tr("Stop")).clicked() {
                        acts.push(Action::Stop);
                    }
                    ui.spinner();
                } else {
                    let ok = p.slots.iter().any(|s| !s.block.trim().is_empty());
                    let run = ui.add_enabled(
                        ok,
                        egui::Button::new(RichText::new(tr("Run search")).strong()),
                    );
                    let count = ui.add_enabled(ok, egui::Button::new(tr("Count sets")));
                    for (r, f) in [
                        (run, Action::Run as fn(Value) -> Action),
                        (count, Action::Count),
                    ] {
                        if r.clicked() {
                            match p.spec() {
                                Ok(v) => {
                                    p.error = None;
                                    acts.push(f(v));
                                }
                                Err(e) => p.error = Some(e),
                            }
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
            if let Some((d, n)) = progress {
                ui.add(
                    egui::ProgressBar::new(if n > 0 { d as f32 / n as f32 } else { 0.0 })
                        .text(trf("Set {}/{}", &[&d, &n])),
                );
            } else if let Some(r) = p.result.clone() {
                egui::CollapsingHeader::new(RichText::new(tr("Result")).strong())
                    .id_salt("swap_result")
                    .default_open(true)
                    .show(ui, |ui| result_ui(ui, p, &r, &mut acts, weak));
            }
            ui.separator();
            ui.add_enabled_ui(!p.running, |ui| {
                section(ui, "swap_slots_sec", tr("Lenses to swap"), |ui| {
                    slots_ui(ui, p, c, weak)
                });
                section(ui, "swap_vars_sec", tr("Gaps optimized per set"), |ui| {
                    vars_ui(ui, p, c, weak)
                });
                section(ui, "swap_terms_sec", tr("Extra merit terms"), |ui| {
                    terms_ui(
                        ui,
                        "swap",
                        &mut p.terms,
                        &mut p.new_term,
                        &mut p.terms_sheet,
                        None,
                        weak,
                    );
                });
                section(ui, "swap_states_sec", tr("Starts and states"), |ui| {
                    starts_states_ui(ui, p, c, weak)
                });
                section(ui, "swap_settings_sec", tr("Settings"), |ui| {
                    settings_ui(ui, p, weak)
                });
            });
        });
    acts
}

fn slots_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    if c.blocks.is_empty() {
        ui.label(
            RichText::new(tr(
                "No lens can be swapped (add catalog lenses to the combination, or open a project with lens blocks that have ids).",
            ))
            .small()
            .color(weak),
        );
    }
    if !p.slots.is_empty() {
        let mut ss = SlotSheet {
            slots: &mut p.slots,
            blocks: &c.blocks,
        };
        sheet::show(ui, "swap_slots", &mut p.slots_sheet, &mut ss);
    }
    let free: Vec<&(String, String)> = c
        .blocks
        .iter()
        .filter(|b| !p.slots.iter().any(|s| s.block == b.0))
        .collect();
    if ui
        .add_enabled(!free.is_empty(), egui::Button::new(tr("Add lens")))
        .clicked()
    {
        p.slots.push(SlotRow {
            block: free[0].0.clone(),
            ..Default::default()
        });
    }
    ui.label(
        RichText::new(tr(
            "Candidates are catalog lenses filtered by focal length, diameter and vendor (empty fields do not filter; focal length may be negative).",
        ))
        .small()
        .color(weak),
    );
}

/// 差し替える所の表（レンズ・メーカー・焦点距離と径の範囲・反転も）
struct SlotSheet<'a> {
    slots: &'a mut Vec<SlotRow>,
    blocks: &'a [(String, String)],
}

impl Sheet for SlotSheet<'_> {
    fn rows(&self) -> usize {
        self.slots.len()
    }
    fn cols(&self) -> usize {
        7
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        match col {
            0 => tr("Lens").into(),
            1 => tr("Vendor").into(),
            2 => trf("{} min", &[&tr("Focal length [mm]")]),
            3 => trf("{} max", &[&tr("Focal length [mm]")]),
            4 => trf("{} min", &[&tr("Diameter [mm]")]),
            5 => trf("{} max", &[&tr("Diameter [mm]")]),
            _ => tr("Flip too").into(),
        }
    }
    fn width(&self, col: usize) -> f32 {
        [140.0, 70.0, 64.0, 64.0, 56.0, 56.0, 56.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let s = &self.slots[row];
        match col {
            0 => Cell::list(s.block.clone(), self.blocks.to_vec(), false).tip(
                self.blocks
                    .iter()
                    .find(|b| b.0 == s.block)
                    .map(|b| b.1.clone())
                    .unwrap_or_default(),
            ),
            1 => Cell::text(s.vendor.clone()).hint(tr("All")),
            2 => Cell::num(s.efl[0].clone()).hint("−∞"),
            3 => Cell::num(s.efl[1].clone()).hint("∞"),
            4 => Cell::num(s.dia[0].clone()).hint("0"),
            5 => Cell::num(s.dia[1].clone()).hint("∞"),
            _ => {
                Cell::boolean(s.flip).tip(tr("Try both orientations (doubles the number of sets)"))
            }
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let t = text.trim();
        if col == 0 && !self.blocks.iter().any(|b| b.0 == t) {
            return Err(trf("No lens {}", &[&t]));
        }
        if (2..=5).contains(&col) {
            crate::widgets::parse_num(t, &self.header(col, None))?;
        }
        let s = &mut self.slots[row];
        match col {
            0 => s.block = t.into(),
            1 => s.vendor = t.into(),
            2 | 3 => s.efl[col - 2] = t.into(),
            4 | 5 => s.dia[col - 4] = t.into(),
            _ => s.flip = t == "true",
        }
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        // まだ表に無い玉
        let Some(b) = self
            .blocks
            .iter()
            .find(|b| !self.slots.iter().any(|s| s.block == b.0))
        else {
            return false;
        };
        self.slots.insert(
            at,
            SlotRow {
                block: b.0.clone(),
                ..Default::default()
            },
        );
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.slots.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.slots.swap(a, a + 1);
        true
    }
}

/// 組ごとに最適化する変数の表
struct VarSheet<'a> {
    vars: &'a mut Vec<VarRow>,
    names: Vec<(String, String)>,
}

impl Sheet for VarSheet<'_> {
    fn rows(&self) -> usize {
        self.vars.len()
    }
    fn cols(&self) -> usize {
        3
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Name"), tr("Min"), tr("Max")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [110.0, 64.0, 64.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let v = &self.vars[row];
        match col {
            0 => {
                let d = self
                    .names
                    .iter()
                    .find(|n| n.0 == v.name)
                    .map(|n| n.1.clone())
                    .unwrap_or_default();
                Cell::list(v.name.clone(), self.names.clone(), true)
                    .hint("d1.t")
                    .tip(d)
            }
            1 => Cell::num(v.lo.clone()).hint("−∞"),
            _ => Cell::num(v.hi.clone()).hint("∞"),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let t = text.trim();
        let v = &mut self.vars[row];
        match col {
            0 => v.name = t.into(),
            _ => {
                crate::widgets::parse_num(t, &v.name)?;
                if col == 1 {
                    v.lo = t.into();
                } else {
                    v.hi = t.into();
                }
            }
        }
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        self.vars.insert(
            at,
            VarRow {
                lo: "0".into(),
                ..Default::default()
            },
        );
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.vars.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.vars.swap(a, a + 1);
        true
    }
}

fn vars_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    let set = Set::Swap { gaps: &c.gaps };
    if !p.vars.is_empty() {
        let mut vs = VarSheet {
            vars: &mut p.vars,
            names: pp::names(&c.sys, set),
        };
        sheet::show(ui, "swap_vars", &mut p.vars_sheet, &mut vs);
    }
    ui.horizontal_wrapped(|ui| {
        let (_, enter) = pp::edit_pick(ui, &mut p.new_var, 90.0, tr("d1.t, G2.shift"), &c.sys, set);
        if (ui.button(tr("Add")).clicked() || enter) && !p.new_var.trim().is_empty() {
            p.vars.push(VarRow {
                name: p.new_var.trim().into(),
                lo: "0".into(),
                hi: String::new(),
            });
            p.new_var.clear();
        }
        ui.menu_button(tr("Add from gaps"), |ui| {
            for (id, d) in &c.gaps {
                let name = format!("{id}.t");
                let have = p.vars.iter().any(|v| v.name == name);
                if ui
                    .add_enabled(!have, egui::Button::new(format!("{name}  {d}")))
                    .clicked()
                {
                    p.vars.push(VarRow {
                        name,
                        lo: "0".into(),
                        hi: String::new(),
                    });
                    ui.close_menu();
                }
            }
        });
    });
    ui.label(
        RichText::new(tr(
            "Combination gaps are d0 (after the stop), d1 (after the first element), ... Only gaps can be put back into the combination.",
        ))
        .small()
        .color(weak),
    );
}

fn starts_states_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    ui.label(tr("Starting points (one per line)"));
    ui.add(
        egui::TextEdit::multiline(&mut p.starts)
            .desired_rows(2)
            .desired_width(260.0)
            .hint_text("d1.t=10, d2.t=5"),
    );
    ui.label(
        RichText::new(tr(
            "Each combination is optimized from every starting point and the best solution is kept (unlisted variables use the project values; empty means one run).",
        ))
        .small()
        .color(weak),
    );
    ui.separator();
    if c.states.is_empty() {
        ui.label(
            RichText::new(tr(
                "In a project with states, several states can be evaluated together.",
            ))
            .small()
            .color(weak),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("States evaluated together"));
        for s in &c.states {
            let mut on = p.states.contains(s);
            if ui.checkbox(&mut on, s).changed() {
                if on {
                    p.states.push(s.clone());
                    // 状態の並びの順にそろえる
                    p.states
                        .sort_by_key(|x| c.states.iter().position(|y| y == x));
                } else {
                    p.states.retain(|x| x != s);
                }
            }
        }
    });
    ui.add_enabled_ui(!p.states.is_empty(), |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("Per-state variables"));
            small_edit(ui, &mut p.state_vars, 160.0, "G2.shift, d3.t");
        });
    });
    ui.label(
        RichText::new(tr(
            "With states selected, each combination is optimized over all of them together (the merit is the image terms of every state; term configs are indices in this list). Space variables are shared; per-state variables get a value for each state.",
        ))
        .small()
        .color(weak),
    );
}

fn settings_ui(ui: &mut Ui, p: &mut SwapPanel, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Merit function"));
        for (k, l) in [("spot", tr("RMS spot")), ("wavefront", tr("RMS wavefront"))] {
            ui.selectable_value(&mut p.merit, k.to_string(), l);
        }
        ui.label(tr("Reference"));
        for (k, l) in [("centroid", tr("Centroid")), ("chief", tr("Chief ray"))] {
            ui.selectable_value(&mut p.reference, k.to_string(), l);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Rings"));
        ui.add(egui::DragValue::new(&mut p.rings).range(1..=20));
        ui.label(tr("Iterations per set"));
        ui.add(egui::DragValue::new(&mut p.iters).range(1..=1000));
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Target focal length"));
        small_edit(ui, &mut p.efl, 60.0, tr("Off"));
        ui.label(tr("Max sets"));
        ui.add(egui::DragValue::new(&mut p.max_count).range(1..=10_000_000));
        ui.label(tr("Table rows"));
        ui.add(egui::DragValue::new(&mut p.top).range(0..=10_000))
            .on_hover_text(tr("0 = all"));
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut p.clear_aperture, tr("Clear aperture condition"))
            .on_hover_text(tr(
            "Require the pupil-edge rays of every field to stay inside the radius on each surface of the swapped lenses",
        ));
        ui.separator();
        ui.label(tr("Derivatives"));
        for (k, l) in [
            ("forward", tr("All forward differences")),
            ("fd", tr("All finite differences")),
            ("auto", tr("Analytic (where possible)")),
        ] {
            ui.selectable_value(&mut p.jac, k.to_string(), l);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut p.pre, tr("Coarse prescreen"))
            .on_hover_text(tr(
                "Optimize all sets with fewer rings first, then re-optimize only the best sets with the full settings (faster with many sets)",
            ));
        ui.add_enabled_ui(p.pre, |ui| {
            ui.label(tr("Rings"));
            ui.add(egui::DragValue::new(&mut p.pre_rings).range(1..=20));
            ui.label(tr("Keep"));
            ui.add(egui::DragValue::new(&mut p.pre_keep).range(1..=10_000_000));
        });
    });
    ui.label(
        RichText::new(tr(
            "Gaps are optimized per set by damped least squares and the sets are sorted by merit. A target focal length is added to the merit function.",
        ))
        .small()
        .color(weak),
    );
}

fn lens_text(vendor: &str, part: &str, reversed: bool) -> String {
    format!("{vendor} {part}{}", if reversed { " ↺" } else { "" })
}

fn cand_text(l: &SwapCand) -> String {
    lens_text(&l.vendor, &l.part, l.reversed)
}

fn result_ui(
    ui: &mut Ui,
    p: &mut SwapPanel,
    r: &SwapReply,
    acts: &mut Vec<Action>,
    weak: egui::Color32,
) {
    let count = r.count;
    let Some(rows) = &r.results else {
        // 数えただけ
        let per: Vec<String> = r.slots.iter().map(|s| s.len().to_string()).collect();
        ui.label(trf(
            "{} sets (candidates per lens {})",
            &[&count, &per.join(" × ")],
        ));
        for (k, s) in r.slots.iter().enumerate() {
            let names: Vec<String> = s.iter().map(cand_text).collect();
            let b = p.slots.get(k).map(|s| s.block.as_str()).unwrap_or("");
            egui::CollapsingHeader::new(format!("{b}: {}", names.len()))
                .id_salt(("swap_cands", k))
                .show(ui, |ui| {
                    ui.label(RichText::new(names.join(crate::i18n::tr(", "))).small());
                });
        }
        return;
    };
    ui.label(trf(
        "{} sets ({} evaluated, {} failed, {} s){}",
        &[
            &count,
            &r.done.unwrap_or(0),
            &r.failed.unwrap_or(0),
            &fmt_num((r.seconds.unwrap_or(0.0) * 10.0).round() / 10.0),
            &if r.cancelled == Some(true) {
                tr(", stopped")
            } else {
                ""
            },
        ],
    ));
    // 焦点距離の目標があれば、評価関数と像だけの値が違う
    let with_efl = rows
        .iter()
        .filter_map(|x| x.fit.as_ref())
        .any(|f| f.image_merit.is_some() && f.image_merit != f.merit);
    let vars = rows.iter().find_map(|x| x.fit.as_ref().map(|f| &f.vars));
    let nvars = vars.map_or(0, Vec::len);
    egui::ScrollArea::horizontal()
        .id_salt("swap_res_h")
        .show(ui, |ui| {
            egui::Grid::new("swap_res").striped(true).show(ui, |ui| {
                let mut hs = vec![tr("Rank").to_string(), String::new()];
                hs.push(tr("Lens").into());
                hs.push(tr("Merit function").into());
                if with_efl {
                    hs.push(tr("Image merit").into());
                }
                hs.push("EFL".into());
                hs.push(tr("Total track").into());
                hs.extend(vars.into_iter().flatten().map(|x| x.name.clone()));
                for h in &hs {
                    ui.label(RichText::new(h).small().color(weak));
                }
                ui.end_row();
                for (i, x) in rows.iter().enumerate() {
                    let mark = if p.inserted == Some(i) { " ✔" } else { "" };
                    ui.label(format!("{}{mark}", i + 1));
                    if ui
                        .add_enabled(x.error.is_none(), egui::Button::new(tr("Use")).small())
                        .on_hover_text(tr("Put this set into the combination"))
                        .clicked()
                    {
                        acts.push(Action::Insert(i));
                    }
                    let ls: Vec<String> = x
                        .lenses
                        .iter()
                        .map(|l| lens_text(&l.vendor, &l.part, l.reversed))
                        .collect();
                    ui.label(ls.join(" / "));
                    let fit = match (&x.error, &x.fit) {
                        (None, Some(f)) => f,
                        (e, _) => {
                            ui.label(
                                RichText::new(e.as_deref().unwrap_or(""))
                                    .small()
                                    .color(weak),
                            );
                            ui.end_row();
                            continue;
                        }
                    };
                    let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
                    let short = |v: Option<f64>| format!("{:.4}", nan(v));
                    ui.monospace(format!("{:.4e}", nan(fit.merit)));
                    if with_efl {
                        ui.monospace(format!("{:.4e}", nan(fit.image_merit)));
                    }
                    ui.monospace(short(fit.efl));
                    ui.monospace(short(fit.total_track));
                    for k in 0..nvars {
                        ui.monospace(short(fit.vars.get(k).and_then(|v| v.x)));
                    }
                    ui.end_row();
                }
            });
        });
    if ui
        .add_enabled(p.undo.is_some(), egui::Button::new(tr("Undo use")))
        .clicked()
    {
        acts.push(Action::Undo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::Pick;

    /// 差し替え探索の 1 組（足りない鍵は既定で埋める）
    fn swap_row(mut v: Value) -> SwapResult {
        for (k, d) in [
            ("i", json!(0)),
            ("stop", json!("")),
            ("iterations", json!(0)),
        ] {
            v[k] = d;
        }
        serde_json::from_value(v).unwrap()
    }

    fn cat(v: &str, q: &str) -> Pick {
        let mut p = Pick::new(ElemKind::Catalog);
        p.vendor = v.into();
        p.part = q.into();
        p.air = 5.0;
        p
    }

    #[test]
    fn spec_round_trip() {
        let mut p = SwapPanel::default();
        p.slots.push(SlotRow {
            block: "A".into(),
            vendor: "LB".into(),
            efl: ["50.5".into(), "".into()],
            dia: ["".into(), "25.4".into()],
            flip: true,
        });
        p.vars.push(VarRow {
            name: "d1.t".into(),
            lo: "0.5".into(),
            hi: "20".into(),
        });
        p.vars.push(VarRow {
            name: "G2.shift".into(),
            ..Default::default()
        });
        p.efl = "100".into();
        assert!(p.spec().unwrap().get("prescreen").is_none());
        p.pre = true;
        p.pre_keep = 500;
        let s = p.spec().unwrap();
        assert_eq!(s["prescreen"], json!({"rings": 2, "keep": 500}));
        assert_eq!(s["vars"], json!(["d1.t=0.5:20", "G2.shift"]));
        assert_eq!(s["slots"][0]["filter"]["efl"], json!([50.5, null]));
        assert_eq!(s["efl"], json!(100.0));
        // 保存するファイル（先頭に $schema）を読み戻しても同じ。$schema は計算の設定に入れない
        let saved = lensbench_project::schema::pretty_with_schema(&s, "swap");
        let file: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            file["$schema"],
            lensbench_project::schema::url("swap").as_str()
        );
        let mut q = SwapPanel::default();
        q.set_spec(&file).unwrap();
        assert_eq!(q.spec().unwrap(), s);
        let mut q = SwapPanel::default();
        q.set_spec(&s).unwrap();
        assert_eq!(q.slots, p.slots);
        assert_eq!(q.vars, p.vars);
        assert_eq!(q.efl, "100");
        assert!(q.pre);
        assert_eq!((q.pre_rings, q.pre_keep), (2, 500));
        assert!(q.set_spec(&json!({"bad": 1})).is_err());
    }

    #[test]
    fn spec_terms_starts_states() {
        let v = json!({
            "slots": [], "vars": ["d1.t=0:"], "merit": "spot", "reference": "centroid",
            "rings": 3, "iters": 30, "max_count": 100000,
            "terms": [{"type": "paraxial", "q": "efl", "min": 351.0}],
            "starts": [{"d1.t": 10.0, "d2.t": 5.0}, {"d1.t": 20.0}],
            "states": ["wide", "tele"], "state_vars": ["G2.shift"],
            "clear_aperture": true, "jac": "auto",
        });
        let mut p = SwapPanel::default();
        p.set_spec(&v).unwrap();
        assert_eq!(p.starts, "d1.t=10, d2.t=5\nd1.t=20");
        assert_eq!(p.spec().unwrap(), v);
        // 状態を選ばなければ state_vars は送らない、出発点の書き間違いは誤り
        p.states.clear();
        assert!(p.spec().unwrap().get("state_vars").is_none());
        p.starts = "d1.t".into();
        assert!(p.spec().is_err());
        assert!(parse_starts("d1.t=1; d2.t=2").unwrap().len() == 2);
    }

    #[test]
    fn combo_ids_and_insert() {
        let c = Combo {
            picks: vec![cat("LB", "PCX-100"), cat("LB", "ACH-45")],
            ..Default::default()
        };
        let (p, cands) = combo_project(&c);
        let airs: Vec<&str> = p["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "air")
            .map(|b| b["id"].as_str().unwrap())
            .collect();
        let lead = c.lead_stop();
        assert_eq!(
            airs,
            if lead {
                vec!["d0", "d1", "d2"]
            } else {
                vec!["d1", "d2"]
            }
        );
        assert_eq!(cands.blocks.len(), 2);
        assert_eq!(cands.gaps.last().unwrap().0, "d2");
        let ids = c.ids();
        let row = swap_row(json!({
            "lenses": [{"block": ids[1], "vendor": "LB", "part": "BCX-50", "reversed": true}],
            "vars": [{"name": "d1.t", "param": "", "x": 7.5}, {"name": "d2.t", "param": "", "x": 40.0}],
        }));
        let mut d = c.clone();
        insert_row(&mut d, &ids, &row).unwrap();
        assert_eq!(d.picks[1].part, "BCX-50");
        assert!(d.picks[1].reversed);
        assert_eq!(d.picks[0].air, 7.5);
        assert_eq!(d.picks[1].air, 40.0);
        // 入れられない変数なら何も変えない
        let bad =
            swap_row(json!({"lenses": [], "vars": [{"name": "G2.shift", "param": "", "x": 1.0}]}));
        let mut e = c.clone();
        assert!(insert_row(&mut e, &ids, &bad).is_err());
        assert_eq!(e.picks[0].air, 5.0);
        // .zmx の玉・ユーザーカタログの項目からの差し替えは、前のファイルを持ち越さずカタログの玉のブロックに
        let mut z = Pick::new(ElemKind::Zmx);
        z.path = "a.zmx".into();
        z.zmx_mut().lens = 2;
        z.stop_front = true;
        z.air = 3.0;
        let c = Combo {
            picks: vec![z, cat("LB", "ACH-45")],
            ..Default::default()
        };
        let ids = c.ids();
        let row = swap_row(json!({
            "lenses": [{"block": ids[0], "vendor": "LB", "part": "BCX-50", "reversed": false}],
            "vars": [],
        }));
        let mut d = c.clone();
        insert_row(&mut d, &ids, &row).unwrap();
        let p = &d.picks[0];
        assert_eq!(
            (p.kind, p.path.as_str(), p.air),
            (ElemKind::Catalog, "", 3.0)
        );
        assert!(p.stop_front);
        let b = p.block("x");
        assert_eq!(b["source"], json!({"vendor": "LB", "part": "BCX-50"}));
    }

    #[test]
    fn combo_stop_element_counts() {
        // 要素の絞りがあっても d1 は 1 つ目の要素の後、d3 は像面の前
        let c = Combo {
            picks: vec![
                cat("LB", "PCX-100"),
                Pick::new(ElemKind::Stop),
                cat("LB", "ACH-45"),
            ],
            ..Default::default()
        };
        let (p, cands) = combo_project(&c);
        let ids = c.ids();
        let bs = p["blocks"].as_array().unwrap();
        let off = if c.lead_stop() { 2 } else { 0 };
        for k in 1..=3 {
            assert_eq!(bs[off + 2 * k - 1]["type"], "air");
            assert_eq!(bs[off + 2 * k - 1]["id"], json!(format!("d{k}")));
            assert_eq!(bs[off + 2 * k - 2]["id"], json!(ids[k - 1]));
        }
        let last = cands.gaps.last().unwrap();
        assert_eq!(last.0, "d3");
        assert!(last.1.contains(&ids[2]), "{}", last.1);
        let mut d = c.clone();
        let row =
            swap_row(json!({"lenses": [], "vars": [{"name": "d2.t", "param": "", "x": 4.0}]}));
        insert_row(&mut d, &ids, &row).unwrap();
        assert_eq!(d.picks[1].air, 4.0);
    }
}
