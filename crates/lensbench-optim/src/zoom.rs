//! ズーム軌跡: 状態として並べたズーム位置ごとに、群の位置（移動・間隔）を解き、状態の間を細かくたどる。
//!
//! 状態ごとに、動かすパラメータ（[`ZoomSpec::moving`]。`G2.shift` などの群の移動や間隔）を変数にして、
//! [`crate`] の減衰最小二乗で解く。評価関数は既定の像の項（全視野・多色の RMS スポットか RMS 波面）と、
//! 目標があれば近軸量（焦点距離か倍率、[`ParaxQ`]）の項。近軸量の項の重みは、像の項の重みの合計の
//! [`ZoomSpec::target_weight`] 倍から始め、目標との差が [`ZoomSpec::target_tol`] に収まるまで上げて解き直す
//! （目標をほぼ拘束として満たす。重みを初めから大きくすると条件が悪く反復が遅い）。
//!
//! 状態の間（[`ZoomSpec::steps`] 点ずつ）は継続法でたどる: 目標を前の状態の値から次の状態の値へ少しずつ動かし
//! （同じ符号なら等比、違えば等差）、一つ前の点の解に、両端の状態の解の差の分を足した所から解く。
//! 目標のない状態では、解いた後の近軸量の値を間の点の目標に使う。間の点の系は前の状態の系に変数の値を入れたもの
//! （状態どうしが動かすパラメータのほかにも違うときは警告を返す）。
//!
//! 1 点が解けなかった（追跡できない系になったなど）ときは、その点を [`PointStop::Failed`] として残し（変数は解き始めの値、
//! 近軸量・評価関数は NaN）、警告を足して残りの点を続ける。設定の誤り（問題の確かめを通らない: 動かすパラメータが
//! 状態の系にない、範囲が逆など）と中断（[`lensbench_core::Error::Cancelled`]）は全体を止めて誤りを返す。
//!
//! 目標の満たし方（[`TargetMethod`]）: [`TargetMethod::Penalty`] は上の重みを上げる方法。
//! 既定の [`TargetMethod::Lagrange`] は拡大ラグランジュ法（重みは [`ZoomSpec::target_weight`] のまま、解いた近軸量と目標の差だけ
//! 項の目標をずらして解き直す。重みを上げないので条件が悪くならず、目標に正確に合う。P17 の目標 70・100・170 mm では
//! 像の評価関数が罰金より小さく（0.2075 → 0.2048、0.0079 → 0.0061）、時間も半分。2026-11-16 masa 決定で既定に）。
//!
//! 状態を持たないので、一括実行のタスクの中からも呼べる。

use crate::{DefaultMerit, DlsOpts, ParaxQ, Problem, Stop, Term, TermKind, Var};
use lensbench_core::error::{Error, Result};
use lensbench_core::invalid;
use lensbench_core::param::Param;
use lensbench_core::sweep::RmsRef;
use lensbench_core::system::System;

/// 近軸量の目標の満たし方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMethod {
    /// 罰金: 近軸量の項の重みを、目標に届くまで 100 倍ずつ 3 回まで上げて解き直す
    Penalty,
    /// 拡大ラグランジュ法: 重みはそのまま、解いた近軸量と目標の差だけ項の目標をずらして解き直す（最大 8 回）
    #[default]
    Lagrange,
}

impl TargetMethod {
    /// 名前（"penalty"・"lagrange"）から
    pub fn parse(s: &str) -> Option<TargetMethod> {
        match s {
            "penalty" => Some(TargetMethod::Penalty),
            "lagrange" => Some(TargetMethod::Lagrange),
            _ => None,
        }
    }

    /// 名前
    pub fn name(self) -> &'static str {
        match self {
            TargetMethod::Penalty => "penalty",
            TargetMethod::Lagrange => "lagrange",
        }
    }
}

/// ズーム軌跡の設定
#[derive(Debug, Clone)]
pub struct ZoomSpec {
    /// 状態ごとに解くパラメータ（群の移動・間隔など）
    pub moving: Vec<Param>,
    /// 動かすパラメータの範囲（moving と同じ長さ。空なら ±∞）
    pub bounds: Vec<(f64, f64)>,
    /// 近軸量の種類
    pub q: ParaxQ,
    /// 状態ごとの近軸量の目標（None は目標なし: 像の項だけで解く）。長さは状態の数
    pub targets: Vec<Option<f64>>,
    /// 近軸量の項の重みの初め（像の項の重みの合計に対する比）。目標との差が target_tol を超えれば 100 倍ずつ
    /// 3 回まで上げて解き直す
    pub target_weight: f64,
    /// 近軸量の目標との差の許し（目標に対する比）
    pub target_tol: f64,
    /// 目標の満たし方
    pub method: TargetMethod,
    /// 像の項
    pub merit: DefaultMerit,
    pub rings: usize,
    pub reference: RmsRef,
    /// 隣り合う状態の間にとる点の数
    pub steps: usize,
    pub opts: DlsOpts,
    /// 止まった光線（全反射・面に届かない）をけられた光線と同じく除く（[`Problem::stopped_as_vignetted`]）
    pub stopped_as_vignetted: bool,
}

impl ZoomSpec {
    pub fn new(moving: Vec<Param>, targets: Vec<Option<f64>>) -> ZoomSpec {
        ZoomSpec {
            moving,
            bounds: Vec::new(),
            q: ParaxQ::Efl,
            targets,
            target_weight: 10.0,
            target_tol: 1e-8,
            method: TargetMethod::Lagrange,
            merit: DefaultMerit::Spot,
            rings: 3,
            reference: RmsRef::Centroid,
            steps: 0,
            opts: DlsOpts::default(),
            stopped_as_vignetted: false,
        }
    }
}

/// 1 点の止まり方
#[derive(Debug, Clone, PartialEq)]
pub enum PointStop {
    /// 解いた（DLS の止まった理由）
    Solved(Stop),
    /// 解けなかった（理由）。点は解き始めの値のまま
    Failed(String),
}

impl PointStop {
    /// 出力用の名前（DLS の止まった理由か `failed`）
    pub fn as_str(&self) -> &'static str {
        match self {
            PointStop::Solved(s) => s.as_str(),
            PointStop::Failed(_) => "failed",
        }
    }
    /// 解けなかった理由
    pub fn error(&self) -> Option<&str> {
        match self {
            PointStop::Solved(_) => None,
            PointStop::Failed(e) => Some(e),
        }
    }
}

/// 軌跡の 1 点
#[derive(Debug, Clone)]
pub struct ZoomPoint {
    /// 状態の番号（0 始まり）。間の点は None
    pub state: Option<usize>,
    /// 軌跡の上の位置（状態 k なら k、状態 k と k+1 の間は k + 割合）
    pub pos: f64,
    /// 近軸量の目標（None は目標なし）
    pub target: Option<f64>,
    /// 解いた系の近軸量
    pub value: f64,
    /// 動かすパラメータの値
    pub x: Vec<f64>,
    /// 像の項だけの評価関数の値（視野の重みで平均した RMS）
    pub merit: f64,
    pub stop: PointStop,
    pub iterations: usize,
}

/// ズーム軌跡の結果
#[derive(Debug, Clone)]
pub struct ZoomResult {
    /// 状態ごとの解
    pub states: Vec<ZoomPoint>,
    /// 状態と間の点を順に並べたもの（steps 0 なら states と同じ）
    pub path: Vec<ZoomPoint>,
    /// 状態ごとの解いた系
    pub systems: Vec<System>,
    pub warnings: Vec<String>,
}

fn problem(
    s: &System,
    spec: &ZoomSpec,
    target: Option<f64>,
    weight: f64,
) -> Result<(Problem, Problem)> {
    let mut img = Problem::single(s.clone());
    img.stopped_as_vignetted = spec.stopped_as_vignetted;
    for (i, &p) in spec.moving.iter().enumerate() {
        let mut v = Var::new(p);
        if let Some(&(lo, hi)) = spec.bounds.get(i) {
            v = v.bounds(lo, hi);
        }
        img.vars.push(v);
    }
    img.add_default(spec.merit, spec.rings, spec.reference);
    let mut full = img.clone();
    if let Some(t) = target {
        let w: f64 = img.terms.iter().map(|t| t.weight).sum();
        full.terms.push(Term {
            side: Default::default(),
            config: 0,
            weight: weight * w,
            kind: TermKind::Paraxial {
                q: spec.q,
                wave: None,
                target: t,
            },
        });
    }
    Ok((full, img))
}

/// 系 s（変数の初期値は s の値）で 1 点を解く。解けなければ [`PointStop::Failed`] の点（と s）を返す。
/// 設定の誤り（問題の確かめ [`Problem::check`] を通らない: 動かすパラメータが系にない、範囲が逆など）と
/// 中断は誤りで返す
fn solve_point(
    s: &System,
    spec: &ZoomSpec,
    target: Option<f64>,
    state: Option<usize>,
    pos: f64,
) -> Result<(ZoomPoint, System)> {
    match try_point(s, spec, target, state, pos)? {
        Ok(v) => Ok(v),
        Err(e) => {
            let x = spec
                .moving
                .iter()
                .map(|&p| s.get_param(p))
                .collect::<Result<Vec<f64>>>()?;
            Ok((
                ZoomPoint {
                    state,
                    pos,
                    target,
                    value: f64::NAN,
                    x,
                    merit: f64::NAN,
                    stop: PointStop::Failed(e.to_string()),
                    iterations: 0,
                },
                s.clone(),
            ))
        }
    }
}

/// 解けない点の誤り（追跡できない系など）は内側の Err、設定の誤りと中断は外側の Err
fn unsolved<T>(r: Result<T>) -> Result<std::result::Result<T, Error>> {
    match r {
        Ok(v) => Ok(Ok(v)),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(_) if lensbench_core::par::cancelled() => Err(Error::Cancelled),
        Err(e) => Ok(Err(e)),
    }
}

type PointResult = std::result::Result<(ZoomPoint, System), Error>;

fn try_point(
    s: &System,
    spec: &ZoomSpec,
    target: Option<f64>,
    state: Option<usize>,
    pos: f64,
) -> Result<PointResult> {
    let lagrange = spec.method == TargetMethod::Lagrange;
    // 罰金: 近軸量の項の重みを、目標に届くまで 100 倍ずつ上げて解き直す（重みを初めから大きくすると条件が悪く、
    // 反復が遅い）。拡大ラグランジュ法: 重みはそのまま、目標との差だけ項の目標をずらす
    let mut w = spec.target_weight;
    let mut shifted = target;
    let mut cur = s.clone();
    let mut iterations = 0;
    let mut tries = 0;
    let max_tries = if lagrange { 9 } else { 4 };
    let (r, img) = loop {
        let (full, img) = problem(&cur, spec, shifted, w)?;
        full.check()?;
        let r = match unsolved(crate::dls_inner(&full, &spec.opts))? {
            Ok(r) => r,
            Err(e) => return Ok(Err(e)),
        };
        iterations += r.iterations;
        tries += 1;
        let sys = &r.systems[0];
        let miss = target.map(|t| spec.q.value(sys) - t);
        let ok = target.is_none_or(|t| {
            miss.expect("miss is set whenever there is a target").abs()
                <= spec.target_tol * t.abs().max(f64::MIN_POSITIVE)
        });
        if ok || tries >= max_tries {
            break (r, img);
        }
        cur = sys.clone();
        if lagrange {
            shifted = shifted.zip(miss).map(|(a, m)| a - m);
        } else {
            w *= 100.0;
        }
    };
    let sys = r
        .systems
        .into_iter()
        .next()
        .expect("the optimizer returns one system per configuration");
    let merit = match unsolved(img.merit(&r.x))? {
        Ok(m) => m,
        Err(e) => return Ok(Err(e)),
    };
    Ok(Ok((
        ZoomPoint {
            state,
            pos,
            target,
            value: spec.q.value(&sys),
            x: r.x,
            merit,
            stop: PointStop::Solved(r.stop),
            iterations,
        },
        sys,
    )))
}

/// 目標を a から b へ割合 f だけ動かした値（同じ符号なら等比）
fn interp(a: f64, b: f64, f: f64) -> f64 {
    if a * b > 0.0 {
        a * (b / a).powf(f)
    } else {
        a + (b - a) * f
    }
}

/// 状態（configs、ズーム位置ごとの系）ごとに群の位置を解き、間をたどる
pub fn solve(configs: &[System], spec: &ZoomSpec) -> Result<ZoomResult> {
    let n = configs.len();
    if n == 0 {
        invalid!("no states");
    }
    if spec.moving.is_empty() {
        invalid!("no parameters to move");
    }
    if spec.targets.len() != n {
        invalid!("{} targets for {n} states", spec.targets.len());
    }
    if !spec.bounds.is_empty() && spec.bounds.len() != spec.moving.len() {
        invalid!(
            "{} ranges for {} parameters",
            spec.bounds.len(),
            spec.moving.len()
        );
    }
    if !(spec.target_weight > 0.0) {
        invalid!("weight of the paraxial term {}", spec.target_weight);
    }
    spec.opts.check()?;
    let mut states = Vec::with_capacity(n);
    let mut systems = Vec::with_capacity(n);
    // 進み具合: 状態ごと・状態の間の点ごとに 1 つ（文脈の受け手へ、lensbench_core::par::with_progress）
    let total = n + n.saturating_sub(1) * spec.steps;
    let t0 = web_time::Instant::now();
    let mut done = 0;
    let mut tick = || {
        done += 1;
        if lensbench_core::par::reporting() {
            let el = t0.elapsed().as_secs_f64();
            lensbench_core::par::report(&lensbench_core::par::Event::Count(
                lensbench_core::par::Count {
                    stage: "",
                    done,
                    total,
                    ran: done,
                    elapsed: el,
                    eta: Some(el / done as f64 * total.saturating_sub(done) as f64),
                },
            ));
        }
    };
    for (k, s) in configs.iter().enumerate() {
        lensbench_core::par::check_cancel()?;
        let (p, sys) = solve_point(s, spec, spec.targets[k], Some(k), k as f64)?;
        tick();
        states.push(p);
        systems.push(sys);
    }
    let mut warnings = Vec::new();
    for p in &states {
        let k = p
            .state
            .expect("every configuration has a state after the solve");
        if let Some(e) = p.stop.error() {
            warnings.push(format!("state {}: not solved ({e})", k + 1));
            continue;
        }
        if let Some(t) = p.target {
            let rel = (p.value - t).abs() / t.abs().max(1e-300);
            if rel > spec.target_tol {
                warnings.push(format!(
                    "state {}: {} does not reach the target ({} / target {t})",
                    k + 1,
                    spec.q.name(),
                    p.value
                ));
            }
        }
    }
    let mut path = Vec::new();
    for k in 0..n {
        path.push(states[k].clone());
        if k + 1 == n || spec.steps == 0 {
            continue;
        }
        // 状態 k の系に状態 k+1 の解を入れて、近軸量が状態 k+1 と同じか（動かすパラメータのほかの違い）
        let ch: Vec<(Param, f64)> = spec
            .moving
            .iter()
            .copied()
            .zip(states[k + 1].x.iter().copied())
            .collect();
        let probe = systems[k].with_params(&ch)?;
        let (va, vb) = (spec.q.value(&probe), states[k + 1].value);
        let solved = states[k].stop.error().is_none() && states[k + 1].stop.error().is_none();
        if solved && (va - vb).abs() > 1e-9 * vb.abs().max(1.0) {
            warnings.push(format!(
                "states {} and {} differ in more than the moved parameters (points between them were found with the system of state {})",
                k + 1,
                k + 2,
                k + 1
            ));
        }
        let (ta, tb) = (
            states[k].target.unwrap_or(states[k].value),
            states[k + 1].target.unwrap_or(states[k + 1].value),
        );
        if !(ta.is_finite() && tb.is_finite()) {
            // 目標のない状態が解けなかった: 間の点の目標が決まらない
            for _ in 0..spec.steps {
                tick();
            }
            warnings.push(format!(
                "points between states {} and {} were skipped (a state without a target was not solved)",
                k + 1,
                k + 2
            ));
            continue;
        }
        let nx = spec.moving.len();
        let dx: Vec<f64> = (0..nx)
            .map(|q| (states[k + 1].x[q] - states[k].x[q]) / (spec.steps + 1) as f64)
            .collect();
        let mut x = states[k].x.clone();
        for j in 1..=spec.steps {
            let f = j as f64 / (spec.steps + 1) as f64;
            let start: Vec<(Param, f64)> = (0..nx)
                .map(|q| {
                    let mut v = x[q] + dx[q];
                    if let Some(&(lo, hi)) = spec.bounds.get(q) {
                        v = v.clamp(lo, hi);
                    }
                    (spec.moving[q], v)
                })
                .collect();
            lensbench_core::par::check_cancel()?;
            let s0 = systems[k].with_params(&start)?;
            let pos = k as f64 + f;
            let (p, _) = solve_point(&s0, spec, Some(interp(ta, tb, f)), None, pos)?;
            tick();
            if let Some(e) = p.stop.error() {
                warnings.push(format!(
                    "between states {} and {} at {f:.3}: not solved ({e})",
                    k + 1,
                    k + 2
                ));
            }
            x = p.x.clone();
            path.push(p);
        }
    }
    Ok(ZoomResult {
        states,
        path,
        systems,
        warnings,
    })
}
