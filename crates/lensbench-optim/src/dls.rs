//! 減衰最小二乗の本体（項・変数・問題・ヤコビアン・反復）。説明はクレートの頭（lib.rs）
use crate::pupil::apodized_gauss;
use lensbench_analysis::analysis::{mtf_only, ref_opl, to_ref, zernike, MtfMethod, ZernKind};
use lensbench_analysis::fiber::{fiber_coupling, FiberOpts};
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::error::{Error, Result};
use lensbench_core::math::*;
use lensbench_core::par;
use lensbench_core::param::{Kind, Param};
use lensbench_core::pupil::{exit_pupil_near, RefSurface};
use lensbench_core::surface::SurfType;
use lensbench_core::system::{Cols, FieldSel, System};
use lensbench_core::{invalid, Waves};
use std::sync::Arc;

/// 最適化の変数
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub param: Param,
    /// 効く構成の番号（None ならすべて）
    pub configs: Option<Vec<usize>>,
    /// 範囲（既定は ±∞）
    pub lo: f64,
    pub hi: f64,
    /// 同じ値を当てるほかの (構成, パラメータ)。光束ごとの系で同じブロックの面が別の番号になるとき・往復で 2 面になるとき。
    /// つないだ列の微分は差分
    pub links: Vec<(usize, Param)>,
}

impl Var {
    pub fn new(param: Param) -> Var {
        Var {
            param,
            configs: None,
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
            links: Vec::new(),
        }
    }
    /// 構成 c のパラメータ p にも同じ値を当てる（[`Var::links`]）
    pub fn linked(mut self, c: usize, p: Param) -> Var {
        self.links.push((c, p));
        self
    }
    pub fn bounds(mut self, lo: f64, hi: f64) -> Var {
        self.lo = lo;
        self.hi = hi;
        self
    }
    pub fn in_configs(mut self, c: Vec<usize>) -> Var {
        self.configs = Some(c);
        self
    }
    fn applies(&self, c: usize) -> bool {
        self.configs.as_ref().is_none_or(|v| v.contains(&c))
    }
    /// 構成 c で値を当てるパラメータ（param と links）
    fn params_in(&self, c: usize) -> Vec<Param> {
        let mut v: Vec<Param> = Vec::new();
        if self.applies(c) {
            v.push(self.param);
        }
        v.extend(self.links.iter().filter(|l| l.0 == c).map(|l| l.1));
        v
    }
    /// 構成 c に効くか
    fn acts(&self, c: usize) -> bool {
        self.applies(c) || self.links.iter().any(|l| l.0 == c)
    }
    /// 構成 c で param 1 つだけに当てるか（解析的な微分を使える）
    fn plain_in(&self, c: usize) -> bool {
        self.applies(c) && !self.links.iter().any(|l| l.0 == c)
    }
}

#[path = "dls_terms.rs"]
mod terms;
pub use terms::*;

#[path = "dls_lm.rs"]
mod lm;
pub use lm::*;

#[path = "dls_jac.rs"]
mod jac;
use jac::{pupil_d, typical, JacCache, PupilCache};

#[path = "dls_spot.rs"]
mod spot;
use spot::{centroid_term, spot_term};

#[path = "dls_ray.rs"]
mod ray;

#[path = "dls_image.rs"]
mod image;

#[path = "dls_card.rs"]
mod card;
pub use card::CardQ;

#[path = "dls_seidel.rs"]
mod seidel;
pub use seidel::SeidelQ;

#[path = "dls_surf.rs"]
mod surf;
#[path = "dls_vol.rs"]
mod vol;
use surf::reads_semi;
pub use surf::SurfQ;

#[path = "dls_cross.rs"]
mod cross;
#[path = "dls_fd.rs"]
mod fd;
#[path = "dls_trace.rs"]
mod trace;
use trace::{launch_of, trace_ok, trace_pupil_cols, trace_pupil_rec};

/// 種類ごとの変数の目安の大きさ（大域の探索の振り幅・同じ解の判定）
pub(crate) fn typical_of(k: Kind) -> f64 {
    typical(k)
}

/// 差分の +h・−h の系（前進差分では −h はなし。境界の外に出る・作れない側は None）
type FdSystems = (Option<System>, Option<System>);

/// ヤコビアンの求め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JacMode {
    /// 解析的に求められる列は解析的に、ほかは中心差分。ただし項ごとに、解析的に求められる列が
    /// [`Problem::analytic_min`] 個より少なければ中心差分（解析的な微分は列の数によらない手間が大きく、
    /// 列が少ないと差分の方が速い）
    #[default]
    Auto,
    /// すべて中心差分
    FiniteDiff,
    /// すべて前進差分（中心差分の半分の追跡で済むが、誤差は刻みの 1 次）。射出瞳の揺れのため刻みを大きく取る
    /// 波面の項（回転対称でない系など）は中心差分のまま
    Forward,
}

impl JacMode {
    /// 名前（"auto"・"fd"・"forward"、大文字小文字は問わない）から
    pub fn parse(s: &str) -> Option<JacMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(JacMode::Auto),
            "fd" => Some(JacMode::FiniteDiff),
            "forward" => Some(JacMode::Forward),
            _ => None,
        }
    }

    /// 名前
    pub fn name(self) -> &'static str {
        match self {
            JacMode::Auto => "auto",
            JacMode::FiniteDiff => "fd",
            JacMode::Forward => "forward",
        }
    }

    /// 選べる名前（エラーの表示用）
    pub const NAMES: &'static str = "auto・fd・forward";
}

/// 既定の評価関数の種類（[`Problem::add_default`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultMerit {
    Spot,
    Wavefront,
}

/// 最適化の問題
#[derive(Debug, Clone)]
pub struct Problem {
    /// 構成（多重構成・ズームの各状態）。1 つなら単一の系
    pub configs: Vec<System>,
    pub vars: Vec<Var>,
    pub terms: Vec<Term>,
    pub jac: JacMode,
    /// 差分の刻みの相対値（刻み = rel × max(|値|, 種類ごとの目安)）
    pub fd_rel: f64,
    /// 回転対称でない系の波面の項の差分の刻みの相対値。射出瞳を実光線の外挿で求める（OpticStudio と同じ）ので
    /// 評価関数に 1e-8 程度の揺れがあり、小さい刻みでは差分が揺れに負ける
    pub fd_rel_noisy: f64,
    /// [`JacMode::Auto`] で解析的に求める列の最少の数（項ごと。これより少なければ中心差分）。
    /// 既定 16 は、11 面の系のスポットで解析的と中心差分の時間が並ぶ列の数。0 なら常に解析的
    pub analytic_min: usize,
    /// 瞳の光線を並べる項（スポット・波面・重心）で、追跡が止まった光線（全反射・面に届かない）を、けられた光線と
    /// 同じく除く（既定 false: 項が評価できない誤り）。除いた数は [`Problem::ray_counts`]
    pub stopped_as_vignetted: bool,
}

/// Zernike の項を当てはめる項の数の下限（[`TermKind::Zernike`]）
const ZERN_MIN_TERMS: usize = 37;

/// 項の残差（重みを掛ける前）と、解析的に求めた列のヤコビアン（行優先 [行][params]）
type TermOut = (Vec<f64>, Option<Vec<f64>>);

impl Problem {
    pub fn new(configs: Vec<System>) -> Problem {
        Problem {
            configs,
            vars: Vec::new(),
            terms: Vec::new(),
            jac: JacMode::Auto,
            fd_rel: 1e-6,
            fd_rel_noisy: 1e-3,
            analytic_min: 16,
            stopped_as_vignetted: false,
        }
    }

    /// 構成だけ configs に替えた問題（今の構成は写さない）
    pub fn with_configs(&self, configs: Vec<System>) -> Problem {
        Problem {
            configs,
            vars: self.vars.clone(),
            terms: self.terms.clone(),
            jac: self.jac,
            fd_rel: self.fd_rel,
            fd_rel_noisy: self.fd_rel_noisy,
            analytic_min: self.analytic_min,
            stopped_as_vignetted: self.stopped_as_vignetted,
        }
    }

    /// 単一の系
    pub fn single(s: System) -> Problem {
        Problem::new(vec![s])
    }

    /// 文字列で変数を足す（[`System::param`] の書き方。最初の構成で解釈する）
    pub fn add_var(&mut self, spec: &str) -> Result<&mut Var> {
        let p = self.configs.first().map(|s| s.param(spec));
        let Some(p) = p else {
            invalid!("no configurations");
        };
        self.vars.push(Var::new(p?));
        Ok(self.vars.last_mut().expect("just pushed"))
    }

    /// 既定の評価関数: 構成ごと・定義済みの視野ごとに、重みのある全波長の RMS の項を足す（重みは視野の重み、
    /// 重み 0 の視野は除く）
    pub fn add_default(&mut self, kind: DefaultMerit, rings: usize, reference: RmsRef) {
        for (c, s) in self.configs.iter().enumerate() {
            for (i, f) in s.fields.iter().enumerate() {
                if !(f.w > 0.0) {
                    continue;
                }
                let field = FieldSel::Index(i);
                let k = match kind {
                    DefaultMerit::Spot => TermKind::Spot {
                        field,
                        wave: None,
                        rings,
                        reference,
                        quad: Quad::Lensbench,
                        axis: SpotAxis::Both,
                    },
                    DefaultMerit::Wavefront => TermKind::Wavefront {
                        field,
                        wave: None,
                        rings,
                        reference,
                        quad: Quad::Lensbench,
                    },
                };
                self.terms.push(Term {
                    side: Default::default(),
                    config: c,
                    weight: f.w,
                    kind: k,
                });
            }
        }
    }

    /// 問題の確かめ（構成・変数・項の番号と値）
    pub fn check(&self) -> Result<()> {
        if self.configs.is_empty() {
            invalid!("no configurations");
        }
        if self.terms.is_empty() {
            invalid!("no merit function terms");
        }
        // 項が使う構成が追跡できるか（複屈折材の板の Mode 1・3）。どの構成かを文に入れる
        for (c, s) in self.configs.iter().enumerate() {
            if self.terms.iter().any(|t| t.config == c) {
                s.check_crystals()
                    .map_err(|e| e.at(format!("configuration {}", c + 1)))?;
            }
        }
        for (q, v) in self.vars.iter().enumerate() {
            if v.param.kind.is_object() {
                invalid!("variable {q}: moving the object point is not system data");
            }
            if v.param.kind.is_system_value() || v.param.kind.is_table_only() {
                invalid!("variable {q}: {} is not a variable", v.param.name());
            }
            if !(v.lo <= v.hi) {
                invalid!("variable {q}: range [{}, {}]", v.lo, v.hi);
            }
            let mut any = false;
            for (c, s) in self.configs.iter().enumerate() {
                for p in v.params_in(c) {
                    if p.kind.is_object() || p.kind.is_system_value() || p.kind.is_table_only() {
                        invalid!("variable {q}: {} is not a variable", p.name());
                    }
                    s.check_param(p)?;
                    any = true;
                }
            }
            if v.links.iter().any(|l| l.0 >= self.configs.len()) {
                invalid!("variable {q}: a link to a configuration out of range");
            }
            if !any {
                invalid!("variable {q}: no configuration it acts on");
            }
            if self.vars[..q]
                .iter()
                .any(|u| u.param == v.param && overlap(u, v))
            {
                invalid!("variable {q}: the same parameter twice in overlapping configurations");
            }
        }
        for (i, t) in self.terms.iter().enumerate() {
            let Some(s) = self.configs.get(t.config) else {
                invalid!("term {i}: config {} (of {})", t.config, self.configs.len());
            };
            if !(t.weight >= 0.0) {
                invalid!("term {i}: weight {}", t.weight);
            }
            check_kind(i, &t.kind, t.side, s)?;
            if let TermKind::Diff { a, b, .. } = &t.kind {
                let n = self.configs.len();
                if *a >= n || *b >= n {
                    invalid!("term {i}: configurations {} and {} (of {n})", a + 1, b + 1);
                }
            }
        }
        Ok(())
    }

    /// 変数の初期値（効く最初の構成の値）
    pub fn x0(&self) -> Result<Vec<f64>> {
        self.vars
            .iter()
            .map(|v| match (0..self.configs.len()).find(|&c| v.applies(c)) {
                Some(c) => self.configs[c].get_param(v.param),
                None => match v.links.first() {
                    Some(&(c, p)) => self.configs[c].get_param(p),
                    None => self.configs[0].get_param(v.param),
                },
            })
            .collect()
    }

    /// 変数の値 x での各構成の系
    pub fn systems(&self, x: &[f64]) -> Result<Vec<System>> {
        (0..self.configs.len()).map(|c| self.system(c, x)).collect()
    }

    /// 結果として返す系（自動の半直径を求め直す）
    fn final_systems(&self, x: &[f64]) -> Result<Vec<System>> {
        let mut v = self.systems(x)?;
        v.iter_mut().for_each(System::update_semi);
        Ok(v)
    }

    fn system(&self, c: usize, x: &[f64]) -> Result<System> {
        if x.len() != self.vars.len() {
            invalid!("{} values for {} variables", x.len(), self.vars.len());
        }
        let ch: Vec<(Param, f64)> = self
            .vars
            .iter()
            .zip(x)
            .flat_map(|(v, &xv)| v.params_in(c).into_iter().map(move |p| (p, xv)))
            // 今の値と同じものは変えない（傾き偏心のない面に偏心 0 を与えると、回転対称でない系の扱いになり
            // 射出瞳の求め方が変わるため。初期値の系を元の系と同じにする）
            .filter(|&(p, xv)| self.configs[c].get_param(p).ok() != Some(xv))
            .collect();
        if ch.is_empty() {
            return Ok(self.configs[c].clone());
        }
        let mut s = self.configs[c].with_params_fast(&ch)?;
        // 半直径を読む項があれば、自動の半直径を評価のたびに求め直す（OpticStudio と同じ）
        if self.terms.iter().any(|t| reads_semi(&t.kind)) {
            s.update_semi();
        }
        Ok(s)
    }

    /// 項ごとの残差の行数（系の形と項で決まり、変数の値によらない）
    pub fn term_rows(&self) -> Vec<usize> {
        self.terms
            .iter()
            .map(|t| term_rows(&self.configs[t.config], &t.kind))
            .collect()
    }

    /// 残差の数
    pub fn n_residuals(&self) -> usize {
        self.term_rows().iter().sum()
    }

    fn weight_sum(&self) -> f64 {
        self.terms.iter().map(|t| t.weight).sum()
    }

    /// 項 t の残差（重みを掛ける前）。構成をまたぐ項（[`TermKind::Spread`]）はすべての構成の系 sys で
    fn eval_plain(&self, sys: &[System], t: &Term) -> Result<Vec<f64>> {
        if let Some(r) = cross::value(sys, &t.kind, self.stopped_as_vignetted) {
            return r;
        }
        let keep = self.stopped_as_vignetted;
        Ok(eval_term(&sys[t.config], &t.kind, None, None, keep)?.0)
    }

    /// 残差（重みの平方根を掛けたもの、項の順に並べる）
    pub fn residuals(&self, x: &[f64]) -> Result<Vec<f64>> {
        let sys = self.systems(x)?;
        let parts = map_maybe_par(&self.terms, |t| {
            let mut r = self.eval_plain(&sys, t)?;
            t.clamp(&mut r, None);
            let a = t.weight.sqrt();
            r.iter_mut().for_each(|v| *v *= a);
            Ok(r)
        })?;
        Ok(parts.concat())
    }

    /// 評価関数の値 √(Σ r² / Σ 重み)
    pub fn merit(&self, x: &[f64]) -> Result<f64> {
        let r = self.residuals(x)?;
        Ok(self.merit_of(&r))
    }

    /// 残差から評価関数の値
    pub fn merit_of(&self, r: &[f64]) -> f64 {
        let ws = self.weight_sum();
        let ss: f64 = r.iter().map(|v| v * v).sum();
        if ws > 0.0 {
            (ss / ws).sqrt()
        } else {
            ss.sqrt()
        }
    }

    /// 項ごとの値: 目標のある項は量そのもの（[`TermValue::Quantity`]）、ない項は残差の √Σr²（[`TermValue::Rms`]。
    /// スポット・波面の項では RMS そのもの）
    pub fn term_values(&self, x: &[f64]) -> Result<Vec<TermValue>> {
        let sys = self.systems(x)?;
        map_maybe_par(&self.terms, |t| {
            let r = self.eval_plain(&sys, t)?;
            Ok(term_value(&t.kind, &r))
        })
    }

    /// [`Self::merit`] と [`Self::term_values`]（系を 1 回作り、項を 1 回ずつ評価する。値はそれぞれと同じ）
    pub fn merit_and_terms(&self, x: &[f64]) -> Result<(f64, Vec<TermValue>)> {
        let sys = self.systems(x)?;
        let parts = map_maybe_par(&self.terms, |t| {
            let r0 = self.eval_plain(&sys, t)?;
            let v = term_value(&t.kind, &r0);
            let mut r = r0;
            t.clamp(&mut r, None);
            let a = t.weight.sqrt();
            r.iter_mut().for_each(|v| *v *= a);
            Ok((r, v))
        })?;
        let r: Vec<f64> = parts.iter().flat_map(|p| p.0.iter().copied()).collect();
        Ok((self.merit_of(&r), parts.into_iter().map(|p| p.1).collect()))
    }

    /// [`Self::term_values`] と、項ごとの評価関数への寄与 [%]（重みを掛けた残差の 2 乗の和の、全体に対する割合。
    /// OpticStudio の評価関数エディタの % Contrib と同じ。全体が 0 なら 0）
    pub fn term_values_contrib(&self, x: &[f64]) -> Result<(Vec<TermValue>, Vec<f64>)> {
        let sys = self.systems(x)?;
        let parts = map_maybe_par(&self.terms, |t| {
            let r0 = self.eval_plain(&sys, t)?;
            let v = term_value(&t.kind, &r0);
            let mut r = r0;
            t.clamp(&mut r, None);
            let ss: f64 = r.iter().map(|v| v * v).sum::<f64>() * t.weight;
            Ok((v, ss))
        })?;
        let total: f64 = parts.iter().map(|p| p.1).sum();
        let pct = |ss: f64| {
            if total > 0.0 && total.is_finite() {
                100.0 * ss / total
            } else {
                0.0
            }
        };
        Ok(parts.into_iter().map(|(v, ss)| (v, pct(ss))).unzip())
    }

    /// 残差とヤコビアン（行優先 [残差][変数]）
    pub fn jacobian(&self, x: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
        self.check()?;
        self.jacobian_checked(x)
    }

    /// [`Self::jacobian`] で、[`Self::check`] を済ませた問題（[`dls`] は入口で 1 回だけ確かめる）
    pub(crate) fn jacobian_checked(&self, x: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
        let nv = self.vars.len();
        if x.len() != nv {
            invalid!("{} values for {nv} variables", x.len());
        }
        let sys = self.systems(x)?;
        let rows = self.term_rows();
        let steps: Vec<f64> = self
            .vars
            .iter()
            .zip(x)
            .map(|(v, &xv)| self.fd_rel * xv.abs().max(typical(v.param.kind)))
            .collect();
        let cache = JacCache::new(self.configs.len(), nv);
        // 項ごと: 残差・解析的な列・差分が要る変数
        struct Part {
            r: Vec<f64>,
            j: Vec<f64>,
            fd: Vec<usize>,
        }
        let parts: Vec<Part> = map_maybe_par(&self.terms, |t| {
            let s = &sys[t.config];
            let act: Vec<usize> = (0..nv).filter(|&q| self.vars[q].acts(t.config)).collect();
            if let Some(r) = cross::value(&sys, &t.kind, self.stopped_as_vignetted) {
                let r = r?;
                let fd = (0..nv).filter(|&q| cross::moves(self, q)).collect();
                return Ok(Part {
                    j: vec![0.0; r.len() * nv],
                    r,
                    fd,
                });
            }
            if let TermKind::User(u) = &t.kind {
                let r = u.eval_checked(&[s])?.remove(0);
                let m = r.len();
                let mut j = vec![0.0; m * nv];
                let ps: Vec<Param> = act.iter().map(|&q| self.vars[q].param).collect();
                let plain = act.iter().all(|&q| self.vars[q].plain_in(t.config));
                let g = if self.jac == JacMode::Auto && !ps.is_empty() && plain {
                    u.0.grad(s, &ps)?
                } else {
                    None
                };
                let fd = match g {
                    Some(g) => {
                        if g.len() != m * ps.len() {
                            invalid!(
                                "user term {}: {} derivatives (expected {m}×{})",
                                u.0.name(),
                                g.len(),
                                ps.len()
                            );
                        }
                        for (k, &q) in act.iter().enumerate() {
                            for i in 0..m {
                                j[i * nv + q] = g[i * ps.len() + k];
                            }
                        }
                        Vec::new()
                    }
                    None => act,
                };
                return Ok(Part { r, j, fd });
            }
            let mut analytic: Vec<usize> = if self.jac == JacMode::Auto
                && act.len() >= self.analytic_min
                && supports_analytic(s, &t.kind)
            {
                act.iter()
                    .copied()
                    .filter(|&q| self.vars[q].plain_in(t.config))
                    .filter(|&q| {
                        !cache.launch_moves(
                            t.config,
                            s,
                            &t.kind,
                            q,
                            self.vars[q].param,
                            x[q],
                            steps[q],
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if analytic.len() < self.analytic_min {
                analytic.clear();
            }
            let ps: Vec<Param> = analytic.iter().map(|&q| self.vars[q].param).collect();
            let (r, ja, ok) = if ps.is_empty() {
                (
                    eval_term(s, &t.kind, None, None, self.stopped_as_vignetted)?.0,
                    None,
                    false,
                )
            } else {
                match eval_term(
                    s,
                    &t.kind,
                    Some(&ps),
                    Some(&cache.pupil[t.config]),
                    self.stopped_as_vignetted,
                ) {
                    Ok((r, j)) => (r, j, true),
                    // 解析的な微分に対応しない面など: すべて差分に
                    Err(_) => (
                        eval_term(s, &t.kind, None, None, self.stopped_as_vignetted)?.0,
                        None,
                        false,
                    ),
                }
            };
            let m = r.len();
            let mut j = vec![0.0; m * nv];
            let mut fd: Vec<usize> = act.clone();
            if ok {
                let ja = ja.expect("ok means the analytic Jacobian was computed");
                for (k, &q) in analytic.iter().enumerate() {
                    for i in 0..m {
                        j[i * nv + q] = ja[i * ps.len() + k];
                    }
                }
                fd.retain(|q| !analytic.contains(q));
            }
            Ok(Part { r, j, fd })
        })?;
        // 差分の列: 変数ごとに ±h の系を作り、その変数を差分で求める項をまとめて評価
        let fd_vars: Vec<usize> = (0..nv)
            .filter(|q| parts.iter().any(|p| p.fd.contains(q)))
            .collect();
        let fds: Vec<&[usize]> = parts.iter().map(|p| p.fd.as_slice()).collect();
        let r0s: Vec<&[f64]> = parts.iter().map(|p| p.r.as_slice()).collect();
        let cols = fd::columns(self, x, &sys, &steps, &fds, &r0s, &fd_vars)?;
        let forward = self.jac == JacMode::Forward;
        let mut parts = parts;
        for (k, &q) in fd_vars.iter().enumerate() {
            for (ti, d) in &cols[k] {
                for (i, v) in d.iter().enumerate() {
                    parts[*ti].j[i * nv + q] = *v;
                }
            }
        }
        // 構成をまたぐ項の差分: 変数ごとにすべての構成の ±h の系で
        for (ti, t) in self.terms.iter().enumerate() {
            if matches!(t.kind, TermKind::Spread { .. } | TermKind::Diff { .. }) {
                for (q, d) in cross::columns(self, x, &steps, t, &parts[ti].fd)? {
                    d.iter()
                        .enumerate()
                        .for_each(|(i, v)| parts[ti].j[i * nv + q] = *v);
                }
            }
        }
        // 利用者の項の差分: ずらした系をすべて作り、項ごとに 1 回でまとめて評価する
        for (ti, t) in self.terms.iter().enumerate() {
            let TermKind::User(u) = &t.kind else {
                continue;
            };
            let fd = parts[ti].fd.clone();
            if fd.is_empty() {
                continue;
            }
            if forward {
                let ps: Vec<System> = map_maybe_par(&fd, |&q| {
                    let mut xp = x.to_vec();
                    xp[q] += steps[q];
                    self.system(t.config, &xp)
                })?;
                let refs: Vec<&System> = ps.iter().collect();
                let v = u.eval_checked(&refs)?;
                for (k, &q) in fd.iter().enumerate() {
                    for i in 0..v[k].len() {
                        parts[ti].j[i * nv + q] = (v[k][i] - parts[ti].r[i]) / steps[q];
                    }
                }
                continue;
            }
            let pm: Vec<(System, System)> = map_maybe_par(&fd, |&q| {
                let mut xp = x.to_vec();
                let mut xm = x.to_vec();
                xp[q] += steps[q];
                xm[q] -= steps[q];
                Ok((self.system(t.config, &xp)?, self.system(t.config, &xm)?))
            })?;
            let refs: Vec<&System> = pm.iter().flat_map(|(a, b)| [a, b]).collect();
            let v = u.eval_checked(&refs)?;
            for (k, &q) in fd.iter().enumerate() {
                let (rp, rm) = (&v[2 * k], &v[2 * k + 1]);
                for i in 0..rp.len() {
                    parts[ti].j[i * nv + q] = (rp[i] - rm[i]) / (2.0 * steps[q]);
                }
            }
        }
        let mut r = Vec::with_capacity(rows.iter().sum());
        let mut j = Vec::with_capacity(r.capacity() * nv);
        for (mut p, t) in parts.into_iter().zip(&self.terms) {
            t.clamp(&mut p.r, Some(&mut p.j));
            let a = t.weight.sqrt();
            r.extend(p.r.iter().map(|v| v * a));
            j.extend(p.j.iter().map(|v| v * a));
        }
        Ok((r, j))
    }
}

fn overlap(a: &Var, b: &Var) -> bool {
    match (&a.configs, &b.configs) {
        (Some(x), Some(y)) => x.iter().any(|c| y.contains(c)),
        _ => true,
    }
}

/// 差分が射出瞳の揺れに負ける組（回転対称でない系か、対称を崩す変数での波面の項）
fn noisy_fd(s: &System, k: &TermKind, v: Kind) -> bool {
    if let TermKind::Calc { args, .. } = k {
        return args
            .iter()
            .any(|a| matches!(a, CalcArg::Term(k) if noisy_fd(s, k, v)));
    }
    matches!(
        k,
        TermKind::Wavefront { .. } | TermKind::Opd { .. } | TermKind::Zernike { .. }
    ) && (!s.symmetric()
        || matches!(v, Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz)
        || v.is_unit_motion())
}

/// v のそれぞれに f をかける（[`par::try_map`]。誤りが出たら残りは始めない）
fn map_maybe_par<T: Sync, R: Send>(
    v: &[T],
    f: impl Fn(&T) -> Result<R> + Sync + Send,
) -> Result<Vec<R>> {
    par::try_map(v, f)
}

/// 項で使う波長（単色はその波長、多色は重みのある波長）
fn waves(s: &System, w: Option<usize>) -> Vec<usize> {
    match w {
        Some(w) => vec![w],
        None => (0..s.wavelengths.len())
            .filter(|&i| s.weights[i] > 0.0)
            .collect(),
    }
}

/// 像面が平面（波面の解析的な微分の前提）
fn flat_image(s: &System) -> bool {
    let d = &s.surfaces[s.image()];
    matches!(d.typ(), SurfType::Standard | SurfType::EvenAsph)
        && d.c == 0.0
        && d.asph().iter().all(|&a| a == 0.0)
}

/// 項の種類ごとの確かめ（i は項の番号、計算の項の引数は同じ番号で）
fn check_kind(i: usize, k: &TermKind, side: Side, s: &System) -> Result<()> {
    let (field, wave) = match k {
        TermKind::Spot {
            field, wave, rings, ..
        }
        | TermKind::Wavefront {
            field, wave, rings, ..
        }
        | TermKind::Centroid {
            field, wave, rings, ..
        } => {
            if *rings == 0 {
                invalid!("term {i}: the number of rings must be 1 or more");
            }
            (*field, *wave)
        }
        TermKind::Mtf {
            field, wave, freq, ..
        } => {
            if !(*freq >= 0.0) {
                invalid!("term {i}: frequency {freq}");
            }
            (*field, *wave)
        }
        TermKind::Paraxial { q, wave, target } => {
            if !target.is_finite() {
                invalid!("term {i}: target {target} of {}", q.name());
            }
            if let Some(w) = wave {
                if !q.has_wave() {
                    invalid!(
                        "term {i}: wave goes with efl, pimh, amag, linv, petc, petz and wlen, not {}",
                        q.name()
                    );
                }
                if *w >= s.wavelengths.len() {
                    invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
                }
            }
            let mirror = match q {
                ParaxQ::Linv => mirror_in(s, 1, s.stop()),
                ParaxQ::Petc | ParaxQ::Petz => mirror_in(s, 1, s.image() - 1),
                _ => false,
            };
            if mirror {
                invalid!(
                    "term {i}: {} of systems with mirrors is not supported yet",
                    q.name()
                );
            }
            if !q.value_at(s, *wave).is_finite() {
                invalid!("term {i}: {} of the system is not finite", q.name());
            }
            return Ok(());
        }
        TermKind::Ray {
            field, wave, surf, ..
        } => {
            if let Some(j) = surf {
                if *j == 0 || *j > s.image() {
                    invalid!("term {i}: surface {j} (1 to {})", s.image());
                }
            }
            (*field, Some(wave.unwrap_or(s.primary)))
        }
        TermKind::ParaxRay {
            field, wave, surf, ..
        } => {
            if *surf == 0 || *surf > s.image() {
                invalid!("term {i}: surface {surf} (1 to {})", s.image());
            }
            (*field, Some(wave.unwrap_or(s.primary)))
        }
        TermKind::RayPath {
            field,
            wave,
            surf,
            to,
            q,
            upper,
            ..
        } => {
            ray::check_path(i, s, *surf, *to, *q, *upper)?;
            (*field, Some(wave.unwrap_or(s.primary)))
        }
        TermKind::Opd { field, wave, .. } | TermKind::FNum { field, wave, .. } => {
            (*field, Some(wave.unwrap_or(s.primary)))
        }
        TermKind::Zernike {
            field,
            wave,
            kind,
            term,
            fit,
            n,
            ..
        } => {
            let max = if *kind == ZernKind::Fringe { 37 } else { 2016 };
            if *term == 0 || *term > max {
                invalid!("term {i}: Zernike term {term} (1 to {max})");
            }
            if let Some(f) = fit.filter(|f| *f < *term || *f > max) {
                invalid!("term {i}: Zernike terms to fit {f} ({term} to {max})");
            }
            if *n < 8 {
                invalid!("term {i}: Zernike grid {n}");
            }
            (*field, Some(wave.unwrap_or(s.primary)))
        }
        TermKind::Energy { .. } | TermKind::Strehl { .. } | TermKind::Illum { .. } => {
            return image::check(i, k, s)
        }
        TermKind::Param { param, .. } => return surf::check_param(i, *param, s),
        TermKind::Spread { term, .. } if term.target().is_none() => {
            invalid!("term {i}: the term of a spread term must have a single value")
        }
        TermKind::Spread { term, .. } => return check_kind(i, term, Side::Eq, s),
        TermKind::Diff { term, .. } if term.target().is_none() => {
            invalid!("term {i}: the term of a difference term must have a single value")
        }
        TermKind::Diff { term, .. } => return check_kind(i, term, Side::Eq, s),
        TermKind::Coupling { wave, .. } => {
            if s.infinite() {
                invalid!("term {i}: fiber coupling is only for finite-object systems");
            }
            (FieldSel::Norm(0.0, 0.0), Some(wave.unwrap_or(s.primary)))
        }
        TermKind::Thickness { surf, to, .. } => {
            if *surf == 0 || surf > to || *to >= s.image() {
                invalid!(
                    "term {i}: thickness of surfaces {surf} to {to} (1 to {})",
                    s.image() - 1
                );
            }
            return Ok(());
        }
        TermKind::Surface { surf, q, wave, .. } => return surf::check(i, *surf, *q, *wave, s),
        TermKind::Cardinal { .. } => return card::check(i, k, s),
        TermKind::SurfEach { .. } => return surf::semi_check(i, k, s),
        TermKind::Volume { .. } | TermKind::Shape { .. } => return vol::check(i, k, s),
        TermKind::Seidel { .. }
        | TermKind::Distortion { .. }
        | TermKind::Longitudinal { .. }
        | TermKind::LateralColor { .. } => return seidel::check(i, k, s),
        TermKind::Calc { op, args, .. } => {
            let (lo, hi) = op.arity();
            let n = args.len();
            if n < lo || n > hi {
                let op = op.name();
                if lo == hi {
                    invalid!("term {i}: {op} takes {lo} arguments ({n})");
                }
                invalid!("term {i}: {op} takes {lo} or more arguments ({n})");
            }
            for a in args {
                match a {
                    CalcArg::Term(k) if k.target().is_none() => {
                        invalid!("term {i}: an argument of a calc term must be a term with a single value");
                    }
                    CalcArg::Term(k) => check_kind(i, k, Side::Eq, s)?,
                    CalcArg::Const(c) if !c.is_finite() => invalid!("term {i}: constant {c}"),
                    CalcArg::Const(_) => {}
                }
            }
            (FieldSel::Norm(0.0, 0.0), Some(s.primary))
        }
        TermKind::User(u) => {
            if u.0.rows() == 0 {
                invalid!("term {i}: user term {} has 0 residuals", u.0.name());
            }
            return Ok(());
        }
    };
    if let Some(tg) = k.target() {
        if !tg.is_finite() {
            invalid!("term {i}: target {tg}");
        }
    } else if side != Side::Eq {
        invalid!("term {i}: min and max need a term with a target");
    }
    if let FieldSel::Index(f) = field {
        if f >= s.fields.len() {
            invalid!("term {i}: field {f} (of {})", s.fields.len());
        }
    }
    if let Some(w) = wave {
        if w >= s.wavelengths.len() {
            invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
        }
    }
    if wave.is_none() && waves(s, None).is_empty() {
        invalid!("term {i}: no weighted wavelength");
    }
    Ok(())
}

fn supports_analytic(s: &System, k: &TermKind) -> bool {
    match k {
        TermKind::Spot { .. } | TermKind::Centroid { .. } => true,
        TermKind::Wavefront { .. } | TermKind::Opd { .. } => {
            flat_image(s) && (s.ropd == 1 || s.ropd == 2)
        }
        TermKind::Ray { q, .. } => !q.by_diff(),
        _ => false,
    }
}

/// 開始の光線を比べる瞳の点
const PROBE: [(f64, f64); 4] = [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (-0.7, -0.7)];

/// 項の残差（重みを掛ける前）。ps を与えると、その列の解析的なヤコビアンも
fn eval_term(
    s: &System,
    k: &TermKind,
    ps: Option<&[Param]>,
    pc: Option<&PupilCache>,
    keep: bool,
) -> Result<TermOut> {
    match k {
        TermKind::Spot {
            field,
            wave,
            rings,
            reference,
            quad,
            axis,
        } => spot_term(
            s,
            *field,
            *wave,
            (*rings, *quad, *axis),
            *reference,
            ps,
            keep,
        ),
        TermKind::Centroid {
            field,
            wave,
            rings,
            y,
            target,
        } => centroid_term(s, *field, *wave, *rings, *y, *target, ps, keep),
        TermKind::Wavefront {
            field,
            wave,
            rings,
            reference,
            quad,
        } => wfe_term(s, *field, *wave, (*rings, *quad), *reference, ps, pc, keep),
        TermKind::Mtf {
            field,
            wave,
            n,
            freq,
            dir,
            kind,
            target,
        } => Ok((
            image::mtf_term(s, *field, *wave, *n, *freq, *dir, *kind, *target)?,
            None,
        )),
        TermKind::Paraxial { q, wave, target } => {
            let v = q.value_at(s, *wave);
            if !v.is_finite() {
                invalid!("{} of the system is not finite", q.name());
            }
            Ok((vec![v - target], None))
        }
        TermKind::Ray {
            field,
            wave,
            px,
            py,
            surf,
            q,
            target,
        } => ray::ray_term(s, *field, *wave, *px, *py, *surf, *q, *target, ps),
        TermKind::RayPath { .. } => ray::path_term(s, k),
        TermKind::ParaxRay { .. } => ray::parax_term(s, k),
        TermKind::Opd {
            field,
            wave,
            px,
            py,
            target,
        } => opd_term(s, *field, *wave, *px, *py, *target, ps, pc),
        TermKind::Zernike {
            field,
            wave,
            kind,
            term,
            fit,
            n,
            target,
        } => {
            // 格子の上の最小二乗では、当てはめる項の数で低い次の係数が動く（Double Gauss 28 の視野 2 で Standard Z11 が
            // 11 項で 0.04221、37 項で 0.041004、231 項で 0.041002 波）。既定は項を多くとった値（直交射影）に寄せるため 37 項
            // 以上で当てはめる。OpticStudio 26.1 の ZERN は Fringe がいつも 37 項、Standard が求める項まで（.zmx から読んだ項は
            // fit をそれに合わせる）
            let nterms = fit.unwrap_or((*term).max(ZERN_MIN_TERMS));
            let z = zernike(s, *field, *wave, *kind, nterms, *n, false)?;
            let v = z.coef[term - 1];
            if !v.is_finite() {
                invalid!("can't compute the Zernike coefficients");
            }
            Ok((vec![v - target], None))
        }
        TermKind::Coupling { wave, opts, target } => {
            let c = fiber_coupling(s, wave.unwrap_or(s.primary), opts, None)?;
            if !c.eta.is_finite() {
                invalid!("can't compute the coupling efficiency");
            }
            Ok((vec![c.eta - target], None))
        }
        TermKind::Thickness { .. } => surf::thickness_term(s, k),
        TermKind::Volume { .. } | TermKind::Shape { .. } => vol::term(s, k),
        TermKind::Surface { .. } => surf::term(s, k),
        TermKind::Cardinal { .. } => card::term(s, k),
        TermKind::SurfEach { .. } => surf::semi_term(s, k),
        TermKind::Seidel { .. }
        | TermKind::Distortion { .. }
        | TermKind::Longitudinal { .. }
        | TermKind::LateralColor { .. } => seidel::term(s, k),
        TermKind::FNum { .. } => ray::fnum_term(s, k),
        TermKind::Param { param, target } => Ok((vec![s.get_param(*param)? - target], None)),
        TermKind::Energy { .. } | TermKind::Strehl { .. } | TermKind::Illum { .. } => {
            Ok((vec![image::term(s, k)?], None))
        }
        TermKind::Calc { op, args, target } => {
            let v: Vec<f64> = args
                .iter()
                .map(|a| match a {
                    CalcArg::Const(c) => Ok(*c),
                    CalcArg::Term(k) => {
                        let r = eval_term(s, k, None, None, keep)?.0;
                        Ok(r[0] + k.target().unwrap_or(0.0))
                    }
                })
                .collect::<Result<_>>()?;
            let x = op.apply(&v)?;
            if !x.is_finite() {
                invalid!("a calc term is not finite");
            }
            Ok((vec![x - target], None))
        }
        TermKind::User(u) => Ok((u.eval_checked(&[s])?.remove(0), None)),
        TermKind::Spread { .. } | TermKind::Diff { .. } => {
            invalid!("a spread term is evaluated over all the configurations")
        }
    }
}

/// 1 本の光線の OPD の項（[`lensbench_analysis::analysis::opd`] の ref_primary = true と同じ値）
#[allow(clippy::too_many_arguments)]
fn opd_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    px: f64,
    py: f64,
    target: f64,
    ps: Option<&[Param]>,
    pcache: Option<&PupilCache>,
) -> Result<TermOut> {
    lensbench_core::pupil::check_ropd(s)?;
    let w = wave.unwrap_or(s.primary);
    let rec = ps.is_some();
    let t = trace_ok(s, field, &[px], &[py], w, rec)?;
    let tc = trace_ok(s, field, &[0.0], &[0.0], w, rec)?;
    let wr = s.primary;
    let tp = if wr == w {
        tc.clone()
    } else {
        trace_ok(s, field, &[0.0], &[0.0], wr, rec)?
    };
    let (pc, kc) = (tp.rays[0].xyz, tp.rays[0].lmn);
    let c = &tc.rays[0];
    let x = &t.rays[0];
    let wl = s.wave(w).mm();
    let lc = to_ref(s, w, pc, kc, c.xyz, c.lmn, c.opl) + c.lens_opd;
    let v = (lc - (to_ref(s, w, pc, kc, x.xyz, x.lmn, x.opl) + x.lens_opd)) / wl;
    if !v.is_finite() {
        invalid!("can't compute the OPD");
    }
    let Some(ps) = ps else {
        return Ok((vec![v - target], None));
    };
    if [x, c, &tp.rays[0]].iter().any(|r| r.lens_opd != 0.0) {
        invalid!("a system with an ideal lens");
    }
    let ni = s.image_index(w);
    let nr = s.image_index(wr);
    let gl = s.land_grad(&t, w, ps, None)?;
    let go = s.opl_grad(&t, w, ps)?;
    let glc = s.land_grad(&tc, w, ps, None)?;
    let goc = s.opl_grad(&tc, w, ps)?;
    let (glp, gop) = if wr == w {
        (glc.clone(), goc.clone())
    } else {
        (s.land_grad(&tp, wr, ps, None)?, s.opl_grad(&tp, wr, ps)?)
    };
    let mut d = Vec::with_capacity(ps.len());
    for (q, &p) in ps.iter().enumerate() {
        let pd = pupil_d(s, p, pcache)?;
        let (de, dn) = (pd.de, pd.dn(w));
        let (dpc, dkc, _) = ray_d(nr, kc, &glp[q * 4..q * 4 + 4], gop[q]);
        let (dqc, dkcw, doc) = ray_d(ni, c.lmn, &glc[q * 4..q * 4 + 4], goc[q]);
        let dlc = to_ref_d(
            s,
            ni,
            pc,
            kc,
            c.xyz,
            c.lmn,
            (dpc, dkc, dqc, dkcw, doc, dn, de),
        );
        let (dq, dk, dop) = ray_d(ni, x.lmn, &gl[q * 4..q * 4 + 4], go[q]);
        let dl = to_ref_d(s, ni, pc, kc, x.xyz, x.lmn, (dpc, dkc, dq, dk, dop, dn, de));
        d.push((dlc - dl) / wl);
    }
    if d.iter().any(|v| !v.is_finite()) {
        invalid!("can't find the OPD derivatives");
    }
    Ok((vec![v - target], Some(d)))
}

/// to_ref（[`lensbench_analysis::analysis`]）の値の微分。ni は像空間の屈折率（波長 w の [`System::image_index`]）、d は (dpc, dkc, dq, dk, dopl, dni, dexpp)
#[allow(clippy::too_many_arguments)]
fn to_ref_d(
    s: &System,
    ni: f64,
    pc: V3,
    kc: V3,
    q: V3,
    k: V3,
    d: (V3, V3, V3, V3, f64, f64, f64),
) -> f64 {
    let (dpc, dkc, dq, dk, dopl, dni, dexpp) = d;
    let expp = s.exit_pupil();
    let dd = sub(q, pc);
    let ddd = sub(dq, dpc);
    if s.afocal && s.ropd == 3 {
        return dopl;
    }
    if s.afocal {
        // opl + ni a / b、a = (pe − q)·kc、pe = 射出瞳の位置で主光線の上の点（無限遠・ROPD 0 なら pc）
        let (pe, dpe) = if exit_pupil_near(expp) && s.ropd != 0 {
            let u = (expp - pc[2]) / kc[2];
            let du = (dexpp - dpc[2]) / kc[2] - u * dkc[2] / kc[2];
            (
                add(pc, scale(kc, u)),
                add(add(dpc, scale(dkc, u)), scale(kc, du)),
            )
        } else {
            (pc, dpc)
        };
        let a = dot(sub(pe, q), kc);
        let da = dot(sub(dpe, dq), kc) + dot(sub(pe, q), dkc);
        let b = dot(k, kc);
        let db = dot(dk, kc) + dot(k, dkc);
        return dopl + dni * a / b + ni * (da / b - a * db / (b * b));
    }
    if RefSurface::far_for(s) {
        // opl − ni (q − pc)·k
        let b = dot(dd, k);
        return dopl - dni * b - ni * (dot(ddd, k) + dot(dd, dk));
    }
    let sc = (expp - pc[2]) / kc[2];
    let dsc = (dexpp - dpc[2]) / kc[2] - sc * dkc[2] / kc[2];
    let rr = sc.abs();
    let drr = if sc < 0.0 { -dsc } else { dsc };
    let b = dot(dd, k);
    let db = dot(ddd, k) + dot(dd, dk);
    let c = dot(dd, dd) - rr * rr;
    let dc = 2.0 * dot(dd, ddd) - 2.0 * rr * drr;
    let e = b * b - c;
    let (disc, ddisc) = if e > 0.0 {
        let disc = e.sqrt();
        (disc, (2.0 * b * db - dc) / (2.0 * disc))
    } else {
        (0.0, 0.0)
    };
    let t = if sc < 0.0 { -b - disc } else { -b + disc };
    let dt = if sc < 0.0 { -db - ddisc } else { -db + ddisc };
    dopl + dni * t + ni * dt
}

/// 着地点の微分 g（[x, y, l, m]）と光路長の微分 go から、平面の像面での (dq, dk, dopl)。
/// 光路長は開始を固定した微分: 端点を固定した微分 + n (l dx + m dy)
fn ray_d(ni: f64, lmn: V3, g: &[f64], go: f64) -> (V3, V3, f64) {
    let dq = [g[0], g[1], 0.0];
    let dn = -(lmn[0] * g[2] + lmn[1] * g[3]) / lmn[2];
    let dk = [g[2], g[3], dn];
    (dq, dk, go + ni * (lmn[0] * g[0] + lmn[1] * g[1]))
}

#[allow(clippy::too_many_arguments)]
fn wfe_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    (rings, quad): (usize, Quad),
    reference: RmsRef,
    ps: Option<&[Param]>,
    pc: Option<&PupilCache>,
    keep: bool,
) -> Result<TermOut> {
    let (pg, wts) = apodized_gauss(s, field, rings, quad, true)?;
    let (px, py, wts) = (&pg.0, &pg.1, &wts);
    let n = px.len();
    let ws = waves(s, wave);
    let rec = ps.is_some();
    let np = ps.map_or(0, |p| p.len());
    // 波長の重み（rms() と同じ: 単色は 1、多色は波長の重みで平均）
    let kw: Vec<f64> = ws
        .iter()
        .map(|&w| if ws.len() == 1 { 1.0 } else { s.weights[w] })
        .collect();
    let ksum: f64 = kw.iter().sum();
    // 近軸の量の微分（射出瞳の位置・像空間の屈折率）
    let par_d: Vec<(f64, Vec<f64>)> = match ps {
        Some(ps) => ps
            .iter()
            .map(|&p| {
                let pd = pupil_d(s, p, pc)?;
                Ok((pd.de, ws.iter().map(|&w| pd.dn(w)).collect()))
            })
            .collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let m = ws.len() * n;
    let mut r = vec![0.0; m];
    let mut jac = ps.map(|_| vec![0.0; m * np]);
    // sweep::rms と同じ定義（OpticStudio の RMS 波面）: 単色は自波長の主光線、多色は主波長の主光線を参照球の中心に。
    // chief は波長ごとのピストンを、centroid はそれに加えて方向余弦の差 (L − Lc, M − Mc) の係数（多色は全波長で共通、
    // Lc は参照の主光線）を重み付き最小二乗で除く
    let poly = wave.is_none();
    let nw = ws.len();
    let nt = if reference == RmsRef::Centroid { 2 } else { 0 };
    let nu = nw + nt;
    // 有効な光線ごと: (波長 k, 光線 i, OPD, L − Lc, M − Mc, 波長の中の重み q)
    let mut rows: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::with_capacity(m);
    // 微分（rows と同じ順）: dOPD・d(L − Lc)・d(M − Mc) [パラメータ]
    let mut drows: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> = Vec::new();
    for (k, &w) in ws.iter().enumerate() {
        // 値は列（位置・方向・光路長・理想レンズの位相）で読む。微分が要るときだけ記録つきの Trace も持つ
        let wr = if poly { s.primary } else { w };
        let (b, bc, bp, tr) = if rec {
            let (t, b) = trace_pupil_rec(s, field, px, py, w, keep, Cols::WAVE)?;
            let (tc, bc) = trace_pupil_rec(s, field, &[0.0], &[0.0], w, false, Cols::WAVE)?;
            let (tp, bp) = if wr == w {
                (tc.clone(), bc.clone())
            } else {
                trace_pupil_rec(s, field, &[0.0], &[0.0], wr, false, Cols::WAVE)?
            };
            (b, bc, bp, Some((t, tc, tp)))
        } else {
            let b = trace_pupil_cols(s, field, px, py, w, keep, Cols::WAVE)?;
            let bc = trace_pupil_cols(s, field, &[0.0], &[0.0], w, false, Cols::WAVE)?;
            let bp = if wr == w {
                bc.clone()
            } else {
                trace_pupil_cols(s, field, &[0.0], &[0.0], wr, false, Cols::WAVE)?
            };
            (b, bc, bp, None)
        };
        let (cxyz, clmn) = (bc.xyz[0], bc.lmn[0]);
        let (pc, kc) = (bp.xyz[0], bp.lmn[0]);
        let to = ref_opl(s, w, pc, kc);
        let lc = to(cxyz, clmn, bc.opl[0]) + bc.lens_opd[0];
        let wl = s.wave(w).mm();
        let opd: Vec<f64> = (0..b.len())
            .map(|i| (lc - (to(b.xyz[i], b.lmn[i], b.opl[i]) + b.lens_opd[i])) / wl)
            .collect();
        let idx: Vec<usize> = (0..n).filter(|&i| b.ok(i) && opd[i].is_finite()).collect();
        let sw: f64 = idx.iter().map(|&i| wts[i]).sum();
        if idx.is_empty() || !(sw > 0.0) {
            invalid!("no valid rays (all vignetted)");
        }
        for &i in &idx {
            // アフォーカルの傾きの基底は瞳座標（sweep::rms と同じ）
            let (bl, bm) = if s.afocal {
                (px[i], py[i])
            } else {
                (b.lmn[i][0] - kc[0], b.lmn[i][1] - kc[1])
            };
            rows.push((k, i, opd[i], bl, bm, wts[i] / sw));
        }
        if let (Some(ps), Some((t, tc, tp))) = (ps, tr.as_ref()) {
            let c = &tc.rays[0];
            if t.rays
                .iter()
                .chain(&tc.rays)
                .chain(&tp.rays)
                .any(|x| x.lens_opd != 0.0)
            {
                invalid!("a system with an ideal lens");
            }
            let ni = s.image_index(w);
            let gl = s.land_grad(t, w, ps, None)?;
            let go = s.opl_grad(t, w, ps)?;
            let glc = s.land_grad(tc, w, ps, None)?;
            let goc = s.opl_grad(tc, w, ps)?;
            let (glp, gop) = if wr == w {
                (glc.clone(), goc.clone())
            } else {
                (s.land_grad(tp, wr, ps, None)?, s.opl_grad(tp, wr, ps)?)
            };
            let nr = s.image_index(wr);
            let mut d: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> =
                vec![(vec![0.0; np], vec![0.0; np], vec![0.0; np]); idx.len()];
            for q in 0..np {
                let (de, ref dn) = par_d[q];
                // 参照の主光線（参照球の中心）と、この波長の主光線
                let (dpc, dkc, _) = ray_d(nr, kc, &glp[q * 4..q * 4 + 4], gop[q]);
                let (dqc, dkc_w, doc) = ray_d(ni, c.lmn, &glc[q * 4..q * 4 + 4], goc[q]);
                let dlc = to_ref_d(
                    s,
                    ni,
                    pc,
                    kc,
                    c.xyz,
                    c.lmn,
                    (dpc, dkc, dqc, dkc_w, doc, dn[k], de),
                );
                for (j, &i) in idx.iter().enumerate() {
                    let x = &t.rays[i];
                    let (dq, dk, dop) = ray_d(
                        ni,
                        x.lmn,
                        &gl[(i * np + q) * 4..(i * np + q) * 4 + 4],
                        go[i * np + q],
                    );
                    let dl = to_ref_d(
                        s,
                        ni,
                        pc,
                        kc,
                        x.xyz,
                        x.lmn,
                        (dpc, dkc, dq, dk, dop, dn[k], de),
                    );
                    d[j].0[q] = (dlc - dl) / wl;
                    if !s.afocal {
                        d[j].1[q] = dk[0] - dkc[0];
                        d[j].2[q] = dk[1] - dkc[1];
                    }
                }
            }
            drows.extend(d);
        }
    }
    // 当てはめの基底（未知数: 波長ごとのピストン nw 個 + 傾き nt 個）。光線ごとの nu 個を 1 本の配列に並べる
    // 区切りの長さ（未知数がない = 波長が 0 個のときは光線もなく、配列は空。長さ 0 では区切れないので 1 に）
    let nc = nu.max(1);
    let mut bs = vec![0.0; rows.len() * nu];
    for (row, b) in rows.iter().zip(bs.chunks_exact_mut(nc)) {
        b[row.0] = 1.0;
        if nt == 2 {
            b[nw] = row.3;
            b[nw + 1] = row.4;
        }
    }
    let basis = |j: usize| -> &[f64] { &bs[j * nu..(j + 1) * nu] };
    // 正規方程式（g は nu × nu を行ごとに並べる。足す順・積の順は光線ごと・a ごと・c ごとのまま）
    let mut gf = vec![0.0; nu * nu];
    let mut v = vec![0.0; nu];
    for (row, b) in rows.iter().zip(bs.chunks_exact(nc)) {
        for ((va, ga), &ba) in v.iter_mut().zip(gf.chunks_exact_mut(nc)).zip(b) {
            let qa = row.5 * ba;
            *va += qa * row.2;
            for (gc, &bc) in ga.iter_mut().zip(b) {
                *gc += qa * bc;
            }
        }
    }
    let g: Vec<Vec<f64>> = gf.chunks_exact(nc).map(<[f64]>::to_vec).collect();
    let cf = lensbench_core::linalg::solve_or_zero(g.clone(), v);
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let e: Vec<f64> = rows
        .iter()
        .zip(bs.chunks_exact(nc))
        .map(|(row, b)| row.2 - dot(b, &cf))
        .collect();
    let amp: Vec<f64> = rows
        .iter()
        .map(|row| (kw[row.0] / ksum * row.5).sqrt())
        .collect();
    for (j, row) in rows.iter().enumerate() {
        r[row.0 * n + row.1] = amp[j] * e[j];
    }
    if let Some(jm) = jac.as_mut() {
        // e = y − B c、c = G⁻¹ Bᵀ Q y の微分:
        // de = (dy − dB c) − B dc、dc = G⁻¹ (dBᵀ Q e + Bᵀ Q (dy − dB c))
        for q in 0..np {
            let dbc = |j: usize| -> f64 {
                if nt == 2 {
                    drows[j].1[q] * cf[nw] + drows[j].2[q] * cf[nw + 1]
                } else {
                    0.0
                }
            };
            let u: Vec<f64> = (0..rows.len()).map(|j| drows[j].0[q] - dbc(j)).collect();
            let mut rhs = vec![0.0; nu];
            for (j, row) in rows.iter().enumerate() {
                for a in 0..nu {
                    rhs[a] += row.5 * basis(j)[a] * u[j];
                }
                if nt == 2 {
                    rhs[nw] += row.5 * drows[j].1[q] * e[j];
                    rhs[nw + 1] += row.5 * drows[j].2[q] * e[j];
                }
            }
            let dc = lensbench_core::linalg::solve_or_zero(g.clone(), rhs);
            for (j, row) in rows.iter().enumerate() {
                jm[(row.0 * n + row.1) * np + q] = amp[j] * (u[j] - dot(basis(j), &dc));
            }
        }
    }
    Ok((r, jac))
}
