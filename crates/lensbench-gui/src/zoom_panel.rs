//! ズーム軌跡のパネル（CLI の `zoom`、serve の `zoom`）: 状態ごとに群の移動・間隔を解き、焦点距離（か近軸倍率）の
//! 目標に合わせる。状態の並びは読み込んだ系の状態（.zmx は構成）から選び、結果は軌跡の表と図にする。

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use egui_plot::{Legend, Line, Plot, Points};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::{section, small_edit};
use crate::theme;

/// 状態の行（名前で覚える。選べる状態に無い行は出さない）
#[derive(Clone, Debug, PartialEq)]
pub struct StateRow {
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
}

/// 選べる状態: (.zmx の構成の番号か, 名前)
pub type Cand = (bool, String);

pub struct ZoomPanel {
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
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl Default for ZoomPanel {
    fn default() -> Self {
        ZoomPanel {
            states: BTreeMap::new(),
            vars: Vec::new(),
            pmag: false,
            steps: 0,
            merit: "spot".into(),
            reference: "centroid".into(),
            rings: 3,
            iters: 50,
            tol: 1e-8,
            running: false,
            result: None,
            error: None,
        }
    }
}

fn num(t: &str, what: &str) -> Result<Option<f64>, String> {
    let t = t.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .map(Some)
        .ok_or_else(|| trf("{} の値 {} が数でない", &[&what, &t]))
}

impl ZoomPanel {
    /// serve の zoom の params。cands は選べる状態（並びの順に解く）
    pub fn params(&self, cands: &[Cand]) -> Result<Value, String> {
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
            targets.push(num(&row.target, name)?);
        }
        if configs.is_empty() {
            return Err(tr("状態を 1 つ以上選んでください").into());
        }
        let mut vars = Vec::new();
        for v in &self.vars {
            let n = v.name.trim();
            if n.is_empty() {
                continue;
            }
            let mut o = json!({ "name": n });
            if let Some(x) = num(&v.lo, n)? {
                o["lo"] = json!(x);
            }
            if let Some(x) = num(&v.hi, n)? {
                o["hi"] = json!(x);
            }
            vars.push(o);
        }
        if vars.is_empty() {
            return Err(tr("解くパラメータ（G2.shift・7.t など）を足してください").into());
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

/// パネルを描く。走らせるなら params を返す。units は群の名前（移動の候補）
pub fn ui(ui: &mut Ui, p: &mut ZoomPanel, cands: &[Cand], units: &[String]) -> Option<Value> {
    let mut run = None;
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical()
        .id_salt("zoom_scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if p.running {
                    ui.spinner();
                    ui.label(tr("解いています…"));
                } else if ui
                    .button(RichText::new(tr("軌跡を解く")).strong())
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
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            if let Some(r) = &p.result {
                section(ui, "zoom_result_sec", tr("結果"), |ui| {
                    result_ui(ui, r, weak)
                });
            }
            section(ui, "zoom_states_sec", tr("状態と目標"), |ui| {
                states_ui(ui, p, cands, weak)
            });
            section(ui, "zoom_vars_sec", tr("解くパラメータ"), |ui| {
                vars_ui(ui, p, units, weak)
            });
            section(ui, "zoom_run_sec", tr("設定"), |ui| run_ui(ui, p, weak));
        });
    run
}

fn states_ui(ui: &mut Ui, p: &mut ZoomPanel, cands: &[Cand], weak: egui::Color32) {
    if cands.is_empty() {
        ui.label(
            RichText::new(tr(
                "状態（プロジェクトの states か .zmx の多重構成）のある系を開くと、状態ごとに解けます。",
            ))
            .small()
            .color(weak),
        );
        return;
    }
    ui.horizontal(|ui| {
        ui.label(tr("目標"));
        ui.selectable_value(&mut p.pmag, false, tr("焦点距離 [mm]"));
        ui.selectable_value(&mut p.pmag, true, tr("近軸倍率"));
    });
    egui::Grid::new("zoom_states")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (is_cfg, name) in cands {
                let row = p.states.entry(name.clone()).or_default();
                let label = if *is_cfg {
                    trf("構成 {}", &[name])
                } else {
                    name.clone()
                };
                ui.checkbox(&mut row.on, label);
                ui.add_enabled_ui(row.on, |ui| {
                    small_edit(ui, &mut row.target, 80.0, tr("目標なし"));
                });
                ui.end_row();
            }
        });
    ui.label(
        RichText::new(tr(
            "選んだ状態を並びの順に解きます。目標が空の状態は像だけで解きます（目標がすべて空なら像だけ）。",
        ))
        .small()
        .color(weak),
    );
}

fn vars_ui(ui: &mut Ui, p: &mut ZoomPanel, units: &[String], weak: egui::Color32) {
    let mut del = None;
    egui::Grid::new("zoom_vars")
        .striped(true)
        .num_columns(4)
        .show(ui, |ui| {
            for (i, v) in p.vars.iter_mut().enumerate() {
                small_edit(ui, &mut v.name, 90.0, "G2.shift");
                small_edit(ui, &mut v.lo, 60.0, tr("下限"));
                small_edit(ui, &mut v.hi, 60.0, tr("上限"));
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
        if ui.button(tr("パラメータを足す")).clicked() {
            p.vars.push(VarRow::default());
        }
        if !units.is_empty() {
            ui.menu_button(tr("群の移動を足す"), |ui| {
                for u in units {
                    if ui.button(format!("{u}.shift")).clicked() {
                        p.vars.push(VarRow {
                            name: format!("{u}.shift"),
                            ..Default::default()
                        });
                        ui.close_menu();
                    }
                }
            });
        }
    });
    ui.label(
        RichText::new(tr(
            "群の移動（G2.shift）や面の後の間隔（7.t）など。状態ごとに減衰最小二乗で解きます。",
        ))
        .small()
        .color(weak),
    );
}

fn run_ui(ui: &mut Ui, p: &mut ZoomPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("間の点の数"));
        ui.add(egui::DragValue::new(&mut p.steps).range(0..=100));
        ui.label(tr("反復の上限"));
        ui.add(egui::DragValue::new(&mut p.iters).range(1..=1000));
    });
    ui.horizontal(|ui| {
        ui.label(tr("像の評価"));
        ui.selectable_value(&mut p.merit, "spot".into(), tr("スポット"));
        ui.selectable_value(&mut p.merit, "wavefront".into(), tr("波面"));
        ui.separator();
        ui.label(tr("基準"));
        ui.selectable_value(&mut p.reference, "centroid".into(), tr("重心"));
        ui.selectable_value(&mut p.reference, "chief".into(), tr("主光線"));
        ui.separator();
        ui.label(tr("瞳の輪"));
        ui.add(egui::DragValue::new(&mut p.rings).range(1..=20));
    });
    ui.horizontal(|ui| {
        ui.label(tr("目標との差の許し（比）"));
        ui.add(
            egui::DragValue::new(&mut p.tol)
                .range(1e-14..=1e-2)
                .speed(0.0)
                .custom_formatter(|x, _| format!("{x:.0e}"))
                .custom_parser(|s| s.trim().parse().ok()),
        );
    });
    ui.label(
        RichText::new(tr(
            "間の点は隣り合う状態の間を継続法でたどります（目標は等比に）。",
        ))
        .small()
        .color(weak),
    );
}

fn result_ui(ui: &mut Ui, r: &Value, weak: egui::Color32) {
    let q = r["quantity"].as_str().unwrap_or("efl");
    let vars: Vec<String> = r["vars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let path = r["path"].as_array().cloned().unwrap_or_default();
    let f = |v: &Value| v.as_f64().map_or("—".into(), fmt_num);
    egui::ScrollArea::horizontal()
        .id_salt("zoom_table")
        .show(ui, |ui| {
            egui::Grid::new("zoom_path")
                .striped(true)
                .num_columns(4 + vars.len())
                .show(ui, |ui| {
                    ui.strong(tr("位置"));
                    ui.strong(tr("状態"));
                    ui.strong(tr("目標"));
                    ui.strong(if q == "pmag" {
                        tr("近軸倍率")
                    } else {
                        tr("焦点距離")
                    });
                    for v in &vars {
                        ui.strong(v);
                    }
                    ui.strong(tr("像の評価"));
                    ui.end_row();
                    for pt in &path {
                        ui.label(f(&pt["pos"]));
                        ui.label(pt["state"].as_str().unwrap_or(""));
                        ui.label(f(&pt["target"]));
                        ui.label(f(&pt[q]));
                        for v in &vars {
                            ui.label(f(&pt[v.as_str()]));
                        }
                        ui.label(f(&pt["merit"]));
                        ui.end_row();
                    }
                });
        });
    // 変数の軌跡（横軸は位置: 状態 k は k）
    Plot::new("zoom_plot")
        .height(180.0)
        .legend(Legend::default())
        .x_axis_label(tr("位置（状態の番号、0 始まり）"))
        .show(ui, |pu| {
            for (i, v) in vars.iter().enumerate() {
                let pts: Vec<[f64; 2]> = path
                    .iter()
                    .filter_map(|pt| Some([pt["pos"].as_f64()?, pt[v.as_str()].as_f64()?]))
                    .collect();
                let c = theme::series(i);
                pu.line(Line::new(pts.clone()).name(v).color(c));
                pu.points(Points::new(pts).radius(3.0_f32).color(c));
            }
        });
    for w in r["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(r["notes"].as_array().into_iter().flatten())
    {
        ui.label(
            RichText::new(w.as_str().unwrap_or_default())
                .small()
                .color(weak),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
