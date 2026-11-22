//! ガラスの置き換え: 面の材料をカタログの実在の硝材から選ぶ（OpticStudio のガラス置換・Hammer のガラスの入れ替えに当たる）。
//!
//! 置き換える面ごとに、今の媒質の実効の nd・vd（モデルガラスの値、または硝材に屈折率のずれ dn・分散の変化 disp を
//! かけた値。d・F・C 線の屈折率から、系の温度・気圧で）に近い硝材を候補の表から [`GlassSubOpts::candidates`] 個選び、
//! それぞれに入れ替えた系で残りの変数を局所の最適化（[`lensbench_optim::dls`]）にかけ、評価関数の最も良い硝材を残す。
//! 近さは (Δnd / nd_step)² + (Δvd / vd_step)²（既定は nd の 0.01 を vd の 1 と同じに数える。[`Closeness`]）で、
//! dpgf_step を与えれば (ΔPgF / dpgf_step)² も足す。
//!
//! 候補は絞り込み（[`GlassFilter`]、OpticStudio のガラス置換のテンプレートに当たる）に合う硝材だけ: 相対価格・耐性の等級
//! （CR・FR・SR・AR・PR）の上限、カタログの状態（標準・推奨・廃止・特殊・溶解）、除く硝材の名前。カタログで置き換えから
//! 除く印（Exclude Substitution）の付いた硝材は使わない。カタログに値の無い性質の上限では除かない。
//!
//! 2 つの経路をこの 1 つで扱う:
//! - モデルガラス（または dn・disp を変数にした硝材）で最適化した後、近い実在の硝材へ寄せる: モデルガラス・
//!   屈折率だけの媒質・dn か disp が 0 でない面は、最初の周で必ず候補のどれかに置き換える（今の材料は候補に入れない）
//! - 実在の硝材を候補と入れ替えて比べる: 硝材の面は今の硝材も候補に入れ、良くなるときだけ入れ替える
//!
//! 置き換えた面の dn・disp は 0 にし、その面の dn・disp の変数は以後の最適化から外す。面を順に 1 つずつ置き換え
//! （前の面の置き換えの結果から次の面を始める）、どの面も変わらなくなるまで [`GlassSubOpts::passes`] 周まで繰り返す。
//! 候補の評価は並列に回す。絞り込みに合わない今の硝材は、モデルガラスと同じく必ず置き換える。
//!
//! 多重構成で面の材料が構成によって違う（多重構成の表の材料の行、.zmx の GLSS）ときは、同じ材料の構成の組ごとに
//! 別々に置き換える（組ごとに目標の nd・vd を取り、その組の構成だけに入れる）。

use lensbench_core::error::{Error, Result};
use lensbench_core::glass::Glass;
use lensbench_core::material::Material;
use lensbench_core::param::{Kind, Param};
use lensbench_core::{invalid, par, System};
use lensbench_optim::{dls_inner, DlsOpts, DlsResult, Problem};
use std::collections::HashMap;
use std::sync::Arc;

/// d・F・C 線の波長 [µm]
const LD: f64 = 0.587_561_8;
const LF: f64 = 0.486_132_7;
const LC: f64 = 0.656_272_5;

/// ガラスの置き換えの設定
#[derive(Debug, Clone)]
pub struct GlassSubOpts {
    /// 置き換える面の番号
    pub surfaces: Vec<usize>,
    /// 候補の硝材（名前 → ガラス。名前は系の材料に書く名前）
    pub glasses: Arc<HashMap<String, Glass>>,
    /// 面ごとに比べる候補の数（近い順）
    pub candidates: usize,
    /// 面を一巡する回数の上限
    pub passes: usize,
    /// 候補ごとの局所の最適化
    pub local: DlsOpts,
    /// 候補の絞り込み
    pub filter: GlassFilter,
    /// 近さの重み
    pub closeness: Closeness,
}

/// 候補の絞り込み（OpticStudio のガラス置換のテンプレートに当たる）。カタログに値の無い性質の上限では除かない
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlassFilter {
    /// 相対価格の上限（AGF の OD の 1 番目）
    pub max_cost: Option<f64>,
    /// 耐性の等級の上限: 耐候性 CR・耐汚染性 FR・耐酸性 SR・耐アルカリ性 AR・耐リン酸塩性 PR（AGF の OD の 2〜6 番目）
    pub max_resist: [Option<f64>; 5],
    /// 使ってよい状態（0 標準・1 推奨・2 廃止・3 特殊・4 溶解。None はすべて）。カタログに状態の無い硝材は除かない
    pub status: Option<Vec<u8>>,
    /// 除く硝材の名前（大文字小文字は問わない。`名前@カタログ` ならそのカタログのものだけ）
    pub exclude: Vec<String>,
}

/// 状態の名前（AGF の NM の 8 番目の順）
pub const STATUS_NAMES: [&str; 5] = ["standard", "preferred", "obsolete", "special", "melt"];

/// 耐性の等級の名前（AGF の OD の 2〜6 番目の順）
pub const RESIST_NAMES: [&str; 5] = ["cr", "fr", "sr", "ar", "pr"];

impl GlassFilter {
    /// 状態の名前（[`STATUS_NAMES`]）を番号に
    pub fn status_of(name: &str) -> Result<u8> {
        let n = name.trim().to_ascii_lowercase();
        match STATUS_NAMES.iter().position(|x| *x == n) {
            Some(i) => Ok(i as u8),
            None => invalid!("glass status {name} (one of {})", STATUS_NAMES.join(", ")),
        }
    }

    /// 表の名前 key の硝材 g を候補にしてよいか
    pub fn allows(&self, key: &str, g: &Glass) -> bool {
        !g.no_substitute && self.passes(key, g)
    }

    /// 利用者の絞り込み（価格・耐性・状態・除く名前）に合うか。カタログの置き換えから除く印は見ない
    pub fn passes(&self, key: &str, g: &Glass) -> bool {
        let base = lensbench_core::glass::base_name(key);
        let excluded = self.exclude.iter().any(|e| {
            let e = e.trim();
            match e.split_once('@') {
                Some((n, c)) => n.eq_ignore_ascii_case(base) && c.eq_ignore_ascii_case(&g.catalog),
                None => e.eq_ignore_ascii_case(base) || e.eq_ignore_ascii_case(key),
            }
        });
        if excluded {
            return false;
        }
        if let (Some(ok), Some(st)) = (&self.status, g.status) {
            if !ok.contains(&st) {
                return false;
            }
        }
        if let (Some(m), Some(c)) = (self.max_cost, g.props.and_then(|p| p.cost())) {
            if c > m {
                return false;
            }
        }
        if let Some(r) = g.props.and_then(|p| p.resist) {
            for (lim, v) in self.max_resist.iter().zip(r) {
                if let (Some(lim), Some(v)) = (lim, v) {
                    if v > *lim {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// 絞り込みが何もしないか（カタログの置き換えから除く印は別）
    pub fn is_empty(&self) -> bool {
        *self == GlassFilter::default()
    }
}

/// 近さ (Δnd / nd_step)² + (Δvd / vd_step)² (+ (ΔPgF / dpgf_step)²) の刻み
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closeness {
    pub nd_step: f64,
    pub vd_step: f64,
    /// None なら ΔPgF は見ない
    pub dpgf_step: Option<f64>,
}

impl Default for Closeness {
    fn default() -> Self {
        Closeness {
            nd_step: 0.01,
            vd_step: 1.0,
            dpgf_step: None,
        }
    }
}

impl Closeness {
    fn check(&self) -> Result<()> {
        let ok = |x: f64| x.is_finite() && x > 0.0;
        if !ok(self.nd_step) || !ok(self.vd_step) || self.dpgf_step.is_some_and(|x| !ok(x)) {
            invalid!("the closeness steps (nd_step, vd_step, dpgf_step) must be positive");
        }
        Ok(())
    }

    fn distance2(&self, a: Nv, b: Nv) -> f64 {
        let mut d = ((a.0 - b.0) / self.nd_step).powi(2) + ((a.1 - b.1) / self.vd_step).powi(2);
        if let Some(s) = self.dpgf_step {
            d += ((a.2 - b.2) / s).powi(2);
        }
        d
    }
}

/// nd・vd・ΔPgF
type Nv = (f64, f64, f64);

/// 1 つの面の置き換え
#[derive(Debug, Clone, PartialEq)]
pub struct GlassChange {
    pub surface: usize,
    /// 前の材料の名前（モデルガラスは "nd,vd"。dn・disp があれば実効の値を target_nd・target_vd に）
    pub from: String,
    pub to: String,
    /// 置き換える前の実効の nd・vd
    pub target_nd: f64,
    pub target_vd: f64,
    /// 選んだ硝材の nd・vd
    pub nd: f64,
    pub vd: f64,
    /// 置き換えた後の評価関数の値
    pub merit: f64,
    /// 置き換えた構成（0 始まり。面の材料が構成で違うときの組。None はすべての構成）
    pub configs: Option<Vec<usize>>,
}

/// ガラスの置き換えの結果
#[derive(Debug, Clone)]
pub struct GlassSubResult {
    /// 置き換えた後の問題（構成は置き換えて最適化した系、変数は外した dn・disp を除いたもの）
    pub problem: Problem,
    /// 最後の局所の最適化の結果（x0・merit0 は置き換える前のもの）
    pub result: DlsResult,
    /// 置き換えた面（置き換えた順。同じ面が周ごとに何度か出ることもある）
    pub changes: Vec<GlassChange>,
    /// 評価した候補の数
    pub tried: usize,
    /// 回した周の数
    pub passes: usize,
}

/// 屈折率の関数から nd・vd
fn nd_vd(n: impl Fn(f64) -> f64) -> (f64, f64) {
    let (d, f, c) = (n(LD), n(LF), n(LC));
    (d, (d - 1.0) / (f - c))
}

/// 面 j の今の媒質の実効の nd・vd（dn・disp・dnd・dvd 込み）
pub fn effective_nd_vd(s: &System, j: usize) -> Result<(f64, f64)> {
    let d = &s.surfaces[j];
    if d.is_mirror() || d.is_coord_break() || d.grin().is_some() {
        invalid!("surface {j}: no glass to substitute (mirror, coordinate break or GRIN)");
    }
    if matches!(d.mat, Material::Air) {
        invalid!("surface {j}: the medium is air");
    }
    let lp = s.wavelengths[s.primary];
    // 硝材の nd・vd のずれ（dnd・dvd）込み
    let b = s
        .surface_medium_index_at(j, &[LD, LF, LC, lp])?
        .ok_or_else(|| format!("surface {j}: no medium"))?;
    let eff = |x: f64| x + d.dn + d.disp * (x - b[3]);
    let (nd, nf, nc) = (eff(b[0]), eff(b[1]), eff(b[2]));
    Ok((nd, (nd - 1.0) / (nf - nc)))
}

/// 面 j の今の媒質の ΔPgF（モデルガラスはその値、硝材はカタログの値。無ければ 0）
fn effective_dpgf(s: &System, j: usize) -> f64 {
    match &s.surfaces[j].mat {
        Material::Model { dpgf, .. } => *dpgf,
        Material::Glass(m) => s
            .glasses
            .get(m)
            .and_then(|g| g.props)
            .map_or(0.0, |p| p.dpgf),
        _ => 0.0,
    }
}

/// 硝材 g の系 s の温度・気圧での nd・vd（系の波長の範囲の外の硝材・求められない硝材は None）
fn glass_nd_vd(s: &System, g: &Glass) -> Option<(f64, f64)> {
    if lensbench_core::usermaterial::missing(g).is_some() {
        return None;
    }
    if let Some([lo, hi]) = g.ld {
        if s.wavelengths.iter().any(|&w| w < lo || w > hi) {
            return None;
        }
    }
    let (t, p, a) = (s.temperature, s.pressure, s.adjust_index);
    let (nd, vd) = nd_vd(|l| g.index(l, t, p, a));
    (nd.is_finite() && vd.is_finite() && nd > 1.0 && vd > 0.0).then_some((nd, vd))
}

/// 目標の nd・vd・ΔPgF に近い候補 n 個（名前の順で同じ近さを分ける）。絞り込みに合わない硝材と exclude の名前は除く
fn nearest(
    s: &System,
    o: &GlassSubOpts,
    target: Nv,
    n: usize,
    exclude: Option<&str>,
) -> Vec<(String, (f64, f64))> {
    let mut v: Vec<(f64, String, (f64, f64))> = o
        .glasses
        .iter()
        .filter(|(k, g)| Some(k.as_str()) != exclude && o.filter.allows(k, g))
        .filter_map(|(k, g)| {
            let (nd, vd) = glass_nd_vd(s, g)?;
            let dp = g.props.map_or(0.0, |p| p.dpgf);
            Some((
                o.closeness.distance2((nd, vd, dp), target),
                k.clone(),
                (nd, vd),
            ))
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    v.into_iter().take(n).map(|(_, k, nv)| (k, nv)).collect()
}

/// 面 j の材料を硝材 name（表 pool のガラス g）にし、dn・disp を 0 にした系と、材料に書いた名前。系に同じ名前で値の違う
/// ガラスがあれば（別のカタログの同じ名前など）、`名前@カタログ` の名前で足す
fn with_glass(s: &System, j: usize, name: &str, g: &Glass) -> Result<(System, String)> {
    let mut t = s.clone();
    let used = match t.glasses.get(name) {
        Some(h) if h == g => name.to_string(),
        None => {
            t.add_glasses(HashMap::from([(name.to_string(), g.clone())]));
            name.to_string()
        }
        Some(_) => {
            let q = lensbench_core::glass::qualified(
                lensbench_core::glass::base_name(name),
                &g.catalog,
            );
            if t.glasses.get(&q) != Some(g) {
                t.add_glasses(HashMap::from([(q.clone(), g.clone())]));
            }
            q
        }
    };
    t.set_material(j, &used)?;
    let d = &t.surfaces[j];
    let mut ch = Vec::new();
    if d.dn != 0.0 {
        ch.push((Param::new(j, Kind::Dn), 0.0));
    }
    if d.disp != 0.0 {
        ch.push((Param::new(j, Kind::Disp), 0.0));
    }
    if !ch.is_empty() {
        t = t.with_params(&ch)?;
    }
    if t.surfaces[j].dnd != 0.0 || t.surfaces[j].dvd != 0.0 {
        t.edit(|rx| {
            rx.surfaces[j].dnd = 0.0;
            rx.surfaces[j].dvd = 0.0;
            Ok(())
        })?;
    }
    Ok((t, used))
}

/// 面 j が必ず置き換える面か（モデルガラス・屈折率だけの媒質・dn・disp・dnd・dvd のある面・絞り込みに合わない硝材の面）
fn must_replace(s: &System, j: usize, o: &GlassSubOpts) -> bool {
    let d = &s.surfaces[j];
    let filtered = match &d.mat {
        Material::Glass(m) => {
            !o.filter.is_empty() && s.glasses.get(m).is_some_and(|g| !o.filter.passes(m, g))
        }
        _ => false,
    };
    matches!(d.mat, Material::Model { .. } | Material::Index(_))
        || d.dn != 0.0
        || d.disp != 0.0
        || d.dnd != 0.0
        || d.dvd != 0.0
        || filtered
}

/// 面 j の材料（名前と dn・disp・dnd・dvd）が同じ構成の組（0 始まり、最初の構成の順）
fn config_groups(p: &Problem, j: usize) -> Vec<Vec<usize>> {
    let key = |s: &System| {
        let d = &s.surfaces[j];
        (
            d.mat.name(),
            d.dn.to_bits(),
            d.disp.to_bits(),
            d.dnd.to_bits(),
            d.dvd.to_bits(),
        )
    };
    let mut out: Vec<(_, Vec<usize>)> = Vec::new();
    for (k, s) in p.configs.iter().enumerate() {
        let x = key(s);
        match out.iter_mut().find(|(y, _)| *y == x) {
            Some((_, v)) => v.push(k),
            None => out.push((x, vec![k])),
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// 面 j の dn・disp の変数を外した問題
fn drop_index_vars(p: &mut Problem, j: usize) {
    p.vars.retain(|v| {
        !(matches!(v.param.kind, Kind::Dn | Kind::Disp)
            && !v.param.kind.is_unit()
            && v.param.index() == j)
    });
}

/// 候補の表を作る: カタログの名前の並び（空なら系 s の読み込んだガラス）。カタログはフォルダ dirs、既定のフォルダ、
/// 同梱のものの順に探す
pub fn candidate_pool(
    s: &System,
    catalogs: &[String],
    dirs: &[std::path::PathBuf],
) -> Result<Arc<HashMap<String, Glass>>> {
    if catalogs.is_empty() {
        if s.glasses.is_empty() {
            invalid!("no candidate glasses: name the glass catalogs to choose from");
        }
        return Ok(s.glasses.clone());
    }
    let dirs = lensbench_core::glass::catalog_dirs(dirs);
    let (g, missing) = lensbench_core::glass::load_catalogs_shared(catalogs, &dirs);
    if !missing.is_empty() {
        invalid!("glass catalogs not found: {}", missing.join(", "));
    }
    if g.is_empty() {
        invalid!("no candidate glasses in {}", catalogs.join(", "));
    }
    Ok(g)
}

/// ガラスの置き換え。p の変数の値 x から始める。説明はモジュールの頭
pub fn substitute(p: &Problem, x: &[f64], o: &GlassSubOpts) -> Result<GlassSubResult> {
    p.check()?;
    if o.surfaces.is_empty() {
        invalid!("no surfaces to substitute glass on");
    }
    if o.candidates == 0 {
        invalid!("candidates must be 1 or more");
    }
    if o.passes == 0 {
        invalid!("passes must be 1 or more");
    }
    o.closeness.check()?;
    if !o.glasses.iter().any(|(k, g)| o.filter.allows(k, g)) {
        invalid!("no candidate glass passes the filter");
    }
    let s0 = &p.configs[0];
    for (i, &j) in o.surfaces.iter().enumerate() {
        if j == 0 || j >= s0.image() {
            invalid!(
                "surface {j} to substitute glass on (1 to {})",
                s0.image() - 1
            );
        }
        if o.surfaces[..i].contains(&j) {
            invalid!("surface {j} given twice");
        }
        for s in &p.configs {
            effective_nd_vd(s, j)?;
        }
    }
    // 始める系（変数の値 x）とその評価
    let mut cur = p.with_configs(p.systems(x)?);
    let x0 = x.to_vec();
    let merit0 = p.merit(x)?;
    let mut best: Option<DlsResult> = None;
    let mut cur_merit = merit0;
    let mut changes = Vec::new();
    let mut tried = 0;
    let mut passes = 0;
    for _ in 0..o.passes {
        passes += 1;
        let mut changed = false;
        for &j in &o.surfaces {
            for group in config_groups(&cur, j) {
                par::check_cancel()?;
                let s = &cur.configs[group[0]];
                let forced = must_replace(s, j, o);
                let (tnd, tvd) = effective_nd_vd(s, j)?;
                let target = (tnd, tvd, effective_dpgf(s, j));
                let from = s.surfaces[j].mat.name();
                let keep = match &s.surfaces[j].mat {
                    Material::Glass(m) if !forced => Some(m.clone()),
                    _ => None,
                };
                let cands = nearest(s, o, target, o.candidates, keep.as_deref());
                if cands.is_empty() {
                    return Err(Error::Invalid(format!(
                    "surface {j}: no candidate glass passes the filter and covers the system wavelengths"
                )));
                }
                let mut q = cur.clone();
                // 面の dn・disp の変数は、どの構成でもこの面の硝材を置き換えれば外す（変数は構成で共有のことがある）
                drop_index_vars(&mut q, j);
                let evals: Vec<Option<(DlsResult, Problem, String)>> =
                    par::map(&cands, |(name, _)| {
                        let g = &o.glasses[name];
                        let mut used = String::new();
                        let mut cs = Vec::with_capacity(q.configs.len());
                        for (k, c) in q.configs.iter().enumerate() {
                            if group.contains(&k) {
                                let (t, u) = with_glass(c, j, name, g).ok()?;
                                used = u;
                                cs.push(t);
                            } else {
                                cs.push(c.clone());
                            }
                        }
                        let qq = q.with_configs(cs);
                        let r = dls_inner(&qq, &o.local).ok()?;
                        r.merit.is_finite().then_some((r, qq, used))
                    });
                tried += cands.len();
                let pick = evals
                    .into_iter()
                    .zip(&cands)
                    .filter_map(|(e, c)| e.map(|e| (e, c)))
                    .min_by(|a, b| a.0 .0.merit.total_cmp(&b.0 .0.merit));
                let Some(((r, qq, used), (_, nv))) = pick else {
                    if forced {
                        return Err(Error::Invalid(format!(
                            "surface {j}: no candidate glass could be traced and optimized"
                        )));
                    }
                    continue;
                };
                // 硝材の面は良くなるときだけ（モデルガラス・dn・disp の面は必ず。置き換えた後は硝材の面になる）
                if !forced && !(r.merit < cur_merit * (1.0 - 1e-9)) {
                    continue;
                }
                changes.push(GlassChange {
                    surface: j,
                    from,
                    to: used,
                    target_nd: target.0,
                    target_vd: target.1,
                    nd: nv.0,
                    vd: nv.1,
                    merit: r.merit,
                    configs: (group.len() < cur.configs.len()).then(|| group.clone()),
                });
                cur_merit = r.merit;
                cur = qq.with_configs(r.systems.clone());
                best = Some(r);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut result = match best {
        Some(r) => r,
        // 何も変わらなければ、始めの値のまま局所の最適化をかけない結果
        None => {
            let systems = p.systems(x)?;
            DlsResult {
                x0: x0.clone(),
                x: x0.clone(),
                merit0,
                merit: merit0,
                history: vec![merit0],
                iterations: 0,
                evals: 1,
                stop: lensbench_optim::Stop::NoProgress,
                note: None,
                inactive: Vec::new(),
                systems,
            }
        }
    };
    result.merit0 = merit0;
    Ok(GlassSubResult {
        problem: cur,
        result,
        changes,
        tried,
        passes,
    })
}
