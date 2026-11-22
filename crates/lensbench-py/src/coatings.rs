//! 膜の一覧を Python から登録する（拡張の表の膜の一覧。Rust の `Extensions::coating_catalog` と同じ表に入る）

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lensbench_core::coating::C64;
use lensbench_core::usercoating::{
    check_coatings, CoatCoefs, CoatInput, CoatPath, CoatingModel, CoatingSet,
};
use lensbench_core::{Error, LockExt as _};
use numpy::{IntoPyArray, PyReadonlyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::err;
use crate::errors::{type_err, value_err};

/// 格子の点の数（cos1 の 0〜1）
const GRID: usize = 2001;

/// 境界の条件の組（波長・屈折率・経路・向き）。cos1 を除いたもの
type Key = (u64, u64, u64, u8, bool);

/// 係数を Python の関数で計算する膜（試作用）。境界の条件の組ごとに cos1 の格子で 1 回呼び、表を覚えて線形に補間する
struct PyCoating {
    name: String,
    func: Py<PyAny>,
    doc: String,
    /// 組ごとの表（格子の点ごとの rs・rp・ts・tp）
    tables: Mutex<HashMap<Key, Arc<Vec<[C64; 4]>>>>,
}

impl std::fmt::Debug for PyCoating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCoating")
            .field("name", &self.name)
            .finish()
    }
}

impl PyCoating {
    fn table(&self, x: &CoatInput) -> lensbench_core::Result<Arc<Vec<[C64; 4]>>> {
        let path = match x.path {
            CoatPath::Transmit => 0,
            CoatPath::Mirror => 1,
            _ => 2,
        };
        let n2 = if x.n2.is_nan() {
            f64::NAN.to_bits()
        } else {
            x.n2.to_bits()
        };
        let key = (x.wl.to_bits(), x.n1.to_bits(), n2, path, x.from_substrate);
        if let Some(t) = self.tables.lock_ok().get(&key) {
            return Ok(t.clone());
        }
        let t = Arc::new(self.call(x)?);
        self.tables.lock_ok().insert(key, t.clone());
        Ok(t)
    }

    fn call(&self, x: &CoatInput) -> lensbench_core::Result<Vec<[C64; 4]>> {
        let path = match x.path {
            CoatPath::Transmit => "transmit",
            CoatPath::Mirror => "mirror",
            _ => "reflect",
        };
        Python::attach(|py| -> PyResult<Vec<[C64; 4]>> {
            let cos: Vec<f64> = (0..GRID).map(|i| i as f64 / (GRID - 1) as f64).collect();
            let r = self.func.bind(py).call1((
                x.wl,
                x.n1,
                x.n2,
                cos.into_pyarray(py),
                path,
                x.from_substrate,
            ))?;
            let d = r
                .cast::<PyDict>()
                .map_err(|_| type_err("a coating function must return a dict"))?;
            let np = py.import("numpy")?;
            let mut cols = Vec::with_capacity(4);
            for k in ["rs", "rp", "ts", "tp"] {
                let v = d
                    .get_item(k)?
                    .ok_or_else(|| value_err(format!("a coating function must return {k}")))?;
                let a = np.call_method1("broadcast_to", (v, GRID))?;
                let a = np.call_method1("ascontiguousarray", (a, "complex128"))?;
                let a: PyReadonlyArray1<'_, C64> = a.extract().map_err(|_| {
                    value_err(format!(
                        "{k} of a coating function must have the shape of cos1"
                    ))
                })?;
                cols.push(a.as_array().to_vec());
            }
            Ok((0..GRID)
                .map(|i| [cols[0][i], cols[1][i], cols[2][i], cols[3][i]])
                .collect())
        })
        .map_err(|e| Error::Invalid(format!("Python coating {}: {e}", self.name)))
    }
}

impl CoatingModel for PyCoating {
    fn coefs(&self, x: &CoatInput) -> lensbench_core::Result<CoatCoefs> {
        let t = self.table(x)?;
        let u = x.cos1.clamp(0.0, 1.0) * (GRID - 1) as f64;
        let i = (u.floor() as usize).min(GRID - 2);
        let f = u - i as f64;
        let c = |j: usize| t[i][j] * (1.0 - f) + t[i + 1][j] * f;
        Ok(CoatCoefs {
            rs: c(0),
            rp: c(1),
            ts: c(2),
            tp: c(3),
        })
    }
    fn doc(&self) -> String {
        self.doc.clone()
    }
}

/// Register a coating catalog (replaces one with the same name). `files` are coating model (.lbcm, .lbcb), measured table (.lbct)
/// or coating file (COATING.DAT format: MATE, COAT, TABLE) files, `dat` is text in the COATING.DAT format. `models` is {coating name: func}
/// of coatings whose coefficients a Python function computes (for prototypes: only inside this Python process, not from the CLI or GUI):
/// `func(wl, n1, n2, cos1, path, from_substrate) -> {"rs", "rp", "ts", "tp"}` (complex arrays like cos1, power-normalized amplitudes; path is
/// "transmit", "mirror" (n2 is NaN) or "reflect"). It is called once per wavelength, indices, path and side with cos1 on a grid of 2001 points
/// from 0 to 1, and the coefficients are interpolated linearly in cos1. The coatings are looked up by
/// the surface's coating name before the coating file of the .zmx and the coating folder. Returns the number of coatings.
/// A coating with the same name in another registered catalog is an error. Register before loading the systems that use it
/// (a system reads its coatings once, at the first polarization trace)
#[pyfunction]
#[pyo3(signature = (name, files=Vec::new(), dat=None, models=None))]
fn register_coating_catalog(
    name: String,
    files: Vec<PathBuf>,
    dat: Option<String>,
    models: Option<&Bound<'_, PyDict>>,
) -> PyResult<usize> {
    let mut set = CoatingSet::new();
    for f in &files {
        set.add_file(f).map_err(err)?;
    }
    if let Some(t) = dat {
        let d = CoatingSet::parse_dat(&t).map_err(err)?;
        set.mate.extend(d.mate);
        set.coat.extend(d.coat);
        set.table.extend(d.table);
    }
    for (k, f) in models.iter().flat_map(|d| d.iter()) {
        let k: String = k.extract()?;
        if !f.is_callable() {
            return Err(type_err(format!("models {k} must be callable")));
        }
        let doc = f
            .getattr("__doc__")
            .ok()
            .and_then(|d| d.extract::<String>().ok())
            .unwrap_or_default();
        set.add_user(
            &k,
            Arc::new(PyCoating {
                name: k.clone(),
                func: f.clone().unbind(),
                doc,
                tables: Mutex::new(HashMap::new()),
            }),
        );
    }
    check_coatings(&set).map_err(err)?;
    let names = set.names();
    if names.is_empty() {
        return Err(value_err(format!("coating catalog {name} has no coatings")));
    }
    lensbench_core::ext::update_default(|e| {
        for c in &names {
            if let Some(o) = e
                .find_coating(c)
                .filter(|o| !o.name.eq_ignore_ascii_case(&name))
            {
                return Err(value_err(format!(
                    "coating {c} is in both coating catalogs {} and {name}; give one of them another name",
                    o.name
                )));
            }
        }
        e.add_coating_catalog(&name, set);
        Ok(names.len())
    })
}

/// Remove a registered coating catalog (True if there was one)
#[pyfunction]
fn unregister_coating_catalog(name: &str) -> bool {
    lensbench_core::ext::update_default(|e| e.remove_coating_catalog(name))
}

/// Registered coating catalogs: {catalog name: [coating names]}
#[pyfunction]
fn coating_catalogs() -> std::collections::BTreeMap<String, Vec<String>> {
    let e = lensbench_core::ext::default_ext();
    e.coating_catalogs()
        .into_iter()
        .filter_map(|c| {
            let names = e.coating_catalog(&c)?.set().names();
            Some((c, names))
        })
        .collect()
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(register_coating_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_coating_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(coating_catalogs, m)?)?;
    Ok(())
}
