//! 利用者定義の光源（`lensbench_core::usersource`。拡張が足したもの）を Python から名前で使う口。
//!
//! - `lb.source_models()` → {名前: 説明}
//! - `lb.source_rays(system, name, config=None, wave=None, frame="object")` → 始点・方向・重みの配列の dict。
//!   start・direction をそのまま `system.trace_rays(start, direction, waves=[wave], frame=frame)` に渡せる

use numpy::ndarray::Array2;
use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::value_err;
use crate::plugin::to_config;
use crate::{err, wave_index, PySystem};

/// Registered user-defined source models: {name: description}
#[pyfunction]
fn source_models() -> std::collections::BTreeMap<String, String> {
    // source_rays と同じ表（今の文脈、無ければプロセスの既定）
    let e = lensbench_core::ext::current();
    e.source_models()
        .into_iter()
        .filter_map(|n| {
            let doc = e.source_model(&n).ok()?.doc();
            Some((n, doc))
        })
        .collect()
}

/// Rays of a registered user-defined source model name with the settings config (dict) at the wavelength number wave (primary wavelength if omitted).
/// frame is "object" (the object coordinate system, as trace_rays) or "global".
/// Returned dict: start and direction ((rays, 3); directions as the source gave them) and weight ((rays,)).
/// Pass start and direction to `system.trace_rays(start, direction, waves=[wave], frame=frame)`
#[pyfunction]
#[pyo3(signature = (system, name, config=None, wave=None, frame="object"))]
fn source_rays<'py>(
    py: Python<'py>,
    system: PyRef<'py, PySystem>,
    name: &str,
    config: Option<&Bound<'py, PyDict>>,
    wave: Option<usize>,
    frame: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let s = system.sys();
    let w = wave
        .map(|w| wave_index(&s, w))
        .transpose()?
        .unwrap_or(s.primary);
    let global = match frame.to_ascii_lowercase().as_str() {
        "object" => false,
        "global" => true,
        f => {
            return Err(value_err(format!(
                "frame must be \"object\" or \"global\" ({f})"
            )))
        }
    };
    let cfg = to_config(config)?;
    let r = py
        .detach(|| -> lensbench_core::Result<_> {
            let m = lensbench_core::ext::current().source_model(name)?;
            let inst = m.prepare(&cfg)?;
            if global {
                s.source_rays(inst.as_ref(), w)
            } else {
                let r = inst.rays(s.wavelengths[w])?;
                r.check()?;
                Ok(r)
            }
        })
        .map_err(err)?;
    let rows =
        |v: &[[f64; 3]]| Array2::from_shape_fn((v.len(), 3), |(i, c)| v[i][c]).into_pyarray(py);
    let d = PyDict::new(py);
    d.set_item("start", rows(&r.p))?;
    d.set_item("direction", rows(&r.k))?;
    d.set_item("weight", r.w.into_pyarray(py))?;
    Ok(d)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(source_models, m)?)?;
    m.add_function(wrap_pyfunction!(source_rays, m)?)?;
    Ok(())
}
