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
use lensbench_core::mech::MechPart;
use lensbench_core::param::{Kind, Pickup, ThickSolve};
use lensbench_core::surface::{Aperture, Surface};
use lensbench_core::System;
use serde::Deserialize;
use serde_json::{Map, Value};

pub use crate::edit_spec::{
    BlockFace, BlockRef, EditOp, Num, PickupOp, Ref, SolveOp, SolveWhat, StopOp, SurfaceOp,
};

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

/// 面（番号か名前）の系の面の番号。プロジェクトの glrs も引く
pub(crate) fn surf_of(s: &System, v: &Ref, what: &str) -> Result<usize, String> {
    surf(s, v, what)
}

fn surf(s: &System, v: &Ref, what: &str) -> Result<usize, String> {
    match v {
        Ref::Index(j) => Some(*j)
            .filter(|&j| j < s.surfaces.len())
            .ok_or_else(|| format!("{what}: no surface {j}")),
        Ref::Name(t) => s.find_surface(t).map_err(|e| format!("{what}: {e}")),
    }
}

fn vec3(v: &[Num; 3]) -> [f64; 3] {
    v.map(|x| x.0)
}

/// 玉・群（名前か、0 始まりの番号）
fn unit_of(s: &System, v: &Ref, what: &str) -> Result<usize, String> {
    match v {
        Ref::Index(i) => Some(*i)
            .filter(|&i| i < s.units.len())
            .ok_or_else(|| format!("{what}: no lens or group {i}")),
        Ref::Name(t) => s
            .find_unit(t)
            .ok_or_else(|| format!("{what}: no lens or group {t}")),
    }
}

/// 機構部品（名前か、0 始まりの番号）
fn mech_of(s: &System, v: &Ref, what: &str) -> Result<usize, String> {
    match v {
        Ref::Index(i) => Some(*i)
            .filter(|&i| i < s.mechanics.len())
            .ok_or_else(|| format!("{what}: mechanical part {i}")),
        Ref::Name(t) => s
            .mechanics
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(t.trim()))
            .ok_or_else(|| format!("{what}: no mechanical part {t}")),
    }
}

/// 機構部品を確かめる（名前・面の番号・寸法）
fn mech_check(s: &System, m: MechPart) -> Result<MechPart, String> {
    let n = s.surfaces.len();
    if m.name.trim().is_empty() {
        return Err("empty mechanical part name".into());
    }
    if m.surface == 0 || m.surface >= n {
        return Err(format!(
            "mechanical part {} surface {} (1 to {})",
            m.name,
            m.surface,
            n - 1
        ));
    }
    if let Some(h) = m.holds.iter().find(|&&h| h >= n) {
        return Err(format!("mechanical part {} holds surface {h}", m.name));
    }
    let fin = m.offset.iter().chain(&m.tilt).all(|x| x.is_finite())
        && [m.length, m.outer_radius, m.hold_radius, m.rod_spacing]
            .iter()
            .all(|x| x.is_finite() && *x >= 0.0);
    if !fin {
        return Err(format!("mechanical part {} position or size", m.name));
    }
    for b in &m.bores {
        if !(b.z0.is_finite() && b.z1.is_finite() && b.z0 <= b.z1 && b.radius > 0.0) {
            return Err(format!(
                "mechanical part {} bore (z {} to {}, radius {})",
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
        Err(format!("number of spider arms {n}"))
    }
}

fn aperture(v: &Value) -> Result<Aperture, String> {
    if v.is_null() {
        return Ok(Aperture::None);
    }
    let g = |k: &str| v.get(k).and_then(num).unwrap_or(0.0);
    let need = |k: &str| {
        v.get(k)
            .and_then(num)
            .ok_or(format!("the aperture needs {k}"))
    };
    let (dx, dy) = (g("dx"), g("dy"));
    let t = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or("the aperture needs type")?;
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
        t => return Err(format!("aperture type {t}")),
    })
}

/// 面の値の dict を名前と値の組に
pub fn surf_fields(v: &Value) -> Result<Vec<(String, SurfValue)>, String> {
    let Some(m) = v.as_object() else {
        return Err("surface values must be an object (name: value)".into());
    };
    surf_fields_map(m)
}

/// 面の値の {名前: 値} を名前と値の組に
pub fn surf_fields_map(m: &Map<String, Value>) -> Result<Vec<(String, SurfValue)>, String> {
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
                        .map(|y| {
                            num(y).ok_or(format!("surface value {k} must be a list of numbers"))
                        })
                        .collect::<Result<_, _>>()?,
                ),
                _ => return Err(format!("type of surface value {k}")),
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
    let d = s.surfaces.get(j).ok_or_else(|| {
        format!(
            "surface {j} (surfaces are 0 to {})",
            s.surfaces.len().saturating_sub(1)
        )
    })?;
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
pub(crate) fn set_surface_data(
    t: &mut System,
    j: usize,
    m: &Map<String, Value>,
) -> Result<(), String> {
    let cur = t.surfaces[j].clone();
    let c = ciborium::value::Value::serialized(&cur).map_err(|e| e.to_string())?;
    let mut v = crate::sysfile::to_json(c)?;
    let o = v.as_object_mut().ok_or("surface data")?;
    for (k, x) in m {
        if ["n", "n_base", "reflect", "stop", "mirror"].contains(&k.as_str()) {
            return Err(format!(
                "surface item {k} is not changed by data (use the material or set_stop)"
            ));
        }
        // user（利用者定義の面）と semi_autoは無ければ書き出さないので、無くても足せる
        if !o.contains_key(k) && !["user", "semi_auto", "bire"].contains(&k.as_str()) {
            return Err(format!("no surface item {k}"));
        }
        let x = coerce(x.clone(), o.get(k), k, k == "bin2");
        o.insert(k.clone(), x);
    }
    // 半径の値を書けば固定（semi_auto も書いたときはそちら）
    if m.contains_key("semi") && !m.contains_key("semi_auto") {
        o.insert("semi_auto".into(), Value::Bool(false));
    }
    let mut d: Surface = crate::lenient::decode(crate::sysfile::from_json(v)?, "surface data")?;
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
    let o = v.as_object().ok_or("system data")?;
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
pub(crate) fn set_system_data(t: &mut System, m: &Map<String, Value>) -> Result<(), String> {
    let c = ciborium::value::Value::serialized(&*t).map_err(|e| e.to_string())?;
    let mut v = crate::sysfile::to_json(c)?;
    let o = v.as_object_mut().ok_or("system data")?;
    for (k, x) in m {
        if !SYSTEM_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "system item {k} is not changed by set_system_data ({})",
                SYSTEM_KEYS.join("・")
            ));
        }
        let x = coerce(x.clone(), o.get(k), k, false);
        o.insert(k.clone(), x);
    }
    let mut n: System = crate::lenient::decode(crate::sysfile::from_json(v)?, "system data")?;
    let nw = n.wavelengths.len();
    if nw == 0 {
        return Err("no wavelengths".into());
    }
    if n.wavelengths.iter().any(|&l| !(l.is_finite() && l > 0.0)) {
        return Err(format!("wavelengths {:?}", n.wavelengths));
    }
    if n.weights.len() != nw {
        return Err(format!(
            "{} wavelength weights ({nw} wavelengths)",
            n.weights.len()
        ));
    }
    if n.primary >= nw {
        return Err(format!(
            "primary wavelength {} (0-based, {nw} wavelengths)",
            n.primary
        ));
    }
    if n.fields.is_empty() {
        return Err("no fields".into());
    }
    if n.glrs >= n.surfaces.len() {
        return Err(format!("global coordinate reference surface {}", n.glrs));
    }
    if !(n.temperature.is_finite() && n.pressure.is_finite()) {
        return Err("temperature and pressure must be numbers".into());
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

/// 操作の列の JSON を読む（知らない操作・キーは誤り。誤りの文の頭に操作の番号）
pub fn parse(ops: &Value) -> Result<Vec<EditOp>, String> {
    let Some(list) = ops.as_array() else {
        return Err("ops must be a list of operations".into());
    };
    list.iter()
        .enumerate()
        .map(|(i, o)| {
            EditOp::deserialize(o).map_err(|e| {
                let name = o.get("op").and_then(Value::as_str).unwrap_or("?");
                format!("operation {} ({name}): {e}", i + 1)
            })
        })
        .collect()
}

/// JSON の操作の列を読んで当てる（[`parse`] と [`apply`]）
pub fn apply_json(s: &mut System, ops: &Value) -> Result<Vec<String>, String> {
    apply(s, &parse(ops)?)
}

/// 操作の列を順に当てる（どれかが失敗すれば系は変えない）。面を消したときなどの説明を返す
pub fn apply(s: &mut System, ops: &[EditOp]) -> Result<Vec<String>, String> {
    let mut t = s.clone();
    let mut notes = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let at = |e: String| format!("operation {}: {e}", i + 1);
        apply_one(&mut t, op, &mut notes).map_err(at)?;
        // 表の操作でなければ、変えた値を表の今の構成の列にも
        if !op.is_config() {
            t.sync_active_config();
        }
    }
    *s = t;
    Ok(notes)
}

fn apply_one(t: &mut System, op: &EditOp, notes: &mut Vec<String>) -> Result<(), String> {
    use EditOp as O;
    let e = |x: lensbench_core::Error| x.to_string();
    let name = op.name();
    let name = name.as_str();
    match op {
        O::InsertSurface { at, surface } => {
            let f = surf_fields_map(surface)?;
            t.insert_surface(*at, Surface::default()).map_err(e)?;
            t.set_surface_fields(*at, &f).map_err(e)?;
        }
        O::DeleteSurface { surface } => {
            let j = surf(t, surface, name)?;
            notes.extend(t.delete_surface(j).map_err(e)?);
        }
        O::SetSurface { surface, fields } => {
            let j = surf(t, surface, name)?;
            let f = surf_fields_map(fields)?;
            t.set_surface_fields(j, &f).map_err(e)?;
        }
        O::SetSurfaceType { surface, typ } => {
            let j = surf(t, surface, name)?;
            let k = lensbench_core::edit::surf_type_parse(typ).map_err(e)?;
            t.set_surface_type(j, k).map_err(e)?;
        }
        O::SetMaterial { surface, material } => {
            let j = surf(t, surface, name)?;
            t.set_material(j, material).map_err(e)?;
        }
        O::SetStop(o) => {
            let j = surf(t, &o.surface, name)?;
            t.set_stop(j).map_err(e)?;
        }
        O::AddPickup(o) => {
            let mut p = Pickup::new(
                surf(t, &o.dst, name)?,
                surf(t, &o.src, name)?,
                pick_kind_parse(&o.what).map_err(e)?,
                o.scale.map_or(1.0, |x| x.0),
                o.offset.map_or(0.0, |x| x.0),
            );
            p.from = match &o.from {
                None => None,
                Some(v) => Some(pick_kind_parse(v).map_err(e)?),
            };
            t.add_pickup(p).map_err(e)?;
        }
        O::RemovePickup { dst, what } => {
            let j = surf(t, dst, name)?;
            let w = pick_kind_parse(what).map_err(e)?;
            t.remove_pickup(j, w).map_err(e)?;
        }
        O::AddSolve(o) => {
            let other = match &o.other {
                None => None,
                Some(v) => Some(surf(t, v, name)?),
            };
            let q = ThickSolve {
                surf: surf(t, &o.surface, name)?,
                height: o.value.map_or(0.0, |x| x.0),
                kind: lensbench_core::edit::solve_kind_parse(
                    o.kind.as_deref().unwrap_or("marginal_height"),
                    o.param.map_or(0.0, |x| x.0),
                    other,
                )
                .map_err(e)?,
            };
            t.add_solve(q).map_err(e)?;
        }
        O::RemoveSolve { surface, what } => {
            let j = surf(t, surface, name)?;
            let k = match what {
                SolveWhat::T => Kind::Thick,
                SolveWhat::C => Kind::Curv,
            };
            t.remove_solve(j, k).map_err(e)?;
        }
        O::SetSurfaceData { surface, data } => {
            let j = surf(t, surface, name)?;
            set_surface_data(t, j, data)?;
        }
        O::SetSystemData { data } => set_system_data(t, data)?,
        O::SetParam { param, value } => {
            let p = t.param(param).map_err(e)?;
            t.set_param(p, value.0).map_err(e)?;
        }
        O::DefineUnit {
            name: n,
            spans,
            pivot,
        } => {
            let pivot = pivot.as_ref().map_or([0.0; 3], vec3);
            t.define_unit(n, spans, pivot).map_err(e)?;
        }
        O::SetUnit {
            unit,
            name: n,
            spans,
            pivot,
        } => {
            let u = unit_of(t, unit, name)?;
            if let Some(v) = spans {
                t.set_unit_spans(u, v).map_err(e)?;
            }
            if let Some(v) = pivot {
                t.set_unit_pivot(u, vec3(v)).map_err(e)?;
            }
            if let Some(n) = n {
                t.rename_unit(u, n).map_err(e)?;
            }
        }
        O::RemoveUnit { unit } => {
            let u = unit_of(t, unit, name)?;
            notes.extend(t.remove_unit(u).map_err(e)?);
        }
        O::AddMech { part } => {
            let m = mech_check(t, part.clone())?;
            if t.mechanics
                .iter()
                .any(|x| x.name.eq_ignore_ascii_case(&m.name))
            {
                return Err(format!("mechanical part {} is already placed", m.name));
            }
            t.mechanics.push(m);
        }
        O::SetMech { mech, part } => {
            let i = mech_of(t, mech, name)?;
            let mut v = serde_json::to_value(&t.mechanics[i]).map_err(|x| x.to_string())?;
            for (k, x) in part {
                if v.get(k).is_none() && !["holds"].contains(&k.as_str()) {
                    return Err(format!("no mechanical part item {k}"));
                }
                v[k] = x.clone();
            }
            let m: MechPart =
                serde_json::from_value(v).map_err(|x| format!("mechanical part: {x}"))?;
            let m = mech_check(t, m)?;
            if t.mechanics
                .iter()
                .enumerate()
                .any(|(k, x)| k != i && x.name.eq_ignore_ascii_case(&m.name))
            {
                return Err(format!("mechanical part {} is already placed", m.name));
            }
            t.mechanics[i] = m;
        }
        O::RemoveMech { mech } => {
            let i = mech_of(t, mech, name)?;
            t.mechanics.remove(i);
        }
        O::SetConfigCount { n } => t.set_config_count(*n).map_err(e)?,
        O::SetConfigRow { param, values } => {
            let p = t.param(param).map_err(e)?;
            t.set_config_row(p, values.iter().map(|x| x.0).collect())
                .map_err(e)?;
        }
        O::AddConfigRow { param } => {
            let p = t.param(param).map_err(e)?;
            let n = t.configs().map_or(1, |c| c.n);
            let v = t.get_param(p).map_err(e)?;
            t.set_config_row(p, vec![v; n]).map_err(e)?;
        }
        O::RemoveConfigRow { param } => {
            let p = t.param(param).map_err(e)?;
            t.remove_config_row(p).map_err(e)?;
        }
        O::SetConfig { k } => {
            *t = t.config(*k).map_err(e)?;
        }
        O::InsertConfig { at, from, pickup } => {
            let from = match from {
                Some(f) => *f,
                None if *pickup => 1,
                None => t.configs().map_or(1, |c| c.active),
            };
            t.insert_config(*at, from, *pickup).map_err(e)?;
        }
        O::RemoveConfig { k } => t.remove_config(*k).map_err(e)?,
        O::SetConfigPickup {
            param,
            k,
            from,
            scale,
            offset,
        } => {
            let p = t.param(param).map_err(e)?;
            let (scale, offset) = (scale.map_or(1.0, |x| x.0), offset.map_or(0.0, |x| x.0));
            t.set_config_pickup(p, *k, *from, scale, offset)
                .map_err(e)?;
        }
        O::RemoveConfigPickup { param, k } => {
            let p = t.param(param).map_err(e)?;
            t.remove_config_pickup(p, *k).map_err(e)?;
        }
    }
    Ok(())
}
