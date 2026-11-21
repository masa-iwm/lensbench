//! ヤコビアンの差分の列（変数ごとに ±h の系を作り、(変数, 項) の組ごとに項を評価する）

use super::{eval_term, noisy_fd, FdSystems, JacMode, Problem, TermKind};
use lensbench_core::error::Result;
use lensbench_core::par;
use lensbench_core::system::System;

/// 1 つの変数の差分の列: (項, 列) の組
pub(super) type Columns = Vec<(usize, Vec<f64>)>;

/// 差分で求める列: fd_vars の k 番目の変数について (項, 列) の組。fds は項ごとの差分で求める変数、
/// r0s は項ごとの元の系の残差（重みを掛ける前、片側差分で使う）。1 段目で変数ごとに ±h の系を作り、
/// 2 段目で (変数, 項) の組ごとに評価する（組は互いに独立なので組ごとに並列に。変数ごとにまとめるより
/// 粒が細かく、スレッドに均等に配れる）。値はスレッド数によらず同じ
pub(super) fn columns(
    p: &Problem,
    x: &[f64],
    sys: &[System],
    steps: &[f64],
    fds: &[&[usize]],
    r0s: &[&[f64]],
    fd_vars: &[usize],
) -> Result<Vec<Columns>> {
    let forward = p.jac == JacMode::Forward;
    // 変数 q・項 ti の差分で使う刻みと、刻みの種類（射出瞳の揺れに負ける項は大きく取る）
    let fd_of = |q: usize, ti: usize| -> Option<(f64, bool)> {
        let t = &p.terms[ti];
        if !fds[ti].contains(&q) || matches!(t.kind, TermKind::User(_) | TermKind::Spread { .. }) {
            return None;
        }
        let noisy = noisy_fd(&sys[t.config], &t.kind, p.vars[q].param.kind);
        let h = steps[q]
            * if noisy {
                p.fd_rel_noisy / p.fd_rel
            } else {
                1.0
            };
        Some((h, noisy))
    };
    // 1 段目: 変数ごとに、(構成, 刻みの種類) ごとの +h と -h の系（前進差分では -h はなし）を作る
    let slots: Vec<Vec<[Option<FdSystems>; 2]>> = par::try_map(fd_vars, |&q| {
        let mut cache: Vec<[Option<FdSystems>; 2]> = vec![[None, None]; p.configs.len()];
        for (ti, t) in p.terms.iter().enumerate() {
            let Some((h, noisy)) = fd_of(q, ti) else {
                continue;
            };
            let slot = &mut cache[t.config][noisy as usize];
            if slot.is_some() {
                continue;
            }
            let (lo, hi) = (p.vars[q].lo, p.vars[q].hi);
            // 境界の外へは出さない（厚さの下限 0 で x − h が負になるなど）。出る側・系が作れない側は片側差分に
            let up_ok = x[q] + h <= hi;
            let dn_ok = x[q] - h >= lo;
            let mk = |d: f64| {
                let mut xs = x.to_vec();
                xs[q] += d;
                p.system(t.config, &xs)
            };
            let sp = if up_ok { mk(h).ok() } else { None };
            // 刻みを大きく取る項（noisy）は前進差分だと誤差が大きいので中心差分のまま
            let want_m = !(forward && !noisy && sp.is_some());
            let sm = if want_m && dn_ok { mk(-h).ok() } else { None };
            if sp.is_none() && sm.is_none() {
                // どちらの側も作れない: 元の誤りを返す
                mk(if up_ok { h } else { -h })?;
                lensbench_core::invalid!("no difference system (variable {})", q + 1);
            }
            *slot = Some((sp, sm));
        }
        Ok(cache)
    })?;
    // 2 段目: (変数, 項) の組ごとに ±h の系で項を評価する（組は互いに独立なので組ごとに並列に。
    // 変数ごとにまとめるより粒が細かく、スレッドに均等に配れる）
    let jobs: Vec<(usize, usize, f64, bool)> = fd_vars
        .iter()
        .enumerate()
        .flat_map(|(k, &q)| {
            (0..p.terms.len())
                .filter_map(move |ti| fd_of(q, ti).map(|(h, noisy)| (k, ti, h, noisy)))
        })
        .collect();
    let ds: Vec<Vec<f64>> = par::try_map(&jobs, |&(k, ti, h, noisy)| {
        let t = &p.terms[ti];
        let (sp, sm) = slots[k][t.config][noisy as usize]
            .as_ref()
            .expect("filled in the first stage");
        let rp = match sp {
            Some(sp) => eval_term(sp, &t.kind, None, None, p.stopped_as_vignetted).map(|v| v.0),
            None => Err(lensbench_core::Error::Invalid(String::new())),
        };
        let rm = match sm {
            Some(sm) => eval_term(sm, &t.kind, None, None, p.stopped_as_vignetted).map(|v| v.0),
            None => Err(lensbench_core::Error::Invalid(String::new())),
        };
        let r0 = r0s[ti];
        Ok(match (rp, rm) {
            (Ok(rp), Ok(rm)) => rp
                .iter()
                .zip(&rm)
                .map(|(a, b)| (a - b) / (2.0 * h))
                .collect(),
            // 片側だけ: 元の系の残差（parts の r、重みを掛ける前）との差
            (Ok(rp), Err(_)) => rp.iter().zip(r0).map(|(a, b)| (a - b) / h).collect(),
            (Err(_), Ok(rm)) => r0.iter().zip(&rm).map(|(a, b)| (a - b) / h).collect(),
            (Err(e), Err(e2)) => {
                return Err(if sp.is_some() { e } else { e2 });
            }
        })
    })?;
    let mut cols: Vec<Columns> = vec![Vec::new(); fd_vars.len()];
    for (&(k, ti, _, _), d) in jobs.iter().zip(ds) {
        cols[k].push((ti, d));
    }
    Ok(cols)
}
