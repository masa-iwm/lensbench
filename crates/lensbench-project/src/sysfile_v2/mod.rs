//! 系ファイル v2 の `system` の形（PRJ-24・IOO-10）: 利用者が書いた値（処方）だけを、core の内部の構造体と
//! 切り離した形で書く。[`System`] との変換は [`write`]（系 → 処方）と [`read`]（処方 → 系）。
//!
//! - 面は 1 行に「面 ＋ その後の間隔」（番号は今どおり OpticStudio と同じ）。行は形（`shape`、種類ごとの値だけ）・
//!   働き（`action`、屈折は省く）・開口・膜・絞り・半直径（数か "auto"）・前後の傾き偏心・無視・コメント・
//!   厚さ `t`・媒質（`medium`）。座標ブレークは `coordinate_break` の行。既定値の欄と種類に関係のない欄は省く。
//! - 曲率は半径でなく `c` で書く（往復でビットが変わらない）。
//! - 媒質は省略（空気。鏡・座標ブレークでは前の媒質）・ガラス名・モデルガラス（値）・GRIN のデータ。
//!   屈折率のずれ dn・disp は行に書く。
//! - 書かないもの: 求めた屈折率、自動の半直径の値、ガラスの式（名前で引き、`catalogs` に使ったカタログの
//!   名前とファイルの hash を残す）、`field_wave`・`import_notes`・`macro_notes`。

pub(crate) mod catalogs;
pub(crate) mod read;
pub(crate) mod write;

use lensbench_core::coating::Table;
use lensbench_core::edit::Configs;
use lensbench_core::freeform::{GridSag, ZernSag};
use lensbench_core::grin::GrinKind;
use lensbench_core::mech::MechPart;
use lensbench_core::param::{Pickup, ThickSolve};
use lensbench_core::plugin::Config;
use lensbench_core::surface::{Binary2, CylFresnel};
use lensbench_core::system::{AfocalUnit, Aiming, ApertureKind, FieldNorm, VigInterp};
use lensbench_core::units::Unit;
use lensbench_core::zpl_solve::{Macro, MacroParm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn yes() -> bool {
    true
}
fn is_yes(b: &bool) -> bool {
    *b
}
fn no(b: &bool) -> bool {
    !*b
}
fn one() -> usize {
    1
}
fn is_one(v: &usize) -> bool {
    *v == 1
}
fn one_f() -> f64 {
    1.0
}
fn is_one_f(v: &f64) -> bool {
    v.to_bits() == 1f64.to_bits()
}
/// +0（-0 は書く。ビット一致のため）
fn zero(v: &f64) -> bool {
    v.to_bits() == 0
}
fn zeros<const N: usize>(v: &[f64; N]) -> bool {
    v.iter().all(zero)
}
fn t20() -> f64 {
    20.0
}
fn is_t20(v: &f64) -> bool {
    v.to_bits() == 20f64.to_bits()
}
fn glrs1() -> usize {
    1
}
fn ropd2() -> i64 {
    2
}
fn is_ropd2(v: &i64) -> bool {
    *v == 2
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// 系（処方）。系ファイルの `system`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Prescription {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub wavelengths: Vec<Wave>,
    /// 主波長の番号（1 始まり）
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub primary: usize,
    pub fields: Fields,
    pub aperture: ApertureSpec,
    /// 物体側テレセントリック
    #[serde(default, skip_serializing_if = "no")]
    pub telecentric: bool,
    /// 光線エイミング
    #[serde(default, skip_serializing_if = "is_default")]
    pub aiming: Aiming,
    /// 温度 [°C]
    #[serde(default = "t20", skip_serializing_if = "is_t20")]
    pub temperature_c: f64,
    /// 気圧 [atm]
    #[serde(default = "one_f", skip_serializing_if = "is_one_f")]
    pub pressure_atm: f64,
    /// ガラスの屈折率を温度・気圧で補正する（OpticStudio の Adjust Index Data To Environment）
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub adjust_index: bool,
    /// 像空間がアフォーカル
    #[serde(default, skip_serializing_if = "no")]
    pub afocal: bool,
    /// アフォーカル像空間の角度の単位
    #[serde(default, skip_serializing_if = "is_default")]
    pub afocal_unit: AfocalUnit,
    /// 定義した視野以外の点のケラレ係数の求め方
    #[serde(default, skip_serializing_if = "is_default")]
    pub vig_interp: VigInterp,
    /// 浮動開口（半直径を固定した面）の外の光線をけるか
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub floating_clip: bool,
    /// 大域座標の基準の面（.zmx の GLRS）
    #[serde(default = "glrs1", skip_serializing_if = "is_one")]
    pub glrs: usize,
    /// OPD の基準（.zmx の ROPD）
    #[serde(default = "ropd2", skip_serializing_if = "is_ropd2")]
    pub ropd: i64,
    /// 瞳のアポダイゼーション（.zmx の GFAC。FFT PSF・Huygens・回折 MTF の瞳の振幅）
    #[serde(default, skip_serializing_if = "is_default")]
    pub apodization: lensbench_core::pupil::Apodization,
    /// 膜ファイル（.zmx の COFN で見つけたファイル）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coating_file: Option<String>,
    /// 膜ファイルの外で定めた TABLE 膜（名前 → 表）
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coat_tables: BTreeMap<String, Table>,
    /// 面の列（面とその後の間隔）。0 が物体面、最後が像面
    pub surfaces: Vec<Row>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pickups: Vec<Pickup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solves: Vec<ThickSolve>,
    /// ソルブが呼ぶ ZPL のマクロ（文を入れる）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<Macro>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_parms: Vec<MacroParm>,
    /// 玉・群
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<Unit>,
    /// 機構部品
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanics: Vec<MechPart>,
    /// 多重構成の表
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configs: Option<Configs>,
    /// 使ったガラスのカタログ（名前で引く順）と、保存したときのファイルの hash
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<CatalogRef>,
}

/// 波長
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wave {
    /// 波長 [µm]
    pub um: f64,
    #[serde(default = "one_f", skip_serializing_if = "is_one_f")]
    pub weight: f64,
}

/// 視野
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    #[serde(rename = "type", default, skip_serializing_if = "is_default")]
    pub kind: FieldKind,
    /// 視野の (x, y)
    pub points: Vec<[f64; 2]>,
    /// 視野ごとの重み（省略はすべて 1）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<f64>,
    /// type = solve のときの条件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solve: Option<FieldSolve>,
    /// 正規化
    #[serde(default, skip_serializing_if = "is_default")]
    pub norm: FieldNorm,
    /// 視野ごとのケラレ係数 [VDX, VDY, VCX, VCY, VAN]（省略はすべて 0）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vignetting: Vec<[f64; 5]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Angle,
    ObjectHeight,
    ParaxialImageHeight,
    RealImageHeight,
    /// 条件で定める視野（solve に条件、points が目標値）
    Solve,
}

/// 条件で定める視野の条件（面は番号）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "cond", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldSolve {
    /// 面 surface での主光線の位置 = 視野の値
    Position { surface: usize },
    /// 面 s1・s2 での主光線の方向余弦 (l, m) の和 = 視野の値
    DirSum { s1: usize, s2: usize },
    /// 差（s1 − s2）= 視野の値
    DirDiff { s1: usize, s2: usize },
}

/// 系の開口
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApertureSpec {
    #[serde(rename = "type")]
    pub kind: ApertureKind,
    pub value: f64,
}

/// 使ったガラスのカタログ
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogRef {
    /// カタログの名前（AGF のファイル名から拡張子を除いたもの）
    pub name: String,
    /// 保存したときのカタログのファイルの中身の SHA-256（"sha256:…"）。ファイルが見つからなかったときは空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

/// 面の列の 1 行（面とその後の間隔）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// 物体面の印（最初の行。読むときは確かめるだけ）
    #[serde(default, skip_serializing_if = "no")]
    pub object: bool,
    /// 像面の印（最後の行）
    #[serde(default, skip_serializing_if = "no")]
    pub image: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// 座標ブレーク（面でなく座標系だけを変える行）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_break: Option<Tilt>,
    /// 面の形（省略は平面の標準面）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    /// 面の働き（省略は屈折）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    /// 面の開口（省略は開口なし）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aperture: Option<Aperture>,
    /// 膜の名前
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub coating: String,
    /// 絞り
    #[serde(default, skip_serializing_if = "no")]
    pub stop: bool,
    /// 半直径 [mm]。"auto" は光線から求める
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semi: Option<Semi>,
    /// 面の前の傾き・偏心
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Tilt>,
    /// 面の後の傾き・偏心（"reverse" は前の変換を戻す）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<After>,
    /// 無視する
    #[serde(default, skip_serializing_if = "no")]
    pub ignore: bool,
    /// 次の面までの厚さ [mm]
    #[serde(default, skip_serializing_if = "zero")]
    pub t: f64,
    /// 面の後の媒質（省略は空気。鏡・座標ブレークでは前の媒質）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<Medium>,
    /// 媒質の屈折率のずれ（全波長に足す）
    #[serde(default, skip_serializing_if = "zero")]
    pub dn: f64,
    /// 媒質の分散の相対変化: n = n0(λ) + dn + disp·(n0(λ) − n0(主波長))
    #[serde(default, skip_serializing_if = "zero")]
    pub disp: f64,
}

/// 傾き・偏心（座標ブレーク・面の前後）
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tilt {
    /// 偏心 x・y [mm]
    #[serde(default, skip_serializing_if = "zeros")]
    pub decenter: [f64; 2],
    /// 傾き x・y・z [deg]
    #[serde(default, skip_serializing_if = "zeros")]
    pub tilt: [f64; 3],
    /// 順序（0: 偏心 → X → Y → Z、1: Z → Y → X → 偏心）
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub order: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reverse {
    Reverse,
}

/// 面の後の傾き・偏心
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum After {
    /// "reverse": 前の変換を戻す
    Reverse(Reverse),
    Tilt(Tilt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Auto {
    Auto,
}

/// 半直径
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Semi {
    Value(f64),
    /// "auto": 光線から求める
    Auto(Auto),
}

/// 面の形（種類ごとの値だけ）。曲率は c [1/mm]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    /// 二次曲面（標準面）
    Conic {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
    },
    /// 偶数次非球面: a は r², r⁴, … の係数
    EvenAsphere {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        a: Vec<f64>,
    },
    /// Odd Asphere: b は r¹, r², … の係数
    OddAsphere {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        b: Vec<f64>,
    },
    /// トロイダル面: YZ 面の曲線（c・k・y², y⁴, … の係数 a）を回転の曲率 cx で回した面（cx = 0 は円柱面）
    Toroid {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        a: Vec<f64>,
        #[serde(default, skip_serializing_if = "zero")]
        cx: f64,
    },
    /// Zernike Standard Sag
    ZernikeStandard(ZernikeShape),
    /// Zernike Fringe Sag
    ZernikeFringe(ZernikeShape),
    /// Grid Sag
    GridSag {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        a: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terms: Option<ZernSag>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid: Option<GridSag>,
        /// 格子の .DAT のファイル名（書き出し用）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        grid_file: String,
    },
    /// Tilted: 平面 z = x tan α + y tan β。c・k は交点を求める初めの値にだけ使う（.zmx から読んだ値が残るとき）
    Tilted {
        #[serde(default, skip_serializing_if = "zeros")]
        tan: [f64; 2],
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
    },
    /// Irregular: 偏心・傾けた標準面に Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y（ρ は半直径で正規化）
    Irregular {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "zeros")]
        decenter: [f64; 2],
        /// 傾き x・y [deg]
        #[serde(default, skip_serializing_if = "zeros")]
        tilt: [f64; 2],
        #[serde(default, skip_serializing_if = "zero")]
        zs: f64,
        #[serde(default, skip_serializing_if = "zero")]
        za: f64,
        #[serde(default, skip_serializing_if = "zero")]
        zc: f64,
        /// 項を回す角度 [deg]
        #[serde(default, skip_serializing_if = "zero")]
        angle: f64,
    },
    /// y だけの偶数次非球面（Cylinder Fresnel の基板。働きは cylinder_fresnel）
    Cylinder {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        a: Vec<f64>,
    },
}

/// Zernike Sag の形
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZernikeShape {
    #[serde(default, skip_serializing_if = "zero")]
    pub c: f64,
    #[serde(default, skip_serializing_if = "zero")]
    pub k: f64,
    /// 土台の偶数次非球面の係数
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub a: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<ZernSag>,
    /// Grid Sag から種類を変えた面に残った格子（近軸の曲率にだけ効く）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSag>,
}

/// 面の働き（屈折は省く）。mirror は反射してから位相・傾きを加える
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// 反射（鏡）
    Reflect,
    /// 屈折面での反射（ゴーストの経路の解析の写し）。medium は光が入らない側の媒質
    GhostReflect { medium: String },
    /// 理想レンズ（焦点距離 f、OPD の求め方 opd_mode）
    Paraxial {
        f: f64,
        #[serde(default, skip_serializing_if = "is_zero_i32")]
        opd_mode: i32,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
    /// Jones 行列 [Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]、または波長ごとに作る偏光子・波長板
    Jones {
        #[serde(default = "identity", skip_serializing_if = "is_identity")]
        matrix: [f64; 8],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retarder: Option<Retarder>,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
    /// Binary 2: 位相 Φ = order · Σ coef[i] ρ^{2(i+1)}（形は偶数次非球面）
    Binary2 {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<Binary2>,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
    /// Fresnel: 交点は頂点の平面、屈折は偶数次非球面（c・k・a）の傾きで
    Fresnel {
        #[serde(default, skip_serializing_if = "zero")]
        c: f64,
        #[serde(default, skip_serializing_if = "zero")]
        k: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        a: Vec<f64>,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
    /// Cylinder Fresnel: 交点は形（cylinder）、屈折は bend の面の傾きで
    CylinderFresnel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bend: Option<CylFresnel>,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
    /// 複屈折材の板の入口（mode 0: 常光の成分だけ、2: 波長板。axis は結晶軸）
    BirefringentIn { mode: u8, axis: [f64; 3] },
    /// 複屈折材の板の出口
    BirefringentOut,
    /// 利用者定義の面（登録のモデルの名前と設定）
    User {
        model: String,
        #[serde(default)]
        config: Config,
        #[serde(default, skip_serializing_if = "no")]
        mirror: bool,
    },
}

fn identity() -> [f64; 8] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}
fn is_identity(m: &[f64; 8]) -> bool {
    m.iter()
        .zip(identity())
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

/// 偏光子・波長板（Jones の面）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retarder {
    pub axis_deg: f64,
    pub tx: f64,
    pub ty: f64,
    /// 設計波長での遅延 [波長]
    pub retardance: f64,
    #[serde(default)]
    pub order: u32,
    /// 設計波長 [µm]
    pub wl0: f64,
    /// 複屈折材の板（ガラスは名前で引く）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plates: Vec<Plate>,
}

/// 波長板の複屈折材の板
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Plate {
    pub material: String,
    /// 厚さ [mm]
    pub thickness: f64,
    pub axis_deg: f64,
}

/// 面の開口
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Aperture {
    /// 円（最小・最大半径、偏心）
    Circular {
        #[serde(default, skip_serializing_if = "zero")]
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dx: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dy: f64,
    },
    /// 矩形（x・y の半幅、偏心）
    Rect {
        hx: f64,
        hy: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dx: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dy: f64,
    },
    /// 浮動開口（半径は面の半直径。違うときだけ semi）
    Floating {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        semi: Option<f64>,
    },
    /// 楕円（x・y の半軸、偏心）
    Ellipse {
        hx: f64,
        hy: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dx: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dy: f64,
    },
    /// 円形遮蔽（最小〜最大半径の間を遮る）
    Obscuration {
        #[serde(default, skip_serializing_if = "zero")]
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dx: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dy: f64,
    },
    /// スパイダー（幅 width の腕 arms 本）
    Spider {
        width: f64,
        arms: u32,
        #[serde(default, skip_serializing_if = "zero")]
        dx: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dy: f64,
    },
}

/// 面の後の媒質
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Medium {
    /// ガラスの名前（カタログから引く。"AIR" は空気）
    Glass(String),
    /// モデルガラス・GRIN
    Data(MediumData),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MediumData {
    /// モデルガラス（nd・vd・ΔPgF）。vd = 0 は分散なし（屈折率 nd）
    Model {
        nd: f64,
        #[serde(default, skip_serializing_if = "zero")]
        vd: f64,
        #[serde(default, skip_serializing_if = "zero")]
        dpgf: f64,
    },
    /// 屈折率分布（Gradium・Gradient 9）
    Grin(Box<Grin>),
}

/// 屈折率分布の媒質
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Grin {
    /// 材料の名前
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub profile: GrinKind,
    /// OpticStudio の刻み Δt
    #[serde(default, skip_serializing_if = "zero")]
    pub dt: f64,
}
