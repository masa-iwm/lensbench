//! 開口の判定（円・矩形・楕円・スパイダー。遮蔽は内側が「外」）

use crate::math::*;
use crate::surface::Aperture;
use crate::tol::APERTURE_EDGE;

/// 開口の外か（遮蔽なら内側が「外」）。pl は面の局所座標の交点
#[inline]
pub(crate) fn aperture_out(a: Aperture, pl: V3) -> bool {
    match a {
        Aperture::Circular { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            match ring_side(u, v, min - APERTURE_EDGE, max + APERTURE_EDGE) {
                Some(inside) => !inside,
                None => {
                    let r = u.hypot(v);
                    r > max + APERTURE_EDGE || r < min - APERTURE_EDGE
                }
            }
        }
        Aperture::Floating { semi } => match ring_side(pl[0], pl[1], 0.0, semi + APERTURE_EDGE) {
            Some(inside) => !inside,
            None => (pl[0]).hypot(pl[1]) > semi + APERTURE_EDGE,
        },
        Aperture::Obsc { min, max, dx, dy } => {
            let (u, v) = (pl[0] - dx, pl[1] - dy);
            match ring_side(u, v, min - APERTURE_EDGE, max + APERTURE_EDGE) {
                Some(inside) => inside,
                None => {
                    let r = u.hypot(v);
                    r <= max + APERTURE_EDGE && r >= min - APERTURE_EDGE
                }
            }
        }
        Aperture::Rect { hx, hy, dx, dy } => {
            (pl[0] - dx).abs() > hx + APERTURE_EDGE || (pl[1] - dy).abs() > hy + APERTURE_EDGE
        }
        Aperture::Ellipse { hx, hy, dx, dy } => {
            let (u, v) = ((pl[0] - dx) / hx, (pl[1] - dy) / hy);
            u * u + v * v > 1.0 + APERTURE_EDGE
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
