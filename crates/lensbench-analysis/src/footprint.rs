//! フットプリント（面の上で光束の通る範囲）と、けられずに通る光線の割合（Vignetting Plot）。
//!
//! 瞳の格子（`lensbench_core::opd::pupil_grid`）の光線を系のケラレ係数込みで追跡し、選んだ面の局所座標の交点を集める。
//! その面より前でけられた光線は面に届かないので点にしない（その面でけられる光線は点にし、外れたことは割合に出す）。

use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, Launch, RecSel, Trace};
use lensbench_core::{invalid, Result, System};

/// 円を描く点の数
const CIRCLE: usize = 96;

/// 視野 field の瞳の点 (px, py) の光線を追跡する。vig なら系のケラレ係数を使い（OpticStudio と同じ）、でなければ瞳を縮めずに
/// 面の開口だけでけられる（物理の開口だけの割合）
fn trace(
    s: &System,
    field: FieldSel,
    w: usize,
    px: &[f64],
    py: &[f64],
    vig: bool,
    rec: RecSel<'_>,
) -> Result<Trace> {
    if vig {
        return s.try_trace(field, px, py, Some(w), None, rec);
    }
    s.check_crystals()?;
    let (hx, hy) = s.field_sel_norm(field, w)?;
    let l = Launch::Pupil {
        spec: s.object_spec(hx, hy)?,
        signed: false,
        px,
        py,
        vig: Some([0.0; 5]),
        hxy: Some((hx, hy)),
    };
    Ok(s.trace_launch(&l, w, None, rec))
}

/// 1 視野・1 波長のフットプリント
#[derive(Debug, Clone, PartialEq)]
pub struct Footprint {
    /// 面に届いた光線の交点 (x, y)（面の局所座標）
    pub points: Vec<[f64; 2]>,
    /// 瞳の縁（半径 1 の CIRCLE 点）の光線の交点。面より前でけられた光線は NaN
    pub edge: Vec<[f64; 2]>,
    /// 交点の x・y の最小・最大（届いた光線が無ければ NaN）
    pub x: [f64; 2],
    pub y: [f64; 2],
    /// 面の頂点からの交点の距離の最大
    pub max_radius: f64,
    /// 瞳の格子の光線の数
    pub rays: usize,
    /// 面に届いた光線の数（面より前でけられていない）
    pub reached: usize,
    /// その面でけられた光線の数
    pub clipped: usize,
    /// 像面までけられずに通った光線の数
    pub through: usize,
}

fn circle(r: f64, cx: f64, cy: f64) -> Vec<[f64; 2]> {
    (0..=CIRCLE)
        .map(|k| {
            let a = 2.0 * std::f64::consts::PI * k as f64 / CIRCLE as f64;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

/// 面 j の開口の輪郭（面の局所座標の閉じた折れ線の並び）。開口の無い面・自動の半径は半直径の円
pub fn aperture_outline(s: &System, j: usize) -> Vec<Vec<[f64; 2]>> {
    let f = &s.surfaces[j];
    let semi = f.semi.abs();
    let clear = || -> Vec<Vec<[f64; 2]>> {
        if semi > 0.0 && semi.is_finite() {
            vec![circle(semi, 0.0, 0.0)]
        } else {
            Vec::new()
        }
    };
    match f.ap {
        Aperture::None => clear(),
        Aperture::Floating { semi: r } => vec![circle(r.abs(), 0.0, 0.0)],
        Aperture::Circular { min, max, dx, dy } => {
            let mut v = vec![circle(max, dx, dy)];
            if min > 0.0 {
                v.push(circle(min, dx, dy));
            }
            v
        }
        Aperture::Rect { hx, hy, dx, dy } => vec![vec![
            [dx - hx, dy - hy],
            [dx + hx, dy - hy],
            [dx + hx, dy + hy],
            [dx - hx, dy + hy],
            [dx - hx, dy - hy],
        ]],
        Aperture::Ellipse { hx, hy, dx, dy } => vec![(0..=CIRCLE)
            .map(|k| {
                let a = 2.0 * std::f64::consts::PI * k as f64 / CIRCLE as f64;
                [dx + hx * a.cos(), dy + hy * a.sin()]
            })
            .collect()],
        Aperture::Obsc { min, max, dx, dy } => {
            let mut v = clear();
            v.push(circle(max, dx, dy));
            if min > 0.0 {
                v.push(circle(min, dx, dy));
            }
            v
        }
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => {
            let mut v = clear();
            let len = if semi > 0.0 { semi + dx.hypot(dy) } else { 1.0 };
            for k in 0..arms.max(1) {
                let a = 2.0 * std::f64::consts::PI * k as f64 / arms.max(1) as f64;
                let (c, sn) = (a.cos(), a.sin());
                let (ox, oy) = (-sn * width / 2.0, c * width / 2.0);
                v.push(vec![
                    [dx + ox, dy + oy],
                    [dx + ox + len * c, dy + oy + len * sn],
                ]);
                v.push(vec![
                    [dx - ox, dy - oy],
                    [dx - ox + len * c, dy - oy + len * sn],
                ]);
            }
            v
        }
    }
}

/// 視野 field・波長 w の、面 j のフットプリント。瞳の格子は n×n（単位円の内）。vig は系のケラレ係数を使うか
pub fn footprint(
    s: &System,
    field: FieldSel,
    w: usize,
    j: usize,
    n: usize,
    vig: bool,
) -> Result<Footprint> {
    if j == 0 || j > s.image() {
        invalid!("surface {j} (1 to {})", s.image());
    }
    if s.surfaces[j].ignore {
        invalid!("surface {j} is ignored");
    }
    let (px, py) = lensbench_core::opd::pupil_grid(n.max(2));
    let surfs = [j];
    let t = trace(s, field, w, &px, &py, vig, RecSel::Surfs(&surfs))?;
    let Some(rec) = t.rec_at(j) else {
        invalid!("surface {j} has no ray record");
    };
    let mut fp = Footprint {
        points: Vec::new(),
        edge: Vec::new(),
        x: [f64::INFINITY, f64::NEG_INFINITY],
        y: [f64::INFINITY, f64::NEG_INFINITY],
        max_radius: f64::NAN,
        rays: px.len(),
        reached: 0,
        clipped: 0,
        through: 0,
    };
    let reaches = |i: usize| {
        let r = &t.rays[i];
        let stopped_before = r.status.surf().is_some_and(|k| k < j);
        let vig_before = r.vig != 0 && (r.vig as usize) < j;
        !stopped_before && !vig_before
    };
    let mut rmax: f64 = f64::NAN;
    for i in 0..px.len() {
        if t.ok(i) {
            fp.through += 1;
        }
        if !reaches(i) {
            continue;
        }
        if t.rays[i].vig as usize == j {
            fp.clipped += 1;
        }
        let p = rec.xyz[i];
        if !(p[0].is_finite() && p[1].is_finite()) {
            continue;
        }
        fp.reached += 1;
        fp.points.push([p[0], p[1]]);
        fp.x = [fp.x[0].min(p[0]), fp.x[1].max(p[0])];
        fp.y = [fp.y[0].min(p[1]), fp.y[1].max(p[1])];
        let r = p[0].hypot(p[1]);
        rmax = if rmax.is_nan() { r } else { rmax.max(r) };
    }
    fp.max_radius = rmax;
    if fp.reached == 0 {
        fp.x = [f64::NAN; 2];
        fp.y = [f64::NAN; 2];
    }
    // 瞳の縁
    let (ex, ey): (Vec<f64>, Vec<f64>) = (0..CIRCLE)
        .map(|k| {
            let a = 2.0 * std::f64::consts::PI * k as f64 / CIRCLE as f64;
            (a.cos(), a.sin())
        })
        .unzip();
    let te = trace(s, field, w, &ex, &ey, vig, RecSel::Surfs(&surfs))?;
    if let Some(re) = te.rec_at(j) {
        for i in 0..ex.len() {
            let r = &te.rays[i];
            let before =
                r.status.surf().is_some_and(|k| k < j) || (r.vig != 0 && (r.vig as usize) < j);
            let p = re.xyz[i];
            fp.edge.push(if before || !p[0].is_finite() {
                [f64::NAN; 2]
            } else {
                [p[0], p[1]]
            });
        }
    }
    Ok(fp)
}

/// けられずに像面まで通る光線の割合（瞳の格子 n×n。OpticStudio の Vignetting Plot の fraction of unvignetted rays）。
/// vig なら系のケラレ係数で縮めた瞳の光線の割合、でなければ面の開口だけでけられる割合
pub fn unvignetted(s: &System, field: FieldSel, w: usize, n: usize, vig: bool) -> Result<f64> {
    let (px, py) = lensbench_core::opd::pupil_grid(n.max(2));
    let t = trace(s, field, w, &px, &py, vig, RecSel::Off)?;
    let ok = (0..px.len()).filter(|&i| t.ok(i)).count();
    Ok(ok as f64 / px.len() as f64)
}
