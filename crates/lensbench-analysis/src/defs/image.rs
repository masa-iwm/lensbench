//! RMS の掃引・PSF

use crate::huygens::{self, HuygensDevice, HuygensOpts};
use crate::psf::{self, PsfOpts, PsfPlane};
use crate::sweep::{self, RmsOpts, RmsRef};
use lensbench_core::pupil::PupilAmp;
use lensbench_core::Waves;
use lensbench_core::{Error, FieldSel, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, pol_amp, spot_unit, wave, Axis};
use crate::registry::{Analysis, Cx, Meta};

// ----------------------------------------------------------------------------------------- rms

/// RMS の横軸
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Vs {
    /// 正規化視野 0〜1
    #[default]
    Field,
    /// 像面のずらし ±range
    Focus,
    /// 系の波長の最小〜最大（単色）
    Wavelength,
}

/// RMS の基準
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Reference {
    /// 重心
    #[default]
    Centroid,
    /// 主光線
    Chief,
}

/// RMS の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RmsParams {
    /// Field number (from 1; all fields if omitted; not used with vs field)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted; not used with vs wavelength)
    pub wave: Option<usize>,
    /// Horizontal axis: field (normalized field 0 to 1), focus (image shift ±range), wavelength (system wavelength minimum to maximum, monochromatic)
    pub vs: Vs,
    /// Number of points on the horizontal axis
    #[schemars(extend("x-min" = 1))]
    pub points: usize,
    /// Shift range of focus ±range [lens units]
    pub range: f64,
    /// Field direction for field (y or x)
    pub axis: Axis,
    /// Reference: centroid or chief (chief ray)
    pub reference: Reference,
    /// Number of rings of the Gaussian pupil quadrature
    #[schemars(extend("x-min" = 1))]
    pub rings: usize,
    /// In monochromatic field and wavelength sweeps, solve fields given by real image height (and fields defined by conditions) with the chief ray of the analysis wavelength
    /// (Same as OpticStudio's monochromatic RMS vs Field and vs Wavelength; solved at the primary wavelength if omitted)
    pub field_wave: bool,
}

impl Default for RmsParams {
    fn default() -> Self {
        RmsParams {
            field: None,
            wave: None,
            vs: Vs::Field,
            points: 11,
            range: 0.1,
            axis: Axis::Y,
            reference: Reference::Centroid,
            rings: 6,
            field_wave: false,
        }
    }
}

/// RMS の 1 点
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RmsRow {
    /// スポット半径の RMS
    #[serde(deserialize_with = "crate::nan::f")]
    pub spot: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub spot_x: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub spot_y: f64,
    /// 波面収差の RMS [波]
    #[serde(deserialize_with = "crate::nan::f")]
    pub wfe: f64,
    /// ストレール比の近似
    #[serde(deserialize_with = "crate::nan::f")]
    pub strehl: f64,
    /// 正規化視野（vs field）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// 視野の値（vs field）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_value: Option<f64>,
    /// 像面のずらし（vs focus）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dz: Option<f64>,
    /// 波長 [µm]（vs wavelength）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wavelength: Option<f64>,
    /// 視野の番号（vs focus・wavelength）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<usize>,
}

impl RmsRow {
    fn of(r: &sweep::Rms) -> RmsRow {
        RmsRow {
            spot: r.spot,
            spot_x: r.spot_x,
            spot_y: r.spot_y,
            wfe: r.wfe,
            strehl: r.strehl,
            h: None,
            field_value: None,
            dz: None,
            wavelength: None,
            field: None,
        }
    }
}

/// RMS の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RmsOut {
    pub unit: String,
    pub wfe_unit: String,
    pub vs: Vs,
    pub reference: Reference,
    pub rms: Vec<RmsRow>,
    pub warnings: Vec<String>,
}

pub(super) struct Rms;

impl Analysis for Rms {
    type Params = RmsParams;
    type Output = RmsOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "rms",
            about: "RMS (spot radius, x, y, wavefront, approximate Strehl ratio) against field, focus or wavelength",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &RmsParams) -> Result<RmsOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let o = RmsOpts {
            rings: p.rings,
            field_wave: p.field_wave,
            reference: match p.reference {
                Reference::Centroid => RmsRef::Centroid,
                Reference::Chief => RmsRef::Chief,
            },
        };
        let (mut v, mut warn) = (Vec::new(), Vec::new());
        match p.vs {
            Vs::Field => {
                if p.field.is_some() {
                    return Err(Error::Invalid("field is not used with vs field".into()));
                }
                let ax = p.axis.core();
                let r = sweep::rms_vs_field(s, ax, &sweep::steps01(p.points), Waves::all_or(w), o);
                for (x, q) in r.x.iter().zip(&r.rms) {
                    let mut e = RmsRow::of(q);
                    e.h = Some(*x);
                    e.field_value = Some(ax.value(s, *x));
                    v.push(e);
                }
                warn = r.warnings;
            }
            Vs::Focus => {
                for i in fields(s, p.field)? {
                    let r = sweep::rms_vs_focus(
                        s,
                        FieldSel::Index(i),
                        &sweep::steps_sym(p.range, p.points),
                        Waves::all_or(w),
                        o,
                    );
                    for (x, q) in r.x.iter().zip(&r.rms) {
                        let mut e = RmsRow::of(q);
                        e.dz = Some(*x);
                        e.field = Some(i + 1);
                        v.push(e);
                    }
                    warn.extend(
                        r.warnings
                            .iter()
                            .map(|x| format!("field {}: {e}", i + 1, e = x)),
                    );
                }
            }
            Vs::Wavelength => {
                if p.wave.is_some() {
                    return Err(Error::Invalid("wave is not used with vs wavelength".into()));
                }
                let lams = sweep::wavelength_steps(s, p.points);
                for i in fields(s, p.field)? {
                    let r = sweep::rms_vs_wavelength(s, FieldSel::Index(i), &lams, o);
                    for (x, q) in r.x.iter().zip(&r.rms) {
                        let mut e = RmsRow::of(q);
                        e.wavelength = Some(*x);
                        e.field = Some(i + 1);
                        v.push(e);
                    }
                    warn.extend(
                        r.warnings
                            .iter()
                            .map(|x| format!("field {}: {e}", i + 1, e = x)),
                    );
                }
            }
        }
        Ok(RmsOut {
            unit: spot_unit(s),
            wfe_unit: "waves".into(),
            vs: p.vs,
            reference: p.reference,
            rms: v,
            warnings: warn,
        })
    }
}

// ----------------------------------------------------------------------------------------- psf

/// PSF の像の面
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Plane {
    /// 像面の上（像面の局所座標）
    #[default]
    Image,
    /// 主光線に垂直な面（OpticStudio の FFT PSF と同じ）
    Chief,
}

/// PSF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PsfParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Huygens PSF (on the image surface; for tilted image surfaces and strongly aberrated systems; slower than FFT)
    pub huygens: bool,
    /// Pupil grid side (automatic if omitted: large enough that neighboring samples differ by π/2 or less in phase)
    pub pupil: Option<usize>,
    /// Image grid side (up to twice the pupil grid at the default spacing)
    #[schemars(extend("x-min" = 1))]
    pub image: usize,
    /// Image grid spacing [lens units; angle units in afocal image space] (1 / (2 × cutoff) if omitted)
    pub pixel: Option<f64>,
    /// Huygens PSF only: sum every pupil sample exactly at every image point (several times slower). By default each pupil sample's phase and amplitude are approximated by a quadratic over small blocks of the image grid; the block size is chosen so that the phase differs from the exact value by at most 1e-6 rad, checked on the corner and center blocks of the grid (the intensity typically agrees with the exact sum to 1e-7 of the unaberrated peak)
    pub exact: bool,
    /// Huygens PSF only: where to compute the Huygens sum: auto (default; a GPU if one is present, otherwise the CPU), cpu, or gpu (use the GPU path even on a software adapter; falls back to the CPU with a warning). The GPU sums in single precision (the intensity agrees with the exact sum to about 5e-7 of the unaberrated peak). Not used with exact
    pub device: HuygensDevice,
    /// Plane of the FFT PSF grid (image: the image surface; chief: perpendicular to the chief ray, as in OpticStudio's FFT PSF)
    pub plane: Plane,
    /// Include coating transmission and Fresnel losses in the pupil amplitude (unpolarized, with the system's coatings; like OpticStudio's Use Polarization). The system's pupil apodization is always included
    pub polarization: bool,
}

impl Default for PsfParams {
    fn default() -> Self {
        PsfParams {
            field: None,
            wave: None,
            huygens: false,
            pupil: None,
            image: 128,
            pixel: None,
            exact: false,
            device: HuygensDevice::Auto,
            plane: Plane::Image,
            polarization: false,
        }
    }
}

/// PSF の格子の 1 点
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PsfPoint {
    pub field: usize,
    #[serde(deserialize_with = "crate::nan::f")]
    pub x: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub y: f64,
    /// 強度（収差のない同じ瞳の中心の値を 1）
    #[serde(deserialize_with = "crate::nan::f")]
    pub i: f64,
}

/// PSF の格子の向き
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PsfAxes {
    pub field: usize,
    /// 主波長の主光線の像点（像面の局所座標）
    #[serde(deserialize_with = "crate::nan::a2")]
    pub center: [f64; 2],
    /// 格子の面（image か chief）
    pub plane: Plane,
    /// 格子の x の向き（像空間の単位ベクトル）
    #[serde(deserialize_with = "crate::nan::a3")]
    pub e1: [f64; 3],
    /// 格子の y の向き
    #[serde(deserialize_with = "crate::nan::a3")]
    pub e2: [f64; 3],
}

/// PSF の標本化
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PsfSampling {
    pub field: usize,
    /// 使った瞳の格子の一辺
    pub pupil: usize,
    /// 瞳の隣り合う標本の位相差の最大 [rad]
    #[serde(deserialize_with = "crate::nan::f")]
    pub phase_step: f64,
    /// 像の格子に入った光量の割合
    #[serde(deserialize_with = "crate::nan::f")]
    pub enclosed: f64,
    /// 格子の最大値（ストレール比の目安）
    #[serde(deserialize_with = "crate::nan::f")]
    pub peak: f64,
    /// 像の格子の間隔（x, y）
    #[serde(deserialize_with = "crate::nan::a2")]
    pub pixel: [f64; 2],
}

/// PSF の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PsfOut {
    pub unit: String,
    /// fft か huygens
    pub method: String,
    pub psf: Vec<PsfPoint>,
    pub axes: Vec<PsfAxes>,
    pub sampling: Vec<PsfSampling>,
    pub warnings: Vec<String>,
}

pub(super) struct Psf;

impl Analysis for Psf {
    type Params = PsfParams;
    type Output = PsfOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "psf",
            about: "Diffraction PSF (DFT of the pupil function). Intensity is 1 at the center of the same pupil without aberrations. Polychromatic if wave is omitted",
            help: "Diffraction PSF (DFT of the pupil function). Intensity is 1 at the center of the same pupil without aberrations. Polychromatic if wave is omitted. huygens gives the Huygens PSF (secondary waves from each point of the reference sphere summed on the image surface). x and y are local image surface coordinates relative to the chief ray; with plane chief, coordinates in the plane perpendicular to the chief ray (along e1 and e2, as in OpticStudio's FFT PSF)",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &PsfParams) -> Result<PsfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let (mut v, mut axes, mut warn, mut samp) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if p.exact && !p.huygens {
            lensbench_core::invalid!("setting exact is for the Huygens PSF (give huygens too)");
        }
        if p.device != HuygensDevice::Auto && !p.huygens {
            lensbench_core::invalid!("setting device is for the Huygens PSF (give huygens too)");
        }
        for i in fields(s, p.field)? {
            let pa = pol_amp(s, i, w, p.polarization)?;
            let pa = pa.as_ref().map(|a| a as &dyn PupilAmp);
            let r = if p.huygens {
                let o = HuygensOpts {
                    pupil: p.pupil,
                    image: p.image,
                    pixel: p.pixel,
                    exact: p.exact,
                    device: p.device,
                };
                huygens::huygens_psf_with(s, FieldSel::Index(i), Waves::all_or(w), o, pa)
            } else {
                let o = PsfOpts {
                    pupil: p.pupil,
                    image: p.image,
                    pixel: p.pixel,
                    plane: match p.plane {
                        Plane::Image => PsfPlane::Image,
                        Plane::Chief => PsfPlane::ChiefPerp,
                    },
                };
                psf::psf_with(s, FieldSel::Index(i), Waves::all_or(w), o, pa)
            }?;
            axes.push(PsfAxes {
                field: i + 1,
                center: [r.center.0, r.center.1],
                plane: match r.plane {
                    PsfPlane::Image => Plane::Image,
                    PsfPlane::ChiefPerp => Plane::Chief,
                },
                e1: r.axes[0],
                e2: r.axes[1],
            });
            for (iy, row) in r.i.rows().enumerate() {
                for (ix, val) in row.iter().enumerate() {
                    v.push(PsfPoint {
                        field: i + 1,
                        x: r.i.x()[ix],
                        y: r.i.y()[iy],
                        i: *val,
                    });
                }
            }
            warn.extend(
                r.warnings
                    .iter()
                    .map(|m| format!("field {}: {e}", i + 1, e = m)),
            );
            samp.push(PsfSampling {
                field: i + 1,
                pupil: r.pupil,
                phase_step: r.phase_step,
                enclosed: r.enclosed,
                peak: r.peak,
                pixel: [r.pixel.0, r.pixel.1],
            });
        }
        Ok(PsfOut {
            unit: spot_unit(s),
            method: if p.huygens { "huygens" } else { "fft" }.into(),
            psf: v,
            axes,
            sampling: samp,
            warnings: warn,
        })
    }
}
