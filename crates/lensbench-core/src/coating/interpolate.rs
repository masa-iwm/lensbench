//! 補間の道具: 波長の区間、有理補間、鏡の分岐点、線形補間

use super::*;

/// wl をはさむ折れ目の間の点の範囲 [s, e)（範囲外や点が足りなければ None）
pub(in crate::coating) fn wave_segment(
    wls: &[f64],
    wl: f64,
    kinks: &[f64],
) -> Option<(usize, usize)> {
    const EPS: f64 = 1e-9;
    let n = wls.len();
    if n == 0 || !(wls[0] - EPS <= wl && wl <= wls[n - 1] + EPS) {
        return None;
    }
    // wl をはさむ折れ目の間 [lo, hi] の点だけを使う（折れ目の上の点は両側で使う）
    let lo = kinks
        .iter()
        .copied()
        .filter(|&k| k <= wl + EPS)
        .fold(f64::NEG_INFINITY, f64::max);
    let hi = kinks
        .iter()
        .copied()
        .filter(|&k| k > wl + EPS)
        .fold(f64::INFINITY, f64::min);
    let s = wls.partition_point(|&w| w < lo - EPS);
    let e = wls.partition_point(|&w| w <= hi + EPS);
    let n = e - s;
    // 折れ目の間に点が 1 つ以下だと補間できない（呼ぶ側が範囲の外と区別して知らせる）
    if n == 0 || (n < 2 && n < wls.len()) {
        return None;
    }
    Some((s, e))
}

/// 波長 wl での値: 近い 4 点（少なければ全点）を 1/λ の多項式で補間する（範囲外は None）
pub(in crate::coating) fn wave_weights(
    wls: &[f64],
    wl: f64,
    kinks: &[f64],
) -> Option<(usize, Vec<f64>)> {
    let (s, e) = wave_segment(wls, wl, kinks)?;
    let seg = &wls[s..e];
    let n = seg.len();
    let m = n.min(4);
    let i = seg
        .partition_point(|&w| w < wl)
        .saturating_sub(2)
        .min(n - m);
    let x: Vec<f64> = seg[i..i + m].iter().map(|w| 1.0 / w).collect();
    let xv = 1.0 / wl;
    let w = (0..m)
        .map(|a| {
            (0..m)
                .filter(|&b| b != a)
                .map(|b| (xv - x[b]) / (x[a] - x[b]))
                .product()
        })
        .collect();
    Some((s + i, w))
}

/// 鏡の 1 つの波長の分岐点（[`mirror_branch`]）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MirrorBranch {
    /// 分岐点の β²
    pub(in crate::coating) xb: f64,
    /// 全反射が最後まで続く最初の点。分岐点は点 k − 1 と k の間にある
    pub(in crate::coating) k: usize,
    /// 分岐点が測った点の上にある（[`MIRROR_BRANCH_SNAP`] で寄せた）
    pub(in crate::coating) on_grid: bool,
}

/// 鏡の分岐点として扱う、全反射の手前の 2 点での |r| の 1 からのずれの下限。これより小さい sqrt の項は、
/// 分岐点をまたいで補間しても誤差がその数分の 1 に収まる（|r| が 1 の 2e-6 手前で全反射に移る膜では、
/// 分岐点として扱うと推定の位置が 1e-4 ずれて係数が 1.2e-3 ずれ、またいで補間すると 3e-7）
pub(in crate::coating) const MIRROR_BRANCH_MIN_DEV: f64 = 1e-5;

/// 鏡の分岐点を探すとき、|r| = 1 からのずれがこれ未満の点を全反射とみる。値の 1e-9 ほどの違い（丸め・測定の桁）で
/// 全反射の判定が外れないよう、それより十分大きくする（1e-10 だと r を 1e-9 に丸めただけで分岐点が消え、
/// 分岐点をまたいで補間して係数が 2e-2 変わった）
pub const MIRROR_TIR_TOL: f64 = 1e-8;

/// 鏡の分岐点の推定が、隣り合う β² の点の間隔のこの割合より格子の点に近ければ、その点に寄せる。
/// 分岐点が測った点の上にある（層の屈折率がちょうど格子の β）とき、推定は値の 1e-9 の違いで 4e-10 ほど揺れ、
/// 分岐点のそばでは r が sqrt(|β² − xb|) で変わるので、揺れが係数で 5e-5 に広がる。格子の点に寄せれば推定は揺れない
pub(in crate::coating) const MIRROR_BRANCH_SNAP: f64 = 1e-6;

/// 鏡の補間に使う有理関数の点の数
pub(in crate::coating) const RATIONAL_N: usize = 8;

/// 昇順の点 xs[s..e] のうち q に近い m 点（[`RATIONAL_N`] から始めて、解けなければ 2 点ずつ減らす）を通る
/// 有理関数（分子 m/2 − 1 次・分母 m/2 次）の q での値。4 点でも解けなければ近い 4 点の多項式にする。
/// 連立方程式が退化する（データがもっと低い次数の有理関数で表せる）ときは、ピボットが相対的に小さくなるので次の m にする
pub(in crate::coating) fn rational_at(
    xs: &[f64],
    s: usize,
    e: usize,
    q: f64,
    f: &dyn Fn(usize) -> C64,
) -> C64 {
    let n = e - s;
    let start = |m: usize| {
        s + xs[s..e]
            .partition_point(|&x| x < q)
            .saturating_sub(m / 2)
            .min(n - m)
    };
    let mut m = RATIONAL_N.min(n - n % 2);
    while m >= 4 {
        if let Some(v) = rational_solve(xs, start(m), m, q, f) {
            return v;
        }
        m -= 2;
    }
    let m = n.min(4);
    let i = start(m);
    (i..i + m)
        .map(|a| {
            let w: f64 = (i..i + m)
                .filter(|&b| b != a)
                .map(|b| (q - xs[b]) / (xs[a] - xs[b]))
                .product();
            f(a) * w
        })
        .sum::<C64>()
}

/// xs[i..i+m] を通る有理関数 P/Q（Q(q) = 1）の q での値。退化していれば None
pub(in crate::coating) fn rational_solve(
    xs: &[f64],
    i: usize,
    m: usize,
    q: f64,
    f: &dyn Fn(usize) -> C64,
) -> Option<C64> {
    let t: Vec<f64> = (i..i + m).map(|j| xs[j] - q).collect();
    let h = t.iter().fold(0.0f64, |a, b| a.max(b.abs())).max(1e-300);
    // P(t) − f Q(t) = 0、Q(0) = 1（t は h で割って条件をよくする）
    let (np, nq) = (m / 2, m - m / 2);
    let mut a = vec![vec![C64::new(0.0, 0.0); m + 1]; m];
    for (r, row) in a.iter_mut().enumerate() {
        let u = t[r] / h;
        let fv = f(i + r);
        for (k, v) in row.iter_mut().enumerate().take(np) {
            *v = C64::new(u.powi(k as i32), 0.0);
        }
        for k in 1..=nq {
            row[np + k - 1] = -fv * u.powi(k as i32);
        }
        row[m] = fv;
    }
    let scale = a
        .iter()
        .flat_map(|r| r[..m].iter().map(|v| v.norm()))
        .fold(0.0, f64::max);
    for c in 0..m {
        let p = (c..m).max_by(|&x, &y| a[x][c].norm().total_cmp(&a[y][c].norm()))?;
        if a[p][c].norm() <= 1e-11 * scale {
            return None;
        }
        a.swap(c, p);
        for r in 0..m {
            if r != c {
                let g = a[r][c] / a[c][c];
                for k in c..=m {
                    let v = a[c][k];
                    a[r][k] -= g * v;
                }
            }
        }
    }
    let v = a[0][m] / a[0][0];
    (v.re.is_finite() && v.im.is_finite()).then_some(v)
}

/// 鏡の 1 つの波長の表（β² の昇順 b2、点ごとに偏光 s・p の r）から、β² 方向の分岐点を探す。
/// 膜の外の媒質（屈折率 n）へ光が抜ける鏡では、β > n で全反射になって |r| = 1 に張り付き、
/// β = n で r が sqrt(n² − β²) の形で変わる。有理関数でも多項式でも表せないので、補間の点がまたがないようにし、
/// 両側それぞれ t = sqrt(|β² − xb|) で補間する（r は t の滑らかな関数）。
/// |r| = 1（[`MIRROR_TIR_TOL`] 以内）が最後まで続く最初の点を k として、位置 xb は k とその手前の点の間で、両側から t = 0 へ外挿した r が一致する点。
/// 推定が k か k − 1 の点にごく近ければ（[`MIRROR_BRANCH_SNAP`]）、その点にする。
/// 層が吸収して |r| < 1 のままなら None（分岐点は扱わない）
pub(in crate::coating) fn mirror_branch(b2: &[f64], r: &[[C64; 2]]) -> Option<MirrorBranch> {
    let dev = |j: usize| {
        r[j].iter()
            .map(|v| (v.norm() - 1.0).abs())
            .fold(0.0, f64::max)
    };
    let n = b2.len();
    let k = (0..n).rev().find(|&j| !(dev(j) < MIRROR_TIR_TOL))? + 1;
    // 両側に 4 点以上。sqrt の形なら手前で |r| は 1 からはっきり離れている（全反射しにくい高反射膜で
    // |r| が β とともに少しずつ 1 に近づくだけのときは分岐点ではない）
    if k < 4 || n - k < 4 || dev(k - 1).max(dev(k - 2)) < MIRROR_BRANCH_MIN_DEV {
        return None;
    }
    let m = RATIONAL_N.min(k).min(n - k);
    // 両側から t = 0 へ外挿した r の食い違い
    let gap = |xb: f64| -> f64 {
        let tl: Vec<f64> = (0..m)
            .map(|i| (xb - b2[k - 1 - i]).max(0.0).sqrt())
            .collect();
        let th: Vec<f64> = (0..m).map(|i| (b2[k + i] - xb).max(0.0).sqrt()).collect();
        (0..2)
            .map(|pol| {
                let lo = rational_at(&tl, 0, m, 0.0, &|i| r[k - 1 - i][pol]);
                let hi = rational_at(&th, 0, m, 0.0, &|i| r[k + i][pol]);
                (lo - hi).norm()
            })
            .sum::<f64>()
    };
    let (a0, c0) = (b2[k - 1], b2[k]);
    const N: usize = 200;
    let grid = |i: usize| a0 + (c0 - a0) * i as f64 / N as f64;
    let j = (0..=N)
        .min_by(|&p, &q| gap(grid(p)).total_cmp(&gap(grid(q))))
        .unwrap_or(0);
    let (mut a, mut c) = (grid(j.saturating_sub(1)), grid((j + 1).min(N)));
    // 黄金分割で詰める
    for _ in 0..60 {
        let (p, q) = (a + (c - a) * 0.381_966, a + (c - a) * 0.618_034);
        if gap(p) < gap(q) {
            c = q;
        } else {
            a = p;
        }
    }
    let xb = 0.5 * (a + c);
    // 格子の点より悪ければ格子の点
    let xb = if gap(xb) <= gap(grid(j)) { xb } else { grid(j) };
    // 測った点のすぐそばなら、その点に寄せる
    let tol = MIRROR_BRANCH_SNAP * (c0 - a0);
    let (xb, on_grid) = if xb - a0 <= tol {
        (a0, true)
    } else if c0 - xb <= tol {
        (c0, true)
    } else {
        (xb, false)
    };
    Some(MirrorBranch { xb, k, on_grid })
}

/// [`wave_weights`] が補間できない理由
pub(in crate::coating) fn out_of_waves(wls: &[f64], wl: f64) -> String {
    match (wls.first(), wls.last()) {
        (Some(&a), Some(&b)) if a - 1e-9 <= wl && wl <= b + 1e-9 => {
            "has fewer than 2 measured wavelengths between the kinks (KINKS) around it".to_string()
        }
        (Some(&a), Some(&b)) => format!("is outside the range ({a} to {b} µm)"),
        _ => "has no wavelength data".to_string(),
    }
}

/// 実数の値の列を β² の not-a-knot スプラインにする
pub(in crate::coating) fn beta_spline(name: &str, b2: &[f64], ys: &[f64]) -> Result<Spline> {
    match Spline::not_a_knot(b2, ys) {
        Some(s) => Ok(s),
        None => invalid!("coating model {name}: β can't be interpolated"),
    }
}

/// 線形補間（xs は昇順。範囲外は端の値、numpy の interp と同じ）
/// 度の位相の列の ±180° の飛びをほどく（隣との差が 180° を超えれば 360° ずらす）
pub(in crate::coating) fn unwrap_deg(v: &mut [f64]) {
    for i in 1..v.len() {
        let d = v[i] - v[i - 1];
        v[i] -= 360.0 * (d / 360.0).round();
    }
}

pub fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len();
    if x.is_nan() {
        return f64::NAN;
    }
    if n == 1 || x <= xs[0] {
        return ys[0];
    }
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    let i = xs.partition_point(|&v| v <= x) - 1;
    let t = (x - xs[i]) / (xs[i + 1] - xs[i]);
    ys[i] + t * (ys[i + 1] - ys[i])
}

/// 鏡の β² 方向の分岐点 xb と、その両側の点の座標 t = sqrt(|β² − xb|)（どちらも t の昇順）。
/// r は分岐点の両側それぞれで t の滑らかな関数なので、t で補間する
#[derive(Debug, Clone)]
pub(in crate::coating) struct Branch {
    pub(in crate::coating) xb: f64,
    /// 下側: 点 lo_end − 1 − i の t
    pub(in crate::coating) lo_end: usize,
    pub(in crate::coating) t_lo: Vec<f64>,
    /// 上側: 点 hi_start + i の t
    pub(in crate::coating) hi_start: usize,
    pub(in crate::coating) t_hi: Vec<f64>,
}

impl Branch {
    pub(in crate::coating) fn new(b2: &[f64], xb: f64) -> Branch {
        // 分岐点ちょうどの点は両側で使う
        let lo_end = b2.partition_point(|&v| v <= xb + 1e-12);
        let hi_start = b2.partition_point(|&v| v < xb - 1e-12);
        Branch {
            xb,
            lo_end,
            t_lo: (0..lo_end)
                .rev()
                .map(|j| (xb - b2[j]).max(0.0).sqrt())
                .collect(),
            hi_start,
            t_hi: (hi_start..b2.len())
                .map(|j| (b2[j] - xb).max(0.0).sqrt())
                .collect(),
        }
    }

    /// β² = x での値（f(j) は点 j の値）
    pub(in crate::coating) fn eval(&self, x: f64, f: &dyn Fn(usize) -> C64) -> C64 {
        if x <= self.xb {
            let q = (self.xb - x).sqrt();
            rational_at(&self.t_lo, 0, self.t_lo.len(), q, &|i| {
                f(self.lo_end - 1 - i)
            })
        } else {
            let q = (x - self.xb).sqrt();
            rational_at(&self.t_hi, 0, self.t_hi.len(), q, &|i| f(self.hi_start + i))
        }
    }
}
