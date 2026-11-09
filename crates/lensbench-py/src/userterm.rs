//! 最適化の項: `terms=` の dict の並びを読み、Python の関数で定義した項（"type": "user"）を
//! [`UserTerm`] に包む。
//!
//! 利用者の関数 `fn(systems)` は System の list を受け取り、系ごとの残差を並べた配列（形 (系の数, 残差の数)）を返す。
//! 差分のヤコビアンでは、ずらした系をすべて 1 回の呼び出しで渡す。`jac(system, names)` を与えると、
//! 残差の偏微分（形 (残差の数, len(names))、names は変数の名前）をそれで求める。
//! 関数の例外は、最適化が終わったあとにそのまま投げ直す。

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use lensbench_core::optim::{Problem, Term, TermKind, UserTerm, UserTermRef};
use lensbench_core::param::Param;
use lensbench_core::System;
use numpy::{PyArrayDyn, PyArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::{err, PySystem};

pub(crate) struct PyUserTerm {
    name: String,
    rows: usize,
    f: Py<PyAny>,
    jac: Option<Py<PyAny>>,
    coat_files: Vec<PathBuf>,
    error: Mutex<Option<PyErr>>,
}

/// 関数の返り値を (系の数, rows) の行に（rows None なら 1 系の結果から決める）
fn to_rows(r: &Bound<'_, PyAny>, k: usize, rows: Option<usize>) -> PyResult<Vec<Vec<f64>>> {
    let np = r.py().import("numpy")?;
    let kw = PyDict::new(r.py());
    kw.set_item("dtype", "float64")?;
    let a = np.call_method("asarray", (r,), Some(&kw))?;
    let a = a.cast::<PyArrayDyn<f64>>()?.readonly();
    let v = a.as_array();
    let sh = v.shape().to_vec();
    let flat: Vec<f64> = v.iter().copied().collect();
    let bad = || {
        PyValueError::new_err(format!(
            "利用者の項の返り値の形 {sh:?}（(系の数 {k}, 残差の数) のはず）"
        ))
    };
    let (n, m) = match sh.len() {
        2 => (sh[0], sh[1]),
        1 if k == 1 => (1, sh[0]),
        1 if rows == Some(1) => (sh[0], 1),
        0 if k == 1 => (1, 1),
        _ => return Err(bad()),
    };
    if n != k || rows.is_some_and(|r| r != m) || m == 0 {
        return Err(bad());
    }
    Ok(flat.chunks(m).map(|c| c.to_vec()).collect())
}

impl PyUserTerm {
    fn fail(&self, e: PyErr) -> lensbench_core::Error {
        let msg = format!("利用者の項 {}: {e}", self.name);
        self.error
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .get_or_insert(e);
        lensbench_core::Error::Invalid(msg)
    }
    fn py_systems<'py>(
        &self,
        py: Python<'py>,
        systems: &[&System],
    ) -> PyResult<Bound<'py, PyList>> {
        let v: Vec<PySystem> = systems
            .iter()
            .map(|s| PySystem::new((*s).clone(), self.coat_files.clone(), OnceLock::new()))
            .collect();
        PyList::new(py, v)
    }
}

impl UserTerm for PyUserTerm {
    fn name(&self) -> &str {
        &self.name
    }
    fn rows(&self) -> usize {
        self.rows
    }
    fn eval(&self, systems: &[&System]) -> lensbench_core::Result<Vec<Vec<f64>>> {
        Python::attach(|py| {
            py.check_signals()?;
            let l = self.py_systems(py, systems)?;
            let r = self.f.call1(py, (l,))?;
            to_rows(r.bind(py), systems.len(), Some(self.rows))
        })
        .map_err(|e| self.fail(e))
    }
    fn grad(&self, s: &System, params: &[Param]) -> lensbench_core::Result<Option<Vec<f64>>> {
        let Some(jac) = &self.jac else {
            return Ok(None);
        };
        let names: Vec<String> = params.iter().map(|p| s.param_name(*p)).collect();
        Python::attach(|py| {
            py.check_signals()?;
            let ps = PySystem::new(s.clone(), self.coat_files.clone(), OnceLock::new());
            let r = jac.call1(py, (ps, names))?;
            // (残差, 変数) を「系」の代わりに残差を行にして読む
            let j = to_rows(r.bind(py), self.rows, Some(params.len()))?;
            Ok(Some(j.concat()))
        })
        .map_err(|e| self.fail(e))
    }
}

/// 最適化の後: 利用者の関数が例外を出していれば、最初のものを投げ直す
pub(crate) fn raise_user_errors(us: &[Arc<PyUserTerm>]) -> PyResult<()> {
    for u in us {
        if let Some(e) = u.error.lock().unwrap_or_else(|x| x.into_inner()).take() {
            return Err(e);
        }
    }
    Ok(())
}

/// terms= の並びを問題に足す。返り値は利用者の項（例外を拾うため）
pub(crate) fn add_terms(
    py: Python<'_>,
    p: &mut Problem,
    terms: &Bound<'_, PyAny>,
    coat_files: &[PathBuf],
) -> PyResult<Vec<Arc<PyUserTerm>>> {
    let dumps = py.import("json")?.getattr("dumps")?;
    let mut users = Vec::new();
    for (i, t) in terms.try_iter()?.enumerate() {
        let t = t?;
        let d = t
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err(format!("terms[{i}] は dict")))?;
        let ty: Option<String> = d.get_item("type")?.map(|v| v.extract()).transpose()?;
        if ty.as_deref() == Some("user") {
            let mut f = None;
            let mut jac = None;
            let mut name = format!("user{}", i + 1);
            let mut weight = 1.0;
            let mut config = 0;
            for (k, v) in d.iter() {
                let k: String = k.extract()?;
                match k.as_str() {
                    "type" => {}
                    "fn" => f = Some(v.unbind()),
                    "jac" if !v.is_none() => jac = Some(v.unbind()),
                    "jac" => {}
                    "name" => name = v.extract()?,
                    "weight" => weight = v.extract()?,
                    "config" => {
                        let c: usize = v.extract()?;
                        if c == 0 || c > p.configs.len() {
                            return Err(PyValueError::new_err(format!(
                                "terms[{i}]: 構成 {c} はない（1〜{}）",
                                p.configs.len()
                            )));
                        }
                        config = c - 1;
                    }
                    _ => {
                        return Err(PyValueError::new_err(format!(
                        "terms[{i}]: user の項に使わないキー {k}（fn・jac・name・weight・config）"
                    )))
                    }
                }
            }
            let Some(f) = f.filter(|f| f.bind(py).is_callable()) else {
                return Err(PyValueError::new_err(format!(
                    "terms[{i}]: user の項には関数 fn が要る"
                )));
            };
            // 残差の数は今の系で 1 回呼んで決める
            let s0 = PySystem::new(
                p.configs[config].clone(),
                coat_files.to_vec(),
                OnceLock::new(),
            );
            let l = PyList::new(py, [s0])?;
            let r0 = f.call1(py, (l,))?;
            let rows = to_rows(r0.bind(py), 1, None)?[0].len();
            let u = Arc::new(PyUserTerm {
                name,
                rows,
                f,
                jac,
                coat_files: coat_files.to_vec(),
                error: Mutex::new(None),
            });
            users.push(u.clone());
            p.terms.push(Term {
                side: Default::default(),
                config,
                weight,
                kind: TermKind::User(UserTermRef(u)),
            });
            continue;
        }
        let txt: String = dumps.call1((d,))?.extract()?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(err)?;
        let term = lensbench_project::merit::term_from_json(&v, &p.configs)
            .map_err(|e| PyValueError::new_err(format!("terms[{i}]: {e}")))?;
        p.terms.push(term);
    }
    Ok(users)
}
