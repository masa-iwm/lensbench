//! スポット・波面・Zernike・ゴースト

use lensbench_core::Waves;
use std::path::PathBuf;

use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::{FieldSel, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{fields, spot_unit, wave};
use crate::registry::{Analysis, Cx, Meta};

// ---------------------------------------------------------------------------------------- spot

/// スポットの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SpotParams {
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// ガウス求積のリング数
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct SpotRow {
    pub field: usize,
    pub rms_centroid: f64,
    pub rms_chief: f64,
    pub geo: f64,
    pub centroid: [f64; 2],
    pub chief: [f64; 2],
}

/// スポットの結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct SpotOut {
    /// 位置の単位（lens、アフォーカル像空間では角度の単位）
    pub unit: String,
    pub spot: Vec<SpotRow>,
}

pub struct Spot;

impl Analysis for Spot {
    type Params = SpotParams;
    type Output = SpotOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "spot",
            about: "スポット（RMS・GEO 半径）",
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
pub struct WavefrontParams {
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// 格子の一辺
    #[schemars(extend("x-min" = 1))]
    pub n: usize,
    /// 波面の格子も出す（"p": 正規化瞳座標、"map": W[iy][ix]、瞳の外・けられは null。GUI の波面図）
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct WavefrontRow {
    pub field: usize,
    pub pv: f64,
    pub rms: f64,
    /// 格子の正規化瞳座標（map のとき）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p: Option<Vec<f64>>,
    /// OPD [波] W[iy][ix]（map のとき。瞳の外・けられは null）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map: Option<Vec<Vec<f64>>>,
}

/// 波面収差の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct WavefrontOut {
    pub unit: String,
    pub wavefront: Vec<WavefrontRow>,
}

pub struct Wavefront;

impl Analysis for Wavefront {
    type Params = WavefrontParams;
    type Output = WavefrontOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "wavefront",
            about: "波面収差（主光線基準の OPD、P-V・RMS [波]）",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &WavefrontParams) -> Result<WavefrontOut> {
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            // 基準はその波長の主光線（OpticStudio と同じ）
            let wf = analysis::wavefront(s, FieldSel::Index(i), w, p.n, false)?;
            v.push(WavefrontRow {
                field: i + 1,
                pv: wf.pv,
                rms: wf.rms,
                p: p.map.then(|| wf.p.clone()),
                map: p.map.then_some(wf.w),
            });
        }
        Ok(WavefrontOut {
            unit: "waves".into(),
            wavefront: v,
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
pub struct ZernikeParams {
    /// 視野の番号（1 始まり、省略で全視野）
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    pub wave: Option<usize>,
    /// standard または fringe
    pub kind: ZKind,
    /// 項数
    #[schemars(extend("x-min" = 1))]
    pub terms: usize,
    /// 格子の一辺
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct ZernikeRow {
    pub field: usize,
    /// 係数 [波]（1 項目から）
    pub coef: Vec<f64>,
    pub pv: f64,
    pub rms_chief: f64,
    pub rms_centroid: f64,
    pub rms_fit: f64,
    pub strehl: f64,
}

/// Zernike の結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct ZernikeOut {
    pub unit: String,
    pub kind: ZKind,
    pub zernike: Vec<ZernikeRow>,
}

pub struct Zernike;

impl Analysis for Zernike {
    type Params = ZernikeParams;
    type Output = ZernikeOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "zernike",
            about: "Zernike 係数 [波]",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ZernikeParams) -> Result<ZernikeOut> {
        let k = match p.kind {
            ZKind::Standard => ZernKind::Standard,
            ZKind::Fringe => ZernKind::Fringe,
        };
        if k == ZernKind::Fringe && p.terms > 37 {
            return Err(lensbench_core::Error::Invalid("Fringe は 37 項まで".into()));
        }
        let s = cx.system;
        let w = wave(s, p.wave)?;
        let mut v = Vec::new();
        for i in fields(s, p.field)? {
            let z = analysis::zernike(s, FieldSel::Index(i), w, k, p.terms, p.n, false)?;
            v.push(ZernikeRow {
                field: i + 1,
                coef: z.coef,
                pv: z.pv,
                rms_chief: z.rms_chief,
                rms_centroid: z.rms_centroid,
                rms_fit: z.rms_fit,
                strehl: z.strehl,
            });
        }
        Ok(ZernikeOut {
            unit: "waves".into(),
            kind: p.kind,
            zernike: v,
        })
    }
}

// --------------------------------------------------------------------------------------- ghost

/// ゴーストの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GhostParams {
    /// 反射させる最初の面
    pub first: usize,
    /// 反射させる最後の面（省略で像面。像面は膜があるときだけ反射させる）
    pub last: Option<usize>,
    /// 膜の実測表（LBCT）・モデル（LBCM）のファイル（複数可）
    #[schemars(extend("x-kind" = "path"))]
    pub coatings: Vec<PathBuf>,
    /// 展開したゴーストの系を書くフォルダ（ghost_a_b.zmx）
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out_dir: Option<PathBuf>,
}

impl Default for GhostParams {
    fn default() -> Self {
        GhostParams {
            first: 1,
            last: None,
            coatings: Vec::new(),
            out_dir: None,
        }
    }
}

/// ゴーストの 1 組
#[derive(Debug, Serialize, JsonSchema)]
pub struct GhostRow {
    /// 先に反射する面
    pub a: usize,
    /// 後で反射する面
    pub b: usize,
    pub marginal_y: f64,
    pub marginal_u: f64,
    /// ゴーストの焦点の位置（無限遠は null）
    pub focus: Option<f64>,
    pub wfno: f64,
    /// 軸上の主光線の強度の比（膜込み）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    /// 強度を求められなかった理由
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_error: Option<String>,
}

/// ゴーストの結果
#[derive(Debug, Serialize, JsonSchema)]
pub struct GhostOut {
    pub first: usize,
    pub last: usize,
    pub ghosts: Vec<GhostRow>,
    /// 書いた .zmx
    pub written: Vec<String>,
    /// .zmx に書けなかったものなどの注意
    pub warnings: Vec<String>,
}

pub struct Ghost;

impl Analysis for Ghost {
    type Params = GhostParams;
    type Output = GhostOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "ghost",
            about: "ゴースト（2 回反射）の一覧: 焦点の位置・像面でのぼけ・強度の比",
            help: "ゴースト（面 a で反射し、面 b で反射する 2 回反射、OpticStudio の Ghost Focus Generator の double bounce）の一覧: \
                   近軸のゴーストの焦点の位置・像面でのぼけの半径と、軸上の主光線の強度の比（膜込み）。out_dir で展開した系を .zmx に書く",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &GhostParams) -> Result<GhostOut> {
        let s = cx.system;
        let last = p.last.unwrap_or(s.image());
        let mut co = Coatings::for_system(s)?;
        for f in &p.coatings {
            co.load_file(f)?;
        }
        let v = lensbench_core::ghost::survey(s, p.first, last, &co)?;
        let (mut written, mut warnings) = (Vec::new(), Vec::new());
        if let Some(dir) = &p.out_dir {
            lensbench_core::vfs::create_dir_all(dir)
                .map_err(|x| lensbench_core::Error::Io(format!("{}: {x}", dir.display())))?;
            for g in &v {
                let gs = lensbench_core::ghost::ghost_system(s, g.par.a, g.par.b)?;
                let f = dir.join(format!("ghost_{}_{}.zmx", g.par.a, g.par.b));
                warnings.extend(lensbench_core::zmx_write::save(&f, &gs.sys)?);
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
