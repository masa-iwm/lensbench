//! 迷光の PST（点光源透過率）: 視野の外の平行光の角度ごとに、非順次の追跡（[`lensbench_project::nsc`]）で検出器
//! （像面）へ届くパワーを数え、検出器の平均の照度を入射の照度で割った値と、効いている経路の表を出す

use std::path::PathBuf;

use lensbench_core::coating::C64;
use lensbench_core::math::{mv, normalize};
use lensbench_core::{Error, Result, System};
use lensbench_project::nsc::{disc_source, DetectorSpec, NscOpts, Split, Tracer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::nsc::{events_text, prepare, Polarization, Prepared, SplitKind};
use lensbench_analysis::defs::wave;
use lensbench_analysis::{Analysis, Cx, Meta};

/// PST の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PstParams {
    /// Source angles from the optical axis [deg] (below 90)
    pub angles: Vec<f64>,
    /// Azimuth of the source direction [deg] (0: the source is tilted toward +y like a y field, 90: toward +x)
    pub azimuth: f64,
    /// Radius of the collimated source disc [mm] (covers the entrance of the system; 1.1 times the largest semi-diameter, mechanical radius and mechanical part if omitted)
    pub radius: Option<f64>,
    /// Rays on a side of the source disc grid
    #[schemars(extend("x-min" = 1))]
    pub grid: usize,
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Source polarization
    pub polarization: Polarization,
    /// Ray splitting at boundaries: monte-carlo (choose one by the power ratio) or deterministic (follow both)
    pub split: SplitKind,
    /// Random seed (monte-carlo, and scatter directions)
    pub seed: u64,
    /// Branches weaker than this fraction of their source ray are dropped (deterministic) or go to Russian roulette (monte-carlo)
    pub threshold: f64,
    /// Maximum number of events of a ray
    #[schemars(extend("x-min" = 1))]
    pub max_events: usize,
    /// Half widths of the detector rectangle [mm] (x, y; one value for a square; the image semi-diameter if omitted)
    pub half: Option<Vec<f64>>,
    /// Rows of the path table for each angle (largest power first)
    pub paths: usize,
    /// Scattered rays made at each scattering event (deterministic split; monte-carlo scatters one ray)
    #[schemars(extend("x-min" = 1))]
    pub scatter_rays: usize,
    /// Measured coating tables (LBCT) and models (LBCM) (more than one allowed)
    #[schemars(extend("x-kind" = "path"))]
    pub coatings: Vec<PathBuf>,
    /// Surface finishes file (JSON list of scatter and reflectance of surfaces, lens edges and mechanical parts by system surface number; added to those of the project)
    #[schemars(extend("x-kind" = "path"))]
    pub finishes: Option<PathBuf>,
}

impl Default for PstParams {
    fn default() -> Self {
        PstParams {
            angles: vec![0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 45.0, 60.0, 80.0],
            azimuth: 0.0,
            radius: None,
            grid: 101,
            wave: None,
            polarization: Polarization::Unpolarized,
            split: SplitKind::MonteCarlo,
            seed: 1,
            threshold: 1e-6,
            max_events: 256,
            half: None,
            paths: 5,
            scatter_rays: 1,
            coatings: Vec::new(),
            finishes: None,
        }
    }
}

pub(super) struct Pst;

/// 円板の光源の既定の半径: 系の面の半直径・機械半径と機構部品の外径の最大の 1.1 倍
fn default_radius(s: &System) -> f64 {
    let surf = s
        .surfaces
        .iter()
        .skip(1)
        .filter(|d| !d.ignore)
        .map(|d| d.semi.max(d.mech_semi.unwrap_or(0.0)))
        .fold(0.0, f64::max);
    let mech = s
        .mechanics
        .iter()
        .flat_map(|p| p.profile())
        .map(|q| q[3])
        .fold(0.0, f64::max);
    1.1 * surf.max(mech)
}

impl Analysis for Pst {
    type Params = PstParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "pst",
            about: "Stray light: point source transmittance (PST) by source angle, with the paths that bring the light",
            help: "Stray light analysis with the non-sequential trace (see nsc): for each source angle, a collimated source disc covering the entrance of the system shines on the scene of the objects (lenses with their edges, mirrors, stops, mechanical parts; scatter and reflectance by the finishes), and the power reaching the detector (the image surface) is counted. PST is the mean detector irradiance divided by the source irradiance (normal to the beam). Returns the PST, the detected power and its fraction of the source, and the strongest ray paths for each angle",
        }
    }
    fn run(&self, cx: &Cx<'_>, a: &PstParams) -> Result<Value> {
        let s0 = cx.system;
        let w = wave(s0, a.wave)?.unwrap_or(s0.primary);
        if a.angles.is_empty() {
            return Err(Error::Invalid("pst: give one or more angles".into()));
        }
        if let Some(t) = a.angles.iter().find(|t| !(t.is_finite() && t.abs() < 90.0)) {
            return Err(Error::Invalid(format!(
                "pst: angle {t}° (must be below 90°)"
            )));
        }
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
        let Prepared {
            systems,
            scene: sc,
            coat: co,
            mut notes,
        } = prepare(cx, &a.coatings, a.finishes.as_deref())?;
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
                pixels: [1, 1],
            },
            filter: None,
            scatter_rays: a.scatter_rays,
            seed: a.seed,
        };
        let refs: Vec<&System> = systems.iter().map(|x| &x.1).collect();
        let tr = Tracer::new(&sc, &refs, &co, &opts)?;
        let labels = tr.labels();
        let radius = a.radius.unwrap_or_else(|| default_radius(s0));
        // 光源の円板の中心の直線: 面 1 の頂点を通る。向きは物体の座標系で傾ける
        let fr = s0.frames();
        let obj = fr.obj;
        let j1 = (1..s0.surfaces.len())
            .find(|&j| !s0.surfaces[j].ignore && !s0.surfaces[j].is_coord_break())
            .unwrap_or(1);
        let c = fr.surf.get(j1).copied().flatten().map_or(obj.o, |f| f.o);
        let pol = match a.polarization {
            Polarization::Unpolarized => None,
            Polarization::X => Some([C64::new(1.0, 0.0), C64::new(0.0, 0.0)]),
            Polarization::Y => Some([C64::new(0.0, 0.0), C64::new(1.0, 0.0)]),
        };
        let area_in = std::f64::consts::PI * radius * radius;
        let mut rows = Vec::new();
        let mut det_info = Value::Null;
        for &t in &a.angles {
            let (st, ct) = t.to_radians().sin_cos();
            let (sp, cp) = a.azimuth.to_radians().sin_cos();
            let k = normalize(mv(&obj.r, [st * sp, st * cp, ct]));
            let src = disc_source(&sc, c, k, radius, a.grid, 1.0, pol, &obj.r)?;
            let r = tr.run(&src)?;
            let d = r
                .detectors
                .first()
                .ok_or_else(|| Error::Invalid("pst: the scene has no detector".into()))?;
            let area_det = 4.0 * d.half[0] * d.half[1];
            let p_det = d.total();
            let e_in = r.budget.source / area_in;
            let pst = p_det / area_det / e_in;
            if det_info.is_null() {
                det_info = json!({"label": d.label, "half_mm": d.half, "area_mm2": area_det});
            }
            let paths: Vec<Value> = r
                .paths
                .iter()
                .take(a.paths)
                .enumerate()
                .map(|(i, p)| {
                    json!({
                        "rank": i + 1, "reflections": p.reflections(), "power_w": p.power,
                        "fraction": p.power / r.budget.source, "rays": p.rays,
                        "events": events_text(labels, &p.events),
                    })
                })
                .collect();
            rows.push(json!({
                "angle_deg": t, "pst": pst, "detected_w": p_det,
                "fraction": p_det / r.budget.source, "source_rays": r.launched,
                "residual_w": r.budget.residual(), "paths": paths,
            }));
        }
        let mut seen = std::collections::HashSet::new();
        notes.retain(|n| seen.insert(n.clone()));
        notes.extend(co.notes.iter().cloned());
        if a.split == SplitKind::MonteCarlo {
            notes.push(
                "monte-carlo: PST is a statistical estimate (more rays: a larger grid)".to_string(),
            );
        }
        Ok(json!({
            "wavelength_um": s0.wavelengths[w],
            "split": a.split,
            "azimuth_deg": a.azimuth,
            "source_radius_mm": radius,
            "source_irradiance_w_mm2": 1.0 / area_in,
            "detector": det_info,
            "angles": rows,
            "warnings": notes,
        }))
    }
}
