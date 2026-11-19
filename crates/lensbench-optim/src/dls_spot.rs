//! スポットの項（RMS スポット半径）と光束の重心の項の値と解析的な微分（dls.rs から分けた）

use super::trace::{trace_ok, trace_pupil, trace_pupil_cols};
use super::{waves, Quad, SpotAxis, TermOut};
use crate::pupil::apodized_gauss;
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::param::Param;
use lensbench_core::system::{Cols, FieldSel, RayBuf, System};

/// 像の点を求める光線の列（アフォーカル像空間は向きから角度）
fn img_cols(s: &System) -> Cols {
    if s.afocal {
        Cols::XYZ | Cols::LMN
    } else {
        Cols::XYZ
    }
}

/// 光線 i の像の点（[`System::image_point`]: アフォーカル像空間は角度）
fn img_pt(s: &System, b: &RayBuf, i: usize) -> [f64; 2] {
    if s.afocal {
        let (x, y) = s.image_point(b.xyz[i], b.lmn[i]);
        [x, y]
    } else {
        [b.xyz[i][0], b.xyz[i][1]]
    }
}

/// 像の点の微分。g は land_grad の 1 つのパラメータの (dx, dy, dl, dm)、lmn は光線の向き
fn img_d(s: &System, lmn: [f64; 3], g: &[f64]) -> [f64; 2] {
    if !s.afocal {
        return [g[0], g[1]];
    }
    let u = s.afocal_unit.radians();
    let [l, m, n] = lmn;
    let dn = -(l * g[2] + m * g[3]) / n;
    // d atan(l / n) = (n dl − l dn) / (l² + n²)
    [
        (n * g[2] - l * dn) / (l * l + n * n) / u,
        (n * g[3] - m * dn) / (m * m + n * n) / u,
    ]
}

pub(super) fn spot_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    (rings, quad, axis): (usize, Quad, SpotAxis),
    reference: RmsRef,
    ps: Option<&[Param]>,
    keep: bool,
) -> Result<TermOut> {
    let (pg, wts) = apodized_gauss(s, field, rings, quad, false)?;
    // 測らない向きの残差・微分は 0 に
    let (fx, fy) = (
        f64::from(axis != SpotAxis::Y),
        f64::from(axis != SpotAxis::X),
    );
    let (px, py, wts) = (&pg.0, &pg.1, &wts);
    let n = px.len();
    let ws = waves(s, wave);
    let rec = ps.is_some();
    let np = ps.map_or(0, |p| p.len());
    // 像面の位置とけられ（微分が要るときだけ記録つきの Trace から写す）
    let mut traces: Vec<RayBuf> = Vec::with_capacity(ws.len());
    let mut grads = Vec::with_capacity(ws.len());
    for &w in &ws {
        if let Some(ps) = ps {
            let t = trace_pupil(s, field, px, py, w, rec, keep)?;
            grads.push(s.land_grad(&t, w, ps, None)?);
            let mut b = RayBuf::with_cols(img_cols(s));
            b.fill_from(&t);
            traces.push(b);
        } else {
            traces.push(trace_pupil_cols(s, field, px, py, w, keep, img_cols(s))?);
        }
    }
    // 重み（spot() と同じ: 単色は波長の重みによらない）
    let mut wt = vec![0.0; ws.len() * n];
    for (k, &w) in ws.iter().enumerate() {
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[w] };
        for i in 0..n {
            if traces[k].ok(i) {
                wt[k * n + i] = wts[i] * kw;
            }
        }
    }
    let sw: f64 = wt.iter().sum();
    if !(sw > 0.0) {
        invalid!("no valid rays (all vignetted)");
    }
    let xy = |k: usize, i: usize| {
        let [x, y] = img_pt(s, &traces[k], i);
        (x, y)
    };
    // 光線 i・パラメータ q の像の点の微分
    let dxy = |k: usize, i: usize, q: usize| {
        img_d(
            s,
            traces[k].lmn.get(i).copied().unwrap_or([0.0; 3]),
            &grads[k][(i * np + q) * 4..],
        )
    };
    // 基準とその微分
    let (cx, cy, dc): (f64, f64, Vec<[f64; 2]>) = match reference {
        RmsRef::Centroid => {
            let (mut cx, mut cy) = (0.0, 0.0);
            let mut dc = vec![[0.0; 2]; np];
            for k in 0..ws.len() {
                for i in 0..n {
                    let a = wt[k * n + i];
                    if a == 0.0 {
                        continue;
                    }
                    let (x, y) = xy(k, i);
                    cx += a * x;
                    cy += a * y;
                    for (q, d) in dc.iter_mut().enumerate() {
                        let g = dxy(k, i, q);
                        d[0] += a * g[0];
                        d[1] += a * g[1];
                    }
                }
            }
            dc.iter_mut().for_each(|d| {
                d[0] /= sw;
                d[1] /= sw;
            });
            (cx / sw, cy / sw, dc)
        }
        RmsRef::Chief => {
            let wc = wave.unwrap_or(s.primary);
            let t = trace_ok(s, field, &[0.0], &[0.0], wc, rec)?;
            if !t.traced(0) {
                invalid!("the chief ray doesn't reach the image");
            }
            let mut dc = vec![[0.0; 2]; np];
            let c = &t.rays[0];
            if let Some(ps) = ps {
                let g = s.land_grad(&t, wc, ps, None)?;
                for (q, d) in dc.iter_mut().enumerate() {
                    *d = img_d(s, c.lmn, &g[q * 4..]);
                }
            }
            let (x, y) = s.image_point(c.xyz, c.lmn);
            (x, y, dc)
        }
    };
    let m = 2 * ws.len() * n;
    let mut r = vec![0.0; m];
    let mut j = ps.map(|_| vec![0.0; m * np]);
    for k in 0..ws.len() {
        for i in 0..n {
            let a = wt[k * n + i];
            if a == 0.0 {
                continue;
            }
            let a = (a / sw).sqrt();
            let (x, y) = xy(k, i);
            let row = 2 * (k * n + i);
            r[row] = fx * a * (x - cx);
            r[row + 1] = fy * a * (y - cy);
            if let Some(j) = j.as_mut() {
                for q in 0..np {
                    let g = dxy(k, i, q);
                    j[row * np + q] = fx * a * (g[0] - dc[q][0]);
                    j[(row + 1) * np + q] = fy * a * (g[1] - dc[q][1]);
                }
            }
        }
    }
    Ok((r, j))
}

/// 像面の光束の重心（スポットの項と同じ光線・重み）の x か y − target と、その解析的な微分
#[allow(clippy::too_many_arguments)]
pub(super) fn centroid_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    rings: usize,
    y: bool,
    target: f64,
    ps: Option<&[Param]>,
    keep: bool,
) -> Result<TermOut> {
    let (pg, wts) = apodized_gauss(s, field, rings, Quad::Lensbench, false)?;
    let (px, py, wts) = (&pg.0, &pg.1, &wts);
    let n = px.len();
    let ws = waves(s, wave);
    let np = ps.map_or(0, |p| p.len());
    let c = usize::from(y);
    let (mut sw, mut sc) = (0.0, 0.0);
    let mut dc = vec![0.0; np];
    for &w in &ws {
        let (t, g) = match ps {
            Some(ps) => {
                let tr = trace_pupil(s, field, px, py, w, true, keep)?;
                let g = s.land_grad(&tr, w, ps, None)?;
                let mut b = RayBuf::with_cols(img_cols(s));
                b.fill_from(&tr);
                (b, Some(g))
            }
            None => (
                trace_pupil_cols(s, field, px, py, w, keep, img_cols(s))?,
                None,
            ),
        };
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[w] };
        for i in 0..n {
            if !t.ok(i) {
                continue;
            }
            let a = wts[i] * kw;
            sw += a;
            sc += a * img_pt(s, &t, i)[c];
            if let Some(g) = &g {
                let lmn = t.lmn.get(i).copied().unwrap_or([0.0; 3]);
                for (q, d) in dc.iter_mut().enumerate() {
                    *d += a * img_d(s, lmn, &g[(i * np + q) * 4..])[c];
                }
            }
        }
    }
    if !(sw > 0.0) {
        invalid!("no valid rays (all vignetted)");
    }
    let j = ps.map(|_| dc.iter().map(|d| d / sw).collect());
    Ok((vec![sc / sw - target], j))
}
