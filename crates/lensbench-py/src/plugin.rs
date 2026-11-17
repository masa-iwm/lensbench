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

use crate::errors::{type_err, value_err};
use std::sync::{Arc, OnceLock, RwLock};

use lensbench_core::coating::C64;
use lensbench_core::plugin::{
    self, Array, Arrays, Config, Context, Data, Instance, Kind, ParamType, Plugin, Registry, Spec,
    Value,
};
use lensbench_core::{with_threads, Error};
use numpy::ndarray::{ArrayD, IxDyn};
use numpy::{IntoPyArray, PyReadonlyArrayDyn};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString};

use crate::{err, PySystem};

/// Register the built-in plugins (when the module is loaded)
fn register_builtin(reg: &mut Registry) {
    lensbench_analysis::register(reg);
}

/// Add the built-in plugins whose names are not in reg yet. A plugin an extension module installed
/// with the same name stays, so the order of `init_module` and the extension's install does not matter
fn add_missing_builtin(reg: &mut Registry) {
    let mut b = Registry::new();
    register_builtin(&mut b);
    for p in b.plugins() {
        if reg.get(p.name()).is_err() {
            reg.register(p.clone());
        }
    }
}

/// The plugins of the current extension table (a copy; the contents are Arc, so it is light)
fn read_registry() -> Registry {
    lensbench_core::ext::current().plugins().clone()
}

/// Change the plugins of the process's default extension table with f (calculations started before the replacement keep the previous table)
fn write_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    lensbench_core::ext::update_default(|e| f(e.plugins_mut()))
}

/// Python value → setting value
fn to_value(k: &str, v: &Bound<'_, PyAny>) -> PyResult<Value> {
    if v.is_instance_of::<PyBool>() {
        return Ok(Value::Bool(v.extract()?));
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
    Err(type_err(format!(
        "setting {k} must be a number, bool, string or list of numbers"
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
            Value::Bool(b) => d.set_item(k, b)?,
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

/// Python array → array (bool, complex128, float64)
fn to_array(k: &str, v: &Bound<'_, PyAny>) -> PyResult<Array> {
    let np = v.py().import("numpy")?;
    let a = np.call_method1("asarray", (v,))?;
    let kind: String = a.getattr("dtype")?.getattr("kind")?.extract()?;
    let (dtype, conv) = match kind.as_str() {
        "b" => ("bool", 0),
        "c" => ("complex128", 1),
        "f" | "i" | "u" => ("float64", 2),
        _ => {
            return Err(type_err(format!(
                "input {k} type (dtype.kind {kind}) can't be used"
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

/// Named arrays into a dict (numpy arrays)
pub fn arrays_to_py<'py>(py: Python<'py>, a: &Arrays) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in a {
        d.set_item(k, array_to_py(py, v)?)?;
    }
    Ok(d)
}

fn spec_to_py<'py>(py: Python<'py>, p: &dyn Plugin) -> PyResult<Bound<'py, PyDict>> {
    let s = p.spec();
    let d = PyDict::new(py);
    d.set_item("name", p.name())?;
    d.set_item("kind", p.kind().as_str())?;
    d.set_item("doc", &s.doc)?;
    // 設定の項目（名前・型・既定・単位・説明。選択肢は choices、並びの個数の上限は len）
    let cfg = PyList::empty(py);
    for q in p.params() {
        let e = PyDict::new(py);
        e.set_item("name", &q.name)?;
        e.set_item("type", q.ty.as_str())?;
        match &q.default {
            None => e.set_item("default", py.None())?,
            Some(v) => {
                let c = config_to_py(py, &Config::from([("v".to_string(), v.clone())]))?;
                e.set_item("default", c.get_item("v")?)?;
            }
        }
        e.set_item("unit", &q.unit)?;
        e.set_item("doc", &q.doc)?;
        match &q.ty {
            ParamType::Choice(c) => e.set_item("choices", c.clone())?,
            ParamType::Nums { len: Some(n) } => e.set_item("len", n)?,
            _ => {}
        }
        cfg.append(e)?;
    }
    d.set_item("config", cfg)?;
    d.set_item("inputs", s.inputs.clone())?;
    // 出力（名前・型・形・単位・説明の dict の並び）
    let outs = PyList::empty(py);
    for o in &s.outputs {
        let e = PyDict::new(py);
        e.set_item("name", &o.name)?;
        e.set_item("dtype", &o.dtype)?;
        e.set_item("shape", &o.shape)?;
        e.set_item("unit", &o.unit)?;
        e.set_item("doc", &o.doc)?;
        outs.append(e)?;
    }
    d.set_item("outputs", outs)?;
    d.set_item("needs_system", s.needs_system)?;
    Ok(d)
}

/// The registered plugins → [(name, kind), ...]
#[pyfunction]
fn plugins() -> Vec<(String, &'static str)> {
    read_registry()
        .list()
        .into_iter()
        .map(|(n, k)| (n, k.as_str()))
        .collect()
}

/// A plugin's description → {"name", "kind", "doc", "config" (list of dicts of setting items), "inputs", "outputs" (list of dicts of output name, dtype, shape and unit)}
#[pyfunction]
fn plugin_spec<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    let p = read_registry().get(name).map_err(err)?;
    spec_to_py(py, p.as_ref())
}

/// A plugin made of a Python function (for prototypes). Takes the GIL on each call
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
                .map_err(|_| type_err("a plugin function must return a dict"))?;
            to_arrays(d)
        })
        .map_err(|e| Error::Invalid(format!("Python plugin: {e}")))
    }
}

/// Register a Python function as a plugin (replaces one with the same name).
/// `func(config: dict, inputs: dict[str, ndarray]) -> dict[str, array]`. Arrays are passed together per call.
/// When called with a system, `func(config, inputs, system)`. With `needs_system=True` the system's coatings are read before passing it
#[pyfunction]
#[pyo3(signature = (name, func, kind="analysis", needs_system=false))]
fn register_plugin(
    name: String,
    func: &Bound<'_, PyAny>,
    kind: &str,
    needs_system: bool,
) -> PyResult<()> {
    if !func.is_callable() {
        return Err(type_err("func must be callable"));
    }
    let kind = Kind::parse(kind).map_err(err)?;
    let doc = func
        .getattr("__doc__")
        .ok()
        .and_then(|d| d.extract::<String>().ok())
        .unwrap_or_default();
    let p = Arc::new(PyFuncPlugin {
        name,
        kind,
        func: func.clone().unbind(),
        doc,
        needs_system,
    });
    write_registry(|r| r.register(p));
    Ok(())
}

/// Remove a registration (True if there was one)
#[pyfunction]
fn unregister_plugin(name: &str) -> bool {
    write_registry(|r| r.unregister(name))
}

/// A plugin with fixed settings. `Plugin(name, **config)`. `run(**inputs)` → dict,
/// `run_many([inputs, ...], threads=None)` → [dict, ...]
#[pyclass(module = "lensbench", name = "Plugin", frozen)]
pub struct PyPlugin {
    name: String,
    kind: Kind,
    needs_system: bool,
    config: Config,
    /// A plugin with its settings applied
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

    /// The settings (dict)
    #[getter]
    fn config<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        config_to_py(py, &self.config)
    }

    /// Call once → a dict of named arrays (Rust plugins compute without the GIL). `system` is the `lb.System` passed to plugins
    /// that trace the system
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

    /// Call with many inputs (a list of dicts) together → [dict, ...]. Parallel over inputs with `threads` threads (None for the default);
    /// results are in input order and independent of the number of threads. If any fails, the first error is raised
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
                .map_err(|_| type_err(format!("inputs[{i}] must be a dict")))?;
            all.push(to_arrays(d)?);
        }
        let rs = self.with_context(py, system, |inst, ctx| {
            with_threads(threads, || plugin::run_many(inst, ctx, &all))
        })?;
        let out = PyList::empty(py);
        for (i, r) in rs.into_iter().enumerate() {
            let r = r.map_err(|e| value_err(format!("inputs[{i}]: {e}")))?;
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
    /// Make the context of a system (with coatings if any) and call f without the GIL
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

/// serde JSON → Python
fn json_to_py<'py>(py: Python<'py>, v: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    crate::pyjson::to_py(py, v)
}

/// Verification: runs descriptions (*.verify.json) or folders containing them (one or a list) with the registered plugins (bundled ones and those added with
/// `register_plugin`) and returns a list of result dicts per verification (the same form as the reports of `lensbench verify --json`).
/// ci=True skips verifications with ci: false
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
    let reg = read_registry();
    let opt = lensbench_verify::Options { ci_only: ci };
    let reps = py
        .detach(|| lensbench_verify::run_paths(&ps, &reg, &opt))
        .map_err(value_err)?;
    let v: Vec<serde_json::Value> = reps.iter().map(|r| r.to_json()).collect();
    json_to_py(py, &serde_json::Value::Array(v))
}

/// Use only the comparison of verification: compares the outputs actual with the references expected (name → array) by checks (name → {abs, rel, exact,
/// note}, the same as the checks of a description) and returns a list of result dicts per quantity. Every reference name needs checks
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
    let c = lensbench_verify::checks_from_json(&c).map_err(value_err)?;
    let r = lensbench_verify::compare(&a, &e, &c).map_err(value_err)?;
    let v: Vec<serde_json::Value> = r.iter().map(|x| x.to_json()).collect();
    json_to_py(py, &serde_json::Value::Array(v))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    write_registry(add_missing_builtin);
    m.add_function(wrap_pyfunction!(verify, m)?)?;
    m.add_function(wrap_pyfunction!(verify_compare, m)?)?;
    m.add_class::<PyPlugin>()?;
    m.add_function(wrap_pyfunction!(plugins, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_spec, m)?)?;
    m.add_function(wrap_pyfunction!(register_plugin, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_plugin, m)?)?;
    Ok(())
}
