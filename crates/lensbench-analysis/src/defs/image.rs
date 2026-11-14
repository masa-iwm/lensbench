//! RMS の掃引・PSF

use lensbench_core::huygens::{self, HuygensOpts};
use lensbench_core::psf::{self, PsfOpts};
use lensbench_core::sweep::{self, RmsOpts, RmsRef};
use lensbench_core::{Error, FieldSel, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, spot_unit, wave, Axis};
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
pub struct RmsParams {
    /// 視野の番号（1 始まり、省略で全視野。vs field では使わない）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色。vs wavelength では使わない）
    pub wave: Option<usize>,
    /// 横軸: field（正規化視野 0〜1）、focus（像面のずらし ±range）、wavelength（系の波長の最小〜最大、単色）
    pub vs: Vs,
    /// 横軸の点数
    #[schemars(extend("x-min" = 1))]
    pub points: usize,
    /// focus のずらしの範囲 ±range [レンズ単位]
    pub range: f64,
    /// field の視野を振る向き（y か x）
    pub axis: Axis,
    /// 基準: centroid（重心）か chief（主光線）
    #[serde(rename = "ref")]
    pub reference: Reference,
    /// 瞳のガウス求積のリング数
    #[schemars(extend("x-min" = 1))]
    pub rings: usize,
    /// 単色の field・wavelength の掃引で、実像高（と条件で定める視野）の視野を解析の波長の主光線で解く
    /// （OpticStudio の単色の RMS vs Field・vs Wavelength と同じ。省くと主波長で解く）
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct RmsRow {
    /// スポット半径の RMS
    pub spot: f64,
    pub spot_x: f64,
    pub spot_y: f64,
    /// 波面収差の RMS [波]
    pub wfe: f64,
    /// ストレール比の近似
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct RmsOut {
    pub unit: String,
    pub wfe_unit: String,
    pub vs: Vs,
    pub reference: Reference,
    pub rms: Vec<RmsRow>,
    pub warnings: Vec<String>,
}

pub struct Rms;

impl Analysis for Rms {
    type Params = RmsParams;
    type Output = RmsOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "rms",
            about: "RMS（スポット半径・x・y、波面、ストレール比の近似）を視野・フォーカス・波長に対して",
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
                    return Err(Error::Invalid("vs field では field は使わない".into()));
                }
                let ax = p.axis.core();
                let r = sweep::rms_vs_field(s, ax, &sweep::steps01(p.points), w, o);
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
                        w,
                        o,
                    );
                    for (x, q) in r.x.iter().zip(&r.rms) {
                        let mut e = RmsRow::of(q);
                        e.dz = Some(*x);
                        e.field = Some(i + 1);
                        v.push(e);
                    }
                    warn.extend(r.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
                }
            }
            Vs::Wavelength => {
                if p.wave.is_some() {
                    return Err(Error::Invalid("vs wavelength では wave は使わない".into()));
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
                    warn.extend(r.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
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

/// PSF の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PsfParams {
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// Huygens PSF（像面の上。傾いた像面・収差の大きい系向け、FFT より遅い）
    pub huygens: bool,
    /// 瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下になる大きさ）
    pub pupil: Option<usize>,
    /// 像の格子の一辺（間隔が既定なら瞳の格子の 2 倍まで）
    #[schemars(extend("x-min" = 1))]
    pub image: usize,
    /// 像の格子の間隔 [レンズ単位、アフォーカル像空間では角度の単位]（省くと 1 / (2 × 遮断周波数)）
    pub delta: Option<f64>,
}

impl Default for PsfParams {
    fn default() -> Self {
        PsfParams {
            field: None,
            wave: None,
            huygens: false,
            pupil: None,
            image: 128,
            delta: None,
        }
    }
}

/// PSF の格子の 1 点
#[derive(Debug, Serialize, JsonSchema)]
pub struct PsfPoint {
    pub field: usize,
    pub x: f64,
    pub y: f64,
    /// 強度（収差のない同じ瞳の中心の値を 1）
    pub i: f64,
}

/// PSF の格子の向き
#[derive(Debug, Serialize, JsonSchema)]
pub struct PsfAxes {
    pub field: usize,
    /// 主波長の主光線の像点（像面の局所座標）
    pub center: [f64; 2],
    /// 格子の x の向き（像空間の単位ベクトル）
    pub e1: [f64; 3],
    /// 格子の y の向き
    pub e2: [f64; 3],
}

/// PSF の標本化
#[derive(Debug, Serialize, JsonSchema)]
pub struct PsfSampling {
    pub field: usize,
    /// 使った瞳の格子の一辺
    pub pupil: usize,
    /// 瞳の隣り合う標本の位相差の最大 [rad]
    pub phase_step: f64,
    /// 像の格子に入った光量の割合
    pub enclosed: f64,
    /// 格子の最大値（ストレール比の目安）
    pub peak: f64,
    /// 像の格子の間隔（x, y）
    pub delta: [f64; 2],
}

/// PSF の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct PsfOut {
    pub unit: String,
    /// fft か huygens
    pub method: String,
    pub psf: Vec<PsfPoint>,
    pub axes: Vec<PsfAxes>,
    pub sampling: Vec<PsfSampling>,
    pub warnings: Vec<String>,
}

pub struct Psf;

impl Analysis for Psf {
    type Params = PsfParams;
    type Output = PsfOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "psf",
            about: "回折 PSF（瞳関数の DFT）。強度は収差のない同じ瞳の中心の値を 1 とする。wave を省くと多色",
            help: "回折 PSF（瞳関数の DFT）。強度は収差のない同じ瞳の中心の値を 1 とする。wave を省くと多色。\
                   huygens で Huygens PSF（参照球の各点からの 2 次波を像面の上で足す。座標は像面の局所座標）。\
                   x・y は主光線に垂直な面の座標（e1・e2 の向き、OpticStudio の FFT PSF と同じ）",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &PsfParams) -> Result<PsfOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let (mut v, mut axes, mut warn, mut samp) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for i in fields(s, p.field)? {
            let r = if p.huygens {
                let o = HuygensOpts {
                    pupil: p.pupil,
                    image: p.image,
                    delta: p.delta,
                };
                huygens::huygens_psf(s, FieldSel::Index(i), w, o)
            } else {
                let o = PsfOpts {
                    pupil: p.pupil,
                    image: p.image,
                    delta: p.delta,
                };
                psf::psf(s, FieldSel::Index(i), w, o)
            }?;
            axes.push(PsfAxes {
                field: i + 1,
                center: [r.center.0, r.center.1],
                e1: r.axes[0],
                e2: r.axes[1],
            });
            for (iy, row) in r.i.iter().enumerate() {
                for (ix, val) in row.iter().enumerate() {
                    v.push(PsfPoint {
                        field: i + 1,
                        x: r.x[ix],
                        y: r.y[iy],
                        i: *val,
                    });
                }
            }
            warn.extend(r.warnings.iter().map(|m| format!("視野 {}: {m}", i + 1)));
            samp.push(PsfSampling {
                field: i + 1,
                pupil: r.pupil,
                phase_step: r.phase_step,
                enclosed: r.enclosed,
                peak: r.peak,
                delta: [r.delta.0, r.delta.1],
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
