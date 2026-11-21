//! 差し替え探索の「出発点と状態」: 出発点の表（行 = 出発点、列 = 変数。空のセルはプロジェクトの値）、
//! 同時に評価する状態、状態ごとに解く変数の表（名前は補完つき）。系に無い名前・変数でない名前は、
//! 走らせる前に表の下に出す

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use egui::{RichText, Ui};
use serde_json::json;

use super::{num_text, Cands, SwapPanel, VarRow, VarSheet};
use crate::i18n::{tr, trf};
use crate::param_pick::{self as pp, Set};
use crate::widgets::sheet::{self, Cell, Sheet};

/// 1 つの出発点（変数の名前 → 初期値。書かない変数はプロジェクトの値）
pub(crate) type Start = BTreeMap<String, f64>;

/// 出発点の表の列: 変数、状態ごとの変数（状態を選んだとき）、そのどちらでもないのに出発点に値がある名前。
/// 最後のものは false（そのまま走らせると誤り）
pub(super) fn columns(p: &SwapPanel) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let per_state: &[VarRow] = if p.states.is_empty() {
        &[]
    } else {
        &p.state_vars
    };
    for v in p.vars.iter().chain(per_state) {
        let n = v.name.trim();
        if !n.is_empty() && !out.iter().any(|c| c.0 == n) {
            out.push((n.to_string(), true));
        }
    }
    for st in &p.starts {
        for k in st.keys() {
            if !out.iter().any(|c| &c.0 == k) {
                out.push((k.clone(), false));
            }
        }
    }
    out
}

/// 出発点の表
struct StartSheet<'a> {
    rows: &'a mut Vec<Start>,
    cols: Vec<(String, bool)>,
    base: &'a [(String, f64)],
}

impl StartSheet<'_> {
    fn base(&self, name: &str) -> Option<f64> {
        self.base.iter().find(|b| b.0 == name).map(|b| b.1)
    }
}

impl Sheet for StartSheet<'_> {
    fn rows(&self) -> usize {
        self.rows.len()
    }
    fn cols(&self) -> usize {
        self.cols.len()
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        let (n, ok) = &self.cols[col];
        if *ok {
            n.clone()
        } else {
            format!("⚠ {n}")
        }
    }
    fn header_tip(&self, col: usize, _cur: Option<usize>) -> String {
        let (n, ok) = &self.cols[col];
        if *ok {
            pp::param_desc(n)
        } else {
            tr("Not a variable of the search (running fails). Add it to the variables or clear the column.").into()
        }
    }
    fn width(&self, _col: usize) -> f32 {
        64.0
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let n = &self.cols[col].0;
        let t = self.rows[row]
            .get(n)
            .map(|x| num_text(&json!(x)))
            .unwrap_or_default();
        // 空のセルはプロジェクトの値から（分かれば薄く出す）
        let hint = match self.base(n) {
            Some(x) => num_text(&json!(x)),
            None => tr("project").into(),
        };
        Cell::num(t).hint(hint)
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let n = self.cols[col].0.clone();
        match crate::widgets::parse_num(text, &n)? {
            Some(x) => {
                self.rows[row].insert(n, x);
            }
            None => {
                self.rows[row].remove(&n);
            }
        }
        Ok(())
    }
    /// 最後の空の行（左端に数を書くと出発点を足す）
    fn new_row(&self, _full: bool) -> Option<Cell> {
        Some(Cell::num(""))
    }
    fn insert(&mut self, at: usize) -> bool {
        self.rows.insert(at, Start::new());
        true
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        self.rows.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.rows.swap(a, a + 1);
        true
    }
}

fn warn(ui: &mut Ui, t: String) {
    let c = ui.visuals().warn_fg_color;
    ui.label(RichText::new(t).small().color(c));
}

/// 変数の表で、系の名前の候補に無い名前（打ち間違いなど）を表の下に出す
pub(super) fn unknown_note(ui: &mut Ui, rows: &[VarRow], names: &[(String, String)]) {
    if names.is_empty() {
        return;
    }
    let bad: Vec<&str> = rows
        .iter()
        .map(|v| v.name.trim())
        .filter(|n| !n.is_empty() && !names.iter().any(|c| c.0 == *n))
        .collect();
    if !bad.is_empty() {
        warn(
            ui,
            trf("Not among the names of this system: {}", &[&bad.join(", ")]),
        );
    }
}

pub(super) fn ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands, weak: egui::Color32) {
    ui.label(tr("Starting points"));
    let cols = columns(p);
    if cols.is_empty() {
        ui.label(
            RichText::new(tr("Add variables to set starting points."))
                .small()
                .color(weak),
        );
    } else {
        let mut sh = StartSheet {
            rows: &mut p.starts,
            cols: cols.clone(),
            base: &c.base,
        };
        sheet::show(ui, "swap_starts", &mut p.starts_sheet, &mut sh);
        let bad: Vec<&str> = cols.iter().filter(|c| !c.1).map(|c| c.0.as_str()).collect();
        if !bad.is_empty() {
            warn(
                ui,
                trf(
                    "Starting points have values for names that are not variables: {}",
                    &[&bad.join(", ")],
                ),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Add start")).clicked() {
                p.starts.push(Start::new());
            }
            if ui
                .button(tr("Add project values"))
                .on_hover_text(tr(
                    "Add a start filled with the project values of the variables",
                ))
                .clicked()
            {
                let row = cols
                    .iter()
                    .filter(|x| x.1)
                    .filter_map(|(n, _)| c.base.iter().find(|b| &b.0 == n).cloned())
                    .collect();
                p.starts.push(row);
            }
            if !bad.is_empty()
                && ui
                    .button(tr("Clear the columns that are not variables"))
                    .clicked()
            {
                for st in &mut p.starts {
                    st.retain(|k, _| !bad.contains(&k.as_str()));
                }
            }
        });
    }
    ui.label(
        RichText::new(tr(
            "Each combination is optimized from every starting point and the best solution is kept (empty cells use the project values; no rows means one run).",
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
    ui.add_enabled_ui(!p.states.is_empty(), |ui| state_vars_ui(ui, p, c));
    ui.label(
        RichText::new(tr(
            "With states selected, each combination is optimized over all of them together (the merit is the image terms of every state; term configs are indices in this list). Space variables are shared; per-state variables get a value for each state.",
        ))
        .small()
        .color(weak),
    );
}

/// 状態ごとに解く変数の表と、名前の補完つきの欄
fn state_vars_ui(ui: &mut Ui, p: &mut SwapPanel, c: &Cands) {
    ui.label(tr("Per-state variables"));
    let set = Set::Swap { gaps: &c.gaps };
    let all = pp::names(&c.sys, set);
    if !p.state_vars.is_empty() {
        // 候補は共通の変数に選んでいない名前
        let names = all
            .iter()
            .filter(|n| !p.vars.iter().any(|v| v.name.trim() == n.0))
            .cloned()
            .collect();
        let mut vs = VarSheet {
            vars: &mut p.state_vars,
            names,
            lo0: "",
        };
        sheet::show(ui, "swap_state_vars", &mut p.state_vars_sheet, &mut vs);
        unknown_note(ui, &p.state_vars, &all);
        let both: Vec<&str> = p
            .state_vars
            .iter()
            .map(|v| v.name.trim())
            .filter(|n| !n.is_empty() && p.vars.iter().any(|v| v.name.trim() == *n))
            .collect();
        if !both.is_empty() {
            warn(
                ui,
                trf(
                    "Both a shared and a per-state variable: {}",
                    &[&both.join(", ")],
                ),
            );
        }
    }
    ui.horizontal_wrapped(|ui| {
        let (_, enter) = pp::edit_pick(ui, &mut p.new_state_var, 90.0, "G2.shift", &c.sys, set);
        let name = p.new_state_var.trim();
        if (ui.button(tr("Add per-state variable")).clicked() || enter) && !name.is_empty() {
            p.state_vars.push(VarRow {
                name: name.into(),
                ..Default::default()
            });
            p.new_state_var.clear();
        }
    });
}
