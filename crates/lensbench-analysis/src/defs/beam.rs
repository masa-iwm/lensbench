//! 近軸ガウスビーム（Paraxial Gaussian Beam）

use lensbench_core::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::wave;
use crate::gauss;
use crate::registry::{Analysis, Cx, Meta};

/// ビームを追う断面
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// YZ 面（トロイダル面などは YZ 面の近軸の曲率）
    #[default]
    Y,
    /// XZ 面（今は回転対称の系だけ。YZ 面と同じ値）
    X,
}

/// 近軸ガウスビームの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct GaussParams {
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Waist radius of the embedded Gaussian beam (1/e² intensity, in object space; the mixed-mode beam is √M² times larger)
    #[schemars(extend("x-min" = 0.0))]
    pub waist: f64,
    /// Distance from surface 1 to the waist (positive if the waist is after surface 1)
    pub surf1_to_waist: f64,
    /// M² factor of the beam (1 for a pure Gaussian)
    #[schemars(extend("x-min" = 1.0))]
    pub m2: f64,
    /// Cross section to trace (y or x)
    pub orientation: Orientation,
}

impl Default for GaussParams {
    fn default() -> Self {
        GaussParams {
            wave: None,
            waist: 0.5,
            surf1_to_waist: 0.0,
            m2: 1.0,
            orientation: Orientation::Y,
        }
    }
}

/// 近軸ガウスビームの 1 面（面の後の空間）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GaussRow {
    /// 面の番号
    pub surface: usize,
    /// 面の頂点でのビームの半径（1/e²、埋め込みのガウスビーム＝基本モード）
    #[serde(deserialize_with = "crate::nan::f")]
    pub size: f64,
    /// ウエストの半径（基本モード）
    #[serde(deserialize_with = "crate::nan::f")]
    pub waist: f64,
    /// ウエストから面の頂点までの距離（面の局所 z の向き。反射が偶数回なら、負は面がウエストより前＝ウエストはこの先）
    #[serde(deserialize_with = "crate::nan::f")]
    pub position: f64,
    /// 面の頂点での波面の曲率半径（面の局所 z の向き、反射が偶数回なら正は発散。ウエストの上なら null）
    pub radius: Option<f64>,
    /// 広がり角の半角 [rad]（媒質の中、基本モード）
    #[serde(deserialize_with = "crate::nan::f")]
    pub divergence: f64,
    /// M² の混合モードのビームの半径・ウエスト・広がり角（半径・ウエストは基本モードの √M² 倍、広がり角は atan(√M² w₀ / z_R)。曲率半径・位置・レイリー長は同じ）
    #[serde(deserialize_with = "crate::nan::f")]
    pub mixed_size: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub mixed_waist: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub mixed_divergence: f64,
    /// レイリー長（媒質の中）
    #[serde(deserialize_with = "crate::nan::f")]
    pub rayleigh: f64,
    /// 面の後の媒質の屈折率
    #[serde(deserialize_with = "crate::nan::f")]
    pub index: f64,
    /// 面の頂点の、展開した面 1 の頂点からの位置
    #[serde(deserialize_with = "crate::nan::f")]
    pub z: f64,
}

/// 近軸ガウスビームの結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GaussOut {
    pub unit: String,
    /// 波長 [µm]
    #[serde(deserialize_with = "crate::nan::f")]
    pub wavelength: f64,
    /// 波長の番号（1 始まり）
    pub wave: usize,
    pub m2: f64,
    pub orientation: Orientation,
    /// 面 1 から像面まで（座標ブレーク・無視する面は直前の面と同じ値）
    pub rows: Vec<GaussRow>,
    /// 光軸に沿ったビームの半径（基本モード。混合モードは √M² 倍）: 面と面の間ごとの (展開した面 1 からの位置, 半径) の並び（GRIN の媒質・無限の間隔は入れない）
    pub profile: Vec<Vec<[f64; 2]>>,
    pub warnings: Vec<String>,
}

pub(super) struct Gauss;

impl Analysis for Gauss {
    type Params = GaussParams;
    type Output = GaussOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "gaussian-beam",
            about: "Paraxial Gaussian beam: beam size, waist, waist position, wavefront radius, divergence and Rayleigh range after each surface",
            help: "Paraxial Gaussian beam (OpticStudio's Paraxial Gaussian Beam): the input beam given by the waist radius of its embedded Gaussian, the distance from surface 1 to the waist and M² is carried through the paraxial ABCD matrix of the system (from two paraxial rays), and for the space after each surface it reports the 1/e² beam radius at the vertex, the waist radius, the distance from the waist to the vertex (negative if the waist is downstream), the wavefront radius, the far-field half-angle divergence and the Rayleigh range in that medium, for the embedded Gaussian and for the mixed-mode beam (√M² times the radius and waist; divergence atan(√M² w₀ / z_R), √M² times at small angles). The position and the radius are signed along the local z of the surface (reversed after an odd number of mirrors, as in OpticStudio). The beam radius along the axis is in profile",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GaussParams) -> Result<GaussOut> {
        let s = cx.system;
        if p.orientation == Orientation::X && !s.symmetric() {
            return Err(Error::Invalid(
                "the x orientation of the Gaussian beam needs a rotationally symmetric system for now (use y)"
                    .into(),
            ));
        }
        let w = wave(s, p.wave)?.unwrap_or(s.primary);
        let g = gauss::gauss(
            s,
            &gauss::Beam {
                wave: w,
                waist: p.waist,
                surf1_to_waist: p.surf1_to_waist,
                m2: p.m2,
            },
        )?;
        let m = p.m2.sqrt();
        let mut warnings = Vec::new();
        if s.surfaces.iter().any(|d| d.grin().is_some()) {
            warnings.push(
                "the beam is carried through the GRIN media paraxially, but the profile is not drawn inside them"
                    .into(),
            );
        }
        Ok(GaussOut {
            unit: "mm".into(),
            wavelength: s.wavelengths[w],
            wave: w + 1,
            m2: p.m2,
            orientation: p.orientation,
            rows: g
                .rows
                .iter()
                .map(|r| GaussRow {
                    surface: r.surface,
                    size: r.size,
                    waist: r.waist,
                    position: r.position,
                    radius: r.radius.is_finite().then_some(r.radius),
                    divergence: r.divergence,
                    mixed_size: r.size * m,
                    mixed_waist: r.waist * m,
                    mixed_divergence: (r.waist * m / r.rayleigh).atan(),
                    rayleigh: r.rayleigh,
                    index: r.index,
                    z: r.z,
                })
                .collect(),
            profile: g.profile,
            warnings,
        })
    }
}
