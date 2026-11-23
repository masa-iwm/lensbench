//! ズーム軌跡のパネル（CLI の `zoom`、serve の `zoom`）: 状態ごとに群の移動・間隔を解き、焦点距離（か近軸倍率）の
//! 目標に合わせる。状態の並びは読み込んだ系の状態（.zmx は構成）から選び、結果は軌跡の表と図にする。

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use egui_plot::{Legend, Line, Plot, Points};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::section;
use crate::param_pick::{self as pp, Set};
use crate::theme;
use crate::widgets::sheet::{self, Cell, Sheet, SheetState};
use lensbench_engine::session::views::{ZoomPoint, ZoomView};

/// 状態の行（名前で覚える。選べる状態に無い行は出さない）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StateRow {
    pub on: bool,
    /// 目標（空は目標なし）
    pub target: String,
}

impl Default for StateRow {
    fn default() -> Self {
        StateRow {
            on: true,
            target: String::new(),
        }
    }
}

/// 解くパラメータ（範囲は空なら無し）
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
}

/// 選べる状態: (.zmx の構成の番号か, 名前)
pub(crate) type Cand = (bool, String);

pub(crate) struct ZoomPanel {
    pub states: BTreeMap<String, StateRow>,
    pub vars: Vec<VarRow>,
    /// 目標を近軸倍率にする（false は焦点距離）
    pub pmag: bool,
    pub steps: usize,
    pub merit: String,
    pub reference: String,
    pub rings: usize,
    pub iters: usize,
    pub tol: f64,
    pub running: bool,
    pub result: Option<ZoomView>,
    pub error: Option<String>,
    /// 状態・変数の表の選択と書き換え中の文字
    pub states_sheet: SheetState,
    pub vars_sheet: SheetState,
}

impl Default for ZoomPanel {
    fn default() -> Self {
        // 既定値は設定の型（CLI・serve・Python と同じ）
        let d = lensbench_design::dls_spec::ZoomParams::default();
        ZoomPanel {
            states: BTreeMap::new(),
            vars: Vec::new(),
            pmag: false,
            steps: d.steps,
            merit: d.merit.to_string(),
            reference: d.reference.to_string(),
            rings: d.rings,
            iters: d.iters,
            tol: d.tol,
            running: false,
            result: None,
            error: None,
            states_sheet: SheetState::default(),
            vars_sheet: SheetState::default(),
        }
    }
}

/// 設定に残す欄（走らせた結果・走行中は残さない）
#[derive(Serialize, Deserialize)]
struct Saved {
    states: BTreeMap<String, StateRow>,
    vars: Vec<VarRow>,
    pmag: bool,
    steps: usize,
    merit: String,
    reference: String,
    rings: usize,
    iters: usize,
    tol: f64,
}

impl ZoomPanel {
    /// 設定・プロジェクトに残す形（既定のままなら None）
    pub(crate) fn saved(&self) -> Option<Value> {
        let z = ZoomPanel::default();
        let s = |p: &ZoomPanel| Saved {
            states: p.states.clone(),
            vars: p.vars.clone(),
            pmag: p.pmag,
            steps: p.steps,
            merit: p.merit.clone(),
            reference: p.reference.clone(),
            rings: p.rings,
            iters: p.iters,
            tol: p.tol,
        };
        let (a, b) = (
            serde_json::to_value(s(self)).ok()?,
            serde_json::to_value(s(&z)).ok()?,
        );
        (a != b).then_some(a)
    }

    /// [`ZoomPanel::saved`] を戻す（読めなければ何もしない）
    pub(crate) fn restore(&mut self, v: &Value) {
        if let Ok(s) = serde_json::from_value::<Saved>(v.clone()) {
            *self = ZoomPanel {
                states: s.states,
                vars: s.vars,
                pmag: s.pmag,
                steps: s.steps,
                merit: s.merit,
                reference: s.reference,
                rings: s.rings,
                iters: s.iters,
                tol: s.tol,
                ..ZoomPanel::default()
            };
        }
    }

    /// serve の zoom の params。cands は選べる状態（並びの順に解く）
    pub(crate) fn params(&self, cands: &[Cand]) -> Result<Value, String> {
        let mut configs = Vec::new();
        let mut targets = Vec::new();
        for (is_cfg, name) in cands {
            let row = self.states.get(name).cloned().unwrap_or_default();
            if !row.on {
                continue;
            }
            configs.push(if *is_cfg {
                json!(name.parse::<u64>().unwrap_or(1))
            } else {
                json!(name)
            });
            targets.push(crate::widgets::parse_num(&row.target, name)?);
        }
        if configs.is_empty() {
            return Err(tr("Select at least one state").into());
        }
        let mut vars = Vec::new();
        for v in &self.vars {
            let n = v.name.trim();
            if n.is_empty() {
                continue;
            }
            let mut o = json!({ "name": n });
            if let Some(x) = crate::widgets::parse_num(&v.lo, n)? {
                o["lo"] = json!(x);
            }
            if let Some(x) = crate::widgets::parse_num(&v.hi, n)? {
                o["hi"] = json!(x);
            }
            vars.push(o);
        }
        if vars.is_empty() {
            return Err(tr("Add parameters to solve (G2.shift, 7.t, etc.)").into());
        }
        let mut p = json!({
            "configs": configs,
            "vars": vars,
            "steps": self.steps,
            "merit": self.merit,
            "reference": self.reference,
            "rings": self.rings,
            "iters": self.iters,
            "tol": self.tol,
        });
        if targets.iter().any(Option::is_some) {
            p[if self.pmag { "pmag" } else { "efl" }] = json!(targets);
        }
        Ok(p)
    }
}

/// パネルを描く。走らせるなら params を返す。sys は名前の欄の補完の候補（群の名前は移動の候補にも）
pub(crate) fn ui(ui: &mut Ui, p: &mut ZoomPanel, cands: &[Cand], sys: &pp::Sys) -> Option<Value> {
    let mut run = None;
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical()
        .id_salt("zoom_scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if p.running {
                    ui.spinner();
                    ui.label(tr("Solving…"));
                } else if ui
                    .button(RichText::new(tr("Solve track")).strong())
                    .clicked()
                {
                    match p.params(cands) {
                        Ok(v) => {
                            p.error = None;
                            run = Some(v);
                        }
                        Err(e) => p.error = Some(e),
                    }
                }
            });
            if let Some(e) = &p.error {
                ui.colored_label(ui.visuals().error_fg_color, lensbench_core::i18n::tr(e));
            }
            if let Some(r) = &p.result {
                section(ui, "zoom_result_sec", tr("Result"), |ui| {
                    result_ui(ui, r, weak)
                });
            }
            section(ui, "zoom_states_sec", tr("States and targets"), |ui| {
                states_ui(ui, p, cands, weak)
            });
            section(ui, "zoom_vars_sec", tr("Parameters to solve"), |ui| {
                vars_ui(ui, p, sys, weak)
            });
            section(ui, "zoom_run_sec", tr("Settings"), |ui| run_ui(ui, p, weak));
        });
    run
}

/// 解く状態の表（使う・状態・目標）。行は選べる状態で決まる（足す・消すはしない）
struct StateSheet<'a> {
    states: &'a mut BTreeMap<String, StateRow>,
    cands: &'a [Cand],
}

impl Sheet for StateSheet<'_> {
    fn rows(&self) -> usize {
        self.cands.len()
    }
    fn cols(&self) -> usize {
        3
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Use"), tr("State"), tr("Target")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [40.0, 110.0, 90.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let (is_cfg, name) = &self.cands[row];
        let st = self.states.get(name).cloned().unwrap_or_default();
        match col {
            0 => Cell::boolean(st.on),
            1 => Cell::text(if *is_cfg {
                trf("Config {}", &[name])
            } else {
                name.clone()
            })
            .ro(Some(String::new())),
            _ => Cell::num(st.target)
                .hint(tr("No target"))
                .ro((!st.on).then(|| tr("This state is not used").to_string())),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let name = &self.cands[row].1;
        let t = text.trim();
        if col == 2 {
            crate::widgets::parse_num(t, tr("Target"))?;
        }
        let st = self.states.entry(name.clone()).or_default();
        match col {
            0 => st.on = t == "true",
            2 => st.target = t.into(),
            _ => return Err(String::new()),
        }
        Ok(())
    }
}

/// 解く変数の表
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
            0 => Cell::list(v.name.clone(), self.names.clone(), true)
                .hint("G2.shift")
                .tip(pp::param_desc(&v.name)),
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
    /// 最後の空の行（左端に書くと行を足す）
    fn new_row(&self, full: bool) -> Option<Cell> {
        Some(Cell::list(
            "",
            if full { self.names.clone() } else { Vec::new() },
            true,
        ))
    }
    fn insert(&mut self, at: usize) -> bool {
        self.vars.insert(at, VarRow::default());
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

fn states_ui(ui: &mut Ui, p: &mut ZoomPanel, cands: &[Cand], weak: egui::Color32) {
    if cands.is_empty() {
        ui.label(
            RichText::new(tr(
                "Open a system with states (project states or .zmx configurations) to solve each state.",
            ))
            .small()
            .color(weak),
        );
        return;
    }
    ui.horizontal(|ui| {
        ui.label(tr("Target"));
        ui.selectable_value(&mut p.pmag, false, tr("Focal length [mm]"));
        ui.selectable_value(&mut p.pmag, true, tr("Paraxial magnification"));
    });
    let mut ss = StateSheet {
        states: &mut p.states,
        cands,
    };
    sheet::show(ui, "zoom_states", &mut p.states_sheet, &mut ss);
    ui.label(
        RichText::new(tr(
            "The selected states are solved in order. A state with an empty target is solved for the image only.",
        ))
        .small()
        .color(weak),
    );
}

fn vars_ui(ui: &mut Ui, p: &mut ZoomPanel, sys: &pp::Sys, weak: egui::Color32) {
    let units = &sys.units;
    if !p.vars.is_empty() {
        let mut vs = VarSheet {
            vars: &mut p.vars,
            names: pp::names(sys, Set::Var { combo: false }),
        };
        sheet::show(ui, "zoom_vars", &mut p.vars_sheet, &mut vs);
    }
    ui.horizontal(|ui| {
        if ui.button(tr("Add parameter")).clicked() {
            p.vars.push(VarRow::default());
        }
        if !units.is_empty() {
            ui.menu_button(tr("Add group shift"), |ui| {
                for u in units {
                    if ui.button(format!("{u}.shift")).clicked() {
                        p.vars.push(VarRow {
                            name: format!("{u}.shift"),
                            ..Default::default()
                        });
                        ui.close();
                    }
                }
            });
        }
    });
    ui.label(
        RichText::new(tr(
            "Group shifts (G2.shift), spaces after surfaces (7.t), etc. Each state is solved by damped least squares.",
        ))
        .small()
        .color(weak),
    );
}

fn run_ui(ui: &mut Ui, p: &mut ZoomPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        let lbl = ui.label(tr("Points between states"));
        ui.add(crate::widgets::Num::new(&mut p.steps).range(0..=100))
            .labelled_by(lbl.id);
        let lbl = ui.label(tr("Max iterations"));
        ui.add(crate::widgets::Num::new(&mut p.iters).range(1..=1000))
            .labelled_by(lbl.id);
    });
    ui.horizontal(|ui| {
        ui.label(tr("Image merit"));
        ui.selectable_value(&mut p.merit, "spot".into(), tr("Spot"));
        ui.selectable_value(&mut p.merit, "wavefront".into(), tr("Wavefront"));
        ui.separator();
        ui.label(tr("Reference"));
        ui.selectable_value(&mut p.reference, "centroid".into(), tr("Centroid"));
        ui.selectable_value(&mut p.reference, "chief".into(), tr("Chief ray"));
        ui.separator();
        let lbl = ui.label(tr("Pupil rings"));
        ui.add(crate::widgets::Num::new(&mut p.rings).range(1..=20))
            .labelled_by(lbl.id);
    });
    ui.horizontal(|ui| {
        let lbl = ui.label(tr("Target tolerance (relative)"));
        ui.add(
            crate::widgets::Num::new(&mut p.tol)
                .range(1e-14..=1e-2)
                .custom_formatter(|x, _| format!("{x:.0e}"))
                .custom_parser(|s| s.trim().parse().ok()),
        )
        .labelled_by(lbl.id);
    });
    ui.label(
        RichText::new(tr(
            "Points between adjacent states are traced by continuation (targets change geometrically).",
        ))
        .small()
        .color(weak),
    );
}

fn result_ui(ui: &mut Ui, r: &ZoomView, weak: egui::Color32) {
    let q = r.quantity.as_str();
    let f = |v: Option<f64>| v.filter(|x| x.is_finite()).map_or("—".into(), fmt_num);
    let key = |pt: &ZoomPoint, k: &str| pt.extra.get(k).and_then(Value::as_f64);
    egui::ScrollArea::horizontal()
        .id_salt("zoom_table")
        .show(ui, |ui| {
            egui::Grid::new("zoom_path")
                .striped(true)
                .num_columns(4 + r.vars.len())
                .show(ui, |ui| {
                    ui.strong(tr("Position"));
                    ui.strong(tr("State"));
                    ui.strong(tr("Target"));
                    ui.strong(if q == "pmag" {
                        tr("Paraxial magnification")
                    } else {
                        tr("Focal length")
                    });
                    for v in &r.vars {
                        ui.strong(v);
                    }
                    ui.strong(tr("Image merit"));
                    ui.end_row();
                    for pt in &r.path {
                        ui.label(f(Some(pt.pos)));
                        ui.label(pt.state.as_deref().unwrap_or(""));
                        ui.label(f(pt.target));
                        ui.label(f(key(pt, q)));
                        for v in &r.vars {
                            ui.label(f(key(pt, v)));
                        }
                        ui.label(f(Some(pt.merit)));
                        ui.end_row();
                    }
                });
            crate::widgets::scroll_gap(ui);
        });
    // 変数の軌跡（横軸は位置: 状態 k は k）
    let plot = Plot::new("zoom_plot")
        .height(180.0)
        .legend(Legend::default())
        .x_axis_label(tr("Position (state index, from 0)"));
    crate::plot_tools::nav(ui, plot, true, |pu| {
        for (i, v) in r.vars.iter().enumerate() {
            let pts: Vec<[f64; 2]> = r
                .path
                .iter()
                .filter_map(|pt| Some([pt.pos, key(pt, v)?]))
                .collect();
            let c = theme::series(i);
            pu.line(Line::new(v, pts.clone()).color(c));
            pu.points(Points::new("", pts).radius(3.0_f32).color(c));
        }
    });
    for w in r.warnings.iter().chain(&r.notes) {
        ui.label(
            RichText::new(lensbench_core::i18n::tr(w))
                .small()
                .color(weak),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ズームの設定は設定・プロジェクトに残る（既定のままなら残さない、結果は残さない）
    #[test]
    fn gpn14_zoom_settings_round_trip() {
        assert!(ZoomPanel::default().saved().is_none());
        let mut p = ZoomPanel::default();
        p.vars.push(VarRow {
            name: "G2.shift".into(),
            lo: "-1".into(),
            ..Default::default()
        });
        p.states.insert(
            "tele".into(),
            StateRow {
                on: false,
                target: "50".into(),
            },
        );
        p.pmag = true;
        p.result = Some(ZoomView::default());
        let v = p.saved().unwrap();
        let mut q = ZoomPanel::default();
        q.restore(&v);
        assert_eq!(
            (q.vars.clone(), q.states.clone(), q.pmag),
            (p.vars, p.states, true)
        );
        assert!(q.result.is_none());
    }

    #[test]
    fn params_follow_rows() {
        let cands: Vec<Cand> = ["wide", "mid", "tele"]
            .map(|x| (false, x.to_string()))
            .to_vec();
        let mut p = ZoomPanel::default();
        assert!(p.params(&cands).is_err());
        p.vars.push(VarRow {
            name: "G2.shift".into(),
            ..Default::default()
        });
        p.vars.push(VarRow {
            name: "G3.shift".into(),
            lo: "-10".into(),
            hi: "30".into(),
        });
        // 目標がすべて空なら efl を送らない
        let v = p.params(&cands).unwrap();
        assert_eq!(v["configs"], json!(["wide", "mid", "tele"]));
        assert!(v.get("efl").is_none());
        assert_eq!(
            v["vars"],
            json!([{ "name": "G2.shift" }, { "name": "G3.shift", "lo": -10.0, "hi": 30.0 }])
        );
        p.states.insert(
            "wide".into(),
            StateRow {
                on: true,
                target: "140".into(),
            },
        );
        p.states.insert(
            "mid".into(),
            StateRow {
                on: false,
                target: "160".into(),
            },
        );
        p.pmag = true;
        let v = p.params(&cands).unwrap();
        assert_eq!(v["configs"], json!(["wide", "tele"]));
        assert_eq!(v["pmag"], json!([140.0, null]));
        // .zmx の構成は番号で
        let v = p.params(&[(true, "2".into())]).unwrap();
        assert_eq!(v["configs"], json!([2]));
        p.states.insert(
            "2".into(),
            StateRow {
                on: true,
                target: "x".into(),
            },
        );
        assert!(p.params(&[(true, "2".into())]).is_err());
    }
}
