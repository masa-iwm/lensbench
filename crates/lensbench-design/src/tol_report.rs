//! モンテカルロ公差の結果（[`crate::tolerance::report`]）。CLI・serve はこの型を JSON にし、GUI は同じ型で読む
//! （鍵の綴りをコンパイラが見る）。有限でない値は JSON の null（読むときは None か NaN）
use serde::{Deserialize, Serialize};

use crate::montecarlo::Eval;

/// 公差の結果
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToleranceReport {
    /// 公称の評価
    pub nominal: Option<Eval>,
    /// 試行の数と、追跡できなかった試行の数
    pub trials: usize,
    pub failed: usize,
    /// 追跡できなかった理由（違うものを 5 つまで）
    pub errors: Vec<String>,
    /// 途中で止めた（終えた試行だけの結果）
    pub cancelled: bool,
    pub seed: u64,
    pub tolerances: Vec<TolInfo>,
    /// 評価関数の統計と、有限の値を小さい順に並べたもの
    pub merit: StatsInfo,
    pub cdf: Vec<f64>,
    /// 項ごとの統計
    pub terms: Vec<TermStats>,
    /// 補償子ごとの値
    pub compensators: Vec<CompStats>,
    /// 傾きの x・y の組の大きさ [分]
    pub tilt_magnitudes: Vec<TiltStats>,
    /// 試行ごとの評価関数（試行の順、追跡できなかった試行は None）
    pub merit_values: Vec<Option<f64>>,
    /// 感度（評価関数の変化の悪い順）
    pub sensitivity: Vec<SensInfo>,
    /// 感度からの見積り（公称 + √Σ 変化²。変化² は公差ごとに lo 側と hi 側の平均）
    pub rss_estimate: Option<f64>,
    /// 試行の評価関数とずれの相関（大きい順）
    pub correlation: Vec<CorrInfo>,
    /// 補償のしかた（"optimize"・"paraxial"・"none"）
    #[serde(default = "optimize_name")]
    pub comp_mode: String,
    /// 逆感度の結果（逆感度をしたときだけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<InverseInfo>,
    /// 試行ごとのずれと評価（samples を頼んだときだけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<Vec<Sample>>,
    /// 読み込んだ後の変更を当てなかった説明（serve だけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<String>>,
}

fn optimize_name() -> String {
    "optimize".into()
}

/// 逆感度の結果
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InverseInfo {
    /// "limit"（評価関数の上限）か "increment"（公称からの変化の上限）
    pub mode: String,
    /// 設定の値（limit なら評価関数の値、increment なら変化）
    pub value: f64,
    /// 評価関数の上限（limit はその値、increment は公称 + 変化）
    pub limit: f64,
    /// 公差ごとの範囲（公差の順）。締めた公差の数
    pub tolerances: Vec<InverseTolInfo>,
    pub tightened: usize,
}

/// 逆感度で求めた公差 1 つの範囲
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InverseTolInfo {
    pub label: String,
    /// 元の範囲と締めた後の範囲（締めなかった側は元のまま）
    pub lo0: f64,
    pub hi0: f64,
    pub lo: f64,
    pub hi: f64,
    /// 締めた側（lo・hi）
    pub tightened_lo: bool,
    pub tightened_hi: bool,
    /// 締めない公差（skip）
    pub skipped: bool,
}

/// 公差 1 つの設定
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TolInfo {
    pub label: String,
    /// 同じずれを与えるパラメータの名前
    pub params: Vec<String>,
    pub lo: f64,
    pub hi: f64,
    pub relative: bool,
    /// 分布の名前
    pub dist: String,
    /// パラメータごとにずれに掛ける数（None はすべて 1）
    pub scale: Option<Vec<f64>>,
}

/// 値の統計（有限の値だけ）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsInfo {
    pub n: usize,
    pub mean: Option<f64>,
    pub std: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// (百分位, 値)
    pub percentiles: Vec<(f64, Option<f64>)>,
}

/// 評価関数の項 1 つの統計
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TermStats {
    /// 項の名前（"spot #1"・"wavefront #2 (config 1)" など）
    pub label: String,
    pub nominal: Option<f64>,
    pub stats: StatsInfo,
    pub cdf: Vec<f64>,
}

/// 補償子 1 つの値
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompStats {
    pub name: String,
    /// 効く構成（1 始まり、None はすべて）
    pub configs: Option<Vec<usize>>,
    pub nominal: Option<f64>,
    pub stats: StatsInfo,
    /// 試行ごとの値（試行の順、追跡できなかった試行は None）
    pub values: Vec<Option<f64>>,
}

/// 傾きの組の大きさ
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TiltStats {
    /// 「M.tilt」の名前と、組の x・y の補償子の名前
    pub name: String,
    pub x: String,
    pub y: String,
    /// 単位（arcmin）
    pub unit: String,
    pub nominal: Option<f64>,
    pub stats: StatsInfo,
    pub values: Vec<Option<f64>>,
}

/// 公差 1 つの感度
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SensInfo {
    pub label: String,
    pub lo: f64,
    pub hi: f64,
    /// lo・hi にしたときの評価関数と、公称からの変化（追跡できなければ None）
    pub merit_lo: Option<f64>,
    pub merit_hi: Option<f64>,
    pub change_lo: Option<f64>,
    pub change_hi: Option<f64>,
    /// 変化の大きい方
    pub worst: Option<f64>,
    /// lo・hi にしたときの補償子の値（補償子の順。補償子が無い・追跡できなければ省く）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp_lo: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp_hi: Option<Vec<f64>>,
}

/// 公差 1 つの相関
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CorrInfo {
    pub label: String,
    /// ずれ・|ずれ| と評価関数の相関
    pub r: Option<f64>,
    pub r_abs: Option<f64>,
}

/// 試行 1 回のずれと評価
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub deltas: Vec<f64>,
    pub eval: Option<Eval>,
    pub error: Option<String>,
}

/// 有限なら Some
pub(crate) fn fin(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}
