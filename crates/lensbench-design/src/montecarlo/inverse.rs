//! 逆感度（公差の範囲を評価の上限まで締める）と、ガラスの nd・vd のずれ

use super::*;

/// ガラスの d 線の屈折率かアッベ数のずれ（OpticStudio の TIND・TABB）。面 `surf` の
/// [`Surface::dnd`](lensbench_core::surface::Surface::dnd)・`dvd` にずれを足し、媒質の屈折率はモデルガラスの式
/// （nd・vd・ΔPgF）の差で波長ごとに変える（OpticStudio と同じ解釈）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassShift {
    /// 面の番号
    pub surf: usize,
    /// アッベ数のずれ（false は屈折率のずれ）
    pub abbe: bool,
}

/// 逆感度の上限
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InverseTarget {
    /// 評価関数の値の上限（公称より大きいこと）
    Limit(f64),
    /// 公称（補償した後）からの評価関数の変化の上限（正）
    Increment(f64),
}

/// 逆感度の設定
#[derive(Debug, Clone, PartialEq)]
pub struct Inverse {
    pub target: InverseTarget,
    /// 締めない公差（[`McSpec::tols`] の番号。OpticStudio の Do Not Adjust During Inverse Tolerancing）
    pub skip: Vec<usize>,
    /// 解きの収まり: 評価関数と上限の差が上限の値 × rtol 以下（既定 1e-8）
    pub rtol: f64,
}

impl Inverse {
    pub fn new(target: InverseTarget) -> Inverse {
        Inverse {
            target,
            skip: Vec::new(),
            rtol: 1e-8,
        }
    }

    /// 公称の評価関数 nominal に対する評価関数の上限
    pub fn limit(&self, nominal: f64) -> f64 {
        match self.target {
            InverseTarget::Limit(x) => x,
            InverseTarget::Increment(d) => nominal + d,
        }
    }
}

/// 逆感度で求めた公差 1 つの範囲
#[derive(Debug, Clone, PartialEq)]
pub struct InverseTol {
    pub tol: usize,
    /// 元の範囲
    pub lo0: f64,
    pub hi0: f64,
    /// 締めた後の範囲（締めなかった側は元のまま）
    pub lo: f64,
    pub hi: f64,
    /// 締めた側
    pub tightened: [bool; 2],
    /// 解きで評価した数（lo・hi の側の合計）
    pub evals: usize,
}

/// 逆感度: 感度を求め、上限を超える側の範囲を締めてから、締めた範囲で試行を回す。一括実行のファイルには書かない
pub(super) fn run_inverse(spec: &McSpec, o: &runs::RunOpts) -> Result<McResult> {
    let Some(inv) = &spec.inverse else {
        return run_to(spec, o);
    };
    if o.out.is_some() {
        invalid!("inverse sensitivity does not write a batch run file (results)");
    }
    let plain = |trials: usize, sensitivity: bool, tols: Vec<Tol>| McSpec {
        trials,
        sensitivity,
        tols,
        inverse: None,
        ..spec.clone()
    };
    let sens = run_to(&plain(0, true, spec.tols.clone()), o)?;
    let nominal = sens.nominal.clone();
    let limit = inv.limit(nominal.merit);
    if !(limit > nominal.merit) {
        invalid!(
            "inverse limit {limit} must be larger than the nominal merit {}",
            nominal.merit
        );
    }
    if sens.cancelled {
        return Ok(McResult {
            inverse: None,
            ..sens
        });
    }
    // 締める側: 端の評価が上限を超える（追跡できない端も）側。(公差, hi か)
    let mut jobs: Vec<(usize, bool)> = Vec::new();
    for x in &sens.sensitivity {
        if inv.skip.contains(&x.tol) {
            continue;
        }
        let t = &spec.tols[x.tol];
        for (hi, e, v) in [(false, &x.lo, t.lo), (true, &x.hi, t.hi)] {
            let over = e.as_ref().is_none_or(|e| !(e.merit <= limit));
            if v != 0.0 && over {
                jobs.push((x.tol, hi));
            }
        }
    }
    let solved: Vec<Result<(f64, Eval, usize)>> = par::map(&jobs, |&(i, hi)| {
        let t = &spec.tols[i];
        let v0 = if hi { t.hi } else { t.lo };
        solve_side(spec, i, v0, nominal.merit, limit, inv.rtol)
    });
    let mut tols = spec.tols.clone();
    let mut out: Vec<InverseTol> = spec
        .tols
        .iter()
        .enumerate()
        .map(|(i, t)| InverseTol {
            tol: i,
            lo0: t.lo,
            hi0: t.hi,
            lo: t.lo,
            hi: t.hi,
            tightened: [false, false],
            evals: 0,
        })
        .collect();
    let mut sensitivity = sens.sensitivity.clone();
    for (&(i, hi), r) in jobs.iter().zip(solved) {
        let (v, e, n) = match r {
            Ok(x) => x,
            Err(lensbench_core::Error::Cancelled) => continue,
            Err(e) => return Err(e),
        };
        let x = &mut out[i];
        x.evals += n;
        x.tightened[hi as usize] = true;
        if hi {
            x.hi = v;
            tols[i].hi = v;
        } else {
            x.lo = v;
            tols[i].lo = v;
        }
        if let Some(sv) = sensitivity.iter_mut().find(|s| s.tol == i) {
            if hi {
                sv.hi = Some(e);
            } else {
                sv.lo = Some(e);
            }
        }
    }
    if par::cancelled() {
        return Ok(McResult {
            nominal,
            trials: Vec::new(),
            sensitivity,
            cancelled: true,
            inverse: Some(out),
        });
    }
    let mc = run_to(&plain(spec.trials, false, tols), o)?;
    Ok(McResult {
        nominal,
        trials: mc.trials,
        sensitivity,
        cancelled: mc.cancelled,
        inverse: Some(out),
    })
}

/// 公差 i だけを v にした評価関数（追跡できなければ +∞）と評価
fn merit_at(spec: &McSpec, i: usize, v: f64) -> (f64, Option<Eval>) {
    let mut d = vec![0.0; spec.tols.len()];
    d[i] = v;
    match evaluate(spec, &d) {
        Ok(e) if e.merit.is_finite() => (e.merit, Some(e)),
        _ => (f64::INFINITY, None),
    }
}

/// 公差 i の端 v0（評価関数が上限を超える）と 0（公称、上限より下）の間で、評価関数が上限 limit になる値を、挟み込みを
/// 保つ割線法（Illinois 法、割線が使えなければ二分）で求める。（値, その評価, 評価の数）
fn solve_side(
    spec: &McSpec,
    i: usize,
    v0: f64,
    nominal: f64,
    limit: f64,
    rtol: f64,
) -> Result<(f64, Eval, usize)> {
    let tol = rtol * limit.abs().max(f64::MIN_POSITIVE);
    let (mut a, mut fa) = (0.0, nominal - limit);
    let (mut b, mut fb) = (v0, merit_at(spec, i, v0).0 - limit);
    let mut n = 1;
    let mut side = 0i8;
    let mut best: Option<(f64, Eval)> = None;
    for _ in 0..100 {
        par::check_cancel()?;
        let mut c = if fb.is_finite() && fa.is_finite() && fb != fa {
            b - fb * (b - a) / (fb - fa)
        } else {
            0.5 * (a + b)
        };
        // 端に寄りすぎた点は二分に替える
        let w = b - a;
        if !c.is_finite() || ((c - a) / w) < 1e-3 || ((b - c) / w) < 1e-3 {
            c = 0.5 * (a + b);
        }
        let (fc, e) = merit_at(spec, i, c);
        n += 1;
        let fc = fc - limit;
        if fc.abs() <= tol {
            if let Some(e) = e {
                return Ok((c, e, n));
            }
        }
        if fc < 0.0 {
            // 上限より下の側（公称の側）を c に。上限を超えない値の最後のものとして持つ
            if let Some(e) = e {
                best = Some((c, e));
            }
            a = c;
            fa = fc;
            if side == -1 {
                fb *= 0.5;
            }
            side = -1;
        } else {
            b = c;
            fb = fc;
            if side == 1 {
                fa *= 0.5;
            }
            side = 1;
        }
        if (b - a).abs() <= 1e-14 * v0.abs() {
            break;
        }
    }
    // 収まらなければ上限を超えない側の最後の値（公称まで締める）
    match best {
        Some((c, e)) => Ok((c, e, n)),
        None => {
            let (_, e) = merit_at(spec, i, 0.0);
            let e = e.ok_or("inverse sensitivity: the nominal system cannot be evaluated")?;
            Ok((0.0, e, n + 1))
        }
    }
}

// ------------------------------------------------------------------------------------------ 簡易設定
