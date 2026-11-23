//! 公差のパネルの補償のしかた・逆感度の欄と、結果の締めた公差の表

use egui::{RichText, Ui};
use serde_json::Value;

use super::TolPanel;
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::optim_panel::small_edit;

/// 補償のしかたと逆感度の欄
pub(super) fn settings(ui: &mut Ui, p: &mut TolPanel) {
    ui.horizontal_wrapped(|ui| {
        let lbl = ui.label(tr("Compensation"));
        let modes = [
            ("optimize", tr("Optimize (DLS)")),
            ("paraxial", tr("Paraxial focus")),
            ("none", tr("None")),
        ];
        egui::ComboBox::from_id_salt("tol_comp_mode")
            .selected_text(
                modes
                    .iter()
                    .find(|m| m.0 == p.comp_mode)
                    .map_or(p.comp_mode.clone(), |m| m.1.to_string()),
            )
            .show_ui(ui, |ui| {
                for (k, l) in &modes {
                    ui.selectable_value(&mut p.comp_mode, k.to_string(), *l);
                }
            })
            .response
            .labelled_by(lbl.id)
            .on_hover_text(tr(
                "Paraxial focus keeps the paraxial image distance by the thickness before the image (the compensators are not used); None evaluates without compensation",
            ));
    });
    ui.horizontal_wrapped(|ui| {
        let lbl = ui.label(tr("Inverse sensitivity"));
        let kinds = [
            ("none", tr("Off")),
            ("limit", tr("Limit (criterion value)")),
            ("increment", tr("Increment (change from nominal)")),
        ];
        let before = p.inverse.clone();
        egui::ComboBox::from_id_salt("tol_inverse")
            .selected_text(
                kinds
                    .iter()
                    .find(|m| m.0 == p.inverse)
                    .map_or(p.inverse.clone(), |m| m.1.to_string()),
            )
            .show_ui(ui, |ui| {
                for (k, l) in &kinds {
                    ui.selectable_value(&mut p.inverse, k.to_string(), *l);
                }
            })
            .response
            .labelled_by(lbl.id)
            .on_hover_text(tr(
                "Tightens each tolerance limit (each side separately) until the criterion reaches the value; limits are never widened. Monte Carlo then uses the tightened limits",
            ));
        if p.inverse != before {
            // 欄で選び直したら、欄で表せなかった読んだ値は捨てる
            p.extra.remove("inverse");
        }
        if p.inverse != "none" {
            small_edit(ui, &mut p.inverse_value, 90.0, "0.02");
            let lbl = ui.label(tr("Do not adjust"));
            small_edit(ui, &mut p.inverse_skip, 160.0, "lens1 dx, 3.t").labelled_by(lbl.id);
        }
    });
}

/// 逆感度の設定 {"limit" か "increment", "skip"} を欄（種類, 値, 締めない名前）に。欄で表せなければ None
pub(super) fn fields(x: &Value) -> Option<(String, String, String)> {
    let o = x.as_object()?;
    let mut kind = None;
    let mut skip = String::new();
    for (k, v) in o {
        match k.as_str() {
            "limit" | "increment" if kind.is_none() => {
                kind = Some((k.clone(), fmt_num(v.as_f64()?)))
            }
            "skip" => {
                skip = v
                    .as_array()?
                    .iter()
                    .map(|t| t.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ")
            }
            _ => return None,
        }
    }
    let (k, v) = kind?;
    Some((k, v, skip))
}

/// 逆感度で締めた公差の表（締めた公差だけ。元の範囲 → 締めた範囲）
pub(super) fn table(
    ui: &mut Ui,
    inv: &lensbench_design::tol_report::InverseInfo,
    weak: egui::Color32,
) {
    let head = if inv.mode == "increment" {
        trf(
            "Inverse sensitivity: increment {} (criterion limit {}), {} tolerances tightened",
            &[
                &fmt_num(inv.value),
                &fmt_num(inv.limit),
                &inv.tightened.to_string(),
            ],
        )
    } else {
        trf(
            "Inverse sensitivity: limit {}, {} tolerances tightened",
            &[&fmt_num(inv.limit), &inv.tightened.to_string()],
        )
    };
    ui.label(RichText::new(head).strong());
    let rows: Vec<_> = inv
        .tolerances
        .iter()
        .filter(|t| t.tightened_lo || t.tightened_hi)
        .collect();
    if rows.is_empty() {
        ui.label(
            RichText::new(tr(
                "No tolerance reaches the criterion; the limits are unchanged",
            ))
            .small()
            .color(weak),
        );
        return;
    }
    egui::Grid::new("tol_inverse_grid")
        .striped(true)
        .show(ui, |ui| {
            ui.label(RichText::new(tr("Tolerance")).strong());
            ui.label(RichText::new(tr("Min")).strong());
            ui.label(RichText::new(tr("Max")).strong());
            ui.end_row();
            let side = |ui: &mut Ui, v0: f64, v: f64, t: bool| {
                if t {
                    ui.label(format!("{} → {}", fmt_num(v0), fmt_num(v)));
                } else {
                    ui.label(RichText::new(fmt_num(v)).color(weak));
                }
            };
            for t in rows {
                ui.label(&t.label);
                side(ui, t.lo0, t.lo, t.tightened_lo);
                side(ui, t.hi0, t.hi, t.tightened_hi);
                ui.end_row();
            }
        });
}
