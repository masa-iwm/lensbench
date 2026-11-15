//! 一括実行を Python から: 抽選 `Sampler`・掃引 `Grid` と `run_batch`。
//!
//! 各組で Python の関数 func(i, system, seed) を呼び、返した値（JSON にできるもの）を JSONL に逐次書く。
//! 並列はスレッドで（プロセスは使わない）。func の中の lensbench の追跡・解析は GIL を外すので、その間はほかの
//! スレッドの func が進む。func が Python の計算ばかりなら並列にはならない（重い所は Rust 側へ の方針）。

use crate::errors::value_err;
use lensbench_core::LockExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use lensbench_core::param::Param;
use lensbench_core::System;
use lensbench_runs::{Dist, Grid, Plan, Progress, RunOpts, Sampler};
use pyo3::exceptions::PyKeyboardInterrupt;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::{err, PySystem};

/// A distribution: ("normal", σ[, mean]), ("uniform", lo, hi), ("trunc", σ, cut[, mean]), ("fixed", value); a number alone is the σ of a normal distribution
fn dist_of(v: &Bound<'_, PyAny>) -> PyResult<Dist> {
    if let Ok(s) = v.extract::<f64>() {
        return Ok(Dist::Normal {
            mean: 0.0,
            sigma: s,
        });
    }
    let t = v.cast::<PyTuple>().map_err(|_| {
        value_err("a distribution is a number σ or a tuple such as (\"normal\", σ)")
    })?;
    let kind: String = t.get_item(0)?.extract()?;
    let x = |k: usize| -> PyResult<f64> { t.get_item(k)?.extract() };
    let opt = |k: usize| -> PyResult<f64> {
        if t.len() > k {
            x(k)
        } else {
            Ok(0.0)
        }
    };
    Ok(match kind.to_ascii_lowercase().as_str() {
        "normal" | "gauss" => Dist::Normal {
            sigma: x(1)?,
            mean: opt(2)?,
        },
        "uniform" => Dist::Uniform {
            lo: x(1)?,
            hi: x(2)?,
        },
        "trunc" | "truncnormal" => Dist::TruncNormal {
            sigma: x(1)?,
            cut: x(2)?,
            mean: opt(3)?,
        },
        "fixed" => Dist::Fixed(x(1)?),
        k => {
            return Err(value_err(format!(
                "distribution {k} (normal uniform trunc fixed)"
            )))
        }
    })
}

fn dist_json(d: &Dist) -> serde_json::Value {
    use serde_json::json;
    match *d {
        Dist::Normal { mean, sigma } => json!({"normal": {"mean": mean, "sigma": sigma}}),
        Dist::Uniform { lo, hi } => json!({"uniform": {"lo": lo, "hi": hi}}),
        Dist::TruncNormal { mean, sigma, cut } => {
            json!({"trunc": {"mean": mean, "sigma": sigma, "cut": cut}})
        }
        Dist::Fixed(v) => json!({ "fixed": v }),
    }
}

/// {name: value} or [(name, value)] into a list (order kept)
fn pairs<'py>(v: &Bound<'py, PyAny>) -> PyResult<Vec<(String, Bound<'py, PyAny>)>> {
    if let Ok(d) = v.cast::<PyDict>() {
        return d.iter().map(|(k, x)| Ok((k.extract()?, x))).collect();
    }
    let l = v.cast::<PyList>()?;
    l.iter()
        .map(|t| {
            let t = t.cast::<PyTuple>()?;
            Ok((t.get_item(0)?.extract()?, t.get_item(1)?))
        })
        .collect()
}

enum PlanKind {
    Sampler(Sampler),
    Grid(Grid),
}

impl PlanKind {
    fn plan(&self) -> &dyn Plan {
        match self {
            PlanKind::Sampler(s) => s,
            PlanKind::Grid(g) => g,
        }
    }
}

/// How the sets are made, and their nominal system
struct PlanBase {
    plan: PlanKind,
    names: Vec<String>,
    base: Arc<System>,
    coat_files: Vec<PathBuf>,
    describe: serde_json::Value,
}

impl PlanBase {
    fn changes(&self, i: usize) -> PyResult<Vec<(Param, f64)>> {
        self.plan.plan().changes(i, &self.base).map_err(err)
    }
    fn draw_dict<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (n, (_, v)) in self.names.iter().zip(self.changes(i)?) {
            d.set_item(n, v)?;
        }
        Ok(d)
    }
}

/// Sampling (Monte Carlo): adds perturbations from a distribution per parameter to the nominal values of the system.
///
/// - params: {"3.t": 0.02, "5.dx": ("uniform", -0.01, 0.01), "G.tx": ("trunc", 0.05, 2.0)} (a number alone is the σ of a normal distribution)
/// - corr: correlation matrix (in the order of params). The marginal distributions stay as they are; correlation comes from a Gaussian copula
/// - seed: random seed. The i-th set depends only on (seed, i) (not on the number of threads, resuming or order)
#[pyclass(module = "lensbench", name = "Sampler", frozen)]
pub(crate) struct PySampler(PlanBase);

#[pymethods]
impl PySampler {
    #[new]
    #[pyo3(signature = (system, params, corr=None, seed=0))]
    fn new(
        system: &PySystem,
        params: &Bound<'_, PyAny>,
        corr: Option<Vec<Vec<f64>>>,
        seed: u64,
    ) -> PyResult<Self> {
        let base = system.sys();
        let mut vars = Vec::new();
        let mut names = Vec::new();
        let mut desc = Vec::new();
        for (n, v) in pairs(params)? {
            let p = base.param(&n).map_err(err)?;
            base.get_param(p).map_err(err)?;
            let d = dist_of(&v)?;
            desc.push(serde_json::json!([base.param_name(p), dist_json(&d)]));
            names.push(n);
            vars.push((p, d));
        }
        let s = Sampler::new(vars, corr.clone(), seed).map_err(err)?;
        Ok(PySampler(PlanBase {
            plan: PlanKind::Sampler(s),
            names,
            base,
            coat_files: system.coat_files.clone(),
            describe: serde_json::json!({ "sampler": desc, "corr": corr, "seed": seed }),
        }))
    }

    /// Values of the i-th set (nominal + perturbation): {name: value}
    fn draw<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        self.0.draw_dict(py, i)
    }

    /// The system with the values of the i-th set
    fn system(&self, i: usize) -> PyResult<PySystem> {
        system_at(&self.0, i)
    }

    #[getter]
    fn seed(&self) -> u64 {
        match &self.0.plan {
            PlanKind::Sampler(s) => s.seed,
            PlanKind::Grid(_) => 0,
        }
    }
}

/// A sweep grid: the Cartesian product of {"3.t": [10, 11, 12], "5.dx": [0, 0.01]} (the values themselves). The first axis changes slowest in numbering
#[pyclass(module = "lensbench", name = "Grid", frozen)]
pub(crate) struct PyGrid(PlanBase);

#[pymethods]
impl PyGrid {
    #[new]
    fn new(system: &PySystem, axes: &Bound<'_, PyAny>) -> PyResult<Self> {
        let base = system.sys();
        let mut ax = Vec::new();
        let mut names = Vec::new();
        let mut desc = Vec::new();
        for (n, v) in pairs(axes)? {
            let p = base.param(&n).map_err(err)?;
            let vals: Vec<f64> = v.extract()?;
            desc.push(serde_json::json!([base.param_name(p), vals]));
            names.push(n);
            ax.push((p, vals));
        }
        let g = Grid::new(ax).map_err(err)?;
        Ok(PyGrid(PlanBase {
            plan: PlanKind::Grid(g),
            names,
            base,
            coat_files: system.coat_files.clone(),
            describe: serde_json::json!({ "grid": desc }),
        }))
    }

    fn __len__(&self) -> usize {
        self.0.plan.plan().count().unwrap_or(0)
    }

    /// Values of the i-th set: {name: value}
    fn draw<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        self.0.draw_dict(py, i)
    }

    /// The system with the values of the i-th set
    fn system(&self, i: usize) -> PyResult<PySystem> {
        system_at(&self.0, i)
    }
}

fn system_at(p: &PlanBase, i: usize) -> PyResult<PySystem> {
    let s = p.base.with_params(&p.changes(i)?).map_err(err)?;
    Ok(PySystem::new(s, p.coat_files.clone(), Default::default()))
}

#[derive(FromPyObject)]
enum PlanArg<'py> {
    Sampler(PyRef<'py, PySampler>),
    Grid(PyRef<'py, PyGrid>),
}

fn fmt_s(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// Batch run: calls func(i, system, seed) for each set of plan. Passing an extension's Rust simulation (`crate::ext::NativeTask`) as func
/// runs it on the set's system in Rust (without the GIL).
///
/// - plan: a `Sampler` or `Grid`. system is the system with the values of the i-th set, seed a seed per task (for noise and the like)
/// - n: number of sets (all for a Grid if omitted)
/// - out: JSONL to write the results to (the first line has the version, system hash, seed and how the sets are made; then one set per line in the order finished). After a stop,
///   resume=True continues the same file (an error if the head differs). None writes nothing
/// - threads: number of threads (1 for sequential). progress: omitted or True prints progress and time left to standard error (every second), False prints nothing,
///   a function is called with a dict (done total ran elapsed eta)
/// - header: values added to the head (a dict that can go into JSON and so on; the same when resuming)
///
/// func returns a value that can go into JSON (dict, list, numbers, strings, numpy arrays and scalars; NaN and inf become None).
///
/// Returns a list of results in numbering order (read back from the file if out is given). If func raises, no new set is started and
/// the same exception is raised after the running sets finish (the written results remain, so it can be resumed). The same with Ctrl-C
#[pyfunction]
#[pyo3(signature = (func, plan, n=None, out=None, threads=None, resume=true, progress=None, header=None))]
#[allow(clippy::too_many_arguments)]
fn run_batch<'py>(
    py: Python<'py>,
    func: Bound<'py, PyAny>,
    plan: PlanArg<'py>,
    n: Option<usize>,
    out: Option<PathBuf>,
    threads: Option<usize>,
    resume: bool,
    progress: Option<Bound<'py, PyAny>>,
    header: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let pb: &PlanBase = match &plan {
        PlanArg::Sampler(s) => &s.0,
        PlanArg::Grid(g) => &g.0,
    };
    let n = match (n, pb.plan.plan().count()) {
        (Some(n), Some(m)) if n > m => {
            return Err(value_err(format!("n = {n} is over the number of sets {m}")))
        }
        (Some(n), _) => n,
        (None, Some(m)) => m,
        (None, None) => return Err(value_err("Sampler needs n")),
    };
    let seed = match &pb.plan {
        PlanKind::Sampler(s) => s.seed,
        PlanKind::Grid(_) => 0,
    };
    let extra: serde_json::Value = match &header {
        None => serde_json::Value::Null,
        Some(h) => crate::pyjson::sorted(crate::pyjson::from_py(h)?),
    };
    let hash = lensbench_project::sysfile::system_hash(&pb.base).map_err(err)?;
    let head = serde_json::json!({
        "lensbench": lensbench_core::VERSION,
        "system_hash": hash,
        "seed": seed,
        "plan": pb.describe,
        "header": extra,
    })
    .to_string();
    // 拡張が登録した Rust の模擬（crate::ext::NativeTask）なら Rust のまま回す（GIL を持たない）
    let native = crate::ext::native_task(&func);
    let func = func.unbind();
    let show = match &progress {
        None => true,
        Some(p) => p.is_truthy()?,
    };
    let cb_py: Option<Py<PyAny>> = progress
        .as_ref()
        .filter(|p| p.is_callable())
        .map(|p| p.clone().unbind());
    let mem: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    let keep_mem = out.is_none();
    let first_err: Mutex<Option<PyErr>> = Mutex::new(None);
    let cancel = AtomicBool::new(false);
    let last = Mutex::new(Instant::now() - std::time::Duration::from_secs(10));
    let report = |p: Progress| {
        let mut g = last.lock_ok();
        let fin = p.done == p.total;
        if !fin && g.elapsed().as_secs_f64() < 1.0 {
            return;
        }
        *g = Instant::now();
        drop(g);
        if let Some(cb) = &cb_py {
            Python::attach(|py| {
                let d = PyDict::new(py);
                let _ = d.set_item("done", p.done);
                let _ = d.set_item("total", p.total);
                let _ = d.set_item("ran", p.ran);
                let _ = d.set_item("elapsed", p.elapsed);
                let _ = d.set_item("eta", p.eta);
                if let Err(e) = cb.call1(py, (d,)) {
                    first_err.lock_ok().get_or_insert(e);
                    cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if show {
            let pc = 100.0 * p.done as f64 / p.total.max(1) as f64;
            let eta = p.eta.map(fmt_s).unwrap_or_else(|| "?".into());
            let (done, total, el) = (p.done, p.total, fmt_s(p.elapsed));
            let line = format!("lensbench: {done}/{total} ({pc:.1}%) elapsed {el} left {eta}");
            eprintln!("{}", lensbench_core::i18n::tr(&line));
        }
    };
    let task = |i: usize, s: u64| -> lensbench_core::Result<String> {
        let fail = |e: PyErr| {
            first_err.lock_ok().get_or_insert(e);
            cancel.store(true, Ordering::Relaxed);
            lensbench_core::Error::Invalid("Python exception".into())
        };
        // 系は GIL なしで作る
        let sys = pb
            .plan
            .plan()
            .changes(i, &pb.base)
            .and_then(|c| pb.base.with_params(&c));
        if let Some(t) = &native {
            Python::attach(|py| py.check_signals()).map_err(fail)?;
            let sys = sys.map_err(|e| fail(err(e)))?;
            let t = t.run_json(&sys, s).map_err(|e| fail(err(e)))?;
            if keep_mem {
                mem.lock_ok().push((i, t.clone()));
            }
            return Ok(t);
        }
        Python::attach(|py| -> lensbench_core::Result<String> {
            py.check_signals().map_err(fail)?;
            let sys = match sys {
                Ok(s) => s,
                Err(e) => return Err(fail(err(e))),
            };
            let ps = PySystem::new(sys, pb.coat_files.clone(), Default::default());
            let r = func.call1(py, (i, ps, s)).map_err(fail)?;
            let t = crate::pyjson::from_py_nan_null(r.bind(py))
                .map_err(fail)?
                .to_string();
            if keep_mem {
                mem.lock_ok().push((i, t.clone()));
            }
            Ok(t)
        })
    };
    let opts = RunOpts {
        threads,
        out: out.as_deref(),
        header: head,
        resume,
        progress: Some(&report),
        cancel: Some(&cancel),
    };
    let res = py.detach(|| lensbench_runs::run(n, seed, &opts, task));
    if let Some(e) = first_err.into_inner().unwrap_or_else(|x| x.into_inner()) {
        return Err(e);
    }
    let sum = res.map_err(err)?;
    if sum.cancelled {
        return Err(PyKeyboardInterrupt::new_err(lensbench_core::i18n::tr(
            "the batch run was stopped",
        )));
    }
    let rows: Vec<(usize, String)> = match &out {
        Some(p) => lensbench_runs::read_results(p)
            .map_err(err)?
            .1
            .into_iter()
            .filter(|r| r.0 < n)
            .map(|r| (r.0, r.2))
            .collect(),
        None => {
            let mut v = mem.into_inner().unwrap_or_else(|x| x.into_inner());
            v.sort_by_key(|x| x.0);
            v
        }
    };
    let l = PyList::empty(py);
    for (_, t) in rows {
        l.append(crate::pyjson::loads(py, &t)?)?;
    }
    Ok(l)
}

/// Read a batch result file (JSONL): (head dict, [{"i", "seed", "result"}] in numbering order)
#[pyfunction]
fn read_batch<'py>(
    py: Python<'py>,
    path: PathBuf,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyList>)> {
    let (h, rows) = lensbench_runs::read_results(&path).map_err(err)?;
    let l = PyList::empty(py);
    for (i, s, r) in rows {
        let d = PyDict::new(py);
        d.set_item("i", i)?;
        d.set_item("seed", s)?;
        d.set_item("result", crate::pyjson::loads(py, &r)?)?;
        l.append(d)?;
    }
    Ok((crate::pyjson::loads(py, &h)?, l))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySampler>()?;
    m.add_class::<PyGrid>()?;
    m.add_function(wrap_pyfunction!(run_batch, m)?)?;
    m.add_function(wrap_pyfunction!(read_batch, m)?)?;
    Ok(())
}
