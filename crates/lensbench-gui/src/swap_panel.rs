//! カタログレンズの差し替え探索のパネル: 差し替える所（玉）と候補の絞り込み、組ごとに最適化する間隔、
//! 設定を並べ、serve の `swap` で回して、評価関数の小さい順の組を表に出す。組合せの系なら、選んだ組を組合せに入れられる。
//!
//! 設定は `lensbench swap --spec` と同じ JSON（[`SwapPanel::spec`]）で、そのままファイルに保存・読込できる。
//! 走っている間は [`lensbench_cli::serve::swap_progress`] で進み具合を読み、止めるのは
//! [`lensbench_cli::serve::swap_cancel`]。
//!
//! 組合せの系では、空気に id を付けたプロジェクト（[`combo_project`]）を serve に渡す: 絞りの後の空気が `d0`、
//! 要素 k（1 始まり）の後の空気が `dk`。

use egui::{RichText, Ui};
use serde_json::{json, Value};

use crate::combo::{Combo, ElemKind};
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::{section, small_edit, term_rows, terms_json, terms_ui, text_of, TermRow};
use crate::param_pick::{self as pp, Set};
use crate::theme;

/// 差し替える所の行
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotRow {
    /// 玉のブロックの id
    pub block: String,
    pub vendor: String,
    pub efl: [String; 2],
    pub dia: [String; 2],
    pub flip: bool,
}

/// 組ごとに最適化する変数の行
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
}

/// パネルの操作（app が serve に送る）
pub enum Action {
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
pub struct Cands {
    /// 玉のブロック (id, 説明)
    pub blocks: Vec<(String, String)>,
    /// id のある空気 (id, 説明)
    pub gaps: Vec<(String, String)>,
    /// 組合せの系（結果を組合せに入れられる）
    pub combo: bool,
    /// プロジェクトの状態の名前（複数の状態で同時に評価するときに選ぶ）
    pub states: Vec<String>,
    /// 名前の欄の補完の候補（開いた系の玉・群）
    pub sys: crate::param_pick::Sys,
}

pub struct SwapPanel {
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
    pub result: Option<Value>,
    pub error: Option<String>,
    /// 組合せに入れた行（表で印を付ける）
    pub inserted: Option<usize>,
    /// 入れる前の組合せ（元に戻す）
    pub undo: Option<Combo>,
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

fn num(s: &str, what: &str) -> Result<Option<f64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<f64>()
        .map(Some)
        .map_err(|_| trf("{} の値 {} が数でない", &[&what, &t]))
}

impl SwapPanel {
    /// `lensbench swap --spec` の JSON（保存するファイルと同じ）
    pub fn spec(&self) -> Result<Value, String> {
        let mut slots = Vec::new();
        for s in &self.slots {
            if s.block.trim().is_empty() {
                continue;
            }
            let b = s.block.trim();
            let mut f = json!({
                "efl": [num(&s.efl[0], b)?, num(&s.efl[1], b)?],
                "diameter": [num(&s.dia[0], b)?, num(&s.dia[1], b)?],
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
            let lo = num(&v.lo, n)?;
            let hi = num(&v.hi, n)?;
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
        if let Some(e) = num(&self.efl, tr("焦点距離の目標"))? {
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
    pub fn set_spec(&mut self, p: &Value) -> Result<(), String> {
        let o = p
            .as_object()
            .ok_or_else(|| tr("差し替え探索の設定がオブジェクトでない").to_string())?;
        let mut n = SwapPanel {
            top: self.top,
            ..Default::default()
        };
        for (k, v) in o {
            match k.as_str() {
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
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                }
                "clear_aperture" => n.clear_aperture = v.as_bool().unwrap_or(false),
                "jac" => n.jac = v.as_str().unwrap_or("forward").into(),
                // GUI では今の系の状態を使う
                "state" => {}
                k => return Err(trf("差し替え探索の設定のキー {}", &[&k])),
            }
        }
        *self = n;
        Ok(())
    }
}

/// 出発点の欄（1 行に 1 つ、「d1.t=10, d2.t=5」）から {名前: 値} の並びへ
pub fn parse_starts(s: &str) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    let mut out = Vec::new();
    for line in s.lines().flat_map(|l| l.split(';')).map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let mut o = serde_json::Map::new();
        for kv in line.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| trf("出発点 {} は 名前=値", &[&kv]))?;
            let x: f64 = v
                .trim()
                .parse()
                .map_err(|_| trf("出発点 {} の値が数でない", &[&kv]))?;
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
pub fn combo_project(c: &Combo) -> (Value, Cands) {
    let mut p = c.project();
    let ids = c.ids();
    let mut cands = Cands {
        combo: true,
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
                tr("絞り → 最初の要素").to_string()
            } else if k == n {
                trf("{} → 像面", &[&ids[k - 1]])
            } else {
                trf("{} の後", &[&ids[k - 1]])
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

/// プロジェクトの JSON の玉と、id のある空気
pub fn project_cands(p: &Value) -> Cands {
    let mut c = Cands::default();
    fn walk(bs: &Value, c: &mut Cands) {
        for b in bs.as_array().into_iter().flatten() {
            let id = b["id"].as_str().unwrap_or_default();
            match b["type"].as_str() {
                Some("lens") if !id.is_empty() => {
                    let s = &b["source"];
                    let d = match (s["vendor"].as_str(), s["part"].as_str()) {
                        (Some(v), Some(q)) => format!("{v} {q}"),
                        _ => String::new(),
                    };
                    c.blocks.push((id.into(), d));
                }
                Some("air") if !id.is_empty() => c.gaps.push((id.into(), String::new())),
                Some("group") => walk(&b["blocks"], c),
                _ => {}
            }
        }
    }
    walk(&p["blocks"], &mut c);
    c
}

/// 結果の行の組を組合せに入れる（玉の型番・向き、変数の間隔 dK.t）。`ids` は回したときの組合せの id。
/// 入れられなければ理由
pub fn insert_row(c: &mut Combo, ids: &[String], row: &Value) -> Result<(), String> {
    let mut n = c.clone();
    for l in row["lenses"].as_array().into_iter().flatten() {
        let b = l["block"].as_str().unwrap_or_default();
        let k = ids
            .iter()
            .position(|x| x == b)
            .filter(|&k| k < n.picks.len())
            .ok_or_else(|| trf("組合せに {} がない（組合せを変えた後は回し直して）", &[&b]))?;
        let pk = &mut n.picks[k];
        pk.kind = ElemKind::Catalog;
        pk.vendor = l["vendor"].as_str().unwrap_or_default().into();
        pk.part = l["part"].as_str().unwrap_or_default().into();
        pk.reversed = l["reversed"].as_bool().unwrap_or(false);
        // 面の上書きは前の玉の面のもの
        pk.edits.clear();
    }
    for v in row["vars"].as_array().into_iter().flatten() {
        let name = v["name"].as_str().unwrap_or_default();
        let x = v["x"].as_f64().unwrap_or(f64::NAN);
        let k = name
            .strip_prefix('d')
            .and_then(|s| s.strip_suffix(".t"))
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&k| k <= n.picks.len() && x.is_finite())
            .ok_or_else(|| trf("変数 {} は組合せに入れられない（間隔 dK.t だけ）", &[&name]))?;
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
pub fn ui(
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
                    if ui.button(tr("止める")).clicked() {
                        acts.push(Action::Stop);
                    }
                    ui.spinner();
                } else {
                    let ok = p.slots.iter().any(|s| !s.block.trim().is_empty());
                    let run = ui.add_enabled(
                        ok,
                        egui::Button::new(RichText::new(tr("探索する")).strong()),
                    );
                    let count = ui.add_enabled(ok, egui::Button::new(tr("組の数を数える")));
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
            if let Some((d, n)) = progress {
                ui.add(
                    egui::ProgressBar::new(if n > 0 { d as f32 / n as f32 } else { 0.0 })
                        .text(trf("組 {}/{}", &[&d, &n])),
                );
            } else if let Some(r) = p.result.clone() {
                egui::CollapsingHeader::new(RichText::new(tr("結果")).strong())
                    .id_salt("swap_result")
                    .default_open(true)
                    .show(ui, |ui| result_ui(ui, p, c, &r, &mut acts, weak));
            }
            ui.separator();
            ui.add_enabled_ui(!p.running, |ui| {
                section(ui, "swap_slots_sec", tr("差し替える玉"), |ui| {
                    slots_ui(ui, p, c, weak)
                });
                section(
                    ui,
                    "swap_vars_sec",
                    tr("組ごとに最適化する間隔"),
                    |ui| vars_ui(ui, p, c, weak),
                );
                section(ui, "swap_terms_sec", tr("評価関数に足す項"), |ui| {
                    terms_ui(ui, "swap", &mut p.terms, &mut p.new_term, weak);
                });
                section(ui, "swap_states_sec", tr("出発点・状態"), |ui| {
                    starts_states_ui(ui, p, c, weak)
                });
                section(ui, "swap_settings_sec", tr("設定"), |ui| {
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
                "差し替えられる玉がありません（カタログの玉を組合せに入れるか、id のある玉のブロックを持つプロジェクトを開いて）。",
            ))
            .small()
            .color(weak),
        );
    }
    let mut del = None;
    egui::Grid::new("swap_slots")
        .striped(true)
        .num_columns(7)
        .show(ui, |ui| {
            for h in [
                tr("玉"),
                tr("メーカー"),
                tr("焦点距離 [mm]"),
                tr("径 [mm]"),
                tr("裏返しも"),
                "",
            ] {
                ui.label(RichText::new(h).small().color(weak));
            }
            ui.end_row();
            for (i, s) in p.slots.iter_mut().enumerate() {
                let label = c
                    .blocks
                    .iter()
                    .find(|b| b.0 == s.block)
                    .map(|b| format!("{}  {}", b.0, b.1))
                    .unwrap_or_else(|| s.block.clone());
                egui::ComboBox::from_id_salt(("swap_slot", i))
                    .selected_text(label)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (id, d) in &c.blocks {
                            ui.selectable_value(&mut s.block, id.clone(), format!("{id}  {d}"));
                        }
                    });
                small_edit(ui, &mut s.vendor, 60.0, tr("すべて"));
                ui.horizontal(|ui| {
                    small_edit(ui, &mut s.efl[0], 44.0, "−∞");
                    ui.label("–");
                    small_edit(ui, &mut s.efl[1], 44.0, "∞");
                });
                ui.horizontal(|ui| {
                    small_edit(ui, &mut s.dia[0], 36.0, "0");
                    ui.label("–");
                    small_edit(ui, &mut s.dia[1], 36.0, "∞");
                });
                ui.checkbox(&mut s.flip, "")
                    .on_hover_text(tr("向きを両方試す（組の数が 2 倍）"));
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.slots.remove(i);
    }
    let free: Vec<&(String, String)> = c
        .blocks
        .iter()
        .filter(|b| !p.slots.iter().any(|s| s.block == b.0))
        .collect();
    if ui
        .add_enabled(!free.is_empty(), egui::Button::new(tr("玉を足す")))
        .clicked()
    {
        p.slots.push(SlotRow {
            block: free[0].0.clone(),
            ..Default::default()
        });
    }
    ui.label(
        RichText::new(tr(
            "候補はカタログの玉を焦点距離・径・メーカーで絞ったものです（空の欄は絞らない、焦点距離は負も可）。",
        ))
        .small()
        .color(weak),
    );
}

fn vars_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    let set = Set::Swap { gaps: &c.gaps };
    let mut del = None;
    egui::Grid::new("swap_vars")
        .striped(true)
        .num_columns(4)
        .show(ui, |ui| {
            for h in [tr("名前"), tr("下限"), tr("上限"), ""] {
                ui.label(RichText::new(h).small().color(weak));
            }
            ui.end_row();
            for (i, v) in p.vars.iter_mut().enumerate() {
                let d = c
                    .gaps
                    .iter()
                    .find(|g| format!("{}.t", g.0) == v.name)
                    .map(|g| g.1.clone())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    pp::edit_pick(ui, &mut v.name, 80.0, "d1.t", &c.sys, set)
                        .0
                        .on_hover_text(d);
                });
                small_edit(ui, &mut v.lo, 50.0, "−∞");
                small_edit(ui, &mut v.hi, 50.0, "∞");
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.vars.remove(i);
    }
    ui.horizontal(|ui| {
        let (_, enter) = pp::edit_pick(ui, &mut p.new_var, 90.0, tr("d1.t・G2.shift"), &c.sys, set);
        if (ui.button(tr("足す")).clicked() || enter) && !p.new_var.trim().is_empty() {
            p.vars.push(VarRow {
                name: p.new_var.trim().into(),
                lo: "0".into(),
                hi: String::new(),
            });
            p.new_var.clear();
        }
        ui.menu_button(tr("間隔から足す"), |ui| {
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
    if c.combo {
        ui.label(
            RichText::new(tr(
                "組合せの間隔は d0（絞りの後）・d1（1 つ目の要素の後）… です。組合せに入れられるのは間隔だけです。",
            ))
            .small()
            .color(weak),
        );
    }
}

fn starts_states_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    ui.label(tr("出発点（1 行に 1 つ）"));
    ui.add(
        egui::TextEdit::multiline(&mut p.starts)
            .desired_rows(2)
            .desired_width(260.0)
            .hint_text("d1.t=10, d2.t=5"),
    );
    ui.label(
        RichText::new(tr(
            "組ごとにすべての出発点から最適化し、評価関数の最も小さい解を採ります（書かない変数はプロジェクトの値、空なら 1 回）。",
        ))
        .small()
        .color(weak),
    );
    ui.separator();
    if c.states.is_empty() {
        ui.label(
            RichText::new(tr(
                "状態のあるプロジェクトでは、複数の状態で同時に評価できます。",
            ))
            .small()
            .color(weak),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("同時に評価する状態"));
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
            ui.label(tr("状態ごとに解く変数"));
            small_edit(ui, &mut p.state_vars, 160.0, "G2.shift, d3.t");
        });
    });
    ui.label(
        RichText::new(tr(
            "状態を選ぶと、組ごとに選んだ状態を同時に最適化します（評価関数は全状態の像の項。項の構成はこの並びの番号）。間隔の変数は全状態で共通、状態ごとに解く変数は状態ごとの値。",
        ))
        .small()
        .color(weak),
    );
}

fn settings_ui(ui: &mut Ui, p: &mut SwapPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("評価関数"));
        for (k, l) in [("spot", tr("RMS スポット")), ("wavefront", tr("RMS 波面"))] {
            ui.selectable_value(&mut p.merit, k.to_string(), l);
        }
        ui.label(tr("基準"));
        for (k, l) in [("centroid", tr("重心")), ("chief", tr("主光線"))] {
            ui.selectable_value(&mut p.reference, k.to_string(), l);
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("リング数"));
        ui.add(egui::DragValue::new(&mut p.rings).range(1..=20));
        ui.label(tr("組ごとの反復"));
        ui.add(egui::DragValue::new(&mut p.iters).range(1..=1000));
    });
    ui.horizontal(|ui| {
        ui.label(tr("焦点距離の目標"));
        small_edit(ui, &mut p.efl, 60.0, tr("なし"));
        ui.label(tr("組の上限"));
        ui.add(egui::DragValue::new(&mut p.max_count).range(1..=10_000_000));
        ui.label(tr("表の行"));
        ui.add(egui::DragValue::new(&mut p.top).range(0..=10_000))
            .on_hover_text(tr("0 はすべて"));
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut p.clear_aperture, tr("有効径の条件"))
            .on_hover_text(tr(
            "差し替える玉の各面で、各視野の瞳の縁の光線が玉の半径の内側にあることを条件にします",
        ));
        ui.separator();
        ui.label(tr("微分"));
        for (k, l) in [
            ("forward", tr("すべて前進差分")),
            ("fd", tr("すべて差分")),
            ("auto", tr("解析的（できる所）")),
        ] {
            ui.selectable_value(&mut p.jac, k.to_string(), l);
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut p.pre, tr("粗いふるい"))
            .on_hover_text(tr(
                "全組を少ないリング数で最適化し、評価関数の小さい組だけを本来の設定で最適化し直します（組が多いとき速くなる）",
            ));
        ui.add_enabled_ui(p.pre, |ui| {
            ui.label(tr("リング数"));
            ui.add(egui::DragValue::new(&mut p.pre_rings).range(1..=20));
            ui.label(tr("残す組"));
            ui.add(egui::DragValue::new(&mut p.pre_keep).range(1..=10_000_000));
        });
    });
    ui.label(
        RichText::new(tr(
            "組ごとに間隔を減衰最小二乗で最適化し、評価関数の小さい順に並べます。焦点距離の目標を入れると評価関数に足します。",
        ))
        .small()
        .color(weak),
    );
}

fn lens_text(l: &Value) -> String {
    format!(
        "{} {}{}",
        l["vendor"].as_str().unwrap_or_default(),
        l["part"].as_str().unwrap_or_default(),
        if l["reversed"].as_bool() == Some(true) {
            " ↺"
        } else {
            ""
        }
    )
}

fn result_ui(
    ui: &mut Ui,
    p: &mut SwapPanel,
    c: &Cands,
    r: &Value,
    acts: &mut Vec<Action>,
    weak: egui::Color32,
) {
    let count = r["count"].as_u64().unwrap_or(0);
    let Some(rows) = r["results"].as_array() else {
        // 数えただけ
        let per: Vec<String> = r["slots"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| s.as_array().map_or(0, Vec::len).to_string())
            .collect();
        ui.label(trf("組 {}（所ごとの候補 {}）", &[&count, &per.join(" × ")]));
        for (k, s) in r["slots"].as_array().into_iter().flatten().enumerate() {
            let names: Vec<String> = s.as_array().into_iter().flatten().map(lens_text).collect();
            let b = p.slots.get(k).map(|s| s.block.as_str()).unwrap_or("");
            egui::CollapsingHeader::new(format!("{b}: {}", names.len()))
                .id_salt(("swap_cands", k))
                .show(ui, |ui| {
                    ui.label(RichText::new(names.join("、")).small());
                });
        }
        return;
    };
    ui.label(trf(
        "組 {}（評価 {}、失敗 {}、{} 秒）{}",
        &[
            &count,
            &r["done"],
            &r["failed"],
            &fmt_num((r["seconds"].as_f64().unwrap_or(0.0) * 10.0).round() / 10.0),
            &if r["cancelled"] == json!(true) {
                tr("・止めた")
            } else {
                ""
            },
        ],
    ));
    // 焦点距離の目標があれば、評価関数と像だけの値が違う
    let with_efl = rows
        .iter()
        .any(|x| x.get("image_merit").is_some() && x["image_merit"] != x["merit"]);
    let nvars = rows
        .iter()
        .find_map(|x| x["vars"].as_array().map(Vec::len))
        .unwrap_or(0);
    egui::ScrollArea::horizontal()
        .id_salt("swap_res_h")
        .show(ui, |ui| {
            egui::Grid::new("swap_res").striped(true).show(ui, |ui| {
                let mut hs = vec![tr("順位").to_string()];
                if c.combo {
                    hs.push(String::new());
                }
                hs.push(tr("玉").into());
                hs.push(tr("評価関数").into());
                if with_efl {
                    hs.push(tr("像の評価").into());
                }
                hs.push("EFL".into());
                hs.push(tr("全長").into());
                if let Some(v) = rows.iter().find_map(|x| x["vars"].as_array()) {
                    hs.extend(v.iter().map(|x| text_of(&x["name"])));
                }
                for h in &hs {
                    ui.label(RichText::new(h).small().color(weak));
                }
                ui.end_row();
                for (i, x) in rows.iter().enumerate() {
                    let mark = if p.inserted == Some(i) { " ✔" } else { "" };
                    ui.label(format!("{}{mark}", i + 1));
                    if c.combo
                        && ui
                            .add_enabled(
                                x.get("error").is_none(),
                                egui::Button::new(tr("入れる")).small(),
                            )
                            .on_hover_text(tr("この組を組合せに入れる"))
                            .clicked()
                    {
                        acts.push(Action::Insert(i));
                    }
                    let ls: Vec<String> = x["lenses"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(lens_text)
                        .collect();
                    ui.label(ls.join(" / "));
                    if let Some(e) = x["error"].as_str() {
                        ui.label(RichText::new(e).small().color(weak));
                        ui.end_row();
                        continue;
                    }
                    let f = |k: &str| x[k].as_f64().unwrap_or(f64::NAN);
                    let short = |v: f64| format!("{v:.4}");
                    ui.monospace(format!("{:.4e}", f("merit")));
                    if with_efl {
                        ui.monospace(format!("{:.4e}", f("image_merit")));
                    }
                    ui.monospace(short(f("efl")));
                    ui.monospace(short(f("total_track")));
                    for k in 0..nvars {
                        ui.monospace(short(x["vars"][k]["x"].as_f64().unwrap_or(f64::NAN)));
                    }
                    ui.end_row();
                }
            });
        });
    ui.horizontal(|ui| {
        if c.combo {
            if ui
                .add_enabled(p.undo.is_some(), egui::Button::new(tr("入れる前に戻す")))
                .clicked()
            {
                acts.push(Action::Undo);
            }
        } else {
            ui.label(
                RichText::new(tr(
                    "組合せの系なら、選んだ組を組合せに入れられます（プロジェクトのファイルでは結果を見るだけ）。",
                ))
                .small()
                .color(weak),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::Pick;

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
        let row = json!({
            "lenses": [{"block": ids[1], "vendor": "LB", "part": "BCX-50", "reversed": true}],
            "vars": [{"name": "d1.t", "x": 7.5}, {"name": "d2.t", "x": 40.0}],
        });
        let mut d = c.clone();
        insert_row(&mut d, &ids, &row).unwrap();
        assert_eq!(d.picks[1].part, "BCX-50");
        assert!(d.picks[1].reversed);
        assert_eq!(d.picks[0].air, 7.5);
        assert_eq!(d.picks[1].air, 40.0);
        // 入れられない変数なら何も変えない
        let bad = json!({"lenses": [], "vars": [{"name": "G2.shift", "x": 1.0}]});
        let mut e = c.clone();
        assert!(insert_row(&mut e, &ids, &bad).is_err());
        assert_eq!(e.picks[0].air, 5.0);
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
        let row = json!({"lenses": [], "vars": [{"name": "d2.t", "x": 4.0}]});
        insert_row(&mut d, &ids, &row).unwrap();
        assert_eq!(d.picks[1].air, 4.0);
    }

    #[test]
    fn project_blocks() {
        let p = json!({"blocks": [
            {"type": "stop"}, {"type": "air", "t": 1},
            {"type": "lens", "id": "A", "source": {"vendor": "LB", "part": "X"}},
            {"type": "air", "id": "gap", "t": 3},
            {"type": "group", "id": "G", "blocks": [{"type": "lens", "id": "B", "source": {"zmx": "a.zmx"}}]},
        ]});
        let c = project_cands(&p);
        assert_eq!(
            c.blocks,
            vec![("A".into(), "LB X".into()), ("B".into(), String::new())]
        );
        assert_eq!(c.gaps, vec![("gap".to_string(), String::new())]);
    }
}
