//! 光束の表: プロジェクトの光束（`beams`）ごとの透過・けられ・主光線の光路長と参照の光束との差

use lensbench_core::{Error, FieldSel, Result, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_analysis::defs::{mean_transmission, wave};
use lensbench_analysis::{Analysis, Cx, Meta};

/// 光束の表の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct BeamsParams {
    /// Field number (from 1)
    #[schemars(extend("x-min" = 1))]
    pub field: usize,
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Side of the pupil ray grid for the transmission and the vignetting
    #[schemars(extend("x-min" = 3))]
    pub grid: usize,
    /// Reference beam for the optical path difference (the first beam with role reference if omitted, else the first beam)
    pub reference: Option<String>,
    /// Include the beams marked off
    pub all: bool,
}

impl Default for BeamsParams {
    fn default() -> Self {
        BeamsParams {
            field: 1,
            wave: None,
            grid: 21,
            reference: None,
            all: false,
        }
    }
}

pub(super) struct Beams;

/// 光束 1 本の系の量: (主光線の光路長、けられた光線の割合、瞳で平均した透過率)
fn beam_row(s: &System, field: usize, w: usize, grid: usize) -> Result<(Option<f64>, f64, Value)> {
    if field == 0 || field > s.fields.len() {
        return Err(Error::Invalid(format!(
            "no field {field} (1 to {})",
            s.fields.len()
        )));
    }
    let f = FieldSel::Index(field - 1);
    let c = s.try_trace(f, &[0.0], &[0.0], Some(w), None, false)?;
    let r = &c.rays[0];
    let chief = (r.status.surf().is_none() && r.vig == 0).then_some(r.opl);
    let (px, py) = lensbench_core::opd::pupil_grid(grid);
    let t = s.try_trace(f, &px, &py, Some(w), None, false)?;
    let lost = t
        .rays
        .iter()
        .filter(|r| r.status.surf().is_some() || r.vig != 0)
        .count();
    let vignetted = lost as f64 / px.len().max(1) as f64;
    let tr = match mean_transmission(s, f, w) {
        Ok(v) => json!(v),
        Err(e) => json!({ "error": e.to_string() }),
    };
    Ok((chief, vignetted, tr))
}

impl Analysis for Beams {
    type Params = BeamsParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "beams",
            about: "Beam table: transmission, vignetting and chief-ray optical path of each beam of a project",
            help: "Beam table for the beams of a project file (beams: named beam splitter paths): for each beam, the pupil-averaged coating and Fresnel transmission (unpolarized, including the splitting ratio), the fraction of vignetted rays of the pupil grid, the chief-ray optical path to the image surface and its difference from the reference beam. Beams marked off are left out unless all",
        }
    }
    fn run(&self, cx: &Cx<'_>, a: &BeamsParams) -> Result<Value> {
        let src = cx.source()?;
        let Some(text) = src.project_text()? else {
            return Err(Error::Invalid(
                "beams are only for project files (.json)".into(),
            ));
        };
        let p = lensbench_project::Project::from_json(&text)?;
        if p.beams.is_empty() {
            return Err(Error::Invalid("the project has no beams".into()));
        }
        if a.grid < 3 {
            return Err(Error::Invalid(format!("grid {} (3 or more)", a.grid)));
        }
        let beams: Vec<&lensbench_project::Beam> =
            p.beams.iter().filter(|b| a.all || !b.off).collect();
        if beams.is_empty() {
            return Err(Error::Invalid(
                "every beam is off (give all to include them)".into(),
            ));
        }
        let reference = match &a.reference {
            Some(n) => {
                if !beams.iter().any(|b| b.name == *n) {
                    let names: Vec<&str> = beams.iter().map(|b| b.name.as_str()).collect();
                    let names = names.join(", ");
                    return Err(Error::Invalid(format!("no beam {n} ({names})")));
                }
                n.clone()
            }
            None => beams
                .iter()
                .find(|b| b.role == Some(lensbench_project::BeamRole::Reference))
                .unwrap_or(&beams[0])
                .name
                .clone(),
        };
        let mut notes = Vec::new();
        let mut rows = Vec::new();
        let mut ref_opl = None;
        let mut wavelength = None;
        for b in &beams {
            let s = src.variant(&b.name, &mut notes)?;
            let w = wave(&s, a.wave)?.unwrap_or(s.primary);
            wavelength.get_or_insert(s.wavelengths[w]);
            let (chief, vignetted, tr) =
                beam_row(&s, a.field, w, a.grid).map_err(|e| format!("beam {}: {e}", b.name))?;
            if b.name == reference {
                ref_opl = chief;
            }
            rows.push(json!({
                "name": b.name, "path": b.path, "state": b.state, "role": b.role, "off": b.off,
                "surfaces": s.surfaces.len(), "transmission": tr, "vignetted": vignetted,
                "chief_opl_mm": chief,
            }));
        }
        for r in &mut rows {
            let d = match (r["chief_opl_mm"].as_f64(), ref_opl) {
                (Some(o), Some(z)) => json!(o - z),
                _ => Value::Null,
            };
            r["opl_difference_mm"] = d;
        }
        Ok(json!({
            "field": a.field, "wavelength_um": wavelength, "reference": reference,
            "beams": rows, "notes": notes,
        }))
    }
}
