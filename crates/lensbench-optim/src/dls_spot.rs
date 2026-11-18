//! スポットの項（RMS スポット半径）と光束の重心の項の値と解析的な微分（dls.rs から分けた）

use super::trace::{trace_ok, trace_pupil, trace_pupil_cols};
use super::{waves, Quad, SpotAxis, TermOut};
use crate::pupil::apodized_gauss;
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::param::Param;
use lensbench_core::sweep::RmsRef;
use lensbench_core::system::{Cols, FieldSel, RayBuf, System};

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
            let mut b = RayBuf::with_cols(Cols::XYZ);
            b.fill_from(&t);
            traces.push(b);
        } else {
            traces.push(trace_pupil_cols(s, field, px, py, w, keep, Cols::XYZ)?);
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
    let xy = |k: usize, i: usize| (traces[k].xyz[i][0], traces[k].xyz[i][1]);
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
                        let g = &grads[k][(i * np + q) * 4..];
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
            if let Some(ps) = ps {
                let g = s.land_grad(&t, wc, ps, None)?;
                for (q, d) in dc.iter_mut().enumerate() {
                    *d = [g[q * 4], g[q * 4 + 1]];
                }
            }
            (t.rays[0].xyz[0], t.rays[0].xyz[1], dc)
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
                    let g = &grads[k][(i * np + q) * 4..];
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
                let mut b = RayBuf::with_cols(Cols::XYZ);
                b.fill_from(&tr);
                (b, Some(g))
            }
            None => (
                trace_pupil_cols(s, field, px, py, w, keep, Cols::XYZ)?,
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
            sc += a * t.xyz[i][c];
            if let Some(g) = &g {
                for (q, d) in dc.iter_mut().enumerate() {
                    *d += a * g[(i * np + q) * 4 + c];
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
