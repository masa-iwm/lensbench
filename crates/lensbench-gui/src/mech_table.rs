//! 機構のケラレ（`mech-check`）の表: 行が視野・波長、列が「通る」「開口で切れる」「部品ごとに最初に切った」割合
//! （行の合計は 100 %）。通る割合が 100 % 未満の行は色を付ける。部品に当たった光線の数（ほかで先に切れたものも
//! 数える）は部品の欄のツールチップに出す。表の下に瞳の点の色分けの図（[`crate::mech_pupil`]）。

use egui::{RichText, Ui};
use serde_json::Value;

use crate::i18n::{tr, trf};
use crate::theme;

/// 割合の文字（小数 1 桁）
fn pct(x: f64) -> String {
    format!("{x:.1}")
}

/// 表を描く。描けない結果（`results` が無い）なら false
pub(crate) fn show(ui: &mut Ui, id: u64, v: &Value) -> bool {
    let Some(rows) = v["results"].as_array() else {
        return false;
    };
    let parts: Vec<&str> = v["parts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| p["name"].as_str().unwrap_or("?"))
                .collect()
        })
        .unwrap_or_default();
    let vis = ui.visuals().clone();
    let note = if v["mech_clip"] == true {
        tr("The other analyses also vignette rays at the mechanical parts (system setting).")
    } else {
        tr("The other analyses do not vignette rays at the mechanical parts (system setting). The layout draws the clipped rays faint.")
    };
    ui.label(RichText::new(note).small().color(theme::weak_text(&vis)));
    let any_failed = rows
        .iter()
        .any(|r| r["failed_pct"].as_f64().unwrap_or(0.0) > 0.0);
    egui::ScrollArea::horizontal()
        .id_salt(("mech-table", id))
        .show(ui, |ui| {
            egui::Grid::new(("mech-grid", id))
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    ui.strong(tr("Field #"));
                    ui.strong(tr("Wave #"));
                    ui.strong(tr("Passes [%]")).on_hover_text(tr(
                        "Rays reaching the image without being vignetted anywhere",
                    ));
                    ui.strong(tr("Aperture [%]")).on_hover_text(tr(
                        "Rays vignetted by an aperture of the optical surfaces before any part",
                    ));
                    if any_failed {
                        ui.strong(tr("Stopped [%]"))
                            .on_hover_text(tr("Rays that missed a surface or were totally reflected"));
                    }
                    for p in &parts {
                        ui.strong(format!("{p} [%]")).on_hover_text(tr(
                            "Rays this part clips first (along the light path)",
                        ));
                    }
                    ui.end_row();
                    for r in rows {
                        let passed = r["passed_pct"].as_f64().unwrap_or(f64::NAN);
                        let cut = passed < 100.0 - 1e-9;
                        let txt = |s: String| {
                            if cut {
                                RichText::new(s).color(theme::warn(&vis))
                            } else {
                                RichText::new(s)
                            }
                        };
                        ui.label(txt(r["field"].to_string()));
                        ui.label(txt(r["wave"].to_string()));
                        ui.label(txt(pct(passed)).strong());
                        ui.label(txt(pct(r["optical_pct"].as_f64().unwrap_or(f64::NAN))));
                        if any_failed {
                            ui.label(txt(pct(r["failed_pct"].as_f64().unwrap_or(0.0))));
                        }
                        let ps = r["parts"].as_array().cloned().unwrap_or_default();
                        for k in 0..parts.len() {
                            let q = ps.get(k).cloned().unwrap_or(Value::Null);
                            let first = q["first_pct"].as_f64().unwrap_or(f64::NAN);
                            let all = q["clipped_pct"].as_f64().unwrap_or(f64::NAN);
                            ui.label(txt(pct(first))).on_hover_text(trf(
                                "Rays leaving this part's bore: {} % (also counting rays clipped earlier by another part or an aperture)",
                                &[&pct(all)],
                            ));
                        }
                        ui.end_row();
                    }
                });
        });
    // 瞳の上でどこがどの部品で切れるか
    crate::mech_pupil::show(ui, v, &parts);
    // 部品の収まり（はみ出し・保持径・ロッド）
    if let Some(fit) = v["fit"].as_array().filter(|f| !f.is_empty()) {
        ui.add_space(6.0);
        ui.strong(tr("Fit of the parts"));
        for f in fit {
            let m = f["message"].as_str().unwrap_or_default();
            let c = if f["error"] == true {
                theme::error(&vis)
            } else {
                theme::weak_text(&vis)
            };
            ui.label(RichText::new(m).small().color(c));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_the_table_and_rejects_other_results() {
        let v = serde_json::json!({
            "mech_clip": true,
            "parts": [{"name": "tube"}, {"name": "cage"}],
            "results": [
                {"field": 1, "wave": 1, "failed_pct": 0.0, "optical_pct": 0.0, "passed_pct": 100.0,
                 "parts": [{"clipped_pct": 0.0, "first_pct": 0.0}, {"clipped_pct": 0.0, "first_pct": 0.0}]},
                {"field": 3, "wave": 1, "failed_pct": 0.0, "optical_pct": 0.0, "passed_pct": 0.0,
                 "parts": [{"clipped_pct": 29.7, "first_pct": 29.7}, {"clipped_pct": 91.5, "first_pct": 70.3}]},
            ],
            "fit": [],
        });
        let ctx = egui::Context::default();
        let mut shown = (false, true);
        let _ = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                shown = (show(ui, 1, &v), show(ui, 2, &serde_json::json!({"x": 1})));
            });
        });
        assert_eq!(shown, (true, false));
    }
}
