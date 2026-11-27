//! 収差図: 横収差・OPD の扇、像面湾曲と歪曲、縦収差、倍率色収差、色の焦点ずれ、グリッド歪曲、ザイデル係数。
//!
//! 計算は評価関数の演算子と同じ（[`crate::aber`]・`lensbench_core::seidel`）。波長を省くと波長ごとに（重み付きの多色ではない）。

use lensbench_core::seidel::{self, DistComp};
use lensbench_core::system::FieldType;
use lensbench_core::{Error, FieldSel, Result, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, spot_unit, wave, Axis};
use crate::aber;
use crate::registry::{Analysis, Cx, Meta};
use crate::sweep::steps01;

/// 波長の番号（None は全波長）の並び（0 始まり）
fn waves(s: &System, w: Option<usize>) -> Result<Vec<usize>> {
    Ok(match wave(s, w)? {
        Some(w) => vec![w],
        None => (0..s.wavelengths.len()).collect(),
    })
}

/// 注意の文の視野の値（小数 3 桁）
fn f3(x: f64) -> String {
    format!("{x:.3}")
}

/// 長さの単位（アフォーカル像空間の系の横の量は角度の単位。縦の量は長さのまま）
fn len_unit() -> String {
    "mm".into()
}

/// 焦点のずれの単位（アフォーカル像空間の系は視度。OpticStudio と同じ）
fn shift_unit(s: &System) -> String {
    if s.afocal { "diopters" } else { "mm" }.into()
}

// ------------------------------------------------------------------------------- ray-fan, opd-fan

/// 扇の設定（ray-fan・opd-fan）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FanParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; every wavelength if omitted)
    pub wave: Option<usize>,
    /// Number of rays on each side of the pupil center (2 rays + 1 points from -1 to 1, as OpticStudio's Number of Rays)
    #[schemars(extend("x-min" = 1))]
    pub rays: usize,
}

impl Default for FanParams {
    fn default() -> Self {
        FanParams {
            field: None,
            wave: None,
            rays: 20,
        }
    }
}

/// 扇の 1 本（1 視野・1 波長）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FanRow {
    /// Field number (from 1)
    pub field: usize,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 正規化瞳座標（子午は Py、球欠は Px）
    #[serde(deserialize_with = "crate::nan::v")]
    pub p: Vec<f64>,
    /// 子午の扇（瞳 (0, p)）。けられた光線は null
    #[serde(deserialize_with = "crate::nan::v")]
    pub tangential: Vec<f64>,
    /// 球欠の扇（瞳 (p, 0)）。けられた光線は null
    #[serde(deserialize_with = "crate::nan::v")]
    pub sagittal: Vec<f64>,
}

/// 扇の結果（ray-fan・opd-fan）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FanOut {
    /// 値の単位（ray-fan は像面の点の単位、opd-fan は waves）
    pub unit: String,
    pub fans: Vec<FanRow>,
    /// 求められなかった視野・波長
    pub warnings: Vec<String>,
}

fn fans(
    s: &System,
    p: &FanParams,
    unit: String,
    f: fn(&System, FieldSel, usize, &[f64]) -> Result<aber::Fan>,
) -> Result<FanOut> {
    let pts = aber::fan_points(p.rays);
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for i in fields(s, p.field)? {
        for w in waves(s, p.wave)? {
            match f(s, FieldSel::Index(i), w, &pts) {
                Ok(v) => out.push(FanRow {
                    field: i + 1,
                    wave: w + 1,
                    p: v.p,
                    tangential: v.tan,
                    sagittal: v.sag,
                }),
                Err(e) => warnings.push(format!(
                    "field {}, wavelength {}: {e}",
                    i + 1,
                    w + 1,
                    e = e.warning_text()
                )),
            }
        }
    }
    if out.is_empty() {
        return Err(Error::Invalid(warnings.join("; ")));
    }
    Ok(FanOut {
        unit,
        fans: out,
        warnings,
    })
}

pub(super) struct RayFan;

impl Analysis for RayFan {
    type Params = FanParams;
    type Output = FanOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "ray-fan",
            about: "Ray aberration fans: transverse ray error versus pupil coordinate (tangential ey vs Py, sagittal ex vs Px)",
            help: "Ray aberration fans (OpticStudio's Ray Fan): for each field and wavelength, the image position of the rays along the pupil y axis (tangential) and x axis (sagittal), relative to the chief ray of the primary wavelength (the TRAY and TRAX operands). Vignetted rays are null. In afocal image space the values are direction angles",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &FanParams) -> Result<FanOut> {
        let s = cx.system;
        fans(s, p, spot_unit(s), aber::ray_fan)
    }
}

pub(super) struct OpdFan;

impl Analysis for OpdFan {
    type Params = FanParams;
    type Output = FanOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "opd-fan",
            about: "OPD fans: optical path difference [waves] versus pupil coordinate (tangential and sagittal)",
            help: "OPD fans (OpticStudio's OPD Fan): for each field and wavelength, the optical path difference in waves along the pupil y axis (tangential) and x axis (sagittal). The reference sphere is centered on the chief ray of the primary wavelength and the path is referenced to the chief ray of each wavelength (the OPDC operand). Vignetted rays are null",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &FanParams) -> Result<FanOut> {
        fans(cx.system, p, "waves".into(), aber::opd_fan)
    }
}

pub(super) struct PupilFan;

impl Analysis for PupilFan {
    type Params = FanParams;
    type Output = FanOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "pupil-fan",
            about: "Pupil aberration fans: where real rays cross the stop against the paraxial point [% of the stop radius]",
            help: "Pupil aberration fans (OpticStudio's Pupil Aberration Fan): for each field and wavelength, real rays aimed at the paraxial entrance pupil coordinates along the pupil y axis (tangential) and x axis (sagittal) without ray aiming, and the y (tangential) or x (sagittal) of the point where they cross the stop surface, as the paraxial point (pupil coordinate × paraxial stop radius of the primary wavelength) minus the real point in percent of the stop radius. Rays vignetted before the stop are null",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &FanParams) -> Result<FanOut> {
        // エイミングをすると実光線が絞りの近軸の点を通るので、エイミングを外して追う（OpticStudio と同じ）
        let mut s = cx.system.clone();
        if s.aiming != lensbench_core::system::Aiming::Off {
            s.edit(|q| {
                q.aiming = lensbench_core::system::Aiming::Off;
                Ok(())
            })?;
        }
        fans(&s, p, "%".into(), aber::pupil_fan)
    }
}

// ---------------------------------------------------------------------------- field-curvature

/// 歪曲の理想の像高
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DistKind {
    /// f tan θ（ふつうの歪曲、演算子 DIST・DISG）
    #[default]
    FTan,
    /// f θ（角度の視野だけ。f は近軸の f tan θ と同じ傾き）
    FTheta,
}

/// 像面湾曲・歪曲の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FieldCurvParams {
    /// Wavelength number (from 1; every wavelength if omitted)
    pub wave: Option<usize>,
    /// Number of field points (normalized field 0 to 1)
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
    /// Field direction (y or x)
    pub axis: Axis,
    /// Ideal image height of the distortion: f-tan (f tan θ) or f-theta (f θ, angle fields only)
    pub distortion: DistKind,
}

impl Default for FieldCurvParams {
    fn default() -> Self {
        FieldCurvParams {
            wave: None,
            points: 21,
            axis: Axis::Y,
            distortion: DistKind::FTan,
        }
    }
}

/// 像面湾曲・歪曲の 1 波長
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CurvWave {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 子午の焦点の像面からの z [mm]（演算子 FCGT）
    #[serde(deserialize_with = "crate::nan::v")]
    pub tangential: Vec<f64>,
    /// 球欠の焦点の像面からの z [mm]（演算子 FCGS）
    #[serde(deserialize_with = "crate::nan::v")]
    pub sagittal: Vec<f64>,
    /// 歪曲 [%]（求められなければ null）
    #[serde(deserialize_with = "crate::nan::v")]
    pub distortion: Vec<f64>,
}

/// 像面湾曲・歪曲の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FieldCurvOut {
    /// 焦点のずれの単位
    pub unit: String,
    /// 正規化視野
    #[serde(deserialize_with = "crate::nan::v")]
    pub h: Vec<f64>,
    /// 視野の値（系の視野の単位）
    #[serde(deserialize_with = "crate::nan::v")]
    pub field_value: Vec<f64>,
    pub distortion_kind: DistKind,
    pub waves: Vec<CurvWave>,
    /// 主波長の歪曲の絶対値の最大 [%]
    #[serde(deserialize_with = "crate::nan::f")]
    pub max_distortion: f64,
    pub warnings: Vec<String>,
}

pub(super) struct FieldCurv;

impl Analysis for FieldCurv {
    type Params = FieldCurvParams;
    type Output = FieldCurvOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "field-curvature",
            about: "Field curvature and distortion versus field (tangential and sagittal focus, distortion %)",
            help: "Field curvature and distortion (OpticStudio's Field Curvature/Distortion): for points steps of the normalized field 0 to 1, the z of the tangential and sagittal focus of the rays near the chief ray measured from the image surface (the FCGT and FCGS operands), and the distortion of the real chief ray against the paraxial ideal height f tan θ (or f θ). Every wavelength if wave is omitted. Vignetting factors are not used, as in OpticStudio. In afocal image space the focus is in diopters (the inverse of the distance in meters along the chief ray, negative for a virtual focus before the image surface) and the distortion compares the direction tangents of the chief ray with the paraxial angular magnification, as in OpticStudio",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &FieldCurvParams) -> Result<FieldCurvOut> {
        let (s, vig) = crate::sweep::without_vig(cx.system);
        let s = &*s;
        if p.distortion == DistKind::FTheta && s.field_type != FieldType::Angle {
            return Err(Error::Invalid(
                "the f-theta distortion needs angle fields".into(),
            ));
        }
        let ax = p.axis.core();
        let h = steps01(p.points.max(2));
        let k = usize::from(p.axis == Axis::Y);
        let mut warnings: Vec<String> = vig.into_iter().collect();
        let mut out = Vec::new();
        let mut max_distortion = f64::NAN;
        for w in waves(s, p.wave)? {
            let (mut tan, mut sag, mut dist) = (Vec::new(), Vec::new(), Vec::new());
            let r = match seidel::distortion_ref(s, w, None) {
                Ok(r) => Some(r),
                Err(e) => {
                    warnings.push(format!(
                        "wavelength {}: distortion: {e}",
                        w + 1,
                        e = e.warning_text()
                    ));
                    None
                }
            };
            for &hh in &h {
                let f = ax.sel(hh);
                let focus = if s.afocal {
                    aber::field_focus_diopters
                } else {
                    aber::field_focus
                };
                let mut get = |t: bool| match focus(s, f, w, s.image(), t) {
                    Ok(z) => z,
                    Err(e) => {
                        warnings.push(format!(
                            "wavelength {}, field {}: {e}",
                            w + 1,
                            f3(hh),
                            e = e.warning_text()
                        ));
                        f64::NAN
                    }
                };
                tan.push(get(true));
                sag.push(get(false));
                let d = match &r {
                    _ if hh == 0.0 => Ok(0.0),
                    None => Ok(f64::NAN),
                    Some(r) => distortion_pct(s, w, ax.sel(hh), r, k, p.distortion),
                };
                dist.push(d.unwrap_or_else(|e| {
                    warnings.push(format!(
                        "wavelength {}, field {}: {e}",
                        w + 1,
                        f3(hh),
                        e = e.warning_text()
                    ));
                    f64::NAN
                }));
            }
            if w == s.primary || p.wave.is_some() {
                max_distortion = dist
                    .iter()
                    .filter(|d| d.is_finite())
                    .fold(0.0_f64, |m, d| m.max(d.abs()));
            }
            out.push(CurvWave {
                wave: w + 1,
                tangential: tan,
                sagittal: sag,
                distortion: dist,
            });
        }
        warnings.dedup();
        Ok(FieldCurvOut {
            unit: shift_unit(s),
            field_value: h.iter().map(|&x| ax.value(s, x)).collect(),
            h,
            distortion_kind: p.distortion,
            waves: out,
            max_distortion,
            warnings,
        })
    }
}

/// 視野 f の歪曲 [%]: 成分 k（0 = x、1 = y）の実主光線の像高と理想の像高の差の、理想の像高に対する割合
fn distortion_pct(
    s: &System,
    w: usize,
    f: FieldSel,
    r: &seidel::DistRef,
    k: usize,
    kind: DistKind,
) -> Result<f64> {
    let h = s.field_sel_norm(f, w)?;
    match kind {
        DistKind::FTan => Ok(seidel::distortion_against(s, w, h, r, DistComp::Radial)?.1),
        DistKind::FTheta => {
            let (real, ideal) = seidel::distortion_points(s, w, h, r)?;
            // 軸上の基準なので理想の点は J t。f θ の理想は J atan(t)
            let m = r.j[k][k];
            let th = m * (ideal[k] / m).atan();
            Ok(100.0 * (real[k] - th) / th)
        }
    }
}

// ------------------------------------------------------------------------------- longitudinal

/// 縦収差の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LongParams {
    /// Wavelength number (from 1; every wavelength if omitted)
    pub wave: Option<usize>,
    /// Number of pupil zones (normalized pupil height 0 to 1; zone 0 is paraxial)
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
}

impl Default for LongParams {
    fn default() -> Self {
        LongParams {
            wave: None,
            points: 21,
        }
    }
}

/// 縦収差の 1 波長
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LongWave {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 軸上の光線が光軸を横切る点の、像面の頂点からの距離（演算子 LONA）
    #[serde(deserialize_with = "crate::nan::v")]
    pub z: Vec<f64>,
}

/// 縦収差の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LongOut {
    pub unit: String,
    /// 正規化瞳の高さ（0 は近軸）
    #[serde(deserialize_with = "crate::nan::v")]
    pub zones: Vec<f64>,
    pub waves: Vec<LongWave>,
    pub warnings: Vec<String>,
}

pub(super) struct Longitudinal;

impl Analysis for Longitudinal {
    type Params = LongParams;
    type Output = LongOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "longitudinal",
            about: "Longitudinal aberration versus pupil height (where on-axis rays cross the axis)",
            help: "Longitudinal aberration (OpticStudio's Longitudinal Aberration): for points zones of the normalized pupil height 0 to 1, the distance from the image surface vertex to where the on-axis ray crosses the optical axis (the LONA operand; zone 0 is the paraxial marginal ray). Every wavelength if wave is omitted",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &LongParams) -> Result<LongOut> {
        let s = cx.system;
        let zones = steps01(p.points.max(2));
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        for w in waves(s, p.wave)? {
            let z = zones
                .iter()
                .map(|&z| {
                    seidel::longitudinal(s, w, z).unwrap_or_else(|e| {
                        warnings.push(format!("wavelength {}: {e}", w + 1, e = e.warning_text()));
                        f64::NAN
                    })
                })
                .collect();
            out.push(LongWave { wave: w + 1, z });
        }
        Ok(LongOut {
            unit: len_unit(),
            zones,
            waves: out,
            warnings,
        })
    }
}

// ------------------------------------------------------------------------------- lateral-color

/// 倍率色収差の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LatParams {
    /// Number of field points (normalized field 0 to 1)
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
    /// Field direction (y or x)
    pub axis: Axis,
}

impl Default for LatParams {
    fn default() -> Self {
        LatParams {
            points: 21,
            axis: Axis::Y,
        }
    }
}

/// 倍率色収差の 1 波長
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LatWave {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 実主光線の像高（視野の向きの成分）の、主波長の実主光線との差
    #[serde(deserialize_with = "crate::nan::v")]
    pub d: Vec<f64>,
}

/// 倍率色収差の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LatOut {
    pub unit: String,
    #[serde(deserialize_with = "crate::nan::v")]
    pub h: Vec<f64>,
    #[serde(deserialize_with = "crate::nan::v")]
    pub field_value: Vec<f64>,
    /// 主波長以外の波長
    pub waves: Vec<LatWave>,
    /// 主波長のエアリー半径 1.22 λ F/#（作動 F 値。アフォーカル像空間では null）
    #[serde(deserialize_with = "crate::nan::f")]
    pub airy: f64,
    pub warnings: Vec<String>,
}

pub(super) struct LateralColor;

impl Analysis for LateralColor {
    type Params = LatParams;
    type Output = LatOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "lateral-color",
            about: "Lateral color versus field (real chief ray height of each wavelength minus the primary wavelength)",
            help: "Lateral color (OpticStudio's Lateral Color): for points steps of the normalized field 0 to 1, the difference of the real chief ray image height (the component along the field direction) of each wavelength from the primary wavelength, with the Airy radius of the primary wavelength. In afocal image space the values are direction angles",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &LatParams) -> Result<LatOut> {
        let s = cx.system;
        if s.wavelengths.len() < 2 {
            return Err(Error::Invalid(
                "lateral color needs two or more wavelengths".into(),
            ));
        }
        let ax = p.axis.core();
        let h = steps01(p.points.max(2));
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        for w in (0..s.wavelengths.len()).filter(|&w| w != s.primary) {
            let d = h
                .iter()
                .map(|&hh| match aber::lateral_color(s, ax.sel(hh), w) {
                    Ok((dx, dy)) => {
                        if p.axis == Axis::Y {
                            dy
                        } else {
                            dx
                        }
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "wavelength {}, field {}: {e}",
                            w + 1,
                            f3(hh),
                            e = e.warning_text()
                        ));
                        f64::NAN
                    }
                })
                .collect();
            out.push(LatWave { wave: w + 1, d });
        }
        let airy = if s.afocal {
            f64::NAN
        } else {
            1.22 * s.wave(s.primary).mm() * lensbench_core::report::system_data(s).wfno
        };
        Ok(LatOut {
            unit: spot_unit(s),
            field_value: h.iter().map(|&x| ax.value(s, x)).collect(),
            h,
            waves: out,
            airy,
            warnings,
        })
    }
}

// --------------------------------------------------------------------------------- focal-shift

/// 色の焦点ずれの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ShiftParams {
    /// Shortest wavelength [µm] (the shortest system wavelength if omitted)
    pub min: Option<f64>,
    /// Longest wavelength [µm] (the longest system wavelength if omitted)
    pub max: Option<f64>,
    /// Number of wavelength points
    #[schemars(extend("x-min" = 2))]
    pub points: usize,
}

impl Default for ShiftParams {
    fn default() -> Self {
        ShiftParams {
            min: None,
            max: None,
            points: 41,
        }
    }
}

/// 色の焦点ずれの結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShiftOut {
    pub unit: String,
    /// 波長 [µm]
    #[serde(deserialize_with = "crate::nan::v")]
    pub wavelengths: Vec<f64>,
    /// 近軸焦点の、主波長の近軸焦点からのずれ
    #[serde(deserialize_with = "crate::nan::v")]
    pub shift: Vec<f64>,
    /// ずれの最大と最小の差
    #[serde(deserialize_with = "crate::nan::f")]
    pub range: f64,
    /// 回折限界の焦点深度 4 λ (F/#)²（主波長、作動 F 値）
    #[serde(deserialize_with = "crate::nan::f")]
    pub diffraction_depth: f64,
    pub warnings: Vec<String>,
}

pub(super) struct FocalShift;

impl Analysis for FocalShift {
    type Params = ShiftParams;
    type Output = ShiftOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "focal-shift",
            about: "Chromatic focal shift versus wavelength (paraxial focus relative to the primary wavelength)",
            help: "Chromatic focal shift (OpticStudio's Chromatic Focal Shift): for points wavelengths from min to max, the shift of the paraxial focus from the paraxial focus of the primary wavelength (the difference of the LONA zone 0 operand, as AXCL), with the diffraction-limited depth 4 λ (F/#)². In afocal image space the shift is in diopters (the difference of the inverse distances of the paraxial focus from the image surface in meters) and the depth is 0, as in OpticStudio",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ShiftParams) -> Result<ShiftOut> {
        let s = cx.system;
        let lo = p
            .min
            .unwrap_or_else(|| s.wavelengths.iter().copied().fold(f64::INFINITY, f64::min));
        let hi = p
            .max
            .unwrap_or_else(|| s.wavelengths.iter().copied().fold(0.0, f64::max));
        if !(lo > 0.0 && hi >= lo) {
            return Err(Error::Invalid(format!("wavelength range {lo} to {hi} µm")));
        }
        let n = p.points.max(2);
        let wl: Vec<f64> = (0..n)
            .map(|k| lo + (hi - lo) * k as f64 / (n - 1) as f64)
            .collect();
        let mut warnings = Vec::new();
        let shift: Vec<f64> = wl
            .iter()
            .map(|&l| {
                let f = if s.afocal {
                    aber::focal_shift_diopters
                } else {
                    aber::focal_shift
                };
                f(s, l).unwrap_or_else(|e| {
                    warnings.push(format!("{l} µm: {e}", e = e.warning_text()));
                    f64::NAN
                })
            })
            .collect();
        let fin = shift.iter().filter(|v| v.is_finite());
        let range = fin.clone().fold(f64::NEG_INFINITY, |a, &b| a.max(b))
            - fin.fold(f64::INFINITY, |a, &b| a.min(b));
        let f = lensbench_core::report::system_data(s).wfno;
        Ok(ShiftOut {
            unit: shift_unit(s),
            wavelengths: wl,
            shift,
            range,
            // アフォーカル像空間では 0（OpticStudio と同じ）
            diffraction_depth: if s.afocal {
                0.0
            } else {
                4.0 * s.wave(s.primary).mm() * f * f
            },
            warnings,
        })
    }
}

// ----------------------------------------------------------------------------- grid-distortion

/// グリッド歪曲の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct GridParams {
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Grid size (points on a side of the square of normalized fields -1 to 1)
    #[schemars(extend("x-min" = 3))]
    pub grid: usize,
}

impl Default for GridParams {
    fn default() -> Self {
        GridParams {
            wave: None,
            grid: 9,
        }
    }
}

/// グリッド歪曲の 1 点
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GridPoint {
    /// 正規化視野
    #[serde(deserialize_with = "crate::nan::f")]
    pub hx: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub hy: f64,
    /// 実主光線の像面の点 (x, y)
    #[serde(deserialize_with = "crate::nan::a2")]
    pub real: [f64; 2],
    /// 近軸の理想の点 (x, y)
    #[serde(deserialize_with = "crate::nan::a2")]
    pub ideal: [f64; 2],
    /// 歪曲 [%]（動径、演算子 DISG の軸上の基準と同じ）
    #[serde(deserialize_with = "crate::nan::f")]
    pub distortion: f64,
}

/// グリッド歪曲の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GridOut {
    pub unit: String,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    pub grid: usize,
    pub points: Vec<GridPoint>,
    /// 歪曲の絶対値の最大 [%] と、その点の正規化視野
    #[serde(deserialize_with = "crate::nan::f")]
    pub max_distortion: f64,
    #[serde(deserialize_with = "crate::nan::a2")]
    pub max_at: [f64; 2],
    pub warnings: Vec<String>,
}

pub(super) struct GridDistortion;

impl Analysis for GridDistortion {
    type Params = GridParams;
    type Output = GridOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "grid-distortion",
            about: "Grid distortion: real chief ray image points of a square grid of fields against the paraxial ideal grid",
            help: "Grid distortion (OpticStudio's Grid Distortion): for a grid × grid square inscribed in the circle of the largest field (evenly spaced in tan θ for angle fields), the real chief ray image point and the ideal point from the paraxial magnification, with the distortion % (the DISG operand against the axis). In afocal image space the points are the direction tangents (l/n, m/n) of the image space ray and the ideal points come from the paraxial angular magnification, as in OpticStudio. Needs a rotationally symmetric system",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GridParams) -> Result<GridOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?.unwrap_or(s.primary);
        // 軸外の視野が無い系は、歪曲の基準を求める前に格子の視野の文で止める
        seidel::grid_field(s, 1.0, 1.0)?;
        let r = seidel::distortion_ref(s, w, None)?;
        let n = p.grid.max(3);
        let mut points = Vec::with_capacity(n * n);
        let mut warnings = Vec::new();
        let (mut max_distortion, mut max_at) = (0.0_f64, [0.0, 0.0]);
        for j in 0..n {
            for i in 0..n {
                let u = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
                let v = 1.0 - 2.0 * j as f64 / (n - 1) as f64;
                let (hx, hy) = seidel::grid_field(s, u, v)?;
                let pt = seidel::distortion_points(s, w, (hx, hy), &r).and_then(|(re, id)| {
                    let d = if u == 0.0 && v == 0.0 {
                        0.0
                    } else {
                        seidel::distortion_against(s, w, (hx, hy), &r, DistComp::Radial)?.1
                    };
                    Ok((re, id, d))
                });
                match pt {
                    Ok((real, ideal, d)) => {
                        if d.abs() > max_distortion.abs() {
                            (max_distortion, max_at) = (d, [hx, hy]);
                        }
                        points.push(GridPoint {
                            hx,
                            hy,
                            real,
                            ideal,
                            distortion: d,
                        });
                    }
                    Err(e) => warnings.push(format!(
                        "field ({}, {}): {e}",
                        f3(hx),
                        f3(hy),
                        e = e.warning_text()
                    )),
                }
            }
        }
        Ok(GridOut {
            // アフォーカル像空間では像空間の向きの tan (l/n, m/n)
            unit: if s.afocal { "tan".into() } else { len_unit() },
            wave: w + 1,
            grid: n,
            points,
            max_distortion,
            max_at,
            warnings,
        })
    }
}

// -------------------------------------------------------------------------------------- seidel

/// ザイデル係数の設定
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SeidelParams {
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
}

/// ザイデル和の 1 面（レンズ単位）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SeidelRow {
    /// 面の番号（和の行は 0）
    pub surf: usize,
    /// S_I 球面収差
    #[serde(deserialize_with = "crate::nan::f")]
    pub spha: f64,
    /// S_II コマ収差
    #[serde(deserialize_with = "crate::nan::f")]
    pub coma: f64,
    /// S_III 非点収差
    #[serde(deserialize_with = "crate::nan::f")]
    pub asti: f64,
    /// S_IV ペッツバール像面湾曲
    #[serde(deserialize_with = "crate::nan::f")]
    pub fcur: f64,
    /// S_V 歪曲
    #[serde(deserialize_with = "crate::nan::f")]
    pub dist: f64,
    /// C_L 軸上色収差（最短と最長の波長。波長が 1 つなら 0）
    #[serde(deserialize_with = "crate::nan::f")]
    pub cla: f64,
    /// C_T 倍率色収差
    #[serde(deserialize_with = "crate::nan::f")]
    pub ctr: f64,
}

/// 波面収差の係数の 1 面 [波]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SeidelWaves {
    pub surf: usize,
    #[serde(deserialize_with = "crate::nan::f")]
    pub w040: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub w131: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub w222: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub w220: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub w311: f64,
}

/// ザイデル係数の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SeidelOut {
    pub unit: String,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    /// 面ごと（物体面・像面・無視する面・座標ブレークは除く）
    pub surfaces: Vec<SeidelRow>,
    /// 全部の面の和
    pub sum: SeidelRow,
    /// 波面収差の係数（面ごと、和は surf 0）。演算子 SPHA・COMA・ASTI・FCUR・DIST と同じ
    pub waves: Vec<SeidelWaves>,
}

pub(super) struct SeidelCoef;

impl Analysis for SeidelCoef {
    type Params = SeidelParams;
    type Output = SeidelOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "seidel",
            about: "Seidel coefficients per surface (S_I to S_V, axial and lateral color) and wavefront coefficients [waves]",
            help: "Seidel coefficients (OpticStudio's Seidel Coefficients and Seidel Diagram): the third-order sums S_I to S_V per surface in lens units (Welford), the first-order axial color C_L and lateral color C_T between the shortest and longest wavelengths, and the wavefront coefficients W040, W131, W222, W220 and W311 in waves (the SPHA, COMA, ASTI, FCUR and DIST operands). Needs a rotationally symmetric system without mirrors",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &SeidelParams) -> Result<SeidelOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?.unwrap_or(s.primary);
        let sd = seidel::seidel(s, w)?;
        let by = |f: fn(&f64, &f64) -> bool| {
            let mut k = 0;
            for (i, l) in s.wavelengths.iter().enumerate() {
                if f(l, &s.wavelengths[k]) {
                    k = i;
                }
            }
            k
        };
        let (short, long) = (by(|a, b| a < b), by(|a, b| a > b));
        let col = seidel::chromatic(s, w, short, long)?;
        let row = |j: usize, v: [f64; 5], c: [f64; 2]| SeidelRow {
            surf: j,
            spha: v[0],
            coma: v[1],
            asti: v[2],
            fcur: v[3],
            dist: v[4],
            cla: c[0],
            ctr: c[1],
        };
        let wrow = |j: usize| {
            let v = sd.waves(j);
            SeidelWaves {
                surf: j,
                w040: v[0],
                w131: v[1],
                w222: v[2],
                w220: v[3],
                w311: v[4],
            }
        };
        let used = |j: usize| {
            let f = &s.surfaces[j];
            !(f.ignore || f.is_coord_break())
        };
        let js: Vec<usize> = (1..s.image()).filter(|&j| used(j)).collect();
        let csum = col.iter().fold([0.0; 2], |a, c| [a[0] + c[0], a[1] + c[1]]);
        let mut waves = vec![wrow(0)];
        waves.extend(js.iter().map(|&j| wrow(j)));
        Ok(SeidelOut {
            unit: len_unit(),
            wave: w + 1,
            surfaces: js.iter().map(|&j| row(j, sd.at(j), col[j])).collect(),
            sum: row(0, sd.at(0), csum),
            waves,
        })
    }
}
