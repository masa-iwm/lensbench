//! 最適化のパネルの大域の探索とガラスの置き換え: 設定の欄、設定の JSON、結果の表、システムに当てる材料

use egui::{RichText, Ui};
use serde_json::{json, Value};

use super::{int_list, small_edit, OptPanel};
use crate::i18n::{tr, trf};
use lensbench_design::glass_sub::{RESIST_NAMES, STATUS_NAMES};
use lensbench_engine::session::views::OptimizeView;

/// ガラスの置き換えの設定（する設定で面があるとき）
pub(super) fn glass_params(p: &OptPanel) -> Result<Option<Value>, String> {
    if !p.glass_on {
        return Ok(None);
    }
    let surfaces =
        int_list(&p.glass_surfaces).map_err(|e| trf("Glass substitution surfaces: {}", &[&e]))?;
    if surfaces.is_empty() {
        return Err(tr("List the surfaces for glass substitution (e.g. 1, 3, 6)").to_string());
    }
    let catalogs: Vec<&str> = p
        .glass_catalogs
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    let mut o = json!({ "surfaces": surfaces });
    if !catalogs.is_empty() {
        o["catalogs"] = json!(catalogs);
    }
    let g = lensbench_design::dls_spec::GlassParams::default();
    if p.glass_candidates != g.candidates {
        o["candidates"] = json!(p.glass_candidates);
    }
    if p.glass_passes != g.passes {
        o["passes"] = json!(p.glass_passes);
    }
    let num = |label: &str, t: &str| -> Result<Option<f64>, String> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .map(Some)
            .ok_or_else(|| trf("{}: not a number ({})", &[&label, &t]))
    };
    if let Some(x) = num(tr("Max relative cost"), &p.glass_max_cost)? {
        o["max_cost"] = json!(x);
    }
    for (k, t) in RESIST_NAMES.iter().zip(&p.glass_max_resist) {
        if let Some(x) = num(&k.to_ascii_uppercase(), t)? {
            o[format!("max_{k}")] = json!(x);
        }
    }
    if p.glass_status.iter().any(|x| *x) {
        let st: Vec<&str> = STATUS_NAMES
            .iter()
            .zip(&p.glass_status)
            .filter(|(_, on)| **on)
            .map(|(n, _)| *n)
            .collect();
        o["status"] = json!(st);
    }
    let ex: Vec<&str> = p
        .glass_exclude
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    if !ex.is_empty() {
        o["exclude"] = json!(ex);
    }
    if p.glass_exclude_incomplete {
        o["exclude_incomplete"] = json!(true);
    }
    if p.glass_nd_step != g.nd_step {
        o["nd_step"] = json!(p.glass_nd_step);
    }
    if p.glass_vd_step != g.vd_step {
        o["vd_step"] = json!(p.glass_vd_step);
    }
    if let Some(x) = num(tr("ΔPgF step"), &p.glass_dpgf_step)? {
        o["dpgf_step"] = json!(x);
    }
    Ok(Some(o))
}

/// 設定の JSON の glass から絞り込みと近さの欄を読む
pub(super) fn load_glass_filter(n: &mut OptPanel, g: &Value) {
    let txt = |v: &Value| v.as_f64().map(|x| x.to_string()).unwrap_or_default();
    n.glass_max_cost = txt(&g["max_cost"]);
    for (k, t) in RESIST_NAMES.iter().zip(n.glass_max_resist.iter_mut()) {
        *t = txt(&g[format!("max_{k}")]);
    }
    let st: Vec<String> = g["status"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|x| x.trim().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    for (on, name) in n.glass_status.iter_mut().zip(STATUS_NAMES) {
        *on = st.iter().any(|x| x == name);
    }
    n.glass_exclude = g["exclude"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    n.glass_exclude_incomplete = g["exclude_incomplete"].as_bool().unwrap_or(false);
    if let Some(x) = g["nd_step"].as_f64() {
        n.glass_nd_step = x;
    }
    if let Some(x) = g["vd_step"].as_f64() {
        n.glass_vd_step = x;
    }
    n.glass_dpgf_step = txt(&g["dpgf_step"]);
}

pub(super) fn search_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        let lbl = ui.label(tr("Start points"))
            .on_hover_text(tr("Global search: optimize from this many start points (the current values first) in parallel and keep the best solutions. Variables with both bounds are drawn in their range, others around their value. 0 or 1: local optimization only"));
        ui.add(crate::widgets::Num::new(&mut p.starts).range(0..=10000)).labelled_by(lbl.id);
        ui.add_enabled_ui(p.starts >= 2, |ui| {
            let lbl = ui.label(tr("Random seed"));
            ui.add(crate::widgets::Num::new(&mut p.seed)).labelled_by(lbl.id);
            let lbl = ui.label(tr("Solutions to keep"))
                .on_hover_text(tr("Number of distinct solutions to keep (best first)"));
            ui.add(crate::widgets::Num::new(&mut p.keep).range(1..=100)).labelled_by(lbl.id);
            let lbl = ui.label(tr("Spread of unbounded variables"))
                .on_hover_text(tr("Spread of variables without bounds, relative to max(|value|, typical size of the kind)"));
            ui.add(
                crate::widgets::Num::new(&mut p.spread)
                    .range(1e-6..=100.0)
                    .max_decimals(4),
            ).labelled_by(lbl.id);
        });
    });
    ui.checkbox(&mut p.glass_on, tr("Substitute glasses after optimizing"))
        .on_hover_text(tr("Choose the glass of the listed surfaces from catalog glasses: for each surface, the candidates nearest in nd and vd to its current medium are tried with the variables re-optimized. Model glasses and surfaces with dn/disp are always replaced by a real glass; real glasses only when the merit improves"));
    ui.add_enabled_ui(p.glass_on, |ui| {
        ui.horizontal_wrapped(|ui| {
            let lbl = ui.label(tr("Surfaces to substitute"));
            small_edit(ui, &mut p.glass_surfaces, 120.0, "1, 3, 6").labelled_by(lbl.id);
            let lbl = ui.label(tr("Glass catalogs"));
            small_edit(ui, &mut p.glass_catalogs, 160.0, tr("System glasses")).labelled_by(lbl.id);
        });
        ui.horizontal_wrapped(|ui| {
            let lbl = ui.label(tr("Candidates per surface"));
            ui.add(crate::widgets::Num::new(&mut p.glass_candidates).range(1..=200)).labelled_by(lbl.id);
            let lbl = ui.label(tr("Max passes"));
            ui.add(crate::widgets::Num::new(&mut p.glass_passes).range(1..=20)).labelled_by(lbl.id);
        });
        ui.horizontal_wrapped(|ui| {
            let lbl = ui.label(tr("Max relative cost"))
                .on_hover_text(tr("Candidates: upper limits from the catalog (relative cost and resistance classes). Empty: no limit. With no filter at all, the system's glass substitution template (GSTD of a .zmx) is used when it is on. Glasses without the value are kept unless excluded"));
            small_edit(ui, &mut p.glass_max_cost, 50.0, "").labelled_by(lbl.id);
            for (k, t) in RESIST_NAMES.iter().zip(p.glass_max_resist.iter_mut()) {
                let lbl = ui.label(k.to_ascii_uppercase());
                small_edit(ui, t, 36.0, "").labelled_by(lbl.id);
            }
            ui.checkbox(&mut p.glass_exclude_incomplete, tr("Exclude glasses without the value"))
                .on_hover_text(tr("Drop candidates that have no value for a property with a limit (a relative cost or class of 0 counts as no value, except FR)"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Catalog status"))
                .on_hover_text(tr("Candidates: only glasses of the checked statuses. None checked: all. Glasses without a status are kept"));
            let labels = [tr("Standard"), tr("Preferred"), tr("Obsolete"), tr("Special"), tr("Melt")];
            for (on, l) in p.glass_status.iter_mut().zip(labels) {
                ui.checkbox(on, l);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let lbl = ui.label(tr("Excluded glasses"))
                .on_hover_text(tr("Glasses never used as candidates (comma-separated; NAME@CATALOG for one catalog's glass). A current glass that fails the filter is always replaced"));
            small_edit(ui, &mut p.glass_exclude, 200.0, "N-LASF9, F2@OHARA").labelled_by(lbl.id);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Closeness steps"))
                .on_hover_text(tr("Candidates are the nearest in (Δnd / nd step)² + (Δvd / vd step)² (+ (ΔPgF / ΔPgF step)² when given)"));
            let lbl = ui.label("nd");
            ui.add(crate::widgets::Num::new(&mut p.glass_nd_step).range(1e-6..=1.0).max_decimals(5)).labelled_by(lbl.id);
            let lbl = ui.label("vd");
            ui.add(crate::widgets::Num::new(&mut p.glass_vd_step).range(1e-6..=1000.0).max_decimals(4)).labelled_by(lbl.id);
            let lbl = ui.label(tr("ΔPgF step"));
            small_edit(ui, &mut p.glass_dpgf_step, 60.0, tr("None")).labelled_by(lbl.id);
        });
        ui.label(
            RichText::new(tr(
                "To apply a substituted glass, its catalog must be one of the system's glass catalogs.",
            ))
            .small()
            .color(weak),
        );
    });
}

/// 結果の大域の探索（残した解）とガラスの置き換え（置き換えの表）。v は評価関数の値の書き方
pub(super) fn result_ui(
    ui: &mut Ui,
    r: &OptimizeView,
    v: &dyn Fn(f64) -> String,
    weak: egui::Color32,
) {
    if let Some(g) = &r.global_search {
        ui.label(trf(
            "Global search: {} start points ({} could not be traced), best from start {}",
            &[&g.ran, &g.failed, &g.best_start],
        ));
        if g.skipped > 0 {
            ui.label(
                RichText::new(trf("{} start points were not run (stopped)", &[&g.skipped]))
                    .small()
                    .color(weak),
            );
        }
        egui::Grid::new("opt_res_global")
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new(tr("Solution")).small().color(weak));
                ui.label(RichText::new(tr("Start point")).small().color(weak));
                ui.label(RichText::new(tr("Merit function")).small().color(weak));
                ui.end_row();
                for (i, s) in g.solutions.iter().enumerate() {
                    ui.label(format!("{}", i + 1));
                    ui.monospace(format!("{}", s.start));
                    ui.monospace(v(s.merit));
                    ui.end_row();
                }
            });
    }
    if let Some(g) = &r.glass {
        ui.label(trf(
            "Glass substitution: merit {} → {} ({} candidates tried)",
            &[&v(g.merit_before), &v(r.merit), &g.tried],
        ));
        if g.changes.is_empty() {
            ui.label(
                RichText::new(tr("No glass was changed (none improved the merit)"))
                    .small()
                    .color(weak),
            );
        } else {
            egui::Grid::new("opt_res_glass")
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        tr("Surface"),
                        tr("Before"),
                        tr("After"),
                        "nd",
                        "vd",
                        tr("Merit function"),
                    ] {
                        ui.label(RichText::new(h).small().color(weak));
                    }
                    ui.end_row();
                    for c in &g.changes {
                        match &c.configs {
                            Some(k) => ui.label(trf(
                                "{} (configurations {})",
                                &[
                                    &c.surface,
                                    &k.iter()
                                        .map(|x| x.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                ],
                            )),
                            None => ui.label(format!("{}", c.surface)),
                        };
                        ui.monospace(&c.from);
                        ui.monospace(&c.to);
                        ui.monospace(format!("{:.5}", c.nd));
                        ui.monospace(format!("{:.2}", c.vd));
                        ui.monospace(v(c.merit));
                        ui.end_row();
                    }
                });
        }
    }
}

/// ガラスの置き換えで変わった 1 つの面の、問題の構成ごとの材料
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlassApply {
    pub surface: usize,
    /// 問題の構成 i（0 から）の (置き換える前, 最後) の材料。その構成を置き換えなかったら None
    pub per: Vec<Option<(String, String)>>,
}

/// 結果のガラスの置き換えで変わった面の、問題の構成（n 個、0 は 1 個とみなす）ごとの材料。置き換えた順
pub(crate) fn result_glass_configs(r: &OptimizeView, n: usize) -> Vec<GlassApply> {
    let Some(g) = &r.glass else {
        return Vec::new();
    };
    let n = n.max(1);
    // 構成 i（0 から）に効く置き換え
    let hits = |c: &lensbench_engine::session::views::GlassChangeView, i: usize| {
        c.configs.as_ref().is_none_or(|k| k.contains(&(i + 1)))
    };
    let mut out: Vec<GlassApply> = Vec::new();
    for c in &g.changes {
        if out.iter().any(|a| a.surface == c.surface) {
            continue;
        }
        let j = c.surface;
        let per = (0..n)
            .map(|i| {
                let first = g.changes.iter().find(|c| c.surface == j && hits(c, i))?;
                let last = g
                    .config_materials
                    .iter()
                    .find(|(k, _)| *k == j)
                    .and_then(|(_, v)| v.get(i))
                    .or_else(|| g.materials.iter().find(|(k, _)| *k == j).map(|x| &x.1))
                    .unwrap_or(&first.to);
                Some((first.from.clone(), last.clone()))
            })
            .collect();
        out.push(GlassApply { surface: j, per });
    }
    out
}

/// ガラスの置き換えを当てる前の材料（元に戻す）。状態の上書きを先に、面の材料を後に当てた順
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GlassUndo {
    /// 系の面 j の材料（要素の面の材料）
    Base(usize, String),
    /// 要素 loc の面 face（座標ブレークを除いた番号）の状態 state の材料の上書き。None は上書きが無かった
    State {
        loc: crate::combo_tree::Loc,
        face: usize,
        state: String,
        old: Option<Value>,
    },
}

/// ガラスの置き換えで 0 にした、変数でない dn・disp の値 (名前, 値)。before なら置き換える前の値、でなければ 0
pub(crate) fn result_resets(r: &OptimizeView, before: bool) -> Vec<(String, f64)> {
    r.glass
        .iter()
        .flat_map(|g| &g.reset)
        .map(|(n, x)| (n.clone(), if before { *x } else { 0.0 }))
        .collect()
}

/// 要素の 1 つの面への、ガラスの置き換えの書き方（[`plan_glass`]）
#[derive(Debug, Default, PartialEq)]
pub(crate) struct GlassPlan {
    /// 要素の面の材料（基本の系）の (前, 後)。None は面の材料を変えない
    pub base: Option<(String, String)>,
    /// 状態の材料の上書き (状態, 材料)
    pub writes: Vec<(String, String)>,
    /// 基本の系にも書く設定なのに、状態の上書きだけにした（面の材料に従う状態の前の硝材が分からない）
    pub base_kept: bool,
}

/// 状態のある組合せの要素の面への置き換えの書き方。users は面の材料に従う（その面の材料の上書きの無い）状態と
/// その .zmx の構成の番号（状態の並びの順）、hits は置き換えた状態の (状態, 前, 後)、base_cfg は基本の系の構成の番号。
/// states_only なら置き換えた状態の上書きだけを書く。そうでなければ基本の系も置き換える: 面の材料を基本の構成の
/// 状態（面の材料に従う最初のもの）の新しい硝材にし、ほかの状態は上書きで今の（置き換えたならその）硝材のままにする
pub(crate) fn plan_glass(
    users: &[(String, usize)],
    hits: &[(String, String, String)],
    base_cfg: usize,
    states_only: bool,
) -> GlassPlan {
    let hit = |n: &str| hits.iter().find(|h| h.0 == n);
    let only_own = |base_kept| GlassPlan {
        base: None,
        writes: hits.iter().map(|h| (h.0.clone(), h.2.clone())).collect(),
        base_kept,
    };
    if states_only {
        return only_own(false);
    }
    // 基本の構成で面の材料に従う状態（無ければ、どの状態も上書きを持つので面の材料は変えない）。置き換えて
    // いなければ基本の系もそのまま
    let Some(b) = users.iter().find(|u| u.1 == base_cfg) else {
        return only_own(false);
    };
    let Some((_, from, to)) = hit(&b.0) else {
        return only_own(false);
    };
    let mut writes = Vec::new();
    for (n, cfg) in users {
        let m = match hit(n) {
            Some(h) => h.2.clone(),
            // 置き換えなかった状態は前の硝材で上書き（同じ構成の、置き換えた状態の前の硝材）
            None => match users
                .iter()
                .find(|u| u.1 == *cfg && hit(&u.0).is_some())
                .and_then(|u| hit(&u.0))
            {
                Some(h) => h.1.clone(),
                None => return only_own(true),
            },
        };
        if m != *to {
            writes.push((n.clone(), m));
        }
    }
    // 面の材料の上書きを持つ状態は、置き換えたらその上書きを
    for (n, _, m) in hits {
        if !users.iter().any(|u| u.0 == *n) {
            writes.push((n.clone(), m.clone()));
        }
    }
    GlassPlan {
        base: Some((from.clone(), to.clone())),
        writes,
        base_kept: false,
    }
}

/// 結果の「システムに反映」「反映を元に戻す」と、ガラスの置き換えを状態だけに書くかのチェック
pub(super) fn apply_ui(
    ui: &mut Ui,
    p: &mut OptPanel,
    r: &OptimizeView,
    acts: &mut Vec<super::Action>,
) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("Apply to system"))
            .on_hover_text(tr(
                "Apply the optimized values to the open system (combination)",
            ))
            .clicked()
        {
            acts.push(super::Action::Apply);
        }
        if ui
            .add_enabled(
                p.undo.is_some() || p.undo_glass.is_some(),
                egui::Button::new(tr("Undo apply")),
            )
            .clicked()
        {
            acts.push(super::Action::Undo);
        }
        // ガラスを置き換えたときだけ（状態の無い組合せでは効かない）
        if r.glass.as_ref().is_some_and(|g| !g.changes.is_empty()) {
            ui.checkbox(&mut p.glass_states_only, tr("Glass only in the states"))
                .on_hover_text(tr("Writes the substituted glass only as overrides of the states, leaving the base system (no state open) as it is. Unchecked, the base system takes the new glass of the state for the base configuration"));
        }
    });
}
