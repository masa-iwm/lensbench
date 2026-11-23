//! スポット・波面・Zernike・ゴースト

use lensbench_core::Waves;
use std::path::PathBuf;

use crate::analysis::{self, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::{FieldSel, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, skip_vignetted, spot_unit, wave};
use crate::registry::{Analysis, Cx, Meta};

// ---------------------------------------------------------------------------------------- spot

/// スポットの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SpotParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Number of rings of the Gaussian quadrature
    #[schemars(extend("x-min" = 1))]
    pub rings: usize,
}

impl Default for SpotParams {
    fn default() -> Self {
        SpotParams {
            field: None,
            wave: None,
            rings: 6,
        }
    }
}

/// スポットの 1 視野
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpotRow {
    pub field: usize,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms_centroid: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms_chief: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub geo: f64,
    #[serde(deserialize_with = "crate::nan::a2")]
    pub centroid: [f64; 2],
    #[serde(deserialize_with = "crate::nan::a2")]
    pub chief: [f64; 2],
    /// Traced rays and why the excluded ones were dropped (summed over wavelengths, chief ray not counted)
    pub rays: RayCountsOut,
}

/// 追跡した光線の本数と、除いた光線の理由ごとの本数
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RayCountsOut {
    /// Traced rays
    pub total: usize,
    /// Rays clipped by a surface aperture
    pub vignetted: usize,
    /// Rays that missed a surface
    pub missed: usize,
    /// Rays stopped by total internal reflection
    pub tir: usize,
    /// Rays whose aiming failed
    pub aim_failed: usize,
}

impl From<lensbench_core::trace::RayCounts> for RayCountsOut {
    fn from(c: lensbench_core::trace::RayCounts) -> Self {
        RayCountsOut {
            total: c.total,
            vignetted: c.vignetted,
            missed: c.missed,
            tir: c.tir,
            aim_failed: c.aim_failed,
        }
    }
}

/// スポットの結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpotOut {
    /// 位置の単位（lens、アフォーカル像空間では角度の単位）
    pub unit: String,
    pub spot: Vec<SpotRow>,
}

pub(super) struct Spot;

impl Analysis for Spot {
    type Params = SpotParams;
    type Output = SpotOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "spot",
            about: "Spot (RMS and GEO radius)",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &SpotParams) -> Result<SpotOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let sp = analysis::spot(s, FieldSel::Index(i), Waves::all_or(w), p.rings)?;
            v.push(SpotRow {
                field: i + 1,
                rms_centroid: sp.rms_centroid,
                rms_chief: sp.rms_chief,
                geo: sp.geo,
                centroid: [sp.centroid.0, sp.centroid.1],
                chief: [sp.chief.0, sp.chief.1],
                rays: sp.rays.into(),
            });
        }
        Ok(SpotOut {
            unit: spot_unit(s),
            spot: v,
        })
    }
}

// ----------------------------------------------------------------------------------- wavefront

/// 波面収差の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct WavefrontParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// Grid side
    #[schemars(extend("x-min" = 1))]
    pub n: usize,
    /// Also output the wavefront grid ("p": normalized pupil coordinates, "map": W[iy][ix], null outside the pupil or when vignetted; the GUI wavefront map)
    pub map: bool,
}

impl Default for WavefrontParams {
    fn default() -> Self {
        WavefrontParams {
            field: None,
            wave: None,
            n: 64,
            map: false,
        }
    }
}

/// 波面収差の 1 視野
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WavefrontRow {
    pub field: usize,
    #[serde(deserialize_with = "crate::nan::f")]
    pub pv: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms: f64,
    /// 格子の正規化瞳座標（map のとき）
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nan::ov",
        default
    )]
    pub p: Option<Vec<f64>>,
    /// OPD [波] W[iy][ix]（map のとき。瞳の外・けられは null）
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nan::ovv",
        default
    )]
    pub map: Option<Vec<Vec<f64>>>,
}

/// 波面収差の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WavefrontOut {
    pub unit: String,
    pub wavefront: Vec<WavefrontRow>,
    /// 注意（全部けられて飛ばした視野など）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub(super) struct Wavefront;

impl Analysis for Wavefront {
    type Params = WavefrontParams;
    type Output = WavefrontOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "wavefront",
            about: "Wavefront aberration (OPD relative to the chief ray, P-V and RMS [waves])",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &WavefrontParams) -> Result<WavefrontOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut rows = Vec::new();
        for i in fields(s, p.field)? {
            // 基準はその波長の主光線（OpticStudio と同じ）
            let r =
                analysis::wavefront(s, FieldSel::Index(i), w, p.n, false).map(|wf| WavefrontRow {
                    field: i + 1,
                    pv: wf.pv,
                    rms: wf.rms,
                    p: p.map.then(|| wf.w.x().to_vec()),
                    map: p.map.then(|| wf.w.to_rows()),
                });
            rows.push((i, r));
        }
        let mut warnings = Vec::new();
        let v = skip_vignetted(rows, &mut warnings)?;
        Ok(WavefrontOut {
            unit: "waves".into(),
            wavefront: v,
            warnings,
        })
    }
}

// ------------------------------------------------------------------------------------- zernike

/// Zernike の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ZKind {
    /// Standard（Noll の順）
    #[default]
    Standard,
    /// Fringe（37 項まで）
    Fringe,
}

/// Zernike の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ZernikeParams {
    /// Field number (from 1; all fields if omitted)
    pub field: Option<usize>,
    /// Wavelength number (from 1; polychromatic if omitted)
    pub wave: Option<usize>,
    /// standard or fringe
    pub kind: ZKind,
    /// Number of terms
    #[schemars(extend("x-min" = 1))]
    pub terms: usize,
    /// Grid side
    #[schemars(extend("x-min" = 1))]
    pub n: usize,
}

impl Default for ZernikeParams {
    fn default() -> Self {
        ZernikeParams {
            field: None,
            wave: None,
            kind: ZKind::Standard,
            terms: 37,
            n: 64,
        }
    }
}

/// Zernike の 1 視野
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZernikeRow {
    pub field: usize,
    /// 係数 [波]（1 項目から）
    #[serde(deserialize_with = "crate::nan::v")]
    pub coef: Vec<f64>,
    #[serde(deserialize_with = "crate::nan::f")]
    pub pv: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms_chief: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms_centroid: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub rms_fit: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub strehl: f64,
}

/// Zernike の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZernikeOut {
    pub unit: String,
    pub kind: ZKind,
    pub zernike: Vec<ZernikeRow>,
    /// 注意（全部けられて飛ばした視野など）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub(super) struct Zernike;

impl Analysis for Zernike {
    type Params = ZernikeParams;
    type Output = ZernikeOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "zernike",
            about: "Zernike coefficients [waves]",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ZernikeParams) -> Result<ZernikeOut> {
        let k = match p.kind {
            ZKind::Standard => ZernKind::Standard,
            ZKind::Fringe => ZernKind::Fringe,
        };
        if k == ZernKind::Fringe && p.terms > 37 {
            return Err(lensbench_core::Error::Invalid(
                "Fringe has at most 37 terms".into(),
            ));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut rows = Vec::new();
        for i in fields(s, p.field)? {
            let r = analysis::zernike(s, FieldSel::Index(i), w, k, p.terms, p.n, false).map(|z| {
                ZernikeRow {
                    field: i + 1,
                    coef: z.coef,
                    pv: z.pv,
                    rms_chief: z.rms_chief,
                    rms_centroid: z.rms_centroid,
                    rms_fit: z.rms_fit,
                    strehl: z.strehl,
                }
            });
            rows.push((i, r));
        }
        let mut warnings = Vec::new();
        let v = skip_vignetted(rows, &mut warnings)?;
        Ok(ZernikeOut {
            unit: "waves".into(),
            kind: p.kind,
            zernike: v,
            warnings,
        })
    }
}

// --------------------------------------------------------------------------------------- ghost

/// ゴーストの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct GhostParams {
    /// First surface to reflect at
    pub first: usize,
    /// Last surface to reflect at (the image surface if omitted; the image surface reflects only when it has a coating)
    pub last: Option<usize>,
    /// Measured coating tables (LBCT) and models (LBCM) (more than one allowed)
    #[schemars(extend("x-kind" = "path"))]
    pub coatings: Vec<PathBuf>,
    /// Folder to write the expanded ghost systems to (ghost_a_b.zmx)
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out: Option<PathBuf>,
}

impl Default for GhostParams {
    fn default() -> Self {
        GhostParams {
            first: 1,
            last: None,
            coatings: Vec::new(),
            out: None,
        }
    }
}

/// ゴーストの 1 組
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GhostRow {
    /// 先に反射する面
    pub a: usize,
    /// 後で反射する面
    pub b: usize,
    #[serde(deserialize_with = "crate::nan::f")]
    pub marginal_y: f64,
    #[serde(deserialize_with = "crate::nan::f")]
    pub marginal_u: f64,
    /// ゴーストの焦点の位置（無限遠は null）
    pub focus: Option<f64>,
    #[serde(deserialize_with = "crate::nan::f")]
    pub wfno: f64,
    /// 軸上の主光線の強度の比（膜込み）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    /// 強度を求められなかった理由
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_error: Option<String>,
}

/// ゴーストの結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GhostOut {
    pub first: usize,
    pub last: usize,
    pub ghosts: Vec<GhostRow>,
    /// 書いた .zmx
    pub written: Vec<String>,
    /// .zmx に書けなかったものなどの注意
    pub warnings: Vec<String>,
}

pub(super) struct Ghost;

impl Analysis for Ghost {
    type Params = GhostParams;
    type Output = GhostOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "ghost",
            about: "Ghosts (double reflections): focus position, blur at the image and intensity ratio",
            help: "List of ghosts (reflected at surface a then at surface b, the double bounce of OpticStudio's Ghost Focus Generator): paraxial ghost focus position, blur radius at the image and on-axis chief ray intensity ratio (with coatings). out writes the expanded systems to .zmx files in a folder",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GhostParams) -> Result<GhostOut> {
        let s = cx.system;
        let last = p.last.unwrap_or(s.image());
        let mut co = Coatings::for_system(s)?;
        for f in &p.coatings {
            co.load_file(f)?;
        }
        let v = crate::ghost::survey(s, p.first, last, &co)?;
        let (mut written, mut warnings) = (Vec::new(), co.notes.clone());
        if let Some(dir) = &p.out {
            lensbench_core::vfs::create_dir_all(dir)
                .map_err(|x| lensbench_core::Error::Io(format!("{}: {x}", dir.display())))?;
            for g in &v {
                let gs = crate::ghost::ghost_system(s, g.par.a, g.par.b)?;
                let f = dir.join(format!("ghost_{}_{}.zmx", g.par.a, g.par.b));
                warnings.extend(lensbench_formats::zmx_write::save(&f, &gs.sys)?);
                written.push(f.display().to_string());
            }
        }
        let ghosts = v
            .iter()
            .map(|g| {
                let q = &g.par;
                let (intensity, intensity_error) = match &g.intensity {
                    Ok(x) => (Some(*x), None),
                    Err(m) => (None, Some(m.to_string())),
                };
                GhostRow {
                    a: q.a,
                    b: q.b,
                    marginal_y: q.marginal_y,
                    marginal_u: q.marginal_u,
                    focus: q.focus.is_finite().then_some(q.focus),
                    wfno: q.wfno,
                    intensity,
                    intensity_error,
                }
            })
            .collect();
        Ok(GhostOut {
            first: p.first,
            last,
            ghosts,
            written,
            warnings,
        })
    }
}
