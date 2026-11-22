//! 大域の探索: 変数の範囲の中で始点を振って減衰最小二乗（[`dls`]）を繰り返し、評価関数の良い解を残す
//! （OpticStudio の Global Search・Hammer の始点を変えて回す部分に当たる）。
//!
//! 始点 0 は今の値（範囲に切り詰めたもの）。始点 1 以降は、両側に範囲のある変数は範囲の中で一様に、範囲の無い側の
//! ある変数は今の値 ± [`GlobalOpts::spread`] × 目安の大きさ（max(|今の値|, 種類ごとの目安)）の中で一様に振り、範囲で
//! 切り詰める。乱数は (種, 始点の番号) で決まる列（SplitMix64）なので、並列の数・終わる順によらず同じ結果になる。
//!
//! 始点ごとの局所の最適化は並列に回す（[`lensbench_core::par::map`]）。始点が終わるたびに、それまでの最良の評価関数の値を
//! [`lensbench_core::par::Event::Iter`] で知らせる（`iter` は終えた始点の数）。中断の印が倒れたら新しい始点を始めず、
//! 走っている始点はその反復の終わりで止め、そこまでの最良の解を返す（[`GlobalResult::cancelled`]）。
//! 追跡できない始点（光線が通らない・面がぶつかるなど）は数えて捨てる（[`GlobalResult::failed`]）。

use crate::{dls_inner, DlsOpts, DlsResult, Problem, Stop};
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::par;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// 大域の探索の設定
#[derive(Debug, Clone, Copy)]
pub struct GlobalOpts {
    /// 始点の数（今の値の始点を含む。1 なら局所の最適化だけ）
    pub starts: usize,
    /// 残す解の数（評価関数の良い順、同じ解は 1 つに）
    pub keep: usize,
    /// 乱数の種
    pub seed: u64,
    /// 範囲の無い変数を振る幅（目安の大きさに対する比）
    pub spread: f64,
    /// 始点ごとの局所の最適化
    pub local: DlsOpts,
}

impl Default for GlobalOpts {
    fn default() -> Self {
        GlobalOpts {
            starts: 16,
            keep: 5,
            seed: 1,
            spread: 0.5,
            local: DlsOpts::default(),
        }
    }
}

impl GlobalOpts {
    /// 設定の確かめ
    pub fn check(&self) -> Result<()> {
        if self.starts == 0 {
            invalid!("starts must be 1 or more");
        }
        if self.keep == 0 {
            invalid!("keep must be 1 or more");
        }
        if !(self.spread > 0.0 && self.spread.is_finite()) {
            invalid!("spread must be a positive number ({})", self.spread);
        }
        self.local.check()
    }
}

/// 残した解
#[derive(Debug, Clone)]
pub struct Solution {
    /// 始点の番号（0 は今の値）
    pub start: usize,
    /// 始点の変数の値
    pub x_start: Vec<f64>,
    /// 局所の最適化の後の変数の値
    pub x: Vec<f64>,
    pub merit: f64,
    pub stop: Stop,
    pub iterations: usize,
}

/// 大域の探索の結果
#[derive(Debug, Clone)]
pub struct GlobalResult {
    /// 最良の解の局所の最適化の結果（x0・merit0 は今の値の始点のもの、history は最良の解の始点からのもの）
    pub best: DlsResult,
    /// 最良の解の始点の番号
    pub best_start: usize,
    /// 残した解（評価関数の良い順。先頭が best）
    pub solutions: Vec<Solution>,
    /// 回した始点の数（中断で始めなかったものを除く）
    pub ran: usize,
    /// 追跡できずに捨てた始点の数
    pub failed: usize,
    /// 中断で始めなかった始点の数（ran・failed のどちらにも数えない）
    pub skipped: usize,
    /// 中断で止めた
    pub cancelled: bool,
}

/// (種, 番号) で決まる乱数列（SplitMix64）
struct Rng(u64);

impl Rng {
    fn new(seed: u64, i: u64) -> Rng {
        let mut r = Rng(seed ^ 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i.wrapping_add(1)));
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// [0, 1) の一様乱数
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 始点 i の変数の値（i = 0 は今の値 x0 を範囲に切り詰めたもの）
pub fn start_point(p: &Problem, x0: &[f64], o: &GlobalOpts, i: usize) -> Vec<f64> {
    let clamp = |v: f64, var: &crate::Var| v.clamp(var.lo, var.hi);
    if i == 0 {
        return x0
            .iter()
            .zip(&p.vars)
            .map(|(&v, var)| clamp(v, var))
            .collect();
    }
    let mut r = Rng::new(o.seed, i as u64);
    x0.iter()
        .zip(&p.vars)
        .map(|(&v, var)| {
            let u = r.uniform();
            if var.lo.is_finite() && var.hi.is_finite() {
                var.lo + u * (var.hi - var.lo)
            } else {
                let w = o.spread * v.abs().max(crate::dls::typical_of(var.param.kind));
                clamp(v + (2.0 * u - 1.0) * w, var)
            }
        })
        .collect()
}

/// 2 つの解が同じか（変数の差がどれも目安の大きさ max(|値|, 種類ごとの目安) の 1e-4 倍以下。局所の最適化の止まり方で
/// 末尾の桁が違う同じ谷の底を 1 つにする）
fn same(p: &Problem, a: &Solution, b: &Solution) -> bool {
    a.x.iter().zip(&b.x).zip(&p.vars).all(|((u, v), var)| {
        (u - v).abs()
            <= 1e-4
                * u.abs()
                    .max(v.abs())
                    .max(crate::dls::typical_of(var.param.kind))
    })
}

/// 始点の番号・始点の変数の値・局所の最適化の結果（中断で始めなかった始点は None）
type StartRun = (usize, Vec<f64>, Result<DlsResult>);

/// 大域の探索。説明はモジュールの頭
pub fn global_search(p: &Problem, o: &GlobalOpts) -> Result<GlobalResult> {
    p.check()?;
    o.check()?;
    let x0 = p.x0()?;
    let idx: Vec<usize> = (0..o.starts).collect();
    let done = AtomicUsize::new(0);
    // 終えた始点のうちの最良（知らせる値）
    let best_now: Mutex<Option<(f64, Vec<f64>)>> = Mutex::new(None);
    let reporting = par::reporting();
    let runs: Vec<Option<StartRun>> = par::map(&idx, |&i| {
        if par::cancelled() {
            return None;
        }
        let xs = start_point(p, &x0, o, i);
        let r = start_problem(p, &xs).and_then(|q| dls_inner(&q, &o.local));
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if reporting {
            let mut b = best_now.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(d) = &r {
                if d.merit.is_finite() && b.as_ref().is_none_or(|(m, _)| d.merit < *m) {
                    *b = Some((d.merit, d.x.clone()));
                }
            }
            if let Some((m, x)) = b.as_ref() {
                par::report(&par::Event::Iter(par::Iter {
                    iter: n,
                    merit: *m,
                    x: x.clone(),
                    damping: 0.0,
                    evals: 0,
                }));
            }
        }
        Some((i, xs, r))
    });
    let skipped = runs.iter().filter(|r| r.is_none()).count();
    let cancelled = par::cancelled() || skipped > 0;
    let mut ran = 0;
    let mut failed = 0;
    let mut ok: Vec<(usize, Vec<f64>, DlsResult)> = Vec::new();
    let mut first_err = None;
    for (i, xs, r) in runs.into_iter().flatten() {
        ran += 1;
        match r {
            Ok(d) if d.merit.is_finite() => ok.push((i, xs, d)),
            Ok(_) => failed += 1,
            Err(e) => {
                failed += 1;
                if i == 0 || first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if ok.is_empty() {
        return Err(first_err.unwrap_or(lensbench_core::Error::Cancelled));
    }
    // 評価関数の良い順（同じ値なら始点の番号の順）
    ok.sort_by(|a, b| a.2.merit.total_cmp(&b.2.merit).then(a.0.cmp(&b.0)));
    let mut solutions: Vec<Solution> = Vec::new();
    for (i, xs, d) in &ok {
        let s = Solution {
            start: *i,
            x_start: xs.clone(),
            x: d.x.clone(),
            merit: d.merit,
            stop: d.stop,
            iterations: d.iterations,
        };
        if solutions.iter().any(|t| same(p, t, &s)) {
            continue;
        }
        solutions.push(s);
        if solutions.len() >= o.keep {
            break;
        }
    }
    let (best_start, _, mut best) = ok.swap_remove(0);
    // 今の値からの比べ: x0・merit0 は今の値の始点のもの
    best.x0 = x0.clone();
    best.merit0 = p.merit(&start_point(p, &x0, o, 0)).unwrap_or(f64::NAN);
    if cancelled {
        best.stop = Stop::Cancelled;
    }
    Ok(GlobalResult {
        best,
        best_start,
        solutions,
        ran,
        failed,
        skipped,
        cancelled,
    })
}

/// 始点 xs から始める問題（構成の変数の値を xs にしたもの）
fn start_problem(p: &Problem, xs: &[f64]) -> Result<Problem> {
    Ok(p.with_configs(p.systems(xs)?))
}
