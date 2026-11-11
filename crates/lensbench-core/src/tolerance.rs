//! モンテカルロ公差シミュレーション。
//!
//! 公差（[`Tol`]）は系のパラメータ（[`crate::param`] の [`Param`]: 厚さ・曲率半径・面の傾き偏心・屈折率のずれ・
//! 玉や群の偏心・傾き・移動など）のずれの範囲 [lo, hi] と分布。1 つの公差が複数のパラメータを同じ乱数で
//! 動かしてもよい（例: 玉の焦点距離 ±1% を、玉のすべての曲率半径を同じ比で変えて表す）。`relative` なら
//! ずれは今の値に対する比（r → r (1 + δ)）。
//!
//! 試行ごとに、すべての公差のずれを引いて系を作り、補償子（[`crate::optim::Var`]: ピント・群の位置など）を
//! 評価関数（[`Problem`] の項）で DLS により合わせ直して（[`McSpec::comp`]）、評価関数と項の値を記録する。
//! 評価量は最適化と同じ項（RMS 波面・RMS スポット・MTF・近軸量・利用者の項 など）で表す。
//! 項の値（[`Problem::term_values`]）は、目標のある項（MTF など）ではその量そのもの、スポット・波面では RMS。
//!
//! 分布（[`Dist`]）:
//! - `Uniform`: [lo, hi] の一様分布
//! - `Normal`: 中心 (lo + hi)/2、標準偏差 (hi − lo)/(2 cut) の正規分布を [lo, hi] で切ったもの（既定 cut = 2、
//!   範囲の端が ±2σ）
//! - `End`: lo か hi を半々（最悪側の組合せを見る）
//!
//! 乱数: 試行 i の乱数は種と i だけで決まる（SplitMix64）。並列に回しても、スレッド数によらず結果は同じになる。
//! 複数の構成（多重構成・ズームの各状態）は、同じずれをすべての構成に与える（同じ部品なので）。構成の間で
//! 面の番号が同じであること（玉・群のパラメータは名前で付けた番号なので、玉・群の並びが同じであること）。
//!
//! 感度（[`McSpec::sensitivity`]）: 公差ごとに、その公差だけを lo・hi にした系を（補償して）評価し、
//! 公称との差を出す（OpticStudio の感度解析の「最悪の項目」に当たる）。モンテカルロの結果からは、
//! 評価関数とずれ δ・|δ| の相関（[`McResult::correlation`]）も出す。
//!
//! 簡易設定（[`simple`]）: 玉ごとの公差の組（[`LensTol`]、既定の組 [`LensTol::preset`]）から公差の並びを作る。

use crate::error::Result;
use crate::invalid;
use crate::optim::{dls, DlsOpts, Problem};
use crate::param::{Kind, Param};
use crate::system::System;
use crate::units::UnitKind;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 分布
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dist {
    Uniform,
    /// 範囲の半分が cut σ の正規分布を範囲で切ったもの
    Normal {
        cut: f64,
    },
    /// lo か hi を半々
    End,
    /// 隣り合う 2 つの公差（x・y の組。[`pair_disc`]）を、中心 (lo+hi)/2・半径 (hi−lo)/2 の円（楕円）の中で引く
    /// 組の x。次の公差は同じ `normal` の [`Dist::DiscY`]。偏心・傾きの大きさが公差以下になる（x・y 別々の一様分布は角で √2 倍まで）。
    /// `normal` が None なら円の中で一様、Some(cut) なら半径が cut σ の等方の 2 次元正規分布を円で切ったもの
    DiscX {
        normal: Option<f64>,
    },
    /// [`Dist::DiscX`] の組の y
    DiscY {
        normal: Option<f64>,
    },
}

impl Dist {
    pub fn parse(s: &str) -> Result<Dist> {
        let l = s.trim().to_ascii_lowercase();
        Ok(match l.as_str() {
            "uniform" | "u" => Dist::Uniform,
            "normal" | "gauss" | "n" => Dist::Normal { cut: 2.0 },
            "end" | "ends" | "e" => Dist::End,
            "disc_x" => Dist::DiscX { normal: None },
            "disc_y" => Dist::DiscY { normal: None },
            _ => {
                // disc_x:normal・disc_x:normal:3（y も）
                for (pre, y) in [("disc_x:", false), ("disc_y:", true)] {
                    if let Some(r) = l.strip_prefix(pre) {
                        let normal = match Dist::parse(r) {
                            Ok(Dist::Normal { cut }) => Some(cut),
                            Ok(Dist::Uniform) => None,
                            _ => invalid!(
                                "分布 {s}（disc_x・disc_x:normal・disc_x:normal:σの数、y も同じ）"
                            ),
                        };
                        return Ok(if y {
                            Dist::DiscY { normal }
                        } else {
                            Dist::DiscX { normal }
                        });
                    }
                }
                // normal:3 のように σ の数を付けられる
                if let Some(c) = l
                    .strip_prefix("normal:")
                    .and_then(|v| v.parse::<f64>().ok())
                {
                    if c > 0.0 {
                        return Ok(Dist::Normal { cut: c });
                    }
                }
                invalid!("分布 {s}（uniform・normal・normal:σの数・end・disc_x と disc_y の組）")
            }
        })
    }
    pub fn name(&self) -> String {
        match self {
            Dist::Uniform => "uniform".into(),
            Dist::Normal { cut } if *cut == 2.0 => "normal".into(),
            Dist::Normal { cut } => format!("normal:{cut}"),
            Dist::End => "end".into(),
            Dist::DiscX { normal } | Dist::DiscY { normal } => {
                let xy = if matches!(self, Dist::DiscX { .. }) {
                    "x"
                } else {
                    "y"
                };
                match normal {
                    None => format!("disc_{xy}"),
                    Some(cut) => format!("disc_{xy}:{}", Dist::Normal { cut: *cut }.name()),
                }
            }
        }
    }
}

/// 公差 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct Tol {
    /// 表示の名前（"lens1 dx" など）
    pub label: String,
    /// 同じずれを与えるパラメータ（1 つ以上）
    pub params: Vec<Param>,
    pub lo: f64,
    pub hi: f64,
    /// ずれを今の値に対する比で与える
    pub relative: bool,
    pub dist: Dist,
    /// パラメータごとにずれに掛ける数（空はすべて 1）。鏡で折り返して往復する玉の厚さのように、往きと戻りで同じずれを
    /// 逆の符号で与えるときに -1
    pub scale: Vec<f64>,
}

impl Tol {
    /// ±pm の一様分布
    pub fn pm(label: impl Into<String>, param: Param, pm: f64) -> Tol {
        Tol {
            label: label.into(),
            params: vec![param],
            lo: -pm,
            hi: pm,
            relative: false,
            dist: Dist::Uniform,
            scale: Vec::new(),
        }
    }

    /// パラメータ k のずれに掛ける数
    pub fn scale_of(&self, k: usize) -> f64 {
        self.scale.get(k).copied().unwrap_or(1.0)
    }
}

/// モンテカルロの設定
#[derive(Debug, Clone)]
pub struct McSpec {
    /// 評価関数（構成と項）。変数（[`Problem::vars`]）は補償子
    pub problem: Problem,
    pub tols: Vec<Tol>,
    /// 試行の数（0 なら感度だけ）
    pub trials: usize,
    pub seed: u64,
    /// 補償子の DLS の設定（反復は少なめでよい）
    pub comp: DlsOpts,
    /// 公差ごとの感度も求める
    pub sensitivity: bool,
}

impl McSpec {
    pub fn new(problem: Problem, tols: Vec<Tol>) -> McSpec {
        McSpec {
            problem,
            tols,
            trials: 100,
            seed: 1,
            comp: DlsOpts {
                max_iter: 10,
                ..Default::default()
            },
            sensitivity: true,
        }
    }

    pub fn check(&self) -> Result<()> {
        self.problem.check()?;
        for (i, t) in self.tols.iter().enumerate() {
            if t.params.is_empty() {
                invalid!("公差 {i}（{}）: パラメータがない", t.label);
            }
            if !t.scale.is_empty() && t.scale.len() != t.params.len() {
                invalid!(
                    "公差 {i}（{}）: scale は {} 個（パラメータは {} 個）",
                    t.label,
                    t.scale.len(),
                    t.params.len()
                );
            }
            if !(t.lo.is_finite() && t.hi.is_finite() && t.lo <= t.hi) {
                invalid!("公差 {i}（{}）: 範囲 [{}, {}]", t.label, t.lo, t.hi);
            }
            let prev = i.checked_sub(1).map(|j| self.tols[j].dist);
            let next = self.tols.get(i + 1).map(|u| u.dist);
            let bad = match t.dist {
                Dist::DiscX { normal } => next != Some(Dist::DiscY { normal }),
                Dist::DiscY { normal } => prev != Some(Dist::DiscX { normal }),
                _ => false,
            };
            if bad {
                invalid!(
                    "公差 {i}（{}）: 円の中の分布は x（disc_x）のすぐ後に同じ分布の y（disc_y）を置く組",
                    t.label
                );
            }
            if let Dist::Normal { cut }
            | Dist::DiscX { normal: Some(cut) }
            | Dist::DiscY { normal: Some(cut) } = t.dist
            {
                if !(cut > 0.0) {
                    invalid!("公差 {i}（{}）: 正規分布の σ の数 {cut}", t.label);
                }
            }
            for p in &t.params {
                if p.kind.is_object() {
                    invalid!("公差 {i}（{}）: 物点の移動は系のデータでない", t.label);
                }
                for s in &self.problem.configs {
                    s.check_param(*p)?;
                    if s.pickup_of(*p).is_some() {
                        invalid!(
                            "公差 {i}（{}）: {} はピックアップで決まる（元の面に与える）",
                            t.label,
                            s.param_name(*p)
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

/// 評価の結果（公称・試行・感度で共通）
#[derive(Debug, Clone, PartialEq)]
pub struct Eval {
    /// 評価関数の値（補償の後）
    pub merit: f64,
    /// 項ごとの値（[`Problem::term_values`]）
    pub terms: Vec<f64>,
    /// 補償子の値（補償の後、変数の順）
    pub comp: Vec<f64>,
}

/// 試行 1 回
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    /// 公差ごとのずれ（relative なら比）
    pub deltas: Vec<f64>,
    /// 評価（追跡できなかった試行は None で、理由は error）
    pub eval: Option<Eval>,
    pub error: Option<String>,
}

/// 公差 1 つの感度
#[derive(Debug, Clone, PartialEq)]
pub struct Sens {
    pub tol: usize,
    /// lo・hi にしたときの評価（追跡できなければ None）
    pub lo: Option<Eval>,
    pub hi: Option<Eval>,
}

impl Sens {
    /// 公称からの評価関数の変化の大きい方（追跡できなければ ∞）
    pub fn worst(&self, nominal: f64) -> f64 {
        let d = |e: &Option<Eval>| {
            e.as_ref()
                .map_or(f64::INFINITY, |e| (e.merit - nominal).abs())
        };
        d(&self.lo).max(d(&self.hi))
    }
}

/// 統計
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub mean: f64,
    pub std: f64,
    pub min: f64,
    pub max: f64,
    /// (百分位, 値)
    pub percentiles: Vec<(f64, f64)>,
}

/// 値の統計（有限の値だけ）。百分位は線形補間
pub fn stats(values: &[f64], pcts: &[f64]) -> Stats {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n == 0 {
        return Stats {
            n,
            mean: f64::NAN,
            std: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
            percentiles: pcts.iter().map(|&p| (p, f64::NAN)).collect(),
        };
    }
    let mean = v.iter().sum::<f64>() / n as f64;
    let var = if n > 1 {
        v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64
    } else {
        0.0
    };
    let pct = |p: f64| {
        let q = (p / 100.0).clamp(0.0, 1.0) * (n - 1) as f64;
        let i = q.floor() as usize;
        let f = q - i as f64;
        if i + 1 < n {
            v[i] + f * (v[i + 1] - v[i])
        } else {
            v[n - 1]
        }
    };
    Stats {
        n,
        mean,
        std: var.sqrt(),
        min: v[0],
        max: v[n - 1],
        percentiles: pcts.iter().map(|&p| (p, pct(p))).collect(),
    }
}

/// モンテカルロの結果
#[derive(Debug, Clone)]
pub struct McResult {
    /// 公称の系の評価（補償子も合わせた後。試行の評価と同じ条件）
    pub nominal: Eval,
    pub trials: Vec<Trial>,
    /// 公差の順（感度を求めなければ空）
    pub sensitivity: Vec<Sens>,
    /// 止めた（progress が false を返した）なら true。trials は終わった分だけ（試行の番号の順）
    pub cancelled: bool,
}

impl McResult {
    /// 追跡できた試行の評価関数の値
    pub fn merits(&self) -> Vec<f64> {
        self.trials
            .iter()
            .filter_map(|t| t.eval.as_ref().map(|e| e.merit))
            .collect()
    }

    /// 項 k の値（追跡できた試行）
    pub fn term(&self, k: usize) -> Vec<f64> {
        self.trials
            .iter()
            .filter_map(|t| t.eval.as_ref().and_then(|e| e.terms.get(k).copied()))
            .collect()
    }

    /// 公差ごとの、評価関数とずれの相関係数（Pearson）: (ずれ δ との相関, |δ − 範囲の中心| との相関)。
    /// 1 次の効き（ピントのずれなど、符号で良くも悪くもなる）は前、2 次の効き（偏心など、どちらに動いても悪くなる）は
    /// 後に出る。試行が 3 回未満か、ずれが変わらない公差は NaN
    pub fn correlation(&self, tols: &[Tol]) -> Vec<(f64, f64)> {
        let ok: Vec<&Trial> = self.trials.iter().filter(|t| t.eval.is_some()).collect();
        let m: Vec<f64> = ok.iter().map(|t| t.eval.as_ref().unwrap().merit).collect();
        tols.iter()
            .enumerate()
            .map(|(i, t)| {
                let c = 0.5 * (t.lo + t.hi);
                let d: Vec<f64> = ok.iter().map(|x| x.deltas[i]).collect();
                let a: Vec<f64> = d.iter().map(|x| (x - c).abs()).collect();
                (pearson(&d, &m), pearson(&a, &m))
            })
            .collect()
    }
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len();
    if n < 3 {
        return f64::NAN;
    }
    let ma = a.iter().sum::<f64>() / n as f64;
    let mb = b.iter().sum::<f64>() / n as f64;
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (x, y) = (a[i] - ma, b[i] - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa > 0.0 && sbb > 0.0 {
        sab / (saa * sbb).sqrt()
    } else {
        f64::NAN
    }
}

// ------------------------------------------------------------------------------------------ 乱数

/// SplitMix64（試行ごとの乱数。外部の crate に頼らず、版によらず同じ列になる）
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// 種 seed の試行 i の乱数
    pub fn for_trial(seed: u64, i: u64) -> Rng {
        let mut r = Rng(seed ^ 0x6a09_e667_f3bc_c909);
        // 試行の番号を混ぜる（隣の番号の列が似ないよう 2 回進める）
        r.0 = r.0.wrapping_add(i.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        r.next_u64();
        r
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// [0, 1)
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    /// 標準正規（Box–Muller、1 回に 1 つ）
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform(); // (0, 1]
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    /// 単位円の中で一様な点（半径 √u、方位一様）
    pub fn disc(&mut self) -> (f64, f64) {
        let r = self.uniform().sqrt();
        let a = std::f64::consts::TAU * self.uniform();
        (r * a.cos(), r * a.sin())
    }
    /// 単位円の中の点を、normal が None なら一様に、Some(cut) なら半径 1 が cut σ の等方の 2 次元正規分布で
    /// （円の外は引き直す。cut が小さくても 1 回で入る確率は 1 − exp(−cut²/2)）
    pub fn disc_dist(&mut self, normal: Option<f64>) -> (f64, f64) {
        let Some(cut) = normal else {
            return self.disc();
        };
        for _ in 0..1000 {
            let (x, y) = (self.normal(), self.normal());
            if x * x + y * y <= cut * cut {
                return (x / cut, y / cut);
            }
        }
        (0.0, 0.0)
    }
    /// 分布 d で [lo, hi] のずれを引く
    pub fn draw(&mut self, d: Dist, lo: f64, hi: f64) -> f64 {
        if lo == hi {
            return lo;
        }
        match d {
            Dist::Uniform => lo + (hi - lo) * self.uniform(),
            Dist::End => {
                if self.next_u64() >> 63 == 0 {
                    lo
                } else {
                    hi
                }
            }
            // 組でなく 1 つだけ引くときは、円の中の一様分布の片方の成分（周辺分布）
            Dist::DiscX { normal } | Dist::DiscY { normal } => {
                let (x, y) = self.disc_dist(normal);
                let v = if matches!(d, Dist::DiscX { .. }) {
                    x
                } else {
                    y
                };
                0.5 * (lo + hi) + 0.5 * (hi - lo) * v
            }
            Dist::Normal { cut } => {
                let c = 0.5 * (lo + hi);
                let h = 0.5 * (hi - lo);
                // 切った正規分布（範囲の外は引き直す。cut が小さくても 1 回で入る確率は 2Φ(cut)−1）
                for _ in 0..1000 {
                    let z = self.normal();
                    if z.abs() <= cut {
                        return c + h * z / cut;
                    }
                }
                c
            }
        }
    }
}

// ------------------------------------------------------------------------------------------ 実行

/// ずれ deltas を与えた構成
pub fn perturbed(spec: &McSpec, deltas: &[f64]) -> Result<Vec<System>> {
    if deltas.len() != spec.tols.len() {
        invalid!("ずれ {} 個（公差は {} 個）", deltas.len(), spec.tols.len());
    }
    spec.problem
        .configs
        .iter()
        .map(|s| {
            // 同じパラメータに 2 つの公差があれば足し合わせる
            let mut ch: Vec<(Param, f64)> = Vec::new();
            for (t, &d) in spec.tols.iter().zip(deltas) {
                if d == 0.0 {
                    continue;
                }
                for (k, &p) in t.params.iter().enumerate() {
                    let v0 = s.get_param(p)?;
                    let dv = t.scale_of(k) * if t.relative { v0 * d } else { d };
                    match ch.iter_mut().find(|x| x.0 == p) {
                        Some(x) => x.1 += dv,
                        None => ch.push((p, v0 + dv)),
                    }
                }
            }
            if ch.is_empty() {
                Ok(s.clone())
            } else {
                s.with_params_fast(&ch)
            }
        })
        .collect()
}

/// ずれ deltas の系を補償して評価する
pub fn evaluate(spec: &McSpec, deltas: &[f64]) -> Result<Eval> {
    let configs = perturbed(spec, deltas)?;
    let mut p = spec.problem.clone();
    p.configs = configs;
    eval_problem(&p, &spec.comp, true)
}

fn eval_problem(p: &Problem, o: &DlsOpts, compensate: bool) -> Result<Eval> {
    let x = if compensate && !p.vars.is_empty() {
        dls(p, o)?.x
    } else {
        p.x0()?
    };
    Ok(Eval {
        merit: p.merit(&x)?,
        terms: p.term_values(&x)?,
        comp: x,
    })
}

/// 試行 i のずれ
pub fn draw_trial(spec: &McSpec, i: usize) -> Vec<f64> {
    let mut r = Rng::for_trial(spec.seed, i as u64);
    let mut out = Vec::with_capacity(spec.tols.len());
    let mut k = 0;
    while k < spec.tols.len() {
        let t = &spec.tols[k];
        match (t.dist, spec.tols.get(k + 1)) {
            (Dist::DiscX { normal }, Some(u)) if u.dist == Dist::DiscY { normal } => {
                let (x, y) = r.disc_dist(normal);
                out.push(0.5 * (t.lo + t.hi) + 0.5 * (t.hi - t.lo) * x);
                out.push(0.5 * (u.lo + u.hi) + 0.5 * (u.hi - u.lo) * y);
                k += 2;
            }
            _ => {
                out.push(r.draw(t.dist, t.lo, t.hi));
                k += 1;
            }
        }
    }
    out
}

/// 隣り合う x・y の公差の組（同じ面・玉・群の dx と dy、tx と ty。パラメータの数が同じもの）を、円の中の
/// 分布（[`Dist::DiscX`]・[`Dist::DiscY`]、normal が None で一様・Some(cut) で 2 次元正規）にする。偏心・傾き・くさびの
/// 大きさを公差以下にしたいときに。組にした数を返す
pub fn pair_disc(tols: &mut [Tol], normal: Option<f64>) -> usize {
    use crate::param::Kind as K;
    use crate::units::UnitKind as U;
    let partner = |a: K| -> Option<K> {
        Some(match a {
            K::Dx => K::Dy,
            K::Tx => K::Ty,
            K::Unit(U::Dx) => K::Unit(U::Dy),
            K::Unit(U::Tx) => K::Unit(U::Ty),
            _ => return None,
        })
    };
    let is_pair = |a: &Tol, b: &Tol| {
        !a.relative
            && !b.relative
            && a.params.len() == b.params.len()
            && a.scale == b.scale
            && a.params
                .iter()
                .zip(&b.params)
                .all(|(p, q)| p.surf == q.surf && partner(p.kind) == Some(q.kind))
    };
    let mut n = 0;
    let mut k = 0;
    while k + 1 < tols.len() {
        if is_pair(&tols[k], &tols[k + 1]) {
            tols[k].dist = Dist::DiscX { normal };
            tols[k + 1].dist = Dist::DiscY { normal };
            n += 1;
            k += 2;
        } else {
            k += 1;
        }
    }
    n
}

/// モンテカルロを回す
pub fn run(spec: &McSpec) -> Result<McResult> {
    run_with(spec, |_, _| true)
}

/// [`run`] で、試行・感度の評価が 1 つ終わるごとに progress(終わった数, 全体の数) を呼ぶ（false で止める）。
/// 試行は並列に回す（[`crate::par::with_threads`] に従う）。progress は別のスレッドから呼ばれる
pub fn run_with(
    spec: &McSpec,
    progress: impl Fn(usize, usize) -> bool + Sync + Send,
) -> Result<McResult> {
    spec.check()?;
    let nominal = eval_problem(&spec.problem, &spec.comp, true)?;
    let ns = if spec.sensitivity {
        2 * spec.tols.len()
    } else {
        0
    };
    let total = ns + spec.trials;
    let done = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let tick = || {
        let k = done.fetch_add(1, Ordering::Relaxed) + 1;
        if !progress(k, total) {
            stop.store(true, Ordering::Relaxed);
        }
    };
    let one = |deltas: Vec<f64>| -> (Vec<f64>, Option<Eval>, Option<String>) {
        if stop.load(Ordering::Relaxed) {
            return (deltas, None, Some("cancelled".into()));
        }
        let r = evaluate(spec, &deltas);
        tick();
        match r {
            Ok(e) if e.merit.is_finite() => (deltas, Some(e), None),
            Ok(_) => (deltas, None, Some("評価関数が有限でない".into())),
            Err(e) => (deltas, None, Some(e.to_string())),
        }
    };
    let seq = crate::par::sequential();
    // 感度: 公差ごとに lo と hi
    let sens_in: Vec<(usize, bool)> = (0..ns).map(|k| (k / 2, k % 2 == 1)).collect();
    let sens_one = |&(i, hi): &(usize, bool)| {
        let mut d = vec![0.0; spec.tols.len()];
        d[i] = if hi { spec.tols[i].hi } else { spec.tols[i].lo };
        one(d)
    };
    let sens_out: Vec<_> = if seq {
        sens_in.iter().map(sens_one).collect()
    } else {
        sens_in.par_iter().map(sens_one).collect()
    };
    let sensitivity: Vec<Sens> = (0..spec.tols.len())
        .filter(|_| spec.sensitivity)
        .map(|i| Sens {
            tol: i,
            lo: sens_out[2 * i].1.clone(),
            hi: sens_out[2 * i + 1].1.clone(),
        })
        .collect();
    let trial_one = |i: &usize| one(draw_trial(spec, *i));
    let ids: Vec<usize> = (0..spec.trials).collect();
    let out: Vec<_> = if seq {
        ids.iter().map(trial_one).collect()
    } else {
        ids.par_iter().map(trial_one).collect()
    };
    let cancelled = stop.load(Ordering::Relaxed);
    let trials = out
        .into_iter()
        .filter(|x| x.2.as_deref() != Some("cancelled"))
        .map(|(deltas, eval, error)| Trial {
            deltas,
            eval,
            error,
        })
        .collect();
    Ok(McResult {
        nominal,
        trials,
        sensitivity,
        cancelled,
    })
}

// ------------------------------------------------------------------------------------------ 簡易設定

/// 玉ごとの公差の組（各値は ±、0 なら作らない）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LensTol {
    /// 曲率半径の比（面ごとに独立、0.002 = ±0.2%）
    pub radius_rel: f64,
    /// 焦点距離の比（玉の曲率半径をすべて同じ比で変える。カタログの公差）
    pub focal_rel: f64,
    /// 中心厚 [mm]（玉の中のガラスの厚さごと）
    pub thickness: f64,
    /// 面の傾き（くさび、玉の後ろの面の x・y の傾き）[deg]
    pub wedge: f64,
    /// 屈折率のずれ（ガラスごと）
    pub dn: f64,
    /// 分散の相対変化（アッベ数の比に当たる、ガラスごと）
    pub disp: f64,
    /// 玉の偏心 x・y [mm]
    pub decenter: f64,
    /// 玉の傾き x・y [deg]
    pub tilt: f64,
    /// 玉の後の空気の間隔 [mm]（次に玉がある場合）
    pub space: f64,
}

impl LensTol {
    /// 既定の組: commercial（一般）・precision（精密）・high（高精度）。一般的な加工・組立の目安の値
    /// （中心厚 ±0.15/0.05/0.025 mm、曲率半径 ±0.2/0.1/0.05%、くさび・傾き 3′/1′/0.5′、
    /// 屈折率 ±0.001/0.0005/0.0002、アッベ数 ±1/0.8/0.5%、偏心 ±0.1/0.05/0.02 mm、間隔 ±0.1/0.05/0.02 mm）
    pub fn preset(name: &str) -> Result<LensTol> {
        let am = 1.0 / 60.0; // 1 分 [deg]
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "commercial" | "一般" => LensTol {
                radius_rel: 0.002,
                focal_rel: 0.0,
                thickness: 0.15,
                wedge: 3.0 * am,
                dn: 0.001,
                disp: 0.01,
                decenter: 0.1,
                tilt: 3.0 * am,
                space: 0.1,
            },
            "precision" | "精密" => LensTol {
                radius_rel: 0.001,
                focal_rel: 0.0,
                thickness: 0.05,
                wedge: 1.0 * am,
                dn: 0.0005,
                disp: 0.008,
                decenter: 0.05,
                tilt: 1.0 * am,
                space: 0.05,
            },
            "high" | "高精度" => LensTol {
                radius_rel: 0.0005,
                focal_rel: 0.0,
                thickness: 0.025,
                wedge: 0.5 * am,
                dn: 0.0002,
                disp: 0.005,
                decenter: 0.02,
                tilt: 0.5 * am,
                space: 0.02,
            },
            "none" => LensTol::default(),
            n => invalid!("公差の組 {n}（commercial・precision・high・none）"),
        })
    }
}

/// 群・ブロック（玉でない玉・群）の公差（±、0 なら作らない）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GroupTol {
    pub decenter: f64,
    pub tilt: f64,
    pub shift: f64,
}

/// 簡易設定で公差の並びを作る。
///
/// - 玉（[`System::lens_units`] で作った lens1 …）ごとに lens(名前) の組。名前は、同じ範囲のブロック・群の玉・群が
///   あればその名前（ブロックの id）、なければ lens1 …。None の玉は公差を作らない
/// - 玉でない玉・群（群・複数の玉のブロック）は group(名前) の組（None なら作らない）
///
/// 分布はすべて dist。ピックアップで決まるパラメータ・変えられないパラメータは除く
pub fn simple(
    s: &System,
    lens: &dyn Fn(&str) -> Option<LensTol>,
    group: &dyn Fn(&str) -> Option<GroupTol>,
    dist: Dist,
) -> Vec<Tol> {
    let ok = |p: Param| s.check_param(p).is_ok() && s.pickup_of(p).is_none();
    let mut out: Vec<Tol> = Vec::new();
    // params は (パラメータ, ずれに掛ける数)
    let mut push = |label: String, params: Vec<(Param, f64)>, pm: f64, relative: bool| {
        let params: Vec<(Param, f64)> = params.into_iter().filter(|&(p, _)| ok(p)).collect();
        if pm > 0.0 && !params.is_empty() {
            let scale: Vec<f64> = if params.iter().all(|x| x.1 == 1.0) {
                Vec::new()
            } else {
                params.iter().map(|x| x.1).collect()
            };
            out.push(Tol {
                label,
                params: params.into_iter().map(|x| x.0).collect(),
                lo: -pm,
                hi: pm,
                relative,
                dist,
                scale,
            });
        }
    };
    let one = |p: Param| vec![(p, 1.0)];
    let auto: Vec<usize> = (0..s.units.len()).filter(|&u| s.units[u].auto).collect();
    let starts: Vec<usize> = auto.iter().map(|&u| s.units[u].spans[0][0]).collect();
    let links = lens_links(s);
    for (k, &u) in auto.iter().enumerate() {
        let unit = &s.units[u];
        // 往復する玉の 2 つ目からの範囲は、最初の範囲の玉にまとめる
        if links.iter().any(|l| l.others.iter().any(|o| o.auto == u)) {
            continue;
        }
        let link = links.iter().find(|l| l.first == u);
        // 同じ範囲のブロックの名前（往復する玉は範囲を全部持つブロック）
        let (name, mu) = match link {
            Some(l) => (s.units[l.unit].name.clone(), l.unit),
            None => s
                .units
                .iter()
                .position(|v| !v.auto && v.spans == unit.spans)
                .map_or((unit.name.clone(), u), |i| (s.units[i].name.clone(), i)),
        };
        let Some(t) = lens(&name) else { continue };
        let [a, b] = unit.spans[0];
        let others: Vec<LinkSpan> = link.map(|l| l.others.clone()).unwrap_or_default();
        // 面 j（最初の範囲）と、往復のほかの範囲の同じ面
        let surf = |j: usize, kind: Kind| -> Vec<(Param, f64)> {
            let mut v = vec![(Param::new(j, kind), 1.0)];
            for o in &others {
                v.push((Param::new(o.surf(a, j), kind), 1.0));
            }
            v
        };
        // 面 j の後の媒質（厚さ・屈折率）と、ほかの範囲の同じ媒質。厚さは逆向きの範囲では符号が逆
        let medium = |j: usize, kind: Kind, thick: bool| -> Vec<(Param, f64)> {
            let mut v = vec![(Param::new(j, kind), 1.0)];
            for o in &others {
                let f = if thick && o.reversed { -1.0 } else { 1.0 };
                v.push((Param::new(o.medium(a, j), kind), f));
            }
            v
        };
        let curved: Vec<usize> = (a..=b)
            .filter(|&j| {
                let d = &s.surfaces[j];
                d.c != 0.0 && ok(Param::new(j, Kind::Radius))
            })
            .collect();
        for &j in &curved {
            push(
                format!("{name} R{}", j - a + 1),
                surf(j, Kind::Radius),
                t.radius_rel,
                true,
            );
        }
        push(
            format!("{name} 焦点距離"),
            curved.iter().flat_map(|&j| surf(j, Kind::Radius)).collect(),
            t.focal_rel,
            true,
        );
        let media: Vec<usize> = s
            .unit_media(u)
            .into_iter()
            .filter(|&j| (a..=b).contains(&j))
            .collect();
        for &j in &media {
            if j < b {
                push(
                    format!("{name} 厚さ{}", j - a + 1),
                    medium(j, Kind::Thick, true),
                    t.thickness,
                    false,
                );
            }
        }
        for &j in &media {
            push(
                format!("{name} 屈折率{}", j - a + 1),
                medium(j, Kind::Dn, false),
                t.dn,
                false,
            );
            push(
                format!("{name} 分散{}", j - a + 1),
                medium(j, Kind::Disp, false),
                t.disp,
                false,
            );
        }
        // くさびは後ろの面の傾き。往復の同じ面に同じ符号（往復の範囲を持つ玉の剛体の傾きと同じ向き）
        push(format!("{name} くさびx"), surf(b, Kind::Tx), t.wedge, false);
        push(format!("{name} くさびy"), surf(b, Kind::Ty), t.wedge, false);
        push(
            format!("{name} 偏心x"),
            one(Param::unit(mu, UnitKind::Dx)),
            t.decenter,
            false,
        );
        push(
            format!("{name} 偏心y"),
            one(Param::unit(mu, UnitKind::Dy)),
            t.decenter,
            false,
        );
        push(
            format!("{name} 傾きx"),
            one(Param::unit(mu, UnitKind::Tx)),
            t.tilt,
            false,
        );
        push(
            format!("{name} 傾きy"),
            one(Param::unit(mu, UnitKind::Ty)),
            t.tilt,
            false,
        );
        if k + 1 < starts.len() && b < starts[k + 1] {
            // 玉の後の間隔。逆向きの範囲では範囲の前の間隔（符号が逆）
            let mut v = vec![(Param::new(b, Kind::Thick), 1.0)];
            for o in &others {
                if let Some((j, f)) = o.space() {
                    v.push((Param::new(j, Kind::Thick), f));
                }
            }
            push(format!("{name} 後の間隔"), v, t.space, false);
        }
    }
    for u in 0..s.units.len() {
        let unit = &s.units[u];
        if unit.auto
            || s.units.iter().any(|v| v.auto && v.spans == unit.spans)
            || links.iter().any(|l| l.unit == u)
        {
            continue;
        }
        let Some(g) = group(&unit.name) else { continue };
        let n = &unit.name;
        push(
            format!("{n} 偏心x"),
            one(Param::unit(u, UnitKind::Dx)),
            g.decenter,
            false,
        );
        push(
            format!("{n} 偏心y"),
            one(Param::unit(u, UnitKind::Dy)),
            g.decenter,
            false,
        );
        push(
            format!("{n} 傾きx"),
            one(Param::unit(u, UnitKind::Tx)),
            g.tilt,
            false,
        );
        push(
            format!("{n} 傾きy"),
            one(Param::unit(u, UnitKind::Ty)),
            g.tilt,
            false,
        );
        push(
            format!("{n} 移動"),
            one(Param::unit(u, UnitKind::Shift)),
            g.shift,
            false,
        );
    }
    out
}

/// 往復する玉の 2 つ目からの範囲（自動の玉の番号と、最初の範囲に対する向き）
#[derive(Debug, Clone, Copy, PartialEq)]
struct LinkSpan {
    /// 自動の玉（lens1 …）の番号
    auto: usize,
    /// 範囲 [始め, 終わり]
    span: [usize; 2],
    /// 最初の範囲と逆向きにたどる（鏡で折り返した戻り）
    reversed: bool,
    /// 範囲の前の面（逆向きの範囲の前の間隔の面）
    before: Option<usize>,
}

impl LinkSpan {
    /// 最初の範囲が a から始まるときの、最初の範囲の面 j と同じ面
    fn surf(&self, a: usize, j: usize) -> usize {
        let k = j - a;
        if self.reversed {
            self.span[1] - k
        } else {
            self.span[0] + k
        }
    }
    /// 最初の範囲の面 j の後の媒質と同じ媒質の面（その面の後の媒質）
    fn medium(&self, a: usize, j: usize) -> usize {
        let k = j - a;
        if self.reversed {
            self.span[1] - k - 1
        } else {
            self.span[0] + k
        }
    }
    /// 最初の範囲の後の間隔と同じ間隔の面と、ずれに掛ける数
    fn space(&self) -> Option<(usize, f64)> {
        if self.reversed {
            self.before.map(|j| (j, -1.0))
        } else {
            Some((self.span[1], 1.0))
        }
    }
}

/// 鏡で折り返して同じ玉を往復する（範囲が 2 つ以上あり、どの範囲も自動の玉の範囲と同じ）ブロックの玉
#[derive(Debug, Clone)]
struct LensLink {
    /// 往復の範囲を持つ玉（ブロック）の番号
    unit: usize,
    /// 最初の範囲の自動の玉の番号
    first: usize,
    others: Vec<LinkSpan>,
}

fn lens_links(s: &System) -> Vec<LensLink> {
    let auto_of = |sp: &[usize; 2]| {
        s.units
            .iter()
            .position(|v| v.auto && v.spans.len() == 1 && v.spans[0] == *sp)
    };
    let mut out = Vec::new();
    for (i, u) in s.units.iter().enumerate() {
        if u.auto || u.spans.len() < 2 {
            continue;
        }
        let Some(autos) = u.spans.iter().map(auto_of).collect::<Option<Vec<usize>>>() else {
            continue;
        };
        let [a, b] = u.spans[0];
        let n = b - a;
        let dir = |sp: &[usize; 2]| s.surfaces[sp[0]].t.signum();
        let mut others = Vec::new();
        let mut fine = true;
        for (k, sp) in u.spans.iter().enumerate().skip(1) {
            if sp[1] - sp[0] != n {
                fine = false;
                break;
            }
            others.push(LinkSpan {
                auto: autos[k],
                span: *sp,
                reversed: dir(sp) != dir(&u.spans[0]),
                before: sp[0].checked_sub(1),
            });
        }
        if fine {
            out.push(LensLink {
                unit: i,
                first: autos[0],
                others,
            });
        }
    }
    out
}
