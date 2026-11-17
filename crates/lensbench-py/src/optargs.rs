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
