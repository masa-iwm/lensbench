//! 機構部品のケラレ（`System.mech_check`、CLI の `mech-check` と同じ）

use lensbench_core::System;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::{err, index_err, value_err};
use crate::wave_index;

/// 機構部品のケラレの dict（[`crate::PySystem`] の `mech_check`）
pub(crate) fn mech_check<'py>(
    py: Python<'py>,
    s: &System,
    fields: Option<Vec<usize>>,
    waves: Option<Vec<usize>>,
    grid: usize,
) -> PyResult<Bound<'py, PyDict>> {
    if s.mechanics.is_empty() {
        return Err(value_err(
            "no mechanical parts (write them in the project mechanics)",
        ));
    }
    if grid < 2 {
        return Err(value_err("grid must be 2 or more"));
    }
    let nf = s.fields.len();
    let fs: Vec<usize> = match fields {
        Some(v) => v
            .into_iter()
            .map(|f| {
                if f == 0 || f > nf {
                    Err(index_err(format!("no field {f} (1 to {nf})")))
                } else {
                    Ok(f - 1)
                }
            })
            .collect::<PyResult<_>>()?,
        None => (0..nf).collect(),
    };
    let ws: Vec<usize> = match waves {
        Some(v) => v
            .into_iter()
            .map(|w| wave_index(s, w))
            .collect::<PyResult<_>>()?,
        None => (0..s.wavelengths.len()).collect(),
    };
    let (px, py_) = lensbench_core::mech::pupil_grid(grid);
    let v = py
        .detach(|| lensbench_core::mech::vignetting(s, &s.mechanics, &fs, &ws, &px, &py_))
        .map_err(err)?;
    let pct = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    let d = PyDict::new(py);
    d.set_item("pupil_rays", px.len())?;
    d.set_item("mech_clip", s.mech_clip)?;
    let parts = pyo3::types::PyList::empty(py);
    for (k, p) in s.mechanics.iter().enumerate() {
        let e = PyDict::new(py);
        e.set_item("name", &p.name)?;
        e.set_item("surface", p.surface)?;
        e.set_item(
            "clipped_max_pct",
            v.iter()
                .map(|r| pct(r.clipped[k], r.launched))
                .fold(0.0, f64::max),
        )?;
        e.set_item(
            "first_max_pct",
            v.iter()
                .map(|r| pct(r.first[k], r.launched))
                .fold(0.0, f64::max),
        )?;
        let bores = pyo3::types::PyList::empty(py);
        for b in &p.bores {
            let x = PyDict::new(py);
            x.set_item("z0", b.z0)?;
            x.set_item("z1", b.z1)?;
            x.set_item("diameter", 2.0 * b.radius)?;
            bores.append(x)?;
        }
        e.set_item("bores", bores)?;
        parts.append(e)?;
    }
    d.set_item("parts", parts)?;
    let rows = pyo3::types::PyList::empty(py);
    for r in &v {
        let e = PyDict::new(py);
        e.set_item("field", r.field + 1)?;
        e.set_item("wave", r.wave + 1)?;
        e.set_item("rays", r.launched)?;
        e.set_item("failed", r.failed)?;
        e.set_item("failed_pct", pct(r.failed, r.launched))?;
        e.set_item("optical_pct", pct(r.optical, r.launched))?;
        e.set_item("passed_pct", pct(r.passed, r.launched))?;
        let ps = pyo3::types::PyList::empty(py);
        for (k, p) in s.mechanics.iter().enumerate() {
            let x = PyDict::new(py);
            x.set_item("name", &p.name)?;
            x.set_item("clipped_pct", pct(r.clipped[k], r.launched))?;
            x.set_item("first_pct", pct(r.first[k], r.launched))?;
            ps.append(x)?;
        }
        e.set_item("parts", ps)?;
        rows.append(e)?;
    }
    d.set_item("results", rows)?;
    let fit = pyo3::types::PyList::empty(py);
    for f in lensbench_core::mech::fit_check(s, &s.mechanics).map_err(err)? {
        let e = PyDict::new(py);
        e.set_item("kind", f.kind)?;
        e.set_item("error", f.error)?;
        e.set_item("part", &s.mechanics[f.part].name)?;
        e.set_item("other", f.other.map(|o| s.mechanics[o].name.clone()))?;
        e.set_item("surface", f.surface)?;
        e.set_item("value", f.value)?;
        e.set_item("message", &f.message)?;
        fit.append(e)?;
    }
    d.set_item("fit", fit)?;
    Ok(d)
}
