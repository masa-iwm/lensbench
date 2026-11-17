//! 薄膜の物理の不変量。OpticStudio の基準値に頼らない。吸収のない膜の R + T = 1・1/4 波長・
//! 向きの相反性は coating/tests.rs の単体試験と polarization.rs、k の符号の誤りを断るのは polarization.rs

use lensbench_core::coating::stack;
use num_complex::Complex64 as C64;

/// 吸収のある層・基板でも R + T ≤ 1（吸収 A = 1 − R − T ≥ 0）、k → 0 で R + T → 1。
/// 層の数・屈折率・厚さ・入射角は決まった種の擬似乱数で選ぶ
#[test]
fn absorbing_stack_does_not_create_energy() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for case in 0..400 {
        let nl = 1 + (rnd() * 6.0) as usize;
        let mut layers: Vec<(C64, f64)> = (0..nl)
            .map(|_| {
                (
                    C64::new(1.3 + 1.2 * rnd(), -0.5 * rnd()),
                    0.02 + 0.3 * rnd(),
                )
            })
            .collect();
        // 基板は透明か吸収（金属に近い k も）
        let ns = C64::new(
            1.4 + 0.6 * rnd(),
            if case % 3 == 0 { -3.0 * rnd() } else { 0.0 },
        );
        let th = 80f64.to_radians() * rnd();
        let wl = 0.4 + 0.6 * rnd();
        let (rs, rp, ts, tp) = stack(1.0, &layers, ns, wl, th);
        for (pol, r, t) in [("s", rs, ts), ("p", rp, tp)] {
            let sum = r.norm_sqr() + t.norm_sqr();
            assert!(
                sum.is_finite() && sum <= 1.0 + 1e-12,
                "{case} {pol}: R + T = {sum}"
            );
            assert!(r.norm_sqr() >= 0.0 && t.norm_sqr() >= 0.0);
        }
        // 吸収を消すと（透明な基板で）R + T = 1 に戻る
        if ns.im == 0.0 {
            for l in &mut layers {
                l.0.im = 0.0;
            }
            let (rs, rp, ts, tp) = stack(1.0, &layers, ns, wl, th);
            for (pol, r, t) in [("s", rs, ts), ("p", rp, tp)] {
                let sum = r.norm_sqr() + t.norm_sqr();
                assert!(
                    (sum - 1.0).abs() < 1e-12,
                    "{case} {pol} lossless: R + T = {sum}"
                );
            }
        }
    }
    // 吸収の強い厚い層は光を通さない（T → 0、R + T < 1）
    let (rs, _, ts, _) = stack(
        1.0,
        &[(C64::new(1.5, -0.5), 5.0)],
        C64::new(1.5, 0.0),
        0.55,
        0.0,
    );
    assert!(ts.norm_sqr() < 1e-20 && rs.norm_sqr() < 0.1);
}
