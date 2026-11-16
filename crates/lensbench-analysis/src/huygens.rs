//! Huygens PSF・Huygens MTF（OpticStudio の Huygens PSF・Huygens MTF 相当）。
//!
//! FFT の PSF（[`lensbench_core::psf::psf`]）は瞳から像の面（既定は像面、照合用に主光線に垂直な面）への写像を一次（対角）で近似するが、Huygens は
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

// NaN を弾く比較（!(x > 0.0)）を core と同じく使う
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use lensbench_core::analysis::{mtf_cutoff, pupil_jacobian, Mtf};
use lensbench_core::coating::C64;
use lensbench_core::error::Result;
use lensbench_core::math::*;
use lensbench_core::psf::{perp_slopes_of, plane_slopes, Psf, PsfPlane};
use lensbench_core::pupil::{PupilAmp, PupilFunction, PupilGrid, RefSurface};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{invalid, notimpl};
use lensbench_core::{TanSag, Waves};
use std::f64::consts::PI;

/// Huygens PSF・MTF の設定
#[derive(Debug, Clone, Copy)]
pub struct HuygensOpts {
    /// 瞳の格子の一辺（直径を n 区間に分けた各区間の中心で標本化）。None で自動（隣り合う標本の位相差が π/2 以下に
    /// なる大きさ、64〜256。超えたら格子を増やして求め直す。計算量は瞳の格子² × 像の格子²）
    pub pupil: Option<usize>,
    /// 像の格子の一辺
    pub image: usize,
    /// 像の格子の間隔 [レンズ単位]（像面の上で。None で像面の上の 1 / (2 × 遮断周波数)、x・y 別。FFT PSF の
    /// 既定の面（[`lensbench_core::psf::PsfPlane::Image`]）の既定の間隔と同じ）
    pub pixel: Option<f64>,
}

impl Default for HuygensOpts {
    fn default() -> Self {
        HuygensOpts {
            pupil: None,
            image: 128,
            pixel: None,
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
pub fn huygens_psf(s: &System, field: FieldSel, w: Waves, o: HuygensOpts) -> Result<Psf> {
    huygens_psf_with(s, field, w, o, None)
}

/// [`huygens_psf`] に、膜・偏光の振幅 extra（[`PupilAmp`]、例えば [`crate::pupil_amp::PolAmp`]）を足す。
/// 系のアポダイゼーションはどちらにも入る
pub fn huygens_psf_with(
    s: &System,
    field: FieldSel,
    w: Waves,
    o: HuygensOpts,
    extra: Option<&dyn PupilAmp>,
) -> Result<Psf> {
    let w = w.one();
    lensbench_core::psf::retry_auto(o.pupil, HUYGENS_PUPIL_CAP, |np| {
        huygens_once(s, field, w, HuygensOpts { pupil: np, ..o }, extra)
    })
}

/// 自動の瞳の格子の上限（Huygens PSF。計算量は瞳の格子² × 像の格子²）
const HUYGENS_PUPIL_CAP: usize = 256;

fn huygens_once(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    o: HuygensOpts,
    extra: Option<&dyn PupilAmp>,
) -> Result<Psf> {
    let ni = o.image;
    if o.pupil.is_some_and(|np| np < 4) || ni < 2 {
        invalid!(
            "Huygens PSF pupil grid {:?} must be 4 or more and image grid {ni} 2 or more",
            o.pupil
        );
    }
    if s.afocal {
        notimpl!("Huygens PSF: afocal image space (use the FFT PSF)");
    }
    if s.ropd != 1 && s.ropd != 2 {
        notimpl!("OPD reference {} (exit pupil or infinity only)", s.ropd);
    }
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
    if !c.traced(0) {
        invalid!("Huygens PSF: can't trace the chief ray of the primary wavelength");
    }
    let (pc, kc) = (c.rays[0].xyz, c.rays[0].lmn);
    if !(kc[2].abs() > 1e-9) {
        invalid!("Huygens PSF: the chief ray is parallel to the image surface");
    }
    // 参照球（OPD の to_ref と同じ）
    let sphere: Sphere = match RefSurface::of(s, pc, kc) {
        RefSurface::Sphere {
            center,
            radius,
            sign,
        } => Some((center, radius, sign)),
        RefSurface::Plane { .. } => None,
    };

    // 既定の間隔と、像の格子が瞳の標本の周期を超えないかの確かめには像面の上の傾き（FFT PSF の既定の面、回折 MTF と
    // 同じ）を、全光量の重み（FFT PSF と同じ Parseval の全強度）には主光線に垂直な面の傾きを使う
    let mut slopes = Vec::new();
    let mut cut: (f64, f64) = (0.0, 0.0);
    for &wi in &ws {
        let jm = pupil_jacobian(s, field, wi)?;
        let (ga, gb) = plane_slopes(s, jm, kc, PsfPlane::Image);
        let (pa, pb) = perp_slopes_of(s, jm, kc);
        let lam = s.wavelengths[wi] * 1e-3;
        slopes.push(((ga / lam, gb / lam), (pa / lam, pb / lam)));
        if ws.len() > 1 && !(s.weights[wi] > 0.0) {
            continue;
        }
        cut.0 = cut.0.max(2.0 * (ga / lam).abs());
        cut.1 = cut.1.max(2.0 * (gb / lam).abs());
    }
    let pixel = match o.pixel {
        Some(d) if d > 0.0 => (d, d),
        Some(d) => invalid!("Huygens PSF image grid spacing {d}"),
        None => (0.5 / cut.0, 0.5 / cut.1),
    };
    let half = (ni / 2) as f64;
    let xs: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * pixel.0).collect();
    let ys: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * pixel.1).collect();
    let (cx, cy) = (pc[0], pc[1]);
    // 像面の上の格子の点（局所座標、z は像面の矢高）。矢高が求まらない点（像面の形の式の外）は z が NaN で、
    // 強度も NaN（平面として計算しない。ANA-24）
    let img = &s.surfaces[s.image()];
    let pts: Vec<V3> = ys
        .iter()
        .flat_map(|&y| {
            xs.iter().map(move |&x| {
                let (px, py) = (cx + x, cy + y);
                // 球の縁の外などでは矢高は端の値に丸めて有限のまま返り、傾きが NaN になる
                let (z, zx, zy) = lensbench_core::freeform::sag_xy(img, px, py);
                let on = z.is_finite() && zx.is_finite() && zy.is_finite();
                [px, py, if on { z } else { f64::NAN }]
            })
        })
        .collect();
    let off_surface = pts.iter().filter(|p| !p[2].is_finite()).count();

    let np = match o.pupil {
        Some(n) => n,
        None => {
            let ws: Vec<usize> = ws
                .iter()
                .copied()
                .filter(|&wi| ws.len() == 1 || s.weights[wi] > 0.0)
                .collect();
            let need = (ni as f64 * (pixel.0 * cut.0).max(pixel.1 * cut.1)).ceil() as usize;
            lensbench_core::psf::auto_pupil(s, field, &ws, need, HUYGENS_PUPIL_CAP)?
        }
    };
    // 瞳の格子（FFT PSF と同じ区間の中心）
    let grid = PupilGrid::Cells(np);
    let h = 2.0 / np as f64;

    let mut tot = vec![0.0; pts.len()];
    let (mut nrm, mut wsum) = (0.0, 0.0);
    let (mut step, mut encl): (f64, f64) = (0.0, 0.0);
    for (&wi, &((ia, ib), (a, b))) in ws.iter().zip(&slopes) {
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[wi] };
        if !(kw > 0.0) {
            continue;
        }
        if ni as f64 * pixel.0 * ia.abs() * h > 1.0 + 1e-9
            || ni as f64 * pixel.1 * ib.abs() * h > 1.0 + 1e-9
        {
            invalid!(
                "Huygens PSF: the image grid is wider than the period of the pupil samples (wavelength {}; increase the pupil grid or reduce the image grid or spacing)",
                wi + 1
            );
        }
        let lam = s.wavelengths[wi] * 1e-3;
        let n = s.image_index(wi);
        let kn = 2.0 * PI * n / lam;
        let op = PupilFunction::trace_amp(s, field, Some(wi), grid, true, extra)?;
        let mut src = Vec::with_capacity(op.len());
        // s1 = Σ f A（収差なしの中心の振幅）、s2 = Σ f A²（光量）。f は区間の面積の割合、A は振幅
        let (mut s1, mut s2) = (0.0, 0.0);
        let mut w_ok: Vec<(usize, usize, f64)> = Vec::new();
        for i in 0..op.len() {
            let Some(w) = op.get(i) else {
                continue;
            };
            let f = op.samples.wt[i] * op.amp[i];
            w_ok.push((op.samples.cell[i].0, op.samples.cell[i].1, w));
            let (q, k) = (op.rays.xyz[i], op.rays.lmn[i]);
            let ph = -2.0 * PI * w;
            let s1_src = match sphere {
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
            src.push(s1_src);
            s1 += f;
            s2 += f * op.amp[i];
        }
        if !(s2 > 0.0) {
            invalid!("Huygens PSF: no valid rays (all vignetted)");
        }
        step = step.max(lensbench_core::psf::phase_step(np, &w_ok));
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
            lensbench_core::trig::cis_scaled(ph, amp, z);
            z.iter().fold(C64::new(0.0, 0.0), |u, &v| u + v).norm_sqr()
        };
        let buf = || (Vec::new(), Vec::new(), Vec::new());
        // 像の点ごとに中断を確かめる（GUI で系が変わったとき。やめた点は NaN、あとで Cancelled を返す）
        let one = |p: &V3, b: &mut (Vec<f64>, Vec<f64>, Vec<C64>)| -> f64 {
            if lensbench_core::par::cancelled() || !p[2].is_finite() {
                f64::NAN
            } else {
                one(p, b)
            }
        };
        let iw: Vec<f64> = if pts.len() * src.len() >= 1 << 18 && !lensbench_core::par::sequential()
        {
            use rayon::prelude::*;
            let cx = lensbench_core::par::current();
            pts.par_iter()
                .map_init(buf, |b, p| lensbench_core::par::with_ctx(&cx, || one(p, b)))
                .collect()
        } else {
            let mut b = buf();
            pts.iter().map(|p| one(p, &mut b)).collect()
        };
        lensbench_core::par::check_cancel()?;
        // 各波長で収差なしの中心が 1（|Σ f A|²）、全強度の重み G = (Σ f A)² / Σ f A² · |a b| h²（FFT PSF と同じ。
        // 振幅が一様なら Σ f |a b| h²）
        let g = s1 * (a * b * h * h).abs() * (s1 / s2);
        for (t, v) in tot.iter_mut().zip(&iw) {
            *t += kw * g * v / (s1 * s1);
        }
        // 格子に入った光量の割合: 全光量は FFT PSF と同じく Σf / |a b h²|（主光線に垂直な面で）。像面の上の
        // 1 点の面積は垂直な面では |k_z| 倍
        let isum: f64 = iw.iter().filter(|v| v.is_finite()).sum();
        encl += kw * isum * pixel.0 * pixel.1 * kc[2].abs() * (a * b * h * h).abs() / s2;
        nrm += kw * g;
        wsum += kw;
    }
    if !(wsum > 0.0) {
        invalid!("no weighted wavelength");
    }
    let i: Vec<Vec<f64>> = tot
        .chunks(ni)
        .map(|r| r.iter().map(|v| v / nrm).collect())
        .collect();
    let peak = i.iter().flatten().cloned().fold(0.0, f64::max);
    let enclosed = encl / wsum;
    let mut warnings = lensbench_core::psf::sampling_warnings("Huygens PSF", np, step, enclosed);
    if off_surface > 0 {
        warnings.push(format!(
            "Huygens PSF: the image surface sag can't be found at {off_surface} of the {npts} image grid points (outside the surface shape); their intensity is NaN",
            npts = pts.len()
        ));
    }
    Ok(Psf {
        i: lensbench_core::grid::Grid2::from_rows(xs, ys, i),
        plane: PsfPlane::Image,
        center: (cx, cy),
        pixel,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        peak,
        pupil: np,
        phase_step: step,
        enclosed,
        warnings,
    })
}

/// PSF の格子の離散フーリエ変換の絶対値（球欠 = x、子午 = y の周波数）。DC で 1。
/// 格子の幅 W = N Δ の調和周波数 k / W で値を求め、その間は 4 点の Lagrange 補間で（調和周波数でない点で
/// 直接変換すると、格子の端の打ち切りの影響が残る。OpticStudio の Huygens MTF も PSF の FFT）。
/// 格子の間隔 Δ のナイキスト 1/(2Δ) を超える周波数は NaN
pub fn psf_mtf(p: &Psf, freqs: &[f64]) -> TanSag<Vec<f64>> {
    // 強度が NaN の点（像面の形の外など）を含む PSF からは、どの周波数の値も決まらない（変換も規格化の全光量も
    // 格子の全点の和）。全部 NaN にする（下の補間の max(0) で NaN が 0 に化けないよう、先に返す）
    if p.i.rows().flatten().any(|v| v.is_nan()) {
        let nan = vec![f64::NAN; freqs.len()];
        return TanSag {
            tan: nan.clone(),
            sag: nan,
        };
    }
    // 周辺分布（x・y への射影）
    let mut mx = vec![0.0; p.i.nx()];
    let mut my = vec![0.0; p.i.ny()];
    for (iy, row) in p.i.rows().enumerate() {
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
                // ナイキスト 1/(2Δ) を超える周波数は折り返した値になるので出さない（ANA-24）
                if f.abs() > 0.5 / d * (1.0 + 1e-12) {
                    return f64::NAN;
                }
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
    TanSag {
        tan: interp(&my, p.i.y(), p.pixel.1),
        sag: interp(&mx, p.i.x(), p.pixel.0),
    }
}

/// Huygens MTF（OpticStudio の Huygens MTF 相当）。周波数 [cycles/レンズ単位] は像面の上の x（球欠）・y（子午）
pub fn huygens_mtf(
    s: &System,
    field: FieldSel,
    w: Waves,
    o: HuygensOpts,
    freqs: &[f64],
) -> Result<Mtf> {
    huygens_mtf_with(s, field, w, o, freqs, None)
}

/// [`huygens_mtf`] に膜・偏光の振幅 extra を足す（[`huygens_psf_with`]）
pub fn huygens_mtf_with(
    s: &System,
    field: FieldSel,
    w: Waves,
    o: HuygensOpts,
    freqs: &[f64],
    extra: Option<&dyn PupilAmp>,
) -> Result<Mtf> {
    let w = w.one();
    let p = huygens_psf_with(s, field, Waves::all_or(w), o, extra)?;
    // 像の格子の一部で強度が決まらない（像面の矢高が求まらない点）PSF の MTF は、どの周波数も決まらない。
    // 結果（Mtf）に注意の欄が無いので、NaN の並びを黙って返さず理由つきの誤りにする
    let off = p.i.rows().flatten().filter(|v| v.is_nan()).count();
    if off > 0 {
        invalid!("Huygens MTF: the PSF intensity is undefined at {off} image grid points (the image surface sag can't be found there); reduce the image grid or spacing");
    }
    let TanSag { tan, sag } = psf_mtf(&p, freqs);
    Ok(Mtf {
        freqs: freqs.to_vec(),
        tan,
        sag,
        cutoff: mtf_cutoff(s, field, w.unwrap_or(s.primary))?,
    })
}
