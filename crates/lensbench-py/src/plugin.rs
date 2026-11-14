//! 検出器・解析のプラグイン（の `lensbench_core::plugin`）を Python から名前で呼ぶ口と、Python の関数を
//! プラグインとして登録する口（試作用）。
//!
//! - `lb.plugins()` → [(名前, 種類), ...]、`lb.plugin_spec(name)` → 説明の dict
//! - `p = lb.Plugin(name, **config)` → `p.run(system=None, **inputs)` で名前つきの配列の dict、
//!   `p.run_many([inputs, ...], system=None, threads=None)` で多数の入力をまとめて（入力ごとに並列、結果は入力の順）。
//!   `system`（`lb.System`）は系を追跡するプラグイン（MTF など）に渡す。`system` は入力の名前には使えない
//! - `lb.register_plugin(name, func, kind="analysis")`: `func(config: dict, inputs: dict) -> dict` を登録する。
//!   系を渡したときは `func(config, inputs, system)`（膜は渡した系のもの）。配列はまとめて（呼び出しごとに全部）
//!   渡す。Rust のプラグインと同じく `lb.Plugin(name, ...)` で呼べる
//!
//! 設定の値は数・文字列・数の並び。入力の配列は bool・complex・それ以外（float64 に直す）の 3 種類。

use std::sync::{Arc, OnceLock, RwLock};

use lensbench_core::coating::C64;
use lensbench_core::plugin::{
    self, Array, Arrays, Config, Context, Data, Instance, Kind, Plugin, Registry, Spec, Value,
};
use lensbench_core::{with_threads, Error};
use numpy::ndarray::{ArrayD, IxDyn};
use numpy::{IntoPyArray, PyReadonlyArrayDyn};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString};

use crate::{err, PySystem};

/// 組み込みのプラグインを登録する（モジュールの読込時）
fn register_builtin(reg: &mut Registry) {
    lensbench_analysis::register(reg);
}

fn read_registry() -> std::sync::RwLockReadGuard<'static, Registry> {
    plugin::global().read().unwrap_or_else(|e| e.into_inner())
}

fn write_registry() -> std::sync::RwLockWriteGuard<'static, Registry> {
    plugin::global().write().unwrap_or_else(|e| e.into_inner())
}

/// Python の値 → 設定の値
fn to_value(k: &str, v: &Bound<'_, PyAny>) -> PyResult<Value> {
    if v.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "設定 {k} に bool は使えない（数か文字列に）"
        )));
    }
    if let Ok(s) = v.cast::<PyString>() {
        return Ok(Value::Str(s.extract::<String>()?));
    }
    if let Ok(x) = v.extract::<f64>() {
        return Ok(Value::Num(x));
    }
    if let Ok(x) = v.extract::<Vec<f64>>() {
        return Ok(Value::Nums(x));
    }
    // numpy の配列など
    let np = v.py().import("numpy")?;
    let a = np.call_method1("ascontiguousarray", (v, "float64"));
    if let Ok(a) = a {
        if let Ok(a) = a.extract::<PyReadonlyArrayDyn<'_, f64>>() {
            return Ok(Value::Nums(a.as_array().iter().copied().collect()));
        }
    }
    Err(PyTypeError::new_err(format!(
        "設定 {k} は数・文字列・数の並びに"
    )))
}

fn to_config(cfg: Option<&Bound<'_, PyDict>>) -> PyResult<Config> {
    let mut c = Config::new();
    if let Some(d) = cfg {
        for (k, v) in d.iter() {
            let k: String = k.extract()?;
            let v = to_value(&k, &v)?;
            c.insert(k, v);
        }
    }
    Ok(c)
}

fn config_to_py<'py>(py: Python<'py>, c: &Config) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in c {
        match v {
            Value::Num(x) => d.set_item(k, x)?,
            Value::Str(s) => d.set_item(k, s)?,
            Value::Nums(x) => d.set_item(k, x.clone())?,
        }
    }
    Ok(d)
}

fn shaped<T: numpy::Element + Copy>(a: &Bound<'_, PyAny>) -> PyResult<(Vec<usize>, Vec<T>)> {
    let a: PyReadonlyArrayDyn<'_, T> = a.extract()?;
    let a = a.as_array();
    Ok((a.shape().to_vec(), a.iter().copied().collect()))
}

/// Python の配列 → 配列（bool・complex128・float64）
fn to_array(k: &str, v: &Bound<'_, PyAny>) -> PyResult<Array> {
    let np = v.py().import("numpy")?;
    let a = np.call_method1("asarray", (v,))?;
    let kind: String = a.getattr("dtype")?.getattr("kind")?.extract()?;
    let (dtype, conv) = match kind.as_str() {
        "b" => ("bool", 0),
        "c" => ("complex128", 1),
        "f" | "i" | "u" => ("float64", 2),
        _ => {
            return Err(PyTypeError::new_err(format!(
                "入力 {k} の型（dtype.kind {kind}）は使えない"
            )))
        }
    };
    // 0 次元の配列も形を保つ（ascontiguousarray は 1 次元にする）
    let kw = PyDict::new(v.py());
    kw.set_item("dtype", dtype)?;
    kw.set_item("order", "C")?;
    let a = np.call_method("asarray", (a,), Some(&kw))?;
    let r = match conv {
        0 => shaped::<bool>(&a).map(|(s, v)| Array::bool(s, v)),
        1 => shaped::<C64>(&a).map(|(s, v)| Array::c64(s, v)),
        _ => shaped::<f64>(&a).map(|(s, v)| Array::f64(s, v)),
    }?;
    r.map_err(err)
}

fn to_arrays(d: &Bound<'_, PyDict>) -> PyResult<Arrays> {
    let mut out = Arrays::new();
    for (k, v) in d.iter() {
        let k: String = k.extract()?;
        let a = to_array(&k, &v)?;
        out.insert(k, a);
    }
    Ok(out)
}

fn array_to_py<'py>(py: Python<'py>, a: &Array) -> PyResult<Bound<'py, PyAny>> {
    let sh = IxDyn(&a.shape);
    Ok(match &a.data {
        Data::F64(v) => ArrayD::from_shape_vec(sh, v.clone())
            .map_err(err)?
            .into_pyarray(py)
            .into_any(),
        Data::C64(v) => ArrayD::from_shape_vec(sh, v.clone())
            .map_err(err)?
            .into_pyarray(py)
            .into_any(),
        Data::Bool(v) => ArrayD::from_shape_vec(sh, v.clone())
            .map_err(err)?
            .into_pyarray(py)
            .into_any(),
    })
}

/// 名前つきの配列を dict に（numpy の配列）
pub fn arrays_to_py<'py>(py: Python<'py>, a: &Arrays) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in a {
        d.set_item(k, array_to_py(py, v)?)?;
    }
    Ok(d)
}

fn spec_to_py<'py>(
    py: Python<'py>,
    name: &str,
    kind: Kind,
    s: &Spec,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("name", name)?;
    d.set_item("kind", kind.as_str())?;
    d.set_item("doc", &s.doc)?;
    d.set_item("config", s.config.clone())?;
    d.set_item("inputs", s.inputs.clone())?;
    d.set_item("outputs", s.outputs.clone())?;
    d.set_item("needs_system", s.needs_system)?;
    Ok(d)
}

/// 登録されたプラグインの一覧 → [(名前, 種類), ...]
#[pyfunction]
fn plugins() -> Vec<(String, &'static str)> {
    read_registry()
        .list()
        .into_iter()
        .map(|(n, k)| (n, k.as_str()))
        .collect()
}

/// プラグインの説明 → {"name", "kind", "doc", "config", "inputs", "outputs"}
#[pyfunction]
fn plugin_spec<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    let p = read_registry().get(name).map_err(err)?;
    spec_to_py(py, p.name(), p.kind(), &p.spec())
}

/// Python の関数のプラグイン（試作用）。呼び出しのたびに GIL を取る
struct PyFuncPlugin {
    name: String,
    kind: Kind,
    func: Py<PyAny>,
    doc: String,
    needs_system: bool,
}

struct PyFuncInstance {
    func: Py<PyAny>,
    config: Config,
}

impl Plugin for PyFuncPlugin {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Kind {
        self.kind
    }
    fn spec(&self) -> Spec {
        Spec {
            doc: self.doc.clone(),
            needs_system: self.needs_system,
            ..Spec::default()
        }
    }
    fn prepare(&self, config: &Config) -> lensbench_core::Result<Box<dyn Instance>> {
        let func = Python::attach(|py| self.func.clone_ref(py));
        Ok(Box::new(PyFuncInstance {
            func,
            config: config.clone(),
        }))
    }
}

impl Instance for PyFuncInstance {
    fn run(&self, ctx: &Context<'_>, inputs: &Arrays) -> lensbench_core::Result<Arrays> {
        Python::attach(|py| -> PyResult<Arrays> {
            let cfg = config_to_py(py, &self.config)?;
            let inp = arrays_to_py(py, inputs)?;
            let f = self.func.bind(py);
            let r = match ctx.system {
                None => f.call1((cfg, inp))?,
                Some(sys) => {
                    let coat = OnceLock::new();
                    if let Some(c) = ctx.coatings {
                        let _ = coat.set(Ok(c.clone()));
                    }
                    let ps = PySystem {
                        sys: RwLock::new(sys.clone()),
                        coat_files: Vec::new(),
                        coat: RwLock::new(Arc::new(coat)),
                    };
                    f.call1((cfg, inp, Bound::new(py, ps)?))?
                }
            };
            let d = r
                .cast::<PyDict>()
                .map_err(|_| PyTypeError::new_err("プラグインの関数は dict を返すこと"))?;
            to_arrays(d)
        })
        .map_err(|e| Error::Invalid(format!("Python のプラグイン: {e}")))
    }
}

/// Python の関数をプラグインとして登録する（同じ名前は差し替える）。
/// `func(config: dict, inputs: dict[str, ndarray]) -> dict[str, array]`。配列は呼び出しごとにまとめて渡す。
/// 系を渡して呼ぶと `func(config, inputs, system)`。`needs_system=True` なら系の膜も読んでから渡す
#[pyfunction]
#[pyo3(signature = (name, func, kind="analysis", needs_system=false))]
fn register_plugin(
    name: String,
    func: &Bound<'_, PyAny>,
    kind: &str,
    needs_system: bool,
) -> PyResult<()> {
    if !func.is_callable() {
        return Err(PyTypeError::new_err("func は呼び出せるものに"));
    }
    let kind = Kind::parse(kind).map_err(err)?;
    let doc = func
        .getattr("__doc__")
        .ok()
        .and_then(|d| d.extract::<String>().ok())
        .unwrap_or_default();
    write_registry().register(Arc::new(PyFuncPlugin {
        name,
        kind,
        func: func.clone().unbind(),
        doc,
        needs_system,
    }));
    Ok(())
}

/// 登録を外す（あれば True）
#[pyfunction]
fn unregister_plugin(name: &str) -> bool {
    write_registry().unregister(name)
}

/// 設定の決まったプラグイン。`Plugin(name, **config)`。`run(**inputs)` → dict、
/// `run_many([inputs, ...], threads=None)` → [dict, ...]
#[pyclass(module = "lensbench", name = "Plugin", frozen)]
pub struct PyPlugin {
    name: String,
    kind: Kind,
    needs_system: bool,
    config: Config,
    /// 設定を当てたプラグイン
    pub inst: Arc<dyn Instance>,
}

#[pymethods]
impl PyPlugin {
    #[new]
    #[pyo3(signature = (name, **config))]
    fn new(name: String, config: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let cfg = to_config(config)?;
        let p = read_registry().get(&name).map_err(err)?;
        let inst: Arc<dyn Instance> = p.prepare(&cfg).map_err(err)?.into();
        Ok(Self {
            name,
            kind: p.kind(),
            needs_system: p.spec().needs_system,
            config: cfg,
            inst,
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    #[getter]
    fn kind(&self) -> &'static str {
        self.kind.as_str()
    }

    /// 設定（dict）
    #[getter]
    fn config<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        config_to_py(py, &self.config)
    }

    /// 1 回呼ぶ → 名前つきの配列の dict（Rust のプラグインは GIL を外して計算する）。`system` は系を追跡する
    /// プラグインに渡す `lb.System`
    #[pyo3(signature = (system=None, **inputs))]
    fn run<'py>(
        &self,
        py: Python<'py>,
        system: Option<&Bound<'py, PySystem>>,
        inputs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let a = match inputs {
            Some(d) => to_arrays(d)?,
            None => Arrays::new(),
        };
        let r = self.with_context(py, system, |inst, ctx| inst.run(ctx, &a))?;
        arrays_to_py(py, &r.map_err(err)?)
    }

    /// 多数の入力（dict の並び）をまとめて呼ぶ → [dict, ...]。入力ごとに `threads` 本で並列（None は既定）、
    /// 結果は入力の順でスレッド数によらない。どれかが誤りなら最初の誤りを出す
    #[pyo3(signature = (inputs, system=None, threads=None))]
    fn run_many<'py>(
        &self,
        py: Python<'py>,
        inputs: &Bound<'py, PyList>,
        system: Option<&Bound<'py, PySystem>>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyList>> {
        let mut all = Vec::with_capacity(inputs.len());
        for (i, d) in inputs.iter().enumerate() {
            let d = d
                .cast::<PyDict>()
                .map_err(|_| PyTypeError::new_err(format!("inputs[{i}] は dict に")))?;
            all.push(to_arrays(d)?);
        }
        let rs = self.with_context(py, system, |inst, ctx| {
            with_threads(threads, || plugin::run_many(inst, ctx, &all))
        })?;
        let out = PyList::empty(py);
        for (i, r) in rs.into_iter().enumerate() {
            let r = r.map_err(|e| PyValueError::new_err(format!("inputs[{i}]: {e}")))?;
            out.append(arrays_to_py(py, &r)?)?;
        }
        Ok(out)
    }

    fn __repr__(&self) -> String {
        format!(
            "Plugin({:?}, kind={:?}, config={:?})",
            self.name,
            self.kind.as_str(),
            self.config
        )
    }
}

impl PyPlugin {
    /// 系（あれば膜も）の文脈を作り、GIL を外して f を呼ぶ
    fn with_context<R: Send>(
        &self,
        py: Python<'_>,
        system: Option<&Bound<'_, PySystem>>,
        f: impl FnOnce(&dyn Instance, &Context<'_>) -> R + Send,
    ) -> PyResult<R> {
        let inst = self.inst.as_ref();
        let Some(ps) = system else {
            return Ok(py.detach(|| f(inst, &Context::default())));
        };
        let ps: &PySystem = ps.get();
        let sys = ps.sys();
        // 膜は系を追跡するプラグインにだけ読む（膜ファイルの誤りはここで出す）
        let coatings = if self.needs_system {
            Some(ps.coatings(&sys)?)
        } else {
            None
        };
        Ok(py.detach(|| {
            let ctx = Context {
                system: Some(&sys),
                coatings: coatings.as_deref(),
            };
            f(inst, &ctx)
        }))
    }
}

/// serde の JSON → Python
fn json_to_py<'py>(py: Python<'py>, v: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    crate::pyjson::to_py(py, v)
}

/// 照合: 記述（*.verify.json）かそれを含むフォルダ（1 つか並び）を、登録のプラグイン（同梱・`register_plugin` で
/// 足した分）で回し、照合ごとの結果の dict の並びを返す（`lensbench verify --json` の reports と同じ形）。
/// ci=True で ci: false の照合を飛ばす
#[pyfunction]
#[pyo3(signature = (paths, ci=false))]
fn verify<'py>(
    py: Python<'py>,
    paths: &Bound<'py, PyAny>,
    ci: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let ps: Vec<std::path::PathBuf> = match paths.extract::<std::path::PathBuf>() {
        Ok(p) => vec![p],
        Err(_) => paths.extract()?,
    };
    let reg = read_registry().clone();
    let opt = lensbench_verify::Options { ci_only: ci };
    let reps = py
        .detach(|| lensbench_verify::run_paths(&ps, &reg, &opt))
        .map_err(PyValueError::new_err)?;
    let v: Vec<serde_json::Value> = reps.iter().map(|r| r.to_json()).collect();
    json_to_py(py, &serde_json::Value::Array(v))
}

/// 照合の比べ方だけを使う: 出力 actual と基準 expected（名前 → 配列）を、checks（名前 → {abs, rel, exact,
/// note}、記述の checks と同じ）で比べ、量ごとの結果の dict の並びを返す。基準の名前には全部 checks が要る
#[pyfunction]
fn verify_compare<'py>(
    py: Python<'py>,
    actual: &Bound<'py, PyDict>,
    expected: &Bound<'py, PyDict>,
    checks: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyAny>> {
    let a = to_arrays(actual)?;
    let e = to_arrays(expected)?;
    let c = crate::pyjson::from_py(checks.as_any())?;
    let c = lensbench_verify::checks_from_json(&c).map_err(PyValueError::new_err)?;
    let r = lensbench_verify::compare(&a, &e, &c).map_err(PyValueError::new_err)?;
    let v: Vec<serde_json::Value> = r.iter().map(|x| x.to_json()).collect();
    json_to_py(py, &serde_json::Value::Array(v))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_builtin(&mut write_registry());
    m.add_function(wrap_pyfunction!(verify, m)?)?;
    m.add_function(wrap_pyfunction!(verify_compare, m)?)?;
    m.add_class::<PyPlugin>()?;
    m.add_function(wrap_pyfunction!(plugins, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_spec, m)?)?;
    m.add_function(wrap_pyfunction!(register_plugin, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_plugin, m)?)?;
    Ok(())
}
