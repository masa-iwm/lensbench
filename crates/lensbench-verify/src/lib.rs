//! プラグインと解析部品の照合試験の枠組み。
//!
//! 照合 1 件を JSON の記述（`*.verify.json`）で書き、登録のプラグイン（同梱・非公開版・Python で足した分）を
//! 基準データ（npz か JSON に直接）と比べる。基準データは別の実装（従来の実装・OpticStudio・Python の試作）が
//! 作ったもので、作った側の版と作り方を記述に一緒に残す。
//!
//! ```json
//! {
//!   "format": "lensbench-verify", "version": 1,
//!   "name": "A0 の Zernike（OpticStudio）",
//!   "source": {"by": "OpticStudio 26.1", "commit": "…", "how": "make_samples.py", "date": "2026-11-09"},
//!   "ci": true,
//!   "system": {"path": "../refs/A0/system.zmx", "cfg": 1, "glass_dirs": ["../glass"]},
//!   "plugin": "zernike",
//!   "config": {"terms": 37, "ref_primary": 0},
//!   "threads": [1, 3],
//!   "checks": {
//!     "coef": {"abs": 5e-6},
//!     "rms_fit": {"abs": 1e-8, "note": "OS の表示桁"}
//!   },
//!   "runs": [
//!     {"config": {"wave": 1}, "inputs": {"field": 1}, "expected": {"coef": [0.5, 0.0]}},
//!     {"inputs": "in.npz", "expected": "out.npz"}
//!   ]
//! }
//! ```
//!
//! 約束:
//! - 比べる量ごとに許容差を書く。`abs`（絶対値の差）・`rel`（基準の配列の最大の絶対値に対する差）・
//!   `exact`（ビット同じ）。`abs` と `rel` を両方書けば、どちらかに収まれば通る。緩めた理由は `note` に書く
//! - 基準の配列（`expected`）の名前には全部 `checks` が要る（書き忘れた量を黙って通さない）。
//!   プラグインの出力で基準にない名前は比べない
//! - 値でない要素（NaN、けられた光線など）は、両方が値でないときだけ一致とする。片方だけなら許容差によらず不一致
//!   （位置の差だけを見て、片方だけ通った光線を見落とさないため）。bool の配列は全要素の一致
//! - `threads`（2 つ以上）を書けば、各スレッド数で回した出力（基準にない名前も含む全部）がビット同じかを確かめる
//! - `ci`: false の照合（時間のかかる通しの照合など）は `--ci`（CI 用）では飛ばし、手元でだけ回す
//! - パスは記述のファイルからの相対。`runs` の `config` は上の `config` に重ねる
//!
//! 入力・基準を JSON に直接書くときは、数は形 []、入れ子の並びは多次元の配列、真偽の並びは bool の配列、
//! `{"re": [...], "im": [...]}` は complex の配列（re・im は同じ形）。npz は float64・complex128・bool の配列。

pub mod zemax_refs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lensbench_core::coating::C64;
use lensbench_core::plugin::{Array, Arrays, Config, Context, Data, Registry, Value as CfgValue};
use lensbench_core::System;
use serde_json::{json, Value};

/// 量ごとの許容差
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tol {
    pub abs: Option<f64>,
    pub rel: Option<f64>,
    /// ビット同じを求める
    pub exact: bool,
    /// 緩めた理由など
    pub note: String,
}

impl Tol {
    fn parse(name: &str, v: &Value) -> Result<Tol, String> {
        let o = v
            .as_object()
            .ok_or_else(|| format!("checks.{name} はオブジェクトに"))?;
        let mut t = Tol::default();
        for (k, x) in o {
            let num = || {
                x.as_f64()
                    .filter(|v| *v >= 0.0)
                    .ok_or_else(|| format!("checks.{name}.{k} は 0 以上の数に"))
            };
            match k.as_str() {
                "abs" => t.abs = Some(num()?),
                "rel" => t.rel = Some(num()?),
                "exact" => {
                    t.exact = x
                        .as_bool()
                        .ok_or_else(|| format!("checks.{name}.exact は true か false に"))?
                }
                "note" => t.note = x.as_str().unwrap_or_default().to_string(),
                _ => {
                    return Err(format!(
                        "checks.{name} の知らない項目 {k}（abs・rel・exact・note）"
                    ))
                }
            }
        }
        if t.abs.is_none() && t.rel.is_none() && !t.exact {
            return Err(format!("checks.{name} に abs・rel・exact のどれかが要る"));
        }
        Ok(t)
    }
}

/// 1 つの量の比較の結果
#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    /// 差の最大（絶対値）
    pub max_abs: f64,
    /// 差の最大 / 基準の最大の絶対値
    pub max_rel: f64,
    /// 差が最大の要素の番号（平らにした並び）
    pub worst: Option<usize>,
    /// 片方だけ値でない要素の数（bool なら値の違う要素の数）
    pub mismatched: usize,
    /// 形・型の違いなど、比べられなかった理由
    pub error: Option<String>,
    pub tol: Tol,
}

/// 値でない数どうしが同じか（NaN 同士か、同じ符号の Inf）。有限の数は等しいとき
fn same_value(p: f64, q: f64) -> bool {
    (p.is_nan() && q.is_nan()) || p == q
}

/// `actual` の量 `name` を基準 `expected` と許容差 `tol` で比べる
pub fn compare_one(name: &str, actual: Option<&Array>, expected: &Array, tol: &Tol) -> CheckResult {
    let mut r = CheckResult {
        name: name.to_string(),
        passed: false,
        max_abs: 0.0,
        max_rel: 0.0,
        worst: None,
        mismatched: 0,
        error: None,
        tol: tol.clone(),
    };
    let Some(a) = actual else {
        r.error = Some("出力にない".into());
        return r;
    };
    // 形 [] と [1] のような長さの同じ違いは許す（数 1 つの書き方の違い）
    if a.shape != expected.shape && !(a.data.len() == 1 && expected.data.len() == 1) {
        r.error = Some(format!(
            "形が違う（{:?} と基準 {:?}）",
            a.shape, expected.shape
        ));
        return r;
    }
    // 差と基準の大きさ（complex は絶対値）、値でない要素
    let (diff, refmag, bits_same): (Vec<Option<f64>>, Vec<f64>, bool) =
        match (&a.data, &expected.data) {
            (Data::Bool(x), Data::Bool(y)) => {
                r.mismatched = x.iter().zip(y).filter(|(p, q)| p != q).count();
                r.passed = r.mismatched == 0;
                return r;
            }
            (Data::F64(x), Data::F64(y)) => (
                x.iter()
                    .zip(y)
                    .map(|(p, q)| match (p.is_finite(), q.is_finite()) {
                        (true, true) => Some((p - q).abs()),
                        (false, false) if same_value(*p, *q) => Some(0.0),
                        _ => None,
                    })
                    .collect(),
                y.iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .map(f64::abs)
                    .collect(),
                x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits()),
            ),
            (Data::C64(x), Data::C64(y)) => (
                x.iter()
                    .zip(y)
                    .map(|(p, q)| {
                        let (fp, fq) = (
                            p.re.is_finite() && p.im.is_finite(),
                            q.re.is_finite() && q.im.is_finite(),
                        );
                        match (fp, fq) {
                            (true, true) => Some((p - q).norm()),
                            // 値でないものは成分ごとに f64 と同じ約束（NaN 同士か同じ Inf）
                            (false, false) if same_value(p.re, q.re) && same_value(p.im, q.im) => {
                                Some(0.0)
                            }
                            _ => None,
                        }
                    })
                    .collect(),
                y.iter()
                    .filter(|v| v.re.is_finite() && v.im.is_finite())
                    .map(|v| v.norm())
                    .collect(),
                x.iter().zip(y).all(|(p, q)| {
                    p.re.to_bits() == q.re.to_bits() && p.im.to_bits() == q.im.to_bits()
                }),
            ),
            (p, q) => {
                r.error = Some(format!("型が違う（{} と基準 {}）", p.dtype(), q.dtype()));
                return r;
            }
        };
    let scale = refmag.iter().copied().fold(0.0, f64::max);
    for (i, d) in diff.iter().enumerate() {
        match d {
            None => r.mismatched += 1,
            Some(d) => {
                if *d > r.max_abs || r.worst.is_none() && *d >= r.max_abs {
                    r.max_abs = *d;
                    r.worst = Some(i);
                }
            }
        }
    }
    r.max_rel = if scale > 0.0 {
        r.max_abs / scale
    } else if r.max_abs > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let within = tol.abs.is_some_and(|t| r.max_abs <= t) || tol.rel.is_some_and(|t| r.max_rel <= t);
    r.passed = r.mismatched == 0 && if tol.exact { bits_same } else { within };
    r
}

/// 許容差の JSON（名前 → {abs, rel, exact, note}）を読む
pub fn checks_from_json(v: &Value) -> Result<BTreeMap<String, Tol>, String> {
    match v {
        Value::Object(c) => c
            .iter()
            .map(|(k, x)| Tol::parse(k, x).map(|t| (k.clone(), t)))
            .collect(),
        _ => Err("checks（量ごとの許容差）はオブジェクトに".into()),
    }
}

fn fin(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        json!(x.to_string())
    }
}

impl Tol {
    pub fn to_json(&self) -> Value {
        json!({"abs": self.abs, "rel": self.rel, "exact": self.exact, "note": self.note})
    }
}

impl CheckResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "passed": self.passed, "max_abs": fin(self.max_abs), "max_rel": fin(self.max_rel),
            "worst": self.worst, "mismatched": self.mismatched, "error": self.error, "tol": self.tol.to_json(),
        })
    }
}

/// 出力 `actual` を基準 `expected` と比べる（基準の名前ごと、`checks` に無い名前は誤り）
pub fn compare(
    actual: &Arrays,
    expected: &Arrays,
    checks: &BTreeMap<String, Tol>,
) -> Result<Vec<CheckResult>, String> {
    let missing: Vec<&str> = expected
        .keys()
        .filter(|k| !checks.contains_key(*k))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "基準の {} に許容差（checks）がない",
            missing.join("・")
        ));
    }
    Ok(expected
        .iter()
        .map(|(k, e)| compare_one(k, actual.get(k), e, &checks[k]))
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------
// 配列の読み込み

/// JSON の値を配列に（数・入れ子の並び・真偽・{"re", "im"}）
pub fn array_from_json(v: &Value) -> Result<Array, String> {
    if let Some(o) = v.as_object() {
        let (Some(re), Some(im)) = (o.get("re"), o.get("im")) else {
            return Err("オブジェクトの配列は {\"re\": …, \"im\": …} に".into());
        };
        let (re, im) = (array_from_json(re)?, array_from_json(im)?);
        let (Some(a), Some(b)) = (re.as_f64(), im.as_f64()) else {
            return Err("re・im は数の配列に".into());
        };
        if re.shape != im.shape {
            return Err("re・im の形が違う".into());
        }
        let c = a.iter().zip(b).map(|(x, y)| C64::new(*x, *y)).collect();
        return Array::c64(re.shape, c).map_err(|e| e.to_string());
    }
    let mut shape = Vec::new();
    let mut cur = v;
    while let Some(a) = cur.as_array() {
        shape.push(a.len());
        match a.first() {
            Some(x) => cur = x,
            None => break,
        }
    }
    let mut leaves = Vec::new();
    fn walk<'a>(
        v: &'a Value,
        depth: usize,
        shape: &[usize],
        out: &mut Vec<&'a Value>,
    ) -> Result<(), String> {
        if depth == shape.len() {
            if v.is_array() {
                return Err("並びの入れ子の深さがそろっていない".into());
            }
            out.push(v);
            return Ok(());
        }
        let a = v.as_array().ok_or("並びの入れ子の深さがそろっていない")?;
        if a.len() != shape[depth] {
            return Err("並びの長さがそろっていない".into());
        }
        a.iter().try_for_each(|x| walk(x, depth + 1, shape, out))
    }
    walk(v, 0, &shape, &mut leaves)?;
    if !leaves.is_empty() && leaves.iter().all(|x| x.is_boolean()) {
        let b = leaves
            .iter()
            .map(|x| x.as_bool().unwrap_or(false))
            .collect();
        return Array::bool(shape, b).map_err(|e| e.to_string());
    }
    let f = leaves
        .iter()
        .map(|x| match x {
            Value::Number(n) => n.as_f64().ok_or_else(|| "数にできない".to_string()),
            // NaN・inf は JSON に書けないので、null を NaN、文字列の "inf"・"-inf"・"nan" を受ける
            Value::Null => Ok(f64::NAN),
            Value::String(s) => match s.as_str() {
                "inf" => Ok(f64::INFINITY),
                "-inf" => Ok(f64::NEG_INFINITY),
                "nan" => Ok(f64::NAN),
                _ => Err(format!("数でない値 {s}")),
            },
            _ => Err(format!("数でない値 {x}")),
        })
        .collect::<Result<Vec<f64>, String>>()?;
    Array::f64(shape, f).map_err(|e| e.to_string())
}

/// 配列を JSON の値に（[`array_from_json`] の逆。NaN は null、無限大は "inf"・"-inf"）
pub fn array_to_json(a: &Array) -> Value {
    fn num(x: f64) -> Value {
        if x.is_nan() {
            Value::Null
        } else if x.is_infinite() {
            json!(if x > 0.0 { "inf" } else { "-inf" })
        } else {
            json!(x)
        }
    }
    fn nest(shape: &[usize], flat: Vec<Value>) -> Value {
        match shape {
            [] => flat.into_iter().next().unwrap_or(Value::Null),
            [_] => Value::Array(flat),
            [_, rest @ ..] => {
                let m: usize = rest.iter().product();
                if m == 0 {
                    return Value::Array(Vec::new());
                }
                let mut it = flat.into_iter();
                let n = shape[0];
                Value::Array(
                    (0..n)
                        .map(|_| nest(rest, it.by_ref().take(m).collect()))
                        .collect(),
                )
            }
        }
    }
    match &a.data {
        Data::F64(v) => nest(&a.shape, v.iter().map(|x| num(*x)).collect()),
        Data::Bool(v) => nest(&a.shape, v.iter().map(|x| json!(x)).collect()),
        Data::C64(v) => json!({
            "re": nest(&a.shape, v.iter().map(|x| num(x.re)).collect()),
            "im": nest(&a.shape, v.iter().map(|x| num(x.im)).collect()),
        }),
    }
}

/// npz の配列を全部読む（float64・complex128・bool。名前の .npy は外す）
pub fn read_npz(path: &Path) -> Result<Arrays, String> {
    use ndarray::ArrayD;
    use ndarray_npy::NpzReader;
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut z = NpzReader::new(f).map_err(|e| format!("{}: {e}", path.display()))?;
    let names = z.names().map_err(|e| e.to_string())?;
    let mut out = Arrays::new();
    for n in names {
        let key = n.strip_suffix(".npy").unwrap_or(&n).to_string();
        let arr = if let Ok(a) = z.by_name::<ndarray::OwnedRepr<f64>, ndarray::IxDyn>(&n) {
            to_array(a, Data::F64)
        } else if let Ok(a) = z.by_name::<ndarray::OwnedRepr<C64>, ndarray::IxDyn>(&n) {
            to_array(a, Data::C64)
        } else if let Ok(a) = z.by_name::<ndarray::OwnedRepr<bool>, ndarray::IxDyn>(&n) {
            to_array(a, Data::Bool)
        } else {
            return Err(format!(
                "{}: {key} は float64・complex128・bool でない",
                path.display()
            ));
        };
        out.insert(key, arr.map_err(|e| e.to_string())?);
        fn to_array<T: Clone>(
            a: ArrayD<T>,
            f: impl Fn(Vec<T>) -> Data,
        ) -> lensbench_core::Result<Array> {
            let shape = a.shape().to_vec();
            let v: Vec<T> = a.as_standard_layout().iter().cloned().collect();
            Array::new(shape, f(v))
        }
    }
    Ok(out)
}

/// 入力・基準: JSON のオブジェクト（名前 → 値）か npz のパス
fn arrays_of(v: &Value, base: &Path, what: &str) -> Result<Arrays, String> {
    match v {
        Value::String(p) => read_npz(&base.join(p)),
        Value::Object(o) => o
            .iter()
            .map(|(k, x)| {
                array_from_json(x)
                    .map(|a| (k.clone(), a))
                    .map_err(|e| format!("{what}.{k}: {e}"))
            })
            .collect(),
        Value::Null => Ok(Arrays::new()),
        _ => Err(format!("{what} はオブジェクトか npz のパスに")),
    }
}

/// 設定の JSON（数・文字列・数の並び、真偽は 0・1）
fn config_of(v: &Value, what: &str) -> Result<Config, String> {
    let Some(o) = v.as_object() else {
        return if v.is_null() {
            Ok(Config::new())
        } else {
            Err(format!("{what} はオブジェクトに"))
        };
    };
    o.iter()
        .map(|(k, x)| {
            let c = match x {
                Value::Number(n) => CfgValue::Num(n.as_f64().unwrap_or(f64::NAN)),
                Value::Bool(b) => CfgValue::Num(*b as u8 as f64),
                Value::String(s) => CfgValue::Str(s.clone()),
                Value::Array(a) => CfgValue::Nums(
                    a.iter()
                        .map(|y| y.as_f64().ok_or_else(|| format!("{what}.{k} は数の並びに")))
                        .collect::<Result<_, _>>()?,
                ),
                _ => return Err(format!("{what}.{k} は数・文字列・数の並びに")),
            };
            Ok((k.clone(), c))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// 記述と実行

/// 基準データの出どころ
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Source {
    /// 作った実装（OpticStudio 26.1・従来の実装・Python の試作など）
    pub by: String,
    /// 作った側の版（コミット）
    pub commit: String,
    /// 作り方（スクリプトと引数）
    pub how: String,
    pub date: String,
}

/// 照合 1 件の記述
#[derive(Debug, Clone)]
pub struct Manifest {
    pub path: PathBuf,
    pub name: String,
    pub about: String,
    pub source: Source,
    pub ci: bool,
    pub plugin: String,
    pub config: Config,
    pub system: Option<SystemSpec>,
    pub threads: Vec<usize>,
    pub checks: BTreeMap<String, Tol>,
    pub runs: Vec<RunSpec>,
}

/// 系の読み方
#[derive(Debug, Clone)]
pub struct SystemSpec {
    pub path: PathBuf,
    pub cfg: usize,
    pub glass_dirs: Vec<PathBuf>,
}

/// 1 回の呼び出し
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub label: String,
    pub config: Config,
    pub inputs: Arrays,
    pub expected: Arrays,
}

const KNOWN: [&str; 13] = [
    "format", "version", "name", "about", "source", "ci", "system", "plugin", "config", "threads",
    "checks", "runs", "comment",
];

impl Manifest {
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value =
            serde_json::from_str(&txt).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&v, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_json(v: &Value, path: &Path) -> Result<Manifest, String> {
        let o = v.as_object().ok_or("記述はオブジェクトに")?;
        if o.get("format").and_then(Value::as_str) != Some("lensbench-verify") {
            return Err("format は \"lensbench-verify\" に".into());
        }
        match o.get("version").and_then(Value::as_u64) {
            Some(1) => {}
            v => return Err(format!("version は 1 に（{v:?}）")),
        }
        if let Some(k) = o.keys().find(|k| !KNOWN.contains(&k.as_str())) {
            return Err(format!("知らない項目 {k}"));
        }
        let base = path.parent().unwrap_or(Path::new("."));
        let s = |k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let src = o.get("source").cloned().unwrap_or(Value::Null);
        let ss = |k: &str| {
            src.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let source = Source {
            by: ss("by"),
            commit: ss("commit"),
            how: ss("how"),
            date: ss("date"),
        };
        if source.by.is_empty() {
            return Err("source.by（基準を作った実装）が要る".into());
        }
        let plugin = s("plugin");
        if plugin.is_empty() {
            return Err("plugin が要る".into());
        }
        let system = match o.get("system") {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) => Some(SystemSpec {
                path: base.join(p),
                cfg: 1,
                glass_dirs: Vec::new(),
            }),
            Some(Value::Object(so)) => Some(SystemSpec {
                path: base.join(
                    so.get("path")
                        .and_then(Value::as_str)
                        .ok_or("system.path が要る")?,
                ),
                cfg: so.get("cfg").and_then(Value::as_u64).unwrap_or(1) as usize,
                glass_dirs: so
                    .get("glass_dirs")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(|p| base.join(p))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => return Err("system はパスかオブジェクトに".into()),
        };
        let threads: Vec<usize> = match o.get("threads") {
            None => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| {
                    x.as_u64()
                        .filter(|n| *n >= 1)
                        .map(|n| n as usize)
                        .ok_or("threads は 1 以上の整数の並びに")
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("threads は 1 以上の整数の並びに".into()),
        };
        let checks = checks_from_json(o.get("checks").unwrap_or(&Value::Null))?;
        let config = config_of(o.get("config").unwrap_or(&Value::Null), "config")?;
        let runs = o
            .get("runs")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
            .ok_or("runs（1 回以上）が要る")?
            .iter()
            .enumerate()
            .map(|(i, r)| -> Result<RunSpec, String> {
                let w = format!("runs[{i}]");
                let ro = r.as_object().ok_or(format!("{w} はオブジェクトに"))?;
                if let Some(k) = ro
                    .keys()
                    .find(|k| !["label", "config", "inputs", "expected"].contains(&k.as_str()))
                {
                    return Err(format!("{w} の知らない項目 {k}"));
                }
                let mut cfg = config.clone();
                cfg.extend(config_of(ro.get("config").unwrap_or(&Value::Null), &w)?);
                let expected = arrays_of(ro.get("expected").unwrap_or(&Value::Null), base, &w)?;
                if expected.is_empty() {
                    return Err(format!("{w} に expected（基準）が要る"));
                }
                Ok(RunSpec {
                    label: ro
                        .get("label")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{}", i + 1)),
                    config: cfg,
                    inputs: arrays_of(ro.get("inputs").unwrap_or(&Value::Null), base, &w)?,
                    expected,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Manifest {
            path: path.to_path_buf(),
            name: if s("name").is_empty() {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            } else {
                s("name")
            },
            about: s("about"),
            source,
            ci: o.get("ci").and_then(Value::as_bool).unwrap_or(true),
            plugin,
            config,
            system,
            threads,
            checks,
            runs,
        })
    }

    fn load_system(&self) -> Result<Option<Arc<System>>, String> {
        let Some(sp) = &self.system else {
            return Ok(None);
        };
        let ext = sp
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        let s = if ext.as_deref() == Some("json") {
            lensbench_project::sysfile::load(&sp.path)?.0
        } else {
            let opt = lensbench_core::zmx::LoadOptions {
                cfg: sp.cfg,
                catalog_dirs: sp.glass_dirs.clone(),
                ..Default::default()
            };
            lensbench_core::zmx::load_with(&sp.path, &opt).map_err(|e| e.to_string())?
        };
        Ok(Some(Arc::new(s)))
    }

    /// 登録 `reg` のプラグインで回して比べる
    pub fn run(&self, reg: &Registry) -> Report {
        let mut rep = Report {
            name: self.name.clone(),
            path: self.path.clone(),
            source: self.source.clone(),
            plugin: self.plugin.clone(),
            runs: Vec::new(),
            error: None,
        };
        let sys = match self.load_system() {
            Ok(s) => s,
            Err(e) => {
                rep.error = Some(format!("系: {e}"));
                return rep;
            }
        };
        let ctx = Context {
            system: sys.as_ref(),
            coatings: None,
        };
        for r in &self.runs {
            let mut rr = RunReport {
                label: r.label.clone(),
                checks: Vec::new(),
                threads_identical: None,
                error: None,
            };
            let res = (|| -> Result<(), String> {
                let inst = reg
                    .prepare(&self.plugin, &r.config)
                    .map_err(|e| e.to_string())?;
                let ts: Vec<Option<usize>> = if self.threads.is_empty() {
                    vec![None]
                } else {
                    self.threads.iter().map(|t| Some(*t)).collect()
                };
                let mut outs = Vec::new();
                for t in &ts {
                    let o = lensbench_core::with_threads(*t, || inst.run(&ctx, &r.inputs))
                        .map_err(|e| e.to_string())?;
                    outs.push(o);
                }
                if outs.len() > 1 {
                    rr.threads_identical =
                        Some(outs.iter().skip(1).all(|o| same_bits(o, &outs[0])));
                }
                rr.checks = compare(&outs[0], &r.expected, &self.checks)?;
                Ok(())
            })();
            if let Err(e) = res {
                rr.error = Some(e);
            }
            rep.runs.push(rr);
        }
        rep
    }
}

/// 名前・形・型・値のビットが全部同じ
pub fn same_bits(a: &Arrays, b: &Arrays) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|((ka, x), (kb, y))| {
            ka == kb
                && x.shape == y.shape
                && match (&x.data, &y.data) {
                    (Data::F64(p), Data::F64(q)) => {
                        p.len() == q.len()
                            && p.iter().zip(q).all(|(u, v)| u.to_bits() == v.to_bits())
                    }
                    (Data::C64(p), Data::C64(q)) => {
                        p.len() == q.len()
                            && p.iter().zip(q).all(|(u, v)| {
                                u.re.to_bits() == v.re.to_bits() && u.im.to_bits() == v.im.to_bits()
                            })
                    }
                    (Data::Bool(p), Data::Bool(q)) => p == q,
                    _ => false,
                }
        })
}

/// 1 回の呼び出しの結果
#[derive(Debug, Clone)]
pub struct RunReport {
    pub label: String,
    pub checks: Vec<CheckResult>,
    /// スレッド数を変えた出力がビット同じか（threads を 2 つ以上書いたときだけ）
    pub threads_identical: Option<bool>,
    pub error: Option<String>,
}

impl RunReport {
    pub fn passed(&self) -> bool {
        self.error.is_none()
            && self.threads_identical != Some(false)
            && self.checks.iter().all(|c| c.passed)
    }
}

/// 照合 1 件の結果
#[derive(Debug, Clone)]
pub struct Report {
    pub name: String,
    pub path: PathBuf,
    pub source: Source,
    pub plugin: String,
    pub runs: Vec<RunReport>,
    pub error: Option<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.error.is_none() && self.runs.iter().all(RunReport::passed)
    }

    /// 量ごとに全ての呼び出しをまとめた (名前, 差の最大, 相対の最大, 許容差, 通ったか)
    pub fn summary(&self) -> Vec<(String, f64, f64, Tol, bool)> {
        let mut m: BTreeMap<String, (f64, f64, Tol, bool)> = BTreeMap::new();
        for r in &self.runs {
            for c in &r.checks {
                let e = m
                    .entry(c.name.clone())
                    .or_insert((0.0, 0.0, c.tol.clone(), true));
                e.0 = e.0.max(c.max_abs);
                e.1 = e.1.max(c.max_rel);
                e.3 &= c.passed;
            }
        }
        m.into_iter()
            .map(|(k, (a, r, t, p))| (k, a, r, t, p))
            .collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "path": self.path.display().to_string(),
            "plugin": self.plugin,
            "source": {"by": self.source.by, "commit": self.source.commit, "how": self.source.how, "date": self.source.date},
            "passed": self.passed(),
            "error": self.error,
            "summary": self.summary().iter().map(|(k, a, r, t, p)| json!({
                "name": k, "max_abs": fin(*a), "max_rel": fin(*r), "tol": t.to_json(), "passed": p,
            })).collect::<Vec<_>>(),
            "runs": self.runs.iter().map(|r| json!({
                "label": r.label,
                "passed": r.passed(),
                "error": r.error,
                "threads_identical": r.threads_identical,
                "checks": r.checks.iter().map(CheckResult::to_json).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    /// 人が読む表
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "{} {}（{}、基準: {}{}）\n",
            if self.passed() { "OK  " } else { "FAIL" },
            self.name,
            self.plugin,
            self.source.by,
            if self.source.commit.is_empty() {
                String::new()
            } else {
                format!(" @ {}", self.source.commit)
            }
        );
        if let Some(e) = &self.error {
            s += &format!("  誤り: {e}\n");
        }
        for (k, a, r, t, p) in self.summary() {
            let tt = match (t.exact, t.abs, t.rel) {
                (true, _, _) => "ビット同じ".to_string(),
                (_, Some(a), Some(r)) => format!("abs {a:e} か rel {r:e}"),
                (_, Some(a), None) => format!("abs {a:e}"),
                (_, None, Some(r)) => format!("rel {r:e}"),
                _ => String::new(),
            };
            s += &format!(
                "  {} {k}: 差 {a:.3e}（相対 {r:.3e}）/ 許容 {tt}{}\n",
                if p { "ok" } else { "NG" },
                if t.note.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", t.note)
                }
            );
        }
        for r in &self.runs {
            if let Some(e) = &r.error {
                s += &format!("  [{}] 誤り: {e}\n", r.label);
            }
            if r.threads_identical == Some(false) {
                s += &format!("  [{}] スレッド数で出力が違う\n", r.label);
            }
            for c in r.checks.iter().filter(|c| !c.passed) {
                s += &format!(
                    "  [{}] {}: {}差 {:.3e} の要素 {:?}、片方だけ値でない {}\n",
                    r.label,
                    c.name,
                    c.error
                        .as_ref()
                        .map(|e| format!("{e}、"))
                        .unwrap_or_default(),
                    c.max_abs,
                    c.worst,
                    c.mismatched
                );
            }
        }
        s
    }
}

/// パス（ファイルかフォルダ。フォルダなら中の `*.verify.json` を下まで、名前の順）の記述を集める
pub fn collect(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let mut es: Vec<PathBuf> = std::fs::read_dir(p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        es.sort();
        for e in es {
            if e.is_dir() {
                walk(&e, out)?;
            } else if e
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".verify.json"))
            {
                out.push(e);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut out)?;
        } else {
            out.push(p.clone());
        }
    }
    Ok(out)
}

/// 照合の実行の選び方
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// ci: false の照合を飛ばす
    pub ci_only: bool,
}

/// 記述を集めて回す（記述が読めなければその照合を誤りにする）。飛ばした照合は返さない
pub fn run_paths(paths: &[PathBuf], reg: &Registry, opt: &Options) -> Result<Vec<Report>, String> {
    let mut out = Vec::new();
    for p in collect(paths)? {
        match Manifest::load(&p) {
            Ok(m) => {
                if opt.ci_only && !m.ci {
                    continue;
                }
                out.push(m.run(reg));
            }
            Err(e) => out.push(Report {
                name: p.display().to_string(),
                path: p.clone(),
                source: Source::default(),
                plugin: String::new(),
                runs: Vec::new(),
                error: Some(e),
            }),
        }
    }
    Ok(out)
}

/// 同梱のプラグイン（とプロセスで共通の登録）で回す
pub fn run_builtin(paths: &[PathBuf], opt: &Options) -> Result<Vec<Report>, String> {
    run_paths(paths, &lensbench_project::plugins::builtin(), opt)
}

/// Rust の試験から: フォルダの照合を全部回し、通らなければ表を出して panic する
pub fn assert_dir(dir: &Path, reg: &Registry) {
    let reps = run_paths(&[dir.to_path_buf()], reg, &Options::default()).unwrap();
    assert!(!reps.is_empty(), "{} に照合がない", dir.display());
    let text: String = reps.iter().map(Report::to_text).collect();
    assert!(reps.iter().all(Report::passed), "{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(shape: Vec<usize>, v: Vec<f64>) -> Array {
        Array::f64(shape, v).unwrap()
    }

    #[test]
    fn abs_rel_exact_and_nan() {
        let e = f(vec![3], vec![1.0, 2.0, f64::NAN]);
        let a = f(vec![3], vec![1.0, 2.0 + 1e-9, f64::NAN]);
        let t = |abs: Option<f64>, rel: Option<f64>| Tol {
            abs,
            rel,
            ..Default::default()
        };
        assert!(compare_one("x", Some(&a), &e, &t(Some(2e-9), None)).passed);
        assert!(!compare_one("x", Some(&a), &e, &t(Some(5e-10), None)).passed);
        // 相対は基準の最大（2）に対して: 1e-9 / 2
        let r = compare_one("x", Some(&a), &e, &t(None, Some(6e-10)));
        assert!(r.passed && (r.max_rel - 0.5e-9).abs() < 1e-15 && r.worst == Some(1));
        let ex = Tol {
            exact: true,
            ..Default::default()
        };
        assert!(!compare_one("x", Some(&a), &e, &ex).passed);
        assert!(compare_one("x", Some(&e), &e, &ex).passed);
        // 片方だけ NaN（けられ）は許容差によらず不一致
        let b = f(vec![3], vec![1.0, 2.0, 3.0]);
        let r = compare_one("x", Some(&b), &e, &t(Some(1e9), None));
        assert!(!r.passed && r.mismatched == 1);
        // 形の違い・出力にない
        assert!(compare_one(
            "x",
            Some(&f(vec![2], vec![1.0, 2.0])),
            &e,
            &t(Some(1.0), None)
        )
        .error
        .is_some());
        assert!(compare_one("x", None, &e, &t(Some(1.0), None))
            .error
            .is_some());
        // 形 [] と [1] は同じ扱い
        assert!(compare_one("x", Some(&Array::scalar(1.0)), &f(vec![1], vec![1.0]), &ex).passed);
    }

    #[test]
    fn complex_and_bool() {
        let c = |v: Vec<(f64, f64)>| {
            Array::c64(
                vec![v.len()],
                v.into_iter().map(|(a, b)| C64::new(a, b)).collect(),
            )
            .unwrap()
        };
        let e = c(vec![(1.0, 0.0), (0.0, 2.0)]);
        let a = c(vec![(1.0, 0.0), (0.0, 2.0 + 2e-9)]);
        let tol = Tol {
            rel: Some(1.1e-9),
            ..Default::default()
        };
        assert!(compare_one("j", Some(&a), &e, &tol).passed);
        // 値でない成分は f64 と同じ約束: NaN 同士・同じ Inf は一致、NaN と Inf は不一致
        let (nan, inf) = (f64::NAN, f64::INFINITY);
        let n1 = c(vec![(nan, 0.0), (inf, 1.0)]);
        assert!(compare_one("j", Some(&n1), &n1, &tol).passed);
        let n2 = c(vec![(inf, 0.0), (inf, 1.0)]);
        let r = compare_one("j", Some(&n2), &n1, &tol);
        assert!(!r.passed && r.mismatched == 1);
        let n3 = c(vec![(nan, 0.0), (-inf, 1.0)]);
        let r = compare_one("j", Some(&n3), &n1, &tol);
        assert!(!r.passed && r.mismatched == 1);
        let b1 = Array::bool(vec![2], vec![true, false]).unwrap();
        let b2 = Array::bool(vec![2], vec![true, true]).unwrap();
        let r = compare_one("ok", Some(&b2), &b1, &tol);
        assert!(!r.passed && r.mismatched == 1);
        assert!(compare_one("ok", Some(&b1), &b1, &tol).passed);
        assert!(compare_one("ok", Some(&a), &b1, &tol).error.is_some());
    }

    #[test]
    fn json_arrays_round_trip() {
        for v in [
            json!(1.5),
            json!([[1.0, 2.0], [3.0, null]]),
            json!([true, false]),
            json!({"re": [1.0, 2.0], "im": [0.5, "inf"]}),
            json!([]),
        ] {
            let a = array_from_json(&v).unwrap();
            assert_eq!(array_to_json(&a), v, "{v}");
        }
        assert_eq!(
            array_from_json(&json!([[1.0], [2.0]])).unwrap().shape,
            vec![2, 1]
        );
        assert!(array_from_json(&json!([[1.0], [2.0, 3.0]])).is_err());
        assert!(array_from_json(&json!([[1.0], 2.0])).is_err());
        assert!(array_from_json(&json!(["a"])).is_err());
    }

    #[test]
    fn every_expected_needs_a_check() {
        let e = Arrays::from([
            ("a".into(), Array::scalar(1.0)),
            ("b".into(), Array::scalar(1.0)),
        ]);
        let checks = BTreeMap::from([(
            "a".to_string(),
            Tol {
                exact: true,
                ..Default::default()
            },
        )]);
        let err = compare(&e, &e, &checks).unwrap_err();
        assert!(err.contains('b'), "{err}");
    }
}
