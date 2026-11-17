//! 3 次元のベクトルと行列（行優先の 3×3）の小さな道具。

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];

pub const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
#[inline]
pub fn normalize(a: V3) -> V3 {
    scale(a, 1.0 / norm(a))
}
#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// M v
#[inline]
pub fn mv(m: &M3, v: V3) -> V3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}
/// Mᵀ v（= v M。ベクトルを列が軸の座標系へ）
#[inline]
pub fn mtv(m: &M3, v: V3) -> V3 {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}
pub fn mm(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}
pub fn transpose(a: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[j][i];
        }
    }
    r
}
/// 列 j
#[inline]
pub fn col(m: &M3, j: usize) -> V3 {
    [m[0][j], m[1][j], m[2][j]]
}
/// 軸 ax（0 x, 1 y, 2 z）まわりの回転 [deg]
pub fn rot(ax: usize, a_deg: f64) -> M3 {
    if a_deg == 0.0 {
        return I3;
    }
    let a = a_deg.to_radians();
    let (s, c) = a.sin_cos();
    match ax {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// 2×2 の連立 J v = e の解（J は行優先 [a, b, c, d]）。特異なら非有限
#[inline]
pub(crate) fn solve2(j: [f64; 4], e: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d] = j;
    let det = a * d - b * c;
    [(d * e[0] - b * e[1]) / det, (-c * e[0] + a * e[1]) / det]
}

/// [`rot`] の角 [deg] による微分
pub(crate) fn drot(ax: usize, a_deg: f64) -> M3 {
    let a = a_deg.to_radians();
    let (s, c) = a.sin_cos();
    let k = std::f64::consts::PI / 180.0;
    let m = match ax {
        0 => [[0.0, 0.0, 0.0], [0.0, -s, -c], [0.0, c, -s]],
        1 => [[-s, 0.0, c], [0.0, 0.0, 0.0], [-c, 0.0, -s]],
        _ => [[-s, -c, 0.0], [c, -s, 0.0], [0.0, 0.0, 0.0]],
    };
    m.map(|r| r.map(|v| v * k))
}

/// 直線 p0 + t·k0 の上で、直線 p1 + s·k1 に最も近い点（3 次元の最近点。平行なら NaN）
pub(crate) fn closest_on_line(p0: V3, k0: V3, p1: V3, k1: V3) -> V3 {
    let w0 = sub(p0, p1);
    let (a, b, c) = (dot(k0, k0), dot(k0, k1), dot(k1, k1));
    let (d, f) = (dot(k0, w0), dot(k1, w0));
    add(p0, scale(k0, (b * f - c * d) / (a * c - b * b)))
}

/// 直線 p0 + t·k0 と p1 + s·k1 を y-z 面に写して交わる点の t（x は見ない。平行なら ±∞・NaN）
pub(crate) fn cross_yz_t(p0: V3, k0: V3, p1: V3, k1: V3) -> f64 {
    let det = k1[1] * k0[2] - k0[1] * k1[2];
    let (ry, rz) = (p1[1] - p0[1], p1[2] - p0[2]);
    (rz * k1[1] - ry * k1[2]) / det
}

/// 回転 R = Rx(a) Ry(b) Rz(c)（[`rot`]、順序 0 の座標ブレーク）の角 [deg]
pub fn xyz_angles(m: &M3) -> [f64; 3] {
    let b = m[0][2].atan2((m[0][0] * m[0][0] + m[0][1] * m[0][1]).sqrt());
    let a = (-m[1][2]).atan2(m[2][2]);
    let c = (-m[0][1]).atan2(m[0][0]);
    [a.to_degrees(), b.to_degrees(), c.to_degrees()]
}
