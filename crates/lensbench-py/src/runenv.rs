//! `lensbench run` から渡される設定と、進みの表示（Python の関数）

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Settings passed by `lensbench run`. All are read from environment variables; empty or None if not set
///
/// - glass_dirs: folders to search for glass catalogs (LENSBENCH_GLASSCAT; System.load and others also read it)
/// - coatings: coating tables and models (LENSBENCH_COATINGS; the default of coatings= of System.load)
/// - threads: number of threads (LENSBENCH_THREADS; the default of calculations with threads= omitted)
/// - out: output folder (LENSBENCH_OUT); for the script's own use
/// - log: log file (LENSBENCH_LOG)
/// - args: arguments passed to the script (same as sys.argv[1:])
/// - under_run: whether started from `lensbench run`
#[pyfunction]
fn run_settings<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    let paths = |k: &str| -> Vec<String> {
        std::env::var_os(k)
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.display().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let opt = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let d = PyDict::new(py);
    d.set_item("glass_dirs", paths("LENSBENCH_GLASSCAT"))?;
    d.set_item("coatings", paths("LENSBENCH_COATINGS"))?;
    d.set_item("threads", lensbench_core::par::default_threads())?;
    d.set_item("out", opt("LENSBENCH_OUT"))?;
    d.set_item("log", opt("LENSBENCH_LOG"))?;
    let argv: Vec<String> = py.import("sys")?.getattr("argv")?.extract()?;
    d.set_item("args", argv.into_iter().skip(1).collect::<Vec<_>>())?;
    d.set_item(
        "under_run",
        std::env::var("LENSBENCH_RUN").is_ok_and(|v| v == "1"),
    )?;
    Ok(d)
}

/// Print the progress on one line to standard error: `lensbench:progress 3/10 message`.
/// `lensbench run` picks up this line and logs it; a future GUI will use it to show progress
#[pyfunction]
#[pyo3(signature = (done, total, message=""))]
fn progress(py: Python<'_>, done: usize, total: usize, message: &str) -> PyResult<()> {
    let line = if message.is_empty() {
        format!("lensbench:progress {done}/{total}\n")
    } else {
        format!("lensbench:progress {done}/{total} {message}\n")
    };
    let err = py.import("sys")?.getattr("stderr")?;
    err.call_method1("write", (line,))?;
    err.call_method0("flush")?;
    Ok(())
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_settings, m)?)?;
    m.add_function(wrap_pyfunction!(progress, m)?)?;
    Ok(())
}
