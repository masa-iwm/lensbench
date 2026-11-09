//! 解析の図（の一部）: スポット図（視野ごとの光線の点）と MTF の曲線。前の結果（値を変える前）も薄く重ねる。
//! ほかの解析の図は figures.rs。

use egui::{Color32, RichText, Sense, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot};
use serde_json::Value;

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::theme;

/// スポット図の光線の点（1 視野・1 波長）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotRays {
    /// 視野の番号（1 始まり）
    pub field: usize,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 像面の局所座標 [レンズの単位]（通った光線だけ）
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

/// 瞳の点（同心円。中心と、k 番目の輪に 6k 点）。最初の点は主光線（0, 0）
pub fn pupil_grid(rings: usize) -> (Vec<f64>, Vec<f64>) {
    let mut px = vec![0.0];
    let mut py = vec![0.0];
    for k in 1..=rings {
        let r = k as f64 / rings as f64;
        let n = 6 * k;
        for j in 0..n {
            let a = std::f64::consts::TAU * j as f64 / n as f64;
            px.push(r * a.cos());
            py.push(r * a.sin());
        }
    }
    (px, py)
}

/// trace の結果（配列）から通った光線の点を取る
pub fn rays_from(field: usize, wave: usize, arrays: &[(String, Vec<f64>)]) -> SpotRays {
    let get = |k: &str| {
        arrays
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    };
    let (x, y, err, vig) = (get("x"), get("y"), get("err"), get("vig"));
    let mut r = SpotRays {
        field,
        wave,
        ..Default::default()
    };
    for i in 0..x.len().min(y.len()) {
        let ok =
            err.get(i).copied().unwrap_or(0.0) == 0.0 && vig.get(i).copied().unwrap_or(0.0) == 0.0;
        if ok && x[i].is_finite() && y[i].is_finite() {
            r.x.push(x[i]);
            r.y.push(y[i]);
        }
    }
    r
}

/// 図に要る系の値
pub struct SysInfo<'a> {
    pub waves_um: &'a [f64],
    pub primary: usize,
    pub wfno: Option<f64>,
}

fn spot_rms(v: &Value, field: usize) -> Option<f64> {
    v["spot"]
        .as_array()?
        .iter()
        .find(|s| s["field"].as_u64() == Some(field as u64))?["rms_centroid"]
        .as_f64()
}

fn delta_text(now: f64, prev: Option<f64>) -> Option<(String, Color32)> {
    let p = prev?;
    let d = now - p;
    if d.abs() < 1e-12 * now.abs().max(1e-300) {
        return None;
    }
    let col = if d < 0.0 {
        Color32::from_rgb(0x8F, 0xD1, 0x9E)
    } else {
        Color32::from_rgb(0xFF, 0x9F, 0x43)
    };
    Some((
        format!("{}{}", if d > 0.0 { "+" } else { "" }, fmt_sig(d, 3)),
        col,
    ))
}

/// 有効数字 n 桁
pub fn fmt_sig(x: f64, n: i32) -> String {
    if x == 0.0 || !x.is_finite() {
        return fmt_num(x);
    }
    let e = x.abs().log10().floor() as i32;
    let k = 10f64.powi(n - 1 - e);
    fmt_num((x * k).round() / k)
}

/// スポット図。点は µm、中心は各視野の主波長の重心（無ければ全体の重心）
pub fn spot(ui: &mut Ui, rays: &[SpotRays], result: &Value, prev: Option<&Value>, sys: &SysInfo) {
    let mut fields: Vec<usize> = rays.iter().map(|r| r.field).collect();
    fields.sort();
    fields.dedup();
    if fields.is_empty() {
        ui.weak(tr("光線を追跡しています…"));
        return;
    }
    // 視野ごとの中心と、全視野で共通の縮尺
    let centers: Vec<(f64, f64)> = fields
        .iter()
        .map(|&f| {
            let pick = |only_primary: bool| {
                let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
                for r in rays
                    .iter()
                    .filter(|r| r.field == f && (!only_primary || r.wave == sys.primary))
                {
                    for i in 0..r.x.len() {
                        sx += r.x[i];
                        sy += r.y[i];
                        n += 1;
                    }
                }
                (n > 0).then(|| (sx / n as f64, sy / n as f64))
            };
            pick(true).or_else(|| pick(false)).unwrap_or((0.0, 0.0))
        })
        .collect();
    let mut rmax: f64 = 0.0;
    for r in rays {
        let c = centers[fields.iter().position(|&f| f == r.field).unwrap()];
        for i in 0..r.x.len() {
            rmax = rmax.max(((r.x[i] - c.0).powi(2) + (r.y[i] - c.1).powi(2)).sqrt());
        }
    }
    let airy_um = sys.wfno.map(|f| {
        1.22 * sys
            .waves_um
            .get(sys.primary.saturating_sub(1))
            .copied()
            .unwrap_or(0.55)
            * f
    });
    // 縮尺は点と Airy 円が入る大きさ（µm）
    let half_um = (rmax * 1e3).max(airy_um.unwrap_or(0.0)).max(1e-3) * 1.15;
    let n = fields.len();
    let avail = ui.available_width();
    let cols = ((avail / 150.0).floor() as usize).clamp(1, n.max(1));
    let cell = ((avail - 8.0 * cols as f32) / cols as f32).clamp(90.0, 260.0);
    let vis = ui.visuals().clone();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(trf("枠の半幅 {} µm", &[&fmt_sig(half_um, 3)]))
                .small()
                .color(theme::weak_text(&vis)),
        );
        if let Some(a) = airy_um {
            ui.label(
                RichText::new(trf("破線の円は Airy 半径 {} µm", &[&fmt_sig(a, 3)]))
                    .small()
                    .color(theme::weak_text(&vis)),
            );
        }
        // 波長の凡例
        let mut waves: Vec<usize> = rays.iter().map(|r| r.wave).collect();
        waves.sort();
        waves.dedup();
        for w in waves {
            let um = sys.waves_um.get(w - 1).copied().unwrap_or(0.55);
            ui.label(
                RichText::new(format!("● {} µm", fmt_num(um)))
                    .small()
                    .color(theme::wave_color(um)),
            );
        }
    });
    egui::Grid::new(ui.next_auto_id())
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (k, &f) in fields.iter().enumerate() {
                ui.vertical(|ui| {
                    let (resp, painter) =
                        ui.allocate_painter(egui::vec2(cell, cell), Sense::hover());
                    let rect = resp.rect;
                    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
                    let c = rect.center();
                    let s = (cell / 2.0 - 4.0) / half_um as f32;
                    let grid = vis.widgets.noninteractive.bg_stroke.color;
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 4.0, c.y),
                            egui::pos2(rect.right() - 4.0, c.y),
                        ],
                        (0.5, grid),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(c.x, rect.top() + 4.0),
                            egui::pos2(c.x, rect.bottom() - 4.0),
                        ],
                        (0.5, grid),
                    );
                    if let Some(a) = airy_um {
                        let pts: Vec<egui::Pos2> = (0..=48)
                            .map(|j| {
                                let t = std::f32::consts::TAU * j as f32 / 48.0;
                                c + egui::vec2(t.cos(), t.sin()) * a as f32 * s
                            })
                            .collect();
                        painter.add(egui::Shape::dashed_line(
                            &pts,
                            (1.0, theme::weak_text(&vis).gamma_multiply(0.7)),
                            3.0,
                            3.0,
                        ));
                    }
                    let cc = centers[k];
                    let painter = painter.with_clip_rect(rect);
                    for r in rays.iter().filter(|r| r.field == f) {
                        let col = theme::wave_color(
                            sys.waves_um.get(r.wave - 1).copied().unwrap_or(0.55),
                        );
                        for i in 0..r.x.len() {
                            let p = c + egui::vec2(
                                ((r.x[i] - cc.0) * 1e3) as f32 * s,
                                // 像の y は上が正
                                -((r.y[i] - cc.1) * 1e3) as f32 * s,
                            );
                            painter.circle_filled(p, 1.3, col);
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(trf("視野 {}", &[&f])).color(theme::weak_text(&vis)),
                        );
                        if let Some(rms) = spot_rms(result, f) {
                            let um = rms * 1e3;
                            ui.label(
                                RichText::new(format!("RMS {} µm", fmt_sig(um, 3))).monospace(),
                            );
                            if let Some((t, col)) =
                                delta_text(um, prev.and_then(|p| spot_rms(p, f)).map(|x| x * 1e3))
                            {
                                ui.label(RichText::new(t).small().color(col))
                                    .on_hover_text(tr("値を変える前との差"));
                            }
                        }
                    });
                });
                if (k + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
}

/// MTF の曲線（視野ごとに色、子午は実線・球欠は破線）。前の結果は薄い点線
pub fn mtf(ui: &mut Ui, result: &Value, prev: Option<&Value>, id: u64) {
    mtf_curves(ui, result, prev, id, "mtf");
}

/// MTF の曲線（key は結果の並びの名前: 回折 MTF は mtf、幾何 MTF は geometric_mtf）
pub fn mtf_curves(ui: &mut Ui, result: &Value, prev: Option<&Value>, id: u64, key: &str) {
    let Some(curves) = result[key].as_array() else {
        ui.weak(tr("MTF の結果がない"));
        return;
    };
    let unit = result["unit"].as_str().unwrap_or("cycles/mm").to_string();
    let lines = |v: &Value| -> Vec<(usize, &'static str, Vec<[f64; 2]>)> {
        let mut out = Vec::new();
        for c in v[key].as_array().into_iter().flatten() {
            let f = c["field"].as_u64().unwrap_or(0) as usize;
            let fr: Vec<f64> = c["freqs"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
                .unwrap_or_default();
            for k in ["tangential", "sagittal"] {
                if let Some(a) = c[k].as_array() {
                    let pts: Vec<[f64; 2]> = fr
                        .iter()
                        .zip(a)
                        .filter_map(|(x, y)| Some([*x, y.as_f64()?]))
                        .collect();
                    out.push((f, k, pts));
                }
            }
        }
        out
    };
    let now = lines(result);
    let before = prev.map(lines).unwrap_or_default();
    let _ = curves;
    Plot::new((key, id))
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .include_x(0.0)
        .x_axis_label(unit)
        .y_axis_label("MTF")
        .allow_scroll(false)
        .show(ui, |p| {
            for (f, k, pts) in before {
                p.line(
                    Line::new(pts)
                        .color(theme::series(f.saturating_sub(1)).gamma_multiply(0.35))
                        .style(LineStyle::Dotted { spacing: 4.0 })
                        .width(1.0_f32),
                );
                let _ = k;
            }
            for (f, k, pts) in now {
                let t = k == "tangential";
                p.line(
                    Line::new(pts)
                        .color(theme::series(f.saturating_sub(1)))
                        .style(if t {
                            LineStyle::Solid
                        } else {
                            LineStyle::Dashed { length: 6.0 }
                        })
                        .width(1.6_f32)
                        .name(if t {
                            trf("視野 {} 子午", &[&f])
                        } else {
                            trf("視野 {} 球欠", &[&f])
                        }),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_and_rays() {
        let (px, py) = pupil_grid(3);
        assert_eq!(px.len(), 1 + 6 + 12 + 18);
        assert_eq!((px[0], py[0]), (0.0, 0.0));
        assert!(px
            .iter()
            .zip(&py)
            .all(|(x, y)| x * x + y * y <= 1.0 + 1e-12));
        let a = vec![
            ("x".to_string(), vec![0.0, 1.0, 2.0]),
            ("y".to_string(), vec![0.0, 1.0, f64::NAN]),
            ("err".to_string(), vec![0.0, 0.0, 0.0]),
            ("vig".to_string(), vec![0.0, 1.0, 0.0]),
        ];
        let r = rays_from(2, 1, &a);
        assert_eq!(r.x, vec![0.0]);
        assert_eq!(r.field, 2);
    }

    #[test]
    fn sig() {
        assert_eq!(fmt_sig(0.0123456, 3), "0.0123");
        assert_eq!(fmt_sig(-1234.5, 2), "-1200");
    }
}
