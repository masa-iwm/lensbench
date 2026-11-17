//! 項の評価に使う追跡（止まった光線の扱い、列に書く追跡）

use super::PROBE;
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::math::*;
use lensbench_core::system::{Cols, FieldSel, RayBuf, System, Trace};

/// 物体の座標系での開始の光線（位置・方向を並べたもの）
pub(super) fn launch_of(s: &System, field: FieldSel, w: usize) -> Result<Vec<f64>> {
    let px: Vec<f64> = PROBE.iter().map(|p| p.0).collect();
    let py: Vec<f64> = PROBE.iter().map(|p| p.1).collect();
    let t = s.try_trace(field, &px, &py, Some(w), Some(1.min(s.image())), false)?;
    let f = s.object_frame();
    let rt = transpose(&f.r);
    let mut v = Vec::new();
    for i in 0..t.p0.len() {
        v.extend(mv(&rt, sub(t.p0[i], f.o)));
        v.extend(mv(&rt, t.k0[i]));
    }
    Ok(v)
}

/// 追跡（止まった光線があればエラー）
pub(super) fn trace_ok(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: usize,
    record: bool,
) -> Result<Trace> {
    let t = s.try_trace(field, px, py, Some(w), None, record)?;
    if let Some(i) = t.rays.iter().position(|r| r.status.surf().is_some()) {
        invalid!(
            "the ray stopped (wavelength {w}, pupil ({:.3}, {:.3}), surface {})",
            px[i],
            py[i],
            t.rays[i].status.code()
        );
    }
    Ok(t)
}

/// 瞳の光線の追跡。keep なら止まった光線も残す（[`Trace::ok`] が偽なので、けられた光線と同じく除かれる）
pub(super) fn trace_pupil(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: usize,
    record: bool,
    keep: bool,
) -> Result<Trace> {
    if keep {
        s.try_trace(field, px, py, Some(w), None, record)
    } else {
        trace_ok(s, field, px, py, w, record)
    }
}

/// 瞳の光線の像面の量（列 cols）だけを追跡する（微分の要らない項の評価）。keep でなければ、止まった光線で
/// エラー（[`trace_ok`] と同じ文）。値は [`trace_pupil`] の同じ量・[`Trace::ok`] と同じビット
pub(super) fn trace_pupil_cols(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: usize,
    keep: bool,
    cols: Cols,
) -> Result<RayBuf> {
    let mut b = RayBuf::with_cols(cols);
    s.try_trace_into(field, px, py, Some(w), None, &mut b)?;
    if !keep {
        if let Some(i) = b.status.iter().position(|r| r.surf().is_some()) {
            invalid!(
                "the ray stopped (wavelength {w}, pupil ({:.3}, {:.3}), surface {})",
                px[i],
                py[i],
                b.status[i].code()
            );
        }
    }
    Ok(b)
}

/// [`trace_pupil`] の記録つきの追跡と、その像面の量（列 cols）の写し（微分の要る項。値の後段は列で読む）
#[allow(clippy::too_many_arguments)]
pub(super) fn trace_pupil_rec(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: usize,
    keep: bool,
    cols: Cols,
) -> Result<(Trace, RayBuf)> {
    let t = trace_pupil(s, field, px, py, w, true, keep)?;
    let mut b = RayBuf::with_cols(cols);
    b.fill_from(&t);
    Ok((t, b))
}
