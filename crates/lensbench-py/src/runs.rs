//! 一括実行を Python から: 抽選 `Sampler`・掃引 `Grid` と `run_batch`。
//!
//! 各組で Python の関数 func(i, system, seed) を呼び、返した値（JSON にできるもの）を JSONL に逐次書く。
//! 並列はスレッドで（プロセスは使わない）。func の中の lensbench の追跡・解析は GIL を外すので、その間はほかの
//! スレッドの func が進む。func が Python の計算ばかりなら並列にはならない（重い所は Rust 側へ の方針）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use lensbench_core::param::Param;
use lensbench_core::runs::{self, Dist, Grid, Plan, Progress, RunOpts, Sampler};
use lensbench_core::System;
use pyo3::exceptions::{PyKeyboardInterrupt, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyDict, PyList, PyTuple};

use crate::{err, PySystem};

/// 分布の指定: ("normal", σ[, 平均]) ・("uniform", lo, hi) ・("trunc", σ, cut[, 平均]) ・("fixed", 値)、数だけなら正規分布の σ
fn dist_of(v: &Bound<'_, PyAny>) -> PyResult<Dist> {
    if let Ok(s) = v.extract::<f64>() {
        return Ok(Dist::Normal {
            mean: 0.0,
            sigma: s,
        });
    }
    let t = v
        .cast::<PyTuple>()
        .map_err(|_| PyValueError::new_err("分布は σ の数か (\"normal\", σ) などの tuple"))?;
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
            return Err(PyValueError::new_err(format!(
                "分布 {k}（normal uniform trunc fixed）"
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

/// {名前: 値} か [(名前, 値)] を並びに（順を保つ）
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

/// 組の作り方と、その公称の系
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

/// 抽選（モンテカルロ）: 系 system の公称値に、パラメータごとの分布のずれを足す。
///
/// - params: {"3.t": 0.02, "5.dx": ("uniform", -0.01, 0.01), "G.tx": ("trunc", 0.05, 2.0)}（数だけなら正規分布の σ）
/// - corr: 相関行列（params の順）。分布ごとの周辺分布はそのままで、ガウスのコピュラで相関を入れる
/// - seed: 乱数の種。i 番目の組は (seed, i) だけで決まる（スレッド数・再開・順序によらない）
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

    /// i 番目の組の値（公称値 + ずれ）: {名前: 値}
    fn draw<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        self.0.draw_dict(py, i)
    }

    /// i 番目の組の値を入れた系
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

/// 掃引の格子: {"3.t": [10, 11, 12], "5.dx": [0, 0.01]} の直積（値そのもの）。番号は最初の軸が最も遅く変わる順
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

    /// i 番目の組の値: {名前: 値}
    fn draw<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        self.0.draw_dict(py, i)
    }

    /// i 番目の組の値を入れた系
    fn system(&self, i: usize) -> PyResult<PySystem> {
        system_at(&self.0, i)
    }
}

fn system_at(p: &PlanBase, i: usize) -> PyResult<PySystem> {
    let s = p.base.with_params(&p.changes(i)?).map_err(err)?;
    Ok(PySystem::new(s, p.coat_files.clone(), OnceLock::new()))
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

/// 一括実行: plan の各組で func(i, system, seed) を呼ぶ。func に拡張の Rust の模擬（`crate::ext::NativeTask`）を
/// 渡すと、組の系で Rust のまま（GIL なしで）回す。
///
/// - plan: `Sampler` か `Grid`。system は i 番目の組の値を入れた系、seed はタスクごとの種（雑音などに使う）
/// - n: 組の数（Grid は省くと全部）
/// - out: 結果を書く JSONL（1 行目に版・系の hash・種・組の作り方、以後 1 行 1 組で終わった順）。止まっても
///   resume=True なら同じファイルの続きから（頭が違えばエラー）。None なら書かない
/// - threads: スレッド数（1 で逐次）。progress: 省略か True で標準エラーに進み具合と残り時間（1 秒ごと）、False で出さない、
///   関数なら dict（done total ran elapsed eta）で呼ぶ
/// - header: 頭に足す値（JSON にできる dict など。再開のときは同じに）
///
/// 返すのは番号の順の結果の list（out があればファイルから読み直したもの）。func が例外を出すと新しい組を始めず、
/// 走っている組が終わってから同じ例外を出す（書いた結果は残るので再開できる）。Ctrl-C でも同じ
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
            return Err(PyValueError::new_err(format!(
                "n = {n} は組の数 {m} を超える"
            )))
        }
        (Some(n), _) => n,
        (None, Some(m)) => m,
        (None, None) => return Err(PyValueError::new_err("Sampler では n が要る")),
    };
    let seed = match &pb.plan {
        PlanKind::Sampler(s) => s.seed,
        PlanKind::Grid(_) => 0,
    };
    let json = py.import("json")?;
    let extra: serde_json::Value = match &header {
        None => serde_json::Value::Null,
        Some(h) => {
            let t: String = json
                .call_method(
                    "dumps",
                    (h,),
                    Some(&[("sort_keys", true)].into_py_dict(py)?),
                )?
                .extract()?;
            serde_json::from_str(&t).map_err(|e| PyValueError::new_err(format!("header: {e}")))?
        }
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
    let dumps = json.getattr("dumps")?.unbind();
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
        let mut g = last.lock().unwrap_or_else(|e| e.into_inner());
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
                    first_err
                        .lock()
                        .unwrap_or_else(|x| x.into_inner())
                        .get_or_insert(e);
                    cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if show {
            let pc = 100.0 * p.done as f64 / p.total.max(1) as f64;
            let eta = p.eta.map(fmt_s).unwrap_or_else(|| "?".into());
            eprintln!(
                "lensbench: {}/{} ({pc:.1}%) 経過 {} 残り {eta}",
                p.done,
                p.total,
                fmt_s(p.elapsed)
            );
        }
    };
    let task = |i: usize, s: u64| -> lensbench_core::Result<String> {
        let fail = |e: PyErr| {
            first_err
                .lock()
                .unwrap_or_else(|x| x.into_inner())
                .get_or_insert(e);
            cancel.store(true, Ordering::Relaxed);
            lensbench_core::Error::Invalid("Python の例外".into())
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
                mem.lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .push((i, t.clone()));
            }
            return Ok(t);
        }
        Python::attach(|py| -> lensbench_core::Result<String> {
            py.check_signals().map_err(fail)?;
            let sys = match sys {
                Ok(s) => s,
                Err(e) => return Err(fail(err(e))),
            };
            let ps = PySystem::new(sys, pb.coat_files.clone(), OnceLock::new());
            let r = func.call1(py, (i, ps, s)).map_err(fail)?;
            let t: String = dumps
                .call1(py, (r,))
                .and_then(|x| x.extract(py))
                .map_err(fail)?;
            if keep_mem {
                mem.lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .push((i, t.clone()));
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
    let res = py.detach(|| runs::run(n, seed, &opts, task));
    if let Some(e) = first_err.into_inner().unwrap_or_else(|x| x.into_inner()) {
        return Err(e);
    }
    let sum = res.map_err(err)?;
    if sum.cancelled {
        return Err(PyKeyboardInterrupt::new_err("一括実行を止めた"));
    }
    let rows: Vec<(usize, String)> = match &out {
        Some(p) => runs::read_results(p)
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
    let loads = json.getattr("loads")?;
    let l = PyList::empty(py);
    for (_, t) in rows {
        l.append(loads.call1((t,))?)?;
    }
    Ok(l)
}

/// 一括実行の結果のファイル（JSONL）を読む: (頭の dict, [{"i", "seed", "result"}] 番号の順)
#[pyfunction]
fn read_batch<'py>(
    py: Python<'py>,
    path: PathBuf,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyList>)> {
    let (h, rows) = runs::read_results(&path).map_err(err)?;
    let loads = py.import("json")?.getattr("loads")?;
    let l = PyList::empty(py);
    for (i, s, r) in rows {
        let d = PyDict::new(py);
        d.set_item("i", i)?;
        d.set_item("seed", s)?;
        d.set_item("result", loads.call1((r,))?)?;
        l.append(d)?;
    }
    Ok((loads.call1((h,))?, l))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySampler>()?;
    m.add_class::<PyGrid>()?;
    m.add_function(wrap_pyfunction!(run_batch, m)?)?;
    m.add_function(wrap_pyfunction!(read_batch, m)?)?;
    Ok(())
}
