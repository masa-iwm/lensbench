//! モンテカルロ公差の設定の JSON と、結果の JSON。CLI の `tolerance`・serve の `tolerance`・Python の
//! `System.tolerance` で同じ形を使う。計算は [`lensbench_core::tolerance`]。
//!
//! 設定（すべて省略可。省くと「一般」の公差の組をすべての玉に、ピントを補償子にせず、RMS 波面で 100 回）:
//!
//! | キー | 意味 |
//! |---|---|
//! | `trials` | 試行の数（100）。0 なら感度だけ |
//! | `seed` | 乱数の種（1）。同じ種なら同じ結果（スレッド数によらない） |
//! | `dist` | 既定の分布（"normal"）: "uniform"・"normal"（範囲の端が ±2σ）・"normal:3"・"end"（端の値を半々） |
//! | `radial` | 偏心・傾き・くさびの x・y を組にして、円の中で引く（false）。`true`・`"uniform"` は円の中で一様、`"normal"`・`"normal:σの数"` は等方の 2 次元正規分布を円で切ったもの（σ = 公差 / σの数、既定 2）。隣り合う x・y の公差（同じ面・玉・群の dx と dy、tx と ty）が対象で、大きさが公差以下になる（x・y 別々では角で √2 倍まで）。組の分布は `dist` によらない |
//! | `preset` | 玉の既定の公差の組: "commercial"（一般）・"precision"（精密）・"high"（高精度）・"none" |
//! | `catalog` | カタログの玉（プロジェクトのレンズカタログのブロック）はカタログの玉の公差（[`catalog_lens_tol`]、`catalog_tols`）を既定にする（true） |
//! | `catalog_tols` | メーカーごとの公差 {"メーカー": {玉の公差と外径・焦点距離の条件つきの rules}}（[`catalog_rule`]。名前は大文字小文字を区別せず、ブロックのメーカー名に含まれれば合う。"*" はほか） |
//! | `lenses` | 玉ごとの上書き {"名前": {玉の公差} か 組の名前 か null（公差なし）}。名前はブロックの id か lens1 … |
//! | `groups` | 群・ブロック（玉でないもの）の公差 {"名前": {"decenter", "tilt", "shift"}} |
//! | `tols` | 個別の公差の並び（下） |
//! | `compensators` | 補償子（"9.t"・"G1.shift=-1:1"、または {"param", "lo", "hi", "configs"}。configs は効く構成の番号（1 から）の並びで、省くと全構成で同じ値。構成ごとに別の値にするときは構成ごとに 1 つずつ書く） |
//! | `comp_iters` | 補償子の DLS の反復の上限（10） |
//! | `merit` | 評価関数: "wavefront"（既定）・"spot"・"none"（terms だけ） |
//! | `rings`・`ref` | 既定の評価関数の瞳のリング数（3）・基準（"centroid"・"chief"） |
//! | `terms` | 足す評価関数の項（[`crate::merit`] の形。MTF・近軸量など） |
//! | `sensitivity` | 公差ごとの感度も求める（true） |
//! | `percentiles` | 出す百分位（[50, 80, 90, 95, 98]） |
//! | `samples` | 試行ごとのずれと値も返す（false） |
//!
//! 玉の公差 {"radius_rel", "focal_rel", "thickness", "wedge", "dn", "disp", "decenter", "tilt", "space"}
//! （[`LensTol`]。比・mm・deg。書いた項目だけ既定の組を上書きする）。
//!
//! 個別の公差: {"param": "3.t"} か {"params": ["2.r", "3.r"]} に、`pm`（±）か `lo`・`hi`、`relative`（比、false）、
//! `dist`、`label`、`scale`（パラメータごとにずれに掛ける数の並び。往復する面の厚さのように逆の符号で与えるとき -1）。
//!
//! 鏡の `back` で同じ玉を往復する系では、玉の公差の組は往きと戻りの面に同じ誤差を与える（曲率・くさびは同じ面、
//! 厚さ・間隔は戻りで逆の符号、偏心・傾きは往復の範囲を持つブロックの動き）。玉の名前はブロックの id。

use std::collections::HashMap;

use lensbench_core::optim::{DefaultMerit, DlsOpts, Problem, Var};
use lensbench_core::sweep::RmsRef;
use lensbench_core::tolerance::{self, Dist, GroupTol, LensTol, McResult, McSpec, Tol};
use lensbench_core::System;
use serde_json::{json, Map, Value};

use crate::{Block, LensSource, Project};

/// カタログの玉の既定の公差（メーカーによらない目安）: 中心厚 ±0.1 mm・焦点距離 ±2%・芯取り 5′。偏心・傾き・間隔は
/// 組立の公差なので、既定の組（`preset`）の値を使う。メーカーごとの公表値は `catalog_tols` で与える
/// （[`catalog_rule`]）
pub fn catalog_lens_tol(base: LensTol) -> LensTol {
    LensTol {
        radius_rel: 0.0,
        focal_rel: 0.02,
        thickness: 0.1,
        wedge: 5.0 / 60.0,
        ..base
    }
}

/// `catalog_tols` の 1 つのメーカーの値を、形 g（外径・焦点距離 [mm]・ガラスの枚数）の玉に当てる。値は玉の公差の項目
/// （[`LensTol`] の名前）に、`rules`（外径・焦点距離の条件つきの上書きの並び。前から順に、条件が合うものを当てる）:
///
/// ```json
/// {"thickness": 0.1, "focal_rel": 0.01, "wedge": 0.05,
///  "rules": [{"dia_lt": 10, "focal_rel": 0.02}, {"dia_le": 12, "thickness": 0.05}, {"efl_gt": 10, "wedge": 0.0167}]}
/// ```
///
/// 条件は `dia_lt` `dia_le` `dia_gt` `dia_ge` `efl_lt` `efl_le` `efl_gt` `efl_ge`（焦点距離は絶対値で比べる）と
/// `cemented`（true は貼り合わせ = ガラスが 2 枚以上の玉、false は単レンズ）。
/// 1 つの規則に条件が複数あればすべてを満たすとき。外径・焦点距離が分からない（NaN）と条件は満たさない
pub fn catalog_rule(
    v: &Value,
    base: LensTol,
    g: LensGeom,
    what: &str,
) -> Result<Option<LensTol>, String> {
    let (dia, efl) = (g.dia, g.efl);
    let Value::Object(m) = v else {
        return lens_tol_from(v, base, what);
    };
    let mut plain = m.clone();
    let rules = plain.remove("rules");
    let Some(mut t) = lens_tol_from(&Value::Object(plain), base, what)? else {
        return Ok(None);
    };
    let Some(rules) = rules else {
        return Ok(Some(t));
    };
    let rules = rules
        .as_array()
        .ok_or_else(|| format!("{what}.rules は並び"))?;
    for (i, r) in rules.iter().enumerate() {
        let w = format!("{what}.rules[{i}]");
        let r = r.as_object().ok_or_else(|| format!("{w} はオブジェクト"))?;
        let mut ok = true;
        let mut fields = Map::new();
        for (k, x) in r {
            if k == "cemented" {
                let c = x.as_bool().ok_or_else(|| format!("{w}.cemented は真偽"))?;
                ok &= c == (g.glasses >= 2);
                continue;
            }
            let (q, op) = match k.split_once('_') {
                Some((q @ ("dia" | "efl"), op @ ("lt" | "le" | "gt" | "ge"))) => (q, op),
                _ => {
                    fields.insert(k.clone(), x.clone());
                    continue;
                }
            };
            let lim = num(x, &format!("{w}.{k}"))?;
            let val = if q == "dia" { dia } else { efl.abs() };
            ok &= match op {
                "lt" => val < lim,
                "le" => val <= lim,
                "gt" => val > lim,
                _ => val >= lim,
            };
        }
        // 条件が合わなくても項目の名前は確かめる
        let u = lens_tol_from(&Value::Object(fields), t, &w)?.unwrap_or(t);
        if ok {
            t = u;
        }
    }
    Ok(Some(t))
}

/// 玉の形（[`catalog_rule`] の条件）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensGeom {
    /// 外径 [mm]（面の半径の最大の 2 倍）
    pub dia: f64,
    /// 主波長の焦点距離 [mm]
    pub efl: f64,
    /// ガラスの枚数（貼り合わせなら 2 以上）
    pub glasses: usize,
}

impl LensGeom {
    pub const UNKNOWN: LensGeom = LensGeom {
        dia: f64::NAN,
        efl: f64::NAN,
        glasses: 1,
    };
}

/// 玉の名前（lens1 … と、同じ範囲のブロックの id）→ 玉の形
fn lens_geometry(s: &System) -> HashMap<String, LensGeom> {
    let mut m = HashMap::new();
    let Ok(els) = lensbench_core::elements::split(s) else {
        return m;
    };
    let lenses: Vec<_> = els
        .iter()
        .filter(|e| e.kind == lensbench_core::elements::Kind::Lens)
        .collect();
    for (k, e) in lenses.iter().enumerate() {
        let dia = 2.0 * e.surfaces.iter().map(|d| d.semi).fold(0.0, f64::max);
        let glasses = e
            .surfaces
            .iter()
            .take(e.surfaces.len().saturating_sub(1))
            .filter(|d| !d.mat.is_empty() && d.mat != "-" && !d.mirror)
            .count()
            .max(1);
        let g = LensGeom {
            dia,
            efl: e.efl(s.primary),
            glasses,
        };
        let auto = format!("lens{}", k + 1);
        if let Some(u) = s.units.iter().find(|u| u.auto && u.name == auto) {
            // 鏡で折り返して往復する玉は、最初の範囲がこの玉のブロック
            for v in s
                .units
                .iter()
                .filter(|v| !v.auto && (v.spans == u.spans || v.spans.first() == u.spans.first()))
            {
                m.insert(v.name.clone(), g);
            }
        }
        m.insert(auto, g);
    }
    m
}

/// プロジェクトのレンズカタログの玉: ブロックの id → メーカー
pub fn catalog_vendors(p: &Project) -> HashMap<String, String> {
    fn walk(bs: &[Block], m: &mut HashMap<String, String>) {
        for b in bs {
            match b {
                Block::Lens {
                    id,
                    source: LensSource::Catalog { vendor, .. },
                    ..
                } if !id.is_empty() => {
                    m.insert(id.clone(), vendor.clone());
                }
                Block::Group { blocks, .. } => walk(blocks, m),
                _ => {}
            }
        }
    }
    let mut m = HashMap::new();
    walk(&p.blocks, &mut m);
    m
}

fn num(v: &Value, k: &str) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("{k} は数（{v}）"))
}

fn lens_tol_from(v: &Value, base: LensTol, what: &str) -> Result<Option<LensTol>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => LensTol::preset(s)
            .map(Some)
            .map_err(|e| format!("{what}: {e}")),
        Value::Object(m) => {
            let mut t = base;
            for (k, x) in m {
                let f = num(x, &format!("{what}.{k}"))?;
                match k.as_str() {
                    "radius_rel" => t.radius_rel = f,
                    "focal_rel" => t.focal_rel = f,
                    "thickness" => t.thickness = f,
                    "wedge" => t.wedge = f,
                    "dn" => t.dn = f,
                    "disp" => t.disp = f,
                    "decenter" => t.decenter = f,
                    "tilt" => t.tilt = f,
                    "space" => t.space = f,
                    _ => {
                        return Err(format!(
                            "{what}: 玉の公差の項目 {k}（radius_rel focal_rel thickness wedge dn disp decenter tilt space）"
                        ))
                    }
                }
            }
            Ok(Some(t))
        }
        _ => Err(format!("{what} はオブジェクト・組の名前・null（{v}）")),
    }
}

fn group_tol_from(v: &Value, what: &str) -> Result<Option<GroupTol>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Object(m) => {
            let mut g = GroupTol::default();
            for (k, x) in m {
                let f = num(x, &format!("{what}.{k}"))?;
                match k.as_str() {
                    "decenter" => g.decenter = f,
                    "tilt" => g.tilt = f,
                    "shift" => g.shift = f,
                    _ => return Err(format!("{what}: 群の公差の項目 {k}（decenter tilt shift）")),
                }
            }
            Ok(Some(g))
        }
        _ => Err(format!("{what} はオブジェクトか null（{v}）")),
    }
}

/// "9.t"・"G1.shift=-1:1"・{"param", "lo", "hi"}
fn var_from(v: &Value, s: &System) -> Result<Var, String> {
    let mut configs = None;
    let (spec, lo, hi) = match v {
        Value::String(t) => match t.split_once('=') {
            Some((a, r)) => {
                let (l, h) = r
                    .split_once(':')
                    .ok_or_else(|| format!("補償子 {t} の範囲は 下限:上限"))?;
                let n = |x: &str, d: f64| -> Result<f64, String> {
                    if x.trim().is_empty() {
                        Ok(d)
                    } else {
                        x.trim()
                            .parse()
                            .map_err(|_| format!("補償子 {t} の範囲 {x}"))
                    }
                };
                (
                    a.trim().to_string(),
                    n(l, f64::NEG_INFINITY)?,
                    n(h, f64::INFINITY)?,
                )
            }
            None => (t.trim().to_string(), f64::NEG_INFINITY, f64::INFINITY),
        },
        Value::Object(m) => {
            let p = m
                .get("param")
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("補償子 {v} に param がない"))?;
            let g = |k: &str, d: f64| -> Result<f64, String> {
                match m.get(k) {
                    None | Some(Value::Null) => Ok(d),
                    Some(x) => num(x, k),
                }
            };
            if let Some(k) = m
                .keys()
                .find(|k| !["param", "lo", "hi", "configs"].contains(&k.as_str()))
            {
                return Err(format!("補償子 {p}: 項目 {k}（param lo hi configs）"));
            }
            if let Some(c) = m.get("configs").filter(|x| !x.is_null()) {
                let a = c
                    .as_array()
                    .ok_or_else(|| format!("補償子 {p}: configs は構成の番号（1 から）の並び"))?;
                let mut out = Vec::new();
                for x in a {
                    match x.as_u64() {
                        Some(k) if k >= 1 => out.push(k as usize - 1),
                        _ => {
                            return Err(format!(
                                "補償子 {p}: configs は構成の番号（1 から）の並び（{x}）"
                            ))
                        }
                    }
                }
                configs = Some(out);
            }
            (
                p.to_string(),
                g("lo", f64::NEG_INFINITY)?,
                g("hi", f64::INFINITY)?,
            )
        }
        _ => return Err(format!("補償子は文字列かオブジェクト（{v}）")),
    };
    let p = s.param(&spec).map_err(|e| e.to_string())?;
    let v = Var::new(p).bounds(lo, hi);
    Ok(match configs {
        Some(c) => v.in_configs(c),
        None => v,
    })
}

fn tol_from(v: &Value, s: &System, dist: Dist, i: usize) -> Result<Tol, String> {
    let m = v
        .as_object()
        .ok_or_else(|| format!("公差 {i} はオブジェクト（{v}）"))?;
    let known = [
        "param", "params", "pm", "lo", "hi", "relative", "dist", "label", "scale",
    ];
    if let Some(k) = m.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("公差 {i}: 項目 {k}（{}）", known.join(" ")));
    }
    let names: Vec<String> = match (m.get("param"), m.get("params")) {
        (Some(Value::String(p)), None) => vec![p.clone()],
        (None, Some(Value::Array(a))) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(|t| t.to_string())
                    .ok_or_else(|| format!("公差 {i}: params は文字列の並び"))
            })
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(format!(
                "公差 {i}: param（文字列）か params（並び）のどちらか"
            ))
        }
    };
    let params = names
        .iter()
        .map(|n| s.param(n).map_err(|e| format!("公差 {i}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let (lo, hi) = match (m.get("pm"), m.get("lo"), m.get("hi")) {
        (Some(p), None, None) => {
            let p = num(p, "pm")?.abs();
            (-p, p)
        }
        (None, Some(l), Some(h)) => (num(l, "lo")?, num(h, "hi")?),
        _ => return Err(format!("公差 {i}: pm か lo・hi の組")),
    };
    let dist = match m.get("dist") {
        None | Some(Value::Null) => dist,
        Some(Value::String(d)) => Dist::parse(d).map_err(|e| format!("公差 {i}: {e}"))?,
        Some(d) => return Err(format!("公差 {i}: dist は文字列（{d}）")),
    };
    let relative = m.get("relative").and_then(|x| x.as_bool()).unwrap_or(false);
    let label = m
        .get("label")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .unwrap_or_else(|| names.join("+"));
    let scale = match m.get("scale") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.len() == params.len() => a
            .iter()
            .map(|x| num(x, "scale"))
            .collect::<Result<Vec<_>, _>>()?,
        Some(x) => {
            return Err(format!(
                "公差 {i}: scale はパラメータと同じ数（{}）の数の並び（{x}）",
                params.len()
            ))
        }
    };
    Ok(Tol {
        label,
        params,
        lo,
        hi,
        relative,
        dist,
        scale,
    })
}

/// 設定の JSON から [`McSpec`] を作る。vendors はブロックの id → メーカー（[`catalog_vendors`]。.zmx なら空）
pub fn spec_from_json(
    v: &Value,
    configs: Vec<System>,
    vendors: &HashMap<String, String>,
) -> Result<McSpec, String> {
    let empty = Map::new();
    let m = match v {
        Value::Null => &empty,
        Value::Object(m) => m,
        _ => return Err("公差の設定はオブジェクト".into()),
    };
    let known = [
        "trials",
        "seed",
        "dist",
        "radial",
        "preset",
        "catalog",
        "catalog_tols",
        "lenses",
        "groups",
        "tols",
        "compensators",
        "comp_iters",
        "merit",
        "rings",
        "ref",
        "terms",
        "sensitivity",
        "percentiles",
        "samples",
        "threads",
    ];
    if let Some(k) = m.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("公差の設定の項目 {k}（{}）", known.join(" ")));
    }
    let s0 = configs.first().ok_or("構成がない")?.clone();
    let get = |k: &str| m.get(k).filter(|x| !x.is_null());
    let dist = match get("dist") {
        None => Dist::Normal { cut: 2.0 },
        Some(Value::String(d)) => Dist::parse(d).map_err(|e| e.to_string())?,
        Some(d) => return Err(format!("dist は文字列（{d}）")),
    };
    let base = match get("preset") {
        None => LensTol::preset("commercial").unwrap(),
        Some(Value::String(p)) => LensTol::preset(p).map_err(|e| e.to_string())?,
        Some(p) => return Err(format!("preset は文字列（{p}）")),
    };
    let use_catalog = get("catalog").and_then(|x| x.as_bool()).unwrap_or(true);
    // メーカー名（大文字小文字を区別せず、ブロックのメーカー名に含まれれば合う。"*" はほかのすべて）→ 公差
    let mut cat_over: Vec<(String, Value)> = Vec::new();
    if let Some(c) = get("catalog_tols") {
        let c = c.as_object().ok_or("catalog_tols はオブジェクト")?;
        for (k, x) in c {
            cat_over.push((k.to_uppercase(), x.clone()));
        }
    }
    let find_cat = |vd: &str| -> Option<&(String, Value)> {
        let u = vd.to_uppercase();
        cat_over
            .iter()
            .find(|(k, _)| k != "*" && u.contains(k.as_str()))
            .or_else(|| cat_over.iter().find(|(k, _)| k == "*"))
    };
    let mut lens_over: HashMap<String, Option<LensTol>> = HashMap::new();
    let geom = lens_geometry(&s0);
    let lens_base = |name: &str| -> Result<LensTol, String> {
        match vendors.get(name).filter(|_| use_catalog) {
            Some(vd) => {
                let g = geom.get(name).copied().unwrap_or(LensGeom::UNKNOWN);
                let t = catalog_lens_tol(base);
                match find_cat(vd) {
                    Some((k, x)) => Ok(
                        catalog_rule(x, t, g, &format!("catalog_tols.{k}"))?.unwrap_or_default()
                    ),
                    None => Ok(t),
                }
            }
            None => Ok(base),
        }
    };
    if let Some(l) = get("lenses") {
        let l = l.as_object().ok_or("lenses はオブジェクト")?;
        for (k, x) in l {
            let b = lens_base(k)?;
            lens_over.insert(k.clone(), lens_tol_from(x, b, &format!("lenses.{k}"))?);
        }
    }
    let mut group_over: HashMap<String, Option<GroupTol>> = HashMap::new();
    if let Some(g) = get("groups") {
        let g = g.as_object().ok_or("groups はオブジェクト")?;
        for (k, x) in g {
            group_over.insert(k.clone(), group_tol_from(x, &format!("groups.{k}"))?);
        }
    }
    // 名前の誤りを見つける
    for k in lens_over.keys().chain(group_over.keys()) {
        if s0.find_unit(k).is_none() {
            return Err(format!("玉・群 {k} がない"));
        }
    }
    let base_err = std::cell::RefCell::new(None);
    let lens_fn = |name: &str| -> Option<LensTol> {
        if let Some(t) = lens_over.get(name) {
            return *t;
        }
        match lens_base(name) {
            Ok(t) => Some(t),
            Err(e) => {
                base_err.borrow_mut().get_or_insert(e);
                None
            }
        }
    };
    let group_fn = |name: &str| group_over.get(name).copied().flatten();
    let mut tols = tolerance::simple(&s0, &lens_fn, &group_fn, dist);
    if let Some(e) = base_err.into_inner() {
        return Err(e);
    }
    if let Some(t) = get("tols") {
        let a = t.as_array().ok_or("tols は並び")?;
        for (i, x) in a.iter().enumerate() {
            tols.push(tol_from(x, &s0, dist, i + 1)?);
        }
    }
    let mut p = Problem::new(configs);
    if let Some(c) = get("compensators") {
        let a: Vec<Value> = match c {
            Value::Array(a) => a.clone(),
            x => vec![x.clone()],
        };
        for x in &a {
            p.vars.push(var_from(x, &s0)?);
        }
    }
    let rings = match get("rings") {
        None => 3,
        Some(x) => x.as_u64().ok_or("rings は整数")? as usize,
    };
    let reference = match get("ref").and_then(|x| x.as_str()).unwrap_or("centroid") {
        "centroid" => RmsRef::Centroid,
        "chief" => RmsRef::Chief,
        r => return Err(format!("ref {r}（centroid・chief）")),
    };
    match get("merit").and_then(|x| x.as_str()).unwrap_or("wavefront") {
        "wavefront" | "wfe" => p.add_default(DefaultMerit::Wavefront, rings, reference),
        "spot" => p.add_default(DefaultMerit::Spot, rings, reference),
        "none" => {}
        x => return Err(format!("merit {x}（wavefront・spot・none）")),
    }
    if let Some(t) = get("terms") {
        let ts = crate::merit::terms_from_json(t, &p.configs)?;
        p.terms.extend(ts);
    }
    match get("radial") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            tolerance::pair_disc(&mut tols, None);
        }
        Some(Value::String(r)) => {
            let normal = match Dist::parse(r) {
                Ok(Dist::Uniform) => None,
                Ok(Dist::Normal { cut }) => Some(cut),
                _ => {
                    return Err(format!(
                        "radial は true・false・uniform・normal・normal:σの数（{r}）"
                    ))
                }
            };
            tolerance::pair_disc(&mut tols, normal);
        }
        Some(x) => {
            return Err(format!(
                "radial は true・false・uniform・normal・normal:σの数（{x}）"
            ))
        }
    }
    let mut spec = McSpec::new(p, tols);
    if let Some(x) = get("trials") {
        spec.trials = x.as_u64().ok_or("trials は 0 以上の整数")? as usize;
    }
    if let Some(x) = get("seed") {
        spec.seed = x.as_u64().ok_or("seed は 0 以上の整数")?;
    }
    if let Some(x) = get("comp_iters") {
        spec.comp = DlsOpts {
            max_iter: x.as_u64().ok_or("comp_iters は整数")? as usize,
            ..spec.comp
        };
    }
    if let Some(x) = get("sensitivity") {
        spec.sensitivity = x.as_bool().ok_or("sensitivity は真偽")?;
    }
    Ok(spec)
}

/// 設定の JSON の百分位・samples・threads
pub struct OutOpts {
    pub percentiles: Vec<f64>,
    pub samples: bool,
    pub threads: Option<usize>,
}

pub fn out_opts(v: &Value) -> Result<OutOpts, String> {
    let g = |k: &str| v.get(k).filter(|x| !x.is_null());
    let percentiles = match g("percentiles") {
        None => vec![50.0, 80.0, 90.0, 95.0, 98.0],
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| num(x, "percentiles"))
            .collect::<Result<_, _>>()?,
        Some(x) => return Err(format!("percentiles は数の並び（{x}）")),
    };
    Ok(OutOpts {
        percentiles,
        samples: g("samples").and_then(|x| x.as_bool()).unwrap_or(false),
        threads: g("threads").and_then(|x| x.as_u64()).map(|x| x as usize),
    })
}

fn stats_json(v: &[f64], pcts: &[f64]) -> Value {
    let st = tolerance::stats(v, pcts);
    let f = |x: f64| if x.is_finite() { json!(x) } else { Value::Null };
    json!({
        "n": st.n, "mean": f(st.mean), "std": f(st.std), "min": f(st.min), "max": f(st.max),
        "percentiles": st.percentiles.iter().map(|(p, x)| json!([p, f(*x)])).collect::<Vec<_>>(),
    })
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

/// 項の説明（"wavefront f1" など）
fn term_label(p: &Problem, k: usize) -> String {
    let t = &p.terms[k];
    let s = format!("{:?}", t.kind);
    let name = s
        .split([' ', '{', '('])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if p.configs.len() > 1 {
        format!("{name} #{} (config {})", k + 1, t.config + 1)
    } else {
        format!("{name} #{}", k + 1)
    }
}

/// 結果の JSON
pub fn result_json(spec: &McSpec, r: &McResult, o: &OutOpts) -> Value {
    let s0 = &spec.problem.configs[0];
    let pc = &o.percentiles;
    let merits = r.merits();
    let nm = r.nominal.merit;
    let tols: Vec<Value> = spec
        .tols
        .iter()
        .map(|t| {
            json!({
                "label": t.label,
                "params": t.params.iter().map(|&p| s0.param_name(p)).collect::<Vec<_>>(),
                "lo": t.lo, "hi": t.hi, "relative": t.relative, "dist": t.dist.name(),
                "scale": if t.scale.is_empty() { Value::Null } else { json!(t.scale) },
            })
        })
        .collect();
    let eval_json = |e: &Option<tolerance::Eval>| match e {
        Some(e) => json!({ "merit": e.merit, "terms": e.terms, "comp": e.comp }),
        None => Value::Null,
    };
    let mut sens: Vec<(f64, Value)> = r
        .sensitivity
        .iter()
        .map(|x| {
            let t = &spec.tols[x.tol];
            let d = |e: &Option<tolerance::Eval>| e.as_ref().map(|e| e.merit - nm);
            let w = x.worst(nm);
            (
                w,
                json!({
                    "label": t.label, "lo": t.lo, "hi": t.hi,
                    "merit_lo": x.lo.as_ref().map(|e| e.merit), "merit_hi": x.hi.as_ref().map(|e| e.merit),
                    "change_lo": d(&x.lo), "change_hi": d(&x.hi),
                    "worst": if w.is_finite() { json!(w) } else { Value::Null },
                }),
            )
        })
        .collect();
    sens.sort_by(|a, b| b.0.total_cmp(&a.0));
    // 感度の RSS による見積り（公称 + √Σ 変化²。片側の悪い方を使う）
    let rss = if r.sensitivity.is_empty() {
        Value::Null
    } else {
        let s2: f64 = sens.iter().map(|x| x.0 * x.0).sum();
        json!(nm + s2.sqrt())
    };
    let corr = r.correlation(&spec.tols);
    let fin = |x: f64| if x.is_finite() { json!(x) } else { Value::Null };
    let mut cv: Vec<(f64, Value)> = corr
        .iter()
        .zip(&spec.tols)
        .map(|(&(r, ra), t)| {
            let k = [r.abs(), ra.abs()]
                .into_iter()
                .filter(|x| x.is_finite())
                .fold(f64::NEG_INFINITY, f64::max);
            (
                k,
                json!({ "label": t.label, "r": fin(r), "r_abs": fin(ra) }),
            )
        })
        .collect();
    cv.sort_by(|a, b| b.0.total_cmp(&a.0));
    let terms: Vec<Value> = (0..spec.problem.terms.len())
        .map(|k| {
            let v = r.term(k);
            json!({
                "label": term_label(&spec.problem, k),
                "nominal": r.nominal.terms.get(k),
                "stats": stats_json(&v, pc),
                "cdf": sorted(v),
            })
        })
        .collect();
    let comps: Vec<Value> = spec
        .problem
        .vars
        .iter()
        .enumerate()
        .map(|(q, v)| {
            let x: Vec<f64> = r
                .trials
                .iter()
                .filter_map(|t| t.eval.as_ref().map(|e| e.comp[q]))
                .collect();
            json!({
                "name": s0.param_name(v.param),
                "configs": v.configs.as_ref().map(|c| c.iter().map(|k| k + 1).collect::<Vec<_>>()),
                "nominal": r.nominal.comp.get(q),
                "stats": stats_json(&x, pc),
            })
        })
        .collect();
    let failed: Vec<&tolerance::Trial> = r.trials.iter().filter(|t| t.eval.is_none()).collect();
    let mut errors: Vec<String> = Vec::new();
    for t in &failed {
        if let Some(e) = &t.error {
            if !errors.contains(e) && errors.len() < 5 {
                errors.push(e.clone());
            }
        }
    }
    let mut out = json!({
        "nominal": eval_json(&Some(r.nominal.clone())),
        "trials": r.trials.len(),
        "failed": failed.len(),
        "errors": errors,
        "cancelled": r.cancelled,
        "seed": spec.seed,
        "tolerances": tols,
        "merit": stats_json(&merits, pc),
        "cdf": sorted(merits),
        "terms": terms,
        "compensators": comps,
        "sensitivity": sens.into_iter().map(|x| x.1).collect::<Vec<_>>(),
        "rss_estimate": rss,
        "correlation": cv.into_iter().map(|x| x.1).collect::<Vec<_>>(),
    });
    if o.samples {
        out["samples"] = r
            .trials
            .iter()
            .map(|t| json!({ "deltas": t.deltas, "eval": eval_json(&t.eval), "error": t.error }))
            .collect();
    }
    out
}
