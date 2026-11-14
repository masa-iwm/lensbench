//! 同梱の解析の定義（設定・結果の型と計算）。CLI・serve・GUI はここから作る（[`crate::registry`]）。
//!
//! 結果の JSON の形は以前の CLI の出力と同じ（GUI・Python のクライアントが鍵で読んでいる）。

mod basic;
mod image;
mod mtfs;

use lensbench_core::{Error, Result, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::registry::Analyses;

/// 同梱の解析を登録する
pub fn register(a: &mut Analyses) {
    a.add(basic::Spot);
    a.add(basic::Wavefront);
    a.add(basic::Zernike);
    a.add(basic::Ghost);
    a.add(mtfs::Mtf);
    a.add(mtfs::MtfFocus);
    a.add(mtfs::MtfField);
    a.add(mtfs::GeoMtf);
    a.add(mtfs::HuygensMtf);
    a.add(image::Rms);
    a.add(image::Psf);
}

/// 視野の番号（1 始まり、None で全視野）を 0 始まりの並びにする
pub fn fields(s: &System, field: Option<usize>) -> Result<Vec<usize>> {
    match field {
        None => Ok((0..s.fields.len()).collect()),
        Some(i) if i >= 1 && i <= s.fields.len() => Ok(vec![i - 1]),
        Some(i) => Err(Error::Invalid(format!(
            "視野 {i} はありません（1〜{}）",
            s.fields.len()
        ))),
    }
}

/// 波長の番号（1 始まり、None で多色）を 0 始まりにする
pub fn wave(s: &System, w: Option<usize>) -> Result<Option<usize>> {
    match w {
        None => Ok(None),
        Some(i) if i >= 1 && i <= s.wavelengths.len() => Ok(Some(i - 1)),
        Some(i) => Err(Error::Invalid(format!(
            "波長 {i} はありません（1〜{}）",
            s.wavelengths.len()
        ))),
    }
}

/// スポット・PSF の位置の単位: mm、アフォーカル像空間では角度の単位（前は mm を "lens" と書いた、XCT-18）
fn spot_unit(s: &System) -> String {
    s.image_unit().to_string()
}

/// 周波数の単位（cycles/mm、アフォーカル像空間では cycles/角度の単位）
fn freq_unit(s: &System) -> String {
    format!("cycles/{}", s.image_unit())
}

/// 0 から top までを steps 等分した周波数
fn auto_freqs(top: f64, steps: usize) -> Result<Vec<f64>> {
    if !(top.is_finite() && top > 0.0) {
        return Err(Error::Invalid(format!("max_freq は正の数（{top}）")));
    }
    if steps == 0 {
        return Err(Error::Invalid("freq_steps は 1 以上".into()));
    }
    Ok((0..=steps).map(|k| top * k as f64 / steps as f64).collect())
}

/// 周波数の選び方（与えた並び、または 0 から上限までの等分。上限の既定は遮断周波数の 0.95 倍）
fn freqs_for(
    s: &System,
    field: usize,
    w: Option<usize>,
    freqs: &Option<Vec<f64>>,
    max_freq: Option<f64>,
    freq_steps: usize,
) -> Result<Vec<f64>> {
    if let Some(f) = freqs {
        if max_freq.is_some() {
            return Err(Error::Invalid("freqs と max_freq は一緒に使えない".into()));
        }
        return Ok(f.clone());
    }
    let top = match max_freq {
        Some(x) => x,
        None => {
            let f = lensbench_core::FieldSel::Index(field);
            0.95 * lensbench_core::analysis::mtf_cutoff(s, f, w.unwrap_or(s.primary))?.min()
        }
    };
    auto_freqs(top, freq_steps)
}

/// 視野を振る向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// y の向き
    #[default]
    Y,
    /// x の向き
    X,
}

impl Axis {
    fn core(self) -> lensbench_core::sweep::FieldAxis {
        match self {
            Axis::X => lensbench_core::sweep::FieldAxis::X,
            Axis::Y => lensbench_core::sweep::FieldAxis::Y,
        }
    }
}

/// 回折 MTF の 1 視野（mtf・geo-mtf・huygens-mtf の共通の形）
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MtfRow {
    /// 視野の番号（1 始まり）
    pub field: usize,
    pub freqs: Vec<f64>,
    /// 子午
    pub tangential: Vec<f64>,
    /// 球欠
    pub sagittal: Vec<f64>,
    /// 回折限界の遮断周波数 [cycles/mm]（計算した波長。多色は主波長）
    pub cutoff: Cutoff,
    /// 波長ごと（mtf の each_wave）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waves: Option<Vec<WaveMtf>>,
}

/// 子午・球欠の遮断周波数
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct Cutoff {
    pub tangential: f64,
    pub sagittal: f64,
}

/// 1 波長の MTF
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WaveMtf {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    pub tangential: Vec<f64>,
    pub sagittal: Vec<f64>,
}

impl MtfRow {
    fn of(field: usize, m: lensbench_core::analysis::Mtf) -> MtfRow {
        MtfRow {
            field: field + 1,
            freqs: m.freqs,
            tangential: m.tan,
            sagittal: m.sag,
            cutoff: Cutoff {
                tangential: m.cutoff.tan,
                sagittal: m.cutoff.sag,
            },
            waves: None,
        }
    }
}

#[cfg(test)]
mod tests;
