//! 系の編集を JSON の操作の列で行う（serve の `edit`、GUI から使う）。
//!
//! ```json
//! [{"op": "insert_surface", "at": 3, "surface": {"r": 50, "t": 5, "material": "N-BK7"}},
//!  {"op": "set_surface", "surface": "L1 front", "fields": {"semi": 8}},
//!  {"op": "add_pickup", "dst": 4, "src": 3, "what": "c", "scale": -1}]
//! ```
//!
//! 面は番号（OpticStudio の面番号）か名前（面のコメント）。操作は順に当て、どれかが失敗すれば系は変えない。
//! 操作と引数:
//! - `insert_surface` at, surface（面の値の名前と値、[`lensbench_core::edit::SURF_FIELDS`]）
//! - `delete_surface` surface ／ `set_surface` surface, fields ／ `set_surface_type` surface, type
//! - `set_material` surface, material ／ `set_stop` surface
//! - `add_pickup` dst, src, what（c r t k p1〜 dn disp semi before0〜4 after0〜4）, scale（既定 1）, offset（既定 0）,
//!   from（元の量、省くと what） ／ `remove_pickup` dst, what
//! - `add_solve` surface, kind（既定 marginal_height、[`lensbench_core::edit::SOLVE_KINDS`]）, value（既定 0）,
//!   other（別の面を参照する種類の面）, param（帯・縁の高さ） ／ `remove_solve` surface, what（t か c、既定 t）
//! - `set_surface_data` surface, data（面のデータを全部の項目で。[`surface_data`] と同じ形で、書いた項目だけ変える。
//!   屈折率 n・絞り stop・鏡 mirror は変えない（材料・set_stop で）。面タイプの形のデータ zern・grid・bin2・cylf・grin、
//!   面の前後の傾き偏心 before・after もここで）
//! - `set_param` param, value（パラメータ名は set と同じ）
//! - `set_system_data` data（系の設定の項目 [`SYSTEM_KEYS`] の書いた分だけ。形は [`system_data`] と同じ）
//! - `define_unit` name, spans（[[最初, 最後], …]）, pivot（既定 [0, 0, 0]） ／ `set_unit` unit（名前か番号）,
//!   name・spans・pivot（書いた分だけ） ／ `remove_unit` unit（玉・群、[`lensbench_core::units`]。範囲を変える・外すのは
//!   動き・移動が 0 のときだけ）
//! - `add_mech` part（機構部品、系ファイルの mechanics の要素と同じ形） ／ `set_mech` mech（名前か番号）, part（書いた項目
//!   だけ変える） ／ `remove_mech` mech
//! - `set_config_count` n ／ `set_config_row` param, values ／ `add_config_row` param（どの構成も今の値）／
//!   `remove_config_row` param ／ `set_config` k
//!
//! 値: 数・数の並び・文字列・真偽。aperture は {"type": "circular", "max": 3, …} か null。
//! JSON に書けない inf は {"$f": "inf"}（系ファイルと同じ）

use lensbench_core::edit::{pick_kind_parse, SurfValue};
use lensbench_core::param::{Kind, Pickup, ThickSolve};
use lensbench_core::surface::{Aperture, Surface};
use lensbench_core::System;
use serde_json::Value;

fn num(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        return Some(x);
    }
    match v.get("$f").and_then(Value::as_str) {
        Some("inf") => Some(f64::INFINITY),
        Some("-inf") => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

fn surf(s: &System, v: &Value, what: &str) -> Result<usize, String> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|j| j as usize)
            .filter(|&j| j < s.surfaces.len())
            .ok_or_else(|| format!("{what}: 面 {n}")),
        Value::String(t) => s.find_surface(t).map_err(|e| format!("{what}: {e}")),
        _ => Err(format!("{what} は面の番号か名前")),
    }
}

/// 玉・群の範囲 [[最初, 最後], …]
fn spans_of(v: &Value, what: &str) -> Result<Vec<[usize; 2]>, String> {
    let bad = || format!("{what} の spans は [[最初, 最後], …]");
    v.as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|x| match x.as_array().map(|a| a.as_slice()) {
            Some([a, b]) => Ok([
                a.as_u64().ok_or_else(bad)? as usize,
                b.as_u64().ok_or_else(bad)? as usize,
            ]),
            _ => Err(bad()),
        })
        .collect()
}

fn vec3(v: &Value, what: &str) -> Result<[f64; 3], String> {
    match v.as_array().map(|a| a.as_slice()) {
        Some([a, b, c]) => match (num(a), num(b), num(c)) {
            (Some(a), Some(b), Some(c)) => Ok([a, b, c]),
            _ => Err(format!("{what} の pivot は数 3 つ")),
        },
        _ => Err(format!("{what} の pivot は数 3 つ")),
    }
}

/// 玉・群（名前か、0 始まりの番号）
fn unit_of(s: &System, v: &Value, what: &str) -> Result<usize, String> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|i| i as usize)
            .filter(|&i| i < s.units.len())
            .ok_or_else(|| format!("{what}: 玉・群 {n}")),
        Value::String(t) => s
            .find_unit(t)
            .ok_or_else(|| format!("{what}: 玉・群 {t} が無い")),
        _ => Err(format!("{what} の unit は玉・群の名前か番号")),
    }
}

/// 機構部品（名前か、0 始まりの番号）
fn mech_of(s: &System, v: &Value, what: &str) -> Result<usize, String> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|i| i as usize)
            .filter(|&i| i < s.mechanics.len())
            .ok_or_else(|| format!("{what}: 機構部品 {n}")),
        Value::String(t) => s
            .mechanics
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(t.trim()))
            .ok_or_else(|| format!("{what}: 機構部品 {t} が無い")),
        _ => Err(format!("{what} の mech は機構部品の名前か番号")),
    }
}

/// 機構部品の JSON を読んで確かめる（名前・面の番号・寸法）
fn mech_from(s: &System, v: Value) -> Result<lensbench_core::mech::MechPart, String> {
    let m: lensbench_core::mech::MechPart =
        serde_json::from_value(v).map_err(|e| format!("機構部品: {e}"))?;
    let n = s.surfaces.len();
    if m.name.trim().is_empty() {
        return Err("機構部品の名前が空".into());
    }
    if m.surface == 0 || m.surface >= n {
        return Err(format!(
            "機構部品 {} の面 {}（1〜{}）",
            m.name,
            m.surface,
            n - 1
        ));
    }
    if let Some(h) = m.holds.iter().find(|&&h| h >= n) {
        return Err(format!("機構部品 {} の保持する面 {h}", m.name));
    }
    let fin = m.offset.iter().chain(&m.tilt).all(|x| x.is_finite())
        && [m.length, m.outer_radius, m.hold_radius, m.rod_spacing]
            .iter()
            .all(|x| x.is_finite() && *x >= 0.0);
    if !fin {
        return Err(format!("機構部品 {} の位置・寸法", m.name));
    }
    for b in &m.bores {
        if !(b.z0.is_finite() && b.z1.is_finite() && b.z0 <= b.z1 && b.radius > 0.0) {
            return Err(format!(
                "機構部品 {} の穴（z {}〜{}・半径 {}）",
                m.name, b.z0, b.z1, b.radius
            ));
        }
    }
    Ok(m)
}

/// スパイダーの腕の本数（0 以上の整数）
fn arms(n: f64) -> Result<u32, String> {
    if n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
        Ok(n as u32)
    } else {
        Err(format!("スパイダーの腕の本数 {n}"))
    }
}

fn aperture(v: &Value) -> Result<Aperture, String> {
    if v.is_null() {
        return Ok(Aperture::None);
    }
    let g = |k: &str| v.get(k).and_then(num).unwrap_or(0.0);
    let need = |k: &str| v.get(k).and_then(num).ok_or(format!("開口に {k} が要る"));
    let (dx, dy) = (g("dx"), g("dy"));
    let t = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or("開口に type が要る")?;
    Ok(match t.to_ascii_lowercase().as_str() {
        "none" => Aperture::None,
        "circular" => Aperture::Circular {
            min: g("min"),
            max: need("max")?,
            dx,
            dy,
        },
        "obsc" => Aperture::Obsc {
            min: g("min"),
            max: need("max")?,
            dx,
            dy,
        },
        "rect" => Aperture::Rect {
            hx: need("hx")?,
            hy: need("hy")?,
            dx,
            dy,
        },
        "ellipse" => Aperture::Ellipse {
            hx: need("hx")?,
            hy: need("hy")?,
            dx,
            dy,
        },
        "spider" => Aperture::Spider {
            width: need("width")?,
            arms: arms(need("arms")?)?,
            dx,
            dy,
        },
        "floating" => Aperture::Floating { semi: g("semi") },
        t => return Err(format!("開口の type {t}")),
    })
}

/// 面の値の dict を名前と値の組に
pub fn surf_fields(v: &Value) -> Result<Vec<(String, SurfValue)>, String> {
    let Some(m) = v.as_object() else {
        return Err("面の値は {名前: 値}".into());
    };
    let mut out = Vec::with_capacity(m.len());
    for (k, x) in m {
        let val = if k == "aperture" {
            SurfValue::Aperture(aperture(x)?)
        } else if let Some(f) = num(x) {
            SurfValue::Num(f)
        } else {
            match x {
                Value::Bool(b) => SurfValue::Bool(*b),
                Value::String(t) => SurfValue::Text(t.clone()),
                Value::Array(a) => SurfValue::Nums(
                    a.iter()
                        .map(|y| num(y).ok_or(format!("面の値 {k} は数の並び")))
                        .collect::<Result<_, _>>()?,
                ),
                _ => return Err(format!("面の値 {k} の型")),
            }
        };
        out.push((k.clone(), val));
    }
    Ok(out)
}

/// 面の値を JSON に（inf は {"$f": "inf"}）
pub fn surf_fields_json(s: &System, j: usize) -> Result<Value, String> {
    let f = s.surface_fields(j).map_err(|e| e.to_string())?;
    let n = |x: f64| -> Value {
        if x.is_finite() {
            Value::from(x)
        } else {
            serde_json::json!({ "$f": if x.is_nan() { "nan" } else if x > 0.0 { "inf" } else { "-inf" } })
        }
    };
    let mut o = serde_json::Map::new();
    for (k, v) in f {
        let x = match v {
            SurfValue::Num(x) => n(x),
            SurfValue::Nums(a) => Value::Array(a.into_iter().map(n).collect()),
            SurfValue::Text(t) => Value::from(t),
            SurfValue::Bool(b) => Value::from(b),
            SurfValue::Aperture(a) => match a {
                Aperture::None => Value::Null,
                Aperture::Circular { min, max, dx, dy } => {
                    serde_json::json!({"type": "circular", "min": min, "max": max, "dx": dx, "dy": dy})
                }
                Aperture::Obsc { min, max, dx, dy } => {
                    serde_json::json!({"type": "obsc", "min": min, "max": max, "dx": dx, "dy": dy})
                }
                Aperture::Rect { hx, hy, dx, dy } => {
                    serde_json::json!({"type": "rect", "hx": hx, "hy": hy, "dx": dx, "dy": dy})
                }
                Aperture::Ellipse { hx, hy, dx, dy } => {
                    serde_json::json!({"type": "ellipse", "hx": hx, "hy": hy, "dx": dx, "dy": dy})
                }
                Aperture::Spider {
                    width,
                    arms,
                    dx,
                    dy,
                } => {
                    serde_json::json!({"type": "spider", "width": width, "arms": arms, "dx": dx, "dy": dy})
                }
                Aperture::Floating { semi } => {
                    serde_json::json!({"type": "floating", "semi": semi})
                }
            },
        };
        o.insert(k.into(), x);
    }
    Ok(Value::Object(o))
}

/// 面 j のソルブ・ピックアップ（{"solves": [...], "pickups": [...]}。GUI の面の設定・面の表の印）
pub fn surface_solves(s: &System, j: usize) -> Value {
    let solves: Vec<Value> = s
        .solves
        .iter()
        .filter(|q| q.surf == j)
        .map(|q| {
            let (kind, param, other) = lensbench_core::edit::solve_kind_info(q.kind);
            serde_json::json!({
                "what": if q.kind.is_curv() { "c" } else { "t" },
                "kind": kind, "value": q.height, "param": param, "other": other,
            })
        })
        .collect();
    let pickups: Vec<Value> = s
        .pickups
        .iter()
        .filter(|p| p.dst == j)
        .map(|p| {
            serde_json::json!({
                "what": lensbench_core::edit::pick_kind_name(p.what),
                "src": p.src, "scale": p.scale, "offset": p.offset,
                "from": p.from.map(lensbench_core::edit::pick_kind_name),
            })
        })
        .collect();
    serde_json::json!({ "solves": solves, "pickups": pickups })
}

/// 面 j のデータを全部の項目で（系ファイルの面と同じ形。計算で作る屈折率 n・n_base・ゴーストの reflect は除く）と、
/// その面のソルブ・ピックアップ（GUI の面の設定）
pub fn surface_data(s: &System, j: usize) -> Result<Value, String> {
    let d = s
        .surfaces
        .get(j)
        .ok_or_else(|| format!("面 {j}（面は 0〜{}）", s.surfaces.len().saturating_sub(1)))?;
    let c = ciborium::value::Value::serialized(d).map_err(|e| e.to_string())?;
    let mut v = crate::sysfile::to_json(c)?;
    if let Some(o) = v.as_object_mut() {
        for k in ["n", "n_base", "reflect"] {
            o.remove(k);
        }
    }
    let sp = surface_solves(s, j);
    Ok(serde_json::json!({ "data": v, "solves": sp["solves"], "pickups": sp["pickups"] }))
}

/// JSON の整数を、面のデータの実数の所では実数にする（ciborium は整数から f64 を読まない）。今の値 tmpl の同じ所が
/// 整数なら整数のまま、今の値が無い所（null から作る形のデータ）は名前で決める
fn coerce(v: Value, tmpl: Option<&Value>, key: &str, in_bin2: bool) -> Value {
    match v {
        Value::Number(n) if n.is_i64() || n.is_u64() => {
            let int = match tmpl {
                Some(Value::Number(t)) => !t.is_f64(),
                _ => {
                    matches!(key, "nx" | "ny" | "cap" | "cb_order" | "opd_mode" | "mode")
                        || (key == "order" && !in_bin2)
                }
            };
            if int {
                Value::Number(n)
            } else {
                serde_json::json!(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::Array(a) => {
            let t = tmpl.and_then(Value::as_array);
            Value::Array(
                a.into_iter()
                    .enumerate()
                    .map(|(i, x)| {
                        // 並びの要素は同じ型: 今の値の最初の要素を手本に
                        let ti = t.and_then(|t| t.get(i).or(t.first()));
                        coerce(x, ti, key, in_bin2)
                    })
                    .collect(),
            )
        }
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, x)| {
                    let t = tmpl.and_then(|t| t.get(&k));
                    let b = in_bin2 || k == "bin2";
                    let x = if k == "$f" { x } else { coerce(x, t, &k, b) };
                    (k, x)
                })
                .collect(),
        ),
        x => x,
    }
}

/// 面 j のデータを data の項目で変える（[`surface_data`] の data と同じ形、書いた項目だけ）
pub(crate) fn set_surface_data(t: &mut System, j: usize, data: &Value) -> Result<(), String> {
    let Some(m) = data.as_object() else {
        return Err("data は {項目: 値}".into());
    };
    let cur = t.surfaces[j].clone();
    let c = ciborium::value::Value::serialized(&cur).map_err(|e| e.to_string())?;
    let mut v = crate::sysfile::to_json(c)?;
    let o = v.as_object_mut().ok_or("面のデータ")?;
    for (k, x) in m {
        if ["n", "n_base", "reflect", "stop", "mirror"].contains(&k.as_str()) {
            return Err(format!(
                "面の項目 {k} は data では変えない（材料・set_stop で）"
            ));
        }
        // user（利用者定義の面）と semi_autoは無ければ書き出さないので、無くても足せる
        if !o.contains_key(k) && !["user", "semi_auto", "bire"].contains(&k.as_str()) {
            return Err(format!("面の項目 {k} は無い"));
        }
        let x = coerce(x.clone(), o.get(k), k, k == "bin2");
        o.insert(k.clone(), x);
    }
    // 半径の値を書けば固定（semi_auto も書いたときはそちら）
    if m.contains_key("semi") && !m.contains_key("semi_auto") {
        o.insert("semi_auto".into(), Value::Bool(false));
    }
    let mut d: Surface = crate::sysfile::from_json(v)?
        .deserialized()
        .map_err(|e| format!("面のデータ: {e}"))?;
    d.mirror = d.mat.trim().eq_ignore_ascii_case("MIRROR");
    if d.mirror {
        d.mat = "MIRROR".into();
    }
    // 屈折率は材料・GRIN から作り直す（dn・disp も当て直す）
    d.n.clear();
    d.n_base.clear();
    t.edit(|s| {
        s.surfaces[j] = d;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 系の設定の項目（[`system_data`]・`set_system_data` で読み書きできるもの。系ファイルの同じ名前の項目と同じ形）
pub const SYSTEM_KEYS: &[&str] = &[
    "name",
    "wavelengths",
    "weights",
    "primary",
    "fields",
    "field_type",
    "field_norm",
    "aperture",
    "telecentric",
    "aiming",
    "afocal",
    "afocal_unit",
    "floating_clip",
    "temperature",
    "pressure",
    "adjust_index",
    "glrs",
    "ropd",
    "coating_file",
];

/// 系の設定（[`SYSTEM_KEYS`] の項目。系ファイルと同じ形、inf は {"$f": "inf"}。主波長 primary は 0 始まり）
pub fn system_data(s: &System) -> Result<Value, String> {
    let c = ciborium::value::Value::serialized(s).map_err(|e| e.to_string())?;
    let v = crate::sysfile::to_json(c)?;
    let o = v.as_object().ok_or("系のデータ")?;
    Ok(Value::Object(
        SYSTEM_KEYS
            .iter()
            .filter_map(|k| match (o.get(*k), *k) {
                (Some(x), _) => Some((k.to_string(), x.clone())),
                // 既定（ける）は系ファイルに書かない
                (None, "floating_clip") => Some((k.to_string(), Value::Bool(true))),
                _ => None,
            })
            .collect(),
    ))
}

/// 系の設定の項目を変える（`set_system_data`）。書いた項目だけ変え、屈折率は波長・温度・気圧から作り直す
pub(crate) fn set_system_data(t: &mut System, data: &Value) -> Result<(), String> {
    let Some(m) = data.as_object() else {
        return Err("data は {項目: 値}".into());
    };
    let c = ciborium::value::Value::serialized(&*t).map_err(|e| e.to_string())?;
    let mut v = crate::sysfile::to_json(c)?;
    let o = v.as_object_mut().ok_or("系のデータ")?;
    for (k, x) in m {
        if !SYSTEM_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "系の項目 {k} は set_system_data では変えない（{}）",
                SYSTEM_KEYS.join("・")
            ));
        }
        let x = coerce(x.clone(), o.get(k), k, false);
        o.insert(k.clone(), x);
    }
    let mut n: System = crate::sysfile::from_json(v)?
        .deserialized()
        .map_err(|e| format!("系のデータ: {e}"))?;
    let nw = n.wavelengths.len();
    if nw == 0 {
        return Err("波長が無い".into());
    }
    if n.wavelengths.iter().any(|&l| !(l.is_finite() && l > 0.0)) {
        return Err(format!("波長 {:?}", n.wavelengths));
    }
    if n.weights.len() != nw {
        return Err(format!("波長の重みの数 {}（波長 {nw}）", n.weights.len()));
    }
    if n.primary >= nw {
        return Err(format!("主波長 {}（0 始まり、波長 {nw}）", n.primary));
    }
    if n.fields.is_empty() {
        return Err("視野が無い".into());
    }
    if n.glrs >= n.surfaces.len() {
        return Err(format!("大域座標の基準面 {}", n.glrs));
    }
    if !(n.temperature.is_finite() && n.pressure.is_finite()) {
        return Err("温度・気圧は数".into());
    }
    // 屈折率は材料・GRIN から作り直す（波長・温度・気圧が変わるかもしれない）
    for d in &mut n.surfaces {
        d.n.clear();
        d.n_base.clear();
    }
    t.edit(|s| {
        *s = n;
        s.check()
    })
    .map_err(|e| e.to_string())
}

/// 操作の列を順に当てる（どれかが失敗すれば系は変えない）。面を消したときなどの説明を返す
pub fn apply(s: &mut System, ops: &Value) -> Result<Vec<String>, String> {
    let Some(list) = ops.as_array() else {
        return Err("ops は操作の並び".into());
    };
    let mut t = s.clone();
    let mut notes = Vec::new();
    for (i, op) in list.iter().enumerate() {
        let at = |e: String| format!("操作 {}: {e}", i + 1);
        apply_one(&mut t, op, &mut notes).map_err(at)?;
        // 表の操作でなければ、変えた値を表の今の構成の列にも
        let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
        if !name.contains("config") {
            t.sync_active_config();
        }
    }
    *s = t;
    Ok(notes)
}

fn apply_one(t: &mut System, op: &Value, notes: &mut Vec<String>) -> Result<(), String> {
    let e = |x: lensbench_core::Error| x.to_string();
    let name = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or("op（操作の名前）がない")?;
    let field = |k: &str| op.get(k).ok_or(format!("{name} には {k}"));
    let text = |k: &str| -> Result<String, String> {
        field(k)?
            .as_str()
            .map(str::to_string)
            .ok_or(format!("{name} の {k} は文字列"))
    };
    let count = |k: &str| -> Result<usize, String> {
        field(k)?
            .as_u64()
            .map(|x| x as usize)
            .ok_or(format!("{name} の {k} は 0 以上の整数"))
    };
    match name {
        "insert_surface" => {
            let at = count("at")?;
            let f = match op.get("surface") {
                Some(v) => surf_fields(v)?,
                None => Vec::new(),
            };
            t.insert_surface(at, Surface::default()).map_err(e)?;
            t.set_surface_fields(at, &f).map_err(e)?;
        }
        "delete_surface" => {
            let j = surf(t, field("surface")?, name)?;
            notes.extend(t.delete_surface(j).map_err(e)?);
        }
        "set_surface" => {
            let j = surf(t, field("surface")?, name)?;
            let f = surf_fields(field("fields")?)?;
            t.set_surface_fields(j, &f).map_err(e)?;
        }
        "set_surface_type" => {
            let j = surf(t, field("surface")?, name)?;
            let k = lensbench_core::edit::surf_type_parse(&text("type")?).map_err(e)?;
            t.set_surface_type(j, k).map_err(e)?;
        }
        "set_material" => {
            let j = surf(t, field("surface")?, name)?;
            t.set_material(j, &text("material")?).map_err(e)?;
        }
        "set_stop" => {
            let j = surf(t, field("surface")?, name)?;
            t.set_stop(j).map_err(e)?;
        }
        "add_pickup" => {
            let mut p = Pickup::new(
                surf(t, field("dst")?, name)?,
                surf(t, field("src")?, name)?,
                pick_kind_parse(&text("what")?).map_err(e)?,
                op.get("scale").and_then(num).unwrap_or(1.0),
                op.get("offset").and_then(num).unwrap_or(0.0),
            );
            p.from = match op.get("from") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    pick_kind_parse(v.as_str().ok_or(format!("{name} の from は量の名前"))?)
                        .map_err(e)?,
                ),
            };
            t.add_pickup(p).map_err(e)?;
        }
        "remove_pickup" => {
            let j = surf(t, field("dst")?, name)?;
            let w = pick_kind_parse(&text("what")?).map_err(e)?;
            t.remove_pickup(j, w).map_err(e)?;
        }
        "add_solve" => {
            let other = match op.get("other") {
                None | Some(Value::Null) => None,
                Some(v) => Some(surf(t, v, name)?),
            };
            let kind = match op.get("kind") {
                None | Some(Value::Null) => "marginal_height".to_string(),
                Some(_) => text("kind")?,
            };
            let q = ThickSolve {
                surf: surf(t, field("surface")?, name)?,
                height: op.get("value").and_then(num).unwrap_or(0.0),
                kind: lensbench_core::edit::solve_kind_parse(
                    &kind,
                    op.get("param").and_then(num).unwrap_or(0.0),
                    other,
                )
                .map_err(e)?,
            };
            t.add_solve(q).map_err(e)?;
        }
        "remove_solve" => {
            let j = surf(t, field("surface")?, name)?;
            let k = match op.get("what").and_then(Value::as_str).unwrap_or("t") {
                "t" => Kind::Thick,
                "c" => Kind::Curv,
                w => return Err(format!("{name} の what は t か c（{w}）")),
            };
            t.remove_solve(j, k).map_err(e)?;
        }
        "set_surface_data" => {
            let j = surf(t, field("surface")?, name)?;
            set_surface_data(t, j, field("data")?)?;
        }
        "set_system_data" => set_system_data(t, field("data")?)?,
        "set_param" => {
            let p = t.param(&text("param")?).map_err(e)?;
            let v = num(field("value")?).ok_or(format!("{name} の value は数"))?;
            t.set_param(p, v).map_err(e)?;
        }
        "define_unit" => {
            let spans = spans_of(field("spans")?, name)?;
            let pivot = match op.get("pivot") {
                None | Some(Value::Null) => [0.0; 3],
                Some(v) => vec3(v, name)?,
            };
            t.define_unit(&text("name")?, &spans, pivot).map_err(e)?;
        }
        "set_unit" => {
            let u = unit_of(t, field("unit")?, name)?;
            if let Some(v) = op.get("spans") {
                t.set_unit_spans(u, &spans_of(v, name)?).map_err(e)?;
            }
            if let Some(v) = op.get("pivot") {
                t.set_unit_pivot(u, vec3(v, name)?).map_err(e)?;
            }
            if op.get("name").is_some() {
                t.rename_unit(u, &text("name")?).map_err(e)?;
            }
        }
        "remove_unit" => {
            let u = unit_of(t, field("unit")?, name)?;
            notes.extend(t.remove_unit(u).map_err(e)?);
        }
        "add_mech" => {
            let m = mech_from(t, field("part")?.clone())?;
            if t.mechanics
                .iter()
                .any(|x| x.name.eq_ignore_ascii_case(&m.name))
            {
                return Err(format!("機構部品 {} は置いてある", m.name));
            }
            t.mechanics.push(m);
        }
        "set_mech" => {
            let i = mech_of(t, field("mech")?, name)?;
            let mut v = serde_json::to_value(&t.mechanics[i]).map_err(|x| x.to_string())?;
            let part = field("part")?
                .as_object()
                .ok_or(format!("{name} の part は {{項目: 値}}"))?;
            for (k, x) in part {
                if v.get(k).is_none() && !["holds"].contains(&k.as_str()) {
                    return Err(format!("機構部品の項目 {k} は無い"));
                }
                v[k] = x.clone();
            }
            let m = mech_from(t, v)?;
            if t.mechanics
                .iter()
                .enumerate()
                .any(|(k, x)| k != i && x.name.eq_ignore_ascii_case(&m.name))
            {
                return Err(format!("機構部品 {} は置いてある", m.name));
            }
            t.mechanics[i] = m;
        }
        "remove_mech" => {
            let i = mech_of(t, field("mech")?, name)?;
            t.mechanics.remove(i);
        }
        "set_config_count" => t.set_config_count(count("n")?).map_err(e)?,
        "set_config_row" => {
            let p = t.param(&text("param")?).map_err(e)?;
            let v: Vec<f64> = field("values")?
                .as_array()
                .ok_or(format!("{name} の values は数の並び"))?
                .iter()
                .map(|x| num(x).ok_or(format!("{name} の values は数の並び")))
                .collect::<Result<_, _>>()?;
            t.set_config_row(p, v).map_err(e)?;
        }
        "add_config_row" => {
            let p = t.param(&text("param")?).map_err(e)?;
            let n = t.configs().map_or(1, |c| c.n);
            let v = t.get_param(p).map_err(e)?;
            t.set_config_row(p, vec![v; n]).map_err(e)?;
        }
        "remove_config_row" => {
            let p = t.param(&text("param")?).map_err(e)?;
            t.remove_config_row(p).map_err(e)?;
        }
        "set_config" => {
            let k = count("k")?;
            *t = t.config(k).map_err(e)?;
        }
        "insert_config" => {
            let at = count("at")?;
            let pickup = op.get("pickup").and_then(Value::as_bool).unwrap_or(false);
            let from = match op.get("from") {
                Some(_) => count("from")?,
                None if pickup => 1,
                None => t.configs().map_or(1, |c| c.active),
            };
            t.insert_config(at, from, pickup).map_err(e)?;
        }
        "remove_config" => t.remove_config(count("k")?).map_err(e)?,
        "set_config_pickup" => {
            let p = t.param(&text("param")?).map_err(e)?;
            let f = |k: &str, d: f64| -> Result<f64, String> {
                op.get(k)
                    .map_or(Ok(d), |x| num(x).ok_or(format!("{name} の {k} は数")))
            };
            let (scale, offset) = (f("scale", 1.0)?, f("offset", 0.0)?);
            t.set_config_pickup(p, count("k")?, count("from")?, scale, offset)
                .map_err(e)?;
        }
        "remove_config_pickup" => {
            let p = t.param(&text("param")?).map_err(e)?;
            t.remove_config_pickup(p, count("k")?).map_err(e)?;
        }
        _ => return Err(format!("操作 {name} はない")),
    }
    Ok(())
}
