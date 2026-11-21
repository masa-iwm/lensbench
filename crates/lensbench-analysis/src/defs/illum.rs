//! フットプリント図・相対照度・けられの図（Footprint Diagram・Relative Illumination・Vignetting Plot）

use lensbench_core::{Error, FieldSel, Result, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, wave, Axis};
use crate::footprint;
use crate::registry::{Analysis, Cx, Meta};
use crate::sweep::steps01;

// ----------------------------------------------------------------------------------- footprint

/// フットプリントの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FootprintParams {
    /// Surface number (the stop surface if omitted)
    pub surface: Option<usize>,
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; every wavelength if omitted)
    pub wave: Option<usize>,
    /// Pupil ray grid (rays across the pupil diameter)
    #[schemars(extend("x-min" = 2))]
    pub rays: usize,
    /// Ignore the vignetting factors of the fields (only the surface apertures clip the full pupil)
    pub no_vignetting_factors: bool,
}

impl Default for FootprintParams {
    fn default() -> Self {
        FootprintParams {
            surface: None,
            field: None,
            wave: None,
            rays: 21,
            no_vignetting_factors: false,
        }
    }
}

/// フットプリントの 1 視野・1 波長
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FootprintRow {
    /// Field number (from 1)
    pub field: usize,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 面に届いた光線の交点 (x, y)（面の局所座標）
    pub points: Vec<[f64; 2]>,
    /// 瞳の縁の光線の交点（面より前でけられた光線は null）
    pub edge: Vec<[Option<f64>; 2]>,
    /// 交点の x・y の最小・最大
    #[serde(deserialize_with = "crate::nan::a2")]
    pub x: [f64; 2],
    #[serde(deserialize_with = "crate::nan::a2")]
    pub y: [f64; 2],
    /// 面の頂点からの交点の距離の最大
    #[serde(deserialize_with = "crate::nan::f")]
    pub max_radius: f64,
    /// max_radius の、面の半直径に対する割合（半直径が 0 なら null）
    #[serde(deserialize_with = "crate::nan::f")]
    pub fill: f64,
    /// 像面までけられずに通った光線の割合
    #[serde(deserialize_with = "crate::nan::f")]
    pub through: f64,
    /// この面でけられた光線の割合
    #[serde(deserialize_with = "crate::nan::f")]
    pub clipped: f64,
}

/// フットプリントの結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FootprintOut {
    /// 面の番号
    pub surface: usize,
    pub unit: String,
    /// 面の半直径
    #[serde(deserialize_with = "crate::nan::f")]
    pub semi: f64,
    /// 面の開口の輪郭（閉じた折れ線の並び、面の局所座標）
    pub aperture: Vec<Vec<[f64; 2]>>,
    pub rows: Vec<FootprintRow>,
    /// 全部の視野・波長の max_radius の最大と、その半直径に対する割合
    #[serde(deserialize_with = "crate::nan::f")]
    pub max_radius: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub fill: f64,
    pub warnings: Vec<String>,
}

pub(super) struct Footprint;

impl Analysis for Footprint {
    type Params = FootprintParams;
    type Output = FootprintOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "footprint",
            about: "Footprint diagram: where the beam of each field and wavelength crosses a surface, against its aperture",
            help: "Footprint diagram (OpticStudio's Footprint Diagram): the local intersections on a surface of a grid of pupil rays for each field and wavelength, with the outline of the surface aperture, the largest radius and its fraction of the semi-diameter, the fraction of rays that reach the image unvignetted and the fraction clipped at this surface. Rays clipped before the surface are not drawn. The vignetting factors of the fields are used unless no_vignetting_factors",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &FootprintParams) -> Result<FootprintOut> {
        let s = cx.system;
        let j = match p.surface {
            Some(j) => j,
            None => s.surfaces.iter().position(|d| d.stop).unwrap_or(1),
        };
        if j == 0 || j > s.image() {
            return Err(Error::Invalid(format!("surface {j} (1 to {})", s.image())));
        }
        let ws: Vec<usize> = match wave(s, p.wave)? {
            Some(w) => vec![w],
            None => (0..s.wavelengths.len()).collect(),
        };
        let semi = s.surfaces[j].semi.abs();
        let frac = |r: f64| if semi > 0.0 { r / semi } else { f64::NAN };
        let mut rows = Vec::new();
        let mut warnings = Vec::new();
        for i in fields(s, p.field)? {
            for &w in &ws {
                match footprint::footprint(
                    s,
                    FieldSel::Index(i),
                    w,
                    j,
                    p.rays,
                    !p.no_vignetting_factors,
                ) {
                    Ok(f) => {
                        let n = f.rays.max(1) as f64;
                        rows.push(FootprintRow {
                            field: i + 1,
                            wave: w + 1,
                            edge: f
                                .edge
                                .iter()
                                .map(|q| {
                                    [
                                        q[0].is_finite().then_some(q[0]),
                                        q[1].is_finite().then_some(q[1]),
                                    ]
                                })
                                .collect(),
                            points: f.points,
                            x: f.x,
                            y: f.y,
                            max_radius: f.max_radius,
                            fill: frac(f.max_radius),
                            through: f.through as f64 / n,
                            clipped: f.clipped as f64 / n,
                        });
                    }
                    Err(e) => warnings.push(format!("field {}, wavelength {}: {e}", i + 1, w + 1)),
                }
            }
        }
        if rows.is_empty() {
            return Err(Error::Invalid(warnings.join("; ")));
        }
        let max_radius = rows
            .iter()
            .map(|r| r.max_radius)
            .filter(|r| r.is_finite())
            .fold(f64::NAN, f64::max);
        Ok(FootprintOut {
            surface: j,
            unit: "mm".into(),
            semi,
            aperture: footprint::aperture_outline(s, j),
            rows,
            max_radius,
            fill: frac(max_radius),
            warnings,
        })
    }
}

// ------------------------------------------------------------------------ relative-illumination

/// 相対照度の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RelIllumParams {
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Number of field points (normalized field 0 to 1)
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
    /// Field direction (y or x)
    pub axis: Axis,
    /// Number of points on the rim of the beam (even, 8 or more)
    #[schemars(extend("x-min" = 8))]
    pub rim: usize,
    /// Include the coating transmission and Fresnel losses (unpolarized, the system coatings; the pupil-averaged transmission relative to the axis)
    pub polarization: bool,
    /// Replace the vignetting factors with floating apertures at the current semi-diameters (OpticStudio's default Remove Vignetting Factors); off ignores the vignetting factors and keeps the surface apertures as they are
    pub remove_vignetting_factors: bool,
}

impl Default for RelIllumParams {
    fn default() -> Self {
        RelIllumParams {
            wave: None,
            points: 21,
            axis: Axis::Y,
            rim: crate::illum::RIM_POINTS,
            polarization: false,
            remove_vignetting_factors: true,
        }
    }
}

/// 相対照度の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RelIllumOut {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    #[serde(deserialize_with = "crate::nan::v")]
    pub h: Vec<f64>,
    #[serde(deserialize_with = "crate::nan::v")]
    pub field_value: Vec<f64>,
    /// 相対照度（軸上の像点を 1、演算子 RELI と同じ。polarization なら透過率の比も掛ける）
    #[serde(deserialize_with = "crate::nan::v")]
    pub relative: Vec<f64>,
    /// 実効 F ナンバー（演算子 EFNO と同じ）
    #[serde(deserialize_with = "crate::nan::v")]
    pub effective_fno: Vec<f64>,
    /// 瞳の上で平均した膜・フレネルの透過率（polarization のときだけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::nan::ov")]
    pub transmission: Option<Vec<f64>>,
    pub warnings: Vec<String>,
}

/// 視野 f の瞳の上で平均した強度の透過率（膜・フレネル、偏光なし = x・y の偏光の平均）。入力の偏光は光線に垂直な面へ
/// 射影した後の強度で割る（斜めの光線で射影の分だけ小さく出さない。OpticStudio の Relative Illumination の Use
/// Polarization と同じく、軸外でも透過率だけの比になる）
fn mean_transmission(s: &System, f: FieldSel, w: usize) -> Result<f64> {
    use lensbench_core::coating::C64;
    use lensbench_core::system::{Launch, PolIn, RecSel};
    let co = lensbench_core::coating::Coatings::for_system(s)?;
    let c = s.try_trace(f, &[0.0], &[0.0], Some(w), None, false)?;
    let (hx, hy) = c.hx_hy;
    let (px, py) = lensbench_core::opd::pupil_grid(21);
    let l = Launch::Pupil {
        spec: s.object_spec(hx, hy)?,
        signed: false,
        px: &px,
        py: &py,
        vig: Some(s.vig_factors(hx, hy)?),
        hxy: Some((hx, hy)),
    };
    let (one, zero) = (C64::new(1.0, 0.0), C64::new(0.0, 0.0));
    let p = s.trace_pol_launch(
        &l,
        w,
        PolIn::new(&[[one, zero], [zero, one]], &co).no_phase(),
        RecSel::Off,
    )?;
    let (mut sum, mut n) = (0.0, 0usize);
    for k in 0..px.len() {
        if !p.trace.ok(k) {
            continue;
        }
        let d = p.trace.k0[k];
        let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let (kx, ky) = (d[0] / r, d[1] / r);
        let e = &p.e[k];
        let i = |v: &[C64; 3]| v.iter().map(|c| c.norm_sqr()).sum::<f64>();
        let t = 0.5 * (i(&e[0]) / (1.0 - kx * kx) + i(&e[1]) / (1.0 - ky * ky));
        if t.is_finite() {
            sum += t;
            n += 1;
        }
    }
    if n == 0 {
        return Err(Error::Invalid(
            "no ray of the pupil has a transmission".into(),
        ));
    }
    Ok(sum / n as f64)
}

pub(super) struct RelIllum;

impl Analysis for RelIllum {
    type Params = RelIllumParams;
    type Output = RelIllumOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "relative-illumination",
            about: "Relative illumination versus field (image irradiance relative to the axis) and effective F-number",
            help: "Relative illumination (OpticStudio's Relative Illumination): for points steps of the normalized field 0 to 1, the irradiance at the image point of the chief ray relative to the axis, from the projected solid angle of the beam that reaches that image point (the RELI operand), and the effective F-number (EFNO). By default the vignetting factors are replaced with floating apertures at the current semi-diameters on the surfaces where the medium changes (OpticStudio's Remove Vignetting Factors); with remove_vignetting_factors off the vignetting factors are ignored and the surface apertures are kept as they are. With polarization the ratio of the pupil-averaged coating and Fresnel transmission to the axis is multiplied in, and the effective F-number includes the transmission (F/√T)",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &RelIllumParams) -> Result<RelIllumOut> {
        let replaced = if p.remove_vignetting_factors {
            Some(crate::illum::replace_vignetting(cx.system)?)
        } else {
            None
        };
        let s = replaced.as_ref().unwrap_or(cx.system);
        let w = wave(s, p.wave)?.unwrap_or(s.primary);
        let ax = p.axis.core();
        let h = steps01(p.points.max(2));
        let mut warnings = Vec::new();
        let a0 = crate::illum::exit_cone(s, FieldSel::Norm(0.0, 0.0), w, p.rim)?;
        let t0 = if p.polarization {
            Some(mean_transmission(s, FieldSel::Norm(0.0, 0.0), w)?)
        } else {
            None
        };
        let (mut rel, mut fno, mut trans) = (Vec::new(), Vec::new(), Vec::new());
        for &hh in &h {
            let f = ax.sel(hh);
            match crate::illum::exit_cone(s, f, w, p.rim) {
                Ok(a) => {
                    rel.push(a / a0);
                    fno.push(1.0 / (2.0 * (a / std::f64::consts::PI).sqrt()));
                }
                Err(e) => {
                    warnings.push(format!("field {}: {e}", f3(hh)));
                    rel.push(f64::NAN);
                    fno.push(f64::NAN);
                }
            }
            if let Some(t0) = t0 {
                let t = mean_transmission(s, f, w).unwrap_or_else(|e| {
                    warnings.push(format!("field {}: {e}", f3(hh)));
                    f64::NAN
                });
                trans.push(t);
                if let Some(r) = rel.last_mut() {
                    *r *= t / t0;
                }
                // 透過を含めた実効 F ナンバー F/√T（T ナンバー。OpticStudio の Use Polarization と同じ）
                if let Some(n) = fno.last_mut() {
                    *n /= t.sqrt();
                }
            }
        }
        Ok(RelIllumOut {
            wave: w + 1,
            field_value: h.iter().map(|&x| ax.value(s, x)).collect(),
            h,
            relative: rel,
            effective_fno: fno,
            transmission: t0.map(|_| trans),
            warnings,
        })
    }
}

/// 注意の文の視野の値（小数 3 桁）
fn f3(x: f64) -> String {
    format!("{x:.3}")
}

// ----------------------------------------------------------------------------------- vignetting

/// けられの図の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct VignettingParams {
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Number of field points (normalized field 0 to 1)
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
    /// Field direction (y or x)
    pub axis: Axis,
    /// Pupil ray grid (rays across the pupil diameter)
    #[schemars(extend("x-min" = 2))]
    pub rays: usize,
    /// Ignore the vignetting factors of the fields (only the surface apertures clip the full pupil; when remove_vignetting_factors is off)
    pub no_vignetting_factors: bool,
    /// Replace the vignetting factors with floating apertures at the current semi-diameters (OpticStudio's default Remove Vignetting Factors)
    pub remove_vignetting_factors: bool,
}

impl Default for VignettingParams {
    fn default() -> Self {
        VignettingParams {
            wave: None,
            points: 21,
            axis: Axis::Y,
            rays: 64,
            no_vignetting_factors: false,
            remove_vignetting_factors: true,
        }
    }
}

/// けられの図の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VignettingOut {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    #[serde(deserialize_with = "crate::nan::v")]
    pub h: Vec<f64>,
    #[serde(deserialize_with = "crate::nan::v")]
    pub field_value: Vec<f64>,
    /// けられずに像面まで通る光線の割合
    #[serde(deserialize_with = "crate::nan::v")]
    pub unvignetted: Vec<f64>,
    pub warnings: Vec<String>,
}

pub(super) struct Vignetting;

impl Analysis for Vignetting {
    type Params = VignettingParams;
    type Output = VignettingOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "vignetting",
            about: "Vignetting plot: fraction of unvignetted rays versus field",
            help: "Vignetting plot (OpticStudio's Vignetting Plot): for points steps of the normalized field 0 to 1, the fraction of a grid of pupil rays (rays across the diameter) that reach the image unvignetted. By default the vignetting factors are replaced with floating apertures at the current semi-diameters on the surfaces where the medium changes (OpticStudio's Remove Vignetting Factors). With remove_vignetting_factors off, the vignetting factors of the fields are used unless no_vignetting_factors (then the full pupil is clipped only by the surface apertures)",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &VignettingParams) -> Result<VignettingOut> {
        let replaced = if p.remove_vignetting_factors {
            Some(crate::illum::replace_vignetting(cx.system)?)
        } else {
            None
        };
        let s = replaced.as_ref().unwrap_or(cx.system);
        let w = wave(s, p.wave)?.unwrap_or(s.primary);
        let ax = p.axis.core();
        let h = steps01(p.points.max(2));
        let mut warnings = Vec::new();
        let unvignetted = h
            .iter()
            .map(|&hh| {
                footprint::unvignetted(s, ax.sel(hh), w, p.rays, !p.no_vignetting_factors)
                    .unwrap_or_else(|e| {
                        warnings.push(format!("field {}: {e}", f3(hh)));
                        f64::NAN
                    })
            })
            .collect();
        Ok(VignettingOut {
            wave: w + 1,
            field_value: h.iter().map(|&x| ax.value(s, x)).collect(),
            h,
            unvignetted,
            warnings,
        })
    }
}
