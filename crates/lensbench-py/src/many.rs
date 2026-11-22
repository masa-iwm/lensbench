//! 多数の系をまとめて追う（`trace_many`）

use super::*;

/// Trace many systems together in one call. Returns a list of dicts, one per system,
/// each bit for bit the same as `systems[i].trace(field, px, py, ...)` (for any number of threads).
///
/// The arguments are as in System.trace, the same for all systems (grad parameter names are resolved per system).
/// Parallel over both systems and rays, so all cores are used even with few rays per system.
/// threads is the total number of threads (1 for sequential)
#[pyfunction]
#[pyo3(signature = (systems, field, px, py, waves=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, object=None, grad=None, threads=None, pupil="normalized", dz=None, land=false))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn trace_many<'py>(
    gil: Python<'py>,
    systems: Vec<Py<PySystem>>,
    field: Option<FieldArg>,
    px: PyArrayLike1<'py, f64, AllowTypeChange>,
    py: PyArrayLike1<'py, f64, AllowTypeChange>,
    waves: Option<Vec<usize>>,
    last: Option<usize>,
    record: RecArg,
    pol: Option<PolArg>,
    pol_phase: bool,
    globals: bool,
    object: Option<ObjArg>,
    grad: Option<Vec<String>>,
    threads: Option<usize>,
    pupil: &str,
    dz: Option<f64>,
    land: bool,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let (px, pyv) = (floats(&px), floats(&py));
    let n = px.len();
    // 系ごとの準備（引数の検査・膜の読み込みは GIL のあるうちに）
    let mut jobs = Vec::with_capacity(systems.len());
    for p in &systems {
        let me = p.get();
        let s = me.sys();
        let pre = Prep::new(&s, field.clone(), object.clone(), pupil, n, pyv.len())?;
        let o = TraceOpts::new(
            &s,
            waves.clone(),
            last,
            record.clone(),
            pol.clone(),
            pol_phase,
            globals,
            grad.clone(),
            threads,
            dz,
            land,
        )?;
        let coat = match o.pw {
            Some(_) => Some(me.coatings(&s)?),
            None => None,
        };
        jobs.push((s, pre, o, coat));
    }
    let outs = gil.detach(|| {
        with_threads(threads, || {
            lensbench_core::par::map(&jobs, |(s, pre, o, coat)| {
                compute(s, coat.as_deref(), &pre.launch(&px, &pyv), n, o)
            })
        })
    });
    let mut res = Vec::with_capacity(jobs.len());
    for ((s, pre, o, _), out) in jobs.iter().zip(outs) {
        let d = to_dict(gil, s, o, out.map_err(err)?)?;
        pre.finish(&d)?;
        res.push(d);
    }
    Ok(res)
}
