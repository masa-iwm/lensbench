//! MTF の結果の dict と、設定の文字列の読み取り（lib.rs から分けた）

use lensbench_core::{analysis, sweep};
use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyDict};

use crate::errors::value_err;
use crate::interference;

pub(crate) fn field_axis(a: &str) -> PyResult<sweep::FieldAxis> {
    match a {
        "x" => Ok(sweep::FieldAxis::X),
        "y" => Ok(sweep::FieldAxis::Y),
        _ => Err(value_err(format!("axis must be \"x\" or \"y\" ({a})"))),
    }
}

/// MTF の求め方（"auto"・"interp"・"fft"・"direct"）
pub(crate) fn method(m: &str) -> PyResult<analysis::MtfMethod> {
    match m {
        "auto" => Ok(analysis::MtfMethod::Auto),
        "interp" => Ok(analysis::MtfMethod::Interp),
        "fft" => Ok(analysis::MtfMethod::Fft),
        "direct" => Ok(analysis::MtfMethod::Direct),
        m => Err(value_err(format!(
            "method must be \"auto\", \"interp\", \"fft\" or \"direct\" ({m})"
        ))),
    }
}

/// 子午・球欠の向き（"field"・"pupil"、[`analysis::MtfAxes`]）
pub(crate) fn axes(a: &str) -> PyResult<analysis::MtfAxes> {
    match a {
        "field" => Ok(analysis::MtfAxes::Field),
        "pupil" => Ok(analysis::MtfAxes::Pupil),
        a => Err(value_err(format!(
            "axes must be \"field\" or \"pupil\" ({a})"
        ))),
    }
}

pub(crate) fn mtf_dict<'py>(py: Python<'py>, m: &analysis::Mtf) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("freqs", m.freqs.clone().into_pyarray(py))?;
    d.set_item("tangential", m.tan.clone().into_pyarray(py))?;
    d.set_item("sagittal", m.sag.clone().into_pyarray(py))?;
    let c = [("tangential", m.cutoff.tan), ("sagittal", m.cutoff.sag)];
    d.set_item("cutoff", c.into_py_dict(py)?)?;
    Ok(d)
}

pub(crate) fn sweep_dict<'py>(
    py: Python<'py>,
    xname: &str,
    m: &sweep::MtfSweep,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("freqs", m.freqs().to_vec().into_pyarray(py))?;
    d.set_item(xname, m.x().to_vec().into_pyarray(py))?;
    let g = |g: &lensbench_core::grid::Grid2| interference::grid_array(g.clone()).into_pyarray(py);
    d.set_item("tangential", g(&m.tan))?;
    d.set_item("sagittal", g(&m.sag))?;
    d.set_item("warnings", m.warnings.clone())?;
    Ok(d)
}
