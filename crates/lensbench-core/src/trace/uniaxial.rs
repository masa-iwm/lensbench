//! 一軸結晶の異常光（複屈折材の板の Mode 1・3）: 板へ入る屈折と、板の中の光線の向き（walk-off）。
//!
//! 板の中の平面波は波数ベクトル m = n(θ) k̂（k̂ は波面法線、θ は k̂ と結晶軸 a の角）で表す。異常光の m は
//! |m|²/ne² + (1/no² − 1/ne²)(m·a)² = 1 を満たす（1/n(θ)² = cos²θ/no² + sin²θ/ne² と同じ）。境界では m の
//! 接線の成分が続く。エネルギー（光線）はこの式の勾配 m/ne² + (1/no² − 1/ne²)(m·a) a の向きに進み、
//! 光線の長さ L あたりの位相（光路長）は m·ŝ L（光線の屈折率 n(θ) cos ρ、ρ は walk-off の角）

use crate::math::*;

/// 異常光の式の勾配の向き（光線の向き、単位ベクトル）
#[inline]
pub(crate) fn ray_dir(m: V3, a: V3, no: f64, ne: f64) -> V3 {
    let d = 1.0 / (no * no) - 1.0 / (ne * ne);
    normalize(add(scale(m, 1.0 / (ne * ne)), scale(a, d * dot(m, a))))
}

/// 等方な媒質（屈折率 n1）から一軸結晶の異常光への屈折。u は入射の向き（単位）、nv は面の法線（単位、向きは
/// どちらでもよい）、a は結晶軸（単位）。返り値は板の中の (波数ベクトル m, 光線の向き ŝ)。光線が面の向こうへ
/// 進む解（ŝ·nv が u·nv と同じ符号）を選ぶ。解がなければ（全反射）None
pub(crate) fn refract_in(u: V3, n1: f64, nv: V3, a: V3, no: f64, ne: f64) -> Option<(V3, V3)> {
    let cu = dot(u, nv);
    // 波数ベクトルの接線の成分（境界で続く）
    let t = scale(sub(u, scale(nv, cu)), n1);
    let d = 1.0 / (no * no) - 1.0 / (ne * ne);
    let (ta, na) = (dot(t, a), dot(nv, a));
    // m = t + γ nv を式に入れた γ の二次式 A γ² + B γ + C = 0
    let qa = 1.0 / (ne * ne) + d * na * na;
    let qb = 2.0 * d * ta * na;
    let qc = dot(t, t) / (ne * ne) + d * ta * ta - 1.0;
    let disc = qb * qb - 4.0 * qa * qc;
    if !(disc >= 0.0) || !(qa > 0.0) {
        return None;
    }
    let r = disc.sqrt();
    // 桁落ちしない根の公式
    let q = -0.5 * (qb + qb.signum() * r);
    let roots = [q / qa, if q != 0.0 { qc / q } else { -q / qa }];
    roots.into_iter().find_map(|g| {
        let m = add(t, scale(nv, g));
        let s = ray_dir(m, a, no, ne);
        (dot(s, nv) * cu > 0.0 && s.iter().all(|v| v.is_finite())).then_some((m, s))
    })
}

/// 板の中の波数ベクトル m から等方な媒質（屈折率 n2）への屈折。nv は面の法線（単位）。返り値は出た光線の
/// 向き（単位）。m の向きに面を越える解。解がなければ（全反射）None
pub(crate) fn refract_out(m: V3, s: V3, nv: V3, n2: f64) -> Option<V3> {
    let mn = dot(m, nv);
    let t = sub(m, scale(nv, mn));
    let g2 = n2 * n2 - dot(t, t);
    if !(g2 >= 0.0) {
        return None;
    }
    // 光線（エネルギー）の進む側へ出る
    let side = dot(s, nv).signum();
    Some(scale(add(t, scale(nv, side * g2.sqrt())), 1.0 / n2))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO: f64 = 1.544;
    const NE: f64 = 1.553;

    fn n_theta(k: V3, a: V3) -> f64 {
        let c = dot(normalize(k), normalize(a));
        1.0 / (c * c / (NO * NO) + (1.0 - c * c) / (NE * NE)).sqrt()
    }

    /// 垂直に入る光線: 波面法線は法線のまま（接線の成分 0）、屈折率は n(θ)、光線は walk-off の角だけ傾く
    /// （tan ρ = (ne² − no²) sin θ cos θ / (no² sin²θ + ne² cos²θ)、正の一軸結晶では軸に寄る）
    #[test]
    fn normal_incidence_walk_off() {
        let th = 30f64.to_radians();
        let a = [th.sin(), 0.0, th.cos()];
        let (m, s) = refract_in([0.0, 0.0, 1.0], 1.0, [0.0, 0.0, 1.0], a, NO, NE).unwrap();
        assert!(m[0].abs() < 1e-15 && m[1].abs() < 1e-15);
        assert!((m[2] - n_theta([0.0, 0.0, 1.0], a)).abs() < 1e-13);
        let (sn, cs) = th.sin_cos();
        let tan = (NE * NE - NO * NO) * sn * cs / (NO * NO * sn * sn + NE * NE * cs * cs);
        // 正の一軸結晶（ne > no）では光線は軸の側（x の正）へ寄る
        assert!((s[0] / s[2] - tan).abs() < 1e-13, "{s:?} {tan}");
        assert!(s[1].abs() < 1e-15);
    }

    /// 軸が面に垂直・平行なときは walk-off なし。軸に沿う光線の屈折率は no
    #[test]
    fn axis_along_or_across_has_no_walk_off() {
        let u = normalize([0.2, 0.1, 1.0]);
        for (a, n_axis) in [([0.0, 0.0, 1.0], NO), ([1.0, 0.0, 0.0], NE)] {
            let (m, s) = refract_in([0.0, 0.0, 1.0], 1.0, [0.0, 0.0, 1.0], a, NO, NE).unwrap();
            assert!((m[2] - n_axis).abs() < 1e-14);
            assert!((s[2] - 1.0).abs() < 1e-15);
            // 斜めの光線でも接線の成分は続き、m は式を満たす
            let (m, _) = refract_in(u, 1.2, [0.0, 0.0, 1.0], a, NO, NE).unwrap();
            assert!((m[0] - 1.2 * u[0]).abs() < 1e-15 && (m[1] - 1.2 * u[1]).abs() < 1e-15);
            assert!((norm(m) - n_theta(m, a)).abs() < 1e-13);
        }
    }

    /// no = ne なら等方な屈折（Snell）と同じ
    #[test]
    fn isotropic_limit_is_snell() {
        let u = normalize([0.3, -0.2, 1.0]);
        let nv = normalize([0.1, 0.05, 1.0]);
        let a = normalize([0.3, 0.4, 0.5]);
        let (m, s) = refract_in(u, 1.0, nv, a, 1.5, 1.5).unwrap();
        let k = refract_out(scale(u, 1.0), u, nv, 1.5).unwrap();
        for c in 0..3 {
            assert!((s[c] - k[c]).abs() < 1e-14, "{s:?} {k:?}");
            assert!((m[c] - 1.5 * k[c]).abs() < 1e-14);
        }
    }

    /// 平行な板を通ると向きは元に戻る（接線の成分が続く）
    #[test]
    fn parallel_plate_restores_direction() {
        let u = normalize([0.25, -0.1, 1.0]);
        let a = normalize([0.6, 0.2, 0.7]);
        let nv = [0.0, 0.0, 1.0];
        let (m, s) = refract_in(u, 1.0, nv, a, NO, NE).unwrap();
        let k = refract_out(m, s, nv, 1.0).unwrap();
        for c in 0..3 {
            assert!((k[c] - u[c]).abs() < 1e-15, "{k:?} {u:?}");
        }
    }

    /// 向こう側から入る光線（u·nv < 0）も、面の向こうへ進む解を選ぶ
    #[test]
    fn backward_side() {
        let a = normalize([0.5, 0.0, 1.0]);
        let (_, s) = refract_in([0.0, 0.0, -1.0], 1.0, [0.0, 0.0, 1.0], a, NO, NE).unwrap();
        assert!(s[2] < 0.0);
    }
}
