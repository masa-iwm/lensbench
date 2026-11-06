//! まとめて計算する sin・cos（干渉の計算の位相因子 exp(iφ) 用）。
//!
//! libm の sin・cos を 1 個ずつ呼ぶ代わりに、分岐のない多項式で 8 個ずつ計算する（コンパイラが SIMD にする）。
//! - 引数の簡約: q = round(φ / (π/2))、r = φ − q·(π/2)（π/2 を 3 つに分けた Cody–Waite、fdlibm の値）。
//!   |q| < 2^20 なら q·(π/2 の上位) が丸めなしで、r の誤差は r の 1 ulp 程度。
//! - |r| ≤ π/4 の sin・cos は fdlibm の __kernel_sin・__kernel_cos と同じ係数（誤差 1〜2 ulp）。
//! - |φ| が [`MAX_ARG`] を超える・NaN・無限大を含む 8 個は libm に戻す。
//!
//! 結果は libm と最下位の数ビット違うことがある（φ の丸め誤差 |φ|·1e-16 と同程度）。
//! 浮動小数の演算の順は固定なので、スレッド数によらず同じビットになる。

// 係数は fdlibm の値をそのまま書く
#![allow(clippy::excessive_precision)]

use num_complex::Complex64;

/// 多項式で計算する |φ| の上限（q·(π/2 の上位 33 ビット) が丸めなしになる範囲）。
pub const MAX_ARG: f64 = 1.0e6;

const INV_PIO2: f64 = std::f64::consts::FRAC_2_PI;
const PIO2_1: f64 = 1.57079632673412561417e+00; // 上位 33 ビット
const PIO2_2: f64 = 6.07710050630396597660e-11; // 次の 33 ビット
const PIO2_2T: f64 = 2.02226624879595063154e-21; // 残り
const ROUND: f64 = 6755399441055744.0; // 1.5·2^52: 足して引くと最も近い整数に丸まる

const S1: f64 = -1.66666666666666324348e-01;
const S2: f64 = 8.33333333332248946124e-03;
const S3: f64 = -1.98412698298579493134e-04;
const S4: f64 = 2.75573137070700676789e-06;
const S5: f64 = -2.50507602534068634195e-08;
const S6: f64 = 1.58969099521155010221e-10;

const C1: f64 = 4.16666666666666019037e-02;
const C2: f64 = -1.38888888888741095749e-03;
const C3: f64 = 2.48015872894767294178e-05;
const C4: f64 = -2.75573143513906633035e-07;
const C5: f64 = 2.08757232129817482790e-09;
const C6: f64 = -1.13596475577881948265e-11;

const LANES: usize = 8;

/// 1 個分（|φ| ≤ MAX_ARG を前提）。返り値は (sin φ, cos φ)。
/// 象限の入れ替え・符号はビット演算で行い、分岐なしにする（8 個まとめて SIMD になるように）。
#[inline(always)]
fn sincos_poly(x: f64) -> (f64, f64) {
    let t = x * INV_PIO2 + ROUND;
    // t の仮数の下位ビットが q（2 の補数）
    let q = t.to_bits();
    let qf = t - ROUND;
    let r = ((x - qf * PIO2_1) - qf * PIO2_2) - qf * PIO2_2T;
    let z = r * r;
    let s = r + r * z * (S1 + z * (S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)))));
    // cos r = 1 − z/2 + z²·P(z)。1 − z/2 の丸めを fdlibm と同じく補正する
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    let c =
        w + (((1.0 - w) - hz) + z * z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6))))));
    // 象限: q mod 4 = 0: (s, c), 1: (c, −s), 2: (−s, −c), 3: (−c, s)
    let swap = 0u64.wrapping_sub(q & 1);
    let (sb, cb) = (s.to_bits(), c.to_bits());
    let a = (cb & swap) | (sb & !swap);
    let b = (sb & swap) | (cb & !swap);
    let sin_neg = (q & 2) << 62;
    let cos_neg = (q.wrapping_add(1) & 2) << 62;
    (f64::from_bits(a ^ sin_neg), f64::from_bits(b ^ cos_neg))
}

/// out[j] = amp[j] · exp(i φ[j])。3 つの長さは同じ（短い方に合わせる）。
/// AVX2 のある CPU では 256 ビットの命令で計算する（演算は同じなので結果は同じビット）。
pub fn cis_scaled(phase: &[f64], amp: &[f64], out: &mut [Complex64]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // 安全: AVX2 があることを上で確かめた
            return unsafe { cis_avx2(phase, amp, out) };
        }
    }
    cis_impl(phase, amp, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn cis_avx2(phase: &[f64], amp: &[f64], out: &mut [Complex64]) {
    cis_impl(phase, amp, out)
}

#[inline(always)]
fn cis_impl(phase: &[f64], amp: &[f64], out: &mut [Complex64]) {
    let n = phase.len().min(amp.len()).min(out.len());
    let (phase, amp, out) = (&phase[..n], &amp[..n], &mut out[..n]);
    let mut pc = phase.chunks_exact(LANES);
    let mut ac = amp.chunks_exact(LANES);
    let mut oc = out.chunks_exact_mut(LANES);
    for ((p, a), o) in (&mut pc).zip(&mut ac).zip(&mut oc) {
        // 範囲外・NaN を含む組は libm で（`!(|x| <= MAX)` は NaN でも真）
        let ok = p.iter().fold(true, |ok, &x| ok & (x.abs() <= MAX_ARG));
        if ok {
            let mut sn = [0.0; LANES];
            let mut cs = [0.0; LANES];
            for l in 0..LANES {
                (sn[l], cs[l]) = sincos_poly(p[l]);
            }
            for l in 0..LANES {
                o[l] = Complex64::new(a[l] * cs[l], a[l] * sn[l]);
            }
        } else {
            for l in 0..LANES {
                o[l] = Complex64::from_polar(a[l], p[l]);
            }
        }
    }
    for ((&p, &a), o) in pc
        .remainder()
        .iter()
        .zip(ac.remainder())
        .zip(oc.into_remainder())
    {
        *o = if p.abs() <= MAX_ARG {
            let (s, c) = sincos_poly(p);
            Complex64::new(a * c, a * s)
        } else {
            Complex64::from_polar(a, p)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(xs: &[f64], tol: f64) {
        let amp = vec![1.0; xs.len()];
        let mut out = vec![Complex64::new(0.0, 0.0); xs.len()];
        cis_scaled(xs, &amp, &mut out);
        for (&x, o) in xs.iter().zip(&out) {
            let (s, c) = x.sin_cos();
            // φ 自体の丸め（|φ|·2^-53）は libm との差に含めて許す
            let t = tol * (1.0 + x.abs());
            assert!(
                (o.re - c).abs() <= t && (o.im - s).abs() <= t,
                "x={x}: {o} vs ({c}, {s})"
            );
        }
    }

    #[test]
    fn matches_libm_small_and_large() {
        // 象限の境目・0・負・端数
        let mut xs: Vec<f64> = (-40..=40)
            .map(|i| i as f64 * std::f64::consts::FRAC_PI_4)
            .collect();
        xs.extend((0..2000).map(|i| -3.0 + i as f64 * 0.003_1));
        check(&xs, 2.5e-16);
        // 干渉の計算で出る大きさ（k ≈ 7500 /mm × 光路差 数十 mm）
        let mut st = 0x2545_f491_4f6c_dd1du64;
        let big: Vec<f64> = (0..5000)
            .map(|_| {
                st ^= st << 13;
                st ^= st >> 7;
                st ^= st << 17;
                ((st >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0 * MAX_ARG
            })
            .collect();
        check(&big, 2.5e-16);
    }

    #[test]
    fn same_bits_with_and_without_avx2() {
        let xs: Vec<f64> = (0..1003)
            .map(|i| (i as f64 * 0.618_034).fract() * 2e3 - 1e3)
            .collect();
        let amp: Vec<f64> = (0..xs.len()).map(|i| 0.5 + i as f64 * 1e-3).collect();
        let mut a = vec![Complex64::new(0.0, 0.0); xs.len()];
        let mut b = a.clone();
        cis_scaled(&xs, &amp, &mut a);
        cis_impl(&xs, &amp, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn out_of_range_and_nan_fall_back() {
        let xs = [
            1.0,
            2.0e6,
            -3.0e9,
            0.5,
            1e300,
            2.0,
            3.0,
            4.0,
            5.0,
            f64::NAN,
            1e7,
        ];
        let amp = vec![2.0; xs.len()];
        let mut out = vec![Complex64::new(0.0, 0.0); xs.len()];
        cis_scaled(&xs, &amp, &mut out);
        for (&x, o) in xs.iter().zip(&out) {
            let w = Complex64::from_polar(2.0, x);
            if x.is_nan() {
                assert!(o.re.is_nan() && o.im.is_nan());
            } else if x.abs() > MAX_ARG {
                assert_eq!(*o, w);
            } else {
                assert!((o - w).norm() < 1e-15);
            }
        }
    }
}
