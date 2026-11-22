//! 最適化の引数（変数・既定の評価関数）を設定の型（`lensbench_design::dls_spec`）にする

use std::path::PathBuf;
use std::sync::Arc;

use lensbench_core::System;
use lensbench_optim as optim;
use pyo3::prelude::*;

use crate::{err, userterm};

/// An optimization variable: "7.t" or ("7.t", lower, upper)
#[derive(FromPyObject, Clone)]
pub(crate) enum VarArg {
    Name(String),
    Range((String, Option<f64>, Option<f64>)),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn opt_problem(
    py: Python<'_>,
    s: &System,
    vars: &[VarArg],
    merit: Option<&str>,
    reference: &str,
    rings: usize,
    jac: &str,
    terms: Option<&Bound<'_, PyAny>>,
    coat_files: &[PathBuf],
) -> PyResult<(optim::Problem, Vec<Arc<userterm::PyUserTerm>>, usize)> {
    use crate::search::pick;
    use lensbench_design::dls_spec::{Merit, OptimizeParams};
    let op = OptimizeParams {
        vars: vars.iter().map(VarArg::spec).collect(),
        merit: match merit {
            None => Merit::None,
            Some(m) => pick("merit", m)?,
        },
        reference: pick("reference", reference)?,
        rings,
        jac: pick("jac", jac)?,
        ..OptimizeParams::default()
    };
    let (mut p, n_default) = op.problem(vec![s.clone()]).map_err(err)?;
    let users = match terms {
        Some(t) => userterm::add_terms(py, &mut p, t, coat_files)?,
        None => Vec::new(),
    };
    p.check().map_err(err)?;
    Ok((p, users, n_default))
}

impl VarArg {
    /// 設定の型の変数
    pub(crate) fn spec(&self) -> lensbench_design::dls_spec::VarSpec {
        use lensbench_design::dls_spec::VarSpec;
        match self {
            VarArg::Name(n) => VarSpec::Full {
                name: n.clone(),
                lo: None,
                hi: None,
                configs: None,
            },
            VarArg::Range((n, lo, hi)) => VarSpec::Full {
                name: n.clone(),
                lo: *lo,
                hi: *hi,
                configs: None,
            },
        }
    }
}

/// optimize の glass（dict）を設定の型に
pub(crate) fn glass_arg(
    g: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<lensbench_design::dls_spec::GlassParams>> {
    let Some(g) = g else {
        return Ok(None);
    };
    serde_json::from_value(crate::pyjson::from_py(g)?)
        .map(Some)
        .map_err(|e| err(lensbench_core::Error::Invalid(format!("glass: {e}"))))
}

/// optimize の返り値の dict（"system" の他）: 変数・評価関数・項の値と、回したときの global_search・glass
pub(crate) fn result_dict<'py>(
    py: Python<'py>,
    s: &System,
    p: &optim::Problem,
    run: &lensbench_design::dls_spec::OptimizeRun,
    n_default: usize,
    users: &[Arc<userterm::PyUserTerm>],
    threads: Option<usize>,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    use lensbench_engine::session::views::{GlassView, GlobalView};
    use pyo3::types::PyDictMethods;
    let r = &run.result;
    let pair = |t: &optim::TermValue| (t.value(), t.kind_name());
    let tv = |q: &optim::Problem, x: &[f64]| -> PyResult<(Vec<f64>, Vec<&str>)> {
        let v = py
            .detach(|| lensbench_core::with_threads(threads, || q.term_values(x)))
            .map_err(err);
        userterm::raise_user_errors(users)?;
        Ok(v?[n_default..].iter().map(pair).unzip())
    };
    let ((tv0, _), (tv1, kinds)) = (tv(p, &r.x0)?, tv(&run.problem, &r.x)?);
    let d = pyo3::types::PyDict::new(py);
    let names: Vec<String> = p.vars.iter().map(|v| s.param_name(v.param)).collect();
    // 外した dn・disp の変数（ガラスの置き換え）は inactive に数えない
    let inactive: Vec<usize> = (0..p.vars.len())
        .filter(|&q| {
            run.problem
                .vars
                .iter()
                .position(|u| *u == p.vars[q])
                .is_some_and(|i| r.inactive.contains(&i))
        })
        .collect();
    d.set_item("names", names)?;
    d.set_item("x0", r.x0.clone())?;
    d.set_item("x", run.x.clone())?;
    d.set_item("merit0", r.merit0)?;
    d.set_item("merit", r.merit)?;
    d.set_item("history", r.history.clone())?;
    d.set_item("iterations", r.iterations)?;
    d.set_item("evals", r.evals)?;
    d.set_item("stop", r.stop.as_str())?;
    d.set_item("note", r.note.clone())?;
    d.set_item("inactive", inactive)?;
    d.set_item("term_values0", tv0)?;
    d.set_item("term_values", tv1)?;
    d.set_item("term_value_kinds", kinds)?;
    if let Some(g) = &run.global {
        let v = serde_json::to_value(GlobalView::from_result(g)).unwrap_or_default();
        d.set_item("global_search", crate::pyjson::to_py(py, &v)?)?;
    }
    if let Some(g) = &run.glass {
        let v = serde_json::to_value(GlassView::from_run(g)).unwrap_or_default();
        d.set_item("glass", crate::pyjson::to_py(py, &v)?)?;
    }
    Ok(d)
}
