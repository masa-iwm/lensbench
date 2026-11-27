//! 非順次の追跡（`nsc`）の図: エネルギーの収支の行、検出器ごとの照度の図（線形・対数、経路ごとの寄与）、
//! 経路ごとのパワーの表と、レイアウト図に重ねる枝の折れ線（[`StrayLines`]）

use egui::{Color32, RichText, Ui};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::figures::{f64s, heatmap, memo, weak, Cmap, Grid};
use crate::i18n::{tr, trf};
use crate::plots::fmt_sig;
use crate::theme;

/// 対数の図で見せる桁の数（ピークから下へ）
const DECADES: f64 = 6.0;

/// 非順次の図の見せ方（解析のパネルごと、[`crate::figures::FigState::nsc`]）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct NscFig {
    /// 照度を対数（log10）で
    pub log: bool,
    /// 経路の表で選んだ行の順位（1 始まり。0 は全部の経路）
    pub path: usize,
    /// レイアウト図に枝を重ねる
    pub layout: bool,
    /// レイアウト図に重ねる枝の反射の回数（最小・最大。経路を選んでいる間は使わない）
    pub refl: [usize; 2],
}

impl Default for NscFig {
    fn default() -> Self {
        NscFig {
            log: true,
            path: 0,
            layout: true,
            refl: [0, 99],
        }
    }
}

/// 非順次の検出器の照度の格子（`irradiance`、番号 j·nx + i、j は +y の向き）。上の行から並べ直す
pub(crate) fn nsc_grid(d: &Value) -> Option<Grid> {
    let g = f64s(&d["irradiance"]);
    grid_of(d, &g)
}

/// 検出器 d の大きさで、値 g（番号 j·nx + i）を上の行からの格子に
fn grid_of(d: &Value, g: &[f64]) -> Option<Grid> {
    let xs = f64s(&d["x"]);
    let ys = f64s(&d["y"]);
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

/// 経路の表の行 row（`pixels` の [画素, 照度] の疎な並び）の照度を、その検出器 d の格子に
pub(crate) fn path_grid(d: &Value, row: &Value) -> Option<Grid> {
    let n = d["nx"].as_u64()? as usize * d["ny"].as_u64()? as usize;
    let mut g = vec![0.0; n];
    for p in row["pixels"].as_array()? {
        let (Some(q), Some(w)) = (p[0].as_u64(), p[1].as_f64()) else {
            continue;
        };
        if let Some(x) = g.get_mut(q as usize) {
            *x = w;
        }
    }
    grid_of(d, &g)
}

/// 照度の図（線形か対数）
fn irradiance_map(ui: &mut Ui, salt: (u64, usize, usize), g: &Grid, log: bool, h: f32) {
    let peak = g.range().map(|r| r.1).filter(|x| *x > 0.0).unwrap_or(1.0);
    if log {
        let floor = peak * 10f64.powf(-DECADES);
        let mut lg = g.clone();
        for x in lg.v.iter_mut() {
            *x = x.max(floor).log10();
        }
        let hi = peak.log10();
        heatmap(
            ui,
            ("nsc", salt, true),
            &lg,
            Cmap::Viridis,
            hi - DECADES,
            hi,
            ("x [mm]", "y [mm]", tr("log10 irradiance [W/mm²]")),
            h,
        );
    } else {
        heatmap(
            ui,
            ("nsc", salt, false),
            g,
            Cmap::Viridis,
            0.0,
            peak,
            ("x [mm]", "y [mm]", tr("Irradiance [W/mm²]")),
            h,
        );
    }
}

/// 非順次の追跡: エネルギーの収支、検出器ごとの照度の図、経路ごとのパワーの表。st は図の見せ方
pub(crate) fn show(ui: &mut Ui, id: u64, gen: u64, v: &Value, st: &mut NscFig) -> bool {
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
            ui.label(
                RichText::new(lensbench_core::i18n::tr(t))
                    .small()
                    .color(theme::warn(ui.visuals())),
            );
        }
    }
    let empty = Vec::new();
    let rows = v["paths"].as_array().unwrap_or(&empty);
    if st.path > rows.len() {
        st.path = 0;
    }
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut st.log, tr("Log (log10)"));
        let lines = v["rays"].as_array().map_or(0, Vec::len);
        ui.add_enabled(
            lines > 0,
            egui::Checkbox::new(&mut st.layout, tr("Show on the layout")),
        )
        .on_hover_text(tr(
            "Draw the detected branches of some source rays on the layout, colored by their row in the path table (gray: not in the table). Choose a row to draw only that path",
        ));
        if st.layout && st.path == 0 {
            ui.label(tr("Reflections"));
            let [lo, hi] = &mut st.refl;
            ui.add(egui::DragValue::new(lo).range(0..=99));
            ui.label("–");
            ui.add(egui::DragValue::new(hi).range(0..=99));
            if *hi < *lo {
                *hi = *lo;
            }
        }
        if st.path > 0 && ui.button(tr("All paths")).clicked() {
            st.path = 0;
        }
    });
    let h = (ui.available_height() * 0.5 / dets.len().max(1) as f32).clamp(160.0, 480.0);
    if st.path > 0 {
        // 選んだ経路だけの照度（その経路の検出器）
        let row = &rows[st.path - 1];
        let d = dets
            .iter()
            .find(|d| d["label"] == row["detector"])
            .unwrap_or(&dets[0]);
        ui.label(trf(
            "Path {} on {}: {} W ({} of the source)",
            &[
                &st.path,
                &row["detector"].as_str().unwrap_or_default(),
                &n(&row["power_w"]),
                &n(&row["fraction"]),
            ],
        ));
        let k = st.path;
        let g = memo(ui, ("nsc-path-grid", id, k), gen, || path_grid(d, row));
        if let Some(g) = &*g {
            irradiance_map(ui, (id, k, 1), g, st.log, h);
        }
    } else {
        let grids = memo(ui, ("nsc-grid", id), gen, || {
            dets.iter().map(nsc_grid).collect::<Vec<_>>()
        });
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
                irradiance_map(ui, (id, k, 0), g, st.log, h);
            }
        }
    }
    if rows.is_empty() {
        return true;
    }
    ui.label(trf(
        "Ray paths by detected power ({} of {})",
        &[&rows.len(), &v["path_count"]],
    ));
    ui.label(weak(
        ui,
        tr("Click a row to see the irradiance of that path only"),
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
                    for (i, r) in rows.iter().enumerate() {
                        let k = i + 1;
                        let mark = RichText::new(r["rank"].to_string()).color(path_color(k));
                        if ui
                            .selectable_label(st.path == k, mark)
                            .on_hover_text(tr("Show this path only (click again for all)"))
                            .clicked()
                        {
                            st.path = if st.path == k { 0 } else { k };
                        }
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

/// 経路の表の順位 k（1 始まり、0 は表に無い経路）の色
pub(crate) fn path_color(k: usize) -> Color32 {
    if k == 0 {
        Color32::from_gray(140)
    } else {
        theme::series(k - 1)
    }
}

/// レイアウト図に重ねる枝（画面の一時データ）の鍵: 作った解析のパネル・結果の世代・見せ方
type StrayKey = Option<(u64, u64, NscFig)>;

fn stray_id() -> egui::Id {
    egui::Id::unique("lensbench-nsc-stray")
}

/// レイアウト図に重ねる枝（[`set_stray`] で置いたもの）
pub(crate) fn stray(ctx: &egui::Context) -> Option<Arc<StrayLines>> {
    ctx.data(|d| d.get_temp::<(StrayKey, Option<Arc<StrayLines>>)>(stray_id()))
        .and_then(|x| x.1)
}

/// レイアウト図に重ねる枝を、鍵 key（None は重ねない）の結果から置く。鍵が前と同じなら作り直さない。
/// result はパネルの番号から結果を引く
pub(crate) fn set_stray<'a>(
    ctx: &egui::Context,
    key: StrayKey,
    result: impl FnOnce(u64) -> Option<&'a Value>,
) {
    let old = ctx.data(|d| d.get_temp::<(StrayKey, Option<Arc<StrayLines>>)>(stray_id()));
    if old.as_ref().is_some_and(|o| o.0 == key) {
        return;
    }
    let lines = key
        .as_ref()
        .and_then(|(id, _, st)| result(*id).map(|v| Arc::new(StrayLines::from_result(v, st))));
    ctx.data_mut(|d| d.insert_temp(stray_id(), (key, lines)));
}

/// レイアウト図に重ねる枝の折れ線（大域座標 mm）: 色と点
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StrayLines {
    pub lines: Vec<(Color32, Vec<[f64; 3]>)>,
}

impl StrayLines {
    /// 結果 v の `rays` を、見せ方 st で選んで色を付ける。パワーの小さい枝ほど薄く（ピークから [`DECADES`] 桁で）
    pub(crate) fn from_result(v: &Value, st: &NscFig) -> StrayLines {
        let Some(rays) = v["rays"].as_array() else {
            return StrayLines::default();
        };
        let keep = |r: &&Value| {
            let k = r["path"].as_u64().unwrap_or(0) as usize;
            if st.path > 0 {
                return k == st.path;
            }
            let m = r["reflections"].as_u64().unwrap_or(0) as usize;
            (st.refl[0]..=st.refl[1]).contains(&m)
        };
        let chosen: Vec<&Value> = rays.iter().filter(keep).collect();
        let top = chosen
            .iter()
            .filter_map(|r| r["power_w"].as_f64())
            .fold(0.0, f64::max);
        let lines = chosen
            .into_iter()
            .filter_map(|r| {
                let p = r["power_w"].as_f64().unwrap_or(0.0);
                let t = if top > 0.0 && p > 0.0 {
                    (1.0 + (p / top).log10() / DECADES).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let pts: Vec<[f64; 3]> = r["pts"]
                    .as_array()?
                    .iter()
                    .filter_map(|q| Some([q[0].as_f64()?, q[1].as_f64()?, q[2].as_f64()?]))
                    .collect();
                let k = r["path"].as_u64().unwrap_or(0) as usize;
                // 表に無い経路は控えめに
                let a = (0.2 + 0.7 * t as f32) * if k == 0 { 0.4 } else { 1.0 };
                (pts.len() >= 2).then(|| (k, path_color(k).gamma_multiply(a), pts))
            })
            .collect::<Vec<_>>();
        // 表に無い経路を先に（表の経路が上に描かれる）。表の経路は順位の大きい方から
        let mut lines = lines;
        lines.sort_by_key(|x| if x.0 == 0 { usize::MAX } else { x.0 });
        lines.reverse();
        StrayLines {
            lines: lines.into_iter().map(|x| (x.1, x.2)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result() -> Value {
        json!({
            "detectors": [{"label": "IMA", "half_mm": [1.0, 1.0], "nx": 2, "ny": 2,
                "x": [-0.5, 0.5], "y": [-0.5, 0.5], "irradiance": [1.0, 2.0, 3.0, 4.0]}],
            "paths": [
                {"rank": 1, "detector": "IMA", "pixels": [[0, 1.0], [3, 4.0]]},
                {"rank": 2, "detector": "IMA", "pixels": [[2, 3.0]]},
            ],
            "rays": [
                {"path": 1, "reflections": 0, "power_w": 1.0, "pts": [[0, 0, 0], [0, 0, 10]]},
                {"path": 2, "reflections": 2, "power_w": 1e-3, "pts": [[0, 0, 0], [0, 1, 10]]},
                {"path": 0, "reflections": 4, "power_w": 1e-9, "pts": [[0, 0, 0], [0, 2, 10]]},
            ],
        })
    }

    /// 経路の画素の疎な並びは、全体の照度と同じ並べ方（上の行から）の格子になる
    #[test]
    fn path_grid_matches_the_detector_layout() {
        let v = result();
        let d = &v["detectors"][0];
        assert_eq!(nsc_grid(d).unwrap().v, [3.0, 4.0, 1.0, 2.0]);
        let g = path_grid(d, &v["paths"][0]).unwrap();
        assert_eq!(g.v, [0.0, 4.0, 1.0, 0.0]);
        assert_eq!(g.extent, [-1.0, 1.0, -1.0, 1.0]);
    }

    /// 重ねる枝は選んだ経路か反射の回数で選び、パワーの小さいものほど薄い
    #[test]
    fn stray_lines_follow_the_choice() {
        let v = result();
        let all = StrayLines::from_result(&v, &NscFig::default());
        assert_eq!(all.lines.len(), 3);
        // 表に無い経路が先、順位 1 が最後（上に描く）。パワーの大きいものほど濃い
        assert_eq!(all.lines[0].1[1], [0.0, 2.0, 10.0]);
        assert_eq!(all.lines[2].1[1], [0.0, 0.0, 10.0]);
        assert!(all.lines[2].0.a() > all.lines[1].0.a());
        assert!(all.lines[1].0.a() > all.lines[0].0.a());
        let st = NscFig {
            refl: [2, 2],
            ..Default::default()
        };
        assert_eq!(StrayLines::from_result(&v, &st).lines.len(), 1);
        let st = NscFig {
            path: 1,
            ..Default::default()
        };
        let one = StrayLines::from_result(&v, &st);
        assert_eq!(one.lines.len(), 1);
        assert_eq!(one.lines[0].1[1], [0.0, 0.0, 10.0]);
        assert!(StrayLines::from_result(&json!({}), &st).lines.is_empty());
    }
}
