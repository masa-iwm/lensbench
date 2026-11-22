//! 最適化のパネルの大域の探索とガラスの置き換え: 設定の欄、設定の JSON、結果の表、システムに当てる材料

use egui::{RichText, Ui};
use serde_json::{json, Value};

use super::{int_list, small_edit, OptPanel};
use crate::i18n::{tr, trf};
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
    Ok(Some(o))
}

pub(super) fn search_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Start points"))
            .on_hover_text(tr("Global search: optimize from this many start points (the current values first) in parallel and keep the best solutions. Variables with both bounds are drawn in their range, others around their value. 0 or 1: local optimization only"));
        ui.add(crate::widgets::Num::new(&mut p.starts).range(0..=10000));
        ui.add_enabled_ui(p.starts >= 2, |ui| {
            ui.label(tr("Random seed"));
            ui.add(crate::widgets::Num::new(&mut p.seed));
            ui.label(tr("Solutions to keep"))
                .on_hover_text(tr("Number of distinct solutions to keep (best first)"));
            ui.add(crate::widgets::Num::new(&mut p.keep).range(1..=100));
            ui.label(tr("Spread of unbounded variables"))
                .on_hover_text(tr("Spread of variables without bounds, relative to max(|value|, typical size of the kind)"));
            ui.add(
                crate::widgets::Num::new(&mut p.spread)
                    .range(1e-6..=100.0)
                    .max_decimals(4),
            );
        });
    });
    ui.checkbox(&mut p.glass_on, tr("Substitute glasses after optimizing"))
        .on_hover_text(tr("Choose the glass of the listed surfaces from catalog glasses: for each surface, the candidates nearest in nd and vd to its current medium are tried with the variables re-optimized. Model glasses and surfaces with dn/disp are always replaced by a real glass; real glasses only when the merit improves"));
    ui.add_enabled_ui(p.glass_on, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Surfaces to substitute"));
            small_edit(ui, &mut p.glass_surfaces, 120.0, "1, 3, 6");
            ui.label(tr("Glass catalogs"));
            small_edit(ui, &mut p.glass_catalogs, 160.0, tr("System glasses"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Candidates per surface"));
            ui.add(crate::widgets::Num::new(&mut p.glass_candidates).range(1..=200));
            ui.label(tr("Max passes"));
            ui.add(crate::widgets::Num::new(&mut p.glass_passes).range(1..=20));
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
                        ui.label(format!("{}", c.surface));
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

/// 結果のガラスの置き換えで変わった面の (面, 材料)。before なら置き換える前の材料
pub(crate) fn result_glasses(r: &OptimizeView, before: bool) -> Vec<(usize, String)> {
    let Some(g) = &r.glass else {
        return Vec::new();
    };
    let mut out: Vec<(usize, String)> = Vec::new();
    for c in &g.changes {
        if out.iter().any(|(j, _)| *j == c.surface) {
            continue;
        }
        let m = if before {
            c.from.clone()
        } else {
            g.materials
                .iter()
                .find(|(j, _)| *j == c.surface)
                .map_or_else(|| c.to.clone(), |x| x.1.clone())
        };
        out.push((c.surface, m));
    }
    out
}
