//! 系の設定の型（視野・開口・エイミング・座標系・近軸量・物体）

use super::*;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum FieldType {
    Angle,
    ObjH,
    ParH,
    RealH,
    /// 条件で定める視野: 視野の (x, y) が条件の目標値になる物体（有限なら物点、無限遠なら方向）を、
    /// 主波長の主光線で解く。実像高は `Solve(FieldCond::Position { surf: 像面 })` と同じ
    Solve(FieldCond),
}

/// 視野の種類（[`FieldType`] の条件を除いたもの。プロジェクトの `fields.type`・系ファイル版 2 の `field.type` の名前）。
/// JSON の名前は "angle"・"object_height"・"paraxial_image_height"・"real_image_height"・"solve"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum FieldKind {
    #[default]
    Angle,
    ObjectHeight,
    ParaxialImageHeight,
    RealImageHeight,
    /// 条件で定める視野（[`FieldType::Solve`]）
    Solve,
}

impl FieldKind {
    /// 視野の種類と条件
    pub fn of(t: FieldType) -> (FieldKind, Option<FieldCond>) {
        match t {
            FieldType::Angle => (FieldKind::Angle, None),
            FieldType::ObjH => (FieldKind::ObjectHeight, None),
            FieldType::ParH => (FieldKind::ParaxialImageHeight, None),
            FieldType::RealH => (FieldKind::RealImageHeight, None),
            FieldType::Solve(c) => (FieldKind::Solve, Some(c)),
        }
    }

    /// 条件のない種類の [`FieldType`]（Solve は None）
    pub fn plain(self) -> Option<FieldType> {
        match self {
            FieldKind::Angle => Some(FieldType::Angle),
            FieldKind::ObjectHeight => Some(FieldType::ObjH),
            FieldKind::ParaxialImageHeight => Some(FieldType::ParH),
            FieldKind::RealImageHeight => Some(FieldType::RealH),
            FieldKind::Solve => None,
        }
    }
}

/// 条件で定める視野の条件（主光線、局所座標）
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(tag = "cond", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldCond {
    /// 面 surf での位置 (x, y) = 視野の値（JSON の欄の名前は surface）
    Position {
        #[serde(rename = "surface")]
        surf: usize,
    },
    /// 面 s1 と s2 での方向余弦 (l, m) の和 = 視野の値（法線入射で戻る主光線なら 0）
    DirSum { s1: usize, s2: usize },
    /// 面 s1 と s2 での方向余弦 (l, m) の差（s1 − s2）= 視野の値
    DirDiff { s1: usize, s2: usize },
}
/// 視野の正規化（瞳・視野の座標の 1 を何にとるか。.zmx の FTYP の 6 番目）。JSON の名前は "radial"・"rect"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum FieldNorm {
    /// 半径（最も遠い視野が 1）
    #[default]
    Radial,
    /// 矩形（x・y の最大がそれぞれ 1）
    Rect,
}
/// 定義した視野以外の点のケラレ係数の求め方（[`System::vig_factors`]）
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub enum VigInterp {
    /// OpticStudio 2026 R1.01 と同じ（y だけの視野は r² で補間、ほかは L1 で最も近い視野の値。視野の間で段差になる）。
    /// OpticStudio と照合するとき
    #[serde(rename = "opticstudio")]
    OpticStudio,
    /// 連続（既定、2026-11-13 masa 決定）: y だけの視野は r² で補間（外は最も外の視野の値。OpticStudio と同じ区間は
    /// 同じ値）、ほかは視野の点の Delaunay 三角形分割で線形、外側は凸包の縁の最も近い点の値
    #[default]
    #[serde(rename = "continuous")]
    Continuous,
}

/// 系の開口の種類。JSON の名前は "epd"・"fnum"・"na"・"float"・"pwfn"・"cone"
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub enum ApertureKind {
    /// 入射瞳径
    #[default]
    #[serde(rename = "epd")]
    Epd,
    /// 像側 F 値
    #[serde(rename = "fnum")]
    FNum,
    /// 物体側 NA
    #[serde(rename = "na")]
    Na,
    /// 絞り径（絞りの半直径）
    #[serde(rename = "float")]
    Float,
    /// 近軸作動 F 値
    #[serde(rename = "pwfn")]
    Pwfn,
    /// 物体側コーン角
    #[serde(rename = "cone")]
    Cone,
}
/// アフォーカル像空間の角度の単位（.zmx の UNIT の値の 6 番目、OpticStudio の Afocal Mode Units）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum AfocalUnit {
    Microradians,
    #[default]
    Milliradians,
    Radians,
    ArcSeconds,
    ArcMinutes,
    Degrees,
}

impl AfocalUnit {
    /// 1 単位あたりのラジアン
    pub fn radians(self) -> f64 {
        match self {
            AfocalUnit::Microradians => 1e-6,
            AfocalUnit::Milliradians => 1e-3,
            AfocalUnit::Radians => 1.0,
            AfocalUnit::ArcSeconds => std::f64::consts::PI / 648_000.0,
            AfocalUnit::ArcMinutes => std::f64::consts::PI / 10_800.0,
            AfocalUnit::Degrees => std::f64::consts::PI / 180.0,
        }
    }
    /// .zmx の UNIT 行の記号（UR・MR・RR・AS・AM・DG）
    pub fn code(self) -> &'static str {
        match self {
            AfocalUnit::Microradians => "UR",
            AfocalUnit::Milliradians => "MR",
            AfocalUnit::Radians => "RR",
            AfocalUnit::ArcSeconds => "AS",
            AfocalUnit::ArcMinutes => "AM",
            AfocalUnit::Degrees => "DG",
        }
    }
    pub fn from_code(c: &str) -> Option<AfocalUnit> {
        [
            AfocalUnit::Microradians,
            AfocalUnit::Milliradians,
            AfocalUnit::Radians,
            AfocalUnit::ArcSeconds,
            AfocalUnit::ArcMinutes,
            AfocalUnit::Degrees,
        ]
        .into_iter()
        .find(|u| u.code().eq_ignore_ascii_case(c))
    }
    /// 表示の名前（urad・mrad・rad・arcsec・arcmin・deg）
    pub fn label(self) -> &'static str {
        match self {
            AfocalUnit::Microradians => "urad",
            AfocalUnit::Milliradians => "mrad",
            AfocalUnit::Radians => "rad",
            AfocalUnit::ArcSeconds => "arcsec",
            AfocalUnit::ArcMinutes => "arcmin",
            AfocalUnit::Degrees => "deg",
        }
    }
}

/// 光線エイミング。JSON の名前は "off"・"paraxial"・"real"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum Aiming {
    #[default]
    Off,
    Paraxial,
    Real,
}

#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Field {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub vdx: f64,
    pub vdy: f64,
    pub vcx: f64,
    pub vcy: f64,
    pub van: f64,
}

impl Field {
    pub fn xy(x: f64, y: f64) -> Field {
        Field {
            x,
            y,
            w: 1.0,
            vdx: 0.0,
            vdy: 0.0,
            vcx: 0.0,
            vcy: 0.0,
            van: 0.0,
        }
    }
}

/// 面の座標系（原点、列が軸の回転行列）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub o: V3,
    pub r: M3,
}

/// 局所座標の間の変換 q = r · p + t。`shift` は r が単位行列で t = (0, 0, shift) のとき（共軸の面の間）
#[derive(Debug, Clone, Copy)]
pub struct Rel {
    pub r: M3,
    pub t: V3,
    pub shift: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Frames {
    /// 面ごと（無視する面と、無限遠の物体面は None）
    pub surf: Arc<Vec<Option<Frame>>>,
    /// 面ごとの、直前の有効な面の局所座標からこの面の局所座標への変換（追跡の内側で大域座標を経ずに進むため）
    pub rel: Arc<Vec<Option<Rel>>>,
    /// 面 1 の変換の前の座標系（入射瞳を測る物体空間の軸）
    pub obj: Frame,
}

#[derive(Debug, Clone)]
pub struct Paraxial {
    pub enpp: f64,
    pub epd: f64,
    pub efl: f64,
    pub expp: f64,
    pub expd: f64,
    pub stop: usize,
    pub stop_radius: f64,
    pub u_obj: Option<f64>,
    pub marginal: (Vec<f64>, Vec<f64>),
    pub chief: (Vec<f64>, Vec<f64>),
    pub chief_image_per_unit: f64,
    /// 主光線の近軸像面（マージナル光線の高さが 0 の面）での高さ。近軸像高の視野はこの面で決める（OpticStudio と同じ）
    pub chief_parimage_per_unit: f64,
    pub wfno: f64,
    pub isfn: f64,
    pub pmag: f64,
}

/// 物体: 無限遠なら物体空間の単位方向、有限なら物体面上の点 (x, y)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjSpec {
    Dir(V3),
    Point([f64; 2]),
}
