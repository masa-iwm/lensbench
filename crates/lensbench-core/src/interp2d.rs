//! 平面の点の上の連続な線形補間（Delaunay 三角形分割と重心座標）。定義した視野以外の点のケラレ係数に使う

/// 三角形の頂点の番号
type Tri = (usize, usize, usize);

/// Delaunay の三角形分割（外接円の中に他の点がない、総当たり。視野は多くても数十）。同じ円に 4 点以上が
/// 乗ると（格子の視野など）候補の三角形が重なるので、番号の順に重ならないものだけを取る（凸多角形を
/// 重ならない三角形で埋め尽くす）
/// `scale` は座標の大きさ。判定の許容差は大きさに対する相対（計算上は同じ円に乗る点を、丸め誤差の符号で
/// 外接円の中と見て捨てると分割に穴があく）
fn delaunay(pts: &[[f64; 2]], scale: f64) -> Vec<Tri> {
    let tol_area = 1e-12 * scale * scale;
    let tol_circle = 1e-9 * scale.powi(4);
    let n = pts.len();
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut tris: Vec<Tri> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                let (a, b, c) = (pts[i], pts[j], pts[k]);
                let area = cross(a, b, c);
                if area.abs() <= tol_area {
                    continue;
                }
                let in_circle = |d: [f64; 2]| {
                    let m = |p: [f64; 2]| {
                        [
                            p[0] - d[0],
                            p[1] - d[1],
                            (p[0] - d[0]).powi(2) + (p[1] - d[1]).powi(2),
                        ]
                    };
                    let (u, v, w) = (m(a), m(b), m(c));
                    let det = u[0] * (v[1] * w[2] - v[2] * w[1])
                        - u[1] * (v[0] * w[2] - v[2] * w[0])
                        + u[2] * (v[0] * w[1] - v[1] * w[0]);
                    det * area.signum() > tol_circle
                };
                if !(0..n).all(|l| l == i || l == j || l == k || !in_circle(pts[l])) {
                    continue;
                }
                let t = [a, b, c];
                if tris
                    .iter()
                    .all(|&(p, q, r)| !tri_overlap(&t, &[pts[p], pts[q], pts[r]], tol_area))
                {
                    tris.push((i, j, k));
                }
            }
        }
    }
    tris
}

/// 2 つの三角形の内部が重なるか（分離軸: どちらかの辺で、もう一方の 3 点がすべて外側か辺の上なら重ならない）
fn tri_overlap(t: &[[f64; 2]; 3], u: &[[f64; 2]; 3], tol: f64) -> bool {
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let separated = |t: &[[f64; 2]; 3], u: &[[f64; 2]; 3]| {
        let sgn = cross(t[0], t[1], t[2]).signum();
        (0..3).any(|e| {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            u.iter().all(|&p| sgn * cross(a, b, p) <= tol)
        })
    };
    !(separated(t, u) || separated(u, t))
}

/// 平面の点 pts の上の連続な線形補間の重み（点の番号, 重み）。Delaunay 三角形分割の三角形の中は重心座標、
/// 外側は凸包の縁の最も近い点（線分の両端の線形）、点が 1 つ・一直線なら最も近い線分・点
pub(crate) fn interp_weights(pts: &[[f64; 2]], q: [f64; 2]) -> Vec<(usize, f64)> {
    let n = pts.len();
    let eps = 1e-12
        * pts
            .iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0, f64::max);
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    // 分割は視野が同じなら同じなので、スレッドごとに直前の結果を使い回す（走査で毎回作らない）
    thread_local! {
        static TRIS: std::cell::RefCell<(Vec<[f64; 2]>, Vec<Tri>)> =
            const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
    }
    let tris = TRIS.with(|c| {
        let mut c = c.borrow_mut();
        if c.0.as_slice() != pts {
            *c = (pts.to_vec(), delaunay(pts, eps / 1e-12));
        }
        c.1.clone()
    });
    for &(i, j, k) in &tris {
        let (a, b, c) = (pts[i], pts[j], pts[k]);
        let area = cross(a, b, c);
        let (wa, wb, wc) = (
            cross(q, b, c) / area,
            cross(a, q, c) / area,
            cross(a, b, q) / area,
        );
        let tol = -1e-12;
        if wa >= tol && wb >= tol && wc >= tol {
            return vec![(i, wa), (j, wb), (k, wc)];
        }
    }
    // 外側（または三角形がない）: 三角形の辺（なければ全部の点の組）の中で最も近い線分の点
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for &(i, j, k) in &tris {
        edges.extend([(i, j), (j, k), (i, k)]);
    }
    if edges.is_empty() {
        for i in 0..n {
            for j in i + 1..n {
                edges.push((i, j));
            }
        }
    }
    let mut best: (f64, Vec<(usize, f64)>) = (f64::INFINITY, vec![(0, 1.0)]);
    if n == 1 {
        return best.1;
    }
    for (i, j) in edges {
        let (a, b) = (pts[i], pts[j]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 > 0.0 {
            (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = [a[0] + t * d[0], a[1] + t * d[1]];
        let dist = (q[0] - p[0]).hypot(q[1] - p[1]);
        if dist < best.0 - eps {
            best = (dist, vec![(i, 1.0 - t), (j, t)]);
        }
    }
    best.1
}
