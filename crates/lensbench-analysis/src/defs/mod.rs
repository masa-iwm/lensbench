//! 同梱の解析の定義（設定・結果の型と計算）。CLI・serve・GUI はここから作る（[`crate::registry`]）。
//!
//! 結果の JSON の形は GUI・Python のクライアントが鍵で読む約束（変えるときは壊れる変更として docs/changes.md に書く）。

mod basic;
mod image;
mod mtfs;

/// 同梱の解析の結果の型（同じプロセスの呼び手は [`crate::Typed::get`] で引く。JSON は同じ形）
pub use basic::{
    GhostOut, GhostRow, RayCountsOut, SpotOut, SpotRow, WavefrontOut, WavefrontRow, ZKind,
    ZernikeOut, ZernikeRow,
};
pub use image::{
    Plane as PsfPlane, PsfAxes, PsfOut, PsfPoint, PsfSampling, Reference as RmsReference, RmsOut,
    RmsRow, Vs as RmsVs,
};
pub use mtfs::{FieldRow, FocusRow, GeoMtfOut, MtfFieldOut, MtfFocusOut, MtfOut};

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
            "no field {i} (1 to {})",
            s.fields.len()
        ))),
    }
}

/// 波長の番号（1 始まり、None で多色）を 0 始まりにする
/// 設定 polarization の膜・偏光の振幅（偏光なし、系の膜。[`crate::pupil_amp::PolAmp`]）。off なら None
fn pol_amp(
    s: &System,
    i: usize,
    w: Option<usize>,
    on: bool,
) -> Result<Option<crate::pupil_amp::PolAmp>> {
    if !on {
        return Ok(None);
    }
    let co = lensbench_core::coating::Coatings::for_system(s)?;
    let ws: Vec<usize> = w.map_or_else(|| (0..s.wavelengths.len()).collect(), |w| vec![w]);
    let f = lensbench_core::FieldSel::Index(i);
    crate::pupil_amp::PolAmp::new(s, f, &ws, &co, crate::pupil_amp::POL_GRID, None).map(Some)
}

pub fn wave(s: &System, w: Option<usize>) -> Result<Option<usize>> {
    lensbench_core::waves::index_opt(w, s.wavelengths.len())
}

/// スポット・PSF の位置の単位: mm、アフォーカル像空間では角度の単位（前は mm を "lens" と書いた）
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
        return Err(Error::Invalid(format!("max_freq must be positive ({top})")));
    }
    if steps == 0 {
        return Err(Error::Invalid("freq_steps must be 1 or more".into()));
    }
    Ok((0..=steps).map(|k| top * k as f64 / steps as f64).collect())
}

/// 周波数の選び方（与えた並び、または 0 から上限までの等分。上限の既定は子午・球欠の向き axes の遮断周波数の
/// 小さい方の 0.95 倍）
fn freqs_for(
    s: &System,
    field: usize,
    w: Option<usize>,
    freqs: &Option<Vec<f64>>,
    max_freq: Option<f64>,
    freq_steps: usize,
    axes: MtfDirs,
) -> Result<Vec<f64>> {
    if let Some(f) = freqs {
        if max_freq.is_some() {
            return Err(Error::Invalid(
                "freqs and max_freq can't be used together".into(),
            ));
        }
        return Ok(f.clone());
    }
    let top = match max_freq {
        Some(x) => x,
        None => {
            let f = lensbench_core::FieldSel::Index(field);
            let w = w.unwrap_or(s.primary);
            0.95 * crate::analysis::mtf_cutoff_axes(s, f, w, axes.core())?.min()
        }
    };
    auto_freqs(top, freq_steps)
}

/// MTF の子午・球欠の向き（core の [`crate::analysis::MtfAxes`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MtfDirs {
    /// 子午は視野の向き（物理的な子午面）、球欠はそれに直交
    #[default]
    Field,
    /// 子午は瞳の y、球欠は x の向き（OpticStudio の FFT MTF と同じ）
    Pupil,
}

impl MtfDirs {
    pub(super) fn core(self) -> crate::analysis::MtfAxes {
        match self {
            MtfDirs::Field => crate::analysis::MtfAxes::Field,
            MtfDirs::Pupil => crate::analysis::MtfAxes::Pupil,
        }
    }
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
    fn core(self) -> crate::sweep::FieldAxis {
        match self {
            Axis::X => crate::sweep::FieldAxis::X,
            Axis::Y => crate::sweep::FieldAxis::Y,
        }
    }
}

/// 回折 MTF の 1 視野（mtf・geo-mtf・huygens-mtf の共通の形）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MtfRow {
    /// Field number (from 1)
    pub field: usize,
    #[serde(deserialize_with = "crate::nan::v")]
    pub freqs: Vec<f64>,
    /// 子午
    #[serde(deserialize_with = "crate::nan::v")]
    pub tangential: Vec<f64>,
    /// 球欠
    #[serde(deserialize_with = "crate::nan::v")]
    pub sagittal: Vec<f64>,
    /// 回折限界の遮断周波数 [cycles/mm]（計算した波長。多色は主波長）
    pub cutoff: Cutoff,
    /// 波長ごと（mtf の each_wave）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waves: Option<Vec<WaveMtf>>,
}

/// 子午・球欠の遮断周波数
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct Cutoff {
    #[serde(deserialize_with = "crate::nan::f")]
    pub tangential: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub sagittal: f64,
}

/// 1 波長の MTF
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaveMtf {
    /// 波長の番号（1 始まり）
    pub wave: usize,
    #[serde(deserialize_with = "crate::nan::v")]
    pub tangential: Vec<f64>,
    #[serde(deserialize_with = "crate::nan::v")]
    pub sagittal: Vec<f64>,
}

impl MtfRow {
    fn of(field: usize, m: crate::analysis::Mtf) -> MtfRow {
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
