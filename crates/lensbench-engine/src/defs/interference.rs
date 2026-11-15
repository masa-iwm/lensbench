//! 単色の干渉縞（core の `interference`）・系の一覧（info）。2 つの光束・状態の一覧は系の読み方（[`lensbench_analysis::Source`]）から

use std::path::{Path, PathBuf};

use lensbench_core::coating::Coatings;
use lensbench_core::interference::{self, DetectorOpts, Polarized, PupilOpts};
use lensbench_core::raygrid::InputPol;
use lensbench_core::{report, Error, FieldSel, Result};
use lensbench_export::image::{self, ImageFormat};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_analysis::defs::wave;
use lensbench_analysis::{Analysis, Cx, Meta};

// ---------------------------------------------------------------------------------------- info

/// 系の一覧の設定（無し）
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InfoParams {}

pub struct Info;

impl Analysis for Info {
    type Params = InfoParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "info",
            about: "System data (EFL, F-number, pupils, etc.)",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, _: &InfoParams) -> Result<Value> {
        let s = cx.system;
        let d = report::system_data(s);
        let src = cx.source;
        Ok(json!({
            "name": s.name, "wavelengths_um": s.wavelengths, "primary": s.primary + 1,
            "fields": s.fields.iter().map(|f| [f.x, f.y]).collect::<Vec<_>>(),
            "effl": d.effl, "epd": d.epdi, "enpp": d.enpp, "exp_diameter": d.expd,
            "expp": d.expp, "wfno": d.wfno, "paraxial_wfno": d.pwfn, "image_fno": d.isfn,
            "paraxial_image_height": d.pimh, "paraxial_mag": d.pmag,
            "total_track": d.totr, "object_na": d.obna, "image_na": d.isna,
            "states": src.map(|x| x.states()).transpose()?.unwrap_or_default(),
            "state": src.and_then(|x| x.state()),
            "paths": src.map(|x| x.paths()).transpose()?.unwrap_or_default(),
            "path": src.and_then(|x| x.path()),
            "lensbench": lensbench_core::VERSION,
            "system_hash": src.map(|x| x.system_hash(s)).transpose()?,
        }))
    }
}

// -------------------------------------------------------------------------------------- interference

/// 干渉縞を求める所
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InterferenceMode {
    /// 瞳上（OpticStudio の Interferogram に当たる）
    #[default]
    Pupil,
    /// 像面上（光線を画素へ補間して場を足す）
    Detector,
}

/// 書き出す量
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Quantity {
    /// 強度
    #[default]
    Intensity,
    /// 瞳上: 縞の位相の差 Δ [波]
    Delta,
    /// 像面上: 光路差
    Opd,
    /// 像面上: 可視度
    Visibility,
    /// 像面上: 試料の強度
    Sample,
    /// 像面上: 参照の強度
    Reference,
}

impl Quantity {
    fn name(self) -> &'static str {
        match self {
            Quantity::Intensity => "intensity",
            Quantity::Delta => "delta",
            Quantity::Opd => "opd",
            Quantity::Visibility => "visibility",
            Quantity::Sample => "sample",
            Quantity::Reference => "reference",
        }
    }
}

/// 干渉縞の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InterferenceParams {
    /// Beam 1 (sample): a configuration number (from 1) for .zmx, a state name for a project file
    pub beam1: String,
    /// Beam 2 (reference): the same; 0 or ref for the reference sphere (mode pupil only)
    pub beam2: String,
    /// On the pupil or on the image surface
    pub mode: InterferenceMode,
    /// Field number (from 1)
    #[schemars(extend("x-min" = 1))]
    pub field: usize,
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Pupil: grid side (an even number 4 or more)
    #[schemars(extend("x-min" = 4))]
    pub n: usize,
    /// Pupil: factor on the OPD difference (2 for a double pass)
    pub scale: f64,
    /// Pupil: number of x tilt fringes (per pupil diameter)
    pub xtilt: f64,
    /// Pupil: number of y tilt fringes (per pupil diameter)
    pub ytilt: f64,
    /// Image: side of the pupil ray grid
    #[schemars(extend("x-min" = 2))]
    pub pupil: usize,
    /// Image: number of pixels (on a side)
    #[schemars(extend("x-min" = 1))]
    pub npx: usize,
    /// Image: center x,y of the area (the center of the area reached by both beams if omitted)
    #[schemars(length(min = 2, max = 2), extend("x-unit" = "mm"))]
    pub center: Option<Vec<f64>>,
    /// Image: side of the area (covers the area reached by both beams if omitted)
    #[schemars(extend("x-unit" = "mm"))]
    pub width: Option<f64>,
    /// Image: factor on the sample intensity
    pub sample_power: f64,
    /// Image: factor on the reference intensity
    pub reference_power: f64,
    /// Image: optical path added to the reference path
    #[schemars(extend("x-unit" = "mm"))]
    pub delay: f64,
    /// Image: overlay the fields of a polarization trace (input polarizations sample_input and reference_input; coatings from the system's coating files)
    pub polarized: bool,
    /// Image: input polarization of the sample (x, y, 45, rcp, lcp, etc.)
    pub sample_input: String,
    /// Image: input polarization of the reference
    pub reference_input: String,
    /// Quantity to write: intensity or delta on the pupil; intensity, opd, visibility, sample or reference on the image
    pub quantity: Quantity,
    /// File to write (csv, json, tif or png)
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out: Option<PathBuf>,
    /// Write tif as 32-bit float
    pub float: bool,
    /// Also include the grid in the result (grid, index j·nx + i, row j along +y (j = 0 at the bottom), null for pixels without data)
    pub grid: bool,
}

impl Default for InterferenceParams {
    fn default() -> Self {
        InterferenceParams {
            beam1: "1".into(),
            beam2: "2".into(),
            mode: InterferenceMode::Pupil,
            field: 1,
            wave: None,
            n: 64,
            scale: 1.0,
            xtilt: 0.0,
            ytilt: 0.0,
            pupil: 65,
            npx: 256,
            center: None,
            width: None,
            sample_power: 1.0,
            reference_power: 1.0,
            delay: 0.0,
            polarized: false,
            sample_input: "x".into(),
            reference_input: "x".into(),
            quantity: Quantity::Intensity,
            out: None,
            float: false,
            grid: false,
        }
    }
}

fn reference_sphere(b: &str) -> bool {
    matches!(
        b.trim().to_ascii_lowercase().as_str(),
        "0" | "ref" | "reference"
    )
}

pub struct Interference;

impl Analysis for Interference {
    type Params = InterferenceParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "interference",
            about: "Monochromatic interferogram: overlay the light of two configurations or states",
            help: "Monochromatic interferogram: overlay the light of two configurations (.zmx) or states (project file). mode pupil is on the pupil (like OpticStudio's Interferogram: Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2, I = (1 + cos 2πΔ)/2), detector is on the image surface (rays interpolated to pixels and fields summed). The out extension gives csv (grid, row = +y), json (summary and grid), tif (16 bit; 32-bit float with float) or png (16 bit)",
        }
    }
    fn run(&self, cx: &Cx<'_>, a: &InterferenceParams) -> Result<Value> {
        let src = cx.source()?;
        let mut notes = Vec::new();
        let s1 = src.variant(&a.beam1, &mut notes)?;
        let s2 = if reference_sphere(&a.beam2) {
            None
        } else {
            Some(src.variant(&a.beam2, &mut notes)?)
        };
        if a.field == 0 || a.field > s1.fields.len() {
            return Err(Error::Invalid(format!(
                "no field {} (1 to {})",
                a.field,
                s1.fields.len()
            )));
        }
        let field = FieldSel::Index(a.field - 1);
        let w = wave(&s1, a.wave)?;
        let q = a.quantity.name();
        let (grid, n, mut summary) = match a.mode {
            InterferenceMode::Pupil => {
                let o = PupilOpts {
                    n: a.n,
                    scale: a.scale,
                    xtilt: a.xtilt,
                    ytilt: a.ytilt,
                };
                let r = interference::interferogram(&s1, s2.as_ref(), field, w, &o)?;
                let p = r.delta.x().to_vec();
                let n = p.len();
                let grid: Vec<f64> = match a.quantity {
                    Quantity::Intensity => r.intensity.data().to_vec(),
                    Quantity::Delta => r.delta.data().to_vec(),
                    _ => {
                        return Err(Error::Invalid(format!(
                            "quantity {q} (on the pupil: intensity, delta)"
                        )))
                    }
                };
                let summary = json!({
                    "mode": "pupil", "quantity": q, "nx": n, "ny": n,
                    "pupil_min": p[0], "pupil_step": p[1] - p[0],
                    "beam1": a.beam1, "beam2": s2.as_ref().map(|_| a.beam2.clone()),
                    "scale": a.scale, "xtilt": a.xtilt, "ytilt": a.ytilt,
                    "pv_waves": r.pv, "rms_waves": r.rms,
                });
                (grid, n, summary)
            }
            InterferenceMode::Detector => {
                let Some(s2) = s2 else {
                    return Err(Error::Invalid(
                        "mode detector needs a configuration or state for beam2".into(),
                    ));
                };
                let o = DetectorOpts {
                    pupil: a.pupil,
                    npx: a.npx,
                    center: a.center.as_ref().map(|c| (c[0], c[1])),
                    width: a.width,
                    sample_power: a.sample_power,
                    reference_power: a.reference_power,
                    delay: a.delay,
                };
                let coat;
                let pol = if a.polarized {
                    coat = Coatings::for_system(&s1)?;
                    Some(Polarized {
                        sample: InputPol::parse(&a.sample_input)?,
                        reference: InputPol::parse(&a.reference_input)?,
                        coat: &coat,
                    })
                } else {
                    None
                };
                let r = interference::detector(&s1, &s2, field, w, &o, pol)?;
                let grid: Vec<f64> = match a.quantity {
                    Quantity::Intensity => r.intensity.clone(),
                    Quantity::Opd => r.opd.clone(),
                    Quantity::Visibility => r.visibility.clone(),
                    Quantity::Sample => r.i_sample.clone(),
                    Quantity::Reference => r.i_reference.clone(),
                    Quantity::Delta => {
                        return Err(Error::Invalid(format!(
                            "quantity {q} (on the image: intensity, opd, visibility, sample, reference)"
                        )))
                    }
                };
                let both = (0..r.n * r.n)
                    .filter(|&q| r.ok_sample[q] && r.ok_reference[q])
                    .count();
                let vis: Vec<f64> = r
                    .visibility
                    .iter()
                    .cloned()
                    .filter(|v| v.is_finite())
                    .collect();
                let summary = json!({
                    "mode": "detector", "quantity": q, "nx": r.n, "ny": r.n,
                    "pixel_mm": r.dx, "first_pixel_center_mm": [r.x0, r.y0],
                    "top_left_pixel_center_mm": [r.x0, r.y0 + (r.n as f64 - 1.0) * r.dx],
                    "wavelength_um": r.wavelength, "beam1": a.beam1, "beam2": a.beam2,
                    "overlap_pixels": both,
                    "visibility_mean": if vis.is_empty() { f64::NAN } else { vis.iter().sum::<f64>() / vis.len() as f64 },
                    "fold_sample": r.fold.0, "fold_reference": r.fold.1,
                    "polarized": a.polarized, "delay_mm": a.delay,
                });
                (grid, r.n, summary)
            }
        };
        if let Some(p) = &a.out {
            write_grid(p, a.float, &grid, n, n, &summary)?;
        }
        if a.grid {
            summary["grid"] = json!(grid);
        }
        summary["notes"] = json!(notes);
        Ok(summary)
    }
}

pub(crate) fn write_grid(
    p: &Path,
    float: bool,
    grid: &[f64],
    nx: usize,
    ny: usize,
    summary: &Value,
) -> Result<()> {
    let io = |x: std::io::Error| Error::Io(format!("{}: {x}", p.display()));
    let ext = p
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => {
            let mut s = String::new();
            for j in 0..ny {
                let row: Vec<String> = grid[j * nx..(j + 1) * nx]
                    .iter()
                    .map(|v| {
                        if v.is_nan() {
                            "nan".into()
                        } else {
                            format!("{v:?}")
                        }
                    })
                    .collect();
                s += &row.join(",");
                s.push('\n');
            }
            lensbench_core::vfs::write(p, s).map_err(io)
        }
        "json" => {
            let mut v = summary.clone();
            v["grid"] = json!(grid
                .iter()
                .map(|x| if x.is_nan() { None } else { Some(*x) })
                .collect::<Vec<_>>());
            lensbench_core::vfs::write(p, serde_json::to_string(&v).unwrap()).map_err(io)
        }
        _ => {
            let fmt = ImageFormat::from_path(p, float).ok_or_else(|| {
                Error::Invalid(format!("out extension {ext} (csv, json, tif, png)"))
            })?;
            let (lo, hi) = image::data_range(grid);
            let mut side = summary.clone();
            side["file"] = json!(p.file_name().map(|x| x.to_string_lossy().to_string()));
            side["values"] = json!(if fmt == ImageFormat::TiffFloat {
                "grid values, NaN = no data".to_string()
            } else {
                format!("linear: 0 = {lo:?}, 65535 = {hi:?}; no data = 0")
            });
            side["rows"] = json!("row 0 = top = largest y; columns -> +x");
            let desc = serde_json::to_string_pretty(&side).unwrap();
            let soft = format!("lensbench {}", lensbench_core::VERSION);
            match fmt {
                ImageFormat::TiffFloat => {
                    let mut f = vec![0f32; nx * ny];
                    for row in 0..ny {
                        for i in 0..nx {
                            f[row * nx + i] = grid[(ny - 1 - row) * nx + i] as f32;
                        }
                    }
                    image::write_tiff(p, nx, ny, None, Some(&f), &desc, &soft).map_err(io)?;
                }
                _ => {
                    let u = image::to_u16(grid, nx, ny, Some((lo, hi)));
                    if fmt == ImageFormat::Png16 {
                        image::write_png16(p, nx, ny, &u).map_err(io)?;
                    } else {
                        image::write_tiff(p, nx, ny, Some(&u), None, &desc, &soft).map_err(io)?;
                    }
                }
            }
            image::write_sidecar(p, &(desc + "\n")).map_err(io)
        }
    }
}
