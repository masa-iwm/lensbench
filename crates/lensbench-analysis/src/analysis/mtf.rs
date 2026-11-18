//! 回折 MTF: 瞳の自己相関（ずらした 2 つの瞳の重なりの上の位相の和）を求める。
//!
//! 求め方は 4 つ: 直接の求積（[`MtfMethod::Direct`]）、瞳の格子の 6 点補間の上の求積（[`MtfMethod::Interp`]）、
//! FFT の自己相関（[`MtfMethod::Fft`]）、瞳が開口・遮蔽で切れているときの線ごとの求積（[`clipped_otf`]）。
//! 数値の設定（点数・許し・格子の大きさ）はこのファイルの [`T`] の 1 か所にまとめる。

use super::*;

mod clipped;
mod fft;
mod interp;
use clipped::*;
use fft::*;
pub use interp::opd_slope_max;
use interp::*;

/// 回折 MTF の数値の設定（点数・許し・格子の大きさ）。どれも結果の精度と速さの釣り合いで決めた値で、
/// 物理の量ではない。値を変えると結果のビットが変わる
pub(crate) struct Tuning {
    /// 自動の求積の点数 n の、OPD の傾きの最大 g [波 / 半径] あたりの点数。ずらした 2 点の位相差は半径あたり最大 2g の
    /// 速さで回り、Gauss-Legendre は 1 周期あたり 1.5 点ほどで収束しはじめる（2g × 1.5 = 3g）。
    /// 確かめ: g = 38 で n = 96 が 1e-10 に収束
    pub points_fac: f64,
    /// 自動の求積の点数に足す点数（OPD がほぼ平らな系でも重なりのレンズ形の縁の形を解く分）
    pub points_pad: usize,
    /// 自動の求積の点数の下限・上限。上限は OPD の傾きが 700 波 / 半径ほどの系（3g ≈ 2100）でも足りる大きさ
    /// （1024 では高い周波数で MTF が 0.01 ほど浮いた）
    pub points_min: usize,
    pub points_max: usize,
    /// OPD の傾きの最大を見積もる格子の一辺の点数（[`opd_slope_max`]）。間隔 1/16 で、それより細かい OPD の
    /// 波打ち（格子より短い周期）は見えない
    pub slope_grid: usize,
    /// 補間の格子の一辺の区間数の初めの上限。OPD がなめらかなら、6 点補間の誤差は間隔の 6 乗で減るので、
    /// 求積の点数 n が大きくても格子は粗くてよい。足りるかは [`Tuning::interp_tol`] で確かめ、足りなければ n まで
    /// 2 倍ずつ細かくする
    pub interp_grid_start: usize,
    /// 補間の誤差の許し [波]（区間の中点で、直接追跡した OPD との差の最大）。OPD が数百波の系では追跡の丸めで
    /// 1e-9 程度の差が残るので、それより少し上
    pub interp_tol: f64,
    /// 格子を n まで細かくしても補間の差がこれを超える区間は、その区間を直接追跡に回す [波]。OPD の段差（面の
    /// 定義域の縁など）や、格子より細かい形を持つ面（格子の面など）を拾う。これ以下の補間の誤差は MTF に
    /// 1e-6 より小さくしか効かないので拾わない
    pub interp_mask_tol: f64,
    /// 周波数ごとに求積の点数を減らす見積もり（[`shift_points`]）の格子の一辺の点数
    pub shift_grid: usize,
    /// 周波数ごとに点数を減らす自動の n の下限。n が小さい系は見積もり（補間 2·shift_grid² 点）が求積（n² 点）と
    /// 同じくらいかかり、速くならない
    pub shift_min_n: usize,
    /// 位相差の傾き g_D [波 / 半径] あたりの求積の点数。位相が g_D で回る因子を Gauss-Legendre で求めるには
    /// 半径あたり 2π g_D 点ほど要る（6 で 1e-12 に収束した）ので、余裕を見て 8
    pub shift_fac: f64,
    /// [`MtfMethod::Auto`] が FFT に切り替える自動の求積の点数 n。求積は n² 点を周波数ごとに足すので、n が大きいと
    /// 周波数の数によらない FFT のほうが速い（n が 2000 を超える系で 4 倍ほど）
    pub auto_fft_n: usize,
    /// [`MtfMethod::Auto`] が FFT に切り替える周波数の数の下限。n が auto_fft_n を超える系では、周波数がおよそ 7 個で
    /// 求積と FFT の時間が並ぶ
    pub auto_fft_freqs: usize,
    /// [`MtfMethod::Fft`] の瞳の標本の数（直径、2n をこの範囲に収める）
    pub fft_min: usize,
    pub fft_max: usize,
    /// けられた瞳の線ごとの求積で、線に沿って通る区間を探す走査の点の数（ふつうの線・区間の数が変わる所の近くの線）
    pub seg_scan: usize,
    pub seg_scan_fine: usize,
    /// けられた瞳の求積の線の数の下限。ずらした区間どうしの重なり方が変わる所（遮蔽の内側の縁が重なり始めるなど、
    /// 周波数で動く）で線ごとの値が折れるので、OPD がなめらかでも線は多めにとる
    pub lines_min: usize,
    /// 線ごとの区間の数を数える弦の走査の点の数
    pub count_scan: usize,
    /// 区間の数が変わる線の位置を探す角度の走査の線の数
    pub line_scan: usize,
    /// けられた面積（単位円の面積に対する割合、[`clipped_area`]）がこれを超えるときだけ、瞳が切れているとして
    /// 線ごとの求積（[`clipped_otf`]）にする。これ以下のけられ（面の定義域の縁で光線が落ちる細い帯など）は、
    /// ふつうの求積でけられた点を除くだけで MTF に割合ほどしか効かない（線ごとの求積の誤差 3e-4 より小さい）。
    /// 求積点がけられに当たるかどうかで経路を選ぶと、点数や点の置き方で値が跳ぶ
    pub clip_area: f64,
}

/// 回折 MTF の数値の設定（[`Tuning`]）
pub(crate) const T: Tuning = Tuning {
    points_fac: 3.0,
    points_pad: 16,
    points_min: 32,
    points_max: 4096,
    slope_grid: 33,
    interp_grid_start: 256,
    interp_tol: 2e-9,
    interp_mask_tol: 1e-6,
    shift_grid: 33,
    shift_min_n: 128,
    shift_fac: 8.0,
    auto_fft_n: 1024,
    auto_fft_freqs: 8,
    fft_min: 512,
    fft_max: 4096,
    seg_scan: 65,
    seg_scan_fine: 257,
    lines_min: 128,
    count_scan: 257,
    line_scan: 65,
    clip_area: 1e-4,
};

/// MTF（変調度）。freqs [cycles/mm] ごとに子午と球欠（向きは [`MtfAxes`]。既定は子午が視野の向き、y だけの視野では
/// 子午が像面の y・球欠が x の向きの周波数）
#[derive(Debug, Clone)]
pub struct Mtf {
    pub freqs: Vec<f64>,
    pub tan: Vec<f64>,
    pub sag: Vec<f64>,
    /// 回折限界の遮断周波数 [cycles/mm]（計算した波長。多色は主波長。ほかの波長は [`mtf_cutoff`]）
    pub cutoff: TanSag<f64>,
}

/// 単位円を ±d/2 ずらした 2 つの円の重なり（レンズ形）の求積: (ずらす向きの座標 t, 直交する座標 u, 重み)。
/// t は両側の先端で幅が sqrt で 0 になるので、t = a (1 − τ²)（a = 1 − d/2）と置いて τ で Gauss-Legendre、
/// u は各 t で幅いっぱいに Gauss-Legendre。重みの和はレンズの面積（d = 0 で π）
/// [`lens_quad`] の Gauss-Legendre の節点と重み（n ごとに 1 回だけ作る）
pub(crate) struct LensGl {
    gt: Vec<f64>,
    wt: Vec<f64>,
    gu: Vec<f64>,
    wu: Vec<f64>,
}

impl LensGl {
    pub(crate) fn new(n: usize) -> Self {
        let (gt, wt) = gauss_legendre(n.div_ceil(2).max(2));
        let (gu, wu) = gauss_legendre(n.max(2));
        LensGl { gt, wt, gu, wu }
    }
}

pub(crate) fn lens_quad(d: f64, gl: &LensGl) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let a = 1.0 - 0.5 * d;
    let (mut t, mut u, mut w) = (Vec::new(), Vec::new(), Vec::new());
    if a <= 0.0 {
        return (t, u, w);
    }
    let (gt, wt, gu, wu) = (&gl.gt, &gl.wt, &gl.gu, &gl.wu);
    let m = gt.len() * 2 * gu.len();
    t.reserve(m);
    u.reserve(m);
    w.reserve(m);
    for (&z, &wz) in gt.iter().zip(wt) {
        // τ ∈ [0, 1]
        let tau = 0.5 * (z + 1.0);
        let y = a * (1.0 - tau * tau);
        let dy = 2.0 * a * tau * 0.5 * wz;
        let x = (1.0 - (y + 0.5 * d).powi(2)).max(0.0).sqrt();
        for sgn in [1.0, -1.0] {
            for (&g, &wg) in gu.iter().zip(wu) {
                t.push(sgn * y);
                u.push(x * g);
                w.push(dy * x * wg);
            }
        }
    }
    (t, u, w)
}

/// [`lens_quad`] の d = 0（単位円）の重みの和（点を並べずに求める。単位円の面積 π の近似）
/// 瞳の振幅 amp の 2 乗の、ずらさない瞳の [`lens_quad`] の求積（振幅つきの瞳の光量。一様なら [`lens_area`] の値に当たる）
fn amp_area(gl: &LensGl, amp: &FrameAmp) -> f64 {
    let (t, u, w) = lens_quad(0.0, gl);
    (0..t.len())
        .map(|i| w[i] * amp.at(u[i], t[i]).powi(2))
        .sum()
}

/// 組の座標 (u, v)（u は球欠 s、v は子午 t の向き。[`MtfFrame::pupil_xy`]）で引く瞳の振幅
#[derive(Clone, Copy)]
pub(super) struct FrameAmp<'a> {
    a: WaveAmp<'a>,
    fr: MtfFrame,
}

impl FrameAmp<'_> {
    #[inline]
    pub(super) fn at(&self, u: f64, v: f64) -> f64 {
        let (x, y) = self.fr.pupil_xy(u, v);
        self.a.at(x, y)
    }

    pub(super) fn rotationally_symmetric(&self) -> bool {
        self.a.rotationally_symmetric()
    }
}

/// 2 点 a・b の振幅の積を重みにかける（amp が None なら一様で何もしない）
fn weigh(amp: Option<&FrameAmp>, w: &mut [f64], xa: &[f64], ya: &[f64], xb: &[f64], yb: &[f64]) {
    if let Some(a) = amp {
        for i in 0..w.len() {
            w[i] *= a.at(xa[i], ya[i]) * a.at(xb[i], yb[i]);
        }
    }
}

pub(crate) fn lens_area(gl: &LensGl) -> f64 {
    let su: f64 = gl.wu.iter().sum();
    let mut acc = 0.0;
    for (&z, &wz) in gl.gt.iter().zip(&gl.wt) {
        let tau = 0.5 * (z + 1.0);
        let y = 1.0 - tau * tau;
        let dy = tau * wz;
        let x = (1.0 - y * y).max(0.0).sqrt();
        acc += 2.0 * dy * x * su;
    }
    acc
}

/// 周波数 1 cycle/mm あたりの瞳でのずれ (δu, δv)（組の座標）。周波数は像面の上で測る: J は像面の局所座標の
/// 方向余弦 (L, M) の瞳での変化なので、像面が傾いていればその傾きも入る。球欠（dir 0）は u の向きに
/// λ / (∂L'/∂u)、子午（dir 1）は v の向きに λ / (∂M'/∂v) ずらし、J の非対角（斜めの視野での瞳の歪み）は使わない
pub(crate) fn pupil_shift(j: &[[f64; 2]; 2], lam: f64, dir: usize) -> (f64, f64) {
    if dir == 0 {
        (lam / j[0][0], 0.0)
    } else {
        (0.0, lam / j[1][1])
    }
}

/// 求積の点の数の目安を OPD の傾きの最大 g [波 / 半径] から決める: n = `Tuning::points_fac`·g +
/// `Tuning::points_pad`（`Tuning::points_min`〜`Tuning::points_max`）
pub fn mtf_points(g: f64) -> usize {
    ((T.points_fac * g).ceil() as usize + T.points_pad).clamp(T.points_min, T.points_max)
}

/// MTF の瞳の OPD の求め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MtfMethod {
    /// 既定。[`MtfMethod::Interp`]。ただし求積の点数 n を自動で決めて `Tuning::auto_fft_n` を超え（収差が大きい系）、
    /// 周波数が [`AUTO_FFT_FREQS`] 個以上のときは [`MtfMethod::Fft`]
    #[default]
    Auto,
    /// 瞳の格子を 1 度追跡し、求積点では 6 点の Lagrange 補間（格子にけられた点がある所だけ直接追跡）
    Interp,
    /// 求積点をすべて直接追跡（遅い。補間の確かめ用）
    Direct,
    /// 瞳の格子の瞳関数の自己相関を FFT で全周波数まとめて求める（周波数が多いとき速い）
    Fft,
}

/// exp(2πi Δ)
#[inline]
fn phasor(dw: f64) -> C64 {
    let ph = 2.0 * std::f64::consts::PI * dw;
    let (s, c) = ph.sin_cos();
    C64::new(c, s)
}

/// Σ w[i] exp(2πi (a[i] − b[i]))（a・b のどちらかが有限でない点は飛ばす）。位相因子は [`lensbench_core::trig::cis_scaled`] で
/// まとめて求め（libm の sin_cos と相対 1e-13 程度の差）、和は i の順に取る
fn phasor_sum(a: &[f64], b: &[f64], w: &[f64]) -> C64 {
    thread_local! {
        // 作業の配列（位相・重み・位相因子）。呼ぶたびに確保と 0 埋めをしないよう、スレッドごとに使い回す
        // （中身は毎回書き直すので、結果には関係しない）
        static WORK: std::cell::RefCell<(Vec<f64>, Vec<f64>, Vec<C64>)> =
            const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
    }
    WORK.with(|work| {
        let (ph, am, z) = &mut *work.borrow_mut();
        let m = w.len();
        if ph.len() < m {
            ph.resize(m, 0.0);
            am.resize(m, 0.0);
            z.resize(m, C64::new(0.0, 0.0));
        }
        let (a, b) = (&a[..m], &b[..m]);
        let mut n = 0;
        for ((&ai, &bi), &wi) in a.iter().zip(b).zip(w) {
            if ai.is_finite() && bi.is_finite() {
                ph[n] = 2.0 * std::f64::consts::PI * (ai - bi);
                am[n] = wi;
                n += 1;
            }
        }
        let (ph, am, z) = (&ph[..n], &am[..n], &mut z[..n]);
        lensbench_core::trig::cis_scaled(ph, am, z);
        z.iter().fold(C64::new(0.0, 0.0), |acc, &v| acc + v)
    })
}

/// 補間できなかった点（直接追跡する）: (xa, ya, xb, yb, 重み, a, b)。a・b は補間できた値か NaN
type Miss = (f64, f64, f64, f64, f64, f64, f64);

/// [`MtfMethod::Auto`] が FFT に切り替える周波数の数の下限（`Tuning::auto_fft_freqs`）
pub const AUTO_FFT_FREQS: usize = T.auto_fft_freqs;

/// 回折 MTF。瞳関数 P = exp(2πi W)（W は主波長の主光線を基準にした OPD [波]）の自己相関を、ずらした瞳の点で
/// 直接求積する（OPD は [`MtfMethod`] の既定で瞳の格子の補間、FFT は [`MtfMethod::Fft`]）。周波数 ν の縞は、像空間の方向余弦で λν だけずれた
/// 2 つの光線の干渉なので、正規化瞳では [`pupil_shift`] の δν ずらす（瞳から方向余弦への写像は線形とみなす）。w None なら全波長を重みで足した多色（複素 OTF の和、波長ごとに光量で規格化）。
/// 重なりの積分は [`lens_quad`]（n は求積の点の数の目安、点は約 n² 個）
pub fn mtf(s: &System, field: FieldSel, w: Waves, n: Option<usize>, freqs: &[f64]) -> Result<Mtf> {
    let w = w.one();
    mtf_with(s, field, Waves::all_or(w), n, freqs, MtfMethod::default())
}

/// 1 波長の複素 OTF（光量で割る前）: (球欠, 子午, ずらさない瞳の光量 (球欠, 子午))。光量は向きごとに
/// その向きの求積と同じ点と重みで求める
type WaveOtf = (Vec<C64>, Vec<C64>, (f64, f64));

/// 1 波長の複素 OTF（[`WaveOtf`]）
#[allow(clippy::too_many_arguments)]
fn wave_otf(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    only: Option<usize>,
    extra: Option<&dyn PupilAmp>,
    fr: &MtfFrame,
) -> Result<WaveOtf> {
    // 瞳の振幅（アポダイゼーション・膜・偏光）。どこでも 1 なら None（前と同じ計算）
    let wa = FrameAmp {
        a: WaveAmp::new(s, field, wi, extra)?,
        fr: *fr,
    };
    let amp = (!wa.a.is_uniform()).then_some(&wa);
    let auto = n.is_none();
    let (n, first) = pupil_points(s, field, wi, n, fr)?;
    if method == MtfMethod::Fft && !freqs.is_empty()
        || method == MtfMethod::Auto && auto && n > T.auto_fft_n && freqs.len() >= T.auto_fft_freqs
    {
        return wave_otf_fft(s, field, wi, n, first, freqs, amp, fr);
    }
    let jm = fr.jacobian(s, field, wi)?;
    let lam = s.wave(wi).mm();
    // 補間の格子: 求積の点数と同じくらいの細かさ（6 点補間の誤差は h^6 で小さい）
    let ip = match method {
        MtfMethod::Auto | MtfMethod::Interp if !freqs.is_empty() => {
            Some(mtf_interp(s, field, wi, n, fr, first)?)
        }
        _ => None,
    };
    // 軸上の回転対称な系（格子の OPD が対称で、瞳の縮尺が x・y で同じ）なら子午は球欠と同じ
    let sym = ip.as_ref().is_some_and(|ip| axis_symmetric(&jm, ip))
        && amp.is_none_or(|a| a.rotationally_symmetric());
    // only の向きだけ求める（もう一方は 0 のまま。対称なら球欠を求めて子午に写す）
    let dirs: &[usize] = match (sym, only) {
        (true, _) => &[0],
        (false, Some(0)) => &[0],
        (false, Some(_)) => &[1],
        (false, None) => &[0, 1],
    };
    let jobs: Vec<(usize, usize)> = (0..freqs.len())
        .flat_map(|k| dirs.iter().map(move |&d| (k, d)))
        .collect();
    // 瞳の点・ずれ・補間の格子はどれも組の座標 (u, v)（u は球欠 s、v は子午 t の向き。恒等なら正規化瞳の
    // x・y）。追跡と振幅だけが正規化瞳の座標に直す
    // 組の座標でのずれ（dir 0: 球欠、1: 子午の周波数）
    let shift = |f: f64, dir: usize| {
        let (sx, sy) = pupil_shift(&jm, lam, dir);
        (sx * f, sy * f)
    };
    // 補間の格子がすべて有限なら、単位円の中の点はどれも補間できて有限（けられなし）。このときは
    // - 瞳の光量を点ごとの OPD から数えなくてよい（収差の大きい系では、点ごとに数えると求積と同じくらいかかる）
    // - 自動の点数なら、周波数・向きごとにずらしの位相差の傾きで点数を減らす（[`shift_points`]）
    // 和はその点数での瞳の光量（[`lens_area`]）で割り、光量は単位円の面積 π とする（点数による縁の誤差を
    // 打ち消し、周波数 0 で 1）
    let all_finite = ip.as_ref().is_some_and(|ip| ip.all_finite());
    let (gls, nks, e0): (Vec<(usize, LensGl)>, Vec<usize>, f64) = if all_finite {
        let ip = ip.as_ref().expect("pupil sampled above");
        let nks: Vec<usize> = jobs
            .iter()
            .map(|&(k, dir)| {
                let (dx, dy) = shift(freqs[k], dir);
                if auto && n >= T.shift_min_n {
                    shift_points(ip, dx, dy).min(n)
                } else {
                    n
                }
            })
            .collect();
        let mut uniq = nks.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let gls = lensbench_core::par::map(&uniq, |&m| (m, LensGl::new(m)));
        // 振幅つきなら光量は振幅の 2 乗の求積（点数 n で）
        let e0 = amp.map_or(std::f64::consts::PI, |a| amp_area(&LensGl::new(n), a));
        (gls, nks, e0)
    } else {
        // ずらさない瞳の光量（けられを含む）
        let gl = LensGl::new(n);
        let (t0, u0, w0) = lens_quad(0.0, &gl);
        let o0 = opd_at(s, field, wi, fr, ip.as_ref(), &u0, &t0)?;
        let e0: f64 = (0..t0.len())
            .filter(|&i| o0[i].is_finite())
            .map(|i| w0[i] * amp.map_or(1.0, |a| a.at(u0[i], t0[i]).powi(2)))
            .sum();
        if !(e0 > 0.0) {
            invalid!("MTF: no valid rays (all vignetted)");
        }
        // 瞳が開口・遮蔽で切れている（けられた面積がある）なら縁に合わせた求積にする。けられた点があっても
        // 面積が小さければ（[`Tuning::clip_area`]）ふつうの求積でけられた点を除く
        if o0.iter().any(|v| !v.is_finite())
            && clipped_area(s, field, wi, fr, &gl, &o0)? > T.clip_area * std::f64::consts::PI
        {
            return clipped_otf(s, field, wi, ip.as_ref(), n, &jm, lam, freqs, sym, amp, fr);
        }
        (vec![(n, gl)], vec![n; jobs.len()], e0)
    };
    // 周波数・向きごとに独立（並列でも値は同じ。和は周波数・向きの中で順に取る）
    // 周波数 f・向き dir の和（点数 nk）
    let eval = |f: f64, dir: usize, nk: usize| -> Result<C64> {
        // GUI で系が変わったら途中でやめる（周波数・向きごとに確かめる）
        lensbench_core::par::check_cancel()?;
        let (dx, dy) = shift(f, dir);
        let d = dx.hypot(dy);
        // ずらす向き (ex, ey) と直交する向き (−ey, ex)。周波数 0 は、けられた点がありうるとき（補間の格子に
        // 有限でない点がある）はその向きの周波数と同じ点に（光量 e0 の割り算で周波数 0 が 1 になる）
        let (ex, ey) = if d > 0.0 {
            (dx / d, dy / d)
        } else if !all_finite && dir == 1 {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        };
        let gl = &gls.iter().find(|g| g.0 == nk).expect("listed glass").1;
        let scale = match (all_finite, amp) {
            (false, _) => 1.0,
            (true, None) => e0 / lens_area(gl),
            (true, Some(a)) => e0 / amp_area(gl, a),
        };
        if let Some(ip) = &ip {
            if ey == 0.0 || ex == 0.0 {
                let r = shifted_rows(s, field, wi, ip, gl, d, (dx, dy), (ex, ey), amp)?;
                return Ok(r * scale);
            }
        }
        let (t, u, mut w) = lens_quad(d, gl);
        let mut acc = C64::new(0.0, 0.0);
        if t.is_empty() {
            return Ok(acc);
        }
        let m = t.len();
        let (mut xa, mut ya, mut xb, mut yb) =
            (vec![0.0; m], vec![0.0; m], vec![0.0; m], vec![0.0; m]);
        for i in 0..m {
            let (x, y) = (t[i] * ex - u[i] * ey, t[i] * ey + u[i] * ex);
            xa[i] = x + 0.5 * dx;
            ya[i] = y + 0.5 * dy;
            xb[i] = x - 0.5 * dx;
            yb[i] = y - 0.5 * dy;
        }
        let a = opd_at(s, field, wi, fr, ip.as_ref(), &xa, &ya)?;
        let b = opd_at(s, field, wi, fr, ip.as_ref(), &xb, &yb)?;
        weigh(amp, &mut w, &xa, &ya, &xb, &yb);
        acc += phasor_sum(&a, &b, &w);
        Ok(acc * scale)
    };
    let one = |ji: usize| eval(freqs[jobs[ji].0], jobs[ji].1, nks[ji]);
    // けられた点がありうるとき（補間の格子に有限でない点がある）は、光量を向きごとに周波数 0 の和（その向きの
    // 求積と同じ点・同じ補間と直接追跡）にする。ずらさない瞳の点ごとの光量とは、細い帯のけられに点が当たるか
    // どうかで違い、周波数 0 が 1 からずれる
    let e0 = if all_finite {
        (e0, e0)
    } else {
        let e = |dir: usize| eval(0.0, dir, n).map(|c| c.re);
        let s0 = if dirs.contains(&0) { e(0)? } else { e0 };
        let t0 = if sym {
            s0
        } else if dirs.contains(&1) {
            e(1)?
        } else {
            e0
        };
        if !(s0 > 0.0 && t0 > 0.0) {
            invalid!("MTF: no valid rays (all vignetted)");
        }
        (s0, t0)
    };
    let accs: Vec<Result<C64>> = if jobs.len() > 1 && !lensbench_core::par::sequential() {
        use rayon::prelude::*;
        (0..jobs.len()).into_par_iter().map(scoped(one)).collect()
    } else {
        (0..jobs.len()).map(one).collect()
    };
    let zero = C64::new(0.0, 0.0);
    let (mut sag, mut tan) = (vec![zero; freqs.len()], vec![zero; freqs.len()]);
    for (&(k, dir), acc) in jobs.iter().zip(accs) {
        let acc = acc?;
        if dir == 0 {
            sag[k] = acc;
            if sym {
                tan[k] = acc;
            }
        } else {
            tan[k] = acc;
        }
    }
    Ok((sag, tan, e0))
}

/// 波長 wave の回折限界の遮断周波数 [cycles/mm]（瞳を 2 だけずらす周波数）。子午・球欠は既定の向き
/// （[`MtfAxes::Field`]: 子午が視野の向き、球欠がそれに直交する向き）。瞳の向きは [`mtf_cutoff_axes`]
pub fn mtf_cutoff(s: &System, field: FieldSel, wave: usize) -> Result<TanSag<f64>> {
    let fr = MtfFrame::new(s, field, wave, MtfAxes::default())?;
    mtf_cutoff_in(s, field, wave, &fr)
}

/// [`mtf_cutoff`] で、子午・球欠の向きを axes で
pub fn mtf_cutoff_axes(
    s: &System,
    field: FieldSel,
    wave: usize,
    axes: MtfAxes,
) -> Result<TanSag<f64>> {
    let fr = MtfFrame::new(s, field, wave, axes)?;
    mtf_cutoff_in(s, field, wave, &fr)
}

/// 視野 field の MTF の子午の向き（像面の上の単位ベクトル (x, y)。球欠はそれを −90° 回した (y, −x)）。
/// y の向き（[`MtfAxes::Pupil`]・軸上・x = 0 の視野）なら None
pub fn mtf_tangential(s: &System, field: FieldSel, axes: MtfAxes) -> Result<Option<[f64; 2]>> {
    Ok(MtfFrame::new(s, field, s.primary, axes)?.tangential())
}

/// [`mtf_cutoff`] で、子午・球欠の向きを fr で
fn mtf_cutoff_in(s: &System, field: FieldSel, wave: usize, fr: &MtfFrame) -> Result<TanSag<f64>> {
    let jm = fr.jacobian(s, field, wave)?;
    let lam = s.wave(wave).mm();
    let cut = |dir: usize| {
        let (sx, sy) = pupil_shift(&jm, lam, dir);
        2.0 / sx.hypot(sy)
    };
    Ok(TanSag::new(cut(1), cut(0)))
}

/// 波長ごとの複素 OTF（[`wave_otf`] の結果）を重みで足して MTF にする
fn mtf_sum(freqs: &[f64], otfs: &[(f64, &WaveOtf)], cutoff: TanSag<f64>) -> Mtf {
    let mut tan = vec![C64::new(0.0, 0.0); freqs.len()];
    let mut sag = tan.clone();
    let mut wsum = 0.0;
    for &(wt, (sg, tn, e0)) in otfs {
        wsum += wt;
        for k in 0..freqs.len() {
            sag[k] += sg[k] * (wt / e0.0);
            tan[k] += tn[k] * (wt / e0.1);
        }
    }
    Mtf {
        freqs: freqs.to_vec(),
        tan: tan.iter().map(|c| c.norm() / wsum).collect(),
        sag: sag.iter().map(|c| c.norm() / wsum).collect(),
        cutoff,
    }
}

fn check_n(n: Option<usize>) -> Result<()> {
    if let Some(n) = n {
        if n < 4 {
            invalid!("MTF pupil grid {n} must be 4 or more");
        }
    }
    Ok(())
}

/// [`mtf`] で瞳の OPD の求め方を選ぶ
pub fn mtf_with(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
) -> Result<Mtf> {
    let w = w.one();
    mtf_only(s, field, Waves::all_or(w), n, freqs, method, None)
}

/// [`mtf_with`] で、only Some(0) なら球欠、Some(1) なら子午だけを求める（もう一方の値は使えない。評価関数の
/// 片方の向きの項用。けられのある系・FFT では両方求める）
#[doc(hidden)]
pub fn mtf_only(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    only: Option<usize>,
) -> Result<Mtf> {
    mtf_amp(
        s,
        field,
        w,
        n,
        freqs,
        method,
        only,
        None,
        MtfAxes::default(),
    )
}

/// [`mtf_only`] に、膜・偏光の振幅 extra（[`PupilAmp`]、OpticStudio の「Use Polarization」に当たる選択肢）を足す。
/// 系のアポダイゼーション（[`System::apodization`]）はどの MTF にも入る。多色の波長の重みには透過した光量の割合を
/// 掛ける（[`lensbench_core::pupil::wave_weight`]）。axes は子午・球欠の向き（[`MtfAxes`]）
#[allow(clippy::too_many_arguments)]
pub fn mtf_amp(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    only: Option<usize>,
    extra: Option<&dyn PupilAmp>,
    axes: MtfAxes,
) -> Result<Mtf> {
    let w = w.one();
    check_n(n)?;
    let fr = MtfFrame::new(s, field, w.unwrap_or(s.primary), axes)?;
    let ws: Vec<usize> = w.map_or_else(|| (0..s.wavelengths.len()).collect(), |w| vec![w]);
    let mut otfs = Vec::new();
    for &wi in &ws {
        let wt = lensbench_core::pupil::wave_weight(s, field, wi, ws.len() > 1, extra)?;
        if wt > 0.0 {
            otfs.push((
                wt,
                wave_otf(s, field, wi, n, freqs, method, only, extra, &fr)?,
            ));
        }
    }
    if otfs.is_empty() {
        lensbench_core::invalid!("no weighted wavelength"); // 重みの和 0 で割って NaN にしない
    }
    let refs: Vec<(f64, &_)> = otfs.iter().map(|(wt, o)| (*wt, o)).collect();
    let cut = mtf_cutoff_in(s, field, w.unwrap_or(s.primary), &fr)?;
    Ok(mtf_sum(freqs, &refs, cut))
}

/// 多色と各波長の MTF（OpticStudio の表示と同じ組）
#[derive(Debug, Clone)]
pub struct MtfWaves {
    /// 多色（[`mtf`] の w None と同じ値）
    pub poly: Mtf,
    /// 波長ごと（[`mtf`] の w Some(i) と同じ値。重み 0 の波長も含む）
    pub waves: Vec<Mtf>,
}

/// 多色と各波長の MTF を 1 回の計算で求める（波長ごとの OTF を使い回すので、別々に呼ぶより約 2 倍速い）
pub fn mtf_waves(
    s: &System,
    field: FieldSel,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
) -> Result<MtfWaves> {
    mtf_waves_amp(s, field, n, freqs, method, None, MtfAxes::default())
}

/// [`mtf_waves`] に膜・偏光の振幅 extra と子午・球欠の向き axes を足す（[`mtf_amp`]）
pub fn mtf_waves_amp(
    s: &System,
    field: FieldSel,
    n: Option<usize>,
    freqs: &[f64],
    method: MtfMethod,
    extra: Option<&dyn PupilAmp>,
    axes: MtfAxes,
) -> Result<MtfWaves> {
    check_n(n)?;
    let fr = MtfFrame::new(s, field, s.primary, axes)?;
    let (mut otfs, mut cuts) = (Vec::new(), Vec::new());
    for wi in 0..s.wavelengths.len() {
        otfs.push(wave_otf(s, field, wi, n, freqs, method, None, extra, &fr)?);
        cuts.push(mtf_cutoff_in(s, field, wi, &fr)?);
    }
    let multi = otfs.len() > 1;
    let mut poly: Vec<(f64, &_)> = Vec::new();
    for (wi, o) in otfs.iter().enumerate() {
        let wt = lensbench_core::pupil::wave_weight(s, field, wi, multi, extra)?;
        if wt > 0.0 {
            poly.push((wt, o));
        }
    }
    if poly.is_empty() {
        invalid!("MTF: no wavelength with positive weight");
    }
    Ok(MtfWaves {
        poly: mtf_sum(freqs, &poly, cuts[s.primary]),
        waves: otfs
            .iter()
            .zip(cuts)
            .map(|(o, c)| mtf_sum(freqs, &[(1.0, o)], c))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtf_of_a_perfect_lens_is_the_diffraction_limit() {
        // 無限遠の物体、絞り径 10 mm、焦点距離 100 mm の理想レンズ（F/10）、像は焦点
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
                   XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 100\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = lensbench_formats::zmx::parse_text(txt).unwrap();
        let s = lensbench_formats::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let nc = 1.0 / (0.55e-3 * 10.0);
        let freqs: Vec<f64> = (0..10).map(|i| nc * i as f64 / 10.0).collect();
        let m = mtf(&s, FieldSel::Index(0), Waves::All, Some(128), &freqs).unwrap();
        // 遮断周波数は 2 sinθ / λ（sinθ = 5/√(100² + 5²)、F/# からの 1/(λF) より 0.12 % 低い）
        let sin = 5.0 / (100.0f64 * 100.0 + 25.0).sqrt();
        let c = 2.0 * sin / 0.55e-3;
        assert!(
            (m.cutoff.sag - c).abs() / c < 1e-9 && (m.cutoff.tan - c).abs() / c < 1e-9,
            "{:?}",
            m.cutoff
        );
        for (k, &f) in freqs.iter().enumerate() {
            let v: f64 = f / c;
            let ideal = 2.0 / std::f64::consts::PI * (v.acos() - v * (1.0 - v * v).sqrt());
            assert!((m.tan[k] - ideal).abs() < 1e-4, "{v} {} {ideal}", m.tan[k]);
            assert!((m.sag[k] - ideal).abs() < 1e-4, "{v} {} {ideal}", m.sag[k]);
        }
    }

    #[test]
    fn mtf_points_per_shift_match_the_full_count() {
        // 理想レンズ（F/10）の像面を 20 mm ずらす（焦点ずれ約 45 波、自動の n ≈ 290）。周波数ごとに点数を減らした
        // 自動の値と、同じ n で減らさない値は同じ。片方の向きだけ求めても値は同じ
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
                   XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 100\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = lensbench_formats::zmx::parse_text(txt).unwrap();
        let s = lensbench_formats::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let s = crate::sweep::defocus(&s, 20.0).unwrap();
        let freqs = [2.0, 10.0, 60.0];
        for f in 0..2 {
            let fs = FieldSel::Index(f);
            let n = mtf_points(opd_slope_max(&s, fs, 0).unwrap());
            assert!(n >= T.shift_min_n, "{n}");
            let a = mtf(&s, fs, Waves::All, None, &freqs).unwrap();
            let b = mtf(&s, fs, Waves::All, Some(n), &freqs).unwrap();
            let t = mtf_only(&s, fs, Waves::All, None, &freqs, MtfMethod::Auto, Some(1)).unwrap();
            let g = mtf_only(&s, fs, Waves::All, None, &freqs, MtfMethod::Auto, Some(0)).unwrap();
            for k in 0..freqs.len() {
                assert!(
                    (a.tan[k] - b.tan[k]).abs() < 1e-8,
                    "{f} {k} {} {}",
                    a.tan[k],
                    b.tan[k]
                );
                assert!(
                    (a.sag[k] - b.sag[k]).abs() < 1e-8,
                    "{f} {k} {} {}",
                    a.sag[k],
                    b.sag[k]
                );
                assert_eq!(t.tan[k], a.tan[k]);
                assert_eq!(g.sag[k], a.sag[k]);
            }
        }
    }
}
