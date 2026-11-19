//! 評価関数の項の JSON の型。`type` で種類を選び、種類ごとに使えるキーだけを受ける（知らないキーは誤り）。
//! 共通のキー `weight`・`config`・`min`・`max` は [`crate::merit::term_from_json`] が先に外し、ここには種類の
//! キーだけが来る。番号・面・目標の意味は [`crate::merit`] の表。`docs/schema/terms.schema.json` はこの型から作る。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// 視野: 番号（1 始まり）か正規化視野 [hx, hy]
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
#[serde(untagged)]
pub enum FieldRef {
    /// 視野の番号（1 始まり）
    Index(usize),
    /// 正規化視野 [hx, hy]
    Norm([f64; 2]),
}

/// RMS の基準
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum RefName {
    /// 重心（既定）
    #[default]
    Centroid,
    /// 主光線
    Chief,
}

/// スポット・波面の瞳の求積の点の置き方
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum QuadName {
    /// 腕 4 rings + 2 本（既定）
    #[default]
    Lensbench,
    /// OpticStudio の RSCE・RSCH・RWCE・RWCH と同じ（腕 2 rings 本を 90° 回して置く）。arms を与えると腕 arms 本を
    /// 0° から等間隔に（評価関数の Wizard のガウス求積と同じ）
    Opticstudio,
    /// 瞳を n × n の升に分けた中心の点を等しい重みで（評価関数の Wizard の Rectangular Array と同じ。rings は使わない）
    Grid,
}

/// スポットの項が測る向き
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AxisName {
    /// x と y（スポットの半径、既定）
    #[default]
    Xy,
    X,
    Y,
}

/// MTF の向き
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum DirName {
    /// タンジェンシャル
    Tan,
    /// サジタル
    Sag,
    /// 両方（既定）
    #[default]
    Both,
    /// 子午と球欠の平均（mtf だけ、残差は 1 つ）
    Avg,
}

/// MTF の求め方
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum MtfKindName {
    /// 回折 MTF（既定）
    #[default]
    Diffraction,
    /// 矩形波の縞の応答（回折 MTF の奇数倍の周波数の級数）
    Square,
    /// 幾何 MTF
    Geometric,
    /// 像面の上の Huygens PSF の MTF
    Huygens,
}

/// 重心の向き
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Xy {
    X,
    Y,
}

/// Zernike の並び
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum ZernName {
    /// Fringe（既定）
    #[default]
    Fringe,
    /// Standard
    Standard,
}

/// zplm のマクロ: 番号 nn（ZPLnn.ZPL）か名前
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum MacroRef {
    /// 番号（0〜99）
    No(u64),
    /// 名前
    Name(String),
}

/// 数か数の並び
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum Nums {
    One(f64),
    Many(Vec<f64>),
}

/// spot・wavefront の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RmsSpec {
    pub field: FieldRef,
    /// 波長の番号（1 始まり）。null・省略は多色
    pub wave: Option<usize>,
    /// 瞳の環の数（既定 3）
    pub rings: Option<usize>,
    pub reference: Option<RefName>,
    /// 瞳の求積の点の置き方（既定 lensbench。.zmx から読んだ行は opticstudio）
    pub quadrature: Option<QuadName>,
    /// quadrature opticstudio の腕の数（省略は 2 rings 本を 90° 回して）
    pub arms: Option<usize>,
    /// quadrature grid の 1 辺の升の数
    pub n: Option<usize>,
    /// spot の測る向き（既定 xy）
    pub axis: Option<AxisName>,
}

/// mtf の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct MtfSpec {
    pub field: FieldRef,
    pub wave: Option<usize>,
    /// 瞳の標本数
    pub n: Option<usize>,
    /// 周波数 [cycles/mm]
    pub freq: f64,
    pub dir: Option<DirName>,
    /// 求め方（既定 diffraction）
    pub kind: Option<MtfKindName>,
    /// 幾何 MTF に回折限界の MTF をかけるか（kind が geometric のときだけ。既定 true）
    pub diff_limit: Option<bool>,
    /// 目標（既定 1）
    pub target: Option<f64>,
}

/// paraxial の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParaxSpec {
    /// 量（efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obsn・isna・amag・linv・petc・petz、OpticStudio の名前も可）
    pub q: String,
    /// 波長の番号（efl・pimh はその波長を主波長にした系の値、amag・linv・petc・petz はその波長の近軸光線）。null・省略は主波長
    pub wave: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// ray の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RaySpec {
    pub field: FieldRef,
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 正規化瞳座標（既定 0）
    pub px: Option<f64>,
    pub py: Option<f64>,
    /// 面の番号（物体 0、既定は像面）
    pub surf: Option<usize>,
    /// true で近軸光線（x・y を別々に近軸で追う。q は x・y・z・l・m・n・r・nl・nm・nn・tanx・tany、OpticStudio の PARX〜PATY）
    pub paraxial: Option<bool>,
    /// 面 surf から to まで（q は opth で面の間の光路長、aoi・aoe は min・max と面ごとに比べてはみ出しを足す）
    pub to: Option<usize>,
    /// 量（x・y・z・l・m・n。trax・tray・trar は主波長の主光線からのずれ、opth は光路長。gx〜gn はグローバル座標の交点と方向、
    /// r は交点の半径、ang は z 軸との角 [rad]、tanx・tany は l/n・m/n、nl〜nn は面の法線、aoi・aoe は入射角・射出角 [度]、
    /// cosi・cose はその cos）
    pub q: String,
    pub target: Option<f64>,
}

/// centroid の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CentroidSpec {
    pub field: FieldRef,
    pub wave: Option<usize>,
    pub rings: Option<usize>,
    pub q: Xy,
    /// 目標 [レンズ単位]
    pub target: Option<f64>,
}

/// opd の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct OpdSpec {
    pub field: FieldRef,
    pub wave: Option<usize>,
    pub px: Option<f64>,
    pub py: Option<f64>,
    /// 目標 [波]
    pub target: Option<f64>,
}

/// zernike の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ZernSpec {
    pub field: FieldRef,
    pub wave: Option<usize>,
    /// 項の番号（1 始まり）
    pub term: usize,
    pub kind: Option<ZernName>,
    /// 当てはめる項の数（term 以上。既定は term と 37 の大きい方。OpticStudio の ZERN は Fringe が 37、Standard が term）
    pub fit: Option<usize>,
    /// 格子（既定 64）
    pub n: Option<usize>,
    /// 目標 [波]
    pub target: Option<f64>,
}

/// thickness の項（中心の厚み。to を書くと surf から to までの和）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ThickSpec {
    /// 面の番号（物体面 0）
    pub surf: usize,
    /// 和をとる最後の面（既定 surf）
    pub to: Option<usize>,
    /// 目標 [レンズ単位]
    pub target: Option<f64>,
    /// 縁の厚み（面とその次の面の矢高を、それぞれの半直径 × zone の高さで測る）
    pub edge: Option<bool>,
    /// 縁の厚みを測る高さ（半直径に対する割合、既定 1）
    pub zone: Option<f64>,
    /// 和でなく面ごとに min・max と比べ、はみ出しを足す（その媒質の面だけ。OpticStudio の MNCT・MNCA・MNEG など）
    pub each: Option<EachName>,
}

/// surface の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SurfSpec {
    /// 面の番号（物体面 0）
    pub surf: usize,
    /// 量（curv・conic・sag・sagx・sagy・thick・total・diam・mech_diam・gx・gy・gz・ga・gb・gc・power・ynip・index・nd・vd・
    /// dpgf・cost・tce・semi・param）
    pub q: String,
    /// 面タイプのパラメータの番号（.zmx の PARM の番号、param だけ）
    pub param: Option<u8>,
    /// 範囲の最後の面（semi・nd・vd・dpgf・cost・tce だけ）。書けば面 surf から to までを面ごとに min か max と比べ、
    /// はみ出しを足す（ガラスの量はガラスの面だけ）
    pub to: Option<usize>,
    /// sag・thick を測る点 [レンズ単位]（既定 0）。total は測る向き（既定 +y）
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// 波長の番号（power・ynip・index だけ）。null・省略は主波長
    pub wave: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// volume の項（面 surf から to までの要素の体積 [cm³] か質量 [g]）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct VolumeSpec {
    /// 最初の面の番号（面 i の要素は面 i と i + 1 の間、空気の間も含む）
    pub surf: usize,
    /// 最後の面（既定 surf）
    pub to: Option<usize>,
    /// 硝材の要素だけの質量 [g]（密度はカタログ）。既定は体積
    pub mass: Option<bool>,
    /// 外径をクリア半径でとる。既定は玉の半径（mech_diam の半分）
    pub clear: Option<bool>,
    /// 要素の形でなく、面 surf から to + 1 を包む最小の円柱の体積 [cm³]（長さは頂点の間の厚みの和、半径は面の半径の最大。
    /// 矢高は入れない）。mass とは使わない
    pub cylinder: Option<bool>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// cardinal の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CardSpec {
    /// 量（efl・f_object・f_image・focal_object・focal_image・principal_object・principal_image・anti_principal_object・
    /// anti_principal_image・nodal_object・nodal_image・anti_nodal_object・anti_nodal_image）。efl は空気換算の 1/パワー、
    /// f_* は媒質の中の焦点距離、位置は物体空間の点が面 from の頂点から、像空間の点が面 to の頂点から [レンズ単位]
    pub q: String,
    /// 面の範囲（既定は面 1 から像面の手前の面まで）
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// fnum の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FNumSpec {
    pub field: FieldRef,
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 向き（tan は瞳の y の縁、sag は x の縁の光線。既定 tan、both は書けない）
    pub dir: Option<DirName>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// seidel の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SeidelSpec {
    /// 量（spha・coma・asti・fcur・dist、波面収差の係数 W040・W131・W222・W220・W311 [波]）
    pub q: String,
    /// 面の番号（0・省略は全部の面の和）
    pub surf: Option<usize>,
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// distortion の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct DistortionSpec {
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// true で実像高 − 予測の像高 [レンズ単位]、false・省略で予測の像高に対する %
    pub absolute: Option<bool>,
    /// 視野（省略は正規化視野 [0, 1]）。peak では軸上からこの視野までの線の上の最大
    pub field: Option<FieldRef>,
    /// 基準の視野（省略は軸上で、予測は近軸主光線の倍率）。ほかの視野では、その視野の主光線の点と、そこでの像の点の
    /// 角の tan（物体高の視野では高さ）による微分から線形に予測する（OpticStudio の DISG）
    pub reference: Option<FieldRef>,
    /// true で軸上から field までの絶対値の最大（OpticStudio の DIMX）
    pub peak: Option<bool>,
    /// true で校正した f-θ 歪曲: y の向きの視野で、主光線の高さに原点を通る y = cθ を当てたずれの絶対値の最大
    /// （角度の視野だけ。OpticStudio の DISC）
    pub ftheta: Option<bool>,
    /// a・b・c・d で、基準の視野でのグリッド歪曲の係数 J（∂(x, y)/∂t の [[a, b], [c, d]]）を値にする（OpticStudio の ABCD）
    pub coef: Option<String>,
    /// 予測に使う [A, B, C, D]（基準の視野での J の代わり。OpticStudio の DISA）
    pub abcd: Option<[f64; 4]>,
    /// 歪曲の向き: radial（既定）・x・y（x・y は差の成分と、予測の変位の成分に対する %。OpticStudio の DISA の Data）
    pub component: Option<String>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// energy の項（包囲エネルギー）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnergySpec {
    pub field: FieldRef,
    /// 波長の番号。null・省略は重みのある全波長
    pub wave: Option<usize>,
    /// 光量の分布: geometric（既定、瞳の格子の光線）・fft（FFT の回折 PSF）・huygens（Huygens PSF）
    pub method: Option<String>,
    /// geometric の円で、円の割合にエアリー像の円の割合をかけるか（既定 false）
    pub airy: Option<bool>,
    /// 形: circle（既定、半径）・x・y（帯の半幅）・square（正方形の半幅）
    pub shape: Option<String>,
    /// 中心: chief（既定、主波長の主光線）・centroid（光量の重心）・vertex（像面の原点）・middle（点の広がりの中心）
    pub center: Option<String>,
    /// 値にする大きさの光量の割合（0〜1）。fraction_at と片方だけ
    pub size_at: Option<f64>,
    /// 値にする割合の形の大きさ [レンズ単位]。size_at と片方だけ
    pub fraction_at: Option<f64>,
    /// 瞳の格子（geometric は既定 256、fft・huygens は自動）
    pub n: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// strehl の項（ストレール比）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct StrehlSpec {
    pub field: FieldRef,
    /// 波長の番号。null・省略は重みのある全波長
    pub wave: Option<usize>,
    /// 瞳の格子（省略は自動）
    pub n: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// illum の項（像点の照度）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct IllumSpec {
    pub field: FieldRef,
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 量: relative（軸上の像点に対する相対照度）・fno（実効 F ナンバー）
    pub q: String,
    /// 瞳の縁の点の数（既定 128）
    pub n: Option<usize>,
    /// 目標（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// longitudinal の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LongSpec {
    /// 波長の番号。null・省略は主波長
    pub wave: Option<usize>,
    /// 差をとる波長の番号（値は wave2 の距離 − wave の距離、軸上色収差）
    pub wave2: Option<usize>,
    /// 瞳の帯 0〜1（0・省略は近軸周辺光線）
    pub zone: Option<f64>,
    /// 目標 [レンズ単位]（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// lateral_color の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LatColorSpec {
    /// 波長の番号（値は wave2 の高さ − wave の高さ）。wave の null・省略は主波長
    pub wave: Option<usize>,
    pub wave2: usize,
    /// 目標 [レンズ単位]（target・min・max のどれかが要る）
    pub target: Option<f64>,
}

/// semi の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SemiSpec {
    /// 面の範囲（to の既定は surf）
    pub surf: usize,
    pub to: Option<usize>,
    /// 目標（min か max が要る。面ごとに半直径と比べてはみ出しを足す）
    pub target: Option<f64>,
}

/// 厚みの項で面ごとに比べる面の媒質
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EachName {
    /// すべての面
    All,
    /// 空気の面（自分の媒質のない座標ブレーク・鏡も）
    Air,
    /// 空気でない媒質の面
    Glass,
}

/// coupling の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CouplingSpec {
    pub wave: Option<usize>,
    /// 送り・受けの NA（na_source・na_receiver が優先）
    pub na: Option<f64>,
    pub na_source: Option<f64>,
    pub na_receiver: Option<f64>,
    /// 送りのファイバの位置 [x, y]
    pub source: Option<[f64; 2]>,
    pub receiver: Option<[f64; 2]>,
    /// 格子（既定 101）
    pub grid: Option<usize>,
    pub na_max: Option<f64>,
    pub clip_stop: Option<bool>,
    /// 目標（既定 1）
    pub target: Option<f64>,
}

/// zplm の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ZplmSpec {
    /// マクロの番号か名前（file とどちらか一方）
    #[serde(rename = "macro")]
    pub macro_: Option<MacroRef>,
    /// .ZPL のパス
    pub file: Option<String>,
    /// OPTRETURN の番号（既定 0）
    pub data: Option<usize>,
    /// PVHX・PVHY・PVPX・PVPY（既定 0）
    pub hx: Option<f64>,
    pub hy: Option<f64>,
    pub px: Option<f64>,
    pub py: Option<f64>,
    /// マクロを探すフォルダの並び
    pub macro_dirs: Option<Vec<String>>,
    /// false なら既定の場所（LENSBENCH_MACROS・今のフォルダ）を探さない
    pub default_dirs: Option<bool>,
    pub target: Option<f64>,
}

/// plugin の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct PluginSpec {
    /// 登録の名前
    pub name: String,
    /// 設定 {"名前": 値}
    pub args: Option<Value>,
    /// 入力 {"名前": 数か数の並び}
    pub inputs: Option<Value>,
    /// inputs の field の略
    pub field: Option<Value>,
    /// 出力の名前
    pub output: String,
    /// その要素だけ
    pub index: Option<usize>,
    /// 目標（数か出力と同じ数の並び、既定 0）
    pub target: Option<Nums>,
}

/// 評価関数の 1 つの項（共通のキー weight・config・min・max を除いた形）
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TermSpec {
    Spot(RmsSpec),
    Wavefront(RmsSpec),
    Mtf(MtfSpec),
    Paraxial(ParaxSpec),
    Ray(RaySpec),
    Centroid(CentroidSpec),
    Opd(OpdSpec),
    Zernike(ZernSpec),
    Coupling(CouplingSpec),
    Thickness(ThickSpec),
    Surface(SurfSpec),
    Volume(VolumeSpec),
    Cardinal(CardSpec),
    Fnum(FNumSpec),
    Seidel(SeidelSpec),
    Distortion(DistortionSpec),
    Energy(EnergySpec),
    Strehl(StrehlSpec),
    Illum(IllumSpec),
    Longitudinal(LongSpec),
    #[serde(rename = "lateral_color")]
    LateralColor(LatColorSpec),
    Semi(SemiSpec),
    Zplm(ZplmSpec),
    Plugin(PluginSpec),
    Calc(CalcSpec),
}

/// calc の項: 引数（数か、1 つの値の項。項には weight・config・min・max を書かない）の op − target
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CalcSpec {
    pub op: CalcName,
    pub args: Vec<CalcArgSpec>,
    /// 目標（既定 0）
    pub target: Option<f64>,
}

/// calc の演算（OpticStudio の SUMM・OSUM は sum、DIFF は diff、PROD は prod、DIVI は div、ABSO は abs、SQRT は sqrt（√|a|）、RECI は reci、MAXX・MINN は max・min、QSUM は qsum（√Σa²）、SINE〜ATAN は sin〜atan（ラジアン））
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum CalcName {
    Sum,
    Diff,
    Prod,
    Div,
    Abs,
    Sqrt,
    Reci,
    Max,
    Min,
    Qsum,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
}

/// calc の引数
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum CalcArgSpec {
    Num(f64),
    Term(Box<TermSpec>),
}

/// 共通のキー（`terms.schema.json` の各種類に足す）
pub const COMMON: [(&str, &str); 4] = [
    ("weight", "重み（既定 1）"),
    ("config", "構成の番号（1 始まり、既定 1）"),
    (
        "min",
        "値 ≥ min なら残差 0（target の代わり、目標が 1 つの値の項だけ）",
    ),
    (
        "max",
        "値 ≤ max なら残差 0（target の代わり、目標が 1 つの値の項だけ）",
    ),
];

/// 項の並び（配列か 1 つの項）のスキーマ。共通のキーを各種類に足す
pub fn schema() -> Value {
    let mut v = lensbench_project::schema::gen::<TermSpec>();
    let props: serde_json::Map<String, Value> = COMMON
        .iter()
        .map(|(k, d)| {
            let ty = if *k == "config" { "integer" } else { "number" };
            (
                k.to_string(),
                serde_json::json!({"description": d, "type": [ty, "null"]}),
            )
        })
        .collect();
    if let Some(Value::Array(vs)) = v.get_mut("oneOf") {
        for x in vs {
            if let Some(Value::Object(p)) = x.get_mut("properties") {
                p.extend(props.clone());
            }
        }
    }
    let one = v.as_object_mut().expect("スキーマはオブジェクト");
    let schema_key = one.remove("$schema");
    let defs = one.remove("$defs");
    let body = Value::Object(std::mem::take(one));
    let r = serde_json::json!({"$ref": "#/$defs/TermSpec"});
    let mut out = serde_json::json!({"oneOf": [r, {"type": "array", "items": r}]});
    let mut defs = defs.unwrap_or_else(|| serde_json::json!({}));
    defs["TermSpec"] = body;
    out["$defs"] = defs;
    if let Some(s) = schema_key {
        out["$schema"] = s;
    }
    out
}
