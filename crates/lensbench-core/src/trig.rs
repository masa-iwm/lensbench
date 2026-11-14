//! まとめて計算する sin・cos（干渉の計算の位相因子 exp(iφ) 用）。
//!
//! libm の sin・cos を 1 個ずつ呼ぶ代わりに、分岐のない多項式で 8 個ずつ計算する（コンパイラが SIMD にする）。
//! - 引数の簡約: q = round(φ / (π/2))、r = φ − q·(π/2)（π/2 を 3 つに分けた Cody–Waite、fdlibm の値）。
//!   |q| < 2^20 なら q·(π/2 の上位) が丸めなしで、r の誤差は r の 1 ulp 程度。
//! - |r| ≤ π/4 の sin・cos は fdlibm の __kernel_sin・__kernel_cos と同じ係数（誤差 1〜2 ulp）。
//! - [`MAX_ARG`] < |φ| ≤ [`MAX_ARG_MED`] は π/2 を 24 ビットずつ 4 つに分けた簡約（|q| < 2^28 なら各項の積が丸めなし）。
//!   r の絶対誤差は |φ|·1e-23 程度で、φ 自体の丸め（|φ|·1e-16）より十分小さい（光路差 300 mm 程度で
//!   |φ| ≈ 2.5e6 になり、ここが libm に戻って遅かった）。|φ| ≤ MAX_ARG の値は今までと同じビット。
//! - |φ| が [`MAX_ARG_MED`] を超える・NaN・無限大を含む 8 個は libm に戻す。
//!
//! 結果は libm と最下位の数ビット違うことがある（φ の丸め誤差 |φ|·1e-16 と同程度）。
//! 浮動小数の演算の順は固定なので、スレッド数によらず同じビットになる。

// 係数は fdlibm の値をそのまま書く
#![allow(clippy::excessive_precision)]

use num_complex::Complex64;

/// 多項式で計算する |φ| の上限（q·(π/2 の上位 33 ビット) が丸めなしになる範囲）。
pub const MAX_ARG: f64 = 1.0e6;

/// 4 つに分けた簡約で多項式を使う |φ| の上限（q < 2^28 で q·(24 ビットの項) が丸めなし）。
pub const MAX_ARG_MED: f64 = 2.0e8;

const INV_PIO2: f64 = std::f64::consts::FRAC_2_PI;
// π/2 を最も近い 24 ビットずつに分けた値（4 つ目は残りの倍精度）。和と π/2 の差は 5e-40
const MPIO2_1: f64 = 1.5707963705062866; // 0x1.921fb6p+0
const MPIO2_2: f64 = -4.371138828673793e-08; // -0x1.777a5cp-25
const MPIO2_3: f64 = -1.7151245100058819e-15; // -0x1.ee59dap-50
const MPIO2_4: f64 = 1.0562999066987428e-23; // 0x1.98a2e03707345p-77
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

/// 1 個分（|φ| ≤ MAX_ARG_MED を前提）。返り値は (sin φ, cos φ)。
/// 象限の入れ替え・符号・簡約の選択はビット演算・選択で行い、分岐なしにする（8 個まとめて SIMD になるように）。
#[inline(always)]
fn sincos_poly(x: f64) -> (f64, f64) {
    let t = x * INV_PIO2 + ROUND;
    // t の仮数の下位ビットが q（2 の補数）
    let q = t.to_bits();
    let qf = t - ROUND;
    let r_small = ((x - qf * PIO2_1) - qf * PIO2_2) - qf * PIO2_2T;
    let r_med = (((x - qf * MPIO2_1) - qf * MPIO2_2) - qf * MPIO2_3) - qf * MPIO2_4;
    let r = if x.abs() <= MAX_ARG { r_small } else { r_med };
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

/// 一度にまとめる 8 個の組の数（AVX2 版）。多項式は 1 個ごとに長い依存の鎖なので、段ごとに多くの組を並べて
/// 待ち時間を隠す
#[cfg(target_arch = "x86_64")]
const AVX_GROUPS: usize = 4;

/// AVX2 版。[`sincos_poly`] と同じ演算を同じ順に、4 個ずつの 256 ビット命令で書いたもの（FMA は使わないので各値は
/// 同じビット）。8 個の組の分け方・libm に戻す条件・端数の扱いは [`cis_impl`] と同じ。組を [`AVX_GROUPS`] 個ずつ
/// まとめ、簡約・sin の多項式・cos の多項式・象限の段ごとに全部の組を計算する（演算の中身は変えずに、依存のない
/// 計算を並べて待ち時間を隠す）。まとめた組がすべて |φ| ≤ MAX_ARG なら 4 つに分けた簡約は計算しない
/// （選ばれない値なので結果は同じ）。
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn cis_avx2(phase: &[f64], amp: &[f64], out: &mut [Complex64]) {
    use std::arch::x86_64::*;
    const NV: usize = AVX_GROUPS * 2;
    let n = phase.len().min(amp.len()).min(out.len());
    let (phase, amp, out) = (&phase[..n], &amp[..n], &mut out[..n]);
    let full = n / LANES * LANES;
    let abs_mask = _mm256_castsi256_pd(_mm256_set1_epi64x(i64::MAX));
    let max_arg = _mm256_set1_pd(MAX_ARG);
    let max_med = _mm256_set1_pd(MAX_ARG_MED);
    let one_i = _mm256_set1_epi64x(1);
    let two_i = _mm256_set1_epi64x(2);
    let zero_i = _mm256_setzero_si256();
    let c = |v: f64| _mm256_set1_pd(v);
    let pp = phase.as_ptr();
    let ap = amp.as_ptr();
    let op = out.as_mut_ptr() as *mut f64;
    let zero = _mm256_setzero_pd();
    let mut i = 0;
    while i < full {
        // まとめる組: 範囲内の組の 4 個ずつの先頭 pos[..nv]（範囲外・NaN を含む組はここで libm で）
        let mut pos = [0usize; NV];
        let mut x = [zero; NV];
        let mut nv = 0;
        let mut med = false;
        let mut g = 0;
        while g < AVX_GROUPS && i < full {
            // 安全: i + 8 ≤ full ≤ n
            let x0 = _mm256_loadu_pd(pp.add(i));
            let x1 = _mm256_loadu_pd(pp.add(i + 4));
            let (a0, a1) = (_mm256_and_pd(x0, abs_mask), _mm256_and_pd(x1, abs_mask));
            // 比較は NaN で偽
            let ok = _mm256_movemask_pd(_mm256_and_pd(
                _mm256_cmp_pd::<_CMP_LE_OQ>(a0, max_med),
                _mm256_cmp_pd::<_CMP_LE_OQ>(a1, max_med),
            )) == 0b1111;
            if ok {
                med |= _mm256_movemask_pd(_mm256_or_pd(
                    _mm256_cmp_pd::<_CMP_GT_OQ>(a0, max_arg),
                    _mm256_cmp_pd::<_CMP_GT_OQ>(a1, max_arg),
                )) != 0;
                pos[nv] = i;
                x[nv] = x0;
                pos[nv + 1] = i + 4;
                x[nv + 1] = x1;
                nv += 2;
            } else {
                for l in i..i + LANES {
                    out[l] = Complex64::from_polar(amp[l], phase[l]);
                }
            }
            i += LANES;
            g += 1;
        }
        // 簡約（[`sincos_poly`] の 1 行ずつに対応する）
        let mut q = [zero_i; NV];
        let mut r = [zero; NV];
        for v in 0..nv {
            let xv = x[v];
            let t = _mm256_add_pd(_mm256_mul_pd(xv, c(INV_PIO2)), c(ROUND));
            q[v] = _mm256_castpd_si256(t);
            let qf = _mm256_sub_pd(t, c(ROUND));
            let r_small = _mm256_sub_pd(
                _mm256_sub_pd(
                    _mm256_sub_pd(xv, _mm256_mul_pd(qf, c(PIO2_1))),
                    _mm256_mul_pd(qf, c(PIO2_2)),
                ),
                _mm256_mul_pd(qf, c(PIO2_2T)),
            );
            r[v] = if med {
                let r_med = _mm256_sub_pd(
                    _mm256_sub_pd(
                        _mm256_sub_pd(
                            _mm256_sub_pd(xv, _mm256_mul_pd(qf, c(MPIO2_1))),
                            _mm256_mul_pd(qf, c(MPIO2_2)),
                        ),
                        _mm256_mul_pd(qf, c(MPIO2_3)),
                    ),
                    _mm256_mul_pd(qf, c(MPIO2_4)),
                );
                let small = _mm256_cmp_pd::<_CMP_LE_OQ>(_mm256_and_pd(xv, abs_mask), max_arg);
                _mm256_blendv_pd(r_med, r_small, small)
            } else {
                r_small
            };
        }
        // sin r = r + r·z·(S1 + z·(S2 + z·(S3 + z·(S4 + z·(S5 + z·S6)))))
        let mut z = [zero; NV];
        let mut sn = [zero; NV];
        for v in 0..nv {
            let (rv, zv) = (r[v], _mm256_mul_pd(r[v], r[v]));
            z[v] = zv;
            let mut ps = _mm256_mul_pd(zv, c(S6));
            ps = _mm256_mul_pd(zv, _mm256_add_pd(c(S5), ps));
            ps = _mm256_mul_pd(zv, _mm256_add_pd(c(S4), ps));
            ps = _mm256_mul_pd(zv, _mm256_add_pd(c(S3), ps));
            ps = _mm256_mul_pd(zv, _mm256_add_pd(c(S2), ps));
            ps = _mm256_add_pd(c(S1), ps);
            sn[v] = _mm256_add_pd(rv, _mm256_mul_pd(_mm256_mul_pd(rv, zv), ps));
        }
        // cos r = w + (((1 − w) − z/2) + z²·(C1 + z·(…)))、w = 1 − z/2
        let mut cs = [zero; NV];
        for v in 0..nv {
            let zv = z[v];
            let hz = _mm256_mul_pd(c(0.5), zv);
            let w = _mm256_sub_pd(c(1.0), hz);
            let mut pc = _mm256_mul_pd(zv, c(C6));
            pc = _mm256_mul_pd(zv, _mm256_add_pd(c(C5), pc));
            pc = _mm256_mul_pd(zv, _mm256_add_pd(c(C4), pc));
            pc = _mm256_mul_pd(zv, _mm256_add_pd(c(C3), pc));
            pc = _mm256_mul_pd(zv, _mm256_add_pd(c(C2), pc));
            pc = _mm256_add_pd(c(C1), pc);
            cs[v] = _mm256_add_pd(
                w,
                _mm256_add_pd(
                    _mm256_sub_pd(_mm256_sub_pd(c(1.0), w), hz),
                    _mm256_mul_pd(_mm256_mul_pd(zv, zv), pc),
                ),
            );
        }
        // 象限（ビット演算は sincos_poly と同じ）、振幅を掛けて (re, im) の並びで書く
        for v in 0..nv {
            let (s, cc, qv) = (sn[v], cs[v], q[v]);
            let swap = _mm256_castsi256_pd(_mm256_sub_epi64(zero_i, _mm256_and_si256(qv, one_i)));
            let a = _mm256_or_pd(_mm256_and_pd(cc, swap), _mm256_andnot_pd(swap, s));
            let b = _mm256_or_pd(_mm256_and_pd(s, swap), _mm256_andnot_pd(swap, cc));
            let sin_neg = _mm256_slli_epi64::<62>(_mm256_and_si256(qv, two_i));
            let cos_neg =
                _mm256_slli_epi64::<62>(_mm256_and_si256(_mm256_add_epi64(qv, one_i), two_i));
            let sv = _mm256_xor_pd(a, _mm256_castsi256_pd(sin_neg));
            let cv = _mm256_xor_pd(b, _mm256_castsi256_pd(cos_neg));
            let k = pos[v];
            // 安全: k + 4 ≤ full ≤ n。Complex64 は (re, im) の並び（repr(C)）
            let am = _mm256_loadu_pd(ap.add(k));
            let re = _mm256_mul_pd(am, cv);
            let im = _mm256_mul_pd(am, sv);
            let lo = _mm256_unpacklo_pd(re, im); // re0 im0 re2 im2
            let hi = _mm256_unpackhi_pd(re, im); // re1 im1 re3 im3
            _mm256_storeu_pd(op.add(2 * k), _mm256_permute2f128_pd::<0x20>(lo, hi));
            _mm256_storeu_pd(op.add(2 * k + 4), _mm256_permute2f128_pd::<0x31>(lo, hi));
        }
    }
    cis_impl(&phase[full..], &amp[full..], &mut out[full..])
}

#[inline(always)]
fn cis_impl(phase: &[f64], amp: &[f64], out: &mut [Complex64]) {
    let n = phase.len().min(amp.len()).min(out.len());
    let (phase, amp, out) = (&phase[..n], &amp[..n], &mut out[..n]);
    let (pc, pr) = phase.as_chunks::<LANES>();
    let (ac, ar) = amp.as_chunks::<LANES>();
    let (oc, or) = out.as_chunks_mut::<LANES>();
    for ((p, a), o) in pc.iter().zip(ac).zip(oc.iter_mut()) {
        // 範囲外・NaN を含む組は libm で（`!(|x| <= MAX)` は NaN でも真）
        let ok = p.iter().fold(true, |ok, &x| ok & (x.abs() <= MAX_ARG_MED));
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
    for ((&p, &a), o) in pr.iter().zip(ar).zip(or.iter_mut()) {
        *o = if p.abs() <= MAX_ARG_MED {
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
    fn medium_range_matches_libm() {
        // 光路差 数百 mm の位相から MAX_ARG_MED まで。8 個の組に MAX_ARG 以下と以上が混ざっても
        // それぞれの簡約で計算する
        let mut st = 0x9e37_79b9_7f4a_7c15u64;
        let mut xs: Vec<f64> = (0..20000)
            .map(|i| {
                st ^= st << 13;
                st ^= st >> 7;
                st ^= st << 17;
                let u = (st >> 11) as f64 / (1u64 << 53) as f64;
                // 対数で一様に 1e5〜MAX_ARG_MED、符号は交互
                let x = 1e5 * (MAX_ARG_MED / 1e5).powf(u);
                if i % 2 == 0 {
                    x
                } else {
                    -x
                }
            })
            .collect();
        // 象限の境目の近く
        xs.extend((0..200).map(|i| (1_000_000 + 37 * i) as f64 * std::f64::consts::FRAC_PI_4));
        let amp = vec![1.0; xs.len()];
        let mut out = vec![Complex64::new(0.0, 0.0); xs.len()];
        cis_scaled(&xs, &amp, &mut out);
        let mut worst = 0.0f64;
        for (&x, o) in xs.iter().zip(&out) {
            let (s, c) = x.sin_cos();
            worst = worst.max((o.re - c).abs()).max((o.im - s).abs());
        }
        // 簡約の誤差（|φ|·1e-23 程度）と多項式の 1〜2 ulp
        assert!(worst < 1.5e-15, "最大の差 {worst}");
        // MAX_ARG 以下は今までの簡約と同じビット（1 個ずつ計算した値と一致）
        let small: Vec<f64> = xs.iter().map(|x| x * 1e-3).collect();
        let mut a = vec![Complex64::new(0.0, 0.0); small.len()];
        cis_scaled(&small, &amp, &mut a);
        for (&x, o) in small.iter().zip(&a) {
            let mut one = [Complex64::new(0.0, 0.0)];
            cis_scaled(&[x], &[1.0], &mut one);
            assert_eq!(*o, one[0]);
        }
    }

    #[test]
    fn same_bits_with_and_without_avx2() {
        let mut xs: Vec<f64> = (0..1003)
            .map(|i| (i as f64 * 0.618_034).fract() * 2e3 - 1e3)
            .collect();
        xs.extend((0..1003).map(|i| (i as f64 * 0.618_034).fract() * 4e8 - 2e8));
        let amp: Vec<f64> = (0..xs.len()).map(|i| 0.5 + i as f64 * 1e-3).collect();
        let mut a = vec![Complex64::new(0.0, 0.0); xs.len()];
        let mut b = a.clone();
        cis_scaled(&xs, &amp, &mut a);
        cis_impl(&xs, &amp, &mut b);
        assert_eq!(a, b);
    }

    /// AVX2 版（x86_64 だけにある）と cis_impl のビット一致を、いろいろな組・端数で
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_groups_match_scalar_bits() {
        // 組の全部が MAX_ARG 以下・MAX_ARG を超える値が混ざる・libm に戻る、の組をいろいろな端数の長さで
        let mut st = 0x1234_5678_9abc_def1u64;
        let mut ys: Vec<f64> = (0..4099)
            .map(|i| {
                st ^= st << 13;
                st ^= st >> 7;
                st ^= st << 17;
                let u = (st >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
                match (i / 8) % 5 {
                    0 => u * 20.0,
                    1 => u * 2.0 * MAX_ARG,
                    2 => u * 2.0 * MAX_ARG_MED,
                    3 => u * 1e10,
                    _ => (i as f64) * std::f64::consts::FRAC_PI_4,
                }
            })
            .collect();
        ys[77] = f64::NAN;
        ys[1000] = f64::INFINITY;
        ys[1001] = -0.0;
        let amp: Vec<f64> = (0..ys.len()).map(|i| 0.25 + i as f64 * 1e-3).collect();
        for len in [0, 1, 7, 8, 9, 63, 64, 65, ys.len()] {
            let mut a = vec![Complex64::new(0.0, 0.0); len];
            let mut b = a.clone();
            cis_scaled(&ys[..len], &amp[..len], &mut a);
            cis_impl(&ys[..len], &amp[..len], &mut b);
            for (x, y) in a.iter().zip(&b) {
                assert!(
                    x.re.to_bits() == y.re.to_bits() && x.im.to_bits() == y.im.to_bits()
                        || (x.re.is_nan() && y.re.is_nan()),
                    "{x} vs {y}"
                );
            }
        }
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
            } else if x.abs() > MAX_ARG_MED {
                assert_eq!(*o, w);
            } else {
                assert!((o - w).norm() < 2e-15, "x={x}: {o} vs {w}");
            }
        }
    }
}
