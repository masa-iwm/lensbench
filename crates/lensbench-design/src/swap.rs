//! カタログレンズの差し替え探索: プロジェクトの玉のブロックを、焦点距離・径で絞ったカタログの候補で
//! 差し替えた組を並列に回し、組ごとに間隔（空気・群の移動）を減衰最小二乗（[`lensbench_optim`]）で
//! 最適化して、評価関数の小さい順に並べる。
//!
//! - 差し替える所（[`Slot`]）: 玉のブロックの id と、候補（型番の並びか、メーカー・焦点距離・径の範囲）。
//!   `flip` なら向き（reversed）を両方試す。
//! - 変数（[`SwapSpec::vars`]）: 展開した系のパラメータの書き方（`G2.shift` など、[`System::param`]）に加えて、
//!   id のある空気のブロックを `id.t` で指せる（その空気が入る面の厚さ。[`lensbench_project::Expansion::gaps`]）。範囲は
//!   `d1.t=0.5:40`。
//! - 組の番号 i（0 から [`SwapSearch::count`] 未満）は、差し替える所ごとの候補の番号を並べた混合基数
//!   （最後の所が最も速く回る）。[`SwapSearch::eval`] は組 i だけから決まる JSON を返す。
//!   [`SwapSearch::run_to`] は一括実行（[`lensbench_runs`]）で組ごとに JSONL へ書き出し、途中から再開できる。
//!
//! 評価関数は既定の像の項（全視野・多色の RMS スポットか RMS 波面）。[`SwapSpec::efl`] で系の焦点距離の目標も足せる
//! （そのときの `merit` は目標の項を含む。像だけの値は `image_merit`）。
//!
//! 候補の焦点距離はカタログの値（無ければ玉の .zmx の主波長の近軸の値）、径は面の半径の最大の 2 倍。

use lensbench_core::catalog::{LensCatalog, VendorKey};
use lensbench_core::elements::{self, Kind as EKind};
use lensbench_core::param::{Kind, Param};
use lensbench_core::sweep::RmsRef;
use lensbench_core::sync::LockExt;
use lensbench_core::{par, System};
use lensbench_formats::zmx;
use lensbench_optim::{
    DefaultMerit, DlsOpts, ParaxQ, Problem, RayQ, Side, Stop, Term, TermKind, Var,
};
use lensbench_project::{
    block_pointers, expand, find_block_mut, Block, Env, ExpandOpts, Gap, LensSource, Project,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// 候補の絞り込み（省いた条件は絞らない）
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// メーカー（大文字・小文字・空白・記号を無視して、カタログのメーカー名に含まれれば合う）
    #[serde(default)]
    pub vendor: Option<String>,
    /// 焦点距離の範囲 [mm]
    #[serde(default)]
    pub efl: [Option<f64>; 2],
    /// 径の範囲 [mm]
    #[serde(default)]
    pub diameter: [Option<f64>; 2],
}

/// 型番 1 つ
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartRef {
    pub vendor: String,
    pub part: String,
}

/// 差し替える所
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    /// 玉のブロック（type lens）の id
    pub block: String,
    /// 候補の型番（空なら filter で絞ったカタログのすべて）
    #[serde(default)]
    pub parts: Vec<PartRef>,
    #[serde(default)]
    pub filter: Filter,
    /// 向き（reversed）を両方試す（false ならブロックの向きのまま）
    #[serde(default)]
    pub flip: bool,
}

/// 差し替え探索の設定（CLI の --spec の JSON）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SwapSpec {
    pub slots: Vec<Slot>,
    /// 組ごとに最適化する変数（"d1.t"・"d1.t=0.5:40"・"G2.shift"、または `{"name": "d1.t", "min": 0.5, "max": 40}`）
    #[serde(default)]
    pub vars: Vec<VarSpec>,
    /// 評価関数
    #[serde(default)]
    pub merit: SwapMerit,
    /// 基準
    #[serde(default)]
    pub reference: SwapRef,
    #[serde(default = "three")]
    pub rings: usize,
    /// 組ごとの反復の上限
    #[serde(default = "thirty")]
    pub iters: usize,
    /// 系の焦点距離の目標（省けば目標なし）。重みは像の項の重みの合計の 10 倍（`TermKind::Paraxial`）
    #[serde(default)]
    pub efl: Option<f64>,
    /// 状態（None なら変更なし）
    #[serde(default)]
    pub state: Option<String>,
    /// 組の数の上限（大きすぎる探索を誤って始めないため。ビーム探索では評価する組の数の見込み）
    #[serde(default = "max_count")]
    pub max_count: usize,
    /// 評価関数に足す項（`optimize --term` と同じ JSON。片側の条件 `min`・`max` も）。
    /// 例: `{"type": "paraxial", "q": "efl", "min": 351}`
    #[serde(default)]
    pub terms: Vec<Value>,
    /// 出発点（変数の名前 → 初期値。書かない変数はプロジェクトの値）。組ごとにすべての出発点から最適化し、
    /// 評価関数の最も小さい解を採る（空なら プロジェクトの値から 1 回）
    #[serde(default)]
    pub starts: Vec<std::collections::BTreeMap<String, f64>>,
    /// 状態をまたいだ探索（ズーム位置など）: プロジェクトの状態の名前の並び。組ごとに全状態を同時に
    /// 最適化する（評価関数は全状態の像の項）。`state` とは同時に使えない。`terms` の `config` は
    /// この並びの番号（1 始まり）
    #[serde(default)]
    pub states: Vec<String>,
    /// 状態ごとに別の値で解く変数（`states` のとき。群の移動 `G2.shift` など、書き方は `vars` と同じ）。
    /// `vars` は全状態で共通の 1 つの値
    #[serde(default)]
    pub state_vars: Vec<VarSpec>,
    /// 有効径の条件: 差し替える玉の各面で、各視野の瞳の縁の実光線（主波長、瞳の上下左右）が面の半径
    /// （カタログの玉の半直径）の内側にあることを片側の条件の項として足す（重みは像の項の重みの合計の 1e6 倍）
    #[serde(default)]
    pub clear_aperture: bool,
    /// ヤコビアン: "forward"（すべて前進差分、既定）・"fd"（すべて中心差分）・"auto"（解析的に求められる列が 16 個以上の項は解析的に、少ない項は中心差分）。
    /// 差し替え探索の変数（間隔・群の移動など少数）では差分の方が速い（実カタログ 7 万組で fd は auto の 4.3 倍）。
    /// forward は差分の追跡が中心差分の半分で済み、fd よりさらに約 1.4 倍速い（実カタログ 3,000 組で上位 300 組の顔ぶれ・順位は
    /// fd と同じ、評価関数の差は相対 3e-7 以下）
    #[serde(default)]
    pub jac: SwapJac,
    /// 粗いふるい（省けばなし）: 全組を光線の少ない評価関数で最適化し、評価関数の小さい組だけを今の評価で
    /// 最適化し直す。残した組の結果はふるいなしと同じ。残さなかった組は結果に出ない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prescreen: Option<Prescreen>,
    /// 所ごとのビーム探索（省けば全組の総当たり）: 全組でなく、所を 1 つずつ候補を全部試して評価関数の小さい
    /// `width` 通りを残しながら進む。評価する組の数は全組の積でなく、およそ `width` × 候補の数の和 × 巡回の数。
    /// 総当たりの最良を見落とすことがある。`prescreen` とは同時に使えない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam: Option<Beam>,
    /// 近軸の前ふるい（省けばなし、`efl` があるときだけ）: 変数（範囲つきの厚さだけ）の範囲の角で系の屈折力を求め、
    /// `efl` の目標に届かない組を光線を追わずに落とす（その組は誤り「…misses the target…」の結果になる）。
    /// 屈折力は厚さのそれぞれについて 1 次なので、範囲の中の最大・最小は角にあり、落とした組はどの値でも目標に届かない。
    /// ピックアップ・ソルブ・マクロのある状態や屈折率分布の媒質の厚さを動かす状態は 1 次とは限らないので確かめない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paraxial: Option<Paraxial>,
}

/// 評価関数（[`SwapSpec::merit`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SwapMerit {
    /// スポットの RMS
    #[default]
    Spot,
    /// 波面の RMS
    Wavefront,
}

/// RMS の基準（[`SwapSpec::reference`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SwapRef {
    /// 重心
    #[default]
    Centroid,
    /// 主光線
    Chief,
}

/// ヤコビアンの求め方（[`SwapSpec::jac`]、`optimize --jac` と同じ名前）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SwapJac {
    /// すべて前進差分
    #[default]
    Forward,
    /// すべて中心差分
    Fd,
    /// 解析的に求められる列が多い項は解析的に
    Auto,
}

impl SwapMerit {
    fn core(self) -> DefaultMerit {
        match self {
            SwapMerit::Spot => DefaultMerit::Spot,
            SwapMerit::Wavefront => DefaultMerit::Wavefront,
        }
    }
}

impl SwapRef {
    fn core(self) -> RmsRef {
        match self {
            SwapRef::Centroid => RmsRef::Centroid,
            SwapRef::Chief => RmsRef::Chief,
        }
    }
}

impl SwapJac {
    fn core(self) -> lensbench_optim::JacMode {
        match self {
            SwapJac::Forward => lensbench_optim::JacMode::Forward,
            SwapJac::Fd => lensbench_optim::JacMode::FiniteDiff,
            SwapJac::Auto => lensbench_optim::JacMode::Auto,
        }
    }
}

/// 変数の範囲（下限, 上限）
type Range = (f64, f64);

/// 差し替え探索の変数: 文字の短い形 "d1.t"・"d1.t=下限:上限"（片側は空でよい）か、オブジェクトの形
/// `{"name": "d1.t", "min": 0.5, "max": 40}`（min・max は省ける）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum VarSpec {
    Short(String),
    Full(VarRange),
}

/// 変数のオブジェクトの形（[`VarSpec::Full`]）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VarRange {
    pub name: String,
    /// 下限（省けば無し）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// 上限（省けば無し）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl From<&str> for VarSpec {
    fn from(s: &str) -> Self {
        VarSpec::Short(s.to_string())
    }
}

impl From<String> for VarSpec {
    fn from(s: String) -> Self {
        VarSpec::Short(s)
    }
}

impl VarSpec {
    /// (名前, 範囲)。範囲を書かなければ None
    pub fn parts(&self) -> lensbench_core::Result<(&str, Option<Range>)> {
        match self {
            VarSpec::Short(s) => parse_var(s),
            VarSpec::Full(VarRange { name, min, max }) => {
                let range = (min.is_some() || max.is_some()).then(|| {
                    (
                        min.unwrap_or(f64::NEG_INFINITY),
                        max.unwrap_or(f64::INFINITY),
                    )
                });
                Ok((name.trim(), range))
            }
        }
    }

    /// 名前（書き方の誤りがあれば書いたまま）
    pub fn name(&self) -> &str {
        match self {
            VarSpec::Short(s) => parse_var(s).map_or(s.trim(), |x| x.0),
            VarSpec::Full(VarRange { name, .. }) => name.trim(),
        }
    }
}

/// 変数の書き方 "d1.t" か "d1.t=下限:上限"（片側は空でよい）を (名前, 範囲) に分ける
fn parse_var(v: &str) -> lensbench_core::Result<(&str, Option<Range>)> {
    let Some((name, r)) = v.split_once('=') else {
        return Ok((v.trim(), None));
    };
    let (lo, hi) = r
        .split_once(':')
        .ok_or_else(|| format!("the range of variable {v} is min:max"))?;
    let num = |t: &str, d: f64| -> lensbench_core::Result<f64> {
        match t.trim() {
            "" => Ok(d),
            t => t
                .parse()
                .map_err(|_| format!("range {t} of variable {v}").into()),
        }
    };
    let range = (num(lo, f64::NEG_INFINITY)?, num(hi, f64::INFINITY)?);
    Ok((name.trim(), Some(range)))
}

/// 粗いふるいの設定（[`SwapSpec::prescreen`]）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Prescreen {
    /// 一段目の評価関数のリング数（`rings` より小さく。実カタログ 5.6 万組で、リング 3 に対して 2 なら上位 1000 組が
    /// 同じ顔ぶれ、1 なら上位 10 組の最悪が 438 位）
    #[serde(default = "two")]
    pub rings: usize,
    /// 二段目に回す組の数（一段目の評価関数の小さい順。評価できなかった組は回さない）
    #[serde(default = "thousand")]
    pub keep: usize,
}

#[path = "swap_paraxial.rs"]
mod paraxial;
pub use paraxial::Paraxial;
#[path = "swap_beam.rs"]
mod beam;
pub use beam::Beam;
#[path = "swap_shape.rs"]
mod shape;
use shape::same_shapes;

fn three() -> usize {
    3
}
fn two() -> usize {
    2
}
fn thousand() -> usize {
    1000
}
fn thirty() -> usize {
    30
}
fn max_count() -> usize {
    100_000
}

impl SwapSpec {
    pub fn from_json(txt: &str) -> lensbench_core::Result<SwapSpec> {
        let r = match lensbench_project::schema::strip_schema_key(txt) {
            Some(v) => serde_json::from_value(v),
            None => serde_json::from_str(txt),
        };
        r.map_err(|e| format!("swap search settings: {e}").into())
    }
}

/// 候補のレンズ
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub vendor: String,
    pub part: String,
    pub efl: f64,
    pub diameter: f64,
    /// 中心厚の合計
    pub thickness: f64,
}

/// カタログのレンズの要約（焦点距離・径・中心厚）。efl_cat が有限ならそれを焦点距離にする
pub fn lens_info(
    zmx_text: &str,
    efl_cat: f64,
    glass_dirs: &[PathBuf],
) -> lensbench_core::Result<(f64, f64, f64)> {
    let s = lensbench_project::catalog_system(&zmx::parse_text(zmx_text)?, glass_dirs, None)?;
    let els = elements::split(&s)?;
    let e = els
        .iter()
        .find(|e| e.kind == EKind::Lens)
        .ok_or("no lenses")?;
    let efl = if efl_cat.is_finite() {
        efl_cat
    } else {
        e.efl(s.primary)
    };
    let semi = e.surfaces.iter().map(|q| q.semi).fold(0.0, f64::max);
    Ok((efl, 2.0 * semi, e.length()))
}

fn in_range(x: f64, r: &[Option<f64>; 2]) -> bool {
    if r[0].is_none() && r[1].is_none() {
        return true;
    }
    x.is_finite() && r[0].is_none_or(|a| x >= a) && r[1].is_none_or(|b| x <= b)
}

/// レンズの要約（焦点距離・径・中心厚）。作れないレンズは None
type Info = Option<(f64, f64, f64)>;

/// メーカーの絞り込みに合うカタログのレンズの番号（カタログの順）
fn vendor_indices(cat: &LensCatalog, f: &Filter) -> Vec<usize> {
    let v = f.vendor.as_deref().map(VendorKey::new);
    cat.lenses()
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            v.as_ref()
                .is_none_or(|v| VendorKey::new(&l.vendor).matches(v))
        })
        .map(|(i, _)| i)
        .collect()
}

/// idx のレンズの要約を並列に作る（まだ infos に無いものだけ。スレッド数は外の指定に従う）
fn fill_infos(
    cat: &LensCatalog,
    idx: &[usize],
    glass_dirs: &[PathBuf],
    infos: &mut HashMap<usize, Info>,
) {
    let todo: Vec<usize> = idx
        .iter()
        .copied()
        .filter(|i| !infos.contains_key(i))
        .collect();
    let lenses = cat.lenses();
    let r = par::map(&todo, |&i| {
        let l = &lenses[i];
        lens_info(&l.zmx, l.efl, glass_dirs).ok()
    });
    infos.extend(todo.into_iter().zip(r));
}

/// 要約済みのレンズから、絞り込みに合うもの（idx の順）
fn filter_candidates(
    cat: &LensCatalog,
    f: &Filter,
    idx: &[usize],
    infos: &HashMap<usize, Info>,
) -> Vec<Candidate> {
    let lenses = cat.lenses();
    idx.iter()
        .filter_map(|i| {
            let (efl, diameter, thickness) = infos.get(i).copied().flatten()?;
            let l = &lenses[*i];
            (in_range(efl, &f.efl) && in_range(diameter, &f.diameter)).then(|| Candidate {
                vendor: l.vendor.clone(),
                part: l.part.clone(),
                efl,
                diameter,
                thickness,
            })
        })
        .collect()
}

/// 絞り込みに合うカタログのレンズ（カタログの順）。要約を作れないレンズは除く。
/// 要約は並列に作る（スレッド数は外の指定に従い、結果はスレッド数によらない）
pub fn candidates(cat: &LensCatalog, f: &Filter, glass_dirs: &[PathBuf]) -> Vec<Candidate> {
    let idx = vendor_indices(cat, f);
    let mut infos = HashMap::new();
    fill_infos(cat, &idx, glass_dirs, &mut infos);
    filter_candidates(cat, f, &idx, &infos)
}

/// 1 つの所の選び方
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub lens: Candidate,
    pub reversed: bool,
}

/// 差し替え探索
pub struct SwapSearch<'a> {
    /// 差し替える前のプロジェクト（状態 spec.state を当てたもの）
    base: Project,
    /// base の JSON の hash（結果のファイルの頭）
    base_hash: String,
    /// カタログの玉の使い回し（組ごとに .zmx を読み直さない）
    lenses: lensbench_project::SourceCache,
    dir: PathBuf,
    glass_dirs: Vec<PathBuf>,
    cat: &'a LensCatalog,
    spec: SwapSpec,
    /// 所ごとの選び方
    pub picks: Vec<Vec<Pick>>,
    /// 所ごとの選び方の代表: 置いた玉の面が同じ（型番違いの同じ玉、対称な玉の向き違い）選び方は最初の
    /// 選び方の番号。同じ形の組は 1 回だけ最適化し、結果を使い回す
    same: Vec<Vec<usize>>,
    /// 同じ形の組の最適化の結果（代表の選び方 → 結果）。仲間のいる代表の組だけ覚える
    fits: std::sync::Mutex<HashMap<Vec<Option<usize>>, SharedFit>>,
}

type SharedFit = std::sync::Arc<std::sync::OnceLock<lensbench_core::Result<SwapFit>>>;

impl<'a> SwapSearch<'a> {
    /// プロジェクト p（状態は spec.state）の玉を差し替える探索を作る。env.base・glass_dirs を使い、
    /// レンズは cat から引く（env.catalog は使わない）
    pub fn new(
        p: &Project,
        env: &Env,
        cat: &'a LensCatalog,
        spec: SwapSpec,
    ) -> lensbench_core::Result<SwapSearch<'a>> {
        if spec.state.is_some() && !spec.states.is_empty() {
            return Err("state and states can't be used together".into());
        }
        // 変数の書き方の誤りは組ごとの失敗にせず、始める前に弾く
        for v in spec.vars.iter().chain(&spec.state_vars) {
            v.parts()?;
        }
        if spec.states.is_empty() && !spec.state_vars.is_empty() {
            return Err("state_vars goes with states".into());
        }
        for n in &spec.states {
            p.with_state(n)?;
        }
        let p = match &spec.state {
            Some(n) => p.with_state(n)?,
            None => p.clone(),
        };
        let base = serde_json::to_value(&p).map_err(|e| e.to_string())?;
        if spec.slots.is_empty() {
            return Err("no place to swap".into());
        }
        if let Some(pr) = &spec.prescreen {
            if pr.rings == 0 || pr.keep == 0 {
                return Err("prescreen rings and keep must be 1 or more".into());
            }
        }
        paraxial::check(&spec)?;
        if let Some(b) = &spec.beam {
            if spec.prescreen.is_some() {
                return Err("beam and prescreen can't be used together".into());
            }
            if b.width == 0 || b.passes == 0 {
                return Err("beam width and passes must be 1 or more".into());
            }
        }
        // 候補の要約は所の間で使い回す（絞り込みの所が使うレンズをまとめて 1 回だけ、並列に作る）
        let slot_idx: Vec<Option<Vec<usize>>> = spec
            .slots
            .iter()
            .map(|sl| sl.parts.is_empty().then(|| vendor_indices(cat, &sl.filter)))
            .collect();
        let mut infos = HashMap::new();
        {
            let mut all: Vec<usize> = slot_idx.iter().flatten().flatten().copied().collect();
            all.sort_unstable();
            all.dedup();
            fill_infos(cat, &all, &env.glass_dirs, &mut infos);
        }
        let mut picks = Vec::new();
        for (k, sl) in spec.slots.iter().enumerate() {
            if spec.slots[..k].iter().any(|x| x.block == sl.block) {
                return Err(format!("place {} appears twice", sl.block).into());
            }
            let hits = block_pointers(&base["blocks"], &sl.block);
            let ptr = match hits.as_slice() {
                [x] => x.clone(),
                [] => return Err(format!("no block {}", sl.block).into()),
                _ => return Err(format!("two or more blocks with id {}", sl.block).into()),
            };
            let b = base
                .pointer(&ptr)
                .expect("the pointer comes from block_pointers on the same document");
            if b["type"] != "lens" {
                return Err(format!("block {} is not a lens (type lens)", sl.block).into());
            }
            let rev0 = b["reversed"].as_bool().unwrap_or(false);
            let lenses = if let Some(idx) = &slot_idx[k] {
                filter_candidates(cat, &sl.filter, idx, &infos)
            } else {
                sl.parts
                    .iter()
                    .map(|q| {
                        let l = cat.get(&q.vendor, &q.part).ok_or_else(|| {
                            format!("no {} {} in the lens catalogs", q.vendor, q.part)
                        })?;
                        let (efl, diameter, thickness) = lens_info(&l.zmx, l.efl, &env.glass_dirs)
                            .map_err(|e| e.at(format!("{} {}", q.vendor, q.part)))?;
                        Ok(Candidate {
                            vendor: l.vendor.clone(),
                            part: l.part.clone(),
                            efl,
                            diameter,
                            thickness,
                        })
                    })
                    .collect::<lensbench_core::Result<Vec<_>>>()?
            };
            if lenses.is_empty() {
                return Err(if cat.is_empty() {
                    format!(
                        "place {}: no candidates (0 items in the lens catalogs; give the maker folder or a folder with .zmx or ZMF)",
                        sl.block
                    ).into()
                } else {
                    format!(
                        "slot {}: no candidates (0 of the {} catalog items match the conditions)",
                        sl.block,
                        cat.len()
                    )
                    .into()
                });
            }
            let mut v = Vec::new();
            for l in lenses {
                v.push(Pick {
                    lens: l.clone(),
                    reversed: rev0,
                });
                if sl.flip {
                    v.push(Pick {
                        lens: l,
                        reversed: !rev0,
                    });
                }
            }
            picks.push(v);
        }
        let same = same_shapes(cat, &picks, &env.glass_dirs);
        let s = SwapSearch {
            base_hash: lensbench_project::sysfile::hash_value(&base),
            base: p,
            lenses: Default::default(),
            dir: env.base.clone(),
            glass_dirs: env.glass_dirs.clone(),
            cat,
            picks,
            same,
            fits: Default::default(),
            spec,
        };
        // ビーム探索なら評価する組の数の見込み（組の番号は全組の積で数えるので、積もあふれてはいけない）
        let n = match &s.spec.beam {
            Some(_) => s.count_checked().and_then(|_| s.beam_estimate()),
            None => s.count_checked(),
        };
        match n {
            Some(n) if n <= s.spec.max_count => Ok(s),
            _ => Err(format!(
                "too many combinations ({}, limit {}; raise it with max_count)",
                n.map_or("overflow".to_string(), |n| n.to_string()),
                s.spec.max_count
            )
            .into()),
        }
    }

    fn count_checked(&self) -> Option<usize> {
        self.picks
            .iter()
            .try_fold(1usize, |a, v| a.checked_mul(v.len()))
    }

    /// 組の数
    pub fn count(&self) -> usize {
        self.count_checked()
            .expect("the number of combinations fits in usize (count_checked reports overflow)")
    }

    /// 組 i の所ごとの選び方の番号（混合基数、最後の所が最も速く回る）
    pub fn combo(&self, i: usize) -> Vec<usize> {
        let mut r = i;
        let mut out = vec![0; self.picks.len()];
        for k in (0..self.picks.len()).rev() {
            let n = self.picks[k].len();
            out[k] = r % n;
            r /= n;
        }
        out
    }

    /// 組 i で差し替えたプロジェクト
    pub fn project(&self, i: usize) -> lensbench_core::Result<Project> {
        self.project_of(&self.assign(i))
    }

    /// 組 i の所ごとの選び方（[`Self::combo`] をすべて Some に）
    fn assign(&self, i: usize) -> Vec<Option<usize>> {
        self.combo(i).into_iter().map(Some).collect()
    }

    /// 所ごとの選び方 a（None の所はプロジェクトの玉のまま）で差し替えたプロジェクト
    fn project_of(&self, a: &[Option<usize>]) -> lensbench_core::Result<Project> {
        let mut p = self.base.clone();
        for (k, c) in a.iter().enumerate() {
            let Some(c) = *c else { continue };
            let pk = &self.picks[k][c];
            // new で id が 1 つだけの玉のブロックであることを確かめてある
            if let Some(Block::Lens {
                source, reversed, ..
            }) = find_block_mut(&mut p.blocks, &self.spec.slots[k].block)
            {
                *source = LensSource::Catalog {
                    vendor: pk.lens.vendor.clone(),
                    part: pk.lens.part.clone(),
                };
                *reversed = pk.reversed;
            }
        }
        Ok(p)
    }

    /// 組 i の系と、最適化の問題
    pub fn problem(&self, i: usize) -> lensbench_core::Result<(System, Problem)> {
        self.problem_n(i).map(|(s, p, _)| (s, p))
    }

    /// [`Self::problem`] と、像の項（既定の評価関数）の数
    fn problem_n(&self, i: usize) -> lensbench_core::Result<(System, Problem, usize)> {
        self.problem_of(&self.assign(i), self.spec.rings)
    }

    /// 選び方 a（[`Self::project_of`]）の [`Self::problem_n`] で、評価関数のリング数を rings にしたもの
    fn problem_of(
        &self,
        a: &[Option<usize>],
        rings: usize,
    ) -> lensbench_core::Result<(System, Problem, usize)> {
        let p = self.project_of(a)?;
        let look = |v: &str, q: &str| self.cat.zmx(v, q);
        let env = Env {
            step_files: None,
            base: self.dir.clone(),
            glass_dirs: self.glass_dirs.clone(),
            catalog: Some(&look),
        };
        // 状態ごとの系（states が空なら 1 つ）
        let mut systems = Vec::new();
        let mut gapss = Vec::new();
        if self.spec.states.is_empty() {
            let (s, g) = expand(
                &p,
                &env,
                &ExpandOpts {
                    cache: Some(&self.lenses),
                    ..Default::default()
                },
            )
            .map(|x| (x.system, x.gaps))?;
            systems.push(s);
            gapss.push(g);
        } else {
            for n in &self.spec.states {
                let (s, g) = expand(
                    &p.with_state(n)?,
                    &env,
                    &ExpandOpts {
                        cache: Some(&self.lenses),
                        ..Default::default()
                    },
                )
                .map(|x| (x.system, x.gaps))?;
                systems.push(s);
                gapss.push(g);
            }
        }
        let s = systems[0].clone();
        let mut pr = Problem::new(systems.clone());
        pr.jac = self.spec.jac.core();
        // 共通の変数（全状態）と、状態ごとの変数（状態 k だけに効く）
        let mut specs: Vec<(&VarSpec, Option<usize>)> =
            self.spec.vars.iter().map(|v| (v, None)).collect();
        for k in 0..self.spec.states.len() {
            specs.extend(self.spec.state_vars.iter().map(|v| (v, Some(k))));
        }
        for (v, cfg) in specs {
            let (spec, range) = v.parts()?;
            let c = cfg.unwrap_or(0);
            let param = var_param(&systems[c], &gapss[c], spec)?;
            if cfg.is_none() {
                // 共通の変数は全状態で同じ面のパラメータでなければならない
                for (k, (sk, gk)) in systems.iter().zip(&gapss).enumerate().skip(1) {
                    if var_param(sk, gk, spec)? != param {
                        return Err(format!(
                            "variable {spec} points at another surface in state {} (put it in state_vars)",
                            self.spec.states[k]
                        ).into());
                    }
                }
            }
            let mut var = Var::new(param);
            if let Some(k) = cfg {
                var.configs = Some(vec![k]);
            }
            if let Some((lo, hi)) = range {
                var = var.bounds(lo, hi);
            }
            pr.vars.push(var);
        }
        pr.add_default(self.spec.merit.core(), rings, self.spec.reference.core());
        let n_image = pr.terms.len();
        if let Some(t) = self.spec.efl {
            let w: f64 = pr.terms.iter().map(|t| t.weight).sum();
            for c in 0..pr.configs.len() {
                pr.terms.push(Term {
                    side: Default::default(),
                    config: c,
                    weight: 10.0 * w,
                    kind: TermKind::Paraxial {
                        q: ParaxQ::Efl,
                        wave: None,
                        target: t,
                    },
                });
            }
        }
        if !self.spec.terms.is_empty() {
            let ts =
                crate::merit::terms_from_json(&Value::Array(self.spec.terms.clone()), &pr.configs)?;
            pr.terms.extend(ts);
        }
        if self.spec.clear_aperture {
            let w: f64 = pr.terms[..n_image].iter().map(|t| t.weight).sum::<f64>() * 1e6;
            let mut add = Vec::new();
            for (c, sc) in pr.configs.iter().enumerate() {
                for sl in &self.spec.slots {
                    let u = sc
                        .units
                        .iter()
                        .find(|u| u.name == sl.block)
                        .ok_or_else(|| {
                            format!(
                                "surfaces of lens {} not found (clear aperture condition)",
                                sl.block
                            )
                        })?;
                    for sp in &u.spans {
                        for j in sp[0]..=sp[1] {
                            let semi = sc.surfaces[j].semi;
                            if semi.is_nan() || semi <= 0.0 || sc.surfaces[j].is_coord_break() {
                                continue;
                            }
                            for f in 0..sc.fields.len() {
                                for (px, py, q, side) in [
                                    (0.0, 1.0, RayQ::Y, Side::Max),
                                    (0.0, -1.0, RayQ::Y, Side::Min),
                                    (1.0, 0.0, RayQ::X, Side::Max),
                                    (-1.0, 0.0, RayQ::X, Side::Min),
                                ] {
                                    add.push(Term {
                                        side,
                                        config: c,
                                        weight: w,
                                        kind: TermKind::Ray {
                                            field: lensbench_core::FieldSel::Index(f),
                                            wave: None,
                                            px,
                                            py,
                                            surf: Some(j),
                                            q,
                                            target: if side == Side::Max { semi } else { -semi },
                                        },
                                    });
                                }
                            }
                        }
                    }
                }
            }
            pr.terms.extend(add);
        }
        Ok((s, pr, n_image))
    }

    /// 組 i を評価した結果（組 i だけから決まる）。評価できなければ error に理由
    pub fn eval(&self, i: usize) -> SwapResult {
        self.result(i, self.optimize(&self.assign(i)))
    }

    /// 組 i の最適化の結果 fit を [`SwapResult`] に
    fn result(&self, i: usize, fit: lensbench_core::Result<SwapFit>) -> SwapResult {
        let lenses = self
            .combo(i)
            .iter()
            .enumerate()
            .map(|(k, &c)| {
                let pk = &self.picks[k][c];
                SwapLens {
                    block: self.spec.slots[k].block.clone(),
                    vendor: pk.lens.vendor.clone(),
                    part: pk.lens.part.clone(),
                    reversed: pk.reversed,
                    efl: fin(pk.lens.efl),
                    diameter: fin(pk.lens.diameter),
                }
            })
            .collect();
        let (fit, error) = match fit {
            Ok(f) => (Some(f), None),
            Err(e) => (None, Some(String::from(e))),
        };
        SwapResult {
            i,
            lenses,
            fit,
            error,
        }
    }

    /// 選び方 a を最適化する。同じ形の選び方（[`Self::same`]）がある探索では、代表の選び方に直して 1 回だけ
    /// 最適化し、結果を使い回す（置いた系が同じなので結果も同じ）
    fn optimize(&self, a: &[Option<usize>]) -> lensbench_core::Result<SwapFit> {
        if self
            .same
            .iter()
            .all(|v| v.iter().enumerate().all(|(c, &r)| c == r))
        {
            return self.optimize_one(a);
        }
        let canon: Vec<Option<usize>> = a
            .iter()
            .enumerate()
            .map(|(k, c)| c.map(|c| self.same[k][c]))
            .collect();
        // 仲間のいない組（所ごとの仲間の数の積が 1）は使い回さないので覚えない（覚えると組の数に比例してメモリを使う）
        let mates: usize = canon
            .iter()
            .enumerate()
            .map(|(k, c)| c.map_or(1, |r| self.same[k].iter().filter(|&&x| x == r).count()))
            .product();
        if mates < 2 {
            return self.optimize_one(&canon);
        }
        let cell = self
            .fits
            .lock_ok()
            .entry(canon.clone())
            .or_default()
            .clone();
        cell.get_or_init(|| self.optimize_one(&canon)).clone()
    }

    /// 覚えている最適化の結果の数（仲間のいる代表の組だけ。試験用）
    #[doc(hidden)]
    pub fn cached_fit_count(&self) -> usize {
        self.fits.lock_ok().len()
    }

    /// 同じ形の組の数（代表の選び方の組の数。[`Self::count`] 以下）
    pub fn distinct_count(&self) -> usize {
        self.same
            .iter()
            .map(|v| v.iter().enumerate().filter(|(c, &r)| *c == r).count())
            .product()
    }

    fn optimize_one(&self, a: &[Option<usize>]) -> lensbench_core::Result<SwapFit> {
        let (s, pr0, n_image) = self.problem_of(a, self.spec.rings)?;
        self.efl_reachable(&pr0)?;
        let (start, pr, r, names) = self.best_start(pr0)?;
        self.report(&s, n_image, start, pr, r, names)
    }

    /// 粗いふるいの評価関数（リング数 `prescreen.rings` で最適化した値。評価できなければ None）
    fn coarse_merit(&self, i: usize, rings: usize) -> Option<f64> {
        let (_, pr0, _) = self.problem_of(&self.assign(i), rings).ok()?;
        self.efl_reachable(&pr0).ok()?;
        let (_, _, r, _) = self.best_start(pr0).ok()?;
        r.merit.is_finite().then_some(r.merit)
    }

    /// 二段目（今の評価）に回す組の番号（小さい順）。粗いふるいがなければ全組。threads は [`Self::run`] と同じ
    pub fn screened(&self, threads: Option<usize>) -> Vec<usize> {
        let o = lensbench_runs::RunOpts {
            threads,
            ..Default::default()
        };
        // ファイルに書かず止めもしなければ失敗しない（組ごとの失敗は None）
        self.screen(&o).map(|x| x.0).unwrap_or_default()
    }

    /// [`Self::screened`] を一括実行（[`lensbench_runs::run`]）で回す: (二段目に回す組の番号, 止めたか)。
    /// o.out があれば一段目の粗い評価関数を組ごとにそのファイルへ書き、o.resume なら同じ頭のファイルの続きから回す
    /// （o.header・o.stage は使わず、一段目の頭を書く）。進み具合は段 `"screen"` で文脈の受け手へ知らせ、文脈の中断の印で止める。
    /// 止めたら番号は空
    pub fn screen(
        &self,
        o: &lensbench_runs::RunOpts,
    ) -> lensbench_core::Result<(Vec<usize>, bool)> {
        let n = self.count();
        let Some(pre) = &self.spec.prescreen else {
            return Ok(((0..n).collect(), false));
        };
        let vals = std::sync::Mutex::new(vec![None; n]);
        let ro = lensbench_runs::RunOpts {
            threads: o.threads,
            out: o.out,
            header: self.header_screen(),
            resume: o.resume,
            stage: SwapStage::Screen.name(),
            keep_going: false,
        };
        let sum = lensbench_runs::run(n, 0, &ro, |i, _| {
            let v = self.coarse_merit(i, pre.rings);
            vals.lock_ok()[i] = v;
            serde_json::to_string(&v).map_err(|e| lensbench_core::Error::Invalid(e.to_string()))
        })?;
        if sum.cancelled {
            return Ok((Vec::new(), true));
        }
        let mut m = vals.into_inner().unwrap_or_else(|e| e.into_inner());
        if let Some(path) = o.out {
            // 再開で飛ばした組は書いた値から
            let (_, rows) = lensbench_runs::read_results(path)?;
            for (i, _, r) in rows {
                if i < n {
                    m[i] = serde_json::from_str::<Option<f64>>(&r).map_err(|e| e.to_string())?;
                }
            }
        }
        let mut ok: Vec<(f64, usize)> = m
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.filter(|v| v.is_finite()).map(|v| (v, i)))
            .collect();
        ok.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut keep: Vec<usize> = ok.into_iter().take(pre.keep).map(|x| x.1).collect();
        keep.sort_unstable();
        Ok((keep, false))
    }

    /// 出発点ごとに最適化して、評価関数の最も小さい解: (出発点の番号, 問題, 結果, 変数の (名前, 状態))
    #[allow(clippy::type_complexity)]
    fn best_start(
        &self,
        pr0: Problem,
    ) -> Result<
        (
            usize,
            Problem,
            lensbench_optim::DlsResult,
            Vec<(String, Option<usize>)>,
        ),
        String,
    > {
        let o = DlsOpts {
            max_iter: self.spec.iters,
            ..Default::default()
        };
        // 出発点ごとに最適化して、評価関数の最も小さい解を採る
        // pr.vars と同じ順の (名前, 状態の番号)
        let vname = |v: &VarSpec| v.name().to_string();
        let mut names: Vec<(String, Option<usize>)> =
            self.spec.vars.iter().map(|v| (vname(v), None)).collect();
        for k in 0..self.spec.states.len() {
            names.extend(self.spec.state_vars.iter().map(|v| (vname(v), Some(k))));
        }
        let empty = std::collections::BTreeMap::new();
        let starts: Vec<&std::collections::BTreeMap<String, f64>> = if self.spec.starts.is_empty() {
            vec![&empty]
        } else {
            self.spec.starts.iter().collect()
        };
        let mut best: Option<(usize, Problem, lensbench_optim::DlsResult)> = None;
        let mut last_err = None;
        for (k, st) in starts.iter().enumerate() {
            let mut pr = pr0.clone();
            // 共通の変数は全状態に、状態ごとの変数はその状態に入れる
            let mut set: Vec<Vec<(Param, f64)>> = vec![Vec::new(); pr.configs.len()];
            for (name, &v) in st.iter() {
                let qs: Vec<usize> = (0..names.len()).filter(|&q| &names[q].0 == name).collect();
                if qs.is_empty() {
                    return Err(format!(
                        "start variable {name} is not in vars or state_vars"
                    ));
                }
                for q in qs {
                    match names[q].1 {
                        None => set.iter_mut().for_each(|x| x.push((pr.vars[q].param, v))),
                        Some(k) => set[k].push((pr.vars[q].param, v)),
                    }
                }
            }
            for (c, sv) in pr.configs.iter_mut().zip(&set) {
                if !sv.is_empty() {
                    *c = c.with_params(sv)?;
                }
            }
            match lensbench_optim::dls_inner(&pr, &o) {
                Ok(r) if r.merit.is_finite() => {
                    let better = match &best {
                        None => true,
                        Some(b) => r.merit < b.2.merit,
                    };
                    if better {
                        best = Some((k, pr, r));
                    }
                }
                Ok(_) => last_err = Some("the merit function is not finite".to_string()),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        let Some((start, pr, r)) = best else {
            return Err(last_err.unwrap_or_else(|| "no start point".into()));
        };
        Ok((start, pr, r, names))
    }

    /// 最適化の結果
    fn report(
        &self,
        s: &System,
        n_image: usize,
        start: usize,
        pr: Problem,
        r: lensbench_optim::DlsResult,
        names: Vec<(String, Option<usize>)>,
    ) -> lensbench_core::Result<SwapFit> {
        let mut img = pr.clone();
        img.terms.truncate(n_image);
        let image_merit = img.merit(&r.x)?;
        let vars = pr
            .vars
            .iter()
            .zip(&names)
            .enumerate()
            .map(|(q, (v, (name, k)))| SwapVar {
                name: name.clone(),
                param: s.param_name(v.param),
                x0: fin(r.x0[q]),
                x: fin(r.x[q]),
                state: k.map(|k| self.spec.states[k].clone()),
            })
            .collect();
        let sys = &r.systems[0];
        let mut states = None;
        if !self.spec.states.is_empty() {
            // 状態ごとの像の評価関数と焦点距離
            let mut st = Vec::new();
            for (k, name) in self.spec.states.iter().enumerate() {
                let mut one = img.clone();
                one.terms.retain(|t| t.config == k);
                let m = one.merit(&r.x)?;
                let efl = fin(r.systems[k].paraxial().efl);
                st.push(SwapState {
                    state: name.clone(),
                    image_merit: fin(m),
                    efl,
                });
            }
            states = Some(st);
        }
        Ok(SwapFit {
            merit0: fin(r.merit0),
            merit: fin(r.merit),
            image_merit: fin(image_merit),
            start: (!self.spec.starts.is_empty()).then_some(start),
            stop: stop_name(r.stop).to_string(),
            iterations: r.iterations,
            vars,
            efl: fin(sys.paraxial().efl),
            total_track: fin((1..sys.image()).map(|j| sys.surfaces[j].t).sum::<f64>()),
            states,
        })
    }

    /// 一括実行の結果のファイルの頭: 版・設定・元のプロジェクトの hash・所ごとの候補。
    /// 候補（カタログ）・設定・プロジェクトが変われば違う頭になり、再開できない
    pub fn header(&self) -> String {
        self.header_value().to_string()
    }

    /// 粗いふるいの段の結果のファイルの頭（`keep` は一段目の値を変えないので含めない）
    fn header_screen(&self) -> String {
        let mut h = self.header_value();
        h["kind"] = json!("swap-prescreen");
        if let Some(p) = h["spec"]["prescreen"].as_object_mut() {
            p.remove("keep");
        }
        h.to_string()
    }

    fn header_value(&self) -> Value {
        let picks: Vec<Vec<Value>> = self
            .picks
            .iter()
            .map(|v| {
                v.iter()
                    .map(|p| json!([p.lens.vendor, p.lens.part, p.reversed]))
                    .collect()
            })
            .collect();
        json!({
            "lensbench": lensbench_core::VERSION,
            "kind": "swap",
            "project_hash": self.base_hash,
            "spec": serde_json::to_value(&self.spec).expect("the spec serializes to JSON"),
            "picks": picks,
        })
    }

    /// [`run`](Self::run) と同じ。out があれば一括実行（[`lensbench_runs::run`]）で結果を 1 組ずつ
    /// JSONL に書き出し、resume なら同じ頭のファイルの続きから回す（書いた組は飛ばす）。返り値はファイルの全部の組。
    /// 粗いふるいがあれば、一段目の値は [`screen_path`]`(out)` に書いて同じく続きから回す。
    /// 進み具合は文脈の受け手（[`par::with_progress`]）へ、段（[`SwapStage::name`]）を付けて知らせる
    pub fn run_to(
        &self,
        out: Option<&std::path::Path>,
        resume: bool,
        threads: Option<usize>,
    ) -> lensbench_core::Result<Vec<SwapResult>> {
        if self.spec.beam.is_some() {
            return self.beam_to(out, threads);
        }
        let Some(path) = out else {
            return Ok(self.run(threads));
        };
        let pre_path = screen_path(path);
        let pre = lensbench_runs::RunOpts {
            threads,
            out: Some(&pre_path),
            resume,
            ..Default::default()
        };
        let (idx, _) = self.screen(&pre)?;
        let mut header = self.header_value();
        if self.spec.prescreen.is_some() {
            // 二段目の番号 j は残した組の j 番目。残した組が変われば続きから回せない
            header["screened"] = json!(lensbench_project::sysfile::hash_value(&json!(idx)));
        }
        let o = lensbench_runs::RunOpts {
            threads,
            out: Some(path),
            header: header.to_string(),
            resume,
            stage: SwapStage::Main.name(),
            keep_going: false,
        };
        lensbench_runs::run(idx.len(), 0, &o, |j, _| {
            serde_json::to_string(&self.eval(idx[j]))
                .map_err(|e| lensbench_core::Error::Invalid(e.to_string()))
        })
        .map_err(|e| {
            let e = e.to_string();
            // 残した組が変わった（keep を変えたなど）: 一段目の結果はそのまま使える
            if self.spec.prescreen.is_some() && pre_path.exists() && e.contains("the header differs") {
                format!(
                    "{}; the prescreen results in {} are still valid: remove {} (keep {}) to rerun only the second stage",
                    e,
                    pre_path.display(),
                    path.display(),
                    pre_path.display()
                )
            } else {
                e
            }
        })?;
        let (_, rows) = lensbench_runs::read_results(path)?;
        let mut v = rows
            .into_iter()
            .map(|(_, _, r)| serde_json::from_str(&r).map_err(|e| e.to_string().into()))
            .collect::<lensbench_core::Result<Vec<SwapResult>>>()?;
        sort_results(&mut v);
        Ok(v)
    }

    /// すべての組（ビーム探索なら [`Self::beam_run`] で評価した組）を並列に評価し、評価関数の小さい順に並べる（評価できなかった組は後ろ、番号の順）。
    /// threads は [`par::with_threads`] と同じ（None は外の指定を引き継ぐ）
    pub fn run(&self, threads: Option<usize>) -> Vec<SwapResult> {
        if self.spec.beam.is_some() {
            return self.beam_run(threads).0;
        }
        let idx = self.screened(threads);
        let mut v = par::with_threads(threads, || par::map(&idx, |&i| self.eval(i)));
        sort_results(&mut v);
        v
    }
}

/// 一括実行の段（[`SwapSearch::run_to`] の進み具合）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStage {
    /// 粗いふるい（全組）
    Screen,
    /// 今の評価（残した組。ふるいがなければ全組）
    Main,
}

impl SwapStage {
    /// 進み具合の段の名前（[`lensbench_core::par::Count::stage`]）: `"screen"`・`"main"`
    pub fn name(self) -> &'static str {
        match self {
            SwapStage::Screen => "screen",
            SwapStage::Main => "main",
        }
    }
}

/// 結果のファイル out に対する、粗いふるいの段の結果のファイル（`r.jsonl` なら `r.pre.jsonl`）
pub fn screen_path(out: &std::path::Path) -> PathBuf {
    out.with_extension("pre.jsonl")
}

/// 評価関数の小さい順（評価できなかった組は後ろ）、同じなら番号の順
pub fn sort_results(v: &mut [SwapResult]) {
    v.sort_by(|a, b| a.merit().total_cmp(&b.merit()).then(a.i.cmp(&b.i)));
}

/// 差し替え探索の 1 組の結果（JSON はこの形。鍵の並びも同じ）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapResult {
    /// 組の番号（0 始まり）
    pub i: usize,
    pub lenses: Vec<SwapLens>,
    /// 最適化の結果（評価できなければ None で error に理由）
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<SwapFit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SwapResult {
    /// 並べる評価関数（評価できなければ無限大）
    pub fn merit(&self) -> f64 {
        self.fit
            .as_ref()
            .and_then(|f| f.merit)
            .unwrap_or(f64::INFINITY)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 差し替え探索の返事（serve の swap の結果。JSON はこの形で鍵の並びも同じ）。count_only なら
/// count と slots だけ
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SwapReply {
    /// 組の数
    pub count: usize,
    /// 粗いふるいで残った組の数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screened: Option<usize>,
    /// 評価した組・失敗した組の数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<usize>,
    /// 途中で止めた
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// 所ごとの候補
    pub slots: Vec<Vec<SwapCand>>,
    /// 評価関数の順に上位の組
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<SwapResult>>,
}

/// 所の候補の玉
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapCand {
    pub vendor: String,
    pub part: String,
    pub reversed: bool,
    pub efl: Option<f64>,
    pub diameter: Option<f64>,
}

impl SwapSearch<'_> {
    /// 所ごとの候補（[`SwapReply::slots`]）
    pub fn slots(&self) -> Vec<Vec<SwapCand>> {
        self.picks
            .iter()
            .map(|v| {
                v.iter()
                    .map(|k| SwapCand {
                        vendor: k.lens.vendor.clone(),
                        part: k.lens.part.clone(),
                        reversed: k.reversed,
                        efl: fin(k.lens.efl),
                        diameter: fin(k.lens.diameter),
                    })
                    .collect()
            })
            .collect()
    }
}

/// 組の所ごとの玉
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapLens {
    pub block: String,
    pub vendor: String,
    pub part: String,
    pub reversed: bool,
    pub efl: Option<f64>,
    pub diameter: Option<f64>,
}

/// 組を最適化した結果（数が有限でなければ None = JSON の null）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapFit {
    pub merit0: Option<f64>,
    pub merit: Option<f64>,
    /// 像の項だけの評価関数
    pub image_merit: Option<f64>,
    /// 採った出発点の番号（starts が無ければ None）
    pub start: Option<usize>,
    pub stop: String,
    pub iterations: usize,
    pub vars: Vec<SwapVar>,
    pub efl: Option<f64>,
    pub total_track: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub states: Option<Vec<SwapState>>,
}

/// 最適化した変数
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapVar {
    pub name: String,
    pub param: String,
    pub x0: Option<f64>,
    pub x: Option<f64>,
    /// 状態ごとの変数の状態の名前
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// 状態ごとの像の評価関数と焦点距離
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwapState {
    pub state: String,
    pub image_merit: Option<f64>,
    pub efl: Option<f64>,
}

/// 有限なら Some（JSON は数、そうでなければ null）
fn fin(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// 変数の書き方: id のある空気は `id.t`、ほかは [`System::param`]
fn var_param(s: &System, gaps: &[Gap], spec: &str) -> lensbench_core::Result<Param> {
    if let Some((id, k)) = spec.rsplit_once('.') {
        let hits: Vec<&Gap> = gaps.iter().filter(|g| g.0 == id.trim()).collect();
        if !hits.is_empty() && k.trim().eq_ignore_ascii_case("t") {
            let (surf, sign) = match hits.as_slice() {
                [h] => (h.1, h.2),
                _ => {
                    return Err(format!(
                        "air {id} is passed twice (move the round-trip gap with the group shift)"
                    )
                    .into())
                }
            };
            if sign < 0.0 {
                return Err(
                    format!("air {id} is after a mirror (move it with the group shift)").into(),
                );
            }
            if !s.surfaces[surf].t.is_finite() {
                return Err(format!("air {id} is the object distance (infinity)").into());
            }
            let p = Param::new(surf, Kind::Thick);
            s.check_param(p)?;
            return Ok(p);
        }
    }
    s.param(spec)
}

fn stop_name(s: Stop) -> &'static str {
    s.as_str()
}
