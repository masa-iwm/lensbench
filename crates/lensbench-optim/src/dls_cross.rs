//! 構成をまたぐ項: 1 つの値の項の、すべての構成の値の最大 − 最小（[`TermKind::Spread`]）と、2 つの構成の値の差
//! （[`TermKind::Diff`]）

use super::*;

/// 構成をまたぐ項なら、すべての構成の系 sys での残差（そうでなければ None）
pub(super) fn value(sys: &[System], k: &TermKind, keep: bool) -> Option<Result<Vec<f64>>> {
    match k {
        TermKind::Spread { term, target } => {
            Some(spread(sys, term, keep).map(|v| vec![v - target]))
        }
        TermKind::Diff { a, b, term, target } => {
            Some(diff(sys, *a, *b, term, keep).map(|v| vec![v - target]))
        }
        _ => None,
    }
}

/// 項 term（1 つの値の項）の、構成ごとの値の最大 − 最小
fn spread(sys: &[System], term: &TermKind, keep: bool) -> Result<f64> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in sys {
        let r = eval_term(s, term, None, None, keep)?.0;
        // 引数の項の値（目標は 0 で作る）
        let v = r[0];
        lo = lo.min(v);
        hi = hi.max(v);
    }
    Ok(hi - lo)
}

/// 項 term（1 つの値の項）の、構成 a の値 − 構成 b の値
fn diff(sys: &[System], a: usize, b: usize, term: &TermKind, keep: bool) -> Result<f64> {
    let (Some(sa), Some(sb)) = (sys.get(a), sys.get(b)) else {
        invalid!(
            "difference term: configurations {} and {} (of {})",
            a + 1,
            b + 1,
            sys.len()
        );
    };
    let va = eval_term(sa, term, None, None, keep)?.0[0];
    let vb = eval_term(sb, term, None, None, keep)?.0[0];
    Ok(va - vb)
}

/// 変数 q がどれかの構成に効くか
pub(super) fn moves(p: &Problem, q: usize) -> bool {
    (0..p.configs.len()).any(|c| p.vars[q].acts(c))
}

/// 構成をまたぐ項 t の、変数 fd の差分の列（変数ごとにすべての構成の +h・−h の系を作る。片側しか作れなければ片側差分）
pub(super) fn columns(
    p: &Problem,
    x: &[f64],
    steps: &[f64],
    t: &Term,
    fd: &[usize],
) -> Result<Vec<(usize, Vec<f64>)>> {
    let keep = p.stopped_as_vignetted;
    let at = |xs: &[f64]| -> Result<Vec<f64>> {
        let sys = p.systems(xs)?;
        value(&sys, &t.kind, keep).expect("a term across configurations")
    };
    let r0 = at(x)?;
    let mut out = Vec::with_capacity(fd.len());
    for &q in fd {
        let h = steps[q];
        let shifted = |d: f64| -> Option<Result<Vec<f64>>> {
            let v = x[q] + d;
            if v > p.vars[q].hi || v < p.vars[q].lo {
                return None;
            }
            let mut xs = x.to_vec();
            xs[q] = v;
            Some(at(&xs))
        };
        let d = match (shifted(h), shifted(-h)) {
            (Some(Ok(a)), Some(Ok(b))) => vec![(a[0] - b[0]) / (2.0 * h)],
            (Some(Ok(a)), _) => vec![(a[0] - r0[0]) / h],
            (_, Some(Ok(b))) => vec![(r0[0] - b[0]) / h],
            (Some(Err(e)), _) | (_, Some(Err(e))) => return Err(e),
            (None, None) => invalid!("no difference system (variable {})", q + 1),
        };
        out.push((q, d));
    }
    Ok(out)
}
