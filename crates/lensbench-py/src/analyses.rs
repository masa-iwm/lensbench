//! The analysis registry from Python: `lensbench.analyses()` and `lensbench.defaults()` (the settings come from the one
//! definition of each analysis, the same as the CLI subcommands, serve's analyze and the GUI, XCT-02).
//! Running one is `System.analyze(name, **settings)`.

use pyo3::prelude::*;
use pyo3::types::PyList;
use serde_json::{json, Value};

use crate::errors::value_err;
use crate::pyjson;

/// The analyses of the registry: a list of dicts {name, about, help, params}. params lists the settings
/// (name, type, default, unit, doc, and choices, min, len where they apply), the same as the keyword arguments of System.analyze
#[pyfunction]
fn analyses(py: Python<'_>) -> PyResult<Bound<'_, PyList>> {
    let l = PyList::empty(py);
    for a in lensbench_engine::analyses().iter() {
        let m = a.meta();
        let ps: Vec<Value> = a.params().iter().map(|p| p.to_json()).collect();
        l.append(pyjson::to_py(
            py,
            &json!({"name": m.name, "about": m.about, "help": m.help, "params": ps}),
        )?)?;
    }
    Ok(l)
}

/// The default settings of an analysis of the registry, or of "optimize" or "zoom", as a dict
/// (the defaults of System.optimize, zoom and the analysis methods come from here)
#[pyfunction]
fn defaults<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    use lensbench_design::dls_spec::{OptimizeParams, ZoomParams};
    let v = match name {
        "optimize" => serde_json::to_value(OptimizeParams::default()),
        "zoom" => serde_json::to_value(ZoomParams::default()),
        _ => {
            let reg = lensbench_engine::analyses();
            let a = reg.get(name).ok_or_else(|| {
                value_err(format!(
                    "no analysis {name} (lensbench.analyses() lists the names)"
                ))
            })?;
            let o: serde_json::Map<String, Value> = a
                .params()
                .into_iter()
                .map(|p| (p.name, p.default))
                .collect();
            Ok(Value::Object(o))
        }
    }
    .map_err(|e| value_err(e.to_string()))?;
    pyjson::to_py(py, &v)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analyses, m)?)?;
    m.add_function(wrap_pyfunction!(defaults, m)?)?;
    Ok(())
}
