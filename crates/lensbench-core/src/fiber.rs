//! 単一モードファイバへの結合効率（OpticStudio の Fiber Coupling・FICL 相当）。
//!
//! 送り側のファイバは有限物体の物点に、受け側のファイバは像面の局所座標の点に置き、どちらも基本モードを
//! ガウス（遠視野の 1/e² の NA）とする。振幅は換算した方向余弦 ρ = n·(l, m) で exp(−|ρ|²/NA²)
//! （受け側は像空間の n′·(L, M)、ファイバの軸は面の局所 z）。
//!
//! 物体空間の方向の格子（tan θ で等間隔、G × G）で光線を追い、像面の点の角アイコナール
//! T = OPL − n′ (r − r_r)·k（理想レンズの位相込み）から、受け側の方向空間での重なり積分
//!
//!   η = |Σ a_s a_r √(|∂ρ/∂(u,v)| |∂ρ′/∂(u,v)|) e^{−ikT}|² / (Σ a_s² |∂ρ/∂(u,v)| · π NA_r²/2)
//!
//! を求める（和は格子、ヤコビアンは格子の差分）。分母の和は標本化する円錐（物体空間の sin θ ≤ `na_max`）の中だけで
//! とるので、η はその円錐に入る送り側の電力に対する比になる。止まった光線（けられ・全反射など）は重なりに入らない。
//! 系の絞り（[`FiberOpts::clip_stop`]）: 絞り面に開口がなければ絞り半径の円を開口とし、その外を通る光線を
//! 系の損失に数える（OpticStudio の FICL と同じ）。境界の格子点は、絞り面での半径から求めた覆う割合で重みをつける
//! （格子の刻みに対して 2 次の精度。整合したガウスを半径 NA の円で切ると grid 101 で 3e-4）。絞り面に明示の開口があれば、ほかの面と同じく追跡で止まる。
//! 偏光込みでは、送り側の局所 x・y の偏光ごとに、出射電場の像面の局所 x・y 成分で同じ和をとった 2 × 2 の複素行列
//! （[`Coupling::jones`]）を返す。入射偏光 e の結合は |C e|²（受け側の 2 つの偏光モードの和）。

use crate::coating::{Coatings, C64};
use crate::error::Result;
use crate::invalid;
use crate::math::*;
use crate::surface::Aperture;
use crate::system::{Launch, System, Trace};

/// 結合の計算の設定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FiberOpts {
    /// 送り側・受け側のモードの NA（遠視野の強度が 1/e² になる n·sin θ）
    pub na_source: f64,
    pub na_receiver: f64,
    /// 送り側の物点（物体面の局所座標 [mm]）
    pub source: [f64; 2],
    /// 受け側の点（像面の局所座標 [mm]）
    pub receiver: [f64; 2],
    /// 格子の点の数（片側、奇数）
    pub grid: usize,
    /// 標本化する物体空間の sin θ（None なら min(3 NA_s / n, 0.999)）
    pub na_max: Option<f64>,
    /// 系の絞り（絞り面に開口がなければ絞り半径の円）の外を通る光線を損失に数える（既定 true）。
    /// false なら絞りを無視する（明示の開口では止まる）
    pub clip_stop: bool,
}

impl Default for FiberOpts {
    fn default() -> Self {
        FiberOpts {
            na_source: 0.1,
            na_receiver: 0.1,
            source: [0.0; 2],
            receiver: [0.0; 2],
            grid: 101,
            na_max: None,
            clip_stop: true,
        }
    }
}

/// 結合の結果
#[derive(Debug, Clone, Copy)]
pub struct Coupling {
    /// 結合効率（偏光なし、膜の透過を含まない）
    pub eta: f64,
    /// 系の効率: 標本化した送り側の電力のうち、受け側の点まで届いた光線の分（系の絞り・開口での蹴られを除いた分）
    pub system: f64,
    /// 受け側の効率 = eta / system
    pub receiver: f64,
    /// 偏光込みの重なり C[出射 x/y][入射 x/y]（偏光込みで計算したときだけ）。位相は格子の中心の光線を基準にする
    pub jones: Option<[[C64; 2]; 2]>,
}

impl Coupling {
    /// 入射偏光 e（送り側の局所 x・y）の結合効率 |C e|²（偏光込みで計算したときだけ）
    pub fn eta_pol(&self, e: [C64; 2]) -> Option<f64> {
        let c = self.jones?;
        let n = (e[0].norm_sqr() + e[1].norm_sqr()).sqrt();
        Some(
            (0..2)
                .map(|i| ((c[i][0] * e[0] + c[i][1] * e[1]) / n).norm_sqr())
                .sum(),
        )
    }
}

/// 格子の差分（numpy.gradient と同じ: 内側は中心差分、端は片側）。両隣が使えない点は None
fn grad(f: &[f64], ok: &[bool], g: usize, d: f64, i: usize, j: usize, axis: usize) -> Option<f64> {
    let at = |a: usize, b: usize| a * g + b;
    let (p, q) = if axis == 0 { (i, j) } else { (j, i) };
    let idx = |k: usize| if axis == 0 { at(k, q) } else { at(q, k) };
    let lo = p > 0 && ok[idx(p - 1)];
    let hi = p + 1 < g && ok[idx(p + 1)];
    let c = f[idx(p)];
    match (lo, hi) {
        (true, true) => Some((f[idx(p + 1)] - f[idx(p - 1)]) / (2.0 * d)),
        (false, true) => Some((f[idx(p + 1)] - c) / d),
        (true, false) => Some((c - f[idx(p - 1)]) / d),
        (false, false) => None,
    }
}

/// 系の絞りの重み（格子点ごと、0〜1）。絞り面に明示の開口があるか絞り半径がないときはすべて 1（追跡で止まる）。
/// 絞り面での f = r/R − 1 とその格子の勾配から、境界までの符号つき距離（格子の単位）を出し、
/// 境界を直線とみなして格子の 1 区画（中心が格子点の正方形）のうち内側の面積の割合をとる
fn stop_weights(s: &System, w: usize, l: &Launch, g: usize, d: f64, inside: &[bool]) -> Vec<f64> {
    let st = s.stop();
    let ones = vec![1.0; g * g];
    if !matches!(
        s.surfaces[st].ap,
        Aperture::None | Aperture::Floating { .. }
    ) {
        return ones;
    }
    let Some(Aperture::Circular { max, dx, dy, .. }) = crate::pop::surface_aperture(s, st) else {
        return ones;
    };
    let t = s.trace_launch(l, w, Some(st), false);
    let f: Vec<f64> = t
        .rays
        .iter()
        .map(|r| {
            let v = (r.xyz[0] - dx).hypot(r.xyz[1] - dy) / max - 1.0;
            if r.err == 0 && v.is_finite() {
                v
            } else {
                f64::NAN
            }
        })
        .collect();
    let okf: Vec<bool> = f.iter().map(|v| v.is_finite()).collect();
    let mut wt = vec![0.0; g * g];
    for i in 0..g {
        for j in 0..g {
            let q = i * g + j;
            if !inside[q] || !okf[q] {
                continue;
            }
            let gx = grad(&f, &okf, g, d, i, j, 1);
            let gy = grad(&f, &okf, g, d, i, j, 0);
            wt[q] = match gx.zip(gy).filter(|(a, b)| a.hypot(*b) > 0.0) {
                // 境界までの符号つき距離（内側が正、格子の単位）と法線
                Some((a, b)) => {
                    let gn = a.hypot(b);
                    cell_cover(-f[q] / (gn * d), a / gn, b / gn)
                }
                None => {
                    if f[q] <= 0.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
            };
        }
    }
    wt
}

/// 中心から内側へ距離 dist（区画の幅を 1）の直線を境界とする、単位の正方形のうち内側（法線 (nx, ny) の逆向き）の面積
fn cell_cover(dist: f64, nx: f64, ny: f64) -> f64 {
    // clamp(x, 0, 1) の原始関数
    let h = |x: f64| {
        if x <= 0.0 {
            0.0
        } else if x <= 1.0 {
            0.5 * x * x
        } else {
            x - 0.5
        }
    };
    let (a, b) = (nx.abs().max(ny.abs()), nx.abs().min(ny.abs()));
    // 正方形の中の点 (x, y)（a の向きが x）で内側は a x + b y ≤ dist。y ごとの x の長さ clamp((dist − b y)/a + 1/2) を積分
    // 正方形が境界をまたがないとき（差の桁落ちを避けるため先に）
    if dist >= 0.5 * (a + b) {
        return 1.0;
    }
    if dist <= -0.5 * (a + b) {
        return 0.0;
    }
    if b < 1e-6 * a {
        return (dist / a + 0.5).clamp(0.0, 1.0);
    }
    let u = |y: f64| (dist - b * y) / a + 0.5;
    (a / b * (h(u(-0.5)) - h(u(0.5)))).clamp(0.0, 1.0)
}

/// 波長 w の結合効率。coat を与えると偏光込み（[`Coupling::jones`]）も求める
pub fn fiber_coupling(
    s: &System,
    w: usize,
    o: &FiberOpts,
    coat: Option<&Coatings>,
) -> Result<Coupling> {
    if s.infinite() {
        invalid!("ファイバ結合は有限物体の系だけ");
    }
    if o.grid < 3 || o.grid.is_multiple_of(2) {
        invalid!("grid は 3 以上の奇数");
    }
    if !(o.na_source > 0.0 && o.na_receiver > 0.0) {
        invalid!("NA は正");
    }
    if w >= s.wavelengths.len() {
        invalid!("波長 {w} がない");
    }
    let n0 = s.surfaces[0].n[w];
    let n1 = s.image_index(w);
    let smax = o.na_max.unwrap_or((3.0 * o.na_source / n0).min(0.999));
    if !(smax > 0.0 && smax < 1.0) {
        invalid!("na_max は 0 と 1 の間");
    }
    let tmax = smax / (1.0 - smax * smax).sqrt();
    let g = o.grid;
    let d = 2.0 / (g - 1) as f64;
    let u: Vec<f64> = (0..g).map(|i| -1.0 + i as f64 * d).collect();
    // 物体の座標系の方向（tan θ = (u, v)·tmax）、行 i が v、列 j が u（numpy の meshgrid と同じ並び）
    let f = s.object_frame();
    let p0 = add(f.o, mv(&f.r, [o.source[0], o.source[1], 0.0]));
    let mut dirs = Vec::with_capacity(g * g);
    let (mut rl, mut rm) = (vec![0.0; g * g], vec![0.0; g * g]);
    let mut inside = vec![false; g * g];
    for i in 0..g {
        for j in 0..g {
            let (tx, ty) = (u[j] * tmax, u[i] * tmax);
            let sn = (1.0 + tx * tx + ty * ty).sqrt();
            let k = [tx / sn, ty / sn, 1.0 / sn];
            let q = i * g + j;
            rl[q] = n0 * k[0];
            rm[q] = n0 * k[1];
            inside[q] = k[0] * k[0] + k[1] * k[1] <= smax * smax * (1.0 + 1e-12);
            dirs.push(mv(&f.r, k));
        }
    }
    let ps = vec![p0; g * g];
    let launch = Launch::Rays { p: &ps, k: &dirs };
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let (t, e): (Trace, Option<Vec<Vec<crate::trace::CV3>>>) = match coat {
        Some(c) => {
            let p = s.trace_pol_launch(&launch, w, &pols, c, false)?;
            (p.trace, Some(p.e))
        }
        None => (s.trace_launch(&launch, w, None, false), None),
    };
    let ok0: Vec<bool> = (0..g * g)
        .map(|q| {
            let r = &t.rays[q];
            t.ok(q) && r.xyz.iter().chain(&r.lmn).all(|x| x.is_finite()) && r.opl.is_finite()
        })
        .collect();
    let (mut ql, mut qm, mut tt) = (
        vec![f64::NAN; g * g],
        vec![f64::NAN; g * g],
        vec![f64::NAN; g * g],
    );
    for q in 0..g * g {
        if ok0[q] {
            let r = &t.rays[q];
            let rel = [r.xyz[0] - o.receiver[0], r.xyz[1] - o.receiver[1], r.xyz[2]];
            ql[q] = n1 * r.lmn[0];
            qm[q] = n1 * r.lmn[1];
            tt[q] = r.opl + r.lens_opd - n1 * dot(rel, r.lmn);
        }
    }
    let wt = if o.clip_stop {
        stop_weights(s, w, &launch, g, d, &inside)
    } else {
        vec![1.0; g * g]
    };
    let all = vec![true; g * g];
    let jac = |a: &[f64], b: &[f64], ok: &[bool], i: usize, j: usize| -> Option<f64> {
        let ax = grad(a, ok, g, d, i, j, 1)?;
        let ay = grad(a, ok, g, d, i, j, 0)?;
        let bx = grad(b, ok, g, d, i, j, 1)?;
        let by = grad(b, ok, g, d, i, j, 0)?;
        Some((ax * by - ay * bx).abs())
    };
    let kw = 2.0 * std::f64::consts::PI / (s.wavelengths[w] * 1e-3);
    let c = g / 2;
    let t0 = if ok0[c * g + c] {
        tt[c * g + c]
    } else {
        let v: Vec<f64> = (0..g * g)
            .filter(|&q| ok0[q] && inside[q])
            .map(|q| tt[q])
            .collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    // 格子点ごとの被積分量 [送り側の電力, 重なり, 偏光の重なり 4 つ]（受け側まで届いた点だけ）
    let mut val: Vec<Option<[C64; 6]>> = vec![None; g * g];
    let mut den = 0.0;
    for i in 0..g {
        for j in 0..g {
            let q = i * g + j;
            if !inside[q] {
                continue;
            }
            let js = jac(&rl, &rm, &all, i, j).unwrap();
            let a_s = (-(rl[q] * rl[q] + rm[q] * rm[q]) / (o.na_source * o.na_source)).exp();
            den += a_s * a_s * js;
            if !ok0[q] {
                continue;
            }
            let Some(jr) = jac(&ql, &qm, &ok0, i, j) else {
                continue;
            };
            let a_r = (-(ql[q] * ql[q] + qm[q] * qm[q]) / (o.na_receiver * o.na_receiver)).exp();
            let z = C64::from_polar(a_s * a_r * (js * jr).sqrt(), -kw * (tt[q] - t0));
            let mut v = [
                C64::new(a_s * a_s * js, 0.0),
                z,
                C64::default(),
                C64::default(),
                C64::default(),
                C64::default(),
            ];
            if let Some(e) = &e {
                // e[光線][入力の偏光][成分]、v[2 + 2a + b] = 出射 a・入射 b
                for a in 0..2 {
                    for b in 0..2 {
                        v[2 + 2 * a + b] = z * e[q][b][a];
                    }
                }
            }
            val[q] = Some(v);
        }
    }
    // 重みつきの和（絞りの境界の区画は内側の面積の割合）
    let mut acc = [C64::default(); 6];
    for q in 0..g * g {
        if let (true, Some(v)) = (inside[q] && wt[q] > 0.0, val[q]) {
            for k in 0..6 {
                acc[k] += v[k] * wt[q];
            }
        }
    }
    let sys = acc[0].re;
    let num = acc[1];
    let cj = [[acc[2], acc[3]], [acc[4], acc[5]]];
    let norm = den * std::f64::consts::PI * o.na_receiver * o.na_receiver / 2.0;
    let eta = num.norm_sqr() * d * d / norm;
    let jones = e.map(|_| {
        let sc = d / norm.sqrt();
        cj.map(|r| r.map(|x| x * sc))
    });
    Ok(Coupling {
        eta,
        system: if den > 0.0 { sys / den } else { 0.0 },
        receiver: if sys > 0.0 { eta * den / sys } else { 0.0 },
        jones,
    })
}
