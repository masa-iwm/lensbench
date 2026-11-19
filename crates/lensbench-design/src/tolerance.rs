//! モンテカルロ公差の設定の JSON と、結果の JSON。CLI の `tolerance`・serve の `tolerance`・Python の
//! `System.tolerance` で同じ形を使う。計算は [`crate::montecarlo`]。
//!
//! 設定は [`crate::tol_spec::TolSpec`] の型で読む（スキーマ `docs/schema/tolerance.schema.json`、知らないキーは誤り）。
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
//! | `wedge_face` | くさびを入れる面（"back"）: "back" は玉の後ろの面、"front" は前の面。前・後ろは玉のもとの向き（カタログ・.zmx の向き）で、ブロックを裏返して置いても（`reversed`）、鏡で往復しても同じ面。玉ごとには `lenses` の {"wedge_face": "front"} |
//! | `catalog` | カタログの玉（プロジェクトのレンズカタログのブロック）はカタログの玉の公差（[`catalog_lens_tol`]、`catalog_tols`）を既定にする（true） |
//! | `catalog_tols` | メーカーごとの公差 {"メーカー": {玉の公差と外径・焦点距離の条件つきの rules}}（[`catalog_rule`]。名前は大文字小文字・空白・記号を無視して、ブロックのメーカー名に含まれれば合う。複数が合えばいちばん長い名前。"*" はほか） |
//! | `lenses` | 玉ごとの上書き {"名前": {玉の公差} か 組の名前 か null（公差なし）}。名前はブロックの id か lens1 … |
//! | `groups` | 群・ブロックの公差 {"名前": {"decenter", "tilt", "shift"}}。玉 1 つのブロックでは玉の公差（`lenses`）と別に足す（組立の公差。名前は「G1 group decenter x」など） |
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
//! 玉の公差 {"radius_rel", "focal_rel", "thickness", "wedge", "wedge_face", "dn", "disp", "decenter", "tilt", "space"}
//! （[`LensTol`]。比・mm・deg。書いた項目だけ既定の組を上書きする）。
//!
//! 個別の公差: {"param": "3.t"} か {"params": ["2.r", "3.r"]} に、`pm`（±）か `lo`・`hi`、`relative`（比、false）、
//! `dist`、`label`、`scale`（パラメータごとにずれに掛ける数の並び。往復する面の厚さのように逆の符号で与えるとき -1）。
//!
//! 鏡の `back` で同じ玉を往復する系では、玉の公差の組は往きと戻りの面に同じ誤差を与える（曲率・くさびは同じ面、
//! 厚さ・間隔は戻りで逆の符号、偏心・傾きは往復の範囲を持つブロックの動き）。玉の名前はブロックの id。
//!
//! 光源の波長のずれは {"param": "wave", "pm": 0.01}（µm。すべての波長を試行ごとに同じだけずらし、全媒質の屈折率が
//! その波長の値になる）。1 つの波長だけなら "wave1" など（1 から）。

use std::collections::{HashMap, HashSet};

use crate::montecarlo::{self as tolerance, Dist, GroupTol, LensTol, McResult, McSpec, Tol};
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::catalog::VendorKey;
use lensbench_core::param::{Kind, Param};
use lensbench_core::System;
use lensbench_optim::{DefaultMerit, DlsOpts, Problem, Var};
use serde_json::{json, Value};

use crate::term_spec::RefName;
pub use crate::tol_report::ToleranceReport;
use crate::tol_report::{
    fin, CompStats, CorrInfo, Sample, SensInfo, StatsInfo, TermStats, TiltStats, TolInfo,
};
use crate::tol_spec::{self, CatEntry, Face, MeritName, Radial, TolItem, TolSpec, VarSpec};
use lensbench_project::lenient;

use lensbench_project::{Block, LensSource, Project};

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
) -> lensbench_core::Result<Option<LensTol>> {
    let e: Option<CatEntry> = lenient::from_json(v, what)?;
    cat_entry(e.as_ref(), base, g, what)
}

/// [`catalog_rule`] の型にした値
fn cat_entry(
    e: Option<&CatEntry>,
    base: LensTol,
    g: LensGeom,
    what: &str,
) -> lensbench_core::Result<Option<LensTol>> {
    Ok(match e {
        None => None,
        Some(CatEntry::Preset(p)) => Some(tol_spec::preset(p, base).map_err(|e| e.at(what))?),
        Some(CatEntry::Fields(f)) => {
            let mut t = f.apply(base);
            for r in f.rules.iter().flatten() {
                if r.matches(g.dia, g.efl, g.glasses) {
                    t = r.fields().apply(t);
                }
            }
            Some(t)
        }
    })
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
            .filter(|d| d.mat.is_medium() && !d.mat.is_air())
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

/// 裏返して置いた玉のブロックの id（`reversed` の玉。[`spec_from_json_with`] の reversed に渡す）
pub fn reversed_lenses(p: &Project) -> HashSet<String> {
    fn walk(bs: &[Block], m: &mut HashSet<String>) {
        for b in bs {
            match b {
                Block::Lens {
                    id, reversed: true, ..
                } if !id.is_empty() => {
                    m.insert(id.clone());
                }
                Block::Group { blocks, .. } => walk(blocks, m),
                _ => {}
            }
        }
    }
    let mut m = HashSet::new();
    walk(&p.blocks, &mut m);
    m
}

/// "9.t"・"G1.shift=-1:1"・{"param", "lo", "hi", "configs"}
fn var_from(v: &VarSpec, s: &System) -> lensbench_core::Result<Var> {
    let (spec, lo, hi, configs) = match v {
        VarSpec::Text(t) => match t.split_once('=') {
            Some((a, r)) => {
                let (l, h) = r
                    .split_once(':')
                    .ok_or_else(|| format!("the range of compensator {t} is min:max"))?;
                let n = |x: &str, d: f64| -> lensbench_core::Result<f64> {
                    if x.trim().is_empty() {
                        Ok(d)
                    } else {
                        x.trim()
                            .parse()
                            .map_err(|_| format!("range {x} of compensator {t}").into())
                    }
                };
                let (l, h) = (n(l, f64::NEG_INFINITY)?, n(h, f64::INFINITY)?);
                (a.trim().to_string(), l, h, None)
            }
            None => (t.trim().to_string(), f64::NEG_INFINITY, f64::INFINITY, None),
        },
        VarSpec::Obj(o) => {
            if o.configs.iter().flatten().any(|&k| k == 0) {
                return Err(format!(
                    "compensator {}: configs must be a list of configuration numbers (from 1)",
                    o.param
                )
                .into());
            }
            (
                o.param.clone(),
                o.lo.unwrap_or(f64::NEG_INFINITY),
                o.hi.unwrap_or(f64::INFINITY),
                o.configs
                    .as_ref()
                    .map(|c| c.iter().map(|k| k - 1).collect()),
            )
        }
    };
    let p = s.param(&spec)?;
    let v = Var::new(p).bounds(lo, hi);
    Ok(match configs {
        Some(c) => v.in_configs(c),
        None => v,
    })
}

fn tol_from(m: &TolItem, s: &System, dist: Dist, i: usize) -> lensbench_core::Result<Tol> {
    let names: Vec<String> = match (&m.param, &m.params) {
        (Some(p), None) => vec![p.clone()],
        (None, Some(a)) => a.clone(),
        _ => {
            return Err(format!("tolerance {i}: either param (a string) or params (a list)").into())
        }
    };
    let mut params = Vec::new();
    for n in &names {
        // "wave" はすべての波長（光源の波長のずれ。全波長を同じだけずらす）
        if n.trim().eq_ignore_ascii_case("wave") {
            params.extend((0..s.wavelengths.len()).map(|w| Param::new(0, Kind::Wave(w as u8))));
            continue;
        }
        params.push(
            s.param(n)
                .map_err(|e| e.map_message(|e| format!("tolerance {i}: {e}")))?,
        );
    }
    let (lo, hi) = match (m.pm, m.lo, m.hi) {
        (Some(p), None, None) => (-p.abs(), p.abs()),
        (None, Some(l), Some(h)) => (l, h),
        _ => return Err(format!("tolerance {i}: pm or the pair lo and hi").into()),
    };
    let dist = m.dist.unwrap_or(dist);
    let scale = m.scale.clone().unwrap_or_default();
    if !scale.is_empty() && scale.len() != params.len() {
        return Err(format!(
            "tolerance {i}: scale must be a list of {} numbers, as many as the parameters ({} given)",
            params.len(),
            scale.len()
        ).into());
    }
    Ok(Tol {
        label: m.label.clone().unwrap_or_else(|| names.join("+")),
        params,
        lo,
        hi,
        relative: m.relative.unwrap_or(false),
        dist,
        scale,
    })
}

/// 設定の JSON を型に読む（null は空の設定）。誤りには欄の道が付く
pub fn read_spec(v: &Value) -> lensbench_core::Result<TolSpec> {
    if v.is_null() {
        return Ok(TolSpec::default());
    }
    if !v.is_object() {
        return Err("tolerance settings must be an object".into());
    }
    lenient::from_json(v, "tolerance")
}

/// 設定の JSON から [`McSpec`] を作る。vendors はブロックの id → メーカー（[`catalog_vendors`]。.zmx なら空）。
/// 裏返した玉は系の玉・群の印（[`lensbench_core::units::Unit::flipped`]、プロジェクトの展開で付く）で分かる
pub fn spec_from_json(
    v: &Value,
    configs: Vec<System>,
    vendors: &HashMap<String, String>,
) -> lensbench_core::Result<McSpec> {
    spec_from_json_with(v, configs, vendors, &HashSet::new())
}

/// [`spec_from_json`] に、裏返して置いた玉の名前（ブロックの id、[`reversed_lenses`]）を足したもの。`wedge_face` の
/// 前・後ろは玉のもとの向き（カタログ・.zmx の向き）で決めるので、裏返した玉では系の中の面の順と逆になる。
/// 名前は、系の玉・群に裏返しの印が無いとき（印を持たない系）だけ使う
pub fn spec_from_json_with(
    v: &Value,
    configs: Vec<System>,
    vendors: &HashMap<String, String>,
    reversed: &HashSet<String>,
) -> lensbench_core::Result<McSpec> {
    let m = read_spec(v)?;
    let s0 = configs.first().ok_or("no configurations")?.clone();
    let dist = m.dist.unwrap_or(Dist::Normal { cut: 2.0 });
    let mut base = LensTol::preset(m.preset.as_deref().unwrap_or("commercial"))?;
    // ここでの wedge_first は玉のもとの向きで前の面か。系の面の順へは lens_fn で直す
    if let Some(w) = m.wedge_face {
        base.wedge_first = w == Face::Front;
    }
    let use_catalog = m.catalog.unwrap_or(true);
    // メーカー名（[`VendorKey::matches`]: 大文字小文字・空白・記号を無視して、ブロックのメーカー名に含まれれば合う。
    // 複数が合えばいちばん長い（具体的な）名前。"*" はほかのすべて）→ 公差
    let cat_over: Vec<(&String, VendorKey, &Option<CatEntry>)> = m
        .catalog_tols
        .iter()
        .flatten()
        .map(|(k, x)| (k, VendorKey::new(k), x))
        .collect();
    let find_cat = |vd: &str| -> Option<(&String, &Option<CatEntry>)> {
        let u = VendorKey::new(vd);
        cat_over
            .iter()
            .filter(|(k, key, _)| *k != "*" && !key.is_empty() && u.matches(key))
            .max_by_key(|(_, key, _)| key.as_str().len())
            .or_else(|| cat_over.iter().find(|(k, _, _)| *k == "*"))
            .map(|(k, _, x)| (*k, *x))
    };
    let geom = lens_geometry(&s0);
    let lens_base = |name: &str| -> lensbench_core::Result<LensTol> {
        match vendors.get(name).filter(|_| use_catalog) {
            Some(vd) => {
                let g = geom.get(name).copied().unwrap_or(LensGeom::UNKNOWN);
                let t = catalog_lens_tol(base);
                match find_cat(vd) {
                    Some((k, x)) => {
                        let w = format!("catalog_tols.{k}");
                        Ok(cat_entry(x.as_ref(), t, g, &w)?.unwrap_or_default())
                    }
                    None => Ok(t),
                }
            }
            None => Ok(base),
        }
    };
    let mut lens_over: HashMap<String, Option<LensTol>> = HashMap::new();
    for (k, x) in m.lenses.iter().flatten() {
        let b = lens_base(k)?;
        let t = x.as_ref().map(|x| x.apply(b)).transpose();
        lens_over.insert(k.clone(), t.map_err(|e| e.at(format!("lenses.{k}")))?);
    }
    let group_over: HashMap<String, Option<GroupTol>> = m
        .groups
        .iter()
        .flatten()
        .map(|(k, x)| (k.clone(), *x))
        .collect();
    // 名前の誤りを見つける
    for k in lens_over.keys().chain(group_over.keys()) {
        if s0.find_unit(k).is_none() {
            return Err(format!("no lens or group {k}").into());
        }
    }
    let base_err = std::cell::RefCell::new(None);
    let lens_fn = |name: &str| -> Option<LensTol> {
        let t = match lens_over.get(name) {
            Some(t) => *t,
            None => match lens_base(name) {
                Ok(t) => Some(t),
                Err(e) => {
                    base_err.borrow_mut().get_or_insert(e);
                    None
                }
            },
        };
        // 裏返した玉は、もとの向きの後ろの面が系の中では最初の面。系の玉・群に裏返しの印（Unit::flipped）があれば
        // simple が直すので、ここでは印の無い系に名前で渡されたときだけ直す
        let marked = s0.units.iter().any(|v| v.name == name && v.flipped);
        t.map(|t| LensTol {
            wedge_first: t.wedge_first != (reversed.contains(name) && !marked),
            ..t
        })
    };
    let group_fn = |name: &str| group_over.get(name).copied().flatten();
    let mut tols = tolerance::simple(&s0, &lens_fn, &group_fn, dist);
    if let Some(e) = base_err.into_inner() {
        return Err(e);
    }
    for (i, x) in m.tols.iter().flatten().enumerate() {
        tols.push(tol_from(x, &s0, dist, i + 1)?);
    }
    let mut p = Problem::new(configs);
    for x in m.compensators.iter().flat_map(|c| c.list()) {
        p.vars.push(var_from(x, &s0)?);
    }
    let rings = m.rings.unwrap_or(3);
    // 名前は CLI・Python と同じ reference
    let reference = match m.reference.unwrap_or_default() {
        RefName::Centroid => RmsRef::Centroid,
        RefName::Chief => RmsRef::Chief,
    };
    match m.merit.unwrap_or(MeritName::Wavefront) {
        MeritName::Wavefront => p.add_default(DefaultMerit::Wavefront, rings, reference),
        MeritName::Spot => p.add_default(DefaultMerit::Spot, rings, reference),
        MeritName::None => {}
    }
    if let Some(t) = &m.terms {
        let ts = crate::merit::terms_from_json(t, &p.configs)?;
        p.terms.extend(ts);
    }
    match &m.radial {
        None | Some(Radial::On(false)) => {}
        Some(Radial::On(true)) => {
            tolerance::pair_disc(&mut tols, None);
        }
        Some(Radial::Dist(r)) => {
            let normal = match Dist::parse(r) {
                Ok(Dist::Uniform) => None,
                Ok(Dist::Normal { cut }) => Some(cut),
                _ => {
                    return Err(format!(
                        "radial must be true, false, uniform, normal or normal:σ ({r})"
                    )
                    .into())
                }
            };
            tolerance::pair_disc(&mut tols, normal);
        }
    }
    let mut spec = McSpec::new(p, tols);
    spec.trials = m.trials.unwrap_or(spec.trials);
    spec.seed = m.seed.unwrap_or(spec.seed);
    if let Some(x) = m.comp_iters {
        spec.comp = DlsOpts {
            max_iter: x,
            ..spec.comp
        };
    }
    spec.sensitivity = m.sensitivity.unwrap_or(spec.sensitivity);
    Ok(spec)
}

/// 公差の評価を書く一括実行のファイル（[`crate::montecarlo::run_to`]）の頭。設定 v（試行の数・出力だけの指定
/// trials・threads・percentiles・samples は除く。試行を足して続きから回せる）・系の hash・玉のメーカーと裏返しから作る
pub fn runs_header(
    v: &Value,
    configs: &[System],
    vendors: &HashMap<String, String>,
    reversed: &HashSet<String>,
) -> lensbench_core::Result<String> {
    let mut settings = v.clone();
    if let Some(m) = settings.as_object_mut() {
        for k in ["trials", "threads", "percentiles", "samples"] {
            m.remove(k);
        }
    }
    let systems = configs
        .iter()
        .map(lensbench_project::sysfile::system_hash)
        .collect::<lensbench_core::Result<Vec<_>>>()?;
    let vendors: std::collections::BTreeMap<_, _> = vendors.iter().collect();
    let mut reversed: Vec<_> = reversed.iter().collect();
    reversed.sort();
    Ok(json!({
        "lensbench": lensbench_core::VERSION,
        "kind": "tolerance",
        "settings": settings,
        "systems": systems,
        "vendors": vendors,
        "reversed": reversed,
    })
    .to_string())
}

/// 設定の JSON の百分位・samples・threads
pub struct OutOpts {
    pub percentiles: Vec<f64>,
    pub samples: bool,
    pub threads: Option<usize>,
}

pub fn out_opts(v: &Value) -> lensbench_core::Result<OutOpts> {
    let m = read_spec(v)?;
    Ok(OutOpts {
        percentiles: m
            .percentiles
            .unwrap_or_else(|| vec![50.0, 80.0, 90.0, 95.0, 98.0]),
        samples: m.samples.unwrap_or(false),
        threads: m.threads,
    })
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

/// 項の説明（"wavefront f1" など）
/// 項の種類の名前（評価関数の JSON の type と同じ）。型の名前（Debug）から作らない
fn kind_name(k: &lensbench_optim::TermKind) -> &'static str {
    use lensbench_optim::TermKind as K;
    match k {
        K::Spot { .. } => "spot",
        K::Wavefront { .. } => "wavefront",
        K::Mtf { .. } => "mtf",
        K::Paraxial { .. } => "paraxial",
        K::Cardinal { .. } => "cardinal",
        K::FNum { .. } => "fnum",
        K::Seidel { .. } => "seidel",
        K::Distortion { .. } => "distortion",
        K::Energy { .. } => "energy",
        K::Strehl { .. } => "strehl",
        K::Param { .. } => "param",
        K::Spread { .. } => "spread",
        K::Illum { .. } => "illum",
        K::Longitudinal { .. } => "longitudinal",
        K::LateralColor { .. } => "lateral_color",
        K::SurfEach {
            q: lensbench_optim::SurfQ::Semi,
            ..
        } => "semi",
        K::SurfEach { .. } => "surface",
        K::Centroid { .. } => "centroid",
        K::Ray { .. } => "ray",
        K::RayPath { .. } => "ray",
        K::ParaxRay { .. } => "ray",
        K::Opd { .. } => "opd",
        K::Zernike { .. } => "zernike",
        K::Coupling { .. } => "coupling",
        K::Thickness { .. } => "thickness",
        K::Surface { .. } => "surface",
        K::Volume { .. } => "volume",
        K::Calc { .. } => "calc",
        K::User(_) => "user",
    }
}

fn term_label(p: &Problem, k: usize) -> String {
    let t = &p.terms[k];
    let name = kind_name(&t.kind);
    if p.configs.len() > 1 {
        format!("{name} #{} (config {})", k + 1, t.config + 1)
    } else {
        format!("{name} #{}", k + 1)
    }
}

/// 補償子の傾きの x・y の組: 同じ玉・群・面の `.tx` と `.ty` で、効く構成も同じもの。(名前, x の位置, y の位置)。
/// 名前は「M.tilt」（面なら「5.tilt」）
pub fn tilt_pairs(vars: &[Var], names: &[String]) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for (qx, nx) in names.iter().enumerate() {
        let Some(base) = nx.strip_suffix(".tx") else {
            continue;
        };
        let ny = format!("{base}.ty");
        if let Some(qy) = names
            .iter()
            .enumerate()
            .position(|(q, n)| *n == ny && vars[q].configs == vars[qx].configs)
        {
            out.push((format!("{base}.tilt"), qx, qy));
        }
    }
    out
}

/// 試行ごとの補償子の CSV（[`result_json`] の結果から）。列は 試行（1 から）・評価関数・補償子・傾きの大きさ [分]。
/// 追跡できなかった試行は値を空にする
pub fn compensators_csv(result: &Value) -> lensbench_core::Result<String> {
    let comps = result["compensators"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let tilts = result["tilt_magnitudes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let merits = result["merit_values"]
        .as_array()
        .ok_or("the result has no per-trial values (merit_values)")?;
    let n = merits.len();
    let cols: Vec<(String, &Vec<Value>)> = comps
        .iter()
        .map(|c| (c["name"].as_str().unwrap_or("").to_string(), &c["values"]))
        .chain(tilts.iter().map(|t| {
            (
                format!("{} [arcmin]", t["name"].as_str().unwrap_or("")),
                &t["values"],
            )
        }))
        .map(|(h, v)| {
            v.as_array()
                .filter(|a| a.len() == n)
                .map(|a| (h.clone(), a))
                .ok_or_else(|| format!("no per-trial values of compensator {h}"))
        })
        .collect::<Result<_, _>>()?;
    let esc = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let cell = |v: &Value| v.as_f64().map_or(String::new(), |x| format!("{x}"));
    let mut out = String::from("trial,merit");
    for (h, _) in &cols {
        out.push(',');
        out.push_str(&esc(h));
    }
    out.push('\n');
    for i in 0..n {
        out.push_str(&format!("{},{}", i + 1, cell(&merits[i])));
        for (_, a) in &cols {
            out.push(',');
            out.push_str(&cell(&a[i]));
        }
        out.push('\n');
    }
    Ok(out)
}

/// 結果の JSON（[`report`] を JSON に）
pub fn result_json(spec: &McSpec, r: &McResult, o: &OutOpts) -> Value {
    serde_json::to_value(report(spec, r, o)).unwrap_or_default()
}

fn stats_info(v: &[f64], pcts: &[f64]) -> StatsInfo {
    let st = tolerance::stats(v, pcts);
    StatsInfo {
        n: st.n,
        mean: fin(st.mean),
        std: fin(st.std),
        min: fin(st.min),
        max: fin(st.max),
        percentiles: st.percentiles.iter().map(|&(p, x)| (p, fin(x))).collect(),
    }
}

/// 結果（CLI・serve・GUI で同じ型）
pub fn report(spec: &McSpec, r: &McResult, o: &OutOpts) -> ToleranceReport {
    let s0 = &spec.problem.configs[0];
    let pc = &o.percentiles;
    let merits = r.merits();
    let nm = r.nominal.merit;
    let tolerances = spec
        .tols
        .iter()
        .map(|t| TolInfo {
            label: t.label.clone(),
            params: t.params.iter().map(|&p| s0.param_name(p)).collect(),
            lo: t.lo,
            hi: t.hi,
            relative: t.relative,
            dist: t.dist.name(),
            scale: (!t.scale.is_empty()).then(|| t.scale.clone()),
        })
        .collect();
    let mut sens: Vec<(f64, SensInfo)> = r
        .sensitivity
        .iter()
        .map(|x| {
            let t = &spec.tols[x.tol];
            let d = |e: &Option<tolerance::Eval>| e.as_ref().map(|e| e.merit - nm);
            let w = x.worst(nm);
            (
                w,
                SensInfo {
                    label: t.label.clone(),
                    lo: t.lo,
                    hi: t.hi,
                    merit_lo: x.lo.as_ref().map(|e| e.merit),
                    merit_hi: x.hi.as_ref().map(|e| e.merit),
                    change_lo: d(&x.lo),
                    change_hi: d(&x.hi),
                    worst: fin(w),
                },
            )
        })
        .collect();
    sens.sort_by(|a, b| b.0.total_cmp(&a.0));
    // 感度の RSS による見積り（公称 + √Σ 変化²。片側の悪い方を使う）
    let rss_estimate = (!r.sensitivity.is_empty()).then(|| {
        let s2: f64 = sens.iter().map(|x| x.0 * x.0).sum();
        nm + s2.sqrt()
    });
    let corr = r.correlation(&spec.tols);
    let mut cv: Vec<(f64, CorrInfo)> = corr
        .iter()
        .zip(&spec.tols)
        .map(|(&(r, ra), t)| {
            let k = [r.abs(), ra.abs()]
                .into_iter()
                .filter(|x| x.is_finite())
                .fold(f64::NEG_INFINITY, f64::max);
            (
                k,
                CorrInfo {
                    label: t.label.clone(),
                    r: fin(r),
                    r_abs: fin(ra),
                },
            )
        })
        .collect();
    cv.sort_by(|a, b| b.0.total_cmp(&a.0));
    let terms = (0..spec.problem.terms.len())
        .map(|k| {
            let v = r.term(k);
            TermStats {
                label: term_label(&spec.problem, k),
                nominal: r.nominal.terms.get(k).copied(),
                stats: stats_info(&v, pc),
                cdf: sorted(v),
            }
        })
        .collect();
    // 補償子の試行ごとの値（試行の順、追跡できなかった試行は None）
    let per_trial = |q: usize| -> Vec<Option<f64>> {
        r.trials
            .iter()
            .map(|t| t.eval.as_ref().map(|e| e.comp[q]))
            .collect()
    };
    let opt =
        |v: &[Option<f64>]| -> Vec<Option<f64>> { v.iter().map(|x| x.and_then(fin)).collect() };
    let names: Vec<String> = spec
        .problem
        .vars
        .iter()
        .map(|v| s0.param_name(v.param))
        .collect();
    let compensators = spec
        .problem
        .vars
        .iter()
        .enumerate()
        .map(|(q, v)| {
            let vals = per_trial(q);
            let x: Vec<f64> = vals.iter().flatten().copied().collect();
            CompStats {
                name: names[q].clone(),
                configs: v
                    .configs
                    .as_ref()
                    .map(|c| c.iter().map(|k| k + 1).collect()),
                nominal: r.nominal.comp.get(q).copied(),
                stats: stats_info(&x, pc),
                values: opt(&vals),
            }
        })
        .collect();
    // 傾きの x・y の組（同じ玉・群・面の tx と ty、効く構成も同じ）は大きさ √(tx² + ty²) を分で
    let tilt_magnitudes = tilt_pairs(&spec.problem.vars, &names)
        .into_iter()
        .map(|(name, qx, qy)| {
            let (vx, vy) = (per_trial(qx), per_trial(qy));
            let vals: Vec<Option<f64>> = vx
                .iter()
                .zip(&vy)
                .map(|(a, b)| Some(60.0 * a.as_ref()?.hypot(*b.as_ref()?)))
                .collect();
            let x: Vec<f64> = vals.iter().flatten().copied().collect();
            let nominal = match (r.nominal.comp.get(qx), r.nominal.comp.get(qy)) {
                (Some(a), Some(b)) => Some(60.0 * a.hypot(*b)),
                _ => None,
            };
            TiltStats {
                name,
                x: names[qx].clone(),
                y: names[qy].clone(),
                unit: "arcmin".into(),
                nominal,
                stats: stats_info(&x, pc),
                values: opt(&vals),
            }
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
    let merit_values = opt(&r
        .trials
        .iter()
        .map(|t| t.eval.as_ref().map(|e| e.merit))
        .collect::<Vec<_>>());
    ToleranceReport {
        nominal: Some(r.nominal.clone()),
        trials: r.trials.len(),
        failed: failed.len(),
        errors,
        cancelled: r.cancelled,
        seed: spec.seed,
        tolerances,
        merit: stats_info(&merits, pc),
        cdf: sorted(merits),
        terms,
        compensators,
        tilt_magnitudes,
        merit_values,
        sensitivity: sens.into_iter().map(|x| x.1).collect(),
        rss_estimate,
        correlation: cv.into_iter().map(|x| x.1).collect(),
        samples: o.samples.then(|| {
            r.trials
                .iter()
                .map(|t| Sample {
                    deltas: t.deltas.clone(),
                    eval: t.eval.clone(),
                    error: t.error.clone(),
                })
                .collect()
        }),
        notes: None,
    }
}
