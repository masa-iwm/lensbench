//! 簡易な回折の伝搬（Physical Optics Propagation の簡易版）。
//!
//! 1. [`beam_from_rays`]: 実光線を面 j まで追跡し、面 j の接平面（局所座標の z = 0）の上に複素振幅の場
//!    （[`Beam`]）を作る。振幅は光線の密度（入射瞳で一様な照度）から、位相は光路長から。
//!    面の開口（絞りは絞り半径の円）は格子の上で解析的に掛ける（縁の画素は面積の割合）。
//! 2. [`propagate`]: 一様な媒質の中を、平行な平面まで距離 d だけ伝える。
//!    - [`PopMethod::Fresnel`]: Fresnel 近似（近軸）の Huygens・Fresnel 積分。核が x・y に分かれるので
//!      行列の積 2 回で求める（出力の格子は自由。O(N²M + NM²)）。
//!    - [`PopMethod::Direct`]: Rayleigh-Sommerfeld の第 1 種の積分を直接（近似なし、O(N²M²)。小さな格子向け）。
//!
//! 場は「パイロット」（傾き a と曲率 κ の波面、[`Pilot`]）を除いた残りを格子に持つ: 全体の場は
//! u(x, y)·exp(i k (a_x X + a_y Y + κ (X² + Y²)/2))（X = x − x0、k = 2π n/λ）。パイロットは解析的に扱うので、
//! 集光する場でも残りの位相（収差・デフォーカス）を標本化できれば足りる。出力のパイロットは
//! Gauss ビームの q（1/q = κ − i λ′/(π w²)）を d だけ進めたもの（焦点付近で平面に近づく）。
//!
//! 全体の位相の定数（e^{ikd} など）は落とす。強度は Σ|u|² dx dy = 1（開口を掛ける前）に正規化する。

use crate::coating::C64;
use crate::error::Result;
use crate::surface::Aperture;
use crate::system::{FieldSel, System};
use crate::{invalid, notimpl};
use std::f64::consts::PI;

/// パイロットの波面: 光路長 n·(a_x X + a_y Y + κ (X² + Y²)/2)（X, Y は格子の中心から）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pilot {
    /// 傾き（伝搬の向きの方向余弦の x・y 成分に当たる）
    pub ax: f64,
    pub ay: f64,
    /// 曲率 [1/mm]（正: 発散、負: 集光。集光点までの距離 R なら −1/R）
    pub curv: f64,
}

/// 平面の上の複素振幅の場
#[derive(Debug, Clone)]
pub struct Beam {
    pub nx: usize,
    pub ny: usize,
    pub dx: f64,
    pub dy: f64,
    /// 格子の中心（標本 i の x は x0 + (i − (nx − 1)/2)·dx）
    pub x0: f64,
    pub y0: f64,
    /// 真空の波長 [mm]
    pub wl: f64,
    /// 媒質の屈折率
    pub index: f64,
    pub pilot: Pilot,
    /// パイロットの Gauss ビームの半径 [mm]（出力のパイロットの曲率を決めるだけ）
    pub w: f64,
    /// 局所座標の z の向きに対する伝搬の向き（+1 / −1。鏡の後は −1）
    pub dir: f64,
    /// パイロットを除いた場（行優先: [iy·nx + ix]）
    pub u: Vec<C64>,
}

impl Beam {
    /// 関数 f(x, y)（パイロットを除いた場）から作る。w は [`Beam::rms_width`] で決める
    #[allow(clippy::too_many_arguments)]
    pub fn from_fn(
        n: (usize, usize),
        d: (f64, f64),
        c: (f64, f64),
        wl: f64,
        index: f64,
        pilot: Pilot,
        f: impl Fn(f64, f64) -> C64,
    ) -> Beam {
        let mut b = Beam {
            nx: n.0,
            ny: n.1,
            dx: d.0,
            dy: d.1,
            x0: c.0,
            y0: c.1,
            wl,
            index,
            pilot,
            w: 0.0,
            dir: 1.0,
            u: Vec::new(),
        };
        b.u = (0..b.ny)
            .flat_map(|j| (0..b.nx).map(move |i| (i, j)))
            .map(|(i, j)| f(b.x(i), b.y(j)))
            .collect();
        b.w = b.rms_width();
        b
    }

    pub fn x(&self, i: usize) -> f64 {
        self.x0 + (i as f64 - (self.nx as f64 - 1.0) / 2.0) * self.dx
    }
    pub fn y(&self, j: usize) -> f64 {
        self.y0 + (j as f64 - (self.ny as f64 - 1.0) / 2.0) * self.dy
    }
    /// 媒質の中の波数 [1/mm]
    pub fn k(&self) -> f64 {
        2.0 * PI * self.index / self.wl
    }
    /// パイロットの位相 [rad]
    pub fn pilot_phase(&self, x: f64, y: f64) -> f64 {
        let (xx, yy) = (x - self.x0, y - self.y0);
        let p = self.pilot;
        self.k() * (p.ax * xx + p.ay * yy + 0.5 * p.curv * (xx * xx + yy * yy))
    }
    /// 全体の場（パイロット込み）
    pub fn field(&self, i: usize, j: usize) -> C64 {
        self.u[j * self.nx + i] * C64::from_polar(1.0, self.pilot_phase(self.x(i), self.y(j)))
    }
    /// 照度 |u|²（行優先）
    pub fn irradiance(&self) -> Vec<f64> {
        self.u.iter().map(|c| c.norm_sqr()).collect()
    }
    /// 全パワー Σ|u|² dx dy
    pub fn power(&self) -> f64 {
        self.u.iter().map(|c| c.norm_sqr()).sum::<f64>() * self.dx * self.dy
    }
    /// パワーを p にする
    pub fn normalize(&mut self, p: f64) {
        let q = self.power();
        if q > 0.0 {
            let s = (p / q).sqrt();
            self.u.iter_mut().for_each(|c| *c *= s);
        }
    }
    /// 照度の重心
    pub fn centroid(&self) -> (f64, f64) {
        let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
        for j in 0..self.ny {
            for i in 0..self.nx {
                let v = self.u[j * self.nx + i].norm_sqr();
                sx += v * self.x(i);
                sy += v * self.y(j);
                s += v;
            }
        }
        if s > 0.0 {
            (sx / s, sy / s)
        } else {
            (self.x0, self.y0)
        }
    }
    /// 重心からの半径の 2 次モーメントの √2 倍（一様な円形なら半径、Gauss なら 1/e² 半径）
    pub fn rms_width(&self) -> f64 {
        let (cx, cy) = self.centroid();
        let (mut s2, mut s) = (0.0, 0.0);
        for j in 0..self.ny {
            for i in 0..self.nx {
                let v = self.u[j * self.nx + i].norm_sqr();
                let (x, y) = (self.x(i) - cx, self.y(j) - cy);
                s2 += v * (x * x + y * y);
                s += v;
            }
        }
        if s > 0.0 {
            (2.0 * s2 / s).sqrt()
        } else {
            0.0
        }
    }
    /// 照度の最大値
    pub fn peak(&self) -> f64 {
        self.u.iter().map(|c| c.norm_sqr()).fold(0.0, f64::max)
    }
    /// 格子の隣り合う標本の、パイロットを除いた位相の差の最大 [rad]（照度が最大の 1e-3 倍以上の所だけ）。
    /// π に近いと標本化が足りない
    pub fn phase_step(&self) -> f64 {
        let th = 1e-3 * self.peak();
        let mut m: f64 = 0.0;
        let ok = |c: C64| c.norm_sqr() > th;
        for j in 0..self.ny {
            for i in 0..self.nx {
                let a = self.u[j * self.nx + i];
                if !ok(a) {
                    continue;
                }
                if i + 1 < self.nx {
                    let b = self.u[j * self.nx + i + 1];
                    if ok(b) {
                        m = m.max((b * a.conj()).arg().abs());
                    }
                }
                if j + 1 < self.ny {
                    let b = self.u[(j + 1) * self.nx + i];
                    if ok(b) {
                        m = m.max((b * a.conj()).arg().abs());
                    }
                }
            }
        }
        m
    }

    /// 開口を掛ける（開口の外は 0。縁の画素は ss × ss の副標本で面積の割合を掛ける）
    pub fn apply_aperture(&mut self, ap: Aperture, ss: usize) {
        let ss = ss.max(1);
        let (nx, dx, dy) = (self.nx, self.dx, self.dy);
        let xs: Vec<f64> = (0..self.nx).map(|i| self.x(i)).collect();
        let ys: Vec<f64> = (0..self.ny).map(|j| self.y(j)).collect();
        crate::par::for_each_chunk_mut(&mut self.u[..], nx, |j, row| {
            for (i, c) in row.iter_mut().enumerate() {
                let f = coverage(ap, xs[i], ys[j], dx, dy, ss);
                *c *= f;
            }
        });
    }
}

/// 点 (x, y) の画素（幅 dx, dy）のうち開口の中の割合
fn coverage(ap: Aperture, x: f64, y: f64, dx: f64, dy: f64, ss: usize) -> f64 {
    let inside = |x: f64, y: f64| !crate::trace::aperture_out(ap, [x, y, 0.0]);
    // 画素の四隅と中心が同じなら、その値
    let h = (0.5 * dx, 0.5 * dy);
    let c = inside(x, y);
    let all = [
        inside(x - h.0, y - h.1),
        inside(x + h.0, y - h.1),
        inside(x - h.0, y + h.1),
        inside(x + h.0, y + h.1),
    ];
    if all.iter().all(|&v| v == c) && ss <= 1 {
        return if c { 1.0 } else { 0.0 };
    }
    if all.iter().all(|&v| v == c) {
        // 縁が画素の中を横切らないかを粗く見る（画素より小さい開口・遮蔽は副標本で拾う）
        let mid = [
            inside(x - h.0, y),
            inside(x + h.0, y),
            inside(x, y - h.1),
            inside(x, y + h.1),
        ];
        if mid.iter().all(|&v| v == c) {
            return if c { 1.0 } else { 0.0 };
        }
    }
    let mut n = 0usize;
    for a in 0..ss {
        for b in 0..ss {
            let xx = x + ((a as f64 + 0.5) / ss as f64 - 0.5) * dx;
            let yy = y + ((b as f64 + 0.5) / ss as f64 - 0.5) * dy;
            n += inside(xx, yy) as usize;
        }
    }
    n as f64 / (ss * ss) as f64
}

/// 伝搬の方法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopMethod {
    /// Fresnel の積分の核が標本化できれば Fresnel（Fresnel 近似の誤差が 0.01 rad を超え、重すぎなければ
    /// Rayleigh-Sommerfeld）、できなければ（近い距離）角スペクトル
    #[default]
    Auto,
    /// Fresnel 近似の Huygens・Fresnel 積分（x・y に分かれる核。出力の格子は自由。遠い距離・集光向き）
    Fresnel,
    /// 角スペクトル（FFT。近い距離向き）。パイロットに曲率があれば、その球面に沿った座標で近軸の
    /// 伝達関数、曲率がなければ近似なしの伝達関数。出力の格子は入力の格子を (1 + κd) 倍したもの
    /// （出力の格子を指定すると、そこへ双線形で補間する）
    Angular,
    /// Rayleigh-Sommerfeld の第 1 種の積分（近似なし、遅い）
    Direct,
}

/// 出力の格子
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OutGrid {
    /// 標本の数（None なら入力と同じ）
    pub n: Option<(usize, usize)>,
    /// 半幅 [mm]（None なら自動: 幾何的な広がり + 回折の広がり）
    pub half: Option<(f64, f64)>,
    /// 中心（None なら入力の中心からパイロットの傾きの向きに進めた点）
    pub center: Option<(f64, f64)>,
}

/// 伝搬の結果
#[derive(Debug, Clone)]
pub struct Prop {
    pub beam: Beam,
    /// 積分の核の位相の、入力の隣り合う標本の間の差の最大 [rad]。π を超えると折り返しが出る。
    /// 精度は（標本の中で位相が回る分）おおむね kernel_step² に比例する（1e-3 の精度には 0.3 rad 程度）
    pub kernel_step: f64,
    /// Fresnel 近似で落とした位相の項 k ρ⁴/(8 d³) の最大 [rad]（ρ は入力と出力の点の横のずれ）。
    /// [`PopMethod::Direct`] と、近似なしの角スペクトルでは 0。0.01 rad を超えると照度が % 程度ずれうるので Direct を使う（速い集光・ピントずれ）。
    /// 近軸の角スペクトルでは伝達関数の近軸近似の誤差（場のある周波数で）
    pub fresnel_error: f64,
    /// 使った方法（Auto なら選んだもの）
    pub method: PopMethod,
}

/// 一様な媒質の中を、伝搬の向きに距離 d [mm] だけ進めた平行な平面の場
pub fn propagate(b: &Beam, d: f64, method: PopMethod, out: OutGrid) -> Result<Prop> {
    if !(d.is_finite() && d != 0.0) {
        invalid!("伝搬の距離 {d}");
    }
    match method {
        PopMethod::Auto => {
            let p = integral(b, d, PopMethod::Fresnel, out, true)?;
            let m = 1.0 + b.pilot.curv * d;
            // 近い距離は角スペクトル。ただし角スペクトルの方が標本化が粗ければ（遠い距離で広げ切れない）積分
            if p.kernel_step > AUTO_STEP && m.abs() >= 0.25 {
                let a = angular(b, d, out)?;
                if a.kernel_step <= p.kernel_step {
                    return Ok(a);
                }
            }
            // Fresnel 近似が効かない（速い集光・ピントずれ）なら、重すぎなければ Rayleigh-Sommerfeld
            let nz = b.u.iter().filter(|c| c.norm_sqr() > 0.0).count();
            let cost = nz as f64 * (p.beam.nx * p.beam.ny) as f64;
            if p.fresnel_error > AUTO_FRESNEL_ERROR && d > 0.0 && cost <= AUTO_DIRECT_COST {
                integral(b, d, PopMethod::Direct, out, false)
            } else {
                integral(b, d, PopMethod::Fresnel, out, false)
            }
        }
        PopMethod::Angular => angular(b, d, out),
        m => integral(b, d, m, out, false),
    }
}

/// Auto で Fresnel の積分を選ぶ核の位相の差の上限 [rad]
const AUTO_STEP: f64 = 0.5;
/// Auto で Fresnel 近似をやめる、落とす位相の項の大きさ [rad]
const AUTO_FRESNEL_ERROR: f64 = 0.01;
/// Auto で Rayleigh-Sommerfeld の積分を使う重さの上限（入力の標本 × 出力の標本）
const AUTO_DIRECT_COST: f64 = 2e9;

/// Fresnel・Rayleigh-Sommerfeld の積分（dry なら格子と診断だけ）
fn integral(b: &Beam, d: f64, method: PopMethod, out: OutGrid, dry: bool) -> Result<Prop> {
    if method == PopMethod::Direct && d < 0.0 {
        invalid!("Rayleigh-Sommerfeld の積分は前向き（d > 0）だけ");
    }
    let lam = b.wl / b.index;
    let k = b.k();
    let p = b.pilot;
    // 出力のパイロット（Gauss ビームの q を進める）
    let w = if b.w > 0.0 {
        b.w
    } else {
        b.rms_width().max(b.dx)
    };
    let inv_q1 = C64::new(p.curv, -lam / (PI * w * w));
    let q2 = C64::new(1.0, 0.0) / inv_q1 + d;
    let inv_q2 = C64::new(1.0, 0.0) / q2;
    let curv2 = inv_q2.re;
    let w2 = (-lam / (PI * inv_q2.im)).sqrt();
    // 出力の中心: 傾きの向きに進める
    let cz = (1.0 - p.ax * p.ax - p.ay * p.ay).max(1e-12).sqrt();
    let g = (b.x0 + d * p.ax / cz, b.y0 + d * p.ay / cz);
    let (cx, cy) = out.center.unwrap_or(g);
    let (nx2, ny2) = out.n.unwrap_or((b.nx, b.ny));
    if nx2 == 0 || ny2 == 0 {
        invalid!("出力の格子の標本が 0");
    }
    let half = out.half.unwrap_or_else(|| {
        // 入力の格子の中で場のある範囲から
        let (ex, ey) = extent(b);
        let f = (1.0 + p.curv * d).abs();
        let gx = ex * f + 8.0 * lam * d.abs() / ex.max(1e-12);
        let gy = ey * f + 8.0 * lam * d.abs() / ey.max(1e-12);
        (gx.max(4.0 * lam), gy.max(4.0 * lam))
    });
    let dx2 = 2.0 * half.0 / nx2 as f64;
    let dy2 = 2.0 * half.1 / ny2 as f64;
    let mut o = Beam {
        nx: nx2,
        ny: ny2,
        dx: dx2,
        dy: dy2,
        x0: cx,
        y0: cy,
        wl: b.wl,
        index: b.index,
        // 幾何的な中心で傾き a、そこからずれた中心ではその点での傾き
        pilot: Pilot {
            ax: p.ax + curv2 * (cx - g.0),
            ay: p.ay + curv2 * (cy - g.1),
            curv: curv2,
        },
        w: w2,
        dir: b.dir,
        u: vec![C64::new(0.0, 0.0); nx2 * ny2],
    };

    // 核の位相の傾き k (a + κ X1 + (x1 − x2)/d) の最大 × 入力の間隔（x・y で線形なので隅で最大）
    let step = |a: f64, x0: f64, xs1: (f64, f64), xs2: (f64, f64), dx1: f64| {
        let mut m: f64 = 0.0;
        for x1 in [xs1.0, xs1.1] {
            for x2 in [xs2.0, xs2.1] {
                m = m.max((k * (a + p.curv * (x1 - x0) + (x1 - x2) / d)).abs());
            }
        }
        m * dx1
    };
    let (sx, sy) = support(b);
    let kernel_step = step(p.ax, b.x0, sx, (o.x(0), o.x(nx2 - 1)), b.dx).max(step(
        p.ay,
        b.y0,
        sy,
        (o.y(0), o.y(ny2 - 1)),
        b.dy,
    ));

    let fresnel_error = match method {
        PopMethod::Fresnel => {
            // 場のある標本と出力の窓の四隅の距離の最大
            let th = 1e-6 * b.peak();
            let corners = [
                (o.x(0), o.y(0)),
                (o.x(nx2 - 1), o.y(0)),
                (o.x(0), o.y(ny2 - 1)),
                (o.x(nx2 - 1), o.y(ny2 - 1)),
            ];
            let mut r2: f64 = 0.0;
            for jj in 0..b.ny {
                for ii in 0..b.nx {
                    if b.u[jj * b.nx + ii].norm_sqr() > th {
                        let (x, y) = (b.x(ii), b.y(jj));
                        for c in corners {
                            r2 = r2.max((x - c.0).powi(2) + (y - c.1).powi(2));
                        }
                    }
                }
            }
            k * r2 * r2 / (8.0 * d.abs().powi(3))
        }
        _ => 0.0,
    };
    if dry {
        return Ok(Prop {
            beam: o,
            kernel_step,
            fresnel_error,
            method,
        });
    }
    match method {
        PopMethod::Direct => direct(b, d, &mut o),
        _ => fresnel(b, d, &mut o),
    }
    Ok(Prop {
        beam: o,
        kernel_step,
        fresnel_error,
        method,
    })
}

/// 角スペクトル。パイロット (a, κ) の場を、M = 1 + κd、d′ = d/M として
/// U₂(X₂) = (1/M) F_{d′}[u](ξ)、ξ = (X₂ − a d)/M（近軸。出力のパイロットは傾き a・曲率 κ/M、中心 x0 + a d）。
/// κ = 0 なら近似なしの伝達関数 exp(i 2π d (w(f + a/λ′) − w(a/λ′)))、w(f) = √(1/λ′² − f²)
/// （出力の中心は x0 + d tan θ）。格子は 0 で広げて（回折で広がる幅、2〜8 倍）回り込みを防ぎ、広げた格子のまま返す
fn angular(b: &Beam, d: f64, out: OutGrid) -> Result<Prop> {
    use rustfft::FftPlanner;
    let lam = b.wl / b.index;
    let k = b.k();
    let p = b.pilot;
    let m = 1.0 + p.curv * d;
    if m.abs() < 1e-3 {
        invalid!("角スペクトル: 焦点の面（1 + κd = {m}）には Fresnel の積分を使う");
    }
    let exact = p.curv == 0.0;
    let dp = d / m;
    let (nx, ny) = (b.nx, b.ny);
    // 広げる幅: 最も高い周波数（1/(2Δ)）の光が横に進む |d′| λ′/(2Δ) を両側に（2 倍〜8 倍）
    let pad = |n: usize, dd: f64| {
        let need = n + (dp.abs() * lam / (dd * dd)).ceil() as usize;
        need.max(2 * n).min(8 * n).next_power_of_two()
    };
    let (px, py) = (pad(nx, b.dx), pad(ny, b.dy));
    // 0 で広げる（元の格子は中央）
    let (ox, oy) = ((px - nx) / 2, (py - ny) / 2);
    let mut a = vec![C64::new(0.0, 0.0); px * py];
    for j in 0..ny {
        a[(j + oy) * px + ox..(j + oy) * px + ox + nx].copy_from_slice(&b.u[j * nx..(j + 1) * nx]);
    }
    let mut planner = FftPlanner::<f64>::new();
    let fx = planner.plan_fft_forward(px);
    let fy = planner.plan_fft_forward(py);
    let ix = planner.plan_fft_inverse(px);
    let iy = planner.plan_fft_inverse(py);
    fft2(&mut a, px, py, &*fx, &*fy);
    // 出力の中心のずれ（入力の中心から）
    let (sx, sy) = if exact {
        let cz = (1.0 - p.ax * p.ax - p.ay * p.ay).max(1e-12).sqrt();
        (d * p.ax / cz, d * p.ay / cz)
    } else {
        (p.ax * d, p.ay * d)
    };
    let (dfx, dfy) = (1.0 / (px as f64 * b.dx), 1.0 / (py as f64 * b.dy));
    let freq = |i: usize, n: usize, df: f64| {
        let ii = if i < n.div_ceil(2) {
            i as f64
        } else {
            i as f64 - n as f64
        };
        ii * df
    };
    let w = |f2: f64| (1.0 / (lam * lam) - f2).sqrt();
    let (fax, fay) = (p.ax / lam, p.ay / lam);
    let w0 = w(fax * fax + fay * fay);
    let peak = a.iter().map(|c| c.norm_sqr()).fold(0.0, f64::max);
    let th = 1e-8 * peak;
    let mut ferr: f64 = 0.0;
    for j in 0..py {
        let fy_ = freq(j, py, dfy);
        for i in 0..px {
            let fx_ = freq(i, px, dfx);
            let c = &mut a[j * px + i];
            let ph = if exact {
                let (gx, gy) = (fx_ + fax, fy_ + fay);
                let q = gx * gx + gy * gy;
                if q >= 1.0 / (lam * lam) {
                    *c = C64::new(0.0, 0.0);
                    continue;
                }
                2.0 * PI * (fx_ * sx + fy_ * sy + d * (w(q) - w0))
            } else {
                let f2 = fx_ * fx_ + fy_ * fy_;
                if c.norm_sqr() > th {
                    // 落とした項 k d′ (√(1 − λ′²f²) − 1 + λ′²f²/2)
                    let t = lam * lam * f2;
                    if t < 1.0 {
                        ferr = ferr.max((k * dp * ((1.0 - t).sqrt() - 1.0 + t / 2.0)).abs());
                    }
                }
                -PI * lam * dp * f2
            };
            *c *= C64::from_polar(1.0, ph);
        }
    }
    ifft2(&mut a, px, py, &*ix, &*iy);
    let norm = 1.0 / (px * py) as f64 / m;
    // 伝達関数の位相の、隣り合う周波数の間の差の最大（π を超えると回り込み）
    let fmx = 0.5 / b.dx;
    let fmy = 0.5 / b.dy;
    let kernel_step = if exact {
        let st = |f: f64, df: f64| {
            let g = (f.abs() + fax.abs().max(fay.abs())).min(0.999 / lam);
            2.0 * PI * d.abs() * g / w(g * g) * df
        };
        st(fmx, dfx).max(st(fmy, dfy))
    } else {
        (2.0 * PI * lam * dp.abs() * fmx * dfx).max(2.0 * PI * lam * dp.abs() * fmy * dfy)
    };
    // 自然な出力の格子（広げた格子の全体、間隔 |M| dx、M < 0 なら向きが反転）
    let (gx0, gy0) = (b.x0 + sx, b.y0 + sy);
    // 広げた格子の中心は元の格子の中心と同じ位置（奇数・偶数のずれを補う）
    let cxo = ((px as f64 - 1.0) / 2.0 - (ox as f64 + (nx as f64 - 1.0) / 2.0)) * b.dx * m;
    let cyo = ((py as f64 - 1.0) / 2.0 - (oy as f64 + (ny as f64 - 1.0) / 2.0)) * b.dy * m;
    let mut o = Beam {
        nx: px,
        ny: py,
        dx: b.dx * m.abs(),
        dy: b.dy * m.abs(),
        x0: gx0 + cxo,
        y0: gy0 + cyo,
        wl: b.wl,
        index: b.index,
        pilot: Pilot {
            ax: p.ax + if exact { 0.0 } else { p.curv / m * cxo },
            ay: p.ay + if exact { 0.0 } else { p.curv / m * cyo },
            curv: if exact { 0.0 } else { p.curv / m },
        },
        w: 0.0,
        dir: b.dir,
        u: vec![C64::new(0.0, 0.0); px * py],
    };
    for j in 0..py {
        for i in 0..px {
            let (si, sj) = if m < 0.0 {
                (px - 1 - i, py - 1 - j)
            } else {
                (i, j)
            };
            o.u[j * px + i] = a[sj * px + si] * norm;
        }
    }
    o.w = o.rms_width();
    let want = out.n.is_some() || out.half.is_some() || out.center.is_some();
    let o = if want { resample(&o, out) } else { o };
    Ok(Prop {
        beam: o,
        kernel_step,
        fresnel_error: ferr,
        method: PopMethod::Angular,
    })
}

fn fft2(
    a: &mut [C64],
    px: usize,
    py: usize,
    fx: &dyn rustfft::Fft<f64>,
    fy: &dyn rustfft::Fft<f64>,
) {
    crate::par::for_each_chunk_mut(&mut a[..], px, |_, r| fx.process(r));
    transpose_apply(a, px, py, fy);
}

fn ifft2(
    a: &mut [C64],
    px: usize,
    py: usize,
    ix: &dyn rustfft::Fft<f64>,
    iy: &dyn rustfft::Fft<f64>,
) {
    crate::par::for_each_chunk_mut(&mut a[..], px, |_, r| ix.process(r));
    transpose_apply(a, px, py, iy);
}

/// 列ごとに f を掛ける（転置して行で）
fn transpose_apply(a: &mut [C64], px: usize, py: usize, f: &dyn rustfft::Fft<f64>) {
    let mut t = vec![C64::new(0.0, 0.0); px * py];
    for j in 0..py {
        for i in 0..px {
            t[i * py + j] = a[j * px + i];
        }
    }
    crate::par::for_each_chunk_mut(&mut t[..], py, |_, r| f.process(r));
    for j in 0..py {
        for i in 0..px {
            a[j * px + i] = t[i * py + j];
        }
    }
}

/// 場を別の格子へ双線形で補間する（パイロットの曲率は同じ、傾きは新しい中心でのもの）
fn resample(b: &Beam, out: OutGrid) -> Beam {
    let (nx, ny) = out.n.unwrap_or((b.nx, b.ny));
    let (cx, cy) = out.center.unwrap_or((b.x0, b.y0));
    let (hx, hy) = out
        .half
        .unwrap_or((b.dx * b.nx as f64 / 2.0, b.dy * b.ny as f64 / 2.0));
    let mut o = Beam {
        nx,
        ny,
        dx: 2.0 * hx / nx as f64,
        dy: 2.0 * hy / ny as f64,
        x0: cx,
        y0: cy,
        pilot: Pilot {
            ax: b.pilot.ax + b.pilot.curv * (cx - b.x0),
            ay: b.pilot.ay + b.pilot.curv * (cy - b.y0),
            curv: b.pilot.curv,
        },
        u: vec![C64::new(0.0, 0.0); nx * ny],
        ..b.clone()
    };
    // パイロットの差（中心を移すと定数だけ違う: 新しい中心で 0 になるよう揃える）
    let c0 = b.pilot_phase(cx, cy);
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = (o.x(i), o.y(j));
            let fi = (x - b.x0) / b.dx + (b.nx as f64 - 1.0) / 2.0;
            let fj = (y - b.y0) / b.dy + (b.ny as f64 - 1.0) / 2.0;
            if fi < 0.0 || fj < 0.0 || fi > (b.nx - 1) as f64 || fj > (b.ny - 1) as f64 {
                continue;
            }
            let (i0, j0) = ((fi as usize).min(b.nx - 2), (fj as usize).min(b.ny - 2));
            let (tx, ty) = (fi - i0 as f64, fj - j0 as f64);
            let g = |ii: usize, jj: usize| b.u[jj * b.nx + ii];
            let v = g(i0, j0) * ((1.0 - tx) * (1.0 - ty))
                + g(i0 + 1, j0) * (tx * (1.0 - ty))
                + g(i0, j0 + 1) * ((1.0 - tx) * ty)
                + g(i0 + 1, j0 + 1) * (tx * ty);
            // 全体の場を保つ: u_old·P_old = u_new·P_new → u_new = u_old·exp(i(P_old − P_new))
            let dph = b.pilot_phase(x, y) - c0 - o.pilot_phase(x, y);
            o.u[j * nx + i] = v * C64::from_polar(1.0, dph);
        }
    }
    o.w = o.rms_width();
    o
}

/// 場のある範囲（照度が最大の 1e-6 倍以上の標本）の x・y の [最小, 最大]
fn support(b: &Beam) -> ((f64, f64), (f64, f64)) {
    let th = 1e-6 * b.peak();
    let (mut x, mut y) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
    for j in 0..b.ny {
        for i in 0..b.nx {
            if b.u[j * b.nx + i].norm_sqr() > th {
                x = (x.0.min(b.x(i)), x.1.max(b.x(i)));
                y = (y.0.min(b.y(j)), y.1.max(b.y(j)));
            }
        }
    }
    if x.0 > x.1 {
        ((b.x0, b.x0), (b.y0, b.y0))
    } else {
        (x, y)
    }
}

/// 場のある範囲の、中心からの半幅（x・y）
fn extent(b: &Beam) -> (f64, f64) {
    let (sx, sy) = support(b);
    let ex = (sx.1 - b.x0).abs().max((sx.0 - b.x0).abs()) + 0.5 * b.dx;
    let ey = (sy.1 - b.y0).abs().max((sy.0 - b.y0).abs()) + 0.5 * b.dy;
    (ex, ey)
}

/// 1 軸の Fresnel の核 K[i2][i1] = exp(i k (a X1 + κ X1²/2 + (x2 − x1)²/(2d) − 出力のパイロット)) · Δx1
#[allow(clippy::too_many_arguments)]
fn kernel_1d(
    k: f64,
    d: f64,
    a1: f64,
    c1: f64,
    x1: &[f64],
    x01: f64,
    a2: f64,
    c2: f64,
    x2: &[f64],
    x02: f64,
    dx1: f64,
) -> Vec<C64> {
    let mut m = vec![C64::new(0.0, 0.0); x1.len() * x2.len()];
    crate::par::for_each_chunk_mut(&mut m[..], x1.len(), |i2, row| {
        let xo = x2[i2] - x02;
        let po = a2 * xo + 0.5 * c2 * xo * xo;
        for (i1, v) in row.iter_mut().enumerate() {
            let xi = x1[i1] - x01;
            let dd = x2[i2] - x1[i1];
            let ph = k * (a1 * xi + 0.5 * c1 * xi * xi + dd * dd / (2.0 * d) - po);
            *v = C64::from_polar(dx1, ph);
        }
    });
    m
}

fn fresnel(b: &Beam, d: f64, o: &mut Beam) {
    let k = b.k();
    let lam = b.wl / b.index;
    let x1: Vec<f64> = (0..b.nx).map(|i| b.x(i)).collect();
    let y1: Vec<f64> = (0..b.ny).map(|j| b.y(j)).collect();
    let x2: Vec<f64> = (0..o.nx).map(|i| o.x(i)).collect();
    let y2: Vec<f64> = (0..o.ny).map(|j| o.y(j)).collect();
    let (p, q) = (b.pilot, o.pilot);
    let kx = kernel_1d(k, d, p.ax, p.curv, &x1, b.x0, q.ax, q.curv, &x2, o.x0, b.dx);
    let ky = kernel_1d(k, d, p.ay, p.curv, &y1, b.y0, q.ay, q.curv, &y2, o.y0, b.dy);
    // 前置きの係数 1/(i λ d)
    let pre = C64::new(0.0, -1.0 / (lam * d));
    // tmp[j1][i2] = Σ_i1 u[j1][i1] Kx[i2][i1]
    let (n1x, n2x) = (b.nx, o.nx);
    let mut tmp = vec![C64::new(0.0, 0.0); b.ny * n2x];
    crate::par::for_each_chunk_mut(&mut tmp[..], n2x, |j1, row| {
        let ur = &b.u[j1 * n1x..(j1 + 1) * n1x];
        if ur.iter().all(|c| c.re == 0.0 && c.im == 0.0) {
            return;
        }
        for (i2, v) in row.iter_mut().enumerate() {
            let kr = &kx[i2 * n1x..(i2 + 1) * n1x];
            let mut s = C64::new(0.0, 0.0);
            for (a, c) in ur.iter().zip(kr) {
                s += a * c;
            }
            *v = s;
        }
    });
    // out[j2][i2] = Σ_j1 Ky[j2][j1] tmp[j1][i2]
    let n1y = b.ny;
    crate::par::for_each_chunk_mut(&mut o.u[..], n2x, |j2, row| {
        let kr = &ky[j2 * n1y..(j2 + 1) * n1y];
        for (j1, c) in kr.iter().enumerate() {
            let tr = &tmp[j1 * n2x..(j1 + 1) * n2x];
            for (v, t) in row.iter_mut().zip(tr) {
                *v += c * t;
            }
        }
        row.iter_mut().for_each(|v| *v *= pre);
    });
}

fn direct(b: &Beam, d: f64, o: &mut Beam) {
    let k = b.k();
    // 入力の 0 でない標本（全体の場）
    let src: Vec<(f64, f64, C64)> = (0..b.ny)
        .flat_map(|j| (0..b.nx).map(move |i| (i, j)))
        .filter(|&(i, j)| b.u[j * b.nx + i].norm_sqr() > 0.0)
        .map(|(i, j)| (b.x(i), b.y(j), b.field(i, j) * (b.dx * b.dy)))
        .collect();
    let (nx, x0, y0, dx, dy, ny) = (o.nx, o.x0, o.y0, o.dx, o.dy, o.ny);
    let pilot = o.pilot;
    let xo = |i: usize| x0 + (i as f64 - (nx as f64 - 1.0) / 2.0) * dx;
    let yo = |j: usize| y0 + (j as f64 - (ny as f64 - 1.0) / 2.0) * dy;
    let d2 = d * d;
    crate::par::for_each_chunk_mut(&mut o.u[..], nx, |j, row| {
        let y = yo(j);
        for (i, v) in row.iter_mut().enumerate() {
            let x = xo(i);
            let (xx, yy) = (x - x0, y - y0);
            let po = k * (pilot.ax * xx + pilot.ay * yy + 0.5 * pilot.curv * (xx * xx + yy * yy));
            let mut s = C64::new(0.0, 0.0);
            for &(sx, sy, c) in &src {
                let (ex, ey) = (x - sx, y - sy);
                let r2 = ex * ex + ey * ey + d2;
                let r = r2.sqrt();
                // (1/2π)(d/r)(1/r − ik) e^{ik(r − d)}/r
                let ph = k * (r - d) - po;
                let e = C64::from_polar(1.0, ph);
                let f = C64::new(1.0 / r, -k) * (d / (r * r));
                s += c * e * f;
            }
            *v = s / (2.0 * PI);
        }
    });
}

/// 光線から場を作るときの設定
#[derive(Debug, Clone, Copy)]
pub struct RayBeamOpts {
    /// 瞳の格子の一辺の光線の数
    pub rays: usize,
    /// 場の格子の一辺の標本の数
    pub n: usize,
    /// 場の格子の半幅 [mm]（None なら光線の届く範囲と開口から）
    pub half: Option<f64>,
    /// 面の開口を格子の上で掛ける（絞り面で開口がなければ絞り半径の円）。このとき瞳を少し広げて光線を追う
    pub aperture: bool,
    /// 瞳の広げ方（外の縁のある開口を掛けるときの既定 1.05、遮蔽・スパイダー・開口なしは 1）
    pub overfill: Option<f64>,
}

impl Default for RayBeamOpts {
    fn default() -> Self {
        RayBeamOpts {
            rays: 65,
            n: 256,
            half: None,
            aperture: true,
            overfill: None,
        }
    }
}

/// 光線から作った場
#[derive(Debug, Clone)]
pub struct RayBeam {
    pub beam: Beam,
    /// 使えた光線の数 / 追跡した光線の数
    pub used: usize,
    pub traced: usize,
    /// 掛けた開口（なければ None）
    pub aperture: Option<Aperture>,
    /// 格子の上のパワー（開口を掛けた後。正規化瞳の単位円の中が 1）
    pub transmitted: f64,
}

/// 正方形 [-1, 1]² → 単位円（境界が円になる滑らかな写像）
fn square_to_disk(u: f64, v: f64) -> (f64, f64) {
    (
        u * (1.0 - v * v / 2.0).sqrt(),
        v * (1.0 - u * u / 2.0).sqrt(),
    )
}

/// 視野 field・波長 w の光線を面 j まで追跡し、面 j の接平面の上の場を作る（面 j の後の媒質の中）
pub fn beam_from_rays(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    j: usize,
    o: &RayBeamOpts,
) -> Result<RayBeam> {
    let w = w.unwrap_or(s.primary);
    if j == 0 || j > s.image() {
        invalid!("面 {j}（1〜像面）");
    }
    if s.surfaces[j].ignore {
        invalid!("面 {j} は無視する面");
    }
    let nr = o.rays.max(5);
    let ap = if o.aperture {
        surface_aperture(s, j)
    } else {
        None
    };
    // 瞳を広げるのは外の縁のある開口（円・矩形・楕円）だけ。遮蔽・スパイダーは外の縁が無いので、光の外の縁は
    // 開口の無いときと同じ瞳の縁（絞り面なら絞りの円。公開 #62: 広げると光が絞り半径の外まで通っていた）
    let rim = ap.is_some_and(|a| aperture_reach(a, 0.0, 0.0) > 0.0);
    let of = o.overfill.unwrap_or(if rim { 1.05 } else { 1.0 });
    let mut px = Vec::with_capacity(nr * nr);
    let mut py = Vec::with_capacity(nr * nr);
    let g = |i: usize| -1.0 + 2.0 * i as f64 / (nr - 1) as f64;
    for b in 0..nr {
        for a in 0..nr {
            let (x, y) = square_to_disk(g(a), g(b));
            px.push(of * x);
            py.push(of * y);
        }
    }
    let t = s.try_trace(field, &px, &py, Some(w), Some(j), false)?;
    let n = s.index_before(j + 1, w);
    // 面 j の接平面へ移す
    let mut ok = vec![false; nr * nr];
    let mut pos = vec![(0.0, 0.0); nr * nr];
    let mut opl = vec![0.0; nr * nr];
    let mut kz = 0.0;
    for (i, r) in t.rays.iter().enumerate() {
        let good = r.err == 0 && (r.vig == 0 || (ap.is_some() && r.vig == j));
        if !good || r.lmn[2].abs() < 1e-9 {
            continue;
        }
        let tt = -r.xyz[2] / r.lmn[2];
        pos[i] = (r.xyz[0] + r.lmn[0] * tt, r.xyz[1] + r.lmn[1] * tt);
        opl[i] = r.opl + r.lens_opd + n * tt;
        ok[i] = true;
        kz += r.lmn[2];
    }
    let used = ok.iter().filter(|&&v| v).count();
    if used < 10 {
        invalid!("面 {j} に届く光線が少ない（{used} 本）");
    }
    let dir = kz.signum();
    // 中心: 主光線（瞳の中心）、届かなければ重心
    let ic = (nr / 2) * nr + nr / 2;
    let (xc, yc) = if nr % 2 == 1 && ok[ic] {
        pos[ic]
    } else {
        let (mut sx, mut sy) = (0.0, 0.0);
        for i in 0..nr * nr {
            if ok[i] {
                sx += pos[i].0;
                sy += pos[i].1;
            }
        }
        (sx / used as f64, sy / used as f64)
    };
    // パイロット: 光路長 ≈ c0 + n (a_x X + a_y Y + κ (X² + Y²)/2)
    let mut rows = Vec::with_capacity(used);
    let mut rhs = Vec::with_capacity(used);
    for i in 0..nr * nr {
        if ok[i] {
            let (x, y) = (pos[i].0 - xc, pos[i].1 - yc);
            rows.push(vec![1.0, x, y, x * x + y * y]);
            rhs.push(opl[i]);
        }
    }
    let c = crate::analysis::lstsq(&rows, &rhs);
    let pilot = Pilot {
        ax: c[1] / n,
        ay: c[2] / n,
        curv: 2.0 * c[3] / n,
    };
    let res: Vec<f64> = (0..nr * nr)
        .map(|i| {
            let (x, y) = (pos[i].0 - xc, pos[i].1 - yc);
            opl[i] - (c[0] + c[1] * x + c[2] * y + c[3] * (x * x + y * y))
        })
        .collect();
    // 照度 = (1/π) 瞳の面積 / 平面の面積（格子の差分のヤコビアン。正規化瞳の単位円の中がパワー 1）
    let jac = |f: &dyn Fn(usize) -> (f64, f64), a: usize, b: usize| -> Option<f64> {
        let id = |a: usize, b: usize| b * nr + a;
        let (a0, a1) = if a == 0 {
            (0, 1)
        } else if a == nr - 1 {
            (nr - 2, nr - 1)
        } else {
            (a - 1, a + 1)
        };
        let (b0, b1) = if b == 0 {
            (0, 1)
        } else if b == nr - 1 {
            (nr - 2, nr - 1)
        } else {
            (b - 1, b + 1)
        };
        for q in [id(a0, b), id(a1, b), id(a, b0), id(a, b1)] {
            if !ok[q] {
                return None;
            }
        }
        let (pa0, pa1, pb0, pb1) = (f(id(a0, b)), f(id(a1, b)), f(id(a, b0)), f(id(a, b1)));
        let du = ((pa1.0 - pa0.0), (pa1.1 - pa0.1));
        let dv = ((pb1.0 - pb0.0), (pb1.1 - pb0.1));
        Some((du.0 * dv.1 - du.1 * dv.0).abs() / ((a1 - a0) * (b1 - b0)) as f64)
    };
    let fp = |i: usize| (px[i], py[i]);
    let fq = |i: usize| pos[i];
    let mut amp = vec![0.0; nr * nr];
    for b in 0..nr {
        for a in 0..nr {
            let i = b * nr + a;
            if !ok[i] {
                continue;
            }
            // 端の光線は内側の隣で代用
            let (aa, bb) = (a, b);
            let v = jac(&fp, aa, bb).zip(jac(&fq, aa, bb));
            amp[i] = match v {
                Some((jp, jq)) if jq > 0.0 => (jp / (jq * PI)).sqrt(),
                _ => f64::NAN,
            };
        }
    }
    // 求まらなかった点（縁の隣）は隣の平均
    for b in 0..nr {
        for a in 0..nr {
            let i = b * nr + a;
            if ok[i] && amp[i].is_nan() {
                let mut s = 0.0;
                let mut m = 0;
                for (da, db) in [
                    (-1i64, 0i64),
                    (1, 0),
                    (0, -1),
                    (0, 1),
                    (-1, -1),
                    (1, 1),
                    (-1, 1),
                    (1, -1),
                ] {
                    let (x, y) = (a as i64 + da, b as i64 + db);
                    if x >= 0 && y >= 0 && (x as usize) < nr && (y as usize) < nr {
                        let q = y as usize * nr + x as usize;
                        if ok[q] && amp[q].is_finite() {
                            s += amp[q];
                            m += 1;
                        }
                    }
                }
                amp[i] = if m > 0 { s / m as f64 } else { 0.0 };
            }
        }
    }
    // 格子
    let half = match o.half {
        Some(h) => h,
        None => {
            let mut m: f64 = 0.0;
            for i in 0..nr * nr {
                if ok[i] {
                    m = m.max((pos[i].0 - xc).abs()).max((pos[i].1 - yc).abs());
                }
            }
            if let Some(a) = ap {
                m = m.max(aperture_reach(a, xc, yc));
            }
            m * 1.02
        }
    };
    let ng = o.n.max(4);
    let dg = 2.0 * half / ng as f64;
    let mut beam = Beam {
        nx: ng,
        ny: ng,
        dx: dg,
        dy: dg,
        x0: xc,
        y0: yc,
        wl: s.wavelengths[w] * 1e-3,
        index: n,
        pilot,
        w: 0.0,
        dir,
        u: vec![C64::new(0.0, 0.0); ng * ng],
    };
    let k0 = 2.0 * PI / beam.wl;
    // 三角形ごとに格子の点を埋める（重心座標で振幅と残りの光路長を補間）
    let gx = |x: f64| (x - xc) / dg + (ng as f64 - 1.0) / 2.0;
    let gy = |y: f64| (y - yc) / dg + (ng as f64 - 1.0) / 2.0;
    for b in 0..nr - 1 {
        for a in 0..nr - 1 {
            let q = [
                b * nr + a,
                b * nr + a + 1,
                (b + 1) * nr + a + 1,
                (b + 1) * nr + a,
            ];
            for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                if !tri.iter().all(|&i| ok[i]) {
                    continue;
                }
                let p: Vec<(f64, f64)> =
                    tri.iter().map(|&i| (gx(pos[i].0), gy(pos[i].1))).collect();
                let det =
                    (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
                if det.abs() < 1e-300 {
                    continue;
                }
                let lo_x = p
                    .iter()
                    .map(|v| v.0)
                    .fold(f64::MAX, f64::min)
                    .ceil()
                    .max(0.0) as usize;
                let hi_x = p.iter().map(|v| v.0).fold(f64::MIN, f64::max).floor();
                let lo_y = p
                    .iter()
                    .map(|v| v.1)
                    .fold(f64::MAX, f64::min)
                    .ceil()
                    .max(0.0) as usize;
                let hi_y = p.iter().map(|v| v.1).fold(f64::MIN, f64::max).floor();
                if hi_x < 0.0 || hi_y < 0.0 {
                    continue;
                }
                let hi_x = (hi_x as usize).min(ng - 1);
                let hi_y = (hi_y as usize).min(ng - 1);
                for iy in lo_y..=hi_y {
                    for ix in lo_x..=hi_x {
                        let (x, y) = (ix as f64, iy as f64);
                        let l1 = ((x - p[0].0) * (p[2].1 - p[0].1)
                            - (p[2].0 - p[0].0) * (y - p[0].1))
                            / det;
                        let l2 = ((p[1].0 - p[0].0) * (y - p[0].1)
                            - (x - p[0].0) * (p[1].1 - p[0].1))
                            / det;
                        let l0 = 1.0 - l1 - l2;
                        let e = -1e-9;
                        if l0 < e || l1 < e || l2 < e {
                            continue;
                        }
                        let am = l0 * amp[tri[0]] + l1 * amp[tri[1]] + l2 * amp[tri[2]];
                        let rr = l0 * res[tri[0]] + l1 * res[tri[1]] + l2 * res[tri[2]];
                        beam.u[iy * ng + ix] = C64::from_polar(am, k0 * rr);
                    }
                }
            }
        }
    }
    beam.w = beam.rms_width();
    if let Some(a) = ap {
        beam.apply_aperture(a, 8);
    }
    let transmitted = beam.power();
    Ok(RayBeam {
        beam,
        used,
        traced: nr * nr,
        aperture: ap,
        transmitted,
    })
}

/// 面 j の開口（浮動の開口は除く。絞り面で開口がなければ絞り半径の円）
pub fn surface_aperture(s: &System, j: usize) -> Option<Aperture> {
    match s.surfaces[j].ap {
        Aperture::None | Aperture::Floating { .. } => {
            if j == s.stop() {
                let (rx, ry) = s.stop_radius();
                let r = rx.abs().max(ry.abs());
                (r > 0.0 && r.is_finite()).then_some(Aperture::Circular {
                    min: 0.0,
                    max: r,
                    dx: 0.0,
                    dy: 0.0,
                })
            } else {
                None
            }
        }
        a => Some(a),
    }
}

/// 開口の通す範囲の、点 (xc, yc) からの x・y の最大のずれ（遮蔽は 0）
fn aperture_reach(a: Aperture, xc: f64, yc: f64) -> f64 {
    let f = |hx: f64, hy: f64, dx: f64, dy: f64| ((dx - xc).abs() + hx).max((dy - yc).abs() + hy);
    match a {
        Aperture::Circular { max, dx, dy, .. } => f(max, max, dx, dy),
        Aperture::Rect { hx, hy, dx, dy } | Aperture::Ellipse { hx, hy, dx, dy } => {
            f(hx, hy, dx, dy)
        }
        _ => 0.0,
    }
}

/// 面 j の後の空間を、次の面（間はすべて平面のダミー面で媒質が同じ）の頂点面まで進む距離（伝搬の向きに測る）
pub fn distance_to(s: &System, j: usize, to: usize, dir: f64) -> Result<f64> {
    if to <= j || to > s.image() {
        invalid!("面 {to} は面 {j} より後の面（〜像面）");
    }
    let mut t = 0.0;
    for q in j..to {
        let d = &s.surfaces[q];
        if q > j && !d.ignore {
            let plain = d.typ == crate::surface::SurfType::Standard
                && d.c == 0.0
                && !d.mirror
                && d.before.is_none()
                && d.mat == s.surfaces[j].mat;
            if !plain {
                notimpl!("面 {j} と面 {to} の間の面 {q}（間は平面のダミー面だけ）");
            }
        }
        if !d.ignore {
            t += d.t;
        }
    }
    let dd = t * dir;
    if !(dd > 0.0) {
        invalid!("面 {to} は伝搬の向きの後ろにない（距離 {t}）");
    }
    Ok(dd)
}
