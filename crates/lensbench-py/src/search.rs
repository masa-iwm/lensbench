//! ズーム軌跡とカタログレンズの差し替え探索の Python 側。

use std::path::PathBuf;
use std::sync::OnceLock;

use lensbench_core::catalog::LensCatalog;
use lensbench_core::optim::{DefaultMerit, DlsOpts, ParaxQ, Stop};
use lensbench_core::sweep::RmsRef;
use lensbench_core::with_threads;
use lensbench_core::zoom::{self as zoom_core, ZoomSpec};
use lensbench_project::swap::{SwapSearch, SwapSpec};
use lensbench_project::{Env, Project};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::{err, PySystem, VarArg};

fn stop_name(s: Stop) -> &'static str {
    match s {
        Stop::Converged => "converged",
        Stop::SmallStep => "small_step",
        Stop::NoProgress => "no_progress",
        Stop::MaxIter => "max_iter",
        Stop::Cancelled => "cancelled",
        Stop::NoVars => "no_vars",
    }
}

fn merit_kind(m: &str) -> PyResult<DefaultMerit> {
    match m {
        "spot" => Ok(DefaultMerit::Spot),
        "wavefront" => Ok(DefaultMerit::Wavefront),
        _ => Err(PyValueError::new_err(format!(
            "merit は \"spot\" か \"wavefront\"（{m}）"
        ))),
    }
}

fn reference_kind(r: &str) -> PyResult<RmsRef> {
    match r {
        "centroid" => Ok(RmsRef::Centroid),
        "chief" => Ok(RmsRef::Chief),
        _ => Err(PyValueError::new_err(format!(
            "reference は \"centroid\" か \"chief\"（{r}）"
        ))),
    }
}

/// ズーム軌跡: 状態（systems、ズーム位置ごとの系。同じ形の系）ごとに vars（"G2.shift" か
/// ("G2.shift", 下限, 上限)）を解く。評価関数は像の RMS（merit: "spot"・"wavefront"、全視野・多色）と、
/// efl か pmag の目標（状態ごとの値の list、None は目標なし）。steps は隣り合う状態の間にとる点の数（継続法）。
/// 返り値の dict: quantity（"efl"・"pmag"）・names（変数の名前）・path（点の dict の list: pos・state（状態の番号、
/// 間の点は None）・target・value（近軸量）・x・merit（像だけの評価関数）・stop・iterations）・systems（状態ごとの
/// 解いた系）・warnings
#[pyfunction]
#[pyo3(signature = (systems, vars, efl=None, pmag=None, steps=0, merit="spot", reference="centroid", rings=3, iters=50, tol=1e-8, threads=None))]
#[allow(clippy::too_many_arguments)]
fn zoom<'py>(
    py: Python<'py>,
    systems: Vec<PyRef<'py, PySystem>>,
    vars: Vec<VarArg>,
    efl: Option<Vec<Option<f64>>>,
    pmag: Option<Vec<Option<f64>>>,
    steps: usize,
    merit: &str,
    reference: &str,
    rings: usize,
    iters: usize,
    tol: f64,
    threads: Option<usize>,
) -> PyResult<Bound<'py, PyDict>> {
    if systems.is_empty() {
        return Err(PyValueError::new_err("systems が空"));
    }
    let cs: Vec<lensbench_core::System> = systems.iter().map(|s| (*s.sys()).clone()).collect();
    let (q, targets) = match (efl, pmag) {
        (Some(_), Some(_)) => return Err(PyValueError::new_err("efl と pmag はどちらか一方")),
        (Some(t), None) => (ParaxQ::Efl, t),
        (None, Some(t)) => (ParaxQ::Pmag, t),
        (None, None) => (ParaxQ::Efl, vec![None; cs.len()]),
    };
    let mut moving = Vec::new();
    let mut bounds = Vec::new();
    for v in &vars {
        let (name, lo, hi) = match v {
            VarArg::Name(n) => (n.clone(), None, None),
            VarArg::Range((n, lo, hi)) => (n.clone(), *lo, *hi),
        };
        moving.push(cs[0].param(&name).map_err(err)?);
        bounds.push((lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY)));
    }
    let mut sp = ZoomSpec::new(moving, targets);
    sp.bounds = bounds;
    sp.q = q;
    sp.steps = steps;
    sp.rings = rings;
    sp.target_tol = tol;
    sp.merit = merit_kind(merit)?;
    sp.reference = reference_kind(reference)?;
    sp.opts = DlsOpts {
        max_iter: iters,
        ..Default::default()
    };
    let r = py
        .detach(|| with_threads(threads, || zoom_core::solve(&cs, &sp)))
        .map_err(err)?;
    let d = PyDict::new(py);
    d.set_item("quantity", q.name())?;
    let names: Vec<String> = sp.moving.iter().map(|&p| cs[0].param_name(p)).collect();
    d.set_item("names", names)?;
    let path = PyList::empty(py);
    for p in &r.path {
        let e = PyDict::new(py);
        e.set_item("pos", p.pos)?;
        e.set_item("state", p.state)?;
        e.set_item("target", p.target)?;
        e.set_item("value", p.value)?;
        e.set_item("x", p.x.clone())?;
        e.set_item("merit", p.merit)?;
        e.set_item("stop", stop_name(p.stop))?;
        e.set_item("iterations", p.iterations)?;
        path.append(e)?;
    }
    d.set_item("path", path)?;
    let sys = PyList::empty(py);
    for (k, s) in r.systems.into_iter().enumerate() {
        let src = &systems[k];
        sys.append(PySystem::new(s, src.coat_files.clone(), OnceLock::new()))?;
    }
    d.set_item("systems", sys)?;
    d.set_item("warnings", r.warnings)?;
    Ok(d)
}

/// JSON の文字列か dict を文字列に
fn json_text(py: Python<'_>, v: &Bound<'_, PyAny>, what: &str) -> PyResult<String> {
    if let Ok(t) = v.extract::<String>() {
        Ok(t)
    } else if v.cast::<PyDict>().is_ok() {
        py.import("json")?.call_method1("dumps", (v,))?.extract()
    } else {
        Err(PyTypeError::new_err(format!(
            "{what} は JSON の文字列か dict"
        )))
    }
}

/// カタログレンズの差し替え探索: project（プロジェクトファイルのパス）の玉のブロックを spec（dict か JSON の
/// 文字列。CLI の swap --spec と同じ: slots・vars・merit・iters・efl・state など）の候補で差し替えた組を並列に回し、
/// 組ごとに変数を最適化する。返り値は組の結果の dict の list（評価関数の小さい順、top > 0 なら先頭の top 個）。
/// 各 dict は CLI の swap --json の results と同じ（i・lenses・merit0・merit・image_merit・vars・efl・stop・
/// iterations・total_track、評価できなければ error）。out（JSONL）を与えると組ごとに逐次書き出し、resume なら
/// 同じ探索のファイルの続きから回す（一括実行の形）
#[pyfunction]
#[pyo3(signature = (project, spec, glass_dirs=None, lens_catalogs=None, top=0, threads=None, out=None, resume=true))]
#[allow(clippy::too_many_arguments)]
fn swap_search<'py>(
    py: Python<'py>,
    project: PathBuf,
    spec: &Bound<'py, PyAny>,
    glass_dirs: Option<Vec<PathBuf>>,
    lens_catalogs: Option<Vec<PathBuf>>,
    top: usize,
    threads: Option<usize>,
    out: Option<PathBuf>,
    resume: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let st = json_text(py, spec, "spec")?;
    let glass_dirs = glass_dirs.unwrap_or_default();
    let lens_catalogs = lens_catalogs.unwrap_or_default();
    let out = py
        .detach(|| -> Result<String, String> {
            let txt = std::fs::read_to_string(&project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            let p = Project::from_json(&txt)?;
            let spec = SwapSpec::from_json(&st)?;
            let cat = LensCatalog::load_registered(&lens_catalogs)?;
            let env = Env {
                base: project
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default(),
                glass_dirs: glass_dirs.clone(),
                catalog: None,
            };
            let s = SwapSearch::new(&p, &env, &cat, spec)?;
            let mut r = s.run_to(out.as_deref(), resume, threads, None)?;
            if top > 0 {
                r.truncate(top);
            }
            Ok(serde_json::to_string(&r).unwrap())
        })
        .map_err(err)?;
    py.import("json")?.call_method1("loads", (out,))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(zoom, m)?)?;
    m.add_function(wrap_pyfunction!(swap_search, m)?)?;
    Ok(())
}
