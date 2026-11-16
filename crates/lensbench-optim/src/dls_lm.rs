//! 減衰最小二乗（Levenberg–Marquardt）の反復（[`dls`]）
use super::*;

// ------------------------------------------------------------------------------------------ DLS

/// DLS の設定
#[derive(Debug, Clone, Copy)]
pub struct DlsOpts {
    /// 反復の上限
    pub max_iter: usize,
    /// 1 回の反復での評価関数（Σ r²）の相対的な減りがこれ未満で止める
    pub ftol: f64,
    /// 変数の変化（目安の大きさで割った値）の最大がこれ未満で止める
    pub xtol: f64,
    /// 減衰の初期値（JᵀJ の対角に対する比）
    pub damping: f64,
    /// 勾配の判定: 残差と変数ごとの列のなす角の余弦の最大 max_q |J_qᵀ r| / (‖J_q‖ ‖r‖)（変数の大きさによらない）が
    /// これ未満で止める（[`Stop::SmallGradient`]）。0 は判定しない（既定）
    pub gtol: f64,
    /// 一歩を、列の大きさで無次元にした [J D⁻¹; √μ I] の QR で解く（正規方程式 JᵀJ を作らないので条件数を 2 乗に
    /// しない）。false（既定）は正規方程式 (JᵀJ + μ diag(JᵀJ)) δ = −Jᵀr を Cholesky で解く。数学的には同じ一歩だが、
    /// 条件の悪い問題では丸めの差で反復の道筋が変わる
    pub qr: bool,
    /// 境界: 一歩で範囲を出る変数を境界に固定して残りを解き直す（有効制約法）。false（既定）は成分ごとに範囲へ
    /// 切り詰める（切り詰めた一歩は降下方向の保証がない。受け入れの判定で守る）
    pub active_set: bool,
}

impl Default for DlsOpts {
    fn default() -> Self {
        DlsOpts {
            max_iter: 50,
            ftol: 1e-10,
            xtol: 1e-12,
            damping: 1e-3,
            gtol: 0.0,
            qr: false,
            active_set: false,
        }
    }
}

impl DlsOpts {
    /// 設定の確かめ（ftol・xtol・gtol は 0 以上、damping は正の有限の値）
    pub fn check(&self) -> Result<()> {
        for (k, v) in [
            ("ftol", self.ftol),
            ("xtol", self.xtol),
            ("gtol", self.gtol),
        ] {
            if !(v >= 0.0) {
                lensbench_core::invalid!("{k} must be 0 or more ({v})");
            }
        }
        if !(self.damping > 0.0 && self.damping.is_finite()) {
            lensbench_core::invalid!("damping must be a positive number ({})", self.damping);
        }
        Ok(())
    }
}

/// 止まった理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// 評価関数がほとんど減らない
    Converged,
    /// 変数がほとんど動かない（減衰を強めずに求めた一歩が [`DlsOpts::xtol`] より小さい）
    SmallStep,
    /// 勾配がほとんど 0（[`DlsOpts::gtol`]）
    SmallGradient,
    /// どの減衰でも評価関数が減らない（局所解。減衰を強めて一歩が小さくなった場合も）
    NoProgress,
    /// 反復の上限
    MaxIter,
    /// 呼び出し側が止めた
    Cancelled,
    /// 変数がない
    NoVars,
    /// 途中でヤコビアンが求まらなかった（差分の系が追跡できないなど。理由は [`DlsResult::note`]）。それまでの
    /// 最良の x・系を返す
    JacobianFailed,
}

impl Stop {
    /// 出力用の名前
    pub fn as_str(self) -> &'static str {
        match self {
            Stop::Converged => "converged",
            Stop::SmallStep => "small_step",
            Stop::SmallGradient => "small_gradient",
            Stop::NoProgress => "no_progress",
            Stop::MaxIter => "max_iter",
            Stop::Cancelled => "cancelled",
            Stop::NoVars => "no_vars",
            Stop::JacobianFailed => "jacobian_failed",
        }
    }
}

/// 反復ごとの進み具合（[`dls`] が文脈の受け手へ知らせる）
pub use lensbench_core::par::Iter as Progress;

/// DLS の結果
#[derive(Debug, Clone)]
pub struct DlsResult {
    pub x0: Vec<f64>,
    pub x: Vec<f64>,
    pub merit0: f64,
    pub merit: f64,
    /// 反復ごとの評価関数の値（初期値を含む）
    pub history: Vec<f64>,
    pub iterations: usize,
    pub evals: usize,
    pub stop: Stop,
    /// 止まった理由の補足（[`Stop::JacobianFailed`] の誤りの文など）
    pub note: Option<String>,
    /// 最後の反復で評価関数に効かないとみなして動かさなかった変数の番号（0 始まり。差分の雑音しかない列）
    pub inactive: Vec<usize>,
    /// 最適化した系（構成ごと）
    pub systems: Vec<System>,
}

/// 評価関数に効かない変数の判定: 目安の大きさだけ動かしたときの残差の変化 ‖J_q‖·目安 が残差の EFFECT_TOL 倍に
/// 満たない変数は動かさない
const EFFECT_TOL: f64 = 1e-9;

/// この比（[`EFFECT_TOL`] と同じ量）より小さい列は、差分の丸めの雑音だけの列かを [`fd_noise`] で確かめる。
/// 差分の刻みは目安の 1e-6 倍で、追跡の丸めによる雑音の列はこの比で 3e-8〜5e-6（A0 の空気の中の絞り面の曲率・
/// 円錐係数で実測、波面の評価関数）。比だけで切ると、目安の大きさが合わない小さいレンズの高次の非球面係数など
/// 本当に効く弱い変数まで外れるので、比は疑いの目安にだけ使う
const SUSPECT_TOL: f64 = 1e-3;

/// 変数 q の列 J_q が差分の雑音だけか: 刻みを 1000 倍にした中心差分の列と比べ、1/10 未満なら雑音（線形に効く
/// 変数なら刻みによらず同じ列になり、雑音なら刻みに反比例して小さくなる）。以前はこの雑音の列が自由な変数になり、
/// 減衰が効かずに大きく動いて（A0 の絞り面が R = −4.9 mm など）系に書かれた
fn fd_noise(p: &Problem, x: &[f64], r: &[f64], j: &[f64], q: usize, scale: f64) -> bool {
    let nv = x.len();
    let h = 1e3 * p.fd_rel * scale;
    let v = &p.vars[q];
    let at = |d: f64| -> Option<Vec<f64>> {
        let xv = x[q] + d;
        if xv < v.lo || xv > v.hi {
            return None;
        }
        let mut xs = x.to_vec();
        xs[q] = xv;
        p.residuals(&xs).ok().filter(|rr| rr.len() == r.len())
    };
    let big: Vec<f64> = match (at(h), at(-h)) {
        (Some(rp), Some(rm)) => rp
            .iter()
            .zip(&rm)
            .map(|(a, b)| (a - b) / (2.0 * h))
            .collect(),
        (Some(rp), None) => rp.iter().zip(r).map(|(a, b)| (a - b) / h).collect(),
        (None, Some(rm)) => r.iter().zip(&rm).map(|(a, b)| (a - b) / h).collect(),
        // 確かめられなければ今の列のまま
        (None, None) => return false,
    };
    let n_small: f64 = (0..r.len())
        .map(|i| j[i * nv + q].powi(2))
        .sum::<f64>()
        .sqrt();
    let n_big: f64 = big.iter().map(|v| v * v).sum::<f64>().sqrt();
    n_big < 0.1 * n_small
}

/// 減衰最小二乗で最適化する。反復が終わるたびに文脈の受け手（[`lensbench_core::par::with_progress`]）へ
/// [`lensbench_core::par::Event::Iter`] で知らせ、文脈の中断の印（[`lensbench_core::par::with_cancel`]）が倒れたら
/// その反復の終わり（ヤコビアンの途中なら、その前の反復の値）で止める（[`Stop::Cancelled`]）
pub fn dls(p: &Problem, o: &DlsOpts) -> Result<DlsResult> {
    dls_report(p, o)
}

/// [`dls`] を、より大きな計算の一部として回す（公差の補償子・差し替え探索・ズーム軌跡）: 反復の進み具合は知らせない
/// （外の計算が自分の進み具合を知らせる）。中断の印は見る
pub fn dls_inner(p: &Problem, o: &DlsOpts) -> Result<DlsResult> {
    par::with_progress(None, || dls_report(p, o))
}

fn dls_report(p: &Problem, o: &DlsOpts) -> Result<DlsResult> {
    p.check()?;
    o.check()?;
    let nv = p.vars.len();
    let x0 = p.x0()?;
    let mut x: Vec<f64> = x0
        .iter()
        .zip(&p.vars)
        .map(|(&v, var)| v.clamp(var.lo, var.hi))
        .collect();
    let mut evals = 0;
    if nv == 0 {
        let r = p.residuals(&x)?;
        let m = p.merit_of(&r);
        return Ok(DlsResult {
            x0: x0.clone(),
            x: x0,
            merit0: m,
            merit: m,
            history: vec![m],
            iterations: 0,
            evals: 1,
            stop: Stop::NoVars,
            note: None,
            inactive: Vec::new(),
            systems: p.final_systems(&x)?,
        });
    }
    let (mut r, mut j) = p.jacobian_checked(&x)?;
    let mut f = dot_n(&r, &r);
    let merit0 = p.merit_of(&r);
    let mut history = vec![merit0];
    let mut mu = o.damping;
    let mut nu = 2.0;
    let mut stop = Stop::MaxIter;
    let mut note = None;
    let mut inactive = Vec::new();
    let mut iterations = 0;
    let scale: Vec<f64> = p
        .vars
        .iter()
        .zip(&x0)
        .map(|(v, &xv)| xv.abs().max(typical(v.param.kind)))
        .collect();
    for it in 0..o.max_iter {
        iterations = it + 1;
        let m = r.len();
        // A = JᵀJ, g = Jᵀr
        let mut a = vec![0.0; nv * nv];
        let mut g = vec![0.0; nv];
        for i in 0..m {
            let row = &j[i * nv..(i + 1) * nv];
            for q in 0..nv {
                g[q] += row[q] * r[i];
                for s in 0..=q {
                    a[q * nv + s] += row[q] * row[s];
                }
            }
        }
        for q in 0..nv {
            for s in 0..q {
                a[s * nv + q] = a[q * nv + s];
            }
        }
        // 境界にあって外へ向かう変数と、評価関数にほとんど効かない変数（目安の大きさだけ動かしても残差の変化が
        // 残差の 1e-9 未満。差分の丸め誤差だけの列で大きく動かさないため）は動かさない
        let rn2 = f.sqrt();
        inactive = (0..nv)
            .filter(|&q| {
                let ratio = a[q * nv + q].sqrt() * scale[q] / rn2.max(f64::MIN_POSITIVE);
                !(ratio > EFFECT_TOL)
                    || (ratio < SUSPECT_TOL && fd_noise(p, &x, &r, &j, q, scale[q]))
            })
            .collect();
        evals += 2
            * (0..nv)
                .filter(|&q| {
                    let ratio = a[q * nv + q].sqrt() * scale[q] / rn2.max(f64::MIN_POSITIVE);
                    ratio > EFFECT_TOL && ratio < SUSPECT_TOL
                })
                .count();
        let effective = |q: usize| !inactive.contains(&q);
        let free: Vec<usize> = (0..nv)
            .filter(|&q| {
                let v = &p.vars[q];
                !((x[q] <= v.lo && g[q] > 0.0) || (x[q] >= v.hi && g[q] < 0.0)) && effective(q)
            })
            .collect();
        if free.is_empty() {
            stop = Stop::NoProgress;
            break;
        }
        if o.gtol > 0.0 {
            // 残差と列のなす角の余弦の最大（free の変数で）
            let gmax = free
                .iter()
                .map(|&q| g[q].abs() / (a[q * nv + q].sqrt() * rn2).max(f64::MIN_POSITIVE))
                .fold(0.0, f64::max);
            if gmax < o.gtol {
                stop = Stop::SmallGradient;
                break;
            }
        }
        let mut accepted = None;
        // この反復で減衰を強めたか（強めて一歩が小さくなったのは「進めない」で、「動かない」ではない）
        let mut raised = false;
        for _ in 0..30 {
            let Some(xn) = lm_point(p, o, &x, &r, &j, &a, &g, &free, mu) else {
                mu *= nu;
                nu *= 2.0;
                raised = true;
                continue;
            };
            let step: Vec<f64> = (0..nv).map(|q| xn[q] - x[q]).collect();
            if step.iter().zip(&scale).all(|(s, c)| s.abs() <= o.xtol * c) {
                stop = if raised {
                    Stop::NoProgress
                } else {
                    Stop::SmallStep
                };
                break;
            }
            evals += 1;
            let rn = match p.residuals(&xn) {
                Ok(rn) => rn,
                // 試しの一歩の評価の途中の中断は、進めないのでなく中断（その前の x を返す）
                Err(Error::Cancelled) => {
                    stop = Stop::Cancelled;
                    break;
                }
                Err(_) if par::cancelled() => {
                    stop = Stop::Cancelled;
                    break;
                }
                Err(_) => {
                    mu *= nu;
                    nu *= 2.0;
                    raised = true;
                    continue;
                }
            };
            let fn_ = dot_n(&rn, &rn);
            // 予測される減り: −(2 gᵀδ + δᵀAδ)
            let mut pred = 0.0;
            for q in 0..nv {
                let mut ad = 0.0;
                for s in 0..nv {
                    ad += a[q * nv + s] * step[s];
                }
                pred -= 2.0 * g[q] * step[q] + step[q] * ad;
            }
            if fn_.is_finite() && fn_ < f {
                let rho = if pred > 0.0 { (f - fn_) / pred } else { 1.0 };
                mu *= (1.0 - (2.0 * rho - 1.0).powi(3)).max(1.0 / 3.0);
                nu = 2.0;
                accepted = Some((xn, rn, fn_));
                break;
            }
            mu *= nu;
            nu *= 2.0;
            raised = true;
        }
        let Some((xn, rn, fn_)) = accepted else {
            if !matches!(stop, Stop::SmallStep | Stop::Cancelled) {
                stop = Stop::NoProgress;
            }
            break;
        };
        let rel = (f - fn_) / f.max(f64::MIN_POSITIVE);
        x = xn;
        f = fn_;
        r = rn;
        history.push(p.merit_of(&r));
        if par::reporting() {
            par::report(&par::Event::Iter(Progress {
                iter: iterations,
                merit: history.last().copied().unwrap_or(f64::NAN),
                x: x.clone(),
                damping: mu,
                evals,
            }));
        }
        if par::cancelled() {
            stop = Stop::Cancelled;
            break;
        }
        if rel < o.ftol {
            stop = Stop::Converged;
            break;
        }
        if it + 1 < o.max_iter {
            // 途中のヤコビアンの失敗は、それまでの改善を捨てずに止める
            match p.jacobian_checked(&x) {
                Ok(v) => (r, j) = v,
                Err(Error::Cancelled) => {
                    stop = Stop::Cancelled;
                    break;
                }
                Err(e) => {
                    stop = Stop::JacobianFailed;
                    note = Some(e.to_string());
                    break;
                }
            }
        }
    }
    Ok(DlsResult {
        x0,
        merit0,
        merit: p.merit_of(&r),
        history,
        iterations,
        evals,
        stop,
        note,
        inactive,
        systems: p.final_systems(&x)?,
        x,
    })
}

fn dot_n(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 減衰 mu の一歩の行き先: free の変数で (JᵀJ + mu diag(JᵀJ)) δ = −Jᵀr を解く（[`DlsOpts::qr`] なら、列の大きさ
/// D_q = ‖J_q‖ で割って無次元にした min ‖J D⁻¹ y + r‖² + mu ‖y‖²（δ = D⁻¹ y）を [J D⁻¹; √mu I] の QR で、でなければ
/// 正規方程式を Cholesky で）。範囲を出る変数は、[`DlsOpts::active_set`] なら境界に固定して残りを解き直し、
/// でなければ範囲に切り詰める。解けなければ None
#[allow(clippy::too_many_arguments)]
fn lm_point(
    p: &Problem,
    o: &DlsOpts,
    x: &[f64],
    r: &[f64],
    j: &[f64],
    a: &[f64],
    g: &[f64],
    free: &[usize],
    mu: f64,
) -> Option<Vec<f64>> {
    let nv = x.len();
    let m = r.len();
    let mut xn = x.to_vec();
    let mut cols: Vec<usize> = free.to_vec();
    // 境界に固定した変数の分の残差 r + Σ J_q (境界 − x_q)
    let mut rr = r.to_vec();
    loop {
        let nc = cols.len();
        if nc == 0 {
            return Some(xn);
        }
        let d = if o.qr {
            qr_step(j, &rr, nv, a, &cols, mu)?
        } else {
            // 固定した変数がなければ呼び出し側の Jᵀr、あれば固定した分を入れた残差で作り直す
            let gg: Vec<f64> = if cols.len() == free.len() {
                g.to_vec()
            } else {
                (0..nv)
                    .map(|q| (0..m).map(|i| j[i * nv + q] * rr[i]).sum())
                    .collect()
            };
            normal_step(a, &gg, nv, &cols, mu)?
        };
        let mut out = Vec::new();
        for (k, &q) in cols.iter().enumerate() {
            let v = x[q] + d[k];
            let (lo, hi) = (p.vars[q].lo, p.vars[q].hi);
            if o.active_set && (v < lo || v > hi) {
                out.push((q, v.clamp(lo, hi)));
            }
            xn[q] = v.clamp(lo, hi);
        }
        if out.is_empty() {
            return Some(xn);
        }
        // 範囲を出た変数を境界に固定して、残りを解き直す
        for &(q, bnd) in &out {
            xn[q] = bnd;
            let d = bnd - x[q];
            for i in 0..m {
                rr[i] += j[i * nv + q] * d;
            }
        }
        cols.retain(|q| !out.iter().any(|e| e.0 == *q));
        for &q in &cols {
            xn[q] = x[q];
        }
    }
}

/// 正規方程式 (A + mu diag(A)) δ = −g を cols の変数で Cholesky で解く（A = JᵀJ、g = Jᵀr）
fn normal_step(a: &[f64], g: &[f64], nv: usize, cols: &[usize], mu: f64) -> Option<Vec<f64>> {
    let nf = cols.len();
    let mut af = vec![0.0; nf * nf];
    let mut bf = vec![0.0; nf];
    for (u, &q) in cols.iter().enumerate() {
        bf[u] = -g[q];
        for (w, &s) in cols.iter().enumerate() {
            af[u * nf + w] = a[q * nv + s];
        }
        af[u * nf + u] += mu * a[q * nv + q];
    }
    cholesky_solve(&mut af, &mut bf, nf)
}

/// [J D⁻¹; √mu I] y ≈ [−r; 0] を cols の変数で QR で解き、δ = D⁻¹ y（D_q = ‖J_q‖ = √A_qq）
fn qr_step(
    j: &[f64],
    r: &[f64],
    nv: usize,
    a: &[f64],
    cols: &[usize],
    mu: f64,
) -> Option<Vec<f64>> {
    let (m, nc) = (r.len(), cols.len());
    let dn: Vec<f64> = cols.iter().map(|&q| a[q * nv + q].sqrt()).collect();
    let rows = m + nc;
    let mut mat = vec![0.0; rows * nc];
    let mut b = vec![0.0; rows];
    for i in 0..m {
        for (k, &q) in cols.iter().enumerate() {
            mat[i * nc + k] = j[i * nv + q] / dn[k];
        }
        b[i] = -r[i];
    }
    let sm = mu.sqrt();
    for k in 0..nc {
        mat[(m + k) * nc + k] = sm;
    }
    let y = qr_lstsq(mat, rows, nc, b)?;
    Some(y.iter().zip(&dn).map(|(y, d)| y / d).collect())
}

/// 対称正定値の n×n を Cholesky で解く（正定値でなければ None）
fn cholesky_solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<Vec<f64>> {
    for c in 0..n {
        let mut d = a[c * n + c];
        for k in 0..c {
            d -= a[c * n + k] * a[c * n + k];
        }
        if !(d > 0.0) {
            return None;
        }
        let d = d.sqrt();
        a[c * n + c] = d;
        for rr in c + 1..n {
            let mut v = a[rr * n + c];
            for k in 0..c {
                v -= a[rr * n + k] * a[c * n + k];
            }
            a[rr * n + c] = v / d;
        }
    }
    let mut y = b.to_vec();
    for i in 0..n {
        for k in 0..i {
            y[i] -= a[i * n + k] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    for i in (0..n).rev() {
        for k in i + 1..n {
            y[i] -= a[k * n + i] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    Some(y)
}

/// 最小二乗 min ‖M y − b‖ を Householder の QR で解く（M は rows × n の行優先で、列は一次独立）。解けなければ None
fn qr_lstsq(mut m: Vec<f64>, rows: usize, n: usize, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let mut v = vec![0.0; rows];
    for k in 0..n {
        let norm = (k..rows).map(|i| m[i * n + k].powi(2)).sum::<f64>().sqrt();
        if !(norm > 0.0) {
            return None;
        }
        let alpha = if m[k * n + k] > 0.0 { -norm } else { norm };
        for i in k..rows {
            v[i] = m[i * n + k];
        }
        v[k] -= alpha;
        let vn2: f64 = (k..rows).map(|i| v[i] * v[i]).sum();
        if vn2 > 0.0 {
            for c in k..n {
                let sdot: f64 = (k..rows).map(|i| v[i] * m[i * n + c]).sum();
                let f = 2.0 * sdot / vn2;
                for i in k..rows {
                    m[i * n + c] -= f * v[i];
                }
            }
            let sdot: f64 = (k..rows).map(|i| v[i] * b[i]).sum();
            let f = 2.0 * sdot / vn2;
            for i in k..rows {
                b[i] -= f * v[i];
            }
        }
    }
    let mut y = vec![0.0; n];
    for i in (0..n).rev() {
        let mut t = b[i];
        for c in i + 1..n {
            t -= m[i * n + c] * y[c];
        }
        let d = m[i * n + i];
        if !(d.abs() > 0.0) {
            return None;
        }
        y[i] = t / d;
    }
    y.iter().all(|v| v.is_finite()).then_some(y)
}
