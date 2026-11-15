//! 回折 MTF・スルーフォーカス・フィールド MTF・幾何 MTF・Huygens MTF

use crate::huygens::{self, HuygensOpts};
use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::Waves;
use lensbench_core::{psf, sweep, Error, FieldSel, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, freq_unit, freqs_for, wave, Axis, MtfRow, WaveMtf};
use crate::registry::{Analysis, Cx, Meta};

// ----------------------------------------------------------------------------------------- mtf

/// 回折 MTF の求め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MtfHow {
    /// 瞳の格子を 1 度追跡して補間の求積。求積点が 1024 を超え周波数が 8 個以上なら FFT（n を省いたとき）
    #[default]
    Auto,
    /// 求積点の数が多い系でも FFT に切り替えず、補間の求積で求める
    Interp,
    /// 求積点の OPD をすべて直接追跡する（遅い、確かめ用）
    Direct,
    /// 瞳関数の自己相関を FFT で全周波数まとめて求める（既定との差は 1e-4 程度）
    Fft,
}

/// 回折 MTF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MtfParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Frequencies [cycles/mm; cycles per angle unit in afocal image space]. If omitted, freq_steps equal steps from 0 to the limit (max_freq, 0.95 times the cutoff by default)
    pub freqs: Option<Vec<f64>>,
    /// Upper limit when frequencies are omitted [cycles/mm; cycles per angle unit in afocal image space]. 0.95 times the cutoff if omitted
    pub max_freq: Option<f64>,
    /// Number of steps when frequencies are omitted (equal steps from 0 to the limit; steps + 1 points)
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// Approximate number of quadrature points for the pupil overlap (about n² points). Chosen automatically from the OPD slope if omitted
    pub n: Option<usize>,
    /// Method: auto, interp, direct or fft
    pub method: MtfHow,
    /// Also output the MTF of each wavelength besides the polychromatic one ("waves"; faster than separate runs because the per-wavelength computation is reused). Can't be used with wave
    pub each_wave: bool,
}

impl Default for MtfParams {
    fn default() -> Self {
        MtfParams {
            field: None,
            wave: None,
            freqs: None,
            max_freq: None,
            freq_steps: 20,
            n: None,
            method: MtfHow::Auto,
            each_wave: false,
        }
    }
}

/// MTF の結果（mtf・huygens-mtf）
#[derive(Debug, Serialize, JsonSchema)]
pub struct MtfOut {
    pub unit: String,
    /// 求め方（huygens-mtf は huygens）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub mtf: Vec<MtfRow>,
}

pub struct Mtf;

impl Analysis for Mtf {
    type Params = MtfParams;
    type Output = MtfOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "mtf",
            about: "Diffraction MTF (modulation, tangential and sagittal). Polychromatic if wave is omitted",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfParams) -> Result<MtfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        if p.each_wave && w.is_some() {
            return Err(Error::Invalid("each_wave can't be used with wave".into()));
        }
        let method = match p.method {
            MtfHow::Auto => MtfMethod::Auto,
            MtfHow::Interp => MtfMethod::Interp,
            MtfHow::Direct => MtfMethod::Direct,
            MtfHow::Fft => MtfMethod::Fft,
        };
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let fs = freqs_for(s, i, w, &p.freqs, p.max_freq, p.freq_steps)?;
            let f = FieldSel::Index(i);
            let row = if p.each_wave {
                let mw = analysis::mtf_waves(s, f, p.n, &fs, method)?;
                let mut r = MtfRow::of(i, mw.poly);
                r.waves = Some(
                    mw.waves
                        .into_iter()
                        .enumerate()
                        .map(|(k, m)| WaveMtf {
                            wave: k + 1,
                            tangential: m.tan,
                            sagittal: m.sag,
                        })
                        .collect(),
                );
                r
            } else {
                MtfRow::of(
                    i,
                    analysis::mtf_with(s, f, Waves::all_or(w), p.n, &fs, method)?,
                )
            };
            v.push(row);
        }
        Ok(MtfOut {
            unit: freq_unit(s),
            method: None,
            mtf: v,
        })
    }
}

// ----------------------------------------------------------------------------------- mtf-focus

/// スルーフォーカス MTF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MtfFocusParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Frequency [cycles/mm; cycles per angle unit in afocal image space] (required)
    pub freqs: Vec<f64>,
    /// Image shift range ±range [lens units]
    pub range: f64,
    /// Number of shift points (from −range to +range)
    #[schemars(extend("x-min" = 1))]
    pub points: usize,
    /// Approximate number of quadrature points for the pupil overlap (as in mtf; automatic if omitted)
    pub n: Option<usize>,
}

impl Default for MtfFocusParams {
    fn default() -> Self {
        MtfFocusParams {
            field: None,
            wave: None,
            freqs: Vec::new(),
            range: 0.1,
            points: 11,
            n: None,
        }
    }
}

/// スルーフォーカス MTF の 1 点
#[derive(Debug, Serialize, JsonSchema)]
pub struct FocusRow {
    pub field: usize,
    pub freq: f64,
    pub dz: f64,
    pub tangential: f64,
    pub sagittal: f64,
}

/// スルーフォーカス MTF の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct MtfFocusOut {
    pub unit: String,
    pub through_focus: Vec<FocusRow>,
    pub warnings: Vec<String>,
}

pub struct MtfFocus;

impl Analysis for MtfFocus {
    type Params = MtfFocusParams;
    type Output = MtfFocusOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "mtf-focus",
            about: "Through-focus MTF (diffraction MTF with the image shifted over ±range in points steps). Polychromatic if wave is omitted",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfFocusParams) -> Result<MtfFocusOut> {
        if p.freqs.is_empty() {
            return Err(Error::Invalid("freqs (frequencies) are needed".into()));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        Waves::all_or(w).weighted(s)?;
        let dz = sweep::steps_sym(p.range, p.points);
        let (mut v, mut warn) = (Vec::new(), Vec::new());
        for i in fields(s, p.field)? {
            let m = sweep::through_focus_mtf(
                s,
                FieldSel::Index(i),
                Waves::all_or(w),
                p.n,
                &p.freqs,
                &dz,
            );
            for (k, &f) in p.freqs.iter().enumerate() {
                for (j, &z) in dz.iter().enumerate() {
                    v.push(FocusRow {
                        field: i + 1,
                        freq: f,
                        dz: z,
                        tangential: m.tan[k][j],
                        sagittal: m.sag[k][j],
                    });
                }
            }
            warn.extend(
                m.warnings
                    .iter()
                    .map(|x| format!("field {}: {e}", i + 1, e = x)),
            );
        }
        Ok(MtfFocusOut {
            unit: freq_unit(s),
            through_focus: v,
            warnings: warn,
        })
    }
}

// ----------------------------------------------------------------------------------- mtf-field

/// フィールド MTF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MtfFieldParams {
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Frequency [cycles/mm; cycles per angle unit in afocal image space] (required)
    pub freqs: Vec<f64>,
    /// Number of field points (normalized field 0 to 1)
    #[schemars(extend("x-min" = 1))]
    pub points: usize,
    /// Field direction (y or x)
    pub axis: Axis,
    /// Approximate number of quadrature points for the pupil overlap (as in mtf; automatic if omitted)
    pub n: Option<usize>,
}

impl Default for MtfFieldParams {
    fn default() -> Self {
        MtfFieldParams {
            wave: None,
            freqs: Vec::new(),
            points: 11,
            axis: Axis::Y,
            n: None,
        }
    }
}

/// フィールド MTF の 1 点
#[derive(Debug, Serialize, JsonSchema)]
pub struct FieldRow {
    pub freq: f64,
    /// 正規化視野
    pub h: f64,
    /// 視野の値（系の視野の単位）
    pub field_value: f64,
    pub tangential: f64,
    pub sagittal: f64,
}

/// フィールド MTF の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct MtfFieldOut {
    pub unit: String,
    pub field_mtf: Vec<FieldRow>,
    pub warnings: Vec<String>,
}

pub struct MtfField;

impl Analysis for MtfField {
    type Params = MtfFieldParams;
    type Output = MtfFieldOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "mtf-field",
            about:
                "Field MTF (diffraction MTF at points steps of the normalized field 0 to 1). Polychromatic if wave is omitted",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfFieldParams) -> Result<MtfFieldOut> {
        if p.freqs.is_empty() {
            return Err(Error::Invalid("freqs (frequencies) are needed".into()));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let ax = p.axis.core();
        let h = sweep::steps01(p.points);
        let m = sweep::field_mtf(s, ax, &h, Waves::all_or(w).weighted(s)?, p.n, &p.freqs);
        let mut v = Vec::new();
        for (k, &f) in p.freqs.iter().enumerate() {
            for (j, &hh) in h.iter().enumerate() {
                v.push(FieldRow {
                    freq: f,
                    h: hh,
                    field_value: ax.value(s, hh),
                    tangential: m.tan[k][j],
                    sagittal: m.sag[k][j],
                });
            }
        }
        Ok(MtfFieldOut {
            unit: freq_unit(s),
            field_mtf: v,
            warnings: m.warnings,
        })
    }
}

// ------------------------------------------------------------------------------------- geo-mtf

/// 幾何 MTF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GeoMtfParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Frequencies [cycles/mm; cycles per angle unit in afocal image space]. If omitted, freq_steps equal steps from 0 to the limit (max_freq, 0.95 times the cutoff by default)
    pub freqs: Option<Vec<f64>>,
    /// Upper limit when frequencies are omitted. 0.95 times the cutoff if omitted
    pub max_freq: Option<f64>,
    /// Number of steps when frequencies are omitted (equal steps from 0 to the limit; steps + 1 points)
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// Approximate number of pupil quadrature points (about n² points)
    #[schemars(extend("x-min" = 4))]
    pub n: usize,
    /// Multiply by the diffraction-limited MTF
    pub diff_limit: bool,
}

impl Default for GeoMtfParams {
    fn default() -> Self {
        GeoMtfParams {
            field: None,
            wave: None,
            freqs: None,
            max_freq: None,
            freq_steps: 20,
            n: 128,
            diff_limit: false,
        }
    }
}

/// 幾何 MTF の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct GeoMtfOut {
    pub unit: String,
    pub geometric_mtf: Vec<MtfRow>,
}

pub struct GeoMtf;

impl Analysis for GeoMtf {
    type Params = GeoMtfParams;
    type Output = GeoMtfOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "geo-mtf",
            about: "Geometric MTF (from the distribution of ray positions). Polychromatic if wave is omitted",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GeoMtfParams) -> Result<GeoMtfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let fs = freqs_for(s, i, w, &p.freqs, p.max_freq, p.freq_steps)?;
            let m = psf::geometric_mtf(
                s,
                FieldSel::Index(i),
                Waves::all_or(w),
                p.n,
                &fs,
                p.diff_limit,
            )?;
            v.push(MtfRow::of(i, m));
        }
        Ok(GeoMtfOut {
            unit: freq_unit(s),
            geometric_mtf: v,
        })
    }
}

// --------------------------------------------------------------------------------- huygens-mtf

/// Huygens MTF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HuygensMtfParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Frequencies [cycles/mm]. If omitted, freq_steps equal steps from 0 to the limit (max_freq, 0.95 times the cutoff by default)
    pub freqs: Option<Vec<f64>>,
    /// Upper limit when frequencies are omitted [cycles/mm]. 0.95 times the cutoff if omitted
    pub max_freq: Option<f64>,
    /// Number of steps when frequencies are omitted (equal steps from 0 to the limit; steps + 1 points)
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// Pupil grid side (automatic if omitted: large enough that neighboring samples differ by π/2 or less in phase)
    pub pupil: Option<usize>,
    /// Image grid side (up to twice the pupil grid at the default spacing). If it doesn't cover the PSF, the low-frequency MTF drops
    #[schemars(extend("x-min" = 1))]
    pub image: usize,
    /// Image grid spacing [mm] (1 / (2 × cutoff) if omitted)
    pub delta: Option<f64>,
}

impl Default for HuygensMtfParams {
    fn default() -> Self {
        HuygensMtfParams {
            field: None,
            wave: None,
            freqs: None,
            max_freq: None,
            freq_steps: 20,
            pupil: None,
            image: 128,
            delta: None,
        }
    }
}

pub struct HuygensMtf;

impl Analysis for HuygensMtf {
    type Params = HuygensMtfParams;
    type Output = MtfOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "huygens-mtf",
            about: "Huygens MTF (Fourier transform of the Huygens PSF, frequencies on the image surface). Polychromatic if wave is omitted. The output has the same form as mtf",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &HuygensMtfParams) -> Result<MtfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let o = HuygensOpts {
            pupil: p.pupil,
            image: p.image,
            delta: p.delta,
        };
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let fs = freqs_for(s, i, w, &p.freqs, p.max_freq, p.freq_steps)?;
            v.push(MtfRow::of(
                i,
                huygens::huygens_mtf(s, FieldSel::Index(i), Waves::all_or(w), o, &fs)?,
            ));
        }
        Ok(MtfOut {
            unit: freq_unit(s),
            method: Some("huygens".into()),
            mtf: v,
        })
    }
}
