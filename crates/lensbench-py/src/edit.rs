//! 系の編集の Python 側の変換: 面の値の dict、開口、ピックアップの量の名前。

use lensbench_core::edit::SurfValue;
use lensbench_core::param::PickKind;
use lensbench_core::surface::Aperture;
use lensbench_core::system::FieldCond;
use lensbench_core::System;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyString, PyTuple};

/// 開口の dict: {"type": "circular", "min": 0, "max": 3, "dx": 0, "dy": 0}（type は circular・rect・ellipse・
/// obsc・spider（width・arms）・floating）。None は開口なし
pub(crate) fn aperture_from(v: &Bound<'_, PyAny>) -> PyResult<Aperture> {
    if v.is_none() {
        return Ok(Aperture::None);
    }
    let d = v.cast::<PyDict>()?;
    let get = |k: &str| -> PyResult<f64> {
        match d.get_item(k)? {
            Some(x) => x.extract(),
            None => Ok(0.0),
        }
    };
    let need = |k: &str| -> PyResult<f64> {
        match d.get_item(k)? {
            Some(x) => x.extract(),
            None => Err(PyValueError::new_err(format!("開口に {k} が要る"))),
        }
    };
    let typ: String = match d.get_item("type")? {
        Some(t) => t.extract()?,
        None => return Err(PyValueError::new_err("開口に type が要る")),
    };
    let (dx, dy) = (get("dx")?, get("dy")?);
    Ok(match typ.to_ascii_lowercase().as_str() {
        "none" => Aperture::None,
        "circular" => Aperture::Circular {
            min: get("min")?,
            max: need("max")?,
            dx,
            dy,
        },
        "obsc" => Aperture::Obsc {
            min: get("min")?,
            max: need("max")?,
            dx,
            dy,
        },
        "rect" => Aperture::Rect {
            hx: need("hx")?,
            hy: need("hy")?,
            dx,
            dy,
        },
        "ellipse" => Aperture::Ellipse {
            hx: need("hx")?,
            hy: need("hy")?,
            dx,
            dy,
        },
        "spider" => {
            let n = need("arms")?;
            if !(n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64) {
                return Err(PyValueError::new_err(format!("スパイダーの腕の本数 {n}")));
            }
            Aperture::Spider {
                width: need("width")?,
                arms: n as u32,
                dx,
                dy,
            }
        }
        "floating" => Aperture::Floating { semi: get("semi")? },
        t => {
            return Err(PyValueError::new_err(format!(
                "開口の type {t}（none circular obsc rect ellipse spider floating）"
            )))
        }
    })
}

pub(crate) fn aperture_dict<'py>(py: Python<'py>, a: &Aperture) -> PyResult<Bound<'py, PyAny>> {
    let d = PyDict::new(py);
    match *a {
        Aperture::None => return Ok(py.None().into_bound(py)),
        Aperture::Circular { min, max, dx, dy } | Aperture::Obsc { min, max, dx, dy } => {
            let t = if matches!(a, Aperture::Circular { .. }) {
                "circular"
            } else {
                "obsc"
            };
            d.set_item("type", t)?;
            d.set_item("min", min)?;
            d.set_item("max", max)?;
            d.set_item("dx", dx)?;
            d.set_item("dy", dy)?;
        }
        Aperture::Rect { hx, hy, dx, dy } | Aperture::Ellipse { hx, hy, dx, dy } => {
            let t = if matches!(a, Aperture::Rect { .. }) {
                "rect"
            } else {
                "ellipse"
            };
            d.set_item("type", t)?;
            d.set_item("hx", hx)?;
            d.set_item("hy", hy)?;
            d.set_item("dx", dx)?;
            d.set_item("dy", dy)?;
        }
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => {
            d.set_item("type", "spider")?;
            d.set_item("width", width)?;
            d.set_item("arms", arms)?;
            d.set_item("dx", dx)?;
            d.set_item("dy", dy)?;
        }
        Aperture::Floating { semi } => {
            d.set_item("type", "floating")?;
            d.set_item("semi", semi)?;
        }
    }
    Ok(d.into_any())
}

/// キーワード引数を面の値の組に（真偽・文字列・数・数の並び、aperture は dict か None）
pub(crate) fn surf_kwargs(kw: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, SurfValue)>> {
    let mut out = Vec::new();
    let Some(kw) = kw else { return Ok(out) };
    for (k, v) in kw.iter() {
        let name: String = k.extract()?;
        let val = if name == "aperture" {
            SurfValue::Aperture(aperture_from(&v)?)
        } else if v.is_instance_of::<PyBool>() {
            SurfValue::Bool(v.extract()?)
        } else if v.is_instance_of::<PyString>() {
            SurfValue::Text(v.extract()?)
        } else if let Ok(x) = v.extract::<f64>() {
            SurfValue::Num(x)
        } else if let Ok(x) = v.extract::<Vec<f64>>() {
            SurfValue::Nums(x)
        } else {
            return Err(PyValueError::new_err(format!("面の値 {name} の型")));
        };
        out.push((name, val));
    }
    Ok(out)
}

pub(crate) fn fields_dict<'py>(
    py: Python<'py>,
    f: Vec<(&'static str, SurfValue)>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in f {
        match v {
            SurfValue::Num(x) => d.set_item(k, x)?,
            SurfValue::Nums(x) => d.set_item(k, x)?,
            SurfValue::Text(x) => d.set_item(k, x)?,
            SurfValue::Bool(x) => d.set_item(k, x)?,
            SurfValue::Aperture(a) => d.set_item(k, aperture_dict(py, &a)?)?,
        }
    }
    Ok(d)
}

/// ピックアップの量: c（曲率）・t（厚さ）・k（円錐係数）・p1〜（面タイプのパラメータ）・dn・disp・semi
pub(crate) fn pick_kind(s: &str) -> PyResult<PickKind> {
    lensbench_core::edit::pick_kind_parse(s).map_err(|e| PyValueError::new_err(e.to_string()))
}

pub(crate) fn pick_name(k: PickKind) -> String {
    lensbench_core::edit::pick_kind_name(k)
}

/// 視野の条件: ("position", 面) / ("dirsum", 面1, 面2) / ("dirdiff", 面1, 面2)
pub fn field_cond(s: &System, c: &Bound<'_, PyTuple>) -> PyResult<FieldCond> {
    let kind: String = c.get_item(0)?.extract()?;
    let at = |i: usize| -> PyResult<usize> { crate::surf_of(s, c.get_item(i)?.extract()?) };
    Ok(match (kind.to_ascii_lowercase().as_str(), c.len()) {
        ("position", 2) => FieldCond::Position { surf: at(1)? },
        ("dirsum", 3) => FieldCond::DirSum {
            s1: at(1)?,
            s2: at(2)?,
        },
        ("dirdiff", 3) => FieldCond::DirDiff {
            s1: at(1)?,
            s2: at(2)?,
        },
        _ => {
            return Err(PyValueError::new_err(
                "cond は (\"position\", 面)、(\"dirsum\", 面1, 面2)、(\"dirdiff\", 面1, 面2)",
            ))
        }
    })
}
