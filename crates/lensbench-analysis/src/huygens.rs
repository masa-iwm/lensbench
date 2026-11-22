//! Huygens PSF・Huygens MTF（OpticStudio の Huygens PSF・Huygens MTF 相当）。
//!
//! FFT の PSF（[`crate::psf::psf`]）は瞳から像の面（既定は像面、照合用に主光線に垂直な面）への写像を一次（対角）で近似するが、Huygens は
//! 射出瞳の参照球（OPD の基準と同じ。ROPD「無限遠」・射出瞳が無限遠なら主光線に垂直な平面）の上の各点を
//! 2 次波源とし、像面（傾き・曲率を含む像面そのもの）の格子の各点まで素直に足し合わせる:
//!
//! U(P) = Σ_i f_i exp(2πi (W_i − n σ (|P − Q_i| − R) / λ)) · (R / |P − Q_i|) · (1 + cos χ_i) / 2
//!
//! Q_i は光線 i を像面から参照球まで戻した点、W_i はその OPD [波]、f_i は瞳の区間の単位円内の面積の割合
//! （FFT PSF と同じ正規化瞳の格子）、R は参照球の半径、σ は参照球が像点の前なら +1・後ろなら −1、χ_i は
//! Q_i → P と Q_i → 像点の向きの角。平面の基準では U(P) = Σ f_i exp(2πi (W_i − n k_i · (P − Q_i) / λ))。
//! 位相の約束は exp(−ikL)（[`lensbench_core::phase`]、FFT PSF の瞳関数 exp(+2πiW) と同じ）。
//! 瞳から像への写像を近似しないので、像面が傾いた系・収差が大きい系・斜めの視野でも像面の上の PSF が出る。
//!
//! 正規化は FFT PSF と同じ: 各波長で収差のない同じ瞳の中心の値を 1 とし、多色は各波長の全強度を重みに
//! 比例させる（重みは FFT PSF と同じく Parseval の全強度 Σf / |a b h²|）。
//! Huygens MTF は Huygens PSF の像面の格子の上の離散フーリエ変換の絶対値（DC で 1）。像の格子が PSF を
//! 覆いきれないと MTF は低周波で下がり、格子の間隔が粗いと高周波で折り返すので、格子は利用者が決める。

// NaN を弾く比較（!(x > 0.0)）を core と同じく使う
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::analysis::{mtf_cutoff_axes, mtf_tangential, pupil_jacobian, Mtf, MtfAxes};
use crate::psf::{perp_slopes_of, plane_slopes, Psf, PsfPlane};
use lensbench_core::coating::C64;
use lensbench_core::error::Result;
use lensbench_core::math::*;
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
    /// 既定の面（[`crate::psf::PsfPlane::Image`]）の既定の間隔と同じ）
    pub pixel: Option<f64>,
    /// true で像の全部の点で各光源を厳密に足す（遅い）。false（既定）は像の格子を小さな区画に分け、区画の中では
    /// 各光源の位相と振幅を像の点の番号の 2 次式で近似する（数倍速い）。区画の大きさは、2 次式と厳密な値の差が
    /// 位相で 1e-6 rad・振幅で 1e-6 倍以下になるように選ぶ（確かめるのは格子の四隅と中央の区画だけで、全部の
    /// 区画の保証ではない。強度の差は照合の系で収差のない中心の値の 2e-7 倍以下）。合う区画が無ければ厳密に計算する
    pub exact: bool,
    /// 和を計算する場所（[`HuygensDevice`]）。exact が true なら使わない（いつも CPU の厳密な和）
    pub device: HuygensDevice,
}

impl Default for HuygensOpts {
    fn default() -> Self {
        HuygensOpts {
            pupil: None,
            image: 128,
            pixel: None,
            exact: false,
            device: HuygensDevice::Auto,
        }
    }
}

/// Huygens の和（光源 × 像の点）を計算する場所。GPU は単精度（位相を小さく保つ書き方で、強度の差は照合の系で
/// 収差のない中心の値の 5e-7 倍ほど。CPU の既定の展開と同じ程度）。GPU は `gpu` 機能を付けて組んだときだけ
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum HuygensDevice {
    /// GPU があれば GPU（CPU で動くアダプタ（WARP・llvmpipe）と macOS 以外の内蔵 GPU は使わない）、無ければ CPU の既定の展開
    #[default]
    Auto,
    /// いつも CPU
    Cpu,
    /// CPU で動くアダプタ・内蔵 GPU でも GPU の経路を使う（試験・照合用）。アダプタが無い・誤りなら CPU で計算して注意を出す
    Gpu,
}

/// `gpu` 機能を付けて組んだか（false なら Huygens の和はいつも CPU）
pub const GPU_SUPPORT: bool = cfg!(feature = "gpu");

/// 使える GPU の名前（[`HuygensDevice::Auto`] で使うもの）。無い・`gpu` 機能なしで組んだなら None
pub fn gpu_name() -> Option<String> {
    #[cfg(feature = "gpu")]
    {
        lensbench_gpu::gpu(false).map(|g| g.name().to_string())
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// GPU の和の入力（[`gpu_sum`]）。`gpu` 機能なしで組むと作るだけで読まない
#[cfg_attr(not(feature = "gpu"), allow(dead_code))]
struct GpuInput<'a> {
    sphere: Sphere,
    /// 平面の基準の点（主光線の像点）と主光線の向き
    pc: V3,
    kc: V3,
    kn: f64,
    q: [&'a [f64]; 3],
    /// 球: 光源から中心へ、平面: 光線の向き
    a: [&'a [f64]; 3],
    f: &'a [f64],
    ph0: &'a [f64],
    pts: &'a [V3],
}

/// GPU で和を求める（値と使ったアダプタの名前）。None なら使わない（CPU で計算する）。誤りで CPU に戻るときは
/// note に理由を入れる（[`HuygensDevice::Gpu`] のときだけ）
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
fn gpu_sum(
    device: HuygensDevice,
    x: &GpuInput<'_>,
    note: &mut Option<String>,
) -> Result<Option<(Vec<f64>, String)>> {
    if device == HuygensDevice::Cpu {
        return Ok(None);
    }
    #[cfg(feature = "gpu")]
    {
        let forced = device == HuygensDevice::Gpu;
        let Some(g) = lensbench_gpu::gpu(forced) else {
            if forced {
                *note = Some("Huygens PSF: no GPU adapter; computed on the CPU".into());
            }
            return Ok(None);
        };
        let ns = x.f.len();
        let [qx, qy, qz] = x.q;
        let [ax, ay, az] = x.a;
        let (reference, dir, phase, origin) = match x.sphere {
            Some((c0, rr, sg)) => {
                // 光源から中心への向き a = C − Q（|a| = R）、位相は定数 kσR を除いた 2πW
                let dir: Vec<[f64; 3]> = (0..ns).map(|j| [ax[j], ay[j], az[j]]).collect();
                let phase: Vec<f64> = x.ph0.iter().map(|p| p - x.kn * sg * rr).collect();
                (
                    lensbench_gpu::Reference::Sphere {
                        radius: rr,
                        ksg: x.kn * sg,
                    },
                    dir,
                    phase,
                    c0,
                )
            }
            None => {
                // 向きは主光線との差 k − k₀、位相に kn k·(Q − P₀) を入れる（kn k₀·(P − P₀) は全部の光源に共通なので落とす）
                let dir: Vec<[f64; 3]> = (0..ns)
                    .map(|j| [ax[j] - x.kc[0], ay[j] - x.kc[1], az[j] - x.kc[2]])
                    .collect();
                let phase: Vec<f64> = (0..ns)
                    .map(|j| {
                        let kq = ax[j] * (qx[j] - x.pc[0])
                            + ay[j] * (qy[j] - x.pc[1])
                            + az[j] * (qz[j] - x.pc[2]);
                        x.ph0[j] + x.kn * kq
                    })
                    .collect();
                (
                    lensbench_gpu::Reference::Plane { kn: x.kn },
                    dir,
                    phase,
                    x.pc,
                )
            }
        };
        let pts: Vec<[f64; 3]> = x.pts.iter().map(|p| sub(*p, origin)).collect();
        let src = lensbench_gpu::Sources {
            dir: &dir,
            amp: x.f,
            phase: &phase,
        };
        match g.huygens(reference, src, &pts, &lensbench_core::par::cancelled) {
            Ok(v) => Ok(Some((v, g.name().to_string()))),
            Err(lensbench_gpu::Error::Cancelled) => {
                lensbench_core::par::check_cancel()?;
                Ok(None)
            }
            Err(e) => {
                if forced {
                    *note = Some(format!("Huygens PSF: {e}; computed on the CPU"));
                }
                Ok(None)
            }
        }
    }
    #[cfg(not(feature = "gpu"))]
    {
        if device == HuygensDevice::Gpu {
            *note = Some("Huygens PSF: built without GPU support; computed on the CPU".into());
        }
        Ok(None)
    }
}

/// 位相の展開（[`HuygensOpts::exact`] が false）で一度に進める光源の数
const LANES: usize = 8;
/// 展開の許容: 2 次式と厳密な値の差（位相 [rad]、振幅は相対）
const EXPAND_TOL: f64 = 1e-6;
/// 区画の一辺（像の点の数）の候補。大きい方から試す
const EXPAND_BLOCKS: [usize; 4] = [32, 16, 8, 4];

/// 区画の 7 点（中心、±x、±y、対角の 2 点。間隔 h 点）の値 f から、中心のまわりの 2 次式の係数
/// [f0, gx, gy, hxx, hyy, hxy]（f ≈ f0 + gx u + gy v + hxx u²/2 + hxy u v + hyy v²/2、u・v は中心からの点の数）
fn fit_quad(f: &[Vec<f64>; 7], h: f64, c: &mut [Vec<f64>; 6]) {
    let n = f[0].len();
    for v in c.iter_mut() {
        v.clear();
        v.resize(n, 0.0);
    }
    let (i2h, ih2) = (0.5 / h, 1.0 / (h * h));
    for j in 0..n {
        let (f0, xp, xm, yp, ym, pp, mm) = (
            f[0][j], f[1][j], f[2][j], f[3][j], f[4][j], f[5][j], f[6][j],
        );
        c[0][j] = f0;
        c[1][j] = (xp - xm) * i2h;
        c[2][j] = (yp - ym) * i2h;
        c[3][j] = (xp - 2.0 * f0 + xm) * ih2;
        c[4][j] = (yp - 2.0 * f0 + ym) * ih2;
        c[5][j] = (pp + mm - xp - xm - yp - ym + 2.0 * f0) * 0.5 * ih2;
    }
}

/// [`fit_quad`] の係数を、区画の隅の点 (0, 0) からの前進差分 [値, x の差分, y の差分, hxx, hyy, hxy] に直し、
/// 長さを LANES の倍数にそろえる（足した分は 0）
fn to_forward(c: &mut [Vec<f64>; 6], h: f64) {
    let n = c[0].len();
    for j in 0..n {
        let (f0, gx, gy, hxx, hyy, hxy) = (c[0][j], c[1][j], c[2][j], c[3][j], c[4][j], c[5][j]);
        c[0][j] = f0 - (gx + gy) * h + (0.5 * hxx + hxy + 0.5 * hyy) * h * h;
        c[1][j] = gx + 0.5 * hxx * (1.0 - 2.0 * h) - hxy * h;
        c[2][j] = gy + 0.5 * hyy * (1.0 - 2.0 * h) - hxy * h;
    }
    let np = n.div_ceil(LANES) * LANES;
    for v in c.iter_mut() {
        v.resize(np, 0.0);
    }
}

/// 区画の中の和: 各光源の exp(i 位相) を漸化式（z ← z r、r ← r e）で、振幅を前進差分で進め、像の点ごとに
/// 光源の組の列ごとの和 acc（[j·bx + i][列]）に足す。e は位相の前進差分の exp、a は振幅の前進差分
fn sweep(
    e: &[Vec<C64>; 6],
    a: &[Vec<f64>; 6],
    bx: usize,
    by: usize,
    acc: &mut [[[f64; LANES]; 2]],
) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // 安全: AVX2 があることを上で確かめた（演算は同じなので結果は同じビット）
            return unsafe { sweep_avx2(e, a, bx, by, acc) };
        }
    }
    sweep_impl(e, a, bx, by, acc)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn sweep_avx2(
    e: &[Vec<C64>; 6],
    a: &[Vec<f64>; 6],
    bx: usize,
    by: usize,
    acc: &mut [[[f64; LANES]; 2]],
) {
    sweep_impl(e, a, bx, by, acc)
}

#[inline(always)]
fn sweep_impl(
    e: &[Vec<C64>; 6],
    a: &[Vec<f64>; 6],
    bx: usize,
    by: usize,
    acc: &mut [[[f64; LANES]; 2]],
) {
    type V = [f64; LANES];
    #[inline(always)]
    fn mul(ar: &mut V, ai: &mut V, br: &V, bi: &V) {
        for l in 0..LANES {
            let (r, i) = (ar[l] * br[l] - ai[l] * bi[l], ar[l] * bi[l] + ai[l] * br[l]);
            ar[l] = r;
            ai[l] = i;
        }
    }
    let n = a[0].len();
    let acc = &mut acc[..bx * by];
    for c in (0..n).step_by(LANES) {
        let re = |k: usize| -> V { std::array::from_fn(|l| e[k][c + l].re) };
        let im = |k: usize| -> V { std::array::from_fn(|l| e[k][c + l].im) };
        let am = |k: usize| -> V { std::array::from_fn(|l| a[k][c + l]) };
        let (mut zr, mut zi, mut rxr, mut rxi, mut ryr, mut ryi) =
            (re(0), im(0), re(1), im(1), re(2), im(2));
        let (exxr, exxi, eyyr, eyyi, exyr, exyi) = (re(3), im(3), re(4), im(4), re(5), im(5));
        let (mut a0, mut dax, mut day) = (am(0), am(1), am(2));
        let (axx, ayy, axy) = (am(3), am(4), am(5));
        for row in acc.chunks_exact_mut(bx) {
            let (mut wr, mut wi, mut qr, mut qi, mut av, mut dv) = (zr, zi, rxr, rxi, a0, dax);
            for u in row.iter_mut() {
                let [ur, ui] = u;
                for l in 0..LANES {
                    ur[l] += av[l] * wr[l];
                    ui[l] += av[l] * wi[l];
                    av[l] += dv[l];
                    dv[l] += axx[l];
                }
                mul(&mut wr, &mut wi, &qr, &qi);
                mul(&mut qr, &mut qi, &exxr, &exxi);
            }
            mul(&mut zr, &mut zi, &ryr, &ryi);
            mul(&mut ryr, &mut ryi, &eyyr, &eyyi);
            mul(&mut rxr, &mut rxi, &exyr, &exyi);
            for l in 0..LANES {
                a0[l] += day[l];
                day[l] += ayy[l];
                dax[l] += axy[l];
            }
        }
    }
}

/// 位相の展開の作業用の配列（スレッドごと）
#[derive(Default)]
struct ExpandBuf {
    ph: [Vec<f64>; 7],
    amp: [Vec<f64>; 7],
    cph: [Vec<f64>; 6],
    camp: [Vec<f64>; 6],
    ones: Vec<f64>,
    e: [Vec<C64>; 6],
    acc: Vec<[[f64; LANES]; 2]>,
    exact: (Vec<f64>, Vec<f64>, Vec<C64>),
}

/// 区画の 7 点の位置（区画の隅からの点の数、h の倍数）: 中心、+x、−x、+y、−y、(+, +)、(−, −)
const NODES: [(i64, i64); 7] = [(1, 1), (2, 1), (0, 1), (1, 2), (1, 0), (2, 2), (0, 0)];

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
            ph[j] = ph0[j] - self.ksg * rho;
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
    crate::psf::retry_auto(o.pupil, HUYGENS_PUPIL_CAP, |np| {
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
    lensbench_core::pupil::check_opd_reference(s)?;
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
        RefSurface::Plane { .. } | RefSurface::Far { .. } | RefSurface::Image => None,
    };
    // 像面の上の光路長そのままの基準なら、光源は像面の点
    let at_image = matches!(RefSurface::of(s, pc, kc), RefSurface::Image);
    // 半径が無限大の参照球なら、光源は主光線の像点から光線へ下ろした垂線の足（OPD の to_ref と同じ）
    let far = matches!(RefSurface::of(s, pc, kc), RefSurface::Far { .. });

    // 既定の間隔と、像の格子が瞳の標本の周期を超えないかの確かめには像面の上の傾き（FFT PSF の既定の面、回折 MTF と
    // 同じ）を、全光量の重み（FFT PSF と同じ Parseval の全強度）には主光線に垂直な面の傾きを使う
    let mut slopes = Vec::new();
    let mut cut: (f64, f64) = (0.0, 0.0);
    for &wi in &ws {
        let jm = pupil_jacobian(s, field, wi)?;
        let (ga, gb) = plane_slopes(s, jm, kc, PsfPlane::Image);
        let (pa, pb) = perp_slopes_of(s, jm, kc);
        let lam = s.wave(wi).mm();
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
    // 強度も NaN（平面として計算しない）
    let img = &s.surfaces[s.image()];
    let on_image = |x: f64, y: f64| -> V3 {
        let (px, py) = (cx + x, cy + y);
        // 球の縁の外などでは矢高は端の値に丸めて有限のまま返り、傾きが NaN になる
        let (z, zx, zy) = lensbench_core::freeform::sag_xy(img, px, py);
        let on = z.is_finite() && zx.is_finite() && zy.is_finite();
        [px, py, if on { z } else { f64::NAN }]
    };
    let pts: Vec<V3> = ys
        .iter()
        .flat_map(|&y| xs.iter().map(move |&x| on_image(x, y)))
        .collect();
    // 格子の点の番号（格子の外でもよい）の像面の上の点。位相の展開の区画の 7 点に使う
    let at = |ix: i64, iy: i64| -> V3 {
        on_image((ix as f64 - half) * pixel.0, (iy as f64 - half) * pixel.1)
    };
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
            crate::psf::auto_pupil(s, field, &ws, need, HUYGENS_PUPIL_CAP)?
        }
    };
    // 瞳の格子（FFT PSF と同じ区間の中心）
    let grid = PupilGrid::Cells(np);
    let h = 2.0 / np as f64;

    let mut tot = vec![0.0; pts.len()];
    let mut gpu_note: Option<String> = None;
    // 和を計算した装置（波長ごと。GPU のアダプタの名前か None = CPU）
    let mut used: Vec<Option<String>> = Vec::new();
    let (mut nrm, mut wsum) = (0.0, 0.0);
    let (mut step, mut encl): (f64, f64) = (0.0, 0.0);
    for (&wi, &((ia, ib), (a, b))) in ws.iter().zip(&slopes) {
        let kw = lensbench_core::pupil::wave_weight(s, field, wi, ws.len() > 1, extra)?;
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
        let lam = s.wave(wi).mm();
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
            let ph = 2.0 * PI * w;
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
                        ph0: ph + kn * sg * rr,
                    }
                }
                None => {
                    let t = if at_image {
                        0.0
                    } else if far {
                        -dot(sub(q, pc), k)
                    } else {
                        -dot(sub(q, pc), kc) / dot(k, kc)
                    };
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
        // 像の点 p への各光源の位相 ph と振幅 amp（厳密）
        let terms = |p: &V3, ph: &mut Vec<f64>, amp: &mut Vec<f64>| {
            ph.resize(ns, 0.0);
            amp.resize(ns, 0.0);
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
                        ph[j] = ph0[j] - kn * kp;
                    }
                }
            }
        };
        let one = |p: &V3, (ph, amp, z): &mut (Vec<f64>, Vec<f64>, Vec<C64>)| -> f64 {
            terms(p, ph, amp);
            z.resize(ns, C64::new(0.0, 0.0));
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
        // 位相の展開: 区画の 7 点で厳密な位相・振幅を求めて 2 次式を作る。点が像面の形の外なら false
        let fit = |x0: i64, y0: i64, blk: usize, b: &mut ExpandBuf| -> bool {
            let h = (blk / 2) as i64;
            for (k, &(nx, ny)) in NODES.iter().enumerate() {
                let p = at(x0 + nx * h, y0 + ny * h);
                if !p[2].is_finite() {
                    return false;
                }
                terms(&p, &mut b.ph[k], &mut b.amp[k]);
            }
            fit_quad(&b.ph, h as f64, &mut b.cph);
            fit_quad(&b.amp, h as f64, &mut b.camp);
            true
        };
        // GPU（展開の代わり。厳密な和は CPU のまま）
        let gpu_iw = if o.exact {
            None
        } else {
            gpu_sum(
                o.device,
                &GpuInput {
                    sphere,
                    pc,
                    kc,
                    kn,
                    q: [&qx, &qy, &qz],
                    a: [&ax, &ay, &az],
                    f: &qf,
                    ph0: &ph0,
                    pts: &pts,
                },
                &mut gpu_note,
            )?
        };
        // 区画の一辺: 格子の四隅と中央の区画で、7 点でない点の 2 次式と厳密な値の差が許容に入る一番大きい候補
        let block = if !o.exact && gpu_iw.is_none() {
            let mut b = ExpandBuf::default();
            let (mut tp, mut ta) = (Vec::new(), Vec::new());
            EXPAND_BLOCKS.iter().copied().find(|&blk| {
                let (h, q) = ((blk / 2) as i64, (blk / 4) as i64);
                let last = (ni.saturating_sub(blk)) as i64;
                let mid = (ni as i64 - blk as i64).max(0) / 2;
                let corners = [(0, 0), (last, 0), (0, last), (last, last), (mid, mid)];
                corners.iter().all(|&(x0, y0)| {
                    if !fit(x0, y0, blk, &mut b) {
                        return false;
                    }
                    let checks = [
                        (2 * h, 0),
                        (0, 2 * h),
                        (q, q),
                        (3 * q, q),
                        (q, 3 * q),
                        (3 * q, 3 * q),
                    ];
                    checks.iter().all(|&(i, j)| {
                        let p = at(x0 + i, y0 + j);
                        if !p[2].is_finite() {
                            return false;
                        }
                        terms(&p, &mut tp, &mut ta);
                        let (u, v) = ((i - h) as f64, (j - h) as f64);
                        let q2 = |c: &[Vec<f64>; 6], k: usize| {
                            c[0][k]
                                + c[1][k] * u
                                + c[2][k] * v
                                + 0.5 * c[3][k] * u * u
                                + c[5][k] * u * v
                                + 0.5 * c[4][k] * v * v
                        };
                        (0..ns).all(|k| {
                            (q2(&b.cph, k) - tp[k]).abs() <= EXPAND_TOL
                                && (q2(&b.camp, k) - ta[k]).abs() <= EXPAND_TOL * ta[k].abs()
                        })
                    })
                })
            })
        } else {
            None
        };
        // 1 つの区画（隅の点 (x0, y0)、一辺 blk、格子の端では小さい）の強度。行の順
        let one_block = |&(x0, y0): &(usize, usize), blk: usize, b: &mut ExpandBuf| -> Vec<f64> {
            let (bx, by) = (blk.min(ni - x0), blk.min(ni - y0));
            let cell = |i: usize, j: usize| &pts[(y0 + j) * ni + x0 + i];
            if lensbench_core::par::cancelled() {
                return vec![f64::NAN; bx * by];
            }
            let all_on = (0..by).all(|j| (0..bx).all(|i| cell(i, j)[2].is_finite()));
            if !all_on || !fit(x0 as i64, y0 as i64, blk, b) {
                // 像面の形の外の点がある区画は 1 点ずつ厳密に
                return (0..bx * by)
                    .map(|k| one(cell(k % bx, k / bx), &mut b.exact))
                    .collect();
            }
            let h = (blk / 2) as f64;
            to_forward(&mut b.cph, h);
            to_forward(&mut b.camp, h);
            let n = b.cph[0].len();
            b.ones.resize(n, 1.0);
            for (e, c) in b.e.iter_mut().zip(&b.cph) {
                e.resize(n, C64::new(0.0, 0.0));
                lensbench_core::trig::cis_scaled(c, &b.ones, e);
            }
            b.acc.clear();
            b.acc.resize(bx * by, [[0.0; LANES]; 2]);
            sweep(&b.e, &b.camp, bx, by, &mut b.acc);
            b.acc
                .iter()
                .map(|[r, i]| C64::new(r.iter().sum(), i.iter().sum()).norm_sqr())
                .collect()
        };
        used.push(gpu_iw.as_ref().map(|(_, n)| n.clone()));
        let iw: Vec<f64> = if let Some((v, _)) = gpu_iw {
            v
        } else if let Some(blk) = block {
            let origins: Vec<(usize, usize)> = (0..ni)
                .step_by(blk)
                .flat_map(|y0| (0..ni).step_by(blk).map(move |x0| (x0, y0)))
                .collect();
            let parts: Vec<Vec<f64>> =
                if pts.len() * src.len() >= 1 << 18 && !lensbench_core::par::sequential() {
                    use rayon::prelude::*;
                    let cx = lensbench_core::par::current();
                    origins
                        .par_iter()
                        .map_init(ExpandBuf::default, |b, og| {
                            lensbench_core::par::with_ctx(&cx, || one_block(og, blk, b))
                        })
                        .collect()
                } else {
                    let mut b = ExpandBuf::default();
                    origins
                        .iter()
                        .map(|og| one_block(og, blk, &mut b))
                        .collect()
                };
            let mut iw = vec![0.0; pts.len()];
            for (&(x0, y0), part) in origins.iter().zip(&parts) {
                let bx = blk.min(ni - x0);
                for (j, row) in part.chunks(bx).enumerate() {
                    iw[(y0 + j) * ni + x0..][..bx].copy_from_slice(row);
                }
            }
            iw
        } else if pts.len() * src.len() >= 1 << 18 && !lensbench_core::par::sequential() {
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
    let mut warnings = crate::psf::sampling_warnings("Huygens PSF", np, step, enclosed);
    warnings.extend(gpu_note);
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
        device: Some(device_label(&used)),
    })
}

/// 視野ごとの装置の表示 d（[`device_label`]、`, ` 区切り）の項目を、まだ無ければ v に足す
pub(crate) fn push_device(v: &mut Vec<String>, d: Option<&str>) {
    for t in d.into_iter().flat_map(|d| d.split(", ")) {
        if !v.iter().any(|x| x == t) {
            v.push(t.to_string());
        }
    }
}

/// 和を計算した装置の表示: `cpu` か `gpu: アダプタの名前`（波長で違えば `, ` で並べる。重複は除く）
pub(crate) fn device_label(used: &[Option<String>]) -> String {
    let mut v: Vec<String> = Vec::new();
    for u in used {
        let t = match u {
            Some(n) => format!("gpu: {n}"),
            None => "cpu".to_string(),
        };
        if !v.contains(&t) {
            v.push(t);
        }
    }
    if v.is_empty() {
        "cpu".into()
    } else {
        v.join(", ")
    }
}

/// PSF の格子の離散フーリエ変換の絶対値（球欠 = x、子午 = y の周波数）。DC で 1。
/// 格子の幅 W = N Δ の調和周波数 k / W で値を求め、その間は 4 点の Lagrange 補間で（調和周波数でない点で
/// 直接変換すると、格子の端の打ち切りの影響が残る。OpticStudio の Huygens MTF も PSF の FFT）。
/// 格子の間隔 Δ のナイキスト 1/(2Δ) を超える周波数は NaN
pub fn psf_mtf(p: &Psf, freqs: &[f64]) -> TanSag<Vec<f64>> {
    psf_mtf_dir(p, freqs, None)
}

/// PSF の向き e（格子の x・y の単位ベクトル）の周波数の MTF: |Σ I exp(−2πi f (x, y)·e)| / Σ I
fn dir_mtf(p: &Psf, freqs: &[f64], e: [f64; 2]) -> Vec<f64> {
    let (xs, ys) = (p.i.x(), p.i.y());
    let tot: f64 = p.i.rows().flatten().sum();
    freqs
        .iter()
        .map(|&f| {
            let ny = |c: f64, d: f64| (f * c).abs() <= 0.5 / d * (1.0 + 1e-12);
            if !(ny(e[0], p.pixel.0) && ny(e[1], p.pixel.1)) {
                return f64::NAN;
            }
            let mut u = C64::new(0.0, 0.0);
            for (row, &y) in p.i.rows().zip(ys) {
                for (v, &x) in row.iter().zip(xs) {
                    let (sn, cs) = (-2.0 * PI * f * (x * e[0] + y * e[1])).sin_cos();
                    u += C64::new(cs, sn) * *v;
                }
            }
            u.norm() / tot
        })
        .collect()
}

/// [`psf_mtf`] で、子午の向きを t（格子の x・y の単位ベクトル。球欠は (t.y, −t.x)）にする。None なら子午は y、
/// 球欠は x（格子の周辺分布の変換）。t のときは強度の 2 次元の和を向きに沿って直接変換する。ナイキストは向きの
/// 成分ごと（|f t.x| ≤ 1/(2Δx) かつ |f t.y| ≤ 1/(2Δy)）
pub fn psf_mtf_dir(p: &Psf, freqs: &[f64], t: Option<[f64; 2]>) -> TanSag<Vec<f64>> {
    // 強度が NaN の点（像面の形の外など）を含む PSF からは、どの周波数の値も決まらない（変換も規格化の全光量も
    // 格子の全点の和）。全部 NaN にする（下の補間の max(0) で NaN が 0 に化けないよう、先に返す）
    if p.i.rows().flatten().any(|v| v.is_nan()) {
        let nan = vec![f64::NAN; freqs.len()];
        return TanSag {
            tan: nan.clone(),
            sag: nan,
        };
    }
    if let Some(t) = t {
        return TanSag {
            tan: dir_mtf(p, freqs, t),
            sag: dir_mtf(p, freqs, [t[1], -t[0]]),
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
                // ナイキスト 1/(2Δ) を超える周波数は折り返した値になるので出さない
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
                let w = lensbench_core::interp::lagrange::<4>(u);
                (0..4)
                    .map(|a| w[a] * at(i + a as i64))
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

/// Huygens MTF（OpticStudio の Huygens MTF 相当）。周波数 [cycles/レンズ単位] は像面の上。子午・球欠の向きは既定の
/// [`MtfAxes::Field`]（y だけの視野なら子午が y、球欠が x）
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
    huygens_mtf_axes(s, field, w, o, freqs, extra, MtfAxes::default())
}

/// [`huygens_mtf_with`] で、子午・球欠の向きを axes で（[`MtfAxes`]）
pub fn huygens_mtf_axes(
    s: &System,
    field: FieldSel,
    w: Waves,
    o: HuygensOpts,
    freqs: &[f64],
    extra: Option<&dyn PupilAmp>,
    axes: MtfAxes,
) -> Result<Mtf> {
    huygens_mtf_device(s, field, w, o, freqs, extra, axes).map(|(m, _)| m)
}

/// [`huygens_mtf_axes`] と、和を計算した装置（[`Psf::device`] と同じ表示）
pub fn huygens_mtf_device(
    s: &System,
    field: FieldSel,
    w: Waves,
    o: HuygensOpts,
    freqs: &[f64],
    extra: Option<&dyn PupilAmp>,
    axes: MtfAxes,
) -> Result<(Mtf, String)> {
    let w = w.one();
    let p = huygens_psf_with(s, field, Waves::all_or(w), o, extra)?;
    // 像の格子の一部で強度が決まらない（像面の矢高が求まらない点）PSF の MTF は、どの周波数も決まらない。
    // 結果（Mtf）に注意の欄が無いので、NaN の並びを黙って返さず理由つきの誤りにする
    let off = p.i.rows().flatten().filter(|v| v.is_nan()).count();
    if off > 0 {
        invalid!("Huygens MTF: the PSF intensity is undefined at {off} image grid points (the image surface sag can't be found there); reduce the image grid or spacing");
    }
    let TanSag { tan, sag } = psf_mtf_dir(&p, freqs, mtf_tangential(s, field, axes)?);
    let m = Mtf {
        freqs: freqs.to_vec(),
        tan,
        sag,
        cutoff: mtf_cutoff_axes(s, field, w.unwrap_or(s.primary), axes)?,
    };
    Ok((m, p.device.unwrap_or_else(|| "cpu".into())))
}
