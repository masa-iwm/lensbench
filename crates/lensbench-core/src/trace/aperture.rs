//! 開口の判定（円・矩形・楕円・スパイダー。遮蔽は内側が「外」）

use crate::math::*;
use crate::surface::Aperture;
use crate::tol::{aperture_edge, APERTURE_EDGE};

/// 開口の外か（遮蔽なら内側が「外」）。pl は面の局所座標の交点
#[inline]
pub(crate) fn aperture_out(a: Aperture, pl: V3) -> bool {
    match a {
        Aperture::Circular { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            let (lo, hi) = (min - aperture_edge(min), max + aperture_edge(max));
            match ring_side(u, v, lo, hi) {
                Some(inside) => !inside,
                None => {
                    let r = u.hypot(v);
                    r > hi || r < lo
                }
            }
        }
        Aperture::Floating { semi } => {
            let hi = semi + aperture_edge(semi);
            match ring_side(pl[0], pl[1], 0.0, hi) {
                Some(inside) => !inside,
                None => (pl[0]).hypot(pl[1]) > hi,
            }
        }
        Aperture::Obsc { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            let (lo, hi) = (min - aperture_edge(min), max + aperture_edge(max));
            match ring_side(u, v, lo, hi) {
                Some(inside) => inside,
                None => {
                    let r = u.hypot(v);
                    r <= hi && r >= lo
                }
            }
        }
        Aperture::Rect { hx, hy, dx, dy } => {
            let e = aperture_edge(hx.abs().max(hy.abs()));
            (pl[0] - dx).abs() > hx + e || (pl[1] - dy).abs() > hy + e
        }
        Aperture::Ellipse { hx, hy, dx, dy } => {
            let (u, v) = ((pl[0] - dx) / hx, (pl[1] - dy) / hy);
            u * u + v * v > 1.0 + 2.0 * APERTURE_EDGE
        }
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => spider_hit(width, arms, pl[0] - dx, pl[1] - dy),
        Aperture::None => false,
    }
}

/// 開口の縁までの符号付きの距離 [mm]（通る側が正、`aperture_out` の境界で 0）。x y は面の局所座標の交点。
/// 楕円は半軸で正規化した半径からの近似（符号と 0 の位置は正しい）。スパイダーは腕の帯までの距離
pub(crate) fn aperture_margin(a: Aperture, x: f64, y: f64) -> f64 {
    let ring = |u: f64, v: f64, min: f64, max: f64| {
        let r = u.hypot(v);
        let out = max - r;
        if min > 0.0 {
            out.min(r - min)
        } else {
            out
        }
    };
    match a {
        Aperture::Circular { min, max, dx, dy } => ring(x - dx, y - dy, min, max),
        Aperture::Floating { semi } => semi - x.hypot(y),
        Aperture::Obsc { min, max, dx, dy } => {
            let r = (x - dx).hypot(y - dy);
            if min > 0.0 {
                (r - max).max(min - r)
            } else {
                r - max
            }
        }
        Aperture::Rect { hx, hy, dx, dy } => (hx - (x - dx).abs()).min(hy - (y - dy).abs()),
        Aperture::Ellipse { hx, hy, dx, dy } => {
            let (u, v) = ((x - dx) / hx, (y - dy) / hy);
            (1.0 - u.hypot(v)) * hx.min(hy)
        }
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => {
            let (u, v) = (x - dx, y - dy);
            let hw = 0.5 * width;
            (0..arms)
                .map(|k| {
                    let (s, c) = (std::f64::consts::TAU * k as f64 / arms as f64).sin_cos();
                    let (along, across) = (u * c + v * s, v * c - u * s);
                    if along >= 0.0 {
                        across.abs() - hw
                    } else {
                        // 腕の根元（中心）の側: 帯の端の線分までの距離
                        let d = (across.abs() - hw).max(0.0);
                        d.hypot(along).max(across.abs() - hw)
                    }
                })
                .fold(f64::INFINITY, f64::min)
        }
        Aperture::None => f64::INFINITY,
    }
}

/// 半径 r = hypot(u, v) が lo < r < hi（Some(true)）か r < lo・r > hi（Some(false)）かを、半径の 2 乗で十分に離れて
/// いるときだけ決める（境界の近く・NaN・無限大は None で、呼ぶ側が hypot で決める。[`crate::batch`] の開口と同じ幅）。
/// hypot は遅いので、ほとんどの光線をこれで決める（決めた結果は hypot と同じ）
#[inline(always)]
fn ring_side(u: f64, v: f64, lo: f64, hi: f64) -> Option<bool> {
    const M: f64 = crate::tol::RADIUS2_MARGIN;
    let r2 = u * u + v * v;
    let (has_lo, has_hi) = (lo > 0.0, hi > 0.0);
    let (lo2, hi2) = (lo * lo, hi * hi);
    let inside = has_hi & (r2 < hi2 * (1.0 - M)) & (!has_lo | (r2 > lo2 * (1.0 + M)));
    let outside = (has_lo & (r2 < lo2 * (1.0 - M)))
        | (has_hi & (r2 > hi2 * (1.0 + M)) & (r2 < f64::INFINITY));
    if inside {
        Some(true)
    } else if outside {
        Some(false)
    } else {
        None
    }
}

/// スパイダーの腕に当たるか（x y は開口の中心から）。腕 k は角度 2πk/arms の向きの、中心から外への幅 width の帯
/// （境界を含む）。OpticStudio 2026 R1.01 と同じく余裕をとらない: 幅 0 では +x の腕の上（y = 0、x ≥ 0）だけ遮り、
/// cos 90° などが 0 にならないほかの腕は中心のほか遮らない（実測）
fn spider_hit(width: f64, arms: u32, x: f64, y: f64) -> bool {
    let hw = 0.5 * width;
    (0..arms).any(|k| {
        let (s, c) = (std::f64::consts::TAU * k as f64 / arms as f64).sin_cos();
        x * c + y * s >= 0.0 && (y * c - x * s).abs() <= hw
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_sign_matches_aperture_out() {
        let aps = [
            Aperture::Circular {
                min: 0.3,
                max: 1.2,
                dx: 0.1,
                dy: -0.2,
            },
            Aperture::Floating { semi: 0.9 },
            Aperture::Obsc {
                min: 0.0,
                max: 0.4,
                dx: 0.0,
                dy: 0.1,
            },
            Aperture::Obsc {
                min: 0.2,
                max: 0.5,
                dx: 0.0,
                dy: 0.0,
            },
            Aperture::Rect {
                hx: 0.8,
                hy: 0.5,
                dx: 0.1,
                dy: 0.0,
            },
            Aperture::Ellipse {
                hx: 0.9,
                hy: 0.6,
                dx: 0.0,
                dy: 0.2,
            },
            Aperture::Spider {
                width: 0.1,
                arms: 3,
                dx: 0.05,
                dy: 0.0,
            },
        ];
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) * 3.0 - 1.5
        };
        for a in aps {
            for _ in 0..20000 {
                let (x, y) = (rnd(), rnd());
                let m = aperture_margin(a, x, y);
                if m.abs() < 1e-6 {
                    continue;
                }
                assert_eq!(m < 0.0, aperture_out(a, [x, y, 0.0]), "{a:?} {x} {y} {m}");
            }
        }
    }

    #[test]
    fn margin_is_the_distance_for_circle_and_rect() {
        let c = Aperture::Circular {
            min: 0.0,
            max: 2.0,
            dx: 0.0,
            dy: 0.0,
        };
        assert!((aperture_margin(c, 1.5, 0.0) - 0.5).abs() < 1e-15);
        assert!((aperture_margin(c, 0.0, -2.5) + 0.5).abs() < 1e-15);
        let r = Aperture::Rect {
            hx: 1.0,
            hy: 2.0,
            dx: 0.0,
            dy: 0.0,
        };
        assert!((aperture_margin(r, 0.75, 0.0) - 0.25).abs() < 1e-15);
        assert!((aperture_margin(r, 0.0, 2.5) + 0.5).abs() < 1e-15);
    }
}
