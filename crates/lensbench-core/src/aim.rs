//! 実光線エイミングの反復の部品（[`crate::system::System`] の aim から使う）。
//!
//! 光線ごとに、入射瞳の座標 v から絞り面の局所座標 q(v) が目標 t になる v を解く。
//! 本数が多いときは、少数の格子の光線を先に解いて t → v を多項式で当てはめ（[`AimFit`]）、
//! その値を初期値に、その微分（逆ヤコビアンの近似）を固定した反復（1 回 1 本の追跡）で解く。
//! 縮小が遅い・外れた光線は、差分のヤコビアンのニュートン法（1 回 3 本）に切り替える。

use crate::newton2::{Diff, Line, Newton2};

/// 反復の回数の上限
pub(crate) const MAX_IT: usize = 40;

/// 1 本の光線の解き始め: 目標 tgt、初期値 v、固定の逆ヤコビアン dv/dq m（行優先。None ならはじめからニュートン法）、
/// 当てはめの初期値で追跡できなかったときに戻る初期値 v0（近軸）
#[derive(Clone, Copy)]
pub(crate) struct AimStart {
    pub tgt: [f64; 2],
    pub v: [f64; 2],
    pub m: Option<[f64; 4]>,
    pub v0: [f64; 2],
}

impl AimStart {
    pub(crate) fn new(tgt: [f64; 2], v: [f64; 2], m: Option<[f64; 4]>, v0: [f64; 2]) -> Self {
        AimStart { tgt, v, m, v0 }
    }
}

/// 解いた結果: 入射瞳の座標 v と、その点の残差 r = q(v) − tgt
#[derive(Clone, Copy)]
pub(crate) struct AimSol {
    pub v: [f64; 2],
    pub r: [f64; 2],
}

impl AimSol {
    /// 解けなかったか（残差が有限で fail_tol を超える。NaN は追跡の側で止まる）
    pub(crate) fn failed(&self, fail_tol: f64) -> bool {
        let e = self.r;
        e[0].is_finite() && e[1].is_finite() && e[0].abs().max(e[1].abs()) > fail_tol
    }
    /// 絞りまで届かなかったか
    pub(crate) fn missed(&self) -> bool {
        !(self.r[0].is_finite() && self.r[1].is_finite())
    }
}

/// 光線の途中の状態（固定の行列の反復の間）
#[derive(Clone, Copy)]
enum Phase {
    /// 固定の逆ヤコビアンの反復（it 回まで更新した）
    Fixed,
    /// ニュートン法へ（初めの点 v、その f の値 q（分からなければ None）、反復の残り）
    Newton([f64; 2], Option<[f64; 2]>, usize),
    Done,
}

/// N 本の光線を、入射瞳の座標 v から絞り面の局所座標 q = f(v) が目標になるように解く（f は N 本を 1 度に）。
/// 固定の逆ヤコビアンがある光線は、まずそれで v ← v − m (q − tgt) を繰り返す（1 回 1 本の追跡）。初めの点で
/// 届かなければ近軸の初期値 v0 から、縮みが半分に満たなければ前の点から、差分のヤコビアン（刻み h の前進差分）の
/// ニュートン法（[`Newton2`]）に切り替える。収束は max(|q − tgt|) ≤ lim、反復は 2 つを合わせて [`MAX_IT`] 回まで。
/// 光線ごとの手順・結果は N によらず同じビット
pub(crate) fn aim_lanes<const N: usize>(
    st: [AimStart; N],
    mut f: impl FnMut(&[[f64; 2]; N]) -> [[f64; 2]; N],
    lim: f64,
    h: f64,
) -> [AimSol; N] {
    let mut v: [[f64; 2]; N] = st.map(|a| a.v);
    let mut q = f(&v);
    let mut ph: [Phase; N] = std::array::from_fn(|l| match st[l].m {
        Some(_) => Phase::Fixed,
        None => Phase::Newton(v[l], Some(q[l]), MAX_IT),
    });
    let (mut vp, mut qp) = (v, q);
    let mut nep = [f64::INFINITY; N];
    let mut it = [0usize; N];
    loop {
        for l in 0..N {
            let Phase::Fixed = ph[l] else { continue };
            if it[l] >= MAX_IT {
                ph[l] = Phase::Done;
                continue;
            }
            let e = [q[l][0] - st[l].tgt[0], q[l][1] - st[l].tgt[1]];
            let ne = e[0].abs().max(e[1].abs());
            if it[l] == 0 && !ne.is_finite() {
                // 当てはめの初期値で絞りまで届かない: 近軸の初期値からニュートン法で解き直す
                ph[l] = Phase::Newton(st[l].v0, None, MAX_IT - 1);
                continue;
            }
            if it[l] > 0 && !(ne <= 0.5 * nep[l]) {
                // 固定の行列での更新が半分以下に縮めなかった（外れた・NaN を含む）: 前の点からニュートン法へ
                ph[l] = Phase::Newton(vp[l], Some(qp[l]), MAX_IT - it[l]);
                continue;
            }
            if !(ne > lim) {
                ph[l] = Phase::Done;
                continue;
            }
            let m = st[l].m.expect("fixed phase has a matrix");
            let dv = [m[0] * e[0] + m[1] * e[1], m[2] * e[0] + m[3] * e[1]];
            nep[l] = ne;
            vp[l] = v[l];
            qp[l] = q[l];
            if dv[0].is_finite() && dv[1].is_finite() {
                v[l] = [v[l][0] - dv[0], v[l][1] - dv[1]];
            }
            it[l] += 1;
        }
        if !ph.iter().any(|p| matches!(p, Phase::Fixed)) {
            break;
        }
        let qn = f(&v);
        for l in 0..N {
            if let Phase::Fixed = ph[l] {
                q[l] = qn[l];
            }
        }
    }
    if !ph.iter().any(|p| matches!(p, Phase::Newton(..))) {
        return std::array::from_fn(|l| AimSol {
            v: v[l],
            r: [q[l][0] - st[l].tgt[0], q[l][1] - st[l].tgt[1]],
        });
    }
    // ニュートン法の光線を解く（終わった光線は今の点のまま、反復 0 回で止める）
    let mut v1 = v;
    let mut known = true;
    let mut q1 = q;
    let mut mx = [0usize; N];
    for l in 0..N {
        if let Phase::Newton(a, b, k) = ph[l] {
            v1[l] = a;
            mx[l] = k;
            match b {
                Some(b) => q1[l] = b,
                None => known = false,
            }
        }
    }
    let q1 = if known {
        q1
    } else {
        let qn = f(&v1);
        std::array::from_fn(|l| match ph[l] {
            Phase::Newton(_, None, _) => qn[l],
            _ => q1[l],
        })
    };
    let nw = Newton2 {
        tol: lim,
        max_it: MAX_IT,
        diff: Diff::fwd(h),
        line: Line::Full,
    };
    let s = nw.solve_lanes_from(f, v1, Some(q1), st.map(|a| a.tgt), [lim; N], mx);
    std::array::from_fn(|l| AimSol {
        v: s[l].v,
        r: s[l].r,
    })
}

/// 正規化した絞りの座標 (qx, qy) → 入射瞳の座標 v の多項式（チェビシェフ多項式の積、全次数 DEG 以下）
pub(crate) struct AimFit {
    c: [f64; 2],
    hw: [f64; 2],
    /// 係数（項ごとに vx, vy）
    coef: Vec<[f64; 2]>,
}

const DEG: usize = 8;
/// 当てはめに使う格子の一辺の点数
pub(crate) const GRID: usize = 16;

fn terms() -> impl Iterator<Item = (usize, usize)> {
    (0..=DEG).flat_map(|d| (0..=d).map(move |a| (a, d - a)))
}

/// T_k(u) と T_k'(u)（k = 0..=DEG）
fn cheb(u: f64) -> ([f64; DEG + 1], [f64; DEG + 1]) {
    let mut t = [0.0; DEG + 1];
    let mut dt = [0.0; DEG + 1];
    // U_k（第 2 種）で T_k' = k U_{k-1}
    let mut uk = [0.0; DEG + 1];
    t[0] = 1.0;
    t[1] = u;
    uk[0] = 1.0;
    uk[1] = 2.0 * u;
    for k in 2..=DEG {
        t[k] = 2.0 * u * t[k - 1] - t[k - 2];
        uk[k] = 2.0 * u * uk[k - 1] - uk[k - 2];
    }
    for k in 1..=DEG {
        dt[k] = k as f64 * uk[k - 1];
    }
    (t, dt)
}

impl AimFit {
    /// 当てはめの範囲（入力の q の範囲）と格子の点。範囲が線や点に潰れていれば None
    #[allow(clippy::type_complexity)]
    pub(crate) fn grid(qx: &[f64], qy: &[f64]) -> Option<([f64; 2], [f64; 2], Vec<[f64; 2]>)> {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for (&x, &y) in qx.iter().zip(qy) {
            if x.is_finite() && y.is_finite() {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let hw = [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0];
        if !(hw[0] > 1e-6 && hw[1] > 1e-6) {
            return None;
        }
        // チェビシェフ点（端を含む）
        let node = |i: usize| -(std::f64::consts::PI * i as f64 / (GRID - 1) as f64).cos();
        let pts = (0..GRID)
            .flat_map(|j| (0..GRID).map(move |i| (i, j)))
            // 範囲の矩形に内接する楕円の中だけ（瞳は円いので、角の点は絞り・面の外で解けず、反復の上限まで回っていた）
            .filter(|&(i, j)| node(i).hypot(node(j)) <= 1.0 + 1e-9)
            .map(|(i, j)| [c[0] + hw[0] * node(i), c[1] + hw[1] * node(j)])
            .collect();
        Some((c, hw, pts))
    }

    /// 格子の点 pts（正規化した q）で解いた v（解けなかった点は None）から当てはめる
    pub(crate) fn new(
        c: [f64; 2],
        hw: [f64; 2],
        pts: &[[f64; 2]],
        v: &[Option<[f64; 2]>],
    ) -> Option<Self> {
        let nt = terms().count();
        // 解けた点の行（項ごとの値）を列優先に並べ、最小二乗を QR で解く（正規方程式は条件数の 2 乗で効く）
        let ok: Vec<(&[f64; 2], [f64; 2])> = pts
            .iter()
            .zip(v)
            .filter_map(|(p, v)| {
                v.filter(|v| v[0].is_finite() && v[1].is_finite())
                    .map(|v| (p, v))
            })
            .collect();
        let m = ok.len();
        if m < 2 * nt {
            return None;
        }
        let mut a = vec![0.0; m * nt];
        for (i, (p, _)) in ok.iter().enumerate() {
            let (tx, _) = cheb((p[0] - c[0]) / hw[0]);
            let (ty, _) = cheb((p[1] - c[1]) / hw[1]);
            for (k, (ia, ib)) in terms().enumerate() {
                a[k * m + i] = tx[ia] * ty[ib];
            }
        }
        let qr = crate::linalg::Qr::from_cols(a, m);
        // チェビシェフ点の上の項は条件数が小さい。対角が潰れていれば（点が偏っている）当てはめない
        let rmax = qr.rdiag().iter().fold(0.0f64, |x, d| x.max(d.abs()));
        if !qr.rdiag().iter().all(|d| d.abs() > 1e-6 * rmax) {
            return None;
        }
        let bx: Vec<f64> = ok.iter().map(|(_, v)| v[0]).collect();
        let by: Vec<f64> = ok.iter().map(|(_, v)| v[1]).collect();
        let (cx, cy) = (qr.solve(&bx), qr.solve(&by));
        if !cx.iter().chain(&cy).all(|x| x.is_finite()) {
            return None;
        }
        let coef = cx.into_iter().zip(cy).map(|(x, y)| [x, y]).collect();
        Some(AimFit { c, hw, coef })
    }

    /// 正規化した q での v と dv/dq（行優先、q は正規化した座標）
    pub(crate) fn eval(&self, q: [f64; 2]) -> ([f64; 2], [f64; 4]) {
        let u = [
            (q[0] - self.c[0]) / self.hw[0],
            (q[1] - self.c[1]) / self.hw[1],
        ];
        let (tx, dtx) = cheb(u[0]);
        let (ty, dty) = cheb(u[1]);
        let mut v = [0.0; 2];
        let mut d = [0.0; 4];
        for ((a, b), c) in terms().zip(&self.coef) {
            let (f, fx, fy) = (tx[a] * ty[b], dtx[a] * ty[b], tx[a] * dty[b]);
            v[0] += c[0] * f;
            v[1] += c[1] * f;
            d[0] += c[0] * fx;
            d[1] += c[0] * fy;
            d[2] += c[1] * fx;
            d[3] += c[1] * fy;
        }
        let (sx, sy) = (1.0 / self.hw[0], 1.0 / self.hw[1]);
        (v, [d[0] * sx, d[1] * sy, d[2] * sx, d[3] * sy])
    }
}
