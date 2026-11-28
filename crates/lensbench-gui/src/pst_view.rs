//! 迷光の PST（`pst`）の図: 入射角ごとの PST の対数グラフと、選んだ角度の経路ごとのパワーの表

use egui::{RichText, Ui};
use egui_plot::{Line, Plot, Points};
use serde_json::Value;

use crate::figures::weak;
use crate::i18n::{tr, trf};
use crate::plots::fmt_sig;
use crate::theme;

/// 角度ごとの (角度 [deg], log10 PST)。PST が 0（何も届かない）の角度は抜く
pub(crate) fn pst_points(v: &Value) -> Vec<[f64; 2]> {
    v["angles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let t = a["angle_deg"].as_f64()?;
            let p = a["pst"].as_f64()?;
            (p > 0.0 && p.is_finite()).then(|| [t, p.log10()])
        })
        .collect()
}

/// PST の図。sel は経路の表を出す角度の番号（0 始まり）
pub(crate) fn show(ui: &mut Ui, id: u64, v: &Value, sel: &mut usize) -> bool {
    let Some(angles) = v["angles"].as_array() else {
        return false;
    };
    let n = |x: &Value| fmt_sig(x.as_f64().unwrap_or(f64::NAN), 4);
    ui.label(weak(
        ui,
        trf(
            "{} µm · azimuth {}° · source radius {} mm · detector {} ({} mm²)",
            &[
                &n(&v["wavelength_um"]),
                &n(&v["azimuth_deg"]),
                &n(&v["source_radius_mm"]),
                &v["detector"]["label"].as_str().unwrap_or_default(),
                &n(&v["detector"]["area_mm2"]),
            ],
        ),
    ));
    for w in v["warnings"].as_array().into_iter().flatten() {
        if let Some(t) = w.as_str() {
            ui.label(
                RichText::new(lensbench_core::i18n::tr(t))
                    .small()
                    .color(theme::warn(ui.visuals())),
            );
        }
    }
    let pts = pst_points(v);
    if *sel >= angles.len() {
        *sel = 0;
    }
    let chosen = angles
        .get(*sel)
        .and_then(|a| Some([a["angle_deg"].as_f64()?, a["pst"].as_f64()?]))
        .filter(|p| p[1] > 0.0)
        .map(|p| [p[0], p[1].log10()]);
    let h = (ui.available_height() * 0.45).clamp(180.0, 420.0);
    let plot = Plot::new(("pst", id))
        .height(h)
        .x_axis_label(tr("Source angle [deg]"))
        .y_axis_label(tr("PST (log10)"))
        .y_axis_formatter(|m, _| format!("1e{}", m.value.round()));
    let accent = theme::series(0);
    crate::plot_tools::show(ui, plot, "°", |p, reg| {
        reg.push(crate::plot_tools::Series::new(
            "PST".to_string(),
            accent,
            &pts,
        ));
        p.line(Line::new("PST", pts.clone()).color(accent).width(1.6_f32));
        p.points(Points::new("", pts.clone()).color(accent).radius(3.0));
        if let Some(c) = chosen {
            p.points(Points::new("", vec![c]).color(theme::series(1)).radius(5.5));
        }
    });
    if pts.len() < angles.len() {
        ui.label(weak(
            ui,
            tr("Angles where no light reaches the detector are left out"),
        ));
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Paths at"));
        let shown = |a: &Value| format!("{}°", n(&a["angle_deg"]));
        egui::ComboBox::from_id_salt(("pst-angle", id))
            .selected_text(angles.get(*sel).map(shown).unwrap_or_default())
            .show_ui(ui, |ui| {
                for (i, a) in angles.iter().enumerate() {
                    ui.selectable_value(sel, i, shown(a));
                }
            })
            .response
            .accessible_name(tr("Source angle"));
        if let Some(a) = angles.get(*sel) {
            ui.label(weak(
                ui,
                trf(
                    "PST {} · detected {} W ({} of the source) · {} source rays",
                    &[
                        &n(&a["pst"]),
                        &n(&a["detected_w"]),
                        &n(&a["fraction"]),
                        &a["source_rays"],
                    ],
                ),
            ));
        }
    });
    let empty = Vec::new();
    let rows = angles
        .get(*sel)
        .and_then(|a| a["paths"].as_array())
        .unwrap_or(&empty);
    if rows.is_empty() {
        return true;
    }
    egui::ScrollArea::horizontal()
        .id_salt(("pst-paths", id))
        .show(ui, |ui| {
            egui::Grid::new(("pst-paths-grid", id))
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    for h in [
                        "#",
                        tr("Reflections"),
                        tr("Power [W]"),
                        tr("Fraction"),
                        tr("Rays"),
                        tr("Events"),
                    ] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for r in rows {
                        ui.label(r["rank"].to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// PST の点は log10 で、何も届かない角度は抜く
    #[test]
    fn pst_points_skip_dark_angles() {
        let v = json!({"angles": [
            {"angle_deg": 0.0, "pst": 1.0},
            {"angle_deg": 5.0, "pst": 1e-3},
            {"angle_deg": 10.0, "pst": 0.0},
        ]});
        assert_eq!(pst_points(&v), vec![[0.0, 0.0], [5.0, -3.0]]);
        assert!(pst_points(&json!({})).is_empty());
    }
}
