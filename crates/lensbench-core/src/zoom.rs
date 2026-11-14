//! ズーム軌跡: 状態として並べたズーム位置ごとに、群の位置（移動・間隔）を解き、状態の間を細かくたどる。
//!
//! 状態ごとに、動かすパラメータ（[`ZoomSpec::moving`]。`G2.shift` などの群の移動や間隔）を変数にして、
//! [`crate::optim`] の減衰最小二乗で解く。評価関数は既定の像の項（全視野・多色の RMS スポットか RMS 波面）と、
//! 目標があれば近軸量（焦点距離か倍率、[`ParaxQ`]）の項。近軸量の項の重みは、像の項の重みの合計の
//! [`ZoomSpec::target_weight`] 倍から始め、目標との差が [`ZoomSpec::target_tol`] に収まるまで上げて解き直す
//! （目標をほぼ拘束として満たす。重みを初めから大きくすると条件が悪く反復が遅い）。
//!
//! 状態の間（[`ZoomSpec::steps`] 点ずつ）は継続法でたどる: 目標を前の状態の値から次の状態の値へ少しずつ動かし
//! （同じ符号なら等比、違えば等差）、一つ前の点の解に、両端の状態の解の差の分を足した所から解く。
//! 目標のない状態では、解いた後の近軸量の値を間の点の目標に使う。間の点の系は前の状態の系に変数の値を入れたもの
//! （状態どうしが動かすパラメータのほかにも違うときは警告を返す）。
//!
//! 状態を持たないので、一括実行のタスクの中からも呼べる。

use crate::error::Result;
use crate::invalid;
use crate::optim::{self, DefaultMerit, DlsOpts, ParaxQ, Problem, Stop, Term, TermKind, Var};
use crate::param::Param;
use crate::sweep::RmsRef;
use crate::system::System;

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
    /// 像の項
    pub merit: DefaultMerit,
    pub rings: usize,
    pub reference: RmsRef,
    /// 隣り合う状態の間にとる点の数
    pub steps: usize,
    pub opts: DlsOpts,
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
            merit: DefaultMerit::Spot,
            rings: 3,
            reference: RmsRef::Centroid,
            steps: 0,
            opts: DlsOpts::default(),
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
    pub stop: Stop,
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
                target: t,
            },
        });
    }
    Ok((full, img))
}

/// 系 s（変数の初期値は s の値）で 1 点を解く
fn solve_point(
    s: &System,
    spec: &ZoomSpec,
    target: Option<f64>,
    state: Option<usize>,
    pos: f64,
) -> Result<(ZoomPoint, System)> {
    // 近軸量の項の重みを、目標に届くまで 100 倍ずつ上げて解き直す（重みを初めから大きくすると条件が悪く、
    // 反復が遅い）
    let mut w = spec.target_weight;
    let mut cur = s.clone();
    let mut iterations = 0;
    let mut tries = 0;
    let (r, img) = loop {
        let (full, img) = problem(&cur, spec, target, w)?;
        let r = optim::dls(&full, &spec.opts)?;
        iterations += r.iterations;
        tries += 1;
        let sys = &r.systems[0];
        let ok = target.is_none_or(|t| {
            (spec.q.value(sys) - t).abs() <= spec.target_tol * t.abs().max(f64::MIN_POSITIVE)
        });
        if ok || tries >= 4 {
            break (r, img);
        }
        cur = sys.clone();
        w *= 100.0;
    };
    let sys = r.systems.into_iter().next().unwrap();
    let merit = img.merit(&r.x)?;
    Ok((
        ZoomPoint {
            state,
            pos,
            target,
            value: spec.q.value(&sys),
            x: r.x,
            merit,
            stop: r.stop,
            iterations,
        },
        sys,
    ))
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
        invalid!("状態がない");
    }
    if spec.moving.is_empty() {
        invalid!("動かすパラメータがない");
    }
    if spec.targets.len() != n {
        invalid!("目標の数 {}（状態は {n}）", spec.targets.len());
    }
    if !spec.bounds.is_empty() && spec.bounds.len() != spec.moving.len() {
        invalid!(
            "範囲の数 {}（動かすパラメータは {}）",
            spec.bounds.len(),
            spec.moving.len()
        );
    }
    if !(spec.target_weight > 0.0) {
        invalid!("近軸量の項の重み {}", spec.target_weight);
    }
    let mut states = Vec::with_capacity(n);
    let mut systems = Vec::with_capacity(n);
    for (k, s) in configs.iter().enumerate() {
        crate::par::check_cancel()?;
        let (p, sys) = solve_point(s, spec, spec.targets[k], Some(k), k as f64)
            .map_err(|e| crate::error::Error::Invalid(format!("状態 {}: {e}", k + 1)))?;
        states.push(p);
        systems.push(sys);
    }
    let mut warnings = Vec::new();
    for p in &states {
        let k = p.state.unwrap();
        if let Some(t) = p.target {
            let rel = (p.value - t).abs() / t.abs().max(1e-300);
            if rel > spec.target_tol {
                warnings.push(format!(
                    "状態 {}: {} が目標に届かない（{} / 目標 {t}）",
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
        if (va - vb).abs() > 1e-9 * vb.abs().max(1.0) {
            warnings.push(format!(
                "状態 {} と {} は動かすパラメータのほかにも違う（間の点は状態 {} の系で求めた）",
                k + 1,
                k + 2,
                k + 1
            ));
        }
        let (ta, tb) = (
            states[k].target.unwrap_or(states[k].value),
            states[k + 1].target.unwrap_or(states[k + 1].value),
        );
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
            crate::par::check_cancel()?;
            let s0 = systems[k].with_params(&start)?;
            let pos = k as f64 + f;
            let (p, _) =
                solve_point(&s0, spec, Some(interp(ta, tb, f)), None, pos).map_err(|e| {
                    crate::error::Error::Invalid(format!(
                        "状態 {} と {} の間 {f:.3}: {e}",
                        k + 1,
                        k + 2
                    ))
                })?;
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
