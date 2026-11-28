//! 非順次の追跡の光源: 系の視野ごとの瞳の格子（[`system_source`]）と、平行光の円板（[`disc_source`]、迷光の PST）

use super::*;

/// 無限遠物体の光線の始点（面 1 の頂点を通る、方向に垂直な面の上）を、面 1 より前へ下げる（面 1 の形が始点より
/// 前に出ていても当たるように）。有限物体の始点（物点）はそのまま
pub fn back_off(s: &System, spec: ObjSpec, p: V3, k: V3) -> V3 {
    if !matches!(spec, ObjSpec::Dir(_)) {
        return p;
    }
    let j1 = (1..s.surfaces.len())
        .find(|&j| !s.surfaces[j].ignore && !s.surfaces[j].is_coord_break())
        .unwrap_or(1);
    let semi = s.surfaces.get(j1).map_or(0.0, |d| d.semi);
    let k = normalize(k);
    let ax = lensbench_core::math::col(&s.object_frame().r, 2);
    let c = dot(k, ax).abs().max(0.1);
    let l = 2.0 * (semi + 1.0) / c;
    [p[0] - k[0] * l, p[1] - k[1] * l, p[2] - k[2] * l]
}

/// 系 s の視野ごとに、瞳の格子（一辺 grid 本、円の中）の光線を光源にする。全パワー power [W] を光線に等しく分け、
/// polarization が None なら偏光なし（1 本を直交する 2 本に）、あれば物体の座標系の (Ex, Ey)。エイミングで解けない
/// 光線は除く
pub fn system_source(
    s: &System,
    fields: &[usize],
    grid: usize,
    w: usize,
    power: f64,
    polarization: Option<[C64; 2]>,
) -> Result<Vec<StartRay>> {
    if !(power.is_finite() && power > 0.0) {
        invalid!("nsc: source power {power} W (must be positive)");
    }
    if grid < 1 {
        invalid!("nsc: pupil grid {grid} (1 or more)");
    }
    let (px, py) = if grid == 1 {
        (vec![0.0], vec![0.0])
    } else {
        lensbench_core::opd::pupil_grid(grid)
    };
    let mut rays: Vec<(V3, V3)> = Vec::new();
    for &f in fields {
        if f >= s.fields.len() {
            invalid!("no field {} (1 to {})", f + 1, s.fields.len());
        }
        let (hx, hy) = s.field_sel_norm(FieldSel::Index(f), w)?;
        let spec: ObjSpec = s.object_spec(hx, hy)?;
        for (x, y) in px.iter().zip(&py) {
            if let Some(v) = s.aim_pupil(spec, [*x, *y], w) {
                let (p, k) = s.pupil_ray(spec, v);
                rays.push((back_off(s, spec, p, k), k));
            }
        }
    }
    if rays.is_empty() {
        invalid!("nsc: no source ray can be aimed");
    }
    let r = s.object_frame().r;
    let each = power / rays.len() as f64;
    Ok(rays
        .into_iter()
        .flat_map(|(p, k)| match polarization {
            Some(j) => vec![StartRay::polarized(p, k, j, &r, each)],
            None => StartRay::unpolarized(p, k, &r, each).to_vec(),
        })
        .collect())
}

/// 平行光の光源（迷光の PST）: 方向 k（大域）に垂直で、点 c を通る直線を中心にした半径 radius の円板の上の格子
/// （一辺 grid 本、円の中）の光線。始点は場面 sc の全部の物より手前（c から −k の向きへ場面の大きさの分）。
/// 全パワー power [W] を光線に等しく分け、polarization が None なら偏光なし（1 本を直交する 2 本に）、あれば
/// 座標系 r の (Ex, Ey)
#[allow(clippy::too_many_arguments)]
pub fn disc_source(
    sc: &Scene,
    c: V3,
    k: V3,
    radius: f64,
    grid: usize,
    power: f64,
    polarization: Option<[C64; 2]>,
    r: &[[f64; 3]; 3],
) -> Result<Vec<StartRay>> {
    if !(power.is_finite() && power > 0.0) {
        invalid!("nsc: source power {power} W (must be positive)");
    }
    if !(radius.is_finite() && radius > 0.0) {
        invalid!("nsc: source radius {radius} mm (must be positive)");
    }
    if grid < 1 {
        invalid!("nsc: source grid {grid} (1 or more)");
    }
    let k = normalize(k);
    // 場面の大きさ: c から各面の原点までの距離＋面の大きさ
    let far = sc
        .faces
        .iter()
        .map(|f| {
            let d = norm(lensbench_core::math::sub(f.frame.o, c));
            let e = match &f.shape {
                Shape::Optical { extent, .. } => *extent,
                Shape::Annulus { r1, z, .. } => r1 + z.abs(),
                Shape::Cylinder { r, z0, z1 } => r + z0.abs().max(z1.abs()),
            };
            d + e
        })
        .fold(0.0, f64::max);
    let back = far + radius + 1.0;
    let (u, v) = basis(k);
    let mut pts = Vec::new();
    if grid == 1 {
        pts.push((0.0, 0.0));
    } else {
        let h = 2.0 / (grid - 1) as f64;
        for i in 0..grid {
            for j in 0..grid {
                let (x, y) = (-1.0 + i as f64 * h, -1.0 + j as f64 * h);
                if x * x + y * y <= 1.0 + 1e-12 {
                    pts.push((x * radius, y * radius));
                }
            }
        }
    }
    let each = power / pts.len() as f64;
    Ok(pts
        .into_iter()
        .flat_map(|(x, y)| {
            let p = [0, 1, 2].map(|i| c[i] + x * u[i] + y * v[i] - back * k[i]);
            match polarization {
                Some(j) => vec![StartRay::polarized(p, k, j, r, each)],
                None => StartRay::unpolarized(p, k, r, each).to_vec(),
            }
        })
        .collect())
}
