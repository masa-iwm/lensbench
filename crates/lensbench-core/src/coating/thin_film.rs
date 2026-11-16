//! 薄膜の特性行列とアドミタンス

use super::*;

/// 媒質 n（実数）の β でのアドミタンス（s: n cos、p: n / cos。cos は実数の枝）
pub fn admittance(n: f64, b: f64, pol: usize) -> C64 {
    let q = C64::new(n * n - b * b, 0.0).sqrt();
    if pol == 0 {
        q
    } else {
        C64::new(n * n, 0.0) / q
    }
}

/// 媒質 n（実数）の β でのアドミタンス。n < β（全反射の先）では減衰する枝（Im(n cos) ≤ 0、[`stack`] と同じ）
pub(in crate::coating) fn admittance_ev(n: f64, b: f64, pol: usize) -> C64 {
    let q2 = n * n - b * b;
    let q = if q2 >= 0.0 {
        C64::new(q2.sqrt(), 0.0)
    } else {
        C64::new(0.0, -(-q2).sqrt())
    };
    if pol == 0 {
        q
    } else {
        C64::new(n * n, 0.0) / q
    }
}

/// 複素屈折率 n の媒質での角の余弦（n0 sinθ0 = n sinθ、Im(n cos) ≤ 0 の枝）
pub(in crate::coating) fn ccos(n0: f64, s0: f64, n: C64) -> C64 {
    let q = C64::new(n0 * s0, 0.0) / n;
    let c = (C64::new(1.0, 0.0) - q * q).sqrt();
    if (n * c).im > 0.0 {
        -c
    } else {
        c
    }
}

/// 媒質 n0（実数）から層 [(N, d [µm])] を経て基板 ns へ、n0 での入射角 theta の薄膜
/// → (r_s, r_p, t_s, t_p)、パワー規格化、OpticStudio の約束
pub fn stack(n0: f64, layers: &[(C64, f64)], ns: C64, wl: f64, theta: f64) -> (C64, C64, C64, C64) {
    let (s0, c0) = theta.sin_cos();
    stack_sc(n0, layers, ns, wl, s0, c0)
}

/// [`stack`] を入射角の sin・cos で
pub(in crate::coating) fn stack_sc(
    n0: f64,
    layers: &[(C64, f64)],
    ns: C64,
    wl: f64,
    s0: f64,
    c0: f64,
) -> (C64, C64, C64, C64) {
    let cs = ccos(n0, s0, ns);
    let one = C64::new(1.0, 0.0);
    let i = C64::new(0.0, 1.0);
    let mut out = [(one, one); 2];
    for (pi, o) in out.iter_mut().enumerate() {
        let eta = |n: C64, c: C64| if pi == 0 { n * c } else { n / c };
        let e0 = eta(C64::new(n0, 0.0), C64::new(c0, 0.0));
        let es = eta(ns, cs);
        let mut m = [[one, C64::new(0.0, 0.0)], [C64::new(0.0, 0.0), one]];
        for &(n, d) in layers {
            let c = ccos(n0, s0, n);
            let e = eta(n, c);
            let dl = n * c * (2.0 * PI * d / wl);
            let (cd, sd) = (dl.cos(), dl.sin());
            let l = [[cd, i * sd / e], [i * e * sd, cd]];
            m = [
                [
                    m[0][0] * l[0][0] + m[0][1] * l[1][0],
                    m[0][0] * l[0][1] + m[0][1] * l[1][1],
                ],
                [
                    m[1][0] * l[0][0] + m[1][1] * l[1][0],
                    m[1][0] * l[0][1] + m[1][1] * l[1][1],
                ],
            ];
        }
        let b = m[0][0] + m[0][1] * es;
        let c = m[1][0] + m[1][1] * es;
        let r = (e0 * b - c) / (e0 * b + c);
        let t = e0 * 2.0 / (e0 * b + c) * C64::new(es.re / e0.re, 0.0).sqrt();
        *o = (r, t);
    }
    let [(rs, ts), (rp, tp)] = out;
    (rs, rp, ts, tp)
}

/// 膜ファイルの層の境界の媒質（前の媒質 n1、基板 ns、位相の約束の外側の媒質 n_out）
#[derive(Clone, Copy)]
pub(in crate::coating) struct StackMedia {
    pub(in crate::coating) n1: f64,
    pub(in crate::coating) ns: C64,
    pub(in crate::coating) n_out: f64,
}

/// 膜ファイルの層の (s, p): 透過、mirror なら反射（[`Iface::eval_cos_media`] で媒質だけ置き換えて使う）
#[allow(clippy::too_many_arguments)]
pub(in crate::coating) fn stack_coef(
    m: StackMedia,
    layers: &[(C64, f64)],
    layer_phase: bool,
    front: bool,
    mirror: bool,
    wl: f64,
    s0: f64,
    c0: f64,
) -> Result<[C64; 2]> {
    let (rs, rp, ts, tp) = stack_sc(m.n1, layers, m.ns, wl, s0, c0);
    let k = 2.0 * PI / wl;
    let d: f64 = layers.iter().map(|l| l.1).sum();
    if mirror {
        let ph = if front {
            C64::new(0.0, 2.0 * k * m.n1 * c0 * d).exp()
        } else {
            C64::new(1.0, 0.0)
        };
        Ok([rs * ph, rp * ph])
    } else {
        let b = m.n1 * s0;
        let co = (1.0 - (b / m.n_out) * (b / m.n_out)).max(0.0).sqrt();
        if co == 0.0 {
            // 膜の外側で全反射（光線の追跡ではその前に止まる）
            let n_out = m.n_out;
            notimpl!(
                "coating transmission: total reflection in the outer medium n = {n_out} (β = {b})"
            );
        }
        let mut ph = d * m.n_out / co;
        // q_j は層の屈折率の実部で（吸収のある層も OpticStudio はそうしている。金属薄膜の透過で照合）
        for &(n, dj) in layers.iter().filter(|_| layer_phase) {
            let q = C64::new(n.re * n.re - b * b, 0.0).sqrt();
            ph -= dj * (C64::new(b * b, 0.0) / q).re;
        }
        let ph = C64::new(0.0, k * ph).exp();
        Ok([ts * ph, tp * ph])
    }
}
