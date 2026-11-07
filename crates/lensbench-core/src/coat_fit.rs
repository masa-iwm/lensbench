//! 膜のモデル（[`CoatModel`]、LBCM 2）を、モデルガラスの媒質の組で測った係数（[`Measured`]、LBCT）から作る。
//!
//! 測る組（どれも `膜|前|後`、モデルガラスは `"n,0.0"`）:
//! - 透過: 前が `AIR`・`1.85,0.0`・`2.05,0.0` など 2 種以上、後が屈折率の違うモデルガラス 2 種以上
//! - 鏡: `AIR|MIRROR` と、基準の `2.05,0.0|MIRROR`
//!
//! 手順:
//! 1. 波長・β = n1 sinθ・偏光ごとに、OpticStudio の位相 exp(ik D n1/cos1) を外した t から
//!    y = 2 e1 / t' = e1 M00 + M10 + e1 e2 M01 + e2 M11（t' = t / sqrt(e2/e1)）を最小二乗で解く。
//!    D は当てはめの残差が最小になるように選ぶ（格子探索のあと黄金分割）。
//! 2. M = exp(iφ) M̃（φ = arg √det M、β の順に連続）と分け、ψ = φ / k を持つ。
//! 3. ψ を 1 点おきに抜いた β² のスプラインで予測し、k·誤差が `tol_psi` [rad] を超える最初の β の手前を BMAX にする。
//! 4. 鏡は、空気側と基準側の r から求めたアドミタンスが合うように鏡の位相の厚さ D_m を選び、
//!    基準側の r から位相 exp(2ik n cos D_m) を外して持つ。

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

use crate::coating::{CoatModel, Measured, MirrorModel, Spline, C64};
use crate::{invalid, Result};

/// 透過側の (n cos)² がこれより小さい点（すれすれ）は使わない
pub const GRAZE: f64 = 1e-3;
/// 鏡の基準の媒質
pub const MIRROR_REF: &str = "2.05,0.0";

/// 当てはめの設定
#[derive(Debug, Clone, Copy)]
pub struct FitOptions {
    /// D を探す範囲の上限 [µm]
    pub d_max: f64,
    /// BMAX を決める ψ の補間誤差の上限 [rad]
    pub tol_psi: f64,
}

impl Default for FitOptions {
    fn default() -> Self {
        FitOptions {
            d_max: 20.0,
            tol_psi: 1e-3,
        }
    }
}

/// 当てはめの結果（モデルと、確かめるための量）
#[derive(Debug, Clone)]
pub struct FitReport {
    pub model: CoatModel,
    /// 特性行列の当てはめの相対残差の最大（媒質が 3 つ以上ある点）
    pub residual_max: f64,
    /// 当てはめた点の数（波長 × β × 偏光）
    pub points: usize,
    /// 測った波長の間隔（波数 1/λ [1/µm]）の最大
    pub wave_gap: f64,
    /// 係数の差を 1e-5 にするのに要る波数の間隔の目安（[`wave_step`]）
    pub wave_gap_needed: f64,
}

/// 波長方向の補間誤差を `tol` にするのに要る波数 1/λ の間隔 [1/µm] の目安。膜の位相の厚さ `d` [µm]
/// （透過の D と鏡の D_m の大きい方）に反比例する（誤差は間隔の 4 乗に比例。多層膜の測定から決めた係数）
pub fn wave_step(d: f64, tol: f64) -> f64 {
    const C: f64 = 0.0352 / 0.2618; // 4.7e-3^(1/4) = 0.2618
    C * tol.powf(0.25) / d.max(0.05)
}

fn model_n(m: &str) -> Option<f64> {
    if m == "AIR" {
        return Some(1.0);
    }
    let (n, v) = m.split_once(',')?;
    let v: f64 = v.parse().ok()?;
    if v != 0.0 {
        return None;
    }
    n.parse().ok()
}

fn eta(n: f64, b: f64, pol: usize) -> C64 {
    let q = C64::new(n * n - b * b, 0.0).sqrt();
    if pol == 0 {
        q
    } else {
        C64::new(n * n, 0.0) / q
    }
}

/// β を表のキーにする（1e-9 で丸める）
fn bkey(b: f64) -> i64 {
    (b * 1e9).round() as i64
}

/// 波長を表のキーにする
fn wkey(w: f64) -> i64 {
    (w * 1e9).round() as i64
}

/// 複素の最小二乗 A x = y（列 4）。正規方程式をピボット付きの消去で解く
fn lstsq4(rows: &[([C64; 4], C64)]) -> Option<[C64; 4]> {
    let mut a = [[C64::new(0.0, 0.0); 5]; 4];
    for (r, y) in rows {
        for i in 0..4 {
            for j in 0..4 {
                a[i][j] += r[i].conj() * r[j];
            }
            a[i][4] += r[i].conj() * y;
        }
    }
    for c in 0..4 {
        let p = (c..4).max_by(|&i, &j| a[i][c].norm().total_cmp(&a[j][c].norm()))?;
        if a[p][c].norm() == 0.0 {
            return None;
        }
        a.swap(c, p);
        for r in 0..4 {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..5 {
                    let v = a[c][k];
                    a[r][k] -= f * v;
                }
            }
        }
    }
    Some([0, 1, 2, 3].map(|i| a[i][4] / a[i][i]))
}

/// 点ごとの M と相対残差
type Fits = BTreeMap<(i64, i64, usize), ([C64; 4], f64)>;

/// 透過の点: (波長, β, 偏光) → [(n1, 行, 右辺)]
type Rows = BTreeMap<(i64, i64, usize), Vec<(f64, [C64; 4], C64)>>;

fn transmit_rows(tabs: &[(&str, &Measured)], d: f64, only: Option<&BTreeSet<(i64, i64)>>) -> Rows {
    let mut rows: Rows = BTreeMap::new();
    for (key, m) in tabs {
        let mut it = key.split('|');
        let (_, m1, m2) = (it.next(), it.next().unwrap_or(""), it.next().unwrap_or(""));
        if m2 == "MIRROR" || m2 == "AIR" || key.contains('@') {
            continue;
        }
        let (Some(n1), Some(n2)) = (model_n(m1), model_n(m2)) else {
            continue;
        };
        for (iw, &wl) in m.wavelengths.iter().enumerate() {
            let k = 2.0 * PI / wl;
            for (ia, &a) in m.angles_deg.iter().enumerate() {
                let b = n1 * a.to_radians().sin();
                if n2 * n2 - b * b < GRAZE {
                    continue;
                }
                if only.is_some_and(|o| !o.contains(&(wkey(wl), bkey(b)))) {
                    continue;
                }
                let c1 = (1.0 - (b / n1) * (b / n1)).sqrt();
                for pol in 0..2 {
                    let t = m.data[iw][ia][pol];
                    if !(t.re.is_finite() && t.im.is_finite()) {
                        continue;
                    }
                    let (e1, e2) = (eta(n1, b, pol), eta(n2, b, pol));
                    let t = t * C64::new(0.0, -k * d * n1 / c1).exp();
                    let tp = t / (e2 / e1).sqrt();
                    rows.entry((wkey(wl), bkey(b), pol)).or_default().push((
                        n1,
                        [e1, C64::new(1.0, 0.0), e1 * e2, e2],
                        e1 * 2.0 / tp,
                    ));
                }
            }
        }
    }
    rows
}

/// 点ごとの M と相対残差（媒質が 3 つ以上のとき。2 つなら NaN）
fn solve_rows(rows: &Rows) -> Fits {
    let mut out = BTreeMap::new();
    for (k, r) in rows {
        let mut ns: Vec<f64> = r.iter().map(|x| x.0).collect();
        ns.sort_by(f64::total_cmp);
        ns.dedup();
        if ns.len() < 2 || r.len() < 4 {
            continue;
        }
        let eq: Vec<([C64; 4], C64)> = r.iter().map(|x| (x.1, x.2)).collect();
        let Some(m) = lstsq4(&eq) else { continue };
        let res = if r.len() > 4 {
            eq.iter()
                .map(|(a, y)| {
                    let p: C64 = (0..4).map(|i| a[i] * m[i]).sum();
                    (p - y).norm() / y.norm()
                })
                .fold(0.0, f64::max)
        } else {
            f64::NAN
        };
        out.insert(*k, (m, res));
    }
    out
}

/// 1 次元の黄金分割（[a, b] で f を最小に）
fn golden(f: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64, tol: f64) -> f64 {
    let g = (5f64.sqrt() - 1.0) / 2.0;
    let mut c = b - g * (b - a);
    let mut d = a + g * (b - a);
    let (mut fc, mut fd) = (f(c), f(d));
    while b - a > tol {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - g * (b - a);
            fc = f(c);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + g * (b - a);
            fd = f(d);
        }
    }
    (a + b) / 2.0
}

/// 格子探索のあと黄金分割
fn minimize(f: &dyn Fn(f64) -> f64, hi: f64, step: f64, tol: f64) -> f64 {
    let n = (hi / step).ceil() as usize;
    let (mut best, mut bv) = (0.0, f64::INFINITY);
    for i in 0..n {
        let x = i as f64 * step;
        let v = f(x);
        if v < bv {
            best = x;
            bv = v;
        }
    }
    golden(f, (best - 1.5 * step).max(0.0), best + 1.5 * step, tol)
}

/// 透過の D と M
fn fit_transmit(tabs: &[(&str, &Measured)], opt: &FitOptions) -> Result<(f64, Fits)> {
    let full = solve_rows(&transmit_rows(tabs, 0.0, None));
    let pts: BTreeSet<(i64, i64)> = full.keys().map(|k| (k.0, k.1)).collect();
    if pts.is_empty() {
        invalid!(
            "膜の当てはめ: 透過のモデルガラスの組が足りない（前の媒質 2 種以上・後のモデルガラス）"
        );
    }
    let step = (pts.len() / 150).max(1);
    let sub: BTreeSet<(i64, i64)> = pts.iter().step_by(step).copied().collect();
    let cost = |d: f64| -> f64 {
        solve_rows(&transmit_rows(tabs, d, Some(&sub)))
            .values()
            .map(|v| v.1)
            .filter(|r| r.is_finite())
            .map(|r| r * r)
            .sum()
    };
    let d = minimize(&cost, opt.d_max, 0.02, 1e-7);
    Ok((d, solve_rows(&transmit_rows(tabs, d, None))))
}

/// 鏡の位相の厚さ D_m
fn mirror_d(air: &Measured, refm: &Measured, opt: &FitOptions) -> f64 {
    let nr = model_n(MIRROR_REF).unwrap();
    let ba: BTreeMap<i64, usize> = air
        .angles_deg
        .iter()
        .enumerate()
        .map(|(i, a)| (bkey(a.to_radians().sin()), i))
        .collect();
    let mut pairs = Vec::new();
    for (jb, a) in refm.angles_deg.iter().enumerate() {
        let b = nr * a.to_radians().sin();
        let Some(&ia) = ba.get(&bkey(b)) else {
            continue;
        };
        if b >= 0.95 {
            continue;
        }
        for (iw, wl) in air.wavelengths.iter().enumerate() {
            if let Some(iwb) = refm.wavelengths.iter().position(|w| wkey(*w) == wkey(*wl)) {
                pairs.push((*wl, b, iw, ia, iwb, jb));
            }
        }
    }
    let step = (pairs.len() / 300).max(1);
    let pairs: Vec<_> = pairs.into_iter().step_by(step).collect();
    let one = C64::new(1.0, 0.0);
    let cost = |d: f64| -> f64 {
        let mut e = 0.0;
        for &(wl, b, iw, ia, iwb, jb) in &pairs {
            let k = 2.0 * PI / wl;
            let (ca, cb) = ((1.0 - b * b).sqrt(), (1.0 - (b / nr) * (b / nr)).sqrt());
            for pol in 0..2 {
                let ra = air.data[iw][ia][pol] * C64::new(0.0, -2.0 * k * ca * d).exp();
                let rb = refm.data[iwb][jb][pol] * C64::new(0.0, -2.0 * k * nr * cb * d).exp();
                let ya = eta(1.0, b, pol) * (one - ra) / (one + ra);
                let yb = eta(nr, b, pol) * (one - rb) / (one + rb);
                let v = (ya - yb).norm_sqr() / (ya.norm_sqr() + yb.norm_sqr());
                if v.is_finite() {
                    e += v;
                }
            }
        }
        e
    };
    minimize(&cost, opt.d_max, 0.01, 1e-8)
}

/// ψ の補間から BMAX
fn bmax_of(betas: &[f64], psi: &[f64], wl: f64, tol: f64) -> f64 {
    let k = 2.0 * PI / wl;
    let mut i = 1;
    while i + 1 < betas.len() {
        let ev: Vec<usize> = (0..=i + 1)
            .step_by(2)
            .filter(|&j| j < betas.len())
            .collect();
        if ev.len() >= 4 {
            let x: Vec<f64> = ev.iter().map(|&j| betas[j] * betas[j]).collect();
            let y: Vec<f64> = ev.iter().map(|&j| psi[j]).collect();
            if let Some(sp) = Spline::not_a_knot(&x, &y) {
                if k * (sp.eval(betas[i] * betas[i]) - psi[i]).abs() > tol {
                    return betas[i - 1];
                }
            }
        }
        i += 2;
    }
    betas[betas.len() - 1]
}

/// 膜 1 つの表（`膜|前|後` の KEY と実測）から膜のモデルを作る
pub fn fit_model(tabs: &[(&str, &Measured)], opt: &FitOptions) -> Result<FitReport> {
    let (d, mfit) = fit_transmit(tabs, opt)?;
    let residual_max = mfit
        .values()
        .map(|v| v.1)
        .filter(|r| r.is_finite())
        .fold(0.0, f64::max);
    let waves: BTreeSet<i64> = mfit.keys().map(|k| k.0).collect();
    // 全波長・両偏光にある β
    let mut betas: Option<BTreeSet<i64>> = None;
    for &w in &waves {
        let s: BTreeSet<i64> = (0..2)
            .map(|p| {
                mfit.keys()
                    .filter(|k| k.0 == w && k.2 == p)
                    .map(|k| k.1)
                    .collect::<BTreeSet<_>>()
            })
            .reduce(|a, b| &a & &b)
            .unwrap();
        betas = Some(match betas {
            None => s,
            Some(b) => &b & &s,
        });
    }
    let betas: Vec<i64> = betas.unwrap_or_default().into_iter().collect();
    if betas.len() < 4 {
        invalid!(
            "膜の当てはめ: 全波長で共通の β が足りない（{} 点）",
            betas.len()
        );
    }
    let bf: Vec<f64> = betas.iter().map(|&b| b as f64 * 1e-9).collect();
    let mut model = CoatModel {
        d,
        bmax: f64::INFINITY,
        ..Default::default()
    };
    for &w in &waves {
        let wl = w as f64 * 1e-9;
        let mut rows = vec![[[C64::new(0.0, 0.0); 4]; 2]; betas.len()];
        let mut psis = vec![[0.0; 2]; betas.len()];
        for pol in 0..2 {
            let mut prev = 0.0;
            for (j, &b) in betas.iter().enumerate() {
                let m = mfit[&(w, b, pol)].0;
                let det = m[0] * m[3] - m[1] * m[2];
                // arg det を β の順に連続に（unwrap）
                let mut a = det.arg();
                if j > 0 {
                    a += 2.0 * PI * ((prev - a) / (2.0 * PI)).round();
                }
                prev = a;
                let phi = a / 2.0;
                let g = C64::new(0.0, -phi).exp();
                rows[j][pol] = m.map(|z| z * g);
                psis[j][pol] = phi * wl / (2.0 * PI);
            }
            let ps: Vec<f64> = psis.iter().map(|p| p[pol]).collect();
            model.bmax = model.bmax.min(bmax_of(&bf, &ps, wl, opt.tol_psi));
        }
        model.wavelengths.push(wl);
        model.m.push(rows);
        model.psi.push(psis);
    }
    model.betas = bf;
    // 鏡
    let find = |m1: &str| {
        tabs.iter().find(|(k, _)| {
            let mut it = k.split('|');
            let _ = it.next();
            it.next() == Some(m1) && it.next() == Some("MIRROR") && !k.contains('@')
        })
    };
    if let (Some((_, air)), Some((_, refm))) = (find("AIR"), find(MIRROR_REF)) {
        let dm = mirror_d(air, refm, opt);
        let nr = model_n(MIRROR_REF).unwrap();
        let bs: Vec<f64> = refm
            .angles_deg
            .iter()
            .map(|a| nr * a.to_radians().sin())
            .collect();
        let mut r = Vec::new();
        for (iw, &wl) in refm.wavelengths.iter().enumerate() {
            let k = 2.0 * PI / wl;
            r.push(
                bs.iter()
                    .enumerate()
                    .map(|(ia, &b)| {
                        let c = (1.0 - (b / nr) * (b / nr)).sqrt();
                        let ph = C64::new(0.0, -2.0 * k * nr * c * dm).exp();
                        [refm.data[iw][ia][0] * ph, refm.data[iw][ia][1] * ph]
                    })
                    .collect(),
            );
        }
        model.mirror = Some(MirrorModel {
            n_ref: nr,
            d: dm,
            wavelengths: refm.wavelengths.clone(),
            betas: bs,
            r,
        });
    }
    let points = mfit.len();
    let sig: Vec<f64> = waves.iter().map(|&w| 1.0 / (w as f64 * 1e-9)).collect();
    let wave_gap = sig
        .windows(2)
        .map(|p| (p[0] - p[1]).abs())
        .fold(0.0, f64::max);
    let dm = model.mirror.as_ref().map_or(0.0, |m| m.d);
    let wave_gap_needed = wave_step(model.d.max(dm), 1e-5);
    Ok(FitReport {
        model,
        residual_max,
        points,
        wave_gap,
        wave_gap_needed,
    })
}

/// 膜のモデルをファイルの形式（[`crate::coating::parse_models`]）にする
pub fn format_models(entries: &[(&str, &CoatModel)]) -> String {
    use std::fmt::Write;
    let mut s = format!("{}\n", crate::coating::MODEL_HEADER);
    for (name, m) in entries {
        writeln!(s, "COAT {name}").unwrap();
        if !m.wavelengths.is_empty() {
            writeln!(s, "D {:?}\nBMAX {:?}", m.d, m.bmax).unwrap();
            for (iw, w) in m.wavelengths.iter().enumerate() {
                writeln!(s, "WAVE {w:?}").unwrap();
                for (j, b) in m.betas.iter().enumerate() {
                    write!(s, "{b:?}").unwrap();
                    for pol in 0..2 {
                        for z in m.m[iw][j][pol] {
                            write!(s, " {:?} {:?}", z.re, z.im).unwrap();
                        }
                        write!(s, " {:?}", m.psi[iw][j][pol]).unwrap();
                    }
                    s.push('\n');
                }
            }
        }
        if let Some(r) = &m.mirror {
            writeln!(s, "MIRROR {:?} {:?}", r.n_ref, r.d).unwrap();
            for (iw, w) in r.wavelengths.iter().enumerate() {
                writeln!(s, "WAVE {w:?}").unwrap();
                for (j, b) in r.betas.iter().enumerate() {
                    let v = r.r[iw][j];
                    writeln!(
                        s,
                        "{b:?} {:?} {:?} {:?} {:?}",
                        v[0].re, v[0].im, v[1].re, v[1].im
                    )
                    .unwrap();
                }
            }
        }
    }
    s
}
