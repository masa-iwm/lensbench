//! 凸でない多角形（凹面のある玉の断面など）の塗り。egui の `convex_polygon` は先頭の点からの扇で
//! 埋めるので、両凹・メニスカスの断面では面の外まで塗ってしまう。耳切りで三角形に分けて塗る

use egui::{Color32, Pos2, Shape};

/// 多角形を塗る形（縁の線は無し）。点は輪郭の順（向きはどちらでもよい）
pub(crate) fn filled(poly: &[Pos2], color: Color32) -> Shape {
    let mut mesh = egui::Mesh::default();
    for &p in poly {
        mesh.colored_vertex(p, color);
    }
    for [a, b, c] in triangulate(poly) {
        mesh.add_triangle(a as u32, b as u32, c as u32);
    }
    Shape::mesh(mesh)
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// 単純な多角形を三角形（点の番号）に分ける（耳切り）。重なった点・一直線の点は飛ばす。
/// 自分と交わる輪郭などで耳が見つからなくなったら、残りは扇で埋める
pub(crate) fn triangulate(poly: &[Pos2]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let area2: f32 = (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    // 左回り（面積が正）として扱う
    let sgn = if area2 < 0.0 { -1.0 } else { 1.0 };
    let scale = poly
        .iter()
        .fold(0.0f32, |m, p| m.max(p.x.abs()).max(p.y.abs()))
        .max(1.0);
    let eps = 1e-6 * scale * scale;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < 2 * n * n {
        guard += 1;
        let m = idx.len();
        let mut cut = None;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            let cr = sgn * cross(a, b, c);
            if cr.abs() <= eps {
                // 重なった点・一直線の点は三角形を作らずに外す
                cut = Some((i, None));
                break;
            }
            if cr < 0.0 {
                continue;
            }
            let blocked = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = poly[j];
                if p == a || p == b || p == c {
                    return false;
                }
                sgn * cross(a, b, p) >= 0.0
                    && sgn * cross(b, c, p) >= 0.0
                    && sgn * cross(c, a, p) >= 0.0
            });
            if !blocked {
                cut = Some((i, Some([ia, ib, ic])));
                break;
            }
        }
        match cut {
            Some((i, tri)) => {
                out.extend(tri);
                idx.remove(i);
            }
            None => break,
        }
    }
    if idx.len() >= 3 {
        for w in 1..idx.len() - 1 {
            let t = [idx[0], idx[w], idx[w + 1]];
            if cross(poly[t[0]], poly[t[1]], poly[t[2]]).abs() > eps {
                out.push(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 両凹の断面（凹面を 2 本の弧に見立てる）: 三角形の重心がどれも断面の中で、面積の和が断面の面積
    #[test]
    fn biconcave_triangles_stay_inside() {
        let arc = |x0: f32, bulge: f32, rev: bool| -> Vec<Pos2> {
            let mut v: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let y = -4.0 + 8.0 * i as f32 / 16.0;
                    Pos2::new(x0 + bulge * (1.0 - (y / 4.0).powi(2)), y)
                })
                .collect();
            if rev {
                v.reverse();
            }
            v
        };
        // 左の面は右へ、右の面は左へ凹む（中心が薄い）
        let mut poly = arc(0.0, 1.5, false);
        poly.extend(arc(4.0, -1.5, true));
        let tris = triangulate(&poly);
        assert_eq!(tris.len(), poly.len() - 2);
        let area = |a: Pos2, b: Pos2, c: Pos2| cross(a, b, c).abs() / 2.0;
        let mut sum = 0.0;
        for t in &tris {
            let (a, b, c) = (poly[t[0]], poly[t[1]], poly[t[2]]);
            let g = Pos2::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
            assert!(super::super::inside(&poly, g), "{t:?} {g:?}");
            sum += area(a, b, c);
        }
        let n = poly.len();
        let poly_area = (0..n)
            .map(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                a.x * b.y - b.x * a.y
            })
            .sum::<f32>()
            .abs()
            / 2.0;
        assert!(
            (sum - poly_area).abs() < 1e-3 * poly_area,
            "{sum} {poly_area}"
        );
    }

    /// 向きが逆・重なった点があっても埋まる
    #[test]
    fn handles_reverse_order_and_duplicates() {
        let mut poly = vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 0.0),
            Pos2::new(4.0, 0.0),
            Pos2::new(2.0, 1.0),
            Pos2::new(4.0, 2.0),
            Pos2::new(0.0, 2.0),
        ];
        let a = triangulate(&poly).len();
        poly.reverse();
        let b = triangulate(&poly).len();
        assert_eq!((a, b), (3, 3));
    }
}
