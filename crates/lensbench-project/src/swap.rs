//! カタログレンズの差し替え探索: プロジェクトの玉のブロックを、焦点距離・径で絞ったカタログの候補で
//! 差し替えた組を並列に回し、組ごとに間隔（空気・群の移動）を減衰最小二乗（[`lensbench_core::optim`]）で
//! 最適化して、評価関数の小さい順に並べる。
//!
//! - 差し替える所（[`Slot`]）: 玉のブロックの id と、候補（型番の並びか、メーカー・焦点距離・径の範囲）。
//!   `flip` なら向き（reversed）を両方試す。
//! - 変数（[`SwapSpec::vars`]）: 展開した系のパラメータの書き方（`G2.shift` など、[`System::param`]）に加えて、
//!   id のある空気のブロックを `id.t` で指せる（その空気が入る面の厚さ。[`crate::expand_gaps`]）。範囲は
//!   `d1.t=0.5:40`。
//! - 組の番号 i（0 から [`SwapSearch::count`] 未満）は、差し替える所ごとの候補の番号を並べた混合基数
//!   （最後の所が最も速く回る）。[`SwapSearch::eval`] は組 i だけから決まる JSON を返す。
//!   [`SwapSearch::run_to`] は一括実行（[`lensbench_core::runs`]）で組ごとに JSONL へ書き出し、途中から再開できる。
//!
//! 評価関数は既定の像の項（全視野・多色の RMS スポットか RMS 波面）。[`SwapSpec::efl`] で系の焦点距離の目標も足せる
//! （そのときの `merit` は目標の項を含む。像だけの値は `image_merit`）。
//!
//! 候補の焦点距離はカタログの値（無ければ玉の .zmx の主波長の近軸の値）、径は面の半径の最大の 2 倍。

use crate::{expand_gaps, find_id, Env, Gap, Project};
use lensbench_core::catalog::LensCatalog;
use lensbench_core::elements::{self, Kind as EKind};
use lensbench_core::optim::{
    self, DefaultMerit, DlsOpts, ParaxQ, Problem, RayQ, Side, Stop, Term, TermKind, Var,
};
use lensbench_core::param::{Kind, Param};
use lensbench_core::sweep::RmsRef;
use lensbench_core::{glass, par, runs, zmx, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// 候補の絞り込み（省いた条件は絞らない）
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// メーカー（大文字・小文字を区別しない）
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
    /// 組ごとに最適化する変数（"d1.t"・"d1.t=0.5:40"・"G2.shift"）
    #[serde(default)]
    pub vars: Vec<String>,
    /// 評価関数: "spot" か "wavefront"
    #[serde(default = "spot")]
    pub merit: String,
    /// 基準: "centroid" か "chief"
    #[serde(default = "centroid")]
    pub reference: String,
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
    /// 組の数の上限（大きすぎる探索を誤って始めないため）
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
    pub state_vars: Vec<String>,
    /// 有効径の条件: 差し替える玉の各面で、各視野の瞳の縁の実光線（主波長、瞳の上下左右）が面の半径
    /// （カタログの玉の半直径）の内側にあることを片側の条件の項として足す（重みは像の項の重みの合計の 1e6 倍）
    #[serde(default)]
    pub clear_aperture: bool,
    /// ヤコビアン: "forward"（すべて前進差分、既定）・"fd"（すべて中心差分）・"auto"（解析的に求められる列が 16 個以上の項は解析的に、少ない項は中心差分）。
    /// 差し替え探索の変数（間隔・群の移動など少数）では差分の方が速い（実カタログ 7 万組で fd は auto の 4.3 倍）。
    /// forward は差分の追跡が中心差分の半分で済み、fd よりさらに約 1.4 倍速い（実カタログ 3,000 組で上位 300 組の顔ぶれ・順位は
    /// fd と同じ、評価関数の差は相対 3e-7 以下）
    #[serde(default = "forward")]
    pub jac: String,
    /// 粗いふるい（省けばなし）: 全組を光線の少ない評価関数で最適化し、評価関数の小さい組だけを今の評価で
    /// 最適化し直す。残した組の結果はふるいなしと同じ。残さなかった組は結果に出ない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prescreen: Option<Prescreen>,
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

fn spot() -> String {
    "spot".into()
}
fn centroid() -> String {
    "centroid".into()
}
fn three() -> usize {
    3
}
fn two() -> usize {
    2
}
fn thousand() -> usize {
    1000
}
fn forward() -> String {
    "forward".into()
}
fn thirty() -> usize {
    30
}
fn max_count() -> usize {
    100_000
}

impl SwapSpec {
    pub fn from_json(txt: &str) -> Result<SwapSpec, String> {
        let r = match crate::schema::strip_schema_key(txt) {
            Some(v) => serde_json::from_value(v),
            None => serde_json::from_str(txt),
        };
        r.map_err(|e| format!("差し替え探索の設定: {e}"))
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
) -> Result<(f64, f64, f64), String> {
    let raw = zmx::parse_text(zmx_text).map_err(|e| e.to_string())?;
    let s = zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            fixed_semi: true,
            ..Default::default()
        },
        &glass::catalog_dirs(glass_dirs),
        None,
    )
    .map_err(|e| e.to_string())?;
    let els = elements::split(&s).map_err(|e| e.to_string())?;
    let e = els
        .iter()
        .find(|e| e.kind == EKind::Lens)
        .ok_or("玉がない")?;
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
    x.is_finite() && r[0].map_or(true, |a| x >= a) && r[1].map_or(true, |b| x <= b)
}

/// レンズの要約（焦点距離・径・中心厚）。作れないレンズは None
type Info = Option<(f64, f64, f64)>;

/// メーカーの絞り込みに合うカタログのレンズの番号（カタログの順）
fn vendor_indices(cat: &LensCatalog, f: &Filter) -> Vec<usize> {
    let v = f.vendor.as_deref().map(|v| v.trim().to_uppercase());
    cat.lenses()
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            v.as_ref()
                .map_or(true, |v| l.vendor.trim().to_uppercase() == *v)
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
    base: Value,
    dir: PathBuf,
    glass_dirs: Vec<PathBuf>,
    cat: &'a LensCatalog,
    spec: SwapSpec,
    /// 所ごとの候補の JSON ポインタ（ブロックの位置）
    ptrs: Vec<String>,
    /// 所ごとの選び方
    pub picks: Vec<Vec<Pick>>,
    merit: DefaultMerit,
    reference: RmsRef,
}

impl<'a> SwapSearch<'a> {
    /// プロジェクト p（状態は spec.state）の玉を差し替える探索を作る。env.base・glass_dirs を使い、
    /// レンズは cat から引く（env.catalog は使わない）
    pub fn new(
        p: &Project,
        env: &Env,
        cat: &'a LensCatalog,
        spec: SwapSpec,
    ) -> Result<SwapSearch<'a>, String> {
        if spec.state.is_some() && !spec.states.is_empty() {
            return Err("state と states は同時に使えない".into());
        }
        // 綴りの誤りは組ごとの失敗にせず、始める前に弾く
        jac_mode(&spec.jac)?;
        if spec.states.is_empty() && !spec.state_vars.is_empty() {
            return Err("state_vars は states と一緒に使う".into());
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
            return Err("差し替える所がない".into());
        }
        if let Some(pr) = &spec.prescreen {
            if pr.rings == 0 || pr.keep == 0 {
                return Err("prescreen の rings・keep は 1 以上".into());
            }
        }
        let merit = match spec.merit.to_ascii_lowercase().as_str() {
            "spot" => DefaultMerit::Spot,
            "wavefront" | "wfe" => DefaultMerit::Wavefront,
            m => return Err(format!("merit {m}（spot・wavefront）")),
        };
        let reference = match spec.reference.to_ascii_lowercase().as_str() {
            "centroid" => RmsRef::Centroid,
            "chief" => RmsRef::Chief,
            r => return Err(format!("reference {r}（centroid・chief）")),
        };
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
        let mut ptrs = Vec::new();
        let mut picks = Vec::new();
        for (k, sl) in spec.slots.iter().enumerate() {
            if spec.slots[..k].iter().any(|x| x.block == sl.block) {
                return Err(format!("所 {} が 2 回ある", sl.block));
            }
            let mut hits = Vec::new();
            if !sl.block.is_empty() {
                find_id(&base["blocks"], "/blocks", &sl.block, &mut hits);
            }
            let ptr = match hits.as_slice() {
                [x] => x.clone(),
                [] => return Err(format!("ブロック {} がない", sl.block)),
                _ => return Err(format!("id {} のブロックが 2 つ以上ある", sl.block)),
            };
            let b = base.pointer(&ptr).unwrap();
            if b["type"] != "lens" {
                return Err(format!("ブロック {} は玉（type lens）でない", sl.block));
            }
            let rev0 = b["reversed"].as_bool().unwrap_or(false);
            let lenses = if let Some(idx) = &slot_idx[k] {
                filter_candidates(cat, &sl.filter, idx, &infos)
            } else {
                sl.parts
                    .iter()
                    .map(|q| {
                        let l = cat.get(&q.vendor, &q.part).ok_or_else(|| {
                            format!("レンズカタログに {} {} がない", q.vendor, q.part)
                        })?;
                        let (efl, diameter, thickness) = lens_info(&l.zmx, l.efl, &env.glass_dirs)
                            .map_err(|e| format!("{} {}: {e}", q.vendor, q.part))?;
                        Ok(Candidate {
                            vendor: l.vendor.clone(),
                            part: l.part.clone(),
                            efl,
                            diameter,
                            thickness,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
            };
            if lenses.is_empty() {
                return Err(format!("所 {}: 候補がない", sl.block));
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
            ptrs.push(ptr);
            picks.push(v);
        }
        let s = SwapSearch {
            base,
            dir: env.base.clone(),
            glass_dirs: env.glass_dirs.clone(),
            cat,
            merit,
            reference,
            ptrs,
            picks,
            spec,
        };
        let n = s.count_checked();
        match n {
            Some(n) if n <= s.spec.max_count => Ok(s),
            _ => Err(format!(
                "組が多すぎる（{}、上限 {}。max_count で上げられる）",
                n.map_or("桁あふれ".to_string(), |n| n.to_string()),
                s.spec.max_count
            )),
        }
    }

    fn count_checked(&self) -> Option<usize> {
        self.picks
            .iter()
            .try_fold(1usize, |a, v| a.checked_mul(v.len()))
    }

    /// 組の数
    pub fn count(&self) -> usize {
        self.count_checked().unwrap()
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
    pub fn project(&self, i: usize) -> Result<Project, String> {
        let mut v = self.base.clone();
        for (k, &c) in self.combo(i).iter().enumerate() {
            let pk = &self.picks[k][c];
            let b = v.pointer_mut(&self.ptrs[k]).unwrap();
            b["source"] = json!({ "vendor": pk.lens.vendor, "part": pk.lens.part });
            b["reversed"] = json!(pk.reversed);
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// 組 i の系と、最適化の問題
    pub fn problem(&self, i: usize) -> Result<(System, Problem), String> {
        self.problem_n(i).map(|(s, p, _)| (s, p))
    }

    /// [`Self::problem`] と、像の項（既定の評価関数）の数
    fn problem_n(&self, i: usize) -> Result<(System, Problem, usize), String> {
        self.problem_rings(i, self.spec.rings)
    }

    /// [`Self::problem_n`] の評価関数のリング数を rings にしたもの
    fn problem_rings(&self, i: usize, rings: usize) -> Result<(System, Problem, usize), String> {
        let p = self.project(i)?;
        let look = |v: &str, q: &str| self.cat.zmx(v, q);
        let env = Env {
            base: self.dir.clone(),
            glass_dirs: self.glass_dirs.clone(),
            catalog: Some(&look),
        };
        // 状態ごとの系（states が空なら 1 つ）
        let mut systems = Vec::new();
        let mut gapss = Vec::new();
        if self.spec.states.is_empty() {
            let (s, g) = expand_gaps(&p, &env)?;
            systems.push(s);
            gapss.push(g);
        } else {
            for n in &self.spec.states {
                let (s, g) = expand_gaps(&p.with_state(n)?, &env)?;
                systems.push(s);
                gapss.push(g);
            }
        }
        let s = systems[0].clone();
        let mut pr = Problem::new(systems.clone());
        pr.jac = jac_mode(&self.spec.jac)?;
        // 共通の変数（全状態）と、状態ごとの変数（状態 k だけに効く）
        let mut specs: Vec<(&String, Option<usize>)> =
            self.spec.vars.iter().map(|v| (v, None)).collect();
        for k in 0..self.spec.states.len() {
            specs.extend(self.spec.state_vars.iter().map(|v| (v, Some(k))));
        }
        for (v, cfg) in specs {
            let (spec, range) = match v.split_once('=') {
                Some((a, b)) => (a.trim(), Some(b)),
                None => (v.trim(), None),
            };
            let c = cfg.unwrap_or(0);
            let param = var_param(&systems[c], &gapss[c], spec)?;
            if cfg.is_none() {
                // 共通の変数は全状態で同じ面のパラメータでなければならない
                for (k, (sk, gk)) in systems.iter().zip(&gapss).enumerate().skip(1) {
                    if var_param(sk, gk, spec)? != param {
                        return Err(format!(
                            "変数 {spec} が状態 {} で別の面を指す（state_vars に）",
                            self.spec.states[k]
                        ));
                    }
                }
            }
            let mut var = Var::new(param);
            if let Some(k) = cfg {
                var.configs = Some(vec![k]);
            }
            if let Some(r) = range {
                let (lo, hi) = r
                    .split_once(':')
                    .ok_or_else(|| format!("変数 {v} の範囲は 下限:上限"))?;
                let num = |t: &str, d: f64| -> Result<f64, String> {
                    if t.trim().is_empty() {
                        Ok(d)
                    } else {
                        t.trim().parse().map_err(|_| format!("変数 {v} の範囲 {t}"))
                    }
                };
                var = var.bounds(num(lo, f64::NEG_INFINITY)?, num(hi, f64::INFINITY)?);
            }
            pr.vars.push(var);
        }
        pr.add_default(self.merit, rings, self.reference);
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
                            format!("玉 {} の面が見つからない（有効径の条件）", sl.block)
                        })?;
                    for sp in &u.spans {
                        for j in sp[0]..=sp[1] {
                            let semi = sc.surfaces[j].semi;
                            if semi.is_nan()
                                || semi <= 0.0
                                || sc.surfaces[j].typ == lensbench_core::surface::SurfType::CoordBrk
                            {
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

    /// 組 i を評価した結果の JSON（組 i だけから決まる）。評価できなければ error に理由
    pub fn eval(&self, i: usize) -> Value {
        let lenses: Vec<Value> = self
            .combo(i)
            .iter()
            .enumerate()
            .map(|(k, &c)| {
                let pk = &self.picks[k][c];
                json!({
                    "block": self.spec.slots[k].block, "vendor": pk.lens.vendor, "part": pk.lens.part,
                    "reversed": pk.reversed, "efl": num(pk.lens.efl), "diameter": num(pk.lens.diameter),
                })
            })
            .collect();
        let mut out = json!({ "i": i, "lenses": lenses });
        match self.optimize(i) {
            Ok(v) => out
                .as_object_mut()
                .unwrap()
                .extend(v.as_object().unwrap().clone()),
            Err(e) => out["error"] = json!(e),
        }
        out
    }

    fn optimize(&self, i: usize) -> Result<Value, String> {
        let (s, pr0, n_image) = self.problem_n(i)?;
        let (start, pr, r, names) = self.best_start(pr0)?;
        self.report(&s, n_image, start, pr, r, names)
    }

    /// 粗いふるいの評価関数（リング数 `prescreen.rings` で最適化した値。評価できなければ None）
    fn coarse_merit(&self, i: usize, rings: usize) -> Option<f64> {
        let (_, pr0, _) = self.problem_rings(i, rings).ok()?;
        let (_, _, r, _) = self.best_start(pr0).ok()?;
        r.merit.is_finite().then_some(r.merit)
    }

    /// 二段目（今の評価）に回す組の番号（小さい順）。粗いふるいがなければ全組。threads は [`Self::run`] と同じ
    pub fn screened(&self, threads: Option<usize>) -> Vec<usize> {
        let idx: Vec<usize> = (0..self.count()).collect();
        let Some(pre) = &self.spec.prescreen else {
            return idx;
        };
        let m = par::with_threads_or_outer(threads, || {
            par::map(&idx, |&i| self.coarse_merit(i, pre.rings))
        });
        let mut ok: Vec<(f64, usize)> = m
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (v, i)))
            .collect();
        ok.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut keep: Vec<usize> = ok.into_iter().take(pre.keep).map(|x| x.1).collect();
        keep.sort_unstable();
        keep
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
            optim::DlsResult,
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
        let vname = |v: &String| {
            v.split_once('=')
                .map_or(v.as_str(), |x| x.0)
                .trim()
                .to_string()
        };
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
        let mut best: Option<(usize, Problem, optim::DlsResult)> = None;
        let mut last_err = None;
        for (k, st) in starts.iter().enumerate() {
            let mut pr = pr0.clone();
            // 共通の変数は全状態に、状態ごとの変数はその状態に入れる
            let mut set: Vec<Vec<(Param, f64)>> = vec![Vec::new(); pr.configs.len()];
            for (name, &v) in st.iter() {
                let qs: Vec<usize> = (0..names.len()).filter(|&q| &names[q].0 == name).collect();
                if qs.is_empty() {
                    return Err(format!("出発点の変数 {name} が vars・state_vars にない"));
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
                    *c = c.with_params(sv).map_err(|e| e.to_string())?;
                }
            }
            match optim::dls(&pr, &o) {
                Ok(r) if r.merit.is_finite() => {
                    let better = match &best {
                        None => true,
                        Some(b) => r.merit < b.2.merit,
                    };
                    if better {
                        best = Some((k, pr, r));
                    }
                }
                Ok(_) => last_err = Some("評価関数が有限でない".to_string()),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        let Some((start, pr, r)) = best else {
            return Err(last_err.unwrap_or_else(|| "出発点がない".into()));
        };
        Ok((start, pr, r, names))
    }

    /// 最適化の結果の JSON
    fn report(
        &self,
        s: &System,
        n_image: usize,
        start: usize,
        pr: Problem,
        r: optim::DlsResult,
        names: Vec<(String, Option<usize>)>,
    ) -> Result<Value, String> {
        let mut img = pr.clone();
        img.terms.truncate(n_image);
        let image_merit = img.merit(&r.x).map_err(|e| e.to_string())?;
        let vars: Vec<Value> = pr
            .vars
            .iter()
            .zip(&names)
            .enumerate()
            .map(|(q, (v, (name, k)))| {
                let mut o = json!({ "name": name, "param": s.param_name(v.param), "x0": r.x0[q], "x": r.x[q] });
                if let Some(k) = k {
                    o["state"] = json!(self.spec.states[*k]);
                }
                o
            })
            .collect();
        let sys = &r.systems[0];
        let mut out = json!({
            "merit0": num(r.merit0), "merit": num(r.merit), "image_merit": num(image_merit),
            "start": if self.spec.starts.is_empty() { Value::Null } else { json!(start) },
            "stop": stop_name(r.stop),
            "iterations": r.iterations, "vars": vars, "efl": num(sys.paraxial().efl),
            "total_track": num((1..sys.image()).map(|j| sys.surfaces[j].t).sum::<f64>()),
        });
        if !self.spec.states.is_empty() {
            // 状態ごとの像の評価関数と焦点距離
            let mut st = Vec::new();
            for (k, name) in self.spec.states.iter().enumerate() {
                let mut one = img.clone();
                one.terms.retain(|t| t.config == k);
                let m = one.merit(&r.x).map_err(|e| e.to_string())?;
                st.push(json!({ "state": name, "image_merit": num(m), "efl": num(r.systems[k].paraxial().efl) }));
            }
            out["states"] = json!(st);
        }
        Ok(out)
    }

    /// 一括実行の結果のファイルの頭: 版・設定・元のプロジェクトの hash・所ごとの候補。
    /// 候補（カタログ）・設定・プロジェクトが変われば違う頭になり、再開できない
    pub fn header(&self) -> String {
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
            "project_hash": crate::sysfile::hash_value(&self.base),
            "spec": serde_json::to_value(&self.spec).unwrap(),
            "picks": picks,
        })
        .to_string()
    }

    /// [`run`](Self::run) と同じ。out があれば一括実行（[`lensbench_core::runs::run`]）で結果を 1 組ずつ
    /// JSONL に書き出し、resume なら同じ頭のファイルの続きから回す（書いた組は飛ばす）。返り値はファイルの全部の組
    pub fn run_to(
        &self,
        out: Option<&std::path::Path>,
        resume: bool,
        threads: Option<usize>,
        progress: Option<&(dyn Fn(runs::Progress) + Sync)>,
    ) -> Result<Vec<Value>, String> {
        let Some(path) = out else {
            return Ok(self.run(threads));
        };
        let o = runs::RunOpts {
            threads,
            out: Some(path),
            header: self.header(),
            resume,
            progress,
            cancel: None,
        };
        let idx = self.screened(threads);
        runs::run(idx.len(), 0, &o, |j, _| Ok(self.eval(idx[j]).to_string()))
            .map_err(|e| e.to_string())?;
        let (_, rows) = runs::read_results(path).map_err(|e| e.to_string())?;
        let mut v = rows
            .into_iter()
            .map(|(_, _, r)| serde_json::from_str(&r).map_err(|e| e.to_string()))
            .collect::<Result<Vec<Value>, String>>()?;
        sort_results(&mut v);
        Ok(v)
    }

    /// すべての組を並列に評価し、評価関数の小さい順に並べる（評価できなかった組は後ろ、番号の順）。
    /// threads は [`par::with_threads_or_outer`] と同じ（None は外の指定を引き継ぐ）
    pub fn run(&self, threads: Option<usize>) -> Vec<Value> {
        let idx = self.screened(threads);
        let mut v = par::with_threads_or_outer(threads, || par::map(&idx, |&i| self.eval(i)));
        sort_results(&mut v);
        v
    }
}

/// 設定の jac（"fd"・"auto"・"forward"）
fn jac_mode(j: &str) -> Result<optim::JacMode, String> {
    optim::JacMode::parse(j).ok_or_else(|| format!("jac {j}（{}）", optim::JacMode::NAMES))
}

/// 評価関数の小さい順（評価できなかった組は後ろ）、同じなら番号の順
pub fn sort_results(v: &mut [Value]) {
    v.sort_by(|a, b| {
        let m = |x: &Value| x["merit"].as_f64().unwrap_or(f64::INFINITY);
        m(a).total_cmp(&m(b))
            .then(a["i"].as_u64().cmp(&b["i"].as_u64()))
    });
}

/// 変数の書き方: id のある空気は `id.t`、ほかは [`System::param`]
fn var_param(s: &System, gaps: &[Gap], spec: &str) -> Result<Param, String> {
    if let Some((id, k)) = spec.rsplit_once('.') {
        let hits: Vec<&Gap> = gaps.iter().filter(|g| g.0 == id.trim()).collect();
        if !hits.is_empty() && k.trim().eq_ignore_ascii_case("t") {
            let (surf, sign) = match hits.as_slice() {
                [h] => (h.1, h.2),
                _ => {
                    return Err(format!(
                        "空気 {id} は 2 回通る（往復の間隔は群の shift で動かす）"
                    ))
                }
            };
            if sign < 0.0 {
                return Err(format!("空気 {id} は鏡の後（群の shift で動かす）"));
            }
            if !s.surfaces[surf].t.is_finite() {
                return Err(format!("空気 {id} は物体からの距離（無限遠）"));
            }
            let p = Param::new(surf, Kind::Thick);
            s.check_param(p).map_err(|e| e.to_string())?;
            return Ok(p);
        }
    }
    s.param(spec).map_err(|e| e.to_string())
}

fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

fn stop_name(s: Stop) -> &'static str {
    s.as_str()
}
