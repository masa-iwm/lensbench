//! 回折 MTF・スルーフォーカス・フィールド MTF・幾何 MTF・Huygens MTF

use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::huygens::{self, HuygensOpts};
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
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]。省くと 0 から上限（max_freq、既定は遮断周波数の 0.95 倍）までの freq_steps 等分
    pub freqs: Option<Vec<f64>>,
    /// 周波数を省いたときの上限 [cycles/mm、アフォーカル像空間では cycles/角度の単位]。省くと遮断周波数の 0.95 倍
    pub max_freq: Option<f64>,
    /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// 瞳の重なりの求積の点の数の目安（点は約 n² 個）。省くと OPD の傾きから自動で決める
    pub n: Option<usize>,
    /// 求め方: auto・interp・direct・fft
    pub method: MtfHow,
    /// 多色に加えて波長ごとの MTF も出す（"waves"。波長ごとの計算を使い回すので別々に求めるより速い）。wave とは一緒に使えない
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
            about: "回折 MTF（変調度、子午・球欠）。wave を省くと多色",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfParams) -> Result<MtfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        if p.each_wave && w.is_some() {
            return Err(Error::Invalid("each_wave は wave と一緒に使えない".into()));
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
                MtfRow::of(i, analysis::mtf_with(s, f, w, p.n, &fs, method)?)
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
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（必須）
    pub freqs: Vec<f64>,
    /// 像面のずらしの範囲 ±range [レンズ単位]
    pub range: f64,
    /// ずらしの点数（−range から +range まで）
    #[schemars(extend("x-min" = 1))]
    pub steps: usize,
    /// 瞳の重なりの求積の点の数の目安（mtf と同じ。省くと自動）
    pub n: Option<usize>,
}

impl Default for MtfFocusParams {
    fn default() -> Self {
        MtfFocusParams {
            field: None,
            wave: None,
            freqs: Vec::new(),
            range: 0.1,
            steps: 11,
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
            about: "スルーフォーカス MTF（像面を ±range で steps 点ずらした回折 MTF）。wave を省くと多色",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfFocusParams) -> Result<MtfFocusOut> {
        if p.freqs.is_empty() {
            return Err(Error::Invalid("freqs（周波数）が要る".into()));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let dz = sweep::steps_sym(p.range, p.steps);
        let (mut v, mut warn) = (Vec::new(), Vec::new());
        for i in fields(s, p.field)? {
            let m = sweep::through_focus_mtf(s, FieldSel::Index(i), w, p.n, &p.freqs, &dz);
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
            warn.extend(m.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
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
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（必須）
    pub freqs: Vec<f64>,
    /// 視野の点数（正規化視野 0 から 1 まで）
    #[schemars(extend("x-min" = 1))]
    pub points: usize,
    /// 視野を振る向き（y か x）
    pub axis: Axis,
    /// 瞳の重なりの求積の点の数の目安（mtf と同じ。省くと自動）
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
                "フィールド MTF（正規化視野 0〜1 を points 点で刻んだ回折 MTF）。wave を省くと多色",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MtfFieldParams) -> Result<MtfFieldOut> {
        if p.freqs.is_empty() {
            return Err(Error::Invalid("freqs（周波数）が要る".into()));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let ax = p.axis.core();
        let h = sweep::steps01(p.points);
        let m = sweep::field_mtf(s, ax, &h, w, p.n, &p.freqs);
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
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]。省くと 0 から上限（max_freq、既定は遮断周波数の 0.95 倍）までの freq_steps 等分
    pub freqs: Option<Vec<f64>>,
    /// 周波数を省いたときの上限。省くと遮断周波数の 0.95 倍
    pub max_freq: Option<f64>,
    /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// 瞳の求積の点の数の目安（点は約 n² 個）
    #[schemars(extend("x-min" = 4))]
    pub n: usize,
    /// 回折限界の MTF をかける
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
            about: "幾何 MTF（光線の位置の分布から）。wave を省くと多色",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GeoMtfParams) -> Result<GeoMtfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let fs = freqs_for(s, i, w, &p.freqs, p.max_freq, p.freq_steps)?;
            let m = psf::geometric_mtf(s, FieldSel::Index(i), w, p.n, &fs, p.diff_limit)?;
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
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 周波数 [cycles/mm]。省くと 0 から上限（max_freq、既定は遮断周波数の 0.95 倍）までの freq_steps 等分
    pub freqs: Option<Vec<f64>>,
    /// 周波数を省いたときの上限 [cycles/mm]。省くと遮断周波数の 0.95 倍
    pub max_freq: Option<f64>,
    /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
    #[schemars(extend("x-min" = 1))]
    pub freq_steps: usize,
    /// 瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下になる大きさ）
    pub pupil: Option<usize>,
    /// 像の格子の一辺（間隔が既定なら瞳の格子の 2 倍まで）。PSF を覆いきれないと低周波の MTF が下がる
    #[schemars(extend("x-min" = 1))]
    pub image: usize,
    /// 像の格子の間隔 [mm]（省くと 1 / (2 × 遮断周波数)）
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
            about: "Huygens MTF（Huygens PSF のフーリエ変換、像面の上の周波数）。wave を省くと多色。出力は mtf と同じ形",
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
                huygens::huygens_mtf(s, FieldSel::Index(i), w, o, &fs)?,
            ));
        }
        Ok(MtfOut {
            unit: freq_unit(s),
            method: Some("huygens".into()),
            mtf: v,
        })
    }
}
