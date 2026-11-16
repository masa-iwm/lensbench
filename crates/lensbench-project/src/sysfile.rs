//! 系ファイル: 順次の面の列の系（[`System`]）を保存・読込する lensbench 独自の JSON。
//!
//! ```json
//! {"format": "lensbench-system", "format_version": 2, "lensbench": "0.1.0",
//!  "hash": "sha256:…", "coatings": ["膜の表.json"], "system": { … }}
//! ```
//!
//! - `system` は処方（利用者が書いた値だけ、[`crate::sysfile_v2::Prescription`]）。形は core の内部の構造体と
//!   切り離して決めてある。読み直した系の追跡は元の系とビット一致する（屈折率・自動の半直径・ピックアップ・ソルブは
//!   読んだ後に求める）。
//! - ガラスは名前だけを書き、`system.catalogs` に使ったカタログの名前とファイルの SHA-256 を残す。読むときは
//!   カタログをフォルダから探して今のカタログの値で追い、hash が違えば注意を返す。
//! - JSON に書けない数（無限遠の物体距離の inf、NaN）は `{"$f": "inf"}`・`{"$f": "-inf"}`・`{"$f": "nan"}`。
//!   実数の欄は整数の書き方でも読め、誤りには欄の道が付く（[`crate::lenient`]）。
//! - `hash` は `system` の部分（キーを名前の順にした空白なしの JSON）の SHA-256。解析の結果に刻んで、後から同じ系か確かめる。
//!   読むときに合わなければ（手で直したなど）注意を返す。
//! - `coatings` は膜の実測表・モデルのファイル（相対パスは系ファイルのフォルダから）。系の `coating_file` は系の中。
//! - 版 1（core の System の serde をそのまま書いた形）は読めるが、書くのはいつも版 2。

use lensbench_core::System;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 系ファイルの `format` の値
pub const FORMAT: &str = "lensbench-system";
/// 書く版（読めるのはこれ以下。版 1 も読める）
pub const FORMAT_VERSION: u64 = 2;

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

pub(crate) fn to_json(v: ciborium::value::Value) -> lensbench_core::Result<Value> {
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
                return Err(format!("integer {i} is too large").into());
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
                    return Err(format!("a table key is not a string ({k:?})").into());
                };
                o.insert(k, to_json(v)?);
            }
            Value::Object(o)
        }
        C::Tag(_, v) => to_json(*v)?,
        C::Bytes(_) => return Err("bytes can't be written".into()),
        _ => return Err("a value that can't be written".into()),
    })
}

pub(crate) fn from_json(v: Value) -> lensbench_core::Result<ciborium::value::Value> {
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
                C::Float(n.as_f64().ok_or("number")?)
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
                        _ => return Err(format!("number {{\"$f\": {s:?}}}").into()),
                    }));
                }
            }
            C::Map(
                m.into_iter()
                    .map(|(k, v)| Ok((C::Text(k), from_json(v)?)))
                    .collect::<lensbench_core::Result<_>>()?,
            )
        }
    })
}

/// core の System の serde をそのまま JSON にした値（版 1 の `system`。ガラスの表は名前の順）。系どうしを
/// 比べる（[`crate::same_system`]）・版 1 の hash を確かめるのに使う
pub(crate) fn internal_value(s: &System) -> lensbench_core::Result<Value> {
    let c =
        ciborium::value::Value::serialized(s).map_err(|e| format!("writing the system: {e}"))?;
    let mut v = to_json(c)?;
    if let Some(Value::Object(g)) = v.get_mut("glasses") {
        let mut items: Vec<(String, Value)> = std::mem::take(g).into_iter().collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        g.extend(items);
    }
    Ok(v)
}

/// 系ファイルの `system` の JSON の値（処方）
fn system_value(s: &System) -> lensbench_core::Result<Value> {
    let p = crate::sysfile_v2::write::prescription(s)?;
    let c =
        ciborium::value::Value::serialized(&p).map_err(|e| format!("writing the system: {e}"))?;
    to_json(c)
}

/// 系のガラスの元のカタログのファイル（このプロセスで読んだ AGF・既定のフォルダから、系のガラスと値が同じもの）。
/// 系ファイルの JSON を別のプロセスに渡すとき（Python の pickle）に、そのフォルダを glass_dirs に渡す
pub fn catalog_files(s: &System) -> Vec<PathBuf> {
    crate::sysfile_v2::catalogs::files(s)
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
pub fn system_hash(s: &System) -> lensbench_core::Result<String> {
    Ok(hash_value(&system_value(s)?))
}

/// 系を系ファイルの JSON（整形した文字列）にする
pub fn to_string(s: &System, coatings: &[PathBuf]) -> lensbench_core::Result<String> {
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
    // キーは名前の順（serde_json の preserve_order の有無によらず同じ文字列）
    Ok(crate::schema::to_text(&Value::Object(o)))
}

/// 系ファイルの JSON を読む（ガラスのカタログは既定のフォルダとこのプロセスで読んだ AGF から探す）。
/// 返すのは (系, 中身, 注意)。注意は hash が合わない・カタログが保存時と違うなど
pub fn from_str(txt: &str) -> lensbench_core::Result<(System, Meta, Vec<String>)> {
    from_str_with(txt, &[])
}

/// [`from_str`] で、ガラスのカタログをフォルダ glass_dirs から先に探す（その後に既定のフォルダ・このプロセスで
/// 読んだ AGF）
pub fn from_str_with(
    txt: &str,
    glass_dirs: &[PathBuf],
) -> lensbench_core::Result<(System, Meta, Vec<String>)> {
    let v: Value = serde_json::from_str(txt).map_err(|e| format!("system file: {e}"))?;
    let Value::Object(mut o) = v else {
        return Err("system file: not a JSON object".into());
    };
    if o.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(format!("system file: format is not {FORMAT}").into());
    }
    let ver = o
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or("system file: no format_version")?;
    if ver > FORMAT_VERSION {
        return Err(format!(
            "system file: version {ver} can't be read (up to {FORMAT_VERSION}; update lensbench)"
        )
        .into());
    }
    let sv = o.remove("system").ok_or("system file: no system")?;
    let mut warn = Vec::new();
    let hash_now = hash_value(&sv);
    let hash = o
        .get("hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 版 1 は面ごとに屈折率を持つ（分散のない媒質の値に使う）
    let v1_n = (ver < 2).then(|| v1_indices(&sv));
    let mut sv = sv;
    if ver < 2 {
        v1_solves(&mut sv)?;
    }
    let c = from_json(sv).map_err(|e| format!("system file: {e}"))?;
    let (sys, hash_back) = if ver >= 2 {
        let p: crate::sysfile_v2::Prescription =
            crate::lenient::decode(c, "system").map_err(|e| format!("system file: {e}"))?;
        let (g, notes) = crate::sysfile_v2::catalogs::load(
            &p.catalogs,
            &lensbench_core::glass::catalog_dirs(glass_dirs),
        )
        .map_err(|e| e.at("system file"))?;
        warn.extend(notes);
        let sys = crate::sysfile_v2::read::system(p, std::sync::Arc::new(g))
            .map_err(|e| format!("system file: {e}"))?;
        check(&sys)?;
        let h = system_hash(&sys)?;
        (sys, h)
    } else {
        let mut p: lensbench_core::Prescription =
            crate::lenient::decode(c, "system").map_err(|e| format!("system file: {e}"))?;
        v1_flat(&mut p, &v1_n.unwrap_or_default());
        let sys = p.build_raw().map_err(|e| format!("system file: {e}"))?;
        check(&sys)?;
        let h = hash_value(&internal_value(&sys)?);
        (sys, h)
    };
    // hash は読み直した系から作り直したものとも比べる（数の書き方の違いでは値が変わる）
    let hash_sys = system_hash(&sys)?;
    if !hash.is_empty() && hash != hash_now && hash != hash_back {
        warn.push(format!(
            "the hash of the system file doesn't match its content (edited after writing? recorded {hash}, now {hash_sys})"
        ));
    }
    let coatings = match o.get("coatings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "system file: coatings must be a list of strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("system file: coatings must be a list of strings".into()),
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

/// 版 1 のソルブ（`{"surf", "height", "kind": "Edge" か {"Edge": r}}`）を今の形（種類ごとに値を持つ）に直す
fn v1_solves(sv: &mut Value) -> Result<(), String> {
    let Some(qs) = sv.get_mut("solves").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for q in qs.iter_mut() {
        let surf = q.get("surf").and_then(Value::as_u64).unwrap_or(0) as usize;
        let h = q.get("height").and_then(Value::as_f64).unwrap_or(0.0);
        let (name, arg) = match q.get("kind") {
            None | Some(Value::Null) => ("MarginalHeight".to_string(), None),
            Some(Value::String(s)) => (s.clone(), None),
            Some(Value::Object(m)) if m.len() == 1 => {
                let (k, v) = m.iter().next().expect("one entry");
                (k.clone(), v.as_f64())
            }
            Some(k) => return Err(format!("system file: solve kind {k}")),
        };
        let snake: String = name
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                let lower = c.to_ascii_lowercase();
                let sep = (i > 0 && c.is_ascii_uppercase()).then_some('_');
                sep.into_iter().chain(std::iter::once(lower))
            })
            .collect();
        let snake = if snake == "f_number" || snake == "fnumber" {
            "f_number".to_string()
        } else {
            snake
        };
        let refers = matches!(
            snake.as_str(),
            "position"
                | "compensator"
                | "center_of_curvature"
                | "concentric_surface"
                | "concentric_radius"
        );
        let a = arg.unwrap_or(0.0);
        let (param, other) = if refers {
            (0.0, Some(a as usize))
        } else {
            (a, None)
        };
        let kind = lensbench_core::solves::solve_kind_parse(&snake, h, param, other)
            .map_err(|e| format!("system file: {e}"))?;
        *q = serde_json::to_value(lensbench_core::solves::Solve { surf, kind })
            .map_err(|e| format!("system file: {e}"))?;
    }
    Ok(())
}

/// 版 1 の面ごとの屈折率（ずれをかける前の値、n_base が無ければ n）
fn v1_indices(sv: &Value) -> Vec<Option<Vec<f64>>> {
    let nums = |v: Option<&Value>| -> Option<Vec<f64>> {
        let a = v?.as_array()?;
        (!a.is_empty())
            .then(|| a.iter().map(|x| x.as_f64()).collect::<Option<Vec<f64>>>())
            .flatten()
    };
    sv.get("surfaces")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|s| nums(s.get("n_base")).or_else(|| nums(s.get("n"))))
                .collect()
        })
        .unwrap_or_default()
}

/// 版 1 で、名前から屈折率が決まらない分散のない媒質（モデルガラス "nd,0"・カタログに無い名前で波長によらない値）は
/// 書いてあった値の屈折率だけの媒質にする
fn v1_flat(p: &mut lensbench_core::Prescription, ns: &[Option<Vec<f64>>]) {
    for j in 0..p.surfaces.len() {
        let Some(Some(n)) = ns.get(j) else { continue };
        let d = &p.surfaces[j];
        if (j > 0 && (d.is_mirror() || d.is_coord_break())) || d.grin.is_some() {
            continue;
        }
        let n0 = n[0];
        if n.iter().any(|&x| x.to_bits() != n0.to_bits()) {
            continue;
        }
        let model0 = matches!(d.mat, lensbench_core::surface::Material::Index(_));
        if model0 || p.material_index_at(&d.mat, &p.wavelengths).is_err() {
            p.surfaces[j].mat = lensbench_core::surface::Material::Index(n0);
        }
    }
}

/// 壊れた・手で直した系で追跡が panic しないよう、配列の長さと系の約束を確かめる
pub fn check(sys: &System) -> lensbench_core::Result<()> {
    let nw = sys.wavelengths.len();
    if nw == 0 || sys.weights.len() != nw || sys.primary >= nw {
        return Err(
            "system file: the numbers of wavelengths, weights and primary don't match".into(),
        );
    }
    check_refs(sys)?;
    Ok(sys.check().map_err(|e| format!("system file: {e}"))?)
}

/// ピックアップ・ソルブ・マクロ・玉と群・大域座標の基準が範囲の中の面を指すか
pub(crate) fn check_refs(sys: &lensbench_core::Prescription) -> lensbench_core::Result<()> {
    let ns = sys.surfaces.len();
    for p in &sys.pickups {
        if p.src >= ns || p.dst >= ns {
            return Err(format!("system file: pickup surfaces ({} → {})", p.src, p.dst).into());
        }
    }
    for q in &sys.solves {
        if q.surf >= ns || q.kind.other_surface().is_some_and(|f| f >= ns) {
            return Err(format!("system file: solve surface {}", q.surf).into());
        }
        if let lensbench_core::solves::SolveKind::MacroCurv { index: i }
        | lensbench_core::solves::SolveKind::MacroThick { index: i } = q.kind
        {
            if i as usize >= sys.macros.len() {
                return Err(format!("system file: no macro {i} for surface {}", q.surf).into());
            }
        }
    }
    for q in &sys.macro_parms {
        if q.surf >= ns || q.id as usize >= sys.macros.len() {
            return Err(format!("system file: macro solve (surface {})", q.surf).into());
        }
    }
    for u in &sys.units {
        if u.spans.iter().any(|s| s[0] > s[1] || s[1] >= ns) {
            return Err(format!("system file: range of lens or group {}", u.name).into());
        }
    }
    if sys.glrs >= ns {
        return Err(format!(
            "system file: global coordinate reference surface {}",
            sys.glrs
        )
        .into());
    }
    Ok(())
}

/// 系ファイルに書く（coatings は書いたとおりに残す）
pub fn save(path: &Path, s: &System, coatings: &[PathBuf]) -> lensbench_core::Result<()> {
    let txt = to_string(s, coatings)?;
    lensbench_core::vfs::write_atomic(path, txt)
        .map_err(lensbench_core::Error::io_at(path.display()))
}

/// 系を系ファイルに書くときの注意（[`save`]・[`to_string`] の後に表に出す）: 系のガラスと値が同じカタログのファイルが
/// 見つからず、`catalogs` に hash を書けないカタログ（読むときに値を確かめられない）
pub fn save_notes(s: &System) -> Vec<String> {
    crate::sysfile_v2::catalogs::unverified(s)
}

/// 系ファイルを読む。coatings の相対パスはファイルのフォルダからにする。ガラスのカタログはファイルのフォルダ・
/// 既定のフォルダ・このプロセスで読んだ AGF から探す
pub fn load(path: &Path) -> lensbench_core::Result<(System, Meta, Vec<String>)> {
    load_with(path, &[])
}

/// [`load`] で、ガラスのカタログをフォルダ glass_dirs から先に探す
pub fn load_with(
    path: &Path,
    glass_dirs: &[PathBuf],
) -> lensbench_core::Result<(System, Meta, Vec<String>)> {
    let txt = lensbench_core::vfs::read_to_string(path)
        .map_err(lensbench_core::Error::io_at(path.display()))?;
    // "a.json" の parent は "" なので、カタログは "." から探す
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut dirs = glass_dirs.to_vec();
    dirs.push(base.to_path_buf());
    let (s, mut m, w) = from_str_with(&txt, &dirs).map_err(|e| e.at(path.display()))?;
    for c in m.coatings.iter_mut() {
        if c.is_relative() {
            *c = base.join(&*c);
        }
    }
    Ok((s, m, w))
}
