//! 非順次の追跡（[`lensbench_project::nsc`]）: 系・プロジェクトの物の場面で、視野ごとの瞳のグリッドの光線を光源にして
//! 検出器（像面）の照度と経路ごとのパワーの表・エネルギーの収支を出す

use std::path::PathBuf;

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::{Error, Result, System};
use lensbench_project::nsc::{
    spread, system_source, DetectorSpec, NscOpts, PathFilter, Split, StartRay, Tracer,
};
use lensbench_project::scene::Scene;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_analysis::defs::{fields, wave};
use lensbench_analysis::{Analysis, Cx, Meta, Variant};

/// 光源の偏光
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum Polarization {
    /// 偏光なし（直交する 2 つの状態に半分ずつ）
    #[default]
    Unpolarized,
    /// 物体の座標系の x
    X,
    /// 物体の座標系の y
    Y,
}

/// 分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SplitKind {
    /// 決まった分岐（R と T の両方を追う）
    #[default]
    Deterministic,
    /// モンテカルロ（R か T を確率で選ぶ）
    MonteCarlo,
}

/// 非順次の追跡の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct NscParams {
    /// Field number of the source (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Side of the pupil ray grid of the source for each field
    #[schemars(extend("x-min" = 1))]
    pub grid: usize,
    /// Total source power [W] (shared equally by the source rays)
    pub power: f64,
    /// Source polarization
    pub polarization: Polarization,
    /// Ray splitting at boundaries: deterministic (follow both the reflected and the transmitted rays) or monte-carlo (choose one by the power ratio)
    pub split: SplitKind,
    /// Random seed (monte-carlo)
    pub seed: u64,
    /// Branches weaker than this fraction of their source ray are dropped (deterministic) or go to Russian roulette (monte-carlo)
    pub threshold: f64,
    /// Maximum number of events of a ray
    #[schemars(extend("x-min" = 1))]
    pub max_events: usize,
    /// Half widths of the detector rectangle [mm] (x, y; one value for a square; the image semi-diameter if omitted)
    pub half: Option<Vec<f64>>,
    /// Pixels on a side of the detector
    #[schemars(extend("x-min" = 1))]
    pub pixels: usize,
    /// Count in the detector image only the paths with this number of reflections (one value, or the least and the most)
    pub reflections: Option<Vec<usize>>,
    /// Rows of the path table (largest power first)
    pub paths: usize,
    /// Source rays traced again to draw their detected branches on the layout (0 for none)
    pub layout_rays: usize,
    /// Find the sequential system of each row of the path table: a system of the scene (beam, beam splitter path), a ghost (reflected at surfaces a and b) or a beam to add to the project (beam splitter path and one turn at a lens surface)
    pub sequential: bool,
    /// Folder to write the ghost systems of the rows to (ghost_a_b.zmx; with sequential)
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out: Option<PathBuf>,
    /// Measured coating tables (LBCT) and models (LBCM) (more than one allowed)
    #[schemars(extend("x-kind" = "path"))]
    pub coatings: Vec<PathBuf>,
}

impl Default for NscParams {
    fn default() -> Self {
        NscParams {
            field: None,
            wave: None,
            grid: 21,
            power: 1.0,
            polarization: Polarization::Unpolarized,
            split: SplitKind::Deterministic,
            seed: 1,
            threshold: 1e-6,
            max_events: 256,
            half: None,
            pixels: 64,
            reflections: None,
            paths: 20,
            layout_rays: 0,
            sequential: false,
            out: None,
            coatings: Vec::new(),
        }
    }
}

pub(super) struct Nsc;

/// 図に描く枝の数の上限
const MAX_LINES: usize = 5000;

/// 場面を作る系: プロジェクトは光束ごと（光束が無ければ BS の経路ごと）、ほかは読み込んだ系。名前・系・読み方
pub(super) type SceneSystem = (String, System, Variant);

fn scene_systems(cx: &Cx<'_>, notes: &mut Vec<String>) -> Result<Vec<SceneSystem>> {
    let one = || vec![("system".to_string(), cx.system.clone(), Variant::default())];
    let Some(src) = cx.source else {
        return Ok(one());
    };
    if !src.is_project() {
        return Ok(one());
    }
    let beams = src.beams()?;
    if !beams.is_empty() {
        return beams
            .into_iter()
            .map(|b| {
                let v = Variant {
                    beam: Some(b.clone()),
                    ..Default::default()
                };
                Ok((format!("beam {b}"), src.load(&v, notes)?, v))
            })
            .collect();
    }
    let paths = src.paths()?;
    if paths.len() < 2 {
        return Ok(one());
    }
    paths
        .into_iter()
        .map(|q| {
            let v = Variant {
                path: Some(q.clone()),
                ..Default::default()
            };
            Ok((format!("path {q}"), src.load(&v, notes)?, v))
        })
        .collect()
}

impl Analysis for Nsc {
    type Params = NscParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "nsc",
            about: "Non-sequential trace: detector irradiance and power by ray path (reflections, ghosts)",
            help: "Non-sequential ray trace in the scene of the objects of the system (lenses closed by their edges, mirrors, stops, mechanical parts and the image surface as a detector; for a project, the objects of all beams or beam splitter paths): the next surface is the nearest one, and the rays split into reflected and transmitted rays at each boundary (Fresnel with coatings, polarization, glass absorption). The source is the pupil ray grid of each field with the total power shared equally. Returns the detector irradiance [W/mm²], the table of ray paths (events in order) by detected power with the irradiance of each path, the energy budget and the detected branches of some source rays to draw on the layout",
        }
    }
    fn run(&self, cx: &Cx<'_>, a: &NscParams) -> Result<Value> {
        let s0 = cx.system;
        let w = wave(s0, a.wave)?.unwrap_or(s0.primary);
        let fs = fields(s0, a.field)?;
        let half = match a.half.as_deref().unwrap_or_default() {
            [] => None,
            [h] => Some([*h, *h]),
            [x, y] => Some([*x, *y]),
            v => {
                let n = v.len();
                return Err(Error::Invalid(format!(
                    "half takes 1 or 2 values ({n} given)"
                )));
            }
        };
        let filter = match a.reflections.as_deref().unwrap_or_default() {
            [] => None,
            [n] => Some((*n, *n)),
            [lo, hi] => Some((*lo, *hi)),
            v => {
                let n = v.len();
                return Err(Error::Invalid(format!(
                    "reflections takes 1 or 2 values ({n} given)"
                )));
            }
        };
        let mut notes = Vec::new();
        let systems = scene_systems(cx, &mut notes)?;
        let mut sc = Scene::new();
        for (label, s, _) in &systems {
            sc.add_system(label, s, None)?;
        }
        sc.finish()?;
        notes.extend(sc.notes.iter().cloned());
        let mut co = Coatings::for_system(&systems[0].1)?;
        for f in &a.coatings {
            co.load_file(f)?;
        }
        let opts = NscOpts {
            wave: Some(w),
            split: match a.split {
                SplitKind::Deterministic => Split::Deterministic,
                SplitKind::MonteCarlo => Split::MonteCarlo { seed: a.seed },
            },
            threshold: a.threshold,
            max_events: a.max_events,
            detector: DetectorSpec {
                half,
                pixels: [a.pixels, a.pixels],
            },
            filter: filter.map(|r| PathFilter {
                reflections: Some(r),
                via: Vec::new(),
            }),
        };
        let refs: Vec<&System> = systems.iter().map(|x| &x.1).collect();
        let tr = Tracer::new(&sc, &refs, &co, &opts)?;
        let pol = match a.polarization {
            Polarization::Unpolarized => None,
            Polarization::X => Some([C64::new(1.0, 0.0), C64::new(0.0, 0.0)]),
            Polarization::Y => Some([C64::new(0.0, 0.0), C64::new(1.0, 0.0)]),
        };
        let src: Vec<StartRay> = system_source(s0, &fs, a.grid, w, a.power, pol)?;
        let r = tr.run(&src)?;
        let labels = tr.labels();
        let b = &r.budget;
        let detectors: Vec<Value> = r
            .detectors
            .iter()
            .map(|d| {
                let (x, y) = d.centers();
                let irr = d.irradiance();
                let peak = irr.iter().copied().fold(0.0, f64::max);
                json!({
                    "label": d.label, "half_mm": d.half, "nx": d.pixels[0], "ny": d.pixels[1],
                    "pixel_mm2": d.pixel_area(), "power_w": d.total(), "peak_w_mm2": peak,
                    "x": x, "y": y, "irradiance": irr, "hits": d.hits,
                })
            })
            .collect();
        // 行ごとの順次の系（プロジェクトは場面の系ごとの玉の面の書き方で、足す光束の折り返しを決める）
        let mut seqs: Vec<Option<Value>> = vec![None; r.paths.len().min(a.paths)];
        if a.sequential {
            let keys: Vec<Vec<Option<Value>>> = match cx.source {
                Some(s) if s.is_project() => systems
                    .iter()
                    .map(|(_, sy, v)| {
                        Ok(s.lens_faces(v)?
                            .filter(|k| k.len() == sy.surfaces.len())
                            .unwrap_or_default())
                    })
                    .collect::<Result<_>>()?,
                _ => Vec::new(),
            };
            for (q, p) in seqs.iter_mut().zip(&r.paths) {
                *q = Some(super::nsc_seq::sequential(
                    cx,
                    &sc,
                    &systems,
                    &keys,
                    &p.events,
                    a.out.as_deref(),
                    &mut notes,
                )?);
            }
        }
        let rows: Vec<Value> = r
            .paths
            .iter()
            .take(a.paths)
            .enumerate()
            .map(|(i, p)| {
                let area = r.detectors[p.detector].pixel_area();
                let px: Vec<Value> = p
                    .pixels
                    .iter()
                    .map(|&(q, w)| json!([q, w / area]))
                    .collect();
                let ev: Vec<String> = p
                    .events
                    .iter()
                    .map(|(f, k)| format!("{} {}", labels[*f], k.code()))
                    .collect();
                json!({
                    "rank": i + 1, "detector": r.detectors[p.detector].label,
                    "reflections": p.reflections(), "power_w": p.power,
                    "fraction": p.power / b.source, "rays": p.rays, "events": ev.join(" > "),
                    "pixels": px,
                    "sequential": seqs[i],
                })
            })
            .collect();
        // 図に描く枝: 経路の表の順位（表に無い経路は 0）
        let rank: std::collections::HashMap<
            &[(usize, lensbench_project::raypath::EventKind)],
            usize,
        > = r
            .paths
            .iter()
            .take(a.paths)
            .enumerate()
            .map(|(i, p)| (&p.events[..], i + 1))
            .collect();
        // 追い直すのは描ける本数まで（出力の上限と同じ）
        let ids = spread(src.len(), a.layout_rays.min(MAX_LINES));
        let lines: Vec<Value> = tr
            .polylines(&src, &ids)
            .into_iter()
            .take(MAX_LINES)
            .map(|l| {
                json!({
                    "path": rank.get(&l.events[..]).copied().unwrap_or(0),
                    "reflections": l.events.iter().filter(|e| e.1.reflects()).count(),
                    "power_w": l.power,
                    "pts": l.pts,
                })
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        notes.retain(|n| seen.insert(n.clone()));
        notes.extend(co.notes.iter().cloned());
        if a.split == SplitKind::MonteCarlo {
            notes.push(
                "monte-carlo: the energy budget closes only on average (Russian roulette), so residual_w is not 0"
                    .to_string(),
            );
        }
        Ok(json!({
            "wavelength_um": r.wavelength,
            "split": a.split,
            "source_rays": r.launched,
            "scene": { "systems": sc.systems, "objects": sc.objects.len(), "faces": sc.faces.len() },
            "budget": {
                "source_w": b.source, "detected_w": b.detected, "absorbed_w": b.absorbed,
                "escaped_w": b.escaped, "discarded_w": b.discarded, "truncated_w": b.truncated,
                "residual_w": b.residual(),
            },
            "detectors": detectors,
            "paths": rows,
            "path_count": r.paths.len(),
            "rays": lines,
            "warnings": notes,
        }))
    }
}
