//! 面の形の項（[`TermKind::Shape`]）: 傾けた向きの矢高と、削る体積が最小の最適球

use super::*;
use lensbench_core::math::rot;

/// 最適球の当てはめの半径の向きの点の数
const BFS_POINTS: usize = 4001;

pub(super) fn check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let TermKind::Shape { surf, q, .. } = k else {
        unreachable!("not a shape term");
    };
    if *surf == 0 || *surf >= s.image() {
        invalid!("term {i}: surface {surf} (1 to {})", s.image() - 1);
    }
    if let ShapeQ::Bfs { rmin, rmax, .. } = q {
        if *rmin < 0.0 || (*rmax != 0.0 && rmax <= rmin) {
            invalid!("term {i}: best-fit sphere from radius {rmin} to {rmax}");
        }
    }
    term(s, k).map(|_| ())
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::Shape { surf, q, target } = k else {
        unreachable!("not a shape term");
    };
    let v = match *q {
        ShapeQ::Tsag { p, tilt, flat } => tsag(s, *surf, p, tilt, flat)?,
        ShapeQ::Bfs { data, rmin, rmax } => bfs(s, *surf, data, rmin, rmax)?,
    };
    if !v.is_finite() {
        invalid!("the shape value of surface {surf} is not finite");
    }
    Ok((vec![v - target], None))
}

/// 点 p から傾けた z 軸の向きに面 j まで測った距離
fn tsag(s: &System, j: usize, p: [f64; 3], tilt: [f64; 3], flat: bool) -> Result<f64> {
    let f = &s.surfaces[j];
    let sh = f.shape();
    // 向き Rz(−tz) Ry(−ty) Rx(−tx) ẑ（OpticStudio 26.1 の TSAG で実測。tx が正なら +y へ、ty が正なら −x へ傾く）
    let mm = lensbench_core::math::mm;
    let m = mm(&mm(&rot(2, -tilt[2]), &rot(1, -tilt[1])), &rot(0, -tilt[0]));
    let d = [m[0][2], m[1][2], m[2][2]];
    // 平らにする半径（玉の縁がクリア半径より外の面だけ、面の形はチップゾーンまで。OpticStudio 26.1 で実測）
    let edge = if flat && s.lens_radius(j) > f.semi * (1.0 + 1e-12) {
        s.flat_from(j)
    } else {
        f64::INFINITY
    };
    // 矢高と傾き（平らな所は縁の矢高・傾き 0）
    let sag = |x: f64, y: f64| -> (f64, f64, f64) {
        let r = x.hypot(y);
        if r > edge {
            (sh.sag(x * edge / r, y * edge / r).0, 0.0, 0.0)
        } else {
            sh.sag(x, y)
        }
    };
    let mut t = sag(p[0], p[1]).0 - p[2];
    for _ in 0..100 {
        let (x, y) = (p[0] + t * d[0], p[1] + t * d[1]);
        let (z, zx, zy) = sag(x, y);
        let g = p[2] + t * d[2] - z;
        let dg = d[2] - zx * d[0] - zy * d[1];
        let dt = g / dg;
        if !dt.is_finite() {
            break;
        }
        t -= dt;
        if dt.abs() <= 1e-13 * (1.0 + t.abs()) {
            return Ok(t);
        }
    }
    invalid!(
        "the line from ({}, {}, {}) doesn't meet surface {j}",
        p[0],
        p[1],
        p[2]
    )
}

/// 面 j の y の向きの断面の、半径 rmin〜rmax の最適球の量
fn bfs(s: &System, j: usize, data: BfsData, rmin: f64, rmax: f64) -> Result<f64> {
    let f = &s.surfaces[j];
    let sh = f.shape();
    let r1 = if rmax == 0.0 { f.semi } else { rmax };
    if r1 <= rmin {
        invalid!("best-fit sphere of surface {j} from radius {rmin} to {r1}");
    }
    let n = BFS_POINTS;
    let r: Vec<f64> = (0..n)
        .map(|i| rmin + (r1 - rmin) * i as f64 / (n - 1) as f64)
        .collect();
    let prof: Vec<(f64, f64)> = r
        .iter()
        .map(|&y| {
            let (z, _, zy) = sh.sag(0.0, y);
            (z, zy)
        })
        .collect();
    if prof.iter().any(|(z, d)| !z.is_finite() || !d.is_finite()) {
        invalid!("the sag of surface {j} is not finite up to radius {r1}");
    }
    // 空気の側: 面の後がガラスなら球は前（z が小さい側）、でなければ後
    let m = &f.mat;
    let below = m.is_medium() && !m.is_air();
    let sph = |c: f64, y: f64| c * y * y / (1.0 + (1.0 - c * c * y * y).sqrt());
    // 曲率 c の球の、頂点の z と削る深さ
    let fit = |c: f64| -> (f64, Vec<f64>) {
        let dz: Vec<f64> = r.iter().zip(&prof).map(|(&y, p)| p.0 - sph(c, y)).collect();
        let d = if below {
            dz.iter().copied().fold(f64::INFINITY, f64::min)
        } else {
            dz.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        };
        let dep = dz.iter().map(|v| (v - d).abs()).collect();
        (d, dep)
    };
    let trapz = |v: &[f64]| -> f64 {
        let h = (r1 - rmin) / (n - 1) as f64;
        let w = |i: usize| if i == 0 || i == n - 1 { 0.5 } else { 1.0 };
        (0..n).map(|i| w(i) * v[i]).sum::<f64>() * h
    };
    let volume = |dep: &[f64]| -> f64 {
        let a: Vec<f64> = r
            .iter()
            .zip(dep)
            .map(|(y, d)| 2.0 * std::f64::consts::PI * y * d)
            .collect();
        trapz(&a)
    };
    // 曲率の探す幅: 近軸の曲率と、頂点と縁を通る球の曲率のまわり（球が r1 まで届く範囲）
    let ze = prof[n - 1].0 - prof[0].0;
    let ca = 2.0 * ze / (r1 * r1 - rmin * rmin + ze * ze).max(f64::MIN_POSITIVE);
    let cp = sh.paraxial_curv();
    let w = (ca - cp).abs() + 1.0 / (r1 * 1e3);
    let lim = (1.0 - 1e-9) / r1;
    let (mut a, mut b) = ((ca.min(cp) - w).max(-lim), (ca.max(cp) + w).min(lim));
    let g = (5f64.sqrt() - 1.0) / 2.0;
    let obj = |c: f64| volume(&fit(c).1);
    let (mut x1, mut x2) = (b - g * (b - a), a + g * (b - a));
    let (mut f1, mut f2) = (obj(x1), obj(x2));
    for _ in 0..200 {
        if (b - a).abs() <= 1e-15 * (1.0 + a.abs()) {
            break;
        }
        if f1 < f2 {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = b - g * (b - a);
            f1 = obj(x1);
        } else {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = a + g * (b - a);
            f2 = obj(x2);
        }
    }
    // 平面に当てはめると曲率は探す幅の丸めの残り（1e-16 ほど）になるので 0 にする（半径はパラメータ `r` と同じく平面で 0）
    let c = match (a + b) / 2.0 {
        c if (c * r1).abs() < 1e-12 => 0.0,
        c => c,
    };
    let (d, dep) = fit(c);
    let slope: Vec<f64> = r
        .iter()
        .zip(&prof)
        .map(|(&y, p)| (p.1 - c * y / (1.0 - c * c * y * y).sqrt()).abs())
        .collect();
    // 面積の重みの RMS
    let rms = |v: &[f64]| -> f64 {
        let num: f64 = r.iter().zip(v).map(|(y, x)| y * x * x).sum();
        let den: f64 = r.iter().sum();
        (num / den).sqrt()
    };
    Ok(match data {
        BfsData::Curv => c,
        BfsData::Radius if c == 0.0 => 0.0,
        BfsData::Radius => 1.0 / c,
        BfsData::Vertex => d,
        BfsData::Depth => dep.iter().copied().fold(0.0, f64::max),
        BfsData::Volume => volume(&dep),
        BfsData::Slope => slope.iter().copied().fold(0.0, f64::max),
        BfsData::DepthRms => rms(&dep),
        BfsData::SlopeRms => rms(&slope),
    })
}
