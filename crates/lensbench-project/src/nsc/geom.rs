//! 場面の面と光線の交点・法線・境界の箱

use crate::scene::{Face, Shape};
use lensbench_core::math::{add, mtv, mv, V3};
use lensbench_core::surface::Aperture;

/// 自分の面に戻る交点として除く距離 [mm]（反射・屈折した直後の同じ面）
pub(super) const SELF_EPS: f64 = 1e-7;
/// ほかの面の交点として数える最小の距離 [mm]。わずかに負: 今の点を通るほかの面（同じ所に置いた絞りと像面など）に
/// 当たるように
pub(super) const MIN_T: f64 = -1e-9;

/// 交点: 距離・局所座標の点・局所の法線（+z 側・円柱は外向き、規格化済み）
#[derive(Debug, Clone, Copy)]
pub(super) struct LocalHit {
    pub t: f64,
    pub p: V3,
    pub n: V3,
}

/// 面の交点の求め方の追加の条件
#[derive(Debug, Clone, Copy)]
pub(super) enum Bound {
    /// 形の及ぶ半径まで（開口があれば開口の中だけ）
    Extent,
    /// 半径 r の円板の中（開口の外も含む。空気中の絞り: 開口の外を遮る有限の板）
    Disc { r: f64 },
    /// 局所の矩形 |x| ≤ hx、|y| ≤ hy（検出器）
    Rect { hx: f64, hy: f64 },
}

/// 大域の光線 (p, k) と面 f の最も近い交点（距離 > tmin）
pub(super) fn intersect(f: &Face, bound: Bound, p: V3, k: V3, tmin: f64) -> Option<LocalHit> {
    let pl = f.to_local(p);
    let kl = mtv(&f.frame.r, k);
    match &f.shape {
        Shape::Optical { surface, extent } => optical(surface, *extent, bound, pl, kl, tmin),
        Shape::Annulus { r0, r1, z } => {
            if kl[2] == 0.0 {
                return None;
            }
            let t = (z - pl[2]) / kl[2];
            if t.is_nan() || t <= tmin {
                return None;
            }
            let q = add(pl, [kl[0] * t, kl[1] * t, kl[2] * t]);
            let r = q[0].hypot(q[1]);
            (r >= *r0 && r <= *r1).then_some(LocalHit {
                t,
                p: [q[0], q[1], *z],
                n: [0.0, 0.0, 1.0],
            })
        }
        Shape::Cylinder { r, z0, z1 } => {
            let a = kl[0] * kl[0] + kl[1] * kl[1];
            if a == 0.0 {
                return None;
            }
            let b = pl[0] * kl[0] + pl[1] * kl[1];
            let c = pl[0] * pl[0] + pl[1] * pl[1] - r * r;
            let disc = b * b - a * c;
            if disc < 0.0 {
                return None;
            }
            let sq = disc.sqrt();
            // 打ち消しを避ける根の式
            let q = -(b + sq.copysign(b));
            let mut ts = [q / a, if q != 0.0 { c / q } else { -b / a }];
            ts.sort_by(f64::total_cmp);
            ts.into_iter().filter(|&t| t > tmin).find_map(|t| {
                let q = add(pl, [kl[0] * t, kl[1] * t, kl[2] * t]);
                (q[2] >= *z0 && q[2] <= *z1).then(|| {
                    let rr = q[0].hypot(q[1]);
                    LocalHit {
                        t,
                        p: q,
                        n: [q[0] / rr, q[1] / rr, 0.0],
                    }
                })
            })
        }
    }
}

/// 順次の系の面の形との交点: 基準の二次曲面の 2 つの根と頂点の平面を初期値にニュートン法で詰め、面の上で
/// 範囲に入る最も近いもの
fn optical(
    d: &lensbench_core::surface::Surface,
    extent: f64,
    bound: Bound,
    pl: V3,
    kl: V3,
    tmin: f64,
) -> Option<LocalHit> {
    let sh = d.shape();
    let (c, k) = (d.c, d.k);
    let mut cand: [f64; 3] = [f64::NAN; 3];
    // 二次曲面 c (x² + y² + (1 + k) z²) − 2 z = 0
    if c != 0.0 {
        let e = 1.0 + k;
        let a = c * (kl[0] * kl[0] + kl[1] * kl[1] + e * kl[2] * kl[2]);
        let b = c * (pl[0] * kl[0] + pl[1] * kl[1] + e * pl[2] * kl[2]) - kl[2];
        let cc = c * (pl[0] * pl[0] + pl[1] * pl[1] + e * pl[2] * pl[2]) - 2.0 * pl[2];
        // a t² + 2 b t + cc = 0
        if a.abs() > 1e-300 {
            let disc = b * b - a * cc;
            if disc >= 0.0 {
                let sq = disc.sqrt();
                let q = -(b + sq.copysign(b));
                cand[0] = q / a;
                cand[1] = if q != 0.0 { cc / q } else { -b / a };
            }
        } else if b != 0.0 {
            cand[0] = -cc / (2.0 * b);
        }
    }
    if kl[2] != 0.0 {
        cand[2] = -pl[2] / kl[2];
    }
    let conic = matches!(
        sh,
        lensbench_core::shape::Shape::Conic { .. } | lensbench_core::shape::Shape::Plane
    ) || (matches!(sh, lensbench_core::shape::Shape::EvenAsphere(_)) && !d.has_asph());
    let mut best: Option<LocalHit> = None;
    for &t0 in cand.iter().filter(|t| t.is_finite()) {
        let Some(t) = refine(&sh, pl, kl, t0, conic && c != 0.0) else {
            continue;
        };
        if t.is_nan() || t <= tmin || best.is_some_and(|b| b.t <= t) {
            continue;
        }
        let q = add(pl, [kl[0] * t, kl[1] * t, kl[2] * t]);
        let (z, zx, zy) = sh.sag(q[0], q[1]);
        let tol = 1e-9 * (1.0 + q[2].abs() + t.abs());
        if !(z.is_finite() && (z - q[2]).abs() <= tol) {
            continue;
        }
        let r = q[0].hypot(q[1]);
        let inside = match bound {
            Bound::Extent => {
                r <= extent * (1.0 + 1e-12) + 1e-12
                    && (matches!(d.ap, Aperture::None | Aperture::Floating { .. })
                        || !lensbench_core::trace::aperture_out(d.ap, q))
            }
            Bound::Disc { r: rd } => r <= rd * (1.0 + 1e-12) + 1e-12,
            // 縁ちょうど（半幅が像面の半直径＝いちばん外の光線の高さ）は丸めの分だけ広げて中に数える
            Bound::Rect { hx, hy } => {
                q[0].abs() <= hx * (1.0 + 1e-9) + 1e-12 && q[1].abs() <= hy * (1.0 + 1e-9) + 1e-12
            }
        };
        if !inside {
            continue;
        }
        let nn = (zx * zx + zy * zy + 1.0).sqrt();
        best = Some(LocalHit {
            t,
            p: [q[0], q[1], z],
            n: [-zx / nn, -zy / nn, 1.0 / nn],
        });
    }
    best
}

/// 初期値 t0 から z(t) = sag(x(t), y(t)) をニュートン法で解く（二次曲面だけの面は根がそのまま交点）
fn refine(
    sh: &lensbench_core::shape::Shape<'_>,
    pl: V3,
    kl: V3,
    t0: f64,
    exact: bool,
) -> Option<f64> {
    if exact {
        return Some(t0);
    }
    let mut t = t0;
    for _ in 0..50 {
        let q = add(pl, [kl[0] * t, kl[1] * t, kl[2] * t]);
        let (z, zx, zy) = sh.sag(q[0], q[1]);
        if !z.is_finite() {
            return None;
        }
        let g = q[2] - z;
        let dg = kl[2] - zx * kl[0] - zy * kl[1];
        if dg == 0.0 || !dg.is_finite() {
            return None;
        }
        let dt = g / dg;
        t -= dt;
        if dt.abs() <= 1e-13 * (1.0 + t.abs()) {
            return Some(t);
        }
    }
    None
}

/// 面の境界の箱（大域、[min, max]）
pub(super) fn bbox(f: &Face, bound: Bound) -> Option<[V3; 2]> {
    let local: [V3; 2] = match (&f.shape, bound) {
        (Shape::Optical { surface, extent }, b) => {
            let (hx, hy, round) = match b {
                Bound::Rect { hx, hy } => (hx, hy, None),
                Bound::Disc { r } => (r, r, Some(r)),
                Bound::Extent => (*extent, *extent, Some(*extent)),
            };
            let sh = surface.shape();
            let (mut z0, mut z1) = (0.0f64, 0.0f64);
            let n = 24;
            for i in 0..=n {
                for j in 0..=n {
                    let x = hx * (2.0 * i as f64 / n as f64 - 1.0);
                    let y = hy * (2.0 * j as f64 / n as f64 - 1.0);
                    if round.is_some_and(|r| x.hypot(y) > r) {
                        continue;
                    }
                    let z = sh.sag(x, y).0;
                    if z.is_finite() {
                        z0 = z0.min(z);
                        z1 = z1.max(z);
                    }
                }
            }
            // 格子の間の膨らみと丸めの分を広げる
            let pad = 0.05 * (z1 - z0) + 1e-6 * (1.0 + hx.max(hy));
            [
                [-hx - pad, -hy - pad, z0 - pad],
                [hx + pad, hy + pad, z1 + pad],
            ]
        }
        (Shape::Annulus { r1, z, .. }, _) => {
            let pad = 1e-6 * (1.0 + r1);
            [
                [-r1 - pad, -r1 - pad, z - pad],
                [r1 + pad, r1 + pad, z + pad],
            ]
        }
        (Shape::Cylinder { r, z0, z1 }, _) => {
            let pad = 1e-6 * (1.0 + r);
            [[-r - pad, -r - pad, z0 - pad], [r + pad, r + pad, z1 + pad]]
        }
    };
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for c in 0..8 {
        let q = [
            local[c & 1][0],
            local[(c >> 1) & 1][1],
            local[(c >> 2) & 1][2],
        ];
        let g = add(f.frame.o, mv(&f.frame.r, q));
        for a in 0..3 {
            lo[a] = lo[a].min(g[a]);
            hi[a] = hi[a].max(g[a]);
        }
    }
    Some([lo, hi])
}

/// 局所の法線を大域へ
pub(super) fn normal_global(f: &Face, n: V3) -> V3 {
    mv(&f.frame.r, n)
}

/// 大域の点 p から方向 k に距離 t 進んだ点
pub(super) fn along(p: V3, k: V3, t: f64) -> V3 {
    add(p, [k[0] * t, k[1] * t, k[2] * t])
}
