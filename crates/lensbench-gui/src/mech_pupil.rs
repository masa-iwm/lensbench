//! 機構のケラレ（`mech-check`）の瞳の図: 視野・波長ごとに瞳の格子の点を、通る・開口で切れる・止まる・最初に切った
//! 部品で色分けして並べる（フットプリントのような見た目）。点の結果は結果の `pupil`（点）と各行の `state`
//! （0 = 通る、-1 = 開口、-2 = 止まる、k ≥ 1 = 部品 k）。

use egui::{Color32, Rect, RichText, Sense, Ui, Vec2};
use serde_json::Value;

use crate::i18n::{tr, trf};
use crate::theme;

/// 1 枚の図の一辺 [px]
const SIDE: f32 = 132.0;

/// 部品の色（通る点の緑・止まる点の赤と重ならない系列の色）
fn part_color(k: usize) -> Color32 {
    const IX: [usize; 5] = [0, 2, 4, 5, 6];
    theme::series(IX[k % IX.len()])
}

/// 点の結果の色
fn state_color(vis: &egui::Visuals, q: i64) -> Color32 {
    match q {
        0 => theme::ok(vis),
        -1 => theme::weak_text(vis),
        k if k >= 1 => part_color(k as usize - 1),
        _ => theme::error(vis),
    }
}

/// 格子の間隔（正規化した瞳で）: 点の x の違う値の数から
fn pitch(px: &[f64]) -> f64 {
    let mut xs: Vec<f64> = px.to_vec();
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    if xs.len() < 2 {
        2.0
    } else {
        2.0 / (xs.len() - 1) as f64
    }
}

/// 図を描く。点の結果が無い（古い結果）なら何もしない
pub(crate) fn show(ui: &mut Ui, v: &Value, parts: &[&str]) {
    let (Some(px), Some(py), Some(rows)) = (
        v["pupil"]["px"].as_array(),
        v["pupil"]["py"].as_array(),
        v["results"].as_array(),
    ) else {
        return;
    };
    let px: Vec<f64> = px.iter().filter_map(Value::as_f64).collect();
    let py: Vec<f64> = py.iter().filter_map(Value::as_f64).collect();
    if px.is_empty() || px.len() != py.len() || rows.iter().all(|r| r["state"].is_null()) {
        return;
    }
    let vis = ui.visuals().clone();
    ui.add_space(6.0);
    ui.strong(tr("Where the pupil is vignetted"));
    // 凡例
    ui.horizontal_wrapped(|ui| {
        let key = |ui: &mut Ui, c: Color32, s: &str| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().rect_filled(r, 1.0, c);
            ui.label(RichText::new(s).small());
            ui.add_space(6.0);
        };
        key(ui, state_color(&vis, 0), tr("Reaches the image"));
        key(ui, state_color(&vis, -1), tr("Vignetted by an aperture"));
        key(ui, state_color(&vis, -2), tr("Trace stopped"));
        for (k, p) in parts.iter().enumerate() {
            key(ui, part_color(k), p);
        }
    });
    let d = pitch(&px);
    ui.horizontal_wrapped(|ui| {
        for r in rows {
            let Some(st) = r["state"].as_array() else {
                continue;
            };
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(trf(
                        "Field {} · wave {}",
                        &[&r["field"].to_string(), &r["wave"].to_string()],
                    ))
                    .small(),
                );
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(SIDE), Sense::hover());
                let pnt = ui.painter_at(rect);
                pnt.circle_stroke(
                    rect.center(),
                    SIDE * 0.5 - 1.0,
                    (1.0, theme::weak_text(&vis).gamma_multiply(0.5)),
                );
                // 正規化した瞳の半径 1 が図の半径（の少し内側）、上が +y
                let scale = (SIDE * 0.5 - 2.0) as f64 / (1.0 + 0.5 * d);
                let cell = ((d * scale) as f32 * 0.9).max(1.0);
                for ((x, y), q) in px.iter().zip(&py).zip(st) {
                    let c = rect.center() + Vec2::new((x * scale) as f32, -(y * scale) as f32);
                    let col = state_color(&vis, q.as_i64().unwrap_or(-2));
                    pnt.rect_filled(Rect::from_center_size(c, Vec2::splat(cell)), 0.0, col);
                }
                resp.on_hover_text(trf(
                    "Passes {} %, aperture {} %",
                    &[
                        &format!("{:.1}", r["passed_pct"].as_f64().unwrap_or(f64::NAN)),
                        &format!("{:.1}", r["optical_pct"].as_f64().unwrap_or(f64::NAN)),
                    ],
                ));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_of_the_grid_and_colors_of_the_states() {
        assert!((pitch(&[-1.0, 0.0, 1.0, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(pitch(&[0.0]), 2.0);
        let vis = egui::Visuals::dark();
        assert_eq!(state_color(&vis, 0), theme::ok(&vis));
        assert_eq!(state_color(&vis, 2), part_color(1));
        assert_ne!(state_color(&vis, -2), state_color(&vis, -1));
    }

    #[test]
    fn draws_the_pupils_and_skips_old_results() {
        let v = serde_json::json!({
            "pupil": {"px": [0.0, 0.5, -0.5], "py": [0.0, 0.0, 0.0]},
            "results": [{"field": 1, "wave": 1, "passed_pct": 66.7, "optical_pct": 0.0, "state": [0, 1, -1]}],
        });
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                show(ui, &v, &["tube"]);
                show(ui, &serde_json::json!({"results": []}), &[]);
            });
        });
    }
}
