//! 利用者定義の光源（`lensbench_core::usersource`。拡張が足したもの）を Python から名前で使う口と、Python の関数を
//! 光源・形（`lensbench_core::usershape`）として登録する口（試作用）。
//!
//! - `lb.source_models()` → {名前: 説明}
//! - `lb.source_rays(system, name, config=None, wave=None, frame="object")` → 始点・方向・重みの配列の dict。
//!   start・direction をそのまま `system.trace_rays(start, direction, waves=[wave], frame=frame)` に渡せる
//! - `lb.register_source(name, func)`: `func(config: dict, wl: float) -> dict`（start・direction・weight の配列）を光源として
//!   登録する（試作用。Python のプロセスの中だけで、CLI・GUI からは使えない）。波長ごとに 1 回、まとめて呼ぶ
//! - `lb.register_shape(name, func, params=None)`: `func(config, x, y) -> (z, dzdx, dzdy)` を形として登録する（試作用）。
//!   形を準備するときに半径 `radius` の内の格子（`grid` × `grid` 点）で 1 回呼び、値と勾配の双 3 次 Hermite で引く

use std::sync::Arc;

use lensbench_core::plugin::{Config, ConfigReader, ParamSpec, ParamType, Value};
use lensbench_core::usershape::{ShapeInstance, ShapeModel};
use lensbench_core::usersource::{SourceInstance, SourceModel, SourceRays};
use lensbench_core::Error;
use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::{type_err, value_err};
use crate::plugin::{config_to_py, to_config};
use crate::{err, wave_index, PySystem};

/// A source made of a Python function (for prototypes). Takes the GIL once per wavelength
struct PySource {
    name: String,
    func: Py<PyAny>,
    doc: String,
}

struct PySourceInstance {
    func: Py<PyAny>,
    config: Config,
}

impl SourceModel for PySource {
    fn name(&self) -> &str {
        &self.name
    }
    fn doc(&self) -> String {
        self.doc.clone()
    }
    fn prepare(&self, config: &Config) -> lensbench_core::Result<Arc<dyn SourceInstance>> {
        let func = Python::attach(|py| self.func.clone_ref(py));
        Ok(Arc::new(PySourceInstance {
            func,
            config: config.clone(),
        }))
    }
}

impl SourceInstance for PySourceInstance {
    fn rays(&self, wl: f64) -> lensbench_core::Result<SourceRays> {
        Python::attach(|py| -> PyResult<SourceRays> {
            let cfg = config_to_py(py, &self.config)?;
            let r = self.func.bind(py).call1((cfg, wl))?;
            let d = r
                .cast::<PyDict>()
                .map_err(|_| type_err("a source function must return a dict"))?;
            let np = py.import("numpy")?;
            let get = |k: &str| -> PyResult<Bound<'_, PyAny>> {
                let v = d
                    .get_item(k)?
                    .ok_or_else(|| value_err(format!("a source function must return {k}")))?;
                np.call_method1("ascontiguousarray", (v, "float64"))
            };
            let rows = |k: &str| -> PyResult<Vec<[f64; 3]>> {
                let a = get(k)?;
                let a: PyReadonlyArray2<'_, f64> = a.extract().map_err(|_| {
                    value_err(format!(
                        "{k} of a source function must have the shape (rays, 3)"
                    ))
                })?;
                let a = a.as_array();
                if a.ncols() != 3 {
                    return Err(value_err(format!(
                        "{k} of a source function must have the shape (rays, 3) ({:?})",
                        a.shape()
                    )));
                }
                Ok(a.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
            };
            let w = get("weight")?;
            let w: PyReadonlyArray1<'_, f64> = w.extract().map_err(|_| {
                value_err("weight of a source function must have the shape (rays,)")
            })?;
            Ok(SourceRays {
                p: rows("start")?,
                k: rows("direction")?,
                w: w.as_array().to_vec(),
            })
        })
        .map_err(|e| Error::Invalid(format!("Python source: {e}")))
    }
}

/// Register a Python function as a user-defined source (replaces one with the same name; for prototypes: only inside this Python process,
/// not from the CLI or GUI). `func(config: dict, wl: float) -> dict` returns start and direction ((rays, 3), object coordinates) and weight ((rays,))
/// for the wavelength wl [µm]. It is called once per wavelength
#[pyfunction]
fn register_source(name: String, func: &Bound<'_, PyAny>) -> PyResult<()> {
    if !func.is_callable() {
        return Err(type_err("func must be callable"));
    }
    let doc = func
        .getattr("__doc__")
        .ok()
        .and_then(|d| d.extract::<String>().ok())
        .unwrap_or_default();
    let m = Arc::new(PySource {
        name,
        func: func.clone().unbind(),
        doc,
    });
    lensbench_core::ext::update_default(|e| {
        e.add_source_model(m);
    });
    Ok(())
}

/// Remove a registered source (True if there was one)
#[pyfunction]
fn unregister_source(name: &str) -> bool {
    lensbench_core::ext::update_default(|e| e.remove_source_model(name))
}

/// Registered user-defined source models: {name: description}
#[pyfunction]
fn source_models() -> std::collections::BTreeMap<String, String> {
    // source_rays と同じ表（今の文脈、無ければプロセスの既定）
    let e = lensbench_core::ext::current();
    e.source_models()
        .into_iter()
        .filter_map(|n| {
            let doc = e.source_model(&n).ok()?.doc();
            Some((n, doc))
        })
        .collect()
}

/// Rays of a registered user-defined source model name with the settings config (dict) at the wavelength number wave (primary wavelength if omitted).
/// frame is "object" (the object coordinate system, as trace_rays) or "global".
/// Returned dict: start and direction ((rays, 3); directions as the source gave them) and weight ((rays,)).
/// Pass start and direction to `system.trace_rays(start, direction, waves=[wave], frame=frame)`
#[pyfunction]
#[pyo3(signature = (system, name, config=None, wave=None, frame="object"))]
fn source_rays<'py>(
    py: Python<'py>,
    system: PyRef<'py, PySystem>,
    name: &str,
    config: Option<&Bound<'py, PyDict>>,
    wave: Option<usize>,
    frame: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let s = system.sys();
    let w = wave
        .map(|w| wave_index(&s, w))
        .transpose()?
        .unwrap_or(s.primary);
    let global = match frame.to_ascii_lowercase().as_str() {
        "object" => false,
        "global" => true,
        f => {
            return Err(value_err(format!(
                "frame must be \"object\" or \"global\" ({f})"
            )))
        }
    };
    let cfg = to_config(config)?;
    let r = py
        .detach(|| -> lensbench_core::Result<_> {
            let m = lensbench_core::ext::current().source_model(name)?;
            let inst = m.prepare(&cfg)?;
            if global {
                s.source_rays(inst.as_ref(), w)
            } else {
                let r = inst.rays(s.wavelengths[w])?;
                r.check()?;
                Ok(r)
            }
        })
        .map_err(err)?;
    let rows =
        |v: &[[f64; 3]]| Array2::from_shape_fn((v.len(), 3), |(i, c)| v[i][c]).into_pyarray(py);
    let d = PyDict::new(py);
    d.set_item("start", rows(&r.p))?;
    d.set_item("direction", rows(&r.k))?;
    d.set_item("weight", r.w.into_pyarray(py))?;
    Ok(d)
}

/// A shape made of a Python function (for prototypes): sampled once per preparation on a grid and interpolated
struct PyShape {
    name: String,
    func: Py<PyAny>,
    doc: String,
    /// 利用者の設定の項目と既定値
    params: Vec<(String, f64)>,
}

/// 格子の上の値と勾配（双 3 次 Hermite で引く）
struct GridShape {
    /// 格子の端 −r〜r、点の数 n、幅 h
    r: f64,
    n: usize,
    h: f64,
    /// 点ごとの z・∂z/∂x・∂z/∂y・∂²z/∂x∂y（行 = y、列 = x）
    v: Vec<[f64; 4]>,
}

impl ShapeModel for PyShape {
    fn name(&self) -> &str {
        &self.name
    }
    fn doc(&self) -> String {
        self.doc.clone()
    }
    fn params(&self) -> Vec<ParamSpec> {
        let mut v: Vec<ParamSpec> = self
            .params
            .iter()
            .map(|(k, d)| ParamSpec::new(k, ParamType::Num, Some(Value::Num(*d)), "", ""))
            .collect();
        v.push(ParamSpec::new(
            "radius",
            ParamType::Num,
            Some(Value::Num(10.0)),
            "mm",
            "Half width of the sampled grid",
        ));
        v.push(ParamSpec::new(
            "grid",
            ParamType::Num,
            Some(Value::Num(65.0)),
            "",
            "Points of the grid per side",
        ));
        v
    }
    fn prepare(&self, config: &Config) -> lensbench_core::Result<Arc<dyn ShapeInstance>> {
        let mut c = ConfigReader::new(config);
        let r = c.num("radius", 10.0)?;
        let n = c.num("grid", 65.0)?;
        if !(r > 0.0 && r.is_finite()) || !(5.0..=2049.0).contains(&n) || n.fract() != 0.0 {
            return Err(Error::Invalid(format!(
                "Python shape {} needs radius > 0 and grid a whole number from 5 to 2049 (radius {r}, grid {n})",
                self.name
            )));
        }
        let n = n as usize;
        let h = 2.0 * r / (n - 1) as f64;
        let cfg = config.clone();
        let zs = Python::attach(|py| -> PyResult<[Vec<f64>; 3]> {
            let mut x = Vec::with_capacity(n * n);
            let mut y = Vec::with_capacity(n * n);
            for i in 0..n {
                for j in 0..n {
                    x.push(-r + h * j as f64);
                    y.push(-r + h * i as f64);
                }
            }
            let out = self.func.bind(py).call1((
                config_to_py(py, &cfg)?,
                x.into_pyarray(py),
                y.into_pyarray(py),
            ))?;
            let np = py.import("numpy")?;
            let mut cols: Vec<Vec<f64>> = Vec::with_capacity(3);
            for k in 0..3 {
                let a = np.call_method1("broadcast_to", (out.get_item(k)?, n * n))?;
                let a = np.call_method1("ascontiguousarray", (a, "float64"))?;
                let a: PyReadonlyArray1<'_, f64> = a.extract()?;
                cols.push(a.as_array().to_vec());
            }
            let [z, zx, zy]: [Vec<f64>; 3] = cols
                .try_into()
                .map_err(|_| value_err("a shape function must return (z, dzdx, dzdy)"))?;
            Ok([z, zx, zy])
        })
        .map_err(|e| Error::Invalid(format!("Python shape {}: {e}", self.name)))?;
        let [z, zx, zy] = zs;
        if !z.iter().chain(&zx).chain(&zy).all(|v| v.is_finite()) {
            return Err(Error::Invalid(format!(
                "Python shape {} gave a value that is not finite on its grid",
                self.name
            )));
        }
        // 交差の微分は ∂z/∂x を y の向きに差分（端は片側）
        let at = |i: usize, j: usize| i * n + j;
        let v = (0..n)
            .flat_map(|i| (0..n).map(move |j| (i, j)))
            .map(|(i, j)| {
                let (a, b) = (i.saturating_sub(1), (i + 1).min(n - 1));
                let zxy = (zx[at(b, j)] - zx[at(a, j)]) / (h * (b - a) as f64);
                [z[at(i, j)], zx[at(i, j)], zy[at(i, j)], zxy]
            })
            .collect();
        Ok(Arc::new(GridShape { r, n, h, v }))
    }
}

impl ShapeInstance for GridShape {
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        // 格子の外は端のセルの多項式をそのまま延ばす
        let cell = |q: f64| {
            let u = (q + self.r) / self.h;
            let i = (u.floor().max(0.0) as usize).min(self.n - 2);
            (i, u - i as f64)
        };
        let ((j, u), (i, t)) = (cell(x), cell(y));
        // Hermite の基底と微分（[0, 1] の上）
        let hb = |s: f64| {
            let (s2, s3) = (s * s, s * s * s);
            (
                [2.0 * s3 - 3.0 * s2 + 1.0, -2.0 * s3 + 3.0 * s2],
                [s3 - 2.0 * s2 + s, s3 - s2],
                [6.0 * s2 - 6.0 * s, -6.0 * s2 + 6.0 * s],
                [3.0 * s2 - 4.0 * s + 1.0, 3.0 * s2 - 2.0 * s],
            )
        };
        let (a0, a1, da0, da1) = hb(u);
        let (b0, b1, db0, db1) = hb(t);
        let h = self.h;
        let (mut z, mut zx, mut zy) = (0.0, 0.0, 0.0);
        for di in 0..2 {
            for dj in 0..2 {
                let p = self.v[(i + di) * self.n + j + dj];
                // x の向きの基底（値・勾配の項）と、その u による微分
                let (ex, gx) = ([a0[dj], h * a1[dj]], [da0[dj], h * da1[dj]]);
                let (ey, gy) = ([b0[di], h * b1[di]], [db0[di], h * db1[di]]);
                let term = |fx: [f64; 2], fy: [f64; 2]| {
                    p[0] * fx[0] * fy[0]
                        + p[1] * fx[1] * fy[0]
                        + p[2] * fx[0] * fy[1]
                        + p[3] * fx[1] * fy[1]
                };
                z += term(ex, ey);
                zx += term(gx, ey) / h;
                zy += term(ex, gy) / h;
            }
        }
        (z, zx, zy)
    }
}

/// Register a Python function as a user-defined shape (surface type USERSHAPE; replaces one with the same name; for prototypes: only inside this
/// Python process, not from the CLI or GUI). `func(config: dict, x: ndarray, y: ndarray) -> (z, dzdx, dzdy)` gives the sag added to the conic and its
/// gradient. params is {name: default} of its numeric settings. When a shape is prepared the function is called once on a grid of `grid` × `grid`
/// points (setting, default 65) from −`radius` to `radius` mm (setting, default 10), and the sag is interpolated by bicubic Hermite
/// interpolation of the values and gradients (the settings are not variables)
#[pyfunction]
#[pyo3(signature = (name, func, params=None))]
fn register_shape(
    name: String,
    func: &Bound<'_, PyAny>,
    params: Option<std::collections::BTreeMap<String, f64>>,
) -> PyResult<()> {
    if !func.is_callable() {
        return Err(type_err("func must be callable"));
    }
    let params: Vec<(String, f64)> = params.unwrap_or_default().into_iter().collect();
    if let Some((k, _)) = params.iter().find(|(k, _)| k == "radius" || k == "grid") {
        return Err(value_err(format!("{k} is a setting of every Python shape")));
    }
    let doc = func
        .getattr("__doc__")
        .ok()
        .and_then(|d| d.extract::<String>().ok())
        .unwrap_or_default();
    let m = Arc::new(PyShape {
        name,
        func: func.clone().unbind(),
        doc,
        params,
    });
    lensbench_core::ext::update_default(|e| {
        e.add_shape_model(m);
    });
    Ok(())
}

/// Remove a registered shape (True if there was one)
#[pyfunction]
fn unregister_shape(name: &str) -> bool {
    lensbench_core::ext::update_default(|e| e.remove_shape_model(name))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(register_shape, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_shape, m)?)?;
    m.add_function(wrap_pyfunction!(source_models, m)?)?;
    m.add_function(wrap_pyfunction!(source_rays, m)?)?;
    m.add_function(wrap_pyfunction!(register_source, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_source, m)?)?;
    Ok(())
}
