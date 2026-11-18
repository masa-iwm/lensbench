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
    /// 目標（既定 1）
    pub target: Option<f64>,
}

/// paraxial の項
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParaxSpec {
    /// 量（efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obsn・isna、OpticStudio の名前も可）
    pub q: String,
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
    /// 量（x・y・z・l・m・n）
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
    Zplm(ZplmSpec),
    Plugin(PluginSpec),
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
