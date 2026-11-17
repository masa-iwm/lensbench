//! 系の編集の Python 側の変換: 面の値の dict、開口、ピックアップの量の名前。

use crate::errors::value_err;
use lensbench_core::edit::SurfValue;
use lensbench_core::param::Kind;
use lensbench_core::surface::Aperture;
use lensbench_core::system::FieldCond;
use lensbench_core::System;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyString, PyTuple};

/// An aperture dict: {"type": "circular", "min": 0, "max": 3, "dx": 0, "dy": 0} (type is circular, rect, ellipse,
/// obsc, spider (width, arms), floating). None for no aperture
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
            None => Err(value_err(format!("the aperture needs {k}"))),
        }
    };
    let typ: String = match d.get_item("type")? {
        Some(t) => t.extract()?,
        None => return Err(value_err("the aperture needs type")),
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
                return Err(value_err(format!("number of spider arms {n}")));
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
            return Err(value_err(format!(
                "aperture type {t} (none circular obsc rect ellipse spider floating)"
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

/// Keyword arguments into a set of surface values (bools, strings, numbers, lists of numbers; aperture is a dict or None)
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
            return Err(value_err(format!("type of surface value {name}")));
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

/// Pickup quantities: c (curvature), t (thickness), k (conic), p1… (parameters of the surface type), dn, disp, semi
pub(crate) fn pick_kind(s: &str) -> PyResult<Kind> {
    lensbench_core::pickup::pick_kind_parse(s).map_err(|e| value_err(e.to_string()))
}

pub(crate) fn pick_name(k: Kind) -> String {
    lensbench_core::pickup::pick_kind_name(k)
}

/// Field conditions: ("position", surface) / ("dirsum", surface1, surface2) / ("dirdiff", surface1, surface2)
pub(crate) fn field_cond(s: &System, c: &Bound<'_, PyTuple>) -> PyResult<FieldCond> {
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
            return Err(value_err(
                "cond must be (\"position\", surface), (\"dirsum\", surface1, surface2) or (\"dirdiff\", surface1, surface2)",
            ))
        }
    })
}
