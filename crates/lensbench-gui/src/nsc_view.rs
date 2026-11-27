//! 非順次の追跡（`nsc`）の図: エネルギーの収支の行、検出器ごとの照度の図、経路ごとのパワーの表

use egui::{RichText, Ui};
use serde_json::Value;

use crate::figures::{f64s, heatmap, memo, weak, Cmap, Grid};
use crate::i18n::{tr, trf};
use crate::plots::fmt_sig;
use crate::theme;

/// 非順次の検出器の照度の格子（`irradiance`、番号 j·nx + i、j は +y の向き）。上の行から並べ直す
pub(crate) fn nsc_grid(d: &Value) -> Option<Grid> {
    let xs = f64s(&d["x"]);
    let ys = f64s(&d["y"]);
    let g = f64s(&d["irradiance"]);
    let (nx, ny) = (xs.len(), ys.len());
    let h = f64s(&d["half_mm"]);
    if nx == 0 || ny == 0 || g.len() != nx * ny || h.len() != 2 {
        return None;
    }
    let mut out = Vec::with_capacity(nx * ny);
    for j in (0..ny).rev() {
        out.extend_from_slice(&g[j * nx..(j + 1) * nx]);
    }
    Some(Grid {
        w: nx,
        h: ny,
        v: out,
        extent: [-h[0], h[0], -h[1], h[1]],
    })
}

/// 非順次の追跡: エネルギーの収支、検出器ごとの照度の図、経路ごとのパワーの表
pub(crate) fn show(ui: &mut Ui, id: u64, gen: u64, v: &Value) -> bool {
    let Some(dets) = v["detectors"].as_array() else {
        return false;
    };
    let n = |x: &Value| fmt_sig(x.as_f64().unwrap_or(f64::NAN), 4);
    let b = &v["budget"];
    ui.label(weak(
        ui,
        trf(
            "{} µm · {} source rays · source {} W · detected {} W · absorbed {} W · escaped {} W · dropped {} W",
            &[
                &n(&v["wavelength_um"]),
                &v["source_rays"],
                &n(&b["source_w"]),
                &n(&b["detected_w"]),
                &n(&b["absorbed_w"]),
                &n(&b["escaped_w"]),
                &fmt_sig(
                    b["discarded_w"].as_f64().unwrap_or(0.0)
                        + b["truncated_w"].as_f64().unwrap_or(0.0),
                    4,
                ),
            ],
        ),
    ));
    for w in v["warnings"].as_array().into_iter().flatten() {
        if let Some(t) = w.as_str() {
            ui.label(RichText::new(t).small().color(theme::warn(ui.visuals())));
        }
    }
    let grids = memo(ui, ("nsc-grid", id), gen, || {
        dets.iter().map(nsc_grid).collect::<Vec<_>>()
    });
    let h = (ui.available_height() * 0.5 / dets.len().max(1) as f32).clamp(160.0, 480.0);
    for (k, (d, g)) in dets.iter().zip(grids.iter()).enumerate() {
        ui.label(trf(
            "Detector {}: {} W, peak {} W/mm²",
            &[
                &d["label"].as_str().unwrap_or_default(),
                &n(&d["power_w"]),
                &n(&d["peak_w_mm2"]),
            ],
        ));
        if let Some(g) = g {
            let hi = d["peak_w_mm2"].as_f64().filter(|x| *x > 0.0).unwrap_or(1.0);
            heatmap(
                ui,
                ("nsc", id, k),
                g,
                Cmap::Viridis,
                0.0,
                hi,
                ("x [mm]", "y [mm]", tr("Irradiance [W/mm²]")),
                h,
            );
        }
    }
    let Some(rows) = v["paths"].as_array() else {
        return true;
    };
    ui.label(trf(
        "Ray paths by detected power ({} of {})",
        &[&rows.len(), &v["path_count"]],
    ));
    egui::ScrollArea::horizontal()
        .id_salt(("nsc-paths", id))
        .show(ui, |ui| {
            egui::Grid::new(("nsc-paths-grid", id))
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("#");
                    ui.strong(tr("Detector"));
                    ui.strong(tr("Reflections"));
                    ui.strong(tr("Power [W]"));
                    ui.strong(tr("Fraction"));
                    ui.strong(tr("Rays"));
                    ui.strong(tr("Events"));
                    ui.end_row();
                    for r in rows {
                        ui.label(r["rank"].to_string());
                        ui.label(r["detector"].as_str().unwrap_or_default());
                        ui.label(r["reflections"].to_string());
                        ui.label(n(&r["power_w"]));
                        ui.label(n(&r["fraction"]));
                        ui.label(r["rays"].to_string());
                        ui.label(r["events"].as_str().unwrap_or_default());
                        ui.end_row();
                    }
                });
        });
    true
}
