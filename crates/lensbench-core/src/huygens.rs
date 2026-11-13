//! Huygens PSF・Huygens MTF（OpticStudio の Huygens PSF・Huygens MTF 相当）。
//!
//! FFT の PSF（[`crate::psf::psf`]）は瞳から主光線に垂直な面への写像を一次（対角）で近似するが、Huygens は
//! 射出瞳の参照球（OPD の基準と同じ。ROPD「無限遠」・射出瞳が無限遠なら主光線に垂直な平面）の上の各点を
//! 2 次波源とし、像面（傾き・曲率を含む像面そのもの）の格子の各点まで素直に足し合わせる:
//!
//! U(P) = Σ_i f_i exp(2πi (−W_i + n σ (|P − Q_i| − R) / λ)) · (R / |P − Q_i|) · (1 + cos χ_i) / 2
//!
//! Q_i は光線 i を像面から参照球まで戻した点、W_i はその OPD [波]、f_i は瞳の区間の単位円内の面積の割合
//! （FFT PSF と同じ正規化瞳の格子）、R は参照球の半径、σ は参照球が像点の前なら +1・後ろなら −1、χ_i は
//! Q_i → P と Q_i → 像点の向きの角。平面の基準では U(P) = Σ f_i exp(2πi (−W_i + n k_i · (P − Q_i) / λ))。
//! 瞳から像への写像を近似しないので、像面が傾いた系・収差が大きい系・斜めの視野でも像面の上の PSF が出る。
//!
//! 正規化は FFT PSF と同じ: 各波長で収差のない同じ瞳の中心の値を 1 とし、多色は各波長の全強度を重みに
//! 比例させる（重みは FFT PSF と同じく Parseval の全強度 Σf / |a b h²|）。
//! Huygens MTF は Huygens PSF の像面の格子の上の離散フーリエ変換の絶対値（DC で 1）。像の格子が PSF を
//! 覆いきれないと MTF は低周波で下がり、格子の間隔が粗いと高周波で折り返すので、格子は利用者が決める。

use crate::analysis::{opd, pupil_jacobian, pupil_shift, Mtf};
use crate::coating::C64;
use crate::error::Result;
use crate::math::*;
use crate::psf::{cell_frac, perp_slopes_of, Psf};
use crate::system::{FieldSel, System};
use crate::{invalid, notimpl};
use std::f64::consts::PI;

/// Huygens PSF・MTF の設定
#[derive(Debug, Clone, Copy)]
pub struct HuygensOpts {
    /// 瞳の格子の一辺（直径を n 区間に分けた各区間の中心で標本化）。None で自動（隣り合う標本の位相差が π/2 以下に
    /// なる大きさ、64〜512。計算量は瞳の格子² × 像の格子²）
    pub pupil: Option<usize>,
    /// 像の格子の一辺
    pub image: usize,
    /// 像の格子の間隔 [レンズ単位]（像面の上で。None で FFT PSF の既定と同じ 1 / (2 × 遮断周波数)、x・y 別）
    pub delta: Option<f64>,
}

impl Default for HuygensOpts {
    fn default() -> Self {
        HuygensOpts {
            pupil: None,
            image: 128,
            delta: None,
        }
    }
}

/// 参照球の 2 次波の振幅と位相（成分ごとの配列）
struct SphereTerms<'a> {
    q: [&'a [f64]; 3],
    /// 光源から球の中心への向き
    a: [&'a [f64]; 3],
    f: &'a [f64],
    ph0: &'a [f64],
    rr: f64,
    /// k·σ
    ksg: f64,
}

impl SphereTerms<'_> {
    /// 像の点 p への各光源の振幅 amp と位相 ph
    fn eval(&self, p: &V3, ph: &mut [f64], amp: &mut [f64]) {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx") {
                // 安全: AVX があることを上で確かめた（演算は同じなので結果は同じビット）
                return unsafe { self.eval_avx(p, ph, amp) };
            }
        }
        self.eval_impl(p, ph, amp)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx")]
    unsafe fn eval_avx(&self, p: &V3, ph: &mut [f64], amp: &mut [f64]) {
        self.eval_impl(p, ph, amp)
    }

    #[inline(always)]
    fn eval_impl(&self, p: &V3, ph: &mut [f64], amp: &mut [f64]) {
        let [qx, qy, qz] = self.q;
        let [ax, ay, az] = self.a;
        let n = ph.len().min(amp.len()).min(qx.len());
        let (qx, qy, qz) = (&qx[..n], &qy[..n], &qz[..n]);
        let (ax, ay, az) = (&ax[..n], &ay[..n], &az[..n]);
        let (f, ph0, ph, amp) = (&self.f[..n], &self.ph0[..n], &mut ph[..n], &mut amp[..n]);
        for j in 0..n {
            let (dx, dy, dz) = (p[0] - qx[j], p[1] - qy[j], p[2] - qz[j]);
            let rho = (dx * dx + dy * dy + dz * dz).sqrt();
            let cosx = ((dx * ax[j] + dy * ay[j] + dz * az[j]) / (rho * self.rr)).abs();
            amp[j] = f[j] * self.rr / rho * 0.5 * (1.0 + cosx);
            ph[j] = ph0[j] + self.ksg * rho;
        }
    }
}

/// 1 本の光線の 2 次波源
struct Src {
    /// 参照球（平面）の上の点
    q: V3,
    /// 平面の基準での光線の向き（球では使わない）
    k: V3,
    /// 振幅（面積の割合）
    f: f64,
    /// 位相 [rad] の定数の部分
    ph0: f64,
}

/// 参照球の形: Some((中心, 半径, σ)) なら球、None なら主光線に垂直な平面
type Sphere = Option<(V3, f64, f64)>;

/// Huygens PSF。w None なら全波長を重みで足した多色。座標は像面の局所座標 (x, y)（中心 = 主波長の主光線の
/// 像点）で、axes は像面の x̂・ŷ
pub fn huygens_psf(s: &System, field: FieldSel, w: Option<usize>, o: HuygensOpts) -> Result<Psf> {
    let ni = o.image;
    if o.pupil.is_some_and(|np| np < 4) || ni < 2 {
        invalid!(
            "Huygens PSF の瞳の格子 {:?} は 4 以上、像の格子 {ni} は 2 以上",
            o.pupil
        );
    }
    if s.afocal {
        notimpl!("Huygens PSF: アフォーカル像空間（FFT PSF を使う）");
    }
    if s.ropd != 1 && s.ropd != 2 {
        notimpl!("OPD の基準 {}（射出瞳・無限遠のみ）", s.ropd);
    }
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
    if !c.traced(0) {
        invalid!("Huygens PSF: 主波長の主光線が追跡できない");
    }
    let (pc, kc) = (c.rays[0].xyz, c.rays[0].lmn);
    if !(kc[2].abs() > 1e-9) {
        invalid!("Huygens PSF: 主光線が像面に平行");
    }
    // 参照球（OPD の to_ref と同じ）
    let expp = s.exit_pupil();
    let sphere: Sphere = if s.ropd == 1 || !expp.is_finite() || expp.abs() > 1e9 {
        None
    } else {
        let sc = (expp - pc[2]) / kc[2];
        Some((pc, sc.abs(), if sc < 0.0 { 1.0 } else { -1.0 }))
    };

    // 既定の間隔と、像の格子が瞳の標本の周期を超えないかの確かめに FFT PSF と同じ傾きを使う
    let mut slopes = Vec::new();
    let mut cut: (f64, f64) = (0.0, 0.0);
    for &wi in &ws {
        let (ga, gb) = perp_slopes_of(s, pupil_jacobian(s, field, wi)?, kc);
        let lam = s.wavelengths[wi] * 1e-3;
        slopes.push((ga / lam, gb / lam));
        if ws.len() > 1 && !(s.weights[wi] > 0.0) {
            continue;
        }
        cut.0 = cut.0.max(2.0 * (ga / lam).abs());
        cut.1 = cut.1.max(2.0 * (gb / lam).abs());
    }
    let delta = match o.delta {
        Some(d) if d > 0.0 => (d, d),
        Some(d) => invalid!("Huygens PSF の像の格子の間隔 {d}"),
        None => (0.5 / cut.0, 0.5 / cut.1),
    };
    let half = (ni / 2) as f64;
    let xs: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * delta.0).collect();
    let ys: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * delta.1).collect();
    let (cx, cy) = (pc[0], pc[1]);
    // 像面の上の格子の点（局所座標、z は像面の矢高）
    let img = &s.surfaces[s.image()];
    let pts: Vec<V3> = ys
        .iter()
        .flat_map(|&y| {
            xs.iter().map(move |&x| {
                let (px, py) = (cx + x, cy + y);
                let z = crate::freeform::sag_xy(img, px, py).0;
                [px, py, if z.is_finite() { z } else { 0.0 }]
            })
        })
        .collect();

    let np = match o.pupil {
        Some(n) => n,
        None => {
            let ws: Vec<usize> = ws
                .iter()
                .copied()
                .filter(|&wi| ws.len() == 1 || s.weights[wi] > 0.0)
                .collect();
            let need = (ni as f64 * (delta.0 * cut.0).max(delta.1 * cut.1)).ceil() as usize;
            crate::psf::auto_pupil(s, field, &ws, need, 512)?
        }
    };
    // 瞳の格子（FFT PSF と同じ区間の中心）
    let h = 2.0 / np as f64;
    let mut cells = Vec::new();
    let mut cell_ix = Vec::new();
    for iy in 0..np {
        for ix in 0..np {
            let ux = -1.0 + h * (ix as f64 + 0.5);
            let uy = -1.0 + h * (iy as f64 + 0.5);
            let (f, qx, qy) = cell_frac(ux, uy, h);
            if f > 0.0 {
                cells.push((f, qx, qy));
                cell_ix.push((ix, iy));
            }
        }
    }
    let px: Vec<f64> = cells.iter().map(|c| c.1).collect();
    let py: Vec<f64> = cells.iter().map(|c| c.2).collect();

    let mut tot = vec![0.0; pts.len()];
    let (mut nrm, mut wsum) = (0.0, 0.0);
    let (mut step, mut encl): (f64, f64) = (0.0, 0.0);
    for (&wi, &(a, b)) in ws.iter().zip(&slopes) {
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[wi] };
        if !(kw > 0.0) {
            continue;
        }
        if ni as f64 * delta.0 * a.abs() * h > 1.0 + 1e-9
            || ni as f64 * delta.1 * b.abs() * h > 1.0 + 1e-9
        {
            invalid!(
                "Huygens PSF: 像の格子の幅が瞳の標本の周期を超える（波長 {}、瞳の格子を増やすか、像の格子・間隔を減らす）",
                wi + 1
            );
        }
        let lam = s.wavelengths[wi] * 1e-3;
        let n = s.image_index(wi);
        let kn = 2.0 * PI * n / lam;
        let op = opd(s, field, &px, &py, Some(wi), true)?;
        let mut src = Vec::with_capacity(cells.len());
        let mut sa = 0.0;
        for (i, &(f, _, _)) in cells.iter().enumerate() {
            if !(op.ok(i) && op.opd[i].is_finite()) {
                continue;
            }
            let r = &op.trace.rays[i];
            let (q, k) = (r.xyz, r.lmn);
            let ph = -2.0 * PI * op.opd[i];
            let s1 = match sphere {
                Some((c0, rr, sg)) => {
                    // 像面の点 q から向き k に沿って参照球まで（OPD の to_ref と同じ根）
                    let dd = sub(q, c0);
                    let bb = dot(dd, k);
                    let cc = dot(dd, dd) - rr * rr;
                    let disc = (bb * bb - cc).max(0.0).sqrt();
                    let t = if sg > 0.0 { -bb - disc } else { -bb + disc };
                    Src {
                        q: add(q, scale(k, t)),
                        k,
                        f,
                        ph0: ph - kn * sg * rr,
                    }
                }
                None => {
                    let t = -dot(sub(q, pc), kc) / dot(k, kc);
                    Src {
                        q: add(q, scale(k, t)),
                        k,
                        f,
                        ph0: ph,
                    }
                }
            };
            src.push(s1);
            sa += f;
        }
        if !(sa > 0.0) {
            invalid!("Huygens PSF: 有効な光線がない（すべてけられた）");
        }
        let w_ok: Vec<(usize, usize, f64)> = (0..cells.len())
            .filter(|&i| op.ok(i) && op.opd[i].is_finite())
            .map(|i| (cell_ix[i].0, cell_ix[i].1, op.opd[i]))
            .collect();
        step = step.max(crate::psf::phase_step(np, &w_ok));
        // 光源を成分ごとの配列に（像の 1 点の内側のループをベクトル化し、位相因子をまとめて求める）
        let (qx, qy, qz): (Vec<f64>, Vec<f64>, Vec<f64>) = (
            src.iter().map(|q| q.q[0]).collect(),
            src.iter().map(|q| q.q[1]).collect(),
            src.iter().map(|q| q.q[2]).collect(),
        );
        let qf: Vec<f64> = src.iter().map(|q| q.f).collect();
        let ph0: Vec<f64> = src.iter().map(|q| q.ph0).collect();
        let ns = src.len();
        // 球: 光源から中心への向き c0 − q、平面: 光線の向き k
        let (ax, ay, az): (Vec<f64>, Vec<f64>, Vec<f64>) = match sphere {
            Some((c0, ..)) => (
                src.iter().map(|q| c0[0] - q.q[0]).collect(),
                src.iter().map(|q| c0[1] - q.q[1]).collect(),
                src.iter().map(|q| c0[2] - q.q[2]).collect(),
            ),
            None => (
                src.iter().map(|q| q.k[0]).collect(),
                src.iter().map(|q| q.k[1]).collect(),
                src.iter().map(|q| q.k[2]).collect(),
            ),
        };
        let one = |p: &V3, (ph, amp, z): &mut (Vec<f64>, Vec<f64>, Vec<C64>)| -> f64 {
            ph.resize(ns, 0.0);
            amp.resize(ns, 0.0);
            z.resize(ns, C64::new(0.0, 0.0));
            match sphere {
                Some((_, rr, sg)) => {
                    let t = SphereTerms {
                        q: [&qx, &qy, &qz],
                        a: [&ax, &ay, &az],
                        f: &qf,
                        ph0: &ph0,
                        rr,
                        ksg: kn * sg,
                    };
                    t.eval(p, ph, amp);
                }
                None => {
                    for j in 0..ns {
                        let kp = ax[j] * (p[0] - qx[j])
                            + ay[j] * (p[1] - qy[j])
                            + az[j] * (p[2] - qz[j]);
                        amp[j] = qf[j];
                        ph[j] = ph0[j] + kn * kp;
                    }
                }
            }
            crate::trig::cis_scaled(ph, amp, z);
            z.iter().fold(C64::new(0.0, 0.0), |u, &v| u + v).norm_sqr()
        };
        let buf = || (Vec::new(), Vec::new(), Vec::new());
        // 像の点ごとに中断を確かめる（GUI で系が変わったとき。やめた点は NaN、あとで Cancelled を返す）
        let one = |p: &V3, b: &mut (Vec<f64>, Vec<f64>, Vec<C64>)| -> f64 {
            if crate::par::cancelled() {
                f64::NAN
            } else {
                one(p, b)
            }
        };
        let iw: Vec<f64> = if pts.len() * src.len() >= 1 << 18 && !crate::par::sequential() {
            use rayon::prelude::*;
            pts.par_iter().map_init(buf, |b, p| one(p, b)).collect()
        } else {
            let mut b = buf();
            pts.iter().map(|p| one(p, &mut b)).collect()
        };
        crate::par::check_cancel()?;
        // 各波長で収差なしの中心が 1（|Σ f|²）、全強度の重み G = Σf |a b| h²（FFT PSF と同じ）
        let g = sa * (a * b * h * h).abs();
        for (t, v) in tot.iter_mut().zip(&iw) {
            *t += kw * g * v / (sa * sa);
        }
        // 格子に入った光量の割合: 全光量は FFT PSF と同じく Σf / |a b h²|（主光線に垂直な面で）。像面の上の
        // 1 点の面積は垂直な面では |k_z| 倍
        encl +=
            kw * iw.iter().sum::<f64>() * delta.0 * delta.1 * kc[2].abs() * (a * b * h * h).abs()
                / sa;
        nrm += kw * g;
        wsum += kw;
    }
    if !(wsum > 0.0) {
        invalid!("重みのある波長がない");
    }
    let i: Vec<Vec<f64>> = tot
        .chunks(ni)
        .map(|r| r.iter().map(|v| v / nrm).collect())
        .collect();
    let peak = i.iter().flatten().cloned().fold(0.0, f64::max);
    let enclosed = encl / wsum;
    Ok(Psf {
        x: xs,
        y: ys,
        i,
        center: (cx, cy),
        delta,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        peak,
        pupil: np,
        phase_step: step,
        enclosed,
        warnings: crate::psf::sampling_warnings("Huygens PSF", np, step, enclosed),
    })
}

/// PSF の格子の離散フーリエ変換の絶対値（球欠 = x、子午 = y の周波数）。DC で 1。
/// 格子の幅 W = N Δ の調和周波数 k / W で値を求め、その間は 4 点の Lagrange 補間で（調和周波数でない点で
/// 直接変換すると、格子の端の打ち切りの影響が残る。OpticStudio の Huygens MTF も PSF の FFT）
pub fn psf_mtf(p: &Psf, freqs: &[f64]) -> (Vec<f64>, Vec<f64>) {
    // 周辺分布（x・y への射影）
    let mut mx = vec![0.0; p.x.len()];
    let mut my = vec![0.0; p.y.len()];
    for (iy, row) in p.i.iter().enumerate() {
        for (ix, v) in row.iter().enumerate() {
            mx[ix] += v;
            my[iy] += v;
        }
    }
    let e: f64 = mx.iter().sum();
    let ft = |m: &[f64], c: &[f64], f: f64| -> f64 {
        let mut u = C64::new(0.0, 0.0);
        for (v, x) in m.iter().zip(c) {
            let (sn, cs) = (-2.0 * PI * f * x).sin_cos();
            u += C64::new(cs, sn) * *v;
        }
        u.norm() / e
    };
    let interp = |m: &[f64], c: &[f64], d: f64| -> Vec<f64> {
        let df = 1.0 / (c.len() as f64 * d);
        let mut cache = std::collections::HashMap::new();
        let mut at = |k: i64| -> f64 {
            *cache
                .entry(k.unsigned_abs())
                .or_insert_with(|| ft(m, c, k.unsigned_abs() as f64 * df))
        };
        freqs
            .iter()
            .map(|&f| {
                let t = f.abs() / df;
                let k0 = t.round();
                if (t - k0).abs() < 1e-9 {
                    return at(k0 as i64);
                }
                let i = t.floor() as i64 - 1;
                let u = t - i as f64;
                // 点 i..i+3 の Lagrange（負の番号は |k| で偶関数）
                (0..4)
                    .map(|a| {
                        let w: f64 = (0..4)
                            .filter(|&b| b != a)
                            .map(|b| (u - b as f64) / (a as f64 - b as f64))
                            .product();
                        w * at(i + a)
                    })
                    .sum::<f64>()
                    .max(0.0)
            })
            .collect()
    };
    let sag = interp(&mx, &p.x, p.delta.0);
    let tan = interp(&my, &p.y, p.delta.1);
    (tan, sag)
}

/// Huygens MTF（OpticStudio の Huygens MTF 相当）。周波数 [cycles/レンズ単位] は像面の上の x（球欠）・y（子午）
pub fn huygens_mtf(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    o: HuygensOpts,
    freqs: &[f64],
) -> Result<Mtf> {
    let p = huygens_psf(s, field, w, o)?;
    let (tan, sag) = psf_mtf(&p, freqs);
    let jm = pupil_jacobian(s, field, s.primary)?;
    let lam = s.wavelengths[s.primary] * 1e-3;
    let cut = |dir: usize| {
        let (a, b) = pupil_shift(&jm, lam, dir);
        2.0 / a.hypot(b)
    };
    Ok(Mtf {
        freqs: freqs.to_vec(),
        tan,
        sag,
        cutoff: (cut(0), cut(1)),
    })
}
