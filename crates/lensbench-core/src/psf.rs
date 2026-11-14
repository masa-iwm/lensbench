//! 回折 PSF（OpticStudio の FFT PSF 相当）と幾何 MTF（Geometric MTF 相当）。
//!
//! PSF は回折 MTF（[`crate::analysis::mtf`]）と同じ瞳関数 P = A exp(2πi W)（W は主波長の主光線を基準にした
//! OPD [波]、A はけられで 0）から、像面の格子の点で直接フーリエ変換して求める（行列の DFT。FFT と同じ値を
//! 任意の間隔で出せる）。像の座標は主光線に垂直な面（OpticStudio の FFT PSF と同じ）で、主波長の主光線の
//! 向き k に垂直な単位ベクトル e1（x 寄り）・e2（y 寄り）に沿う。瞳から像への写像は MTF と同じく対角だけを使い、
//! 正規化瞳 (px, py) の向きの光線の向きの傾き ∂d/∂px・∂d/∂py を e1・e2 に射影したもの（斜めの視野では
//! 像面上より 1/cos だけ細かい。OS の FFT PSF と照合して確かめた）。
//!
//! 幾何 MTF は像面に届いた光線の位置の分布の特性関数 |Σ w exp(2πi ν x)| / Σ w（瞳の重なり求積と同じ
//! Gauss-Legendre の求積で光線を選ぶ）。球欠は x、子午は y の周波数。

use crate::analysis::{lens_quad, opd, pupil_jacobian, pupil_shift, LensGl};
use crate::coating::C64;
use crate::error::Result;
use crate::invalid;
use crate::par::scoped;
use crate::system::{FieldSel, System};

/// 回折 PSF
#[derive(Debug, Clone)]
pub struct Psf {
    /// 格子の座標 [レンズ単位]（中心 = 主波長の主光線の像点、主光線に垂直な面の e1・e2 に沿う）。x[m] = (m − M/2)·delta.0。
    /// アフォーカル像空間では向きの角度 [System::afocal_unit]（atan(l/n)・atan(m/n) の主光線との差、center・delta も同じ）
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// 強度 I[iy][ix]。収差のない同じ瞳（けられ込み）の多色 PSF の中心の値を 1 とする（ピークはストレール比）
    pub i: Vec<Vec<f64>>,
    /// 主波長の主光線の像点（像面の局所座標）
    pub center: (f64, f64),
    pub delta: (f64, f64),
    /// 格子の x・y の向き e1・e2（像空間の単位ベクトル、主波長の主光線に垂直）。像面上の点は
    /// v = x e1 + y e2 を主光線の向き k = e1 × e2 に沿って像面へ移した v − k v_z / k_z（アフォーカル像空間では使わない）
    pub axes: [[f64; 3]; 2],
    /// 格子の最大値（ストレール比の目安）
    pub peak: f64,
    /// 使った瞳の格子の一辺（自動ならその値）
    pub pupil: usize,
    /// 瞳の隣り合う標本の位相差の最大 [rad]（使う波長のうち最大）。π を超えると像が折り返して重なり、値は格子で
    /// 変わる（[`PHASE_STEP_WARN`] を超えたら warnings に入れる）
    pub phase_step: f64,
    /// 像の格子に入った光量の割合（全光量に対する、重みつきの波長の平均）。小さければ格子が PSF を覆えていない
    pub enclosed: f64,
    /// 標本不足・格子の外の光量の注意（空なら問題なし）
    pub warnings: Vec<String>,
}

/// 瞳の隣り合う標本の位相差の注意のしきい値 [rad]
pub const PHASE_STEP_WARN: f64 = std::f64::consts::FRAC_PI_2;
/// 像の格子に入った光量の割合の注意のしきい値
pub const ENCLOSED_WARN: f64 = 0.95;

/// 瞳の格子の一辺の自動の値: 隣り合う標本の位相差が [`PHASE_STEP_WARN`] 以下になるよう OPD の傾きの最大 g
/// [波 / 半径] から n ≥ 8g（標本の間隔 2/n で位相差 2π·g·2/n ≤ π/2）。64〜cap。加えて像の格子の幅が瞳の標本の
/// 周期を超えない大きさ（need）以上
pub(crate) fn auto_pupil(
    s: &System,
    field: FieldSel,
    ws: &[usize],
    need: usize,
    cap: usize,
) -> Result<usize> {
    let mut g: f64 = 0.0;
    for &wi in ws {
        g = g.max(crate::analysis::opd_slope_max(s, field, wi)?);
    }
    let n = if g.is_finite() {
        ((8.0 * g).ceil() as usize).next_multiple_of(16)
    } else {
        cap
    };
    Ok(n.max(need).clamp(64, cap.max(need)))
}

/// 瞳の格子 (ix, iy) の OPD [波]（使えない標本は NaN）から、隣り合う標本の位相差の最大 [rad]
pub(crate) fn phase_step(np: usize, w: &[(usize, usize, f64)]) -> f64 {
    let mut g = vec![f64::NAN; np * np];
    for &(ix, iy, v) in w {
        g[iy * np + ix] = v;
    }
    let mut m: f64 = 0.0;
    for iy in 0..np {
        for ix in 0..np {
            let v = g[iy * np + ix];
            if !v.is_finite() {
                continue;
            }
            if ix + 1 < np && g[iy * np + ix + 1].is_finite() {
                m = m.max((g[iy * np + ix + 1] - v).abs());
            }
            if iy + 1 < np && g[(iy + 1) * np + ix].is_finite() {
                m = m.max((g[(iy + 1) * np + ix] - v).abs());
            }
        }
    }
    2.0 * std::f64::consts::PI * m
}

/// 標本不足・格子の外の光量の注意の文
pub(crate) fn sampling_warnings(what: &str, np: usize, step: f64, enclosed: f64) -> Vec<String> {
    let mut w = Vec::new();
    if step > PHASE_STEP_WARN {
        w.push(format!(
            "{what}: 瞳の隣り合う標本の位相差が最大 {step:.2} rad（瞳の格子 {np}）。π/2 を超えると値が瞳の格子で変わる（瞳の格子を増やす）"
        ));
    }
    if enclosed < ENCLOSED_WARN {
        w.push(format!(
            "{what}: 像の格子に入った光量が全体の {:.1} %。格子が PSF を覆えていない（像の格子・間隔を増やす）",
            enclosed * 100.0
        ));
    }
    w
}

/// PSF の設定
#[derive(Debug, Clone, Copy)]
pub struct PsfOpts {
    /// 瞳の格子の一辺（直径を n 区間に分けた各区間の中心で標本化）。None で自動（隣り合う標本の位相差が π/2 以下に
    /// なる大きさ、64〜2048。超えたら格子を増やして求め直す）
    pub pupil: Option<usize>,
    /// 像の格子の一辺
    pub image: usize,
    /// 像の格子の間隔 [レンズ単位]（None で 1 / (2 × 遮断周波数)、x・y 別、使う波長のうち最も短い波長で。
    /// 単色なら OpticStudio の FFT PSF で像の格子 = 瞳の格子 × 2 のときと同じ）。
    /// 像の格子の幅は瞳の標本の周期を超えられない（間隔が既定なら image ≤ 2 pupil）
    pub delta: Option<f64>,
}

impl Default for PsfOpts {
    fn default() -> Self {
        PsfOpts {
            pupil: None,
            image: 128,
            delta: None,
        }
    }
}

/// 瞳の区間 [a, a + h]² の単位円内の面積の割合（8×8 の小区間の中心で数える）と、円内の部分の中心の近似
pub(crate) fn cell_frac(cx: f64, cy: f64, h: f64) -> (f64, f64, f64) {
    let r0 = cx.hypot(cy);
    let half = 0.5 * h * std::f64::consts::SQRT_2;
    if r0 + half <= 1.0 {
        return (1.0, cx, cy);
    }
    if r0 - half >= 1.0 {
        return (0.0, cx, cy);
    }
    let k = 8;
    let (mut n, mut sx, mut sy) = (0usize, 0.0, 0.0);
    for a in 0..k {
        for b in 0..k {
            let x = cx + h * ((a as f64 + 0.5) / k as f64 - 0.5);
            let y = cy + h * ((b as f64 + 0.5) / k as f64 - 0.5);
            if x * x + y * y <= 1.0 {
                n += 1;
                sx += x;
                sy += y;
            }
        }
    }
    if n == 0 {
        return (0.0, cx, cy);
    }
    let f = n as f64 / (k * k) as f64;
    (f, sx / n as f64, sy / n as f64)
}

/// 主光線（向き k）に垂直な面の単位ベクトル e1（x 寄り）・e2（y 寄り）
fn perp_basis(k: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f64; 3]| {
        let n = dot(a, a).sqrt();
        [a[0] / n, a[1] / n, a[2] / n]
    };
    // e2 = ŷ から k の成分を除いたもの、e1 = e2 × k（k = ẑ なら x̂・ŷ）
    let e2 = unit([-k[1] * k[0], 1.0 - k[1] * k[1], -k[1] * k[2]]);
    let e1 = [
        e2[1] * k[2] - e2[2] * k[1],
        e2[2] * k[0] - e2[0] * k[2],
        e2[0] * k[1] - e2[1] * k[0],
    ];
    (e1, e2)
}

/// 瞳の px・py の向きの、主光線に垂直な面での光線の向きの傾き (∂d/∂px·e1, ∂d/∂py·e2)。
/// J = ∂(L, M)/∂(px, py) から ∂N = −(L ∂L + M ∂M)/N（主光線で）。アフォーカル像空間では J がすでに
/// 角度の座標（atan(l/n)・atan(m/n)）の値なので、対角をそのまま使う
pub(crate) fn perp_slopes_of(s: &System, jm: [[f64; 2]; 2], k: [f64; 3]) -> (f64, f64) {
    if s.afocal {
        (jm[0][0], jm[1][1])
    } else {
        perp_slopes(jm, k)
    }
}

fn perp_slopes(jm: [[f64; 2]; 2], k: [f64; 3]) -> (f64, f64) {
    let (e1, e2) = perp_basis(k);
    let v = |c: usize| {
        let (dl, dm) = (jm[0][c], jm[1][c]);
        [dl, dm, -(k[0] * dl + k[1] * dm) / k[2]]
    };
    let (vx, vy) = (v(0), v(1));
    (
        vx[0] * e1[0] + vx[1] * e1[1] + vx[2] * e1[2],
        vy[0] * e2[0] + vy[1] * e2[1] + vy[2] * e2[2],
    )
}

/// 回折 PSF。w None なら全波長を重みで足した多色（各波長の全強度を重みに比例させる）
pub fn psf(s: &System, field: FieldSel, w: Option<usize>, o: PsfOpts) -> Result<Psf> {
    retry_auto(o.pupil, PSF_PUPIL_CAP, |np| {
        psf_once(s, field, w, PsfOpts { pupil: np, ..o })
    })
}

/// 自動の瞳の格子の上限（FFT PSF）
const PSF_PUPIL_CAP: usize = 2048;

/// 瞳の格子が自動（pupil = None）なら、見積もりで求めた結果の位相差が [`PHASE_STEP_WARN`] を超えたとき、超えた比の
/// 1.2 倍に格子を増やして求め直す（上限 cap まで、3 回まで）。見積もり（33×33 の OPD の傾き）は格子の上の差より少し
/// 小さく出るため
pub(crate) fn retry_auto(
    pupil: Option<usize>,
    cap: usize,
    run: impl Fn(Option<usize>) -> Result<Psf>,
) -> Result<Psf> {
    let mut r = run(pupil)?;
    if pupil.is_some() {
        return Ok(r);
    }
    for _ in 0..3 {
        if r.phase_step <= PHASE_STEP_WARN || r.pupil >= cap {
            break;
        }
        let n = ((r.pupil as f64 * r.phase_step / PHASE_STEP_WARN * 1.2).ceil() as usize)
            .next_multiple_of(16)
            .min(cap);
        r = run(Some(n))?;
    }
    Ok(r)
}

fn psf_once(s: &System, field: FieldSel, w: Option<usize>, o: PsfOpts) -> Result<Psf> {
    let ni = o.image;
    if o.pupil.is_some_and(|np| np < 4) || ni < 2 {
        invalid!(
            "PSF の瞳の格子 {:?} は 4 以上、像の格子 {ni} は 2 以上",
            o.pupil
        );
    }
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    // 主波長の主光線（像の格子の中心と、垂直な面の向き）
    let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
    // 基準の光線なので開口は無視する。届かないなら中心は NaN、垂直な面は像面とする
    let (center, kc) = if c.traced(0) {
        (s.image_point(c.rays[0].xyz, c.rays[0].lmn), c.rays[0].lmn)
    } else {
        ((f64::NAN, f64::NAN), [0.0, 0.0, 1.0])
    };
    if !(kc[2].abs() > 1e-9) {
        invalid!("PSF: 主光線が像面に平行");
    }
    let (e1, e2) = perp_basis(kc);
    // 遮断周波数（x・y）の、使う波長のうちの最大（短い波長）
    let mut cut: (f64, f64) = (0.0, 0.0);
    for &wi in &ws {
        if ws.len() > 1 && !(s.weights[wi] > 0.0) {
            continue;
        }
        let (ga, gb) = perp_slopes_of(s, pupil_jacobian(s, field, wi)?, kc);
        let lam = s.wavelengths[wi] * 1e-3;
        cut.0 = cut.0.max(2.0 * (ga / lam).abs());
        cut.1 = cut.1.max(2.0 * (gb / lam).abs());
    }
    let delta = match o.delta {
        Some(d) if d > 0.0 => (d, d),
        Some(d) => invalid!("PSF の像の格子の間隔 {d}"),
        None => (0.5 / cut.0, 0.5 / cut.1),
    };
    let half = (ni / 2) as f64;
    let xs: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * delta.0).collect();
    let ys: Vec<f64> = (0..ni).map(|m| (m as f64 - half) * delta.1).collect();
    let np = match o.pupil {
        Some(n) => n,
        None => {
            let ws: Vec<usize> = ws
                .iter()
                .copied()
                .filter(|&wi| ws.len() == 1 || s.weights[wi] > 0.0)
                .collect();
            // 像の格子の幅が瞳の標本の周期を超えない大きさ（ni·Δ·|a|·h ≤ 1、h = 2/n）
            let need = (ni as f64 * (delta.0 * cut.0).max(delta.1 * cut.1)).ceil() as usize;
            auto_pupil(s, field, &ws, need, PSF_PUPIL_CAP)?
        }
    };

    // 瞳の格子（区間の中心）と円内の割合
    let h = 2.0 / np as f64;
    let mut cells = Vec::new(); // (ix, iy, 面積の割合, 追跡する点)
    for iy in 0..np {
        for ix in 0..np {
            let cx = -1.0 + h * (ix as f64 + 0.5);
            let cy = -1.0 + h * (iy as f64 + 0.5);
            let (f, qx, qy) = cell_frac(cx, cy, h);
            if f > 0.0 {
                cells.push((ix, iy, f, qx, qy));
            }
        }
    }
    let px: Vec<f64> = cells.iter().map(|c| c.3).collect();
    let py: Vec<f64> = cells.iter().map(|c| c.4).collect();
    let pcx: Vec<f64> = (0..np).map(|i| -1.0 + h * (i as f64 + 0.5)).collect();

    let mut tot = vec![vec![0.0; ni]; ni];
    let (mut norm, mut wsum) = (0.0, 0.0);
    let (mut step, mut encl): (f64, f64) = (0.0, 0.0);
    for &wi in &ws {
        crate::par::check_cancel()?;
        let k = if ws.len() == 1 { 1.0 } else { s.weights[wi] };
        if !(k > 0.0) {
            continue;
        }
        let (ga, gb) = perp_slopes_of(s, pupil_jacobian(s, field, wi)?, kc);
        let lam = s.wavelengths[wi] * 1e-3;
        // 位相の係数: exp(−2πi (a px x + b py y))、a = (∂d/∂px·e1) / λ
        let (a, b) = (ga / lam, gb / lam);
        // 瞳の標本の間隔 h で像の DFT は周期 1 / (a h) で繰り返すので、像の格子がそれを超えると重なる
        // （OpticStudio の FFT PSF で像の格子 = 瞳の格子 × 2 のときがちょうど 1 周期）
        if ni as f64 * delta.0 * a.abs() * h > 1.0 + 1e-9
            || ni as f64 * delta.1 * b.abs() * h > 1.0 + 1e-9
        {
            invalid!(
                "PSF: 像の格子の幅が瞳の標本の周期を超える（波長 {}、瞳の格子を増やすか、像の格子・間隔を減らす）",
                wi + 1
            );
        }
        let op = opd(s, field, &px, &py, Some(wi), true)?;
        // 瞳の格子の複素振幅（面積の割合を重みに）
        let mut p = vec![vec![C64::new(0.0, 0.0); np]; np];
        let mut sa = 0.0;
        for (q, &(ix, iy, f, _, _)) in cells.iter().enumerate() {
            if op.ok(q) && op.opd[q].is_finite() {
                let ph = 2.0 * std::f64::consts::PI * op.opd[q];
                p[iy][ix] = C64::new(ph.cos(), ph.sin()) * f;
                sa += f;
            }
        }
        if !(sa > 0.0) {
            invalid!("PSF: 有効な光線がない（すべてけられた）");
        }
        let w_ok: Vec<(usize, usize, f64)> = cells
            .iter()
            .enumerate()
            .filter(|&(q, _)| op.ok(q) && op.opd[q].is_finite())
            .map(|(q, c)| (c.0, c.1, op.opd[q]))
            .collect();
        step = step.max(phase_step(np, &w_ok));
        // 行列の DFT: U[y][x] = Σ_iy e^{−2πi b py y} Σ_ix e^{−2πi a px x} P[iy][ix]
        let ex: Vec<Vec<C64>> = xs
            .iter()
            .map(|&x| {
                pcx.iter()
                    .map(|&u| {
                        let t = -2.0 * std::f64::consts::PI * a * u * x;
                        C64::new(t.cos(), t.sin())
                    })
                    .collect()
            })
            .collect();
        let ey: Vec<Vec<C64>> = ys
            .iter()
            .map(|&y| {
                pcx.iter()
                    .map(|&v| {
                        let t = -2.0 * std::f64::consts::PI * b * v * y;
                        C64::new(t.cos(), t.sin())
                    })
                    .collect()
            })
            .collect();
        // 1 段目: 各瞳の行 iy について x 方向の変換 G[iy][mx]
        let g: Vec<Vec<C64>> = p
            .iter()
            .map(|row| {
                ex.iter()
                    .map(|e| row.iter().zip(e).map(|(a, b)| a * b).sum())
                    .collect()
            })
            .collect();
        let one = |my: usize| -> Vec<f64> {
            (0..ni)
                .map(|mx| {
                    let mut u = C64::new(0.0, 0.0);
                    for iy in 0..np {
                        u += g[iy][mx] * ey[my][iy];
                    }
                    u.norm_sqr()
                })
                .collect()
        };
        let iw: Vec<Vec<f64>> = if ni * ni * np >= 1 << 20 && !crate::par::sequential() {
            use rayon::prelude::*;
            (0..ni).into_par_iter().map(scoped(one)).collect()
        } else {
            (0..ni).map(one).collect()
        };
        // 格子の点の強度は |Σ P|²（U の連続の積分の 1/h² 倍）。全強度は Parseval で
        // ∫|U|² dx dy = ∫|P|² dp / |a b| なので、同じ単位で Σ f / |a b h²|
        let e_w = sa / (a * b * h * h).abs();
        // 収差なしの中心の値 |Σ A|²（全強度あたり）
        let dl = sa * sa / e_w;
        for (t, r) in tot.iter_mut().zip(&iw) {
            for (a, v) in t.iter_mut().zip(r) {
                *a += k * v / e_w;
            }
        }
        // 格子に入った光量（格子の点の強度 × 1 点の面積）の全光量に対する割合
        encl += k * iw.iter().flatten().sum::<f64>() * delta.0 * delta.1 / e_w;
        norm += k * dl;
        wsum += k;
    }
    if !(wsum > 0.0) {
        invalid!("重みのある波長がない");
    }
    for r in tot.iter_mut() {
        for v in r.iter_mut() {
            *v /= norm;
        }
    }
    let peak = tot.iter().flatten().cloned().fold(0.0, f64::max);
    let enclosed = encl / wsum;
    Ok(Psf {
        x: xs,
        y: ys,
        i: tot,
        center,
        delta,
        axes: [e1, e2],
        peak,
        pupil: np,
        phase_step: step,
        enclosed,
        warnings: sampling_warnings("PSF", np, step, enclosed),
    })
}

// ------------------------------------------------------------------------------------------ 幾何 MTF
/// 幾何 MTF の結果（[`crate::analysis::Mtf`] と同じ形）
pub use crate::analysis::Mtf;

/// 円形瞳の回折限界の MTF（v = ν / 遮断周波数）
pub fn diffraction_limit(v: f64) -> f64 {
    let v = v.abs();
    if v >= 1.0 {
        return 0.0;
    }
    2.0 / std::f64::consts::PI * (v.acos() - v * (1.0 - v * v).sqrt())
}

/// 幾何 MTF（OpticStudio の Geometric MTF 相当）。n は瞳の求積の点の数の目安（点は約 n² 個）。
/// diff_limit なら波長ごとの回折限界の MTF をかける（OpticStudio の「Multiply by Diffraction Limit」）。
/// w None なら多色（全波長の光線をまとめたスポットで。波長ごとに有効な光線の重みの和を波長の重みにそろえる）
pub fn geometric_mtf(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    n: usize,
    freqs: &[f64],
    diff_limit: bool,
) -> Result<Mtf> {
    if n < 4 {
        invalid!("幾何 MTF の求積の点の数 {n} は 4 以上");
    }
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let gl = LensGl::new(n);
    let (t, u, wq) = lens_quad(0.0, &gl);
    // 波長ごとの (重み / 有効な光線の重みの和, 光線の位置と求積の重み, 遮断周波数)
    let mut sets = Vec::new();
    for &wi in &ws {
        crate::par::check_cancel()?;
        let k = if ws.len() == 1 { 1.0 } else { s.weights[wi] };
        if !(k > 0.0) {
            continue;
        }
        let r = s.try_trace(field, &u, &t, Some(wi), None, false)?;
        let pts: Vec<(f64, f64, f64)> = (0..t.len())
            .filter(|&i| r.ok(i))
            .map(|i| {
                let (x, y) = s.image_point(r.rays[i].xyz, r.rays[i].lmn);
                (x, y, wq[i])
            })
            .collect();
        let e: f64 = pts.iter().map(|p| p.2).sum();
        if pts.is_empty() || !(e > 0.0) {
            invalid!("幾何 MTF: 有効な光線がない（すべてけられた）");
        }
        let cut = if diff_limit {
            let jm = pupil_jacobian(s, field, wi)?;
            let lam = s.wavelengths[wi] * 1e-3;
            let c = |dir: usize| {
                let (a, b) = pupil_shift(&jm, lam, dir);
                2.0 / a.hypot(b)
            };
            (c(0), c(1))
        } else {
            (f64::INFINITY, f64::INFINITY)
        };
        sets.push((k / e, pts, cut));
    }
    let wsum: f64 = sets
        .iter()
        .map(|x| x.0 * x.1.iter().map(|p| p.2).sum::<f64>())
        .sum();
    if sets.is_empty() || !(wsum > 0.0) {
        invalid!("重みのある波長がない");
    }
    // 全波長の光線をまとめたスポットの特性関数（位置は全体の重心から測る。絶対値には効かない）
    let (mut mx, mut my) = (0.0, 0.0);
    for (k, pts, _) in &sets {
        for p in pts {
            mx += k * p.2 * p.0;
            my += k * p.2 * p.1;
        }
    }
    let (mx, my) = (mx / wsum, my / wsum);
    let mut tan = vec![0.0; freqs.len()];
    let mut sag = tan.clone();
    for (q, &f) in freqs.iter().enumerate() {
        let ph = 2.0 * std::f64::consts::PI * f;
        let (mut a, mut b) = (C64::new(0.0, 0.0), C64::new(0.0, 0.0));
        for (k, pts, cut) in &sets {
            let (mut aw, mut bw) = (C64::new(0.0, 0.0), C64::new(0.0, 0.0));
            for &(x, y, wq) in pts {
                let (x, y) = (x - mx, y - my);
                aw += C64::new((ph * x).cos(), (ph * x).sin()) * wq;
                bw += C64::new((ph * y).cos(), (ph * y).sin()) * wq;
            }
            let (ds, dt) = if diff_limit {
                (diffraction_limit(f / cut.0), diffraction_limit(f / cut.1))
            } else {
                (1.0, 1.0)
            };
            a += aw * (k * ds);
            b += bw * (k * dt);
        }
        sag[q] = a.norm() / wsum;
        tan[q] = b.norm() / wsum;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffraction_limit_ends() {
        assert!((diffraction_limit(0.0) - 1.0).abs() < 1e-15);
        assert_eq!(diffraction_limit(1.0), 0.0);
        // 負の周波数は絶対値で（v ≤ −1 で NaN にならない）
        assert_eq!(diffraction_limit(-1.0), 0.0);
        assert_eq!(diffraction_limit(-2.5), 0.0);
        assert_eq!(diffraction_limit(-0.5), diffraction_limit(0.5));
        assert!((diffraction_limit(0.5) - 0.391_002_218_955_6).abs() < 1e-12);
    }

    #[test]
    fn cell_fraction_sums_to_pi() {
        let n = 64;
        let h = 2.0 / n as f64;
        let mut a = 0.0;
        for iy in 0..n {
            for ix in 0..n {
                let cx = -1.0 + h * (ix as f64 + 0.5);
                let cy = -1.0 + h * (iy as f64 + 0.5);
                a += cell_frac(cx, cy, h).0 * h * h;
            }
        }
        assert!((a - std::f64::consts::PI).abs() < 2e-4, "{a}");
    }
}

#[cfg(test)]
mod perp_tests {
    use super::*;

    #[test]
    fn perp_slopes_divide_by_cos_for_a_tilted_chief_ray() {
        // 軸上（k = ẑ）は J の対角のまま
        let jm = [[0.1, 0.0], [0.0, 0.2]];
        let (a, b) = perp_slopes(jm, [0.0, 0.0, 1.0]);
        assert!((a - 0.1).abs() < 1e-15 && (b - 0.2).abs() < 1e-15);
        // y-z 面で傾いた主光線: y は 1/cos、x はそのまま
        let t: f64 = 0.35;
        let (a, b) = perp_slopes(jm, [0.0, t.sin(), t.cos()]);
        assert!((a - 0.1).abs() < 1e-15);
        assert!((b - 0.2 / t.cos()).abs() < 1e-14);
        // 基底は直交で k に垂直
        let k = [0.2, -0.3, (1.0f64 - 0.13).sqrt()];
        let (e1, e2) = perp_basis(k);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for (u, v) in [(e1, k), (e2, k), (e1, e2)] {
            assert!(dot(u, v).abs() < 1e-15);
        }
        assert!(e1[0] > 0.9 && e2[1] > 0.9);
    }
}
