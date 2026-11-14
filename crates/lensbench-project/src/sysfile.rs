//! 系ファイル: 順次の面の列の系（[`System`]）を欠けなく保存・読込する lensbench 独自の JSON。
//!
//! ```json
//! {"format": "lensbench-system", "format_version": 1, "lensbench": "0.1.0",
//!  "hash": "sha256:…", "coatings": ["膜の表.json"], "system": { … }}
//! ```
//!
//! - `system` は [`System`] の serde（面・ガラスのデータ・パラメータの変更の結果・ピックアップ・ソルブ・玉と群・
//!   多重構成の表・モデルガラスの dn・disp）。読み直した系の追跡は元の系とビット一致する。
//! - JSON に書けない数（無限遠の物体距離の inf、NaN）は `{"$f": "inf"}`・`{"$f": "-inf"}`・`{"$f": "nan"}`。
//! - ガラスの表は名前の順に並べる（同じ系なら同じファイル・同じ hash になる）。
//! - `hash` は `system` の部分（キーを名前の順にした空白なしの JSON）の SHA-256。解析の結果に刻んで、後から同じ系か確かめる。
//!   読むときに合わなければ（手で直したなど）注意を返す。
//! - `coatings` は膜の実測表・モデルのファイル（相対パスは系ファイルのフォルダから）。系の `coating_file` は系の中。

use lensbench_core::System;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 系ファイルの `format` の値
pub const FORMAT: &str = "lensbench-system";
/// 書く版（読めるのはこれ以下）
pub const FORMAT_VERSION: u64 = 1;

/// 系ファイルの中身（系以外）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    /// 書いた lensbench の版
    pub lensbench: String,
    /// 系の hash（"sha256:…"）
    pub hash: String,
    /// 膜の実測表・モデルのファイル
    pub coatings: Vec<PathBuf>,
}

/// JSON の文字列が系ファイルか（`format` を見る）
pub fn is_system_json(txt: &str) -> bool {
    // 先頭だけ見て決めずに、きちんと読む（大きな系でも 1 回読むだけ）
    match serde_json::from_str::<Value>(txt) {
        Ok(Value::Object(m)) => m.get("format").and_then(Value::as_str) == Some(FORMAT),
        _ => false,
    }
}

/// ファイルが系ファイルか（拡張子 .json で、`format` が系ファイル）
pub fn is_system_file(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        return false;
    }
    lensbench_core::vfs::read_to_string(path).is_ok_and(|t| is_system_json(&t))
}

pub(crate) fn to_json(v: ciborium::value::Value) -> Result<Value, String> {
    use ciborium::value::Value as C;
    Ok(match v {
        C::Null => Value::Null,
        C::Bool(b) => Value::Bool(b),
        C::Integer(i) => {
            let i: i128 = i.into();
            if let Ok(u) = u64::try_from(i) {
                Value::from(u)
            } else if let Ok(s) = i64::try_from(i) {
                Value::from(s)
            } else {
                return Err(format!("整数 {i} が大きすぎる"));
            }
        }
        C::Float(f) => {
            if f.is_finite() {
                Value::from(f)
            } else {
                let s = if f.is_nan() {
                    "nan"
                } else if f > 0.0 {
                    "inf"
                } else {
                    "-inf"
                };
                let mut m = Map::new();
                m.insert("$f".into(), Value::from(s));
                Value::Object(m)
            }
        }
        C::Text(s) => Value::String(s),
        C::Array(a) => Value::Array(a.into_iter().map(to_json).collect::<Result<_, _>>()?),
        C::Map(m) => {
            let mut o = Map::new();
            for (k, v) in m {
                let C::Text(k) = k else {
                    return Err(format!("表のキーが文字列でない（{k:?}）"));
                };
                o.insert(k, to_json(v)?);
            }
            Value::Object(o)
        }
        C::Tag(_, v) => to_json(*v)?,
        C::Bytes(_) => return Err("バイト列は書けない".into()),
        _ => return Err("書けない値".into()),
    })
}

pub(crate) fn from_json(v: Value) -> Result<ciborium::value::Value, String> {
    use ciborium::value::Value as C;
    Ok(match v {
        Value::Null => C::Null,
        Value::Bool(b) => C::Bool(b),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                C::Integer(u.into())
            } else if let Some(i) = n.as_i64() {
                C::Integer(i.into())
            } else {
                C::Float(n.as_f64().ok_or("数")?)
            }
        }
        Value::String(s) => C::Text(s),
        Value::Array(a) => C::Array(a.into_iter().map(from_json).collect::<Result<_, _>>()?),
        Value::Object(m) => {
            if m.len() == 1 {
                if let Some(Value::String(s)) = m.get("$f") {
                    return Ok(C::Float(match s.as_str() {
                        "inf" => f64::INFINITY,
                        "-inf" => f64::NEG_INFINITY,
                        "nan" => f64::NAN,
                        _ => return Err(format!("数 {{\"$f\": {s:?}}}")),
                    }));
                }
            }
            C::Map(
                m.into_iter()
                    .map(|(k, v)| Ok((C::Text(k), from_json(v)?)))
                    .collect::<Result<_, String>>()?,
            )
        }
    })
}

/// 系の部分の JSON の値（ガラスの表は名前の順）
fn system_value(s: &System) -> Result<Value, String> {
    let c = ciborium::value::Value::serialized(s).map_err(|e| format!("系の書き出し: {e}"))?;
    let mut v = to_json(c)?;
    if let Some(Value::Object(g)) = v.get_mut("glasses") {
        let mut items: Vec<(String, Value)> = std::mem::take(g).into_iter().collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        g.extend(items);
    }
    Ok(v)
}

/// キーを名前の順に並べ、空白なしで書いた JSON（serde_json の preserve_order の有無によらず同じ）
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).expect("JSON"));
                out.push(':');
                canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        x => out.push_str(&serde_json::to_string(x).expect("JSON")),
    }
}

pub fn hash_value(v: &Value) -> String {
    let mut txt = String::new();
    canonical(v, &mut txt);
    let d = Sha256::digest(txt.as_bytes());
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// 系の hash（"sha256:…"、系ファイルの `hash` と同じ）。同じ系なら同じ値
pub fn system_hash(s: &System) -> Result<String, String> {
    Ok(hash_value(&system_value(s)?))
}

/// 系を系ファイルの JSON（整形した文字列）にする
pub fn to_string(s: &System, coatings: &[PathBuf]) -> Result<String, String> {
    let sv = system_value(s)?;
    let hash = hash_value(&sv);
    let mut o = Map::new();
    o.insert("format".into(), Value::from(FORMAT));
    o.insert("format_version".into(), Value::from(FORMAT_VERSION));
    o.insert("lensbench".into(), Value::from(lensbench_core::VERSION));
    o.insert("hash".into(), Value::from(hash));
    o.insert(
        "coatings".into(),
        Value::Array(
            coatings
                .iter()
                .map(|p| Value::from(p.to_string_lossy().to_string()))
                .collect(),
        ),
    );
    o.insert("system".into(), sv);
    Ok(serde_json::to_string_pretty(&Value::Object(o)).expect("JSON") + "\n")
}

/// 系ファイルの JSON を読む。返すのは (系, 中身, 注意)。注意は hash が合わない・新しい版が書いたなど
pub fn from_str(txt: &str) -> Result<(System, Meta, Vec<String>), String> {
    let v: Value = serde_json::from_str(txt).map_err(|e| format!("系ファイル: {e}"))?;
    let Value::Object(mut o) = v else {
        return Err("系ファイル: JSON のオブジェクトでない".into());
    };
    if o.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(format!("系ファイル: format が {FORMAT} でない"));
    }
    let ver = o
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or("系ファイル: format_version がない")?;
    if ver > FORMAT_VERSION {
        return Err(format!(
            "系ファイル: 版 {ver} は読めない（{FORMAT_VERSION} まで、lensbench を新しくする）"
        ));
    }
    let sv = o.remove("system").ok_or("系ファイル: system がない")?;
    let mut warn = Vec::new();
    let hash_now = hash_value(&sv);
    let hash = o
        .get("hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let c = from_json(sv).map_err(|e| format!("系ファイル: {e}"))?;
    let sys: System =
        crate::lenient::decode(c, "system").map_err(|e| format!("系ファイル: {e}"))?;
    check(&sys)?;
    // hash は読み直した系から作り直す（数の書き方の違い・手での直しでは値が変わる）
    let hash_sys = system_hash(&sys)?;
    if !hash.is_empty() && hash != hash_now && hash != hash_sys {
        warn.push(format!(
            "系ファイルの hash が中身と合わない（書いた後に直した？ 記録 {hash}、今 {hash_sys}）"
        ));
    }
    let coatings = match o.get("coatings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "系ファイル: coatings は文字列の並び".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("系ファイル: coatings は文字列の並び".into()),
    };
    let meta = Meta {
        lensbench: o
            .get("lensbench")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        hash: hash_sys,
        coatings,
    };
    Ok((sys, meta, warn))
}

/// 壊れた・手で直した系で追跡が panic しないよう、配列の長さと系の約束を確かめる
pub fn check(sys: &System) -> Result<(), String> {
    let nw = sys.wavelengths.len();
    if nw == 0 || sys.weights.len() != nw || sys.primary >= nw {
        return Err("系ファイル: 波長・重み・主波長の数が合わない".into());
    }
    for (j, d) in sys.surfaces.iter().enumerate() {
        if d.n.len() != nw {
            return Err(format!(
                "系ファイル: 面 {j} の屈折率の数 {}（波長 {nw}）",
                d.n.len()
            ));
        }
        if !d.n_base.is_empty() && d.n_base.len() != nw {
            return Err(format!("系ファイル: 面 {j} の n_base の数"));
        }
    }
    let ns = sys.surfaces.len();
    for p in &sys.pickups {
        if p.src >= ns || p.dst >= ns {
            return Err(format!(
                "系ファイル: ピックアップの面（{} → {}）",
                p.src, p.dst
            ));
        }
    }
    for q in &sys.solves {
        if q.surf >= ns || q.kind.other_surface().is_some_and(|f| f >= ns) {
            return Err(format!("系ファイル: ソルブの面 {}", q.surf));
        }
        if let lensbench_core::param::SolveKind::MacroCurv(i)
        | lensbench_core::param::SolveKind::MacroThick(i) = q.kind
        {
            if i as usize >= sys.macros.len() {
                return Err(format!("系ファイル: 面 {} のマクロ {i} がない", q.surf));
            }
        }
    }
    for q in &sys.macro_parms {
        if q.surf >= ns || q.id as usize >= sys.macros.len() {
            return Err(format!("系ファイル: マクロのソルブ（面 {}）", q.surf));
        }
    }
    for u in &sys.units {
        if u.spans.iter().any(|s| s[0] > s[1] || s[1] >= ns) {
            return Err(format!("系ファイル: 玉・群 {} の範囲", u.name));
        }
    }
    if sys.glrs >= ns {
        return Err(format!("系ファイル: 大域座標の基準面 {}", sys.glrs));
    }
    sys.check().map_err(|e| format!("系ファイル: {e}"))
}

/// 系ファイルに書く（coatings は書いたとおりに残す）
pub fn save(path: &Path, s: &System, coatings: &[PathBuf]) -> Result<(), String> {
    let txt = to_string(s, coatings)?;
    lensbench_core::vfs::write_atomic(path, txt).map_err(|e| format!("{}: {e}", path.display()))
}

/// 系ファイルを読む。coatings の相対パスはファイルのフォルダからにする
pub fn load(path: &Path) -> Result<(System, Meta, Vec<String>), String> {
    let txt = lensbench_core::vfs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let (s, mut m, w) = from_str(&txt).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    for c in m.coatings.iter_mut() {
        if c.is_relative() {
            *c = base.join(&*c);
        }
    }
    Ok((s, m, w))
}
