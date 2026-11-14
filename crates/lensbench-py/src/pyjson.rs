//! Python の値 ⇄ serde の JSON の値の直接の変換（json モジュールを往復しない、CLI-20）。numpy の配列・
//! スカラー（`np.int64`・`np.float32` など）も、`tolist()` で Python の数・並びにして受ける

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use serde_json::{Map, Number, Value};

/// Python の値 → JSON の値。dict のキーは文字列だけ。NaN・inf は JSON に書けないので誤り（設定の値）
pub fn from_py(v: &Bound<'_, PyAny>) -> PyResult<Value> {
    conv(v, false)
}

/// [`from_py`] で、NaN・inf を null にする（一括実行の結果など、失敗した値を残したいとき）
pub fn from_py_nan_null(v: &Bound<'_, PyAny>) -> PyResult<Value> {
    conv(v, true)
}

fn conv(v: &Bound<'_, PyAny>, nan_null: bool) -> PyResult<Value> {
    if v.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = v.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if v.is_instance_of::<PyInt>() {
        if let Ok(i) = v.extract::<i64>() {
            return Ok(Value::from(i));
        }
        if let Ok(u) = v.extract::<u64>() {
            return Ok(Value::from(u));
        }
        return Err(PyValueError::new_err(format!(
            "整数 {v} は 64 ビットに入らない"
        )));
    }
    if v.is_instance_of::<PyFloat>() {
        let x: f64 = v.extract()?;
        return match Number::from_f64(x) {
            Some(n) => Ok(Value::Number(n)),
            None if nan_null => Ok(Value::Null),
            None => Err(PyValueError::new_err(format!(
                "{x} は JSON に書けない（NaN・inf）"
            ))),
        };
    }
    if let Ok(s) = v.cast::<PyString>() {
        return Ok(Value::String(s.to_string()));
    }
    if let Ok(d) = v.cast::<PyDict>() {
        let mut m = Map::new();
        for (k, x) in d.iter() {
            m.insert(key(&k)?, conv(&x, nan_null)?);
        }
        return Ok(Value::Object(m));
    }
    if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
        return v
            .try_iter()?
            .map(|x| conv(&x?, nan_null))
            .collect::<PyResult<Vec<_>>>()
            .map(Value::Array);
    }
    // numpy の配列・スカラー（tolist で Python の並び・数になる）
    if v.hasattr("tolist")? {
        return conv(&v.call_method0("tolist")?, nan_null);
    }
    Err(PyTypeError::new_err(format!(
        "{} は JSON にできない（None・bool・数・文字列・list・tuple・dict・numpy の配列）",
        v.get_type().name()?
    )))
}

/// dict のキーの文字列（json.dumps と同じ: 文字列はそのまま、bool は true・false、None は null、整数は 10 進、
/// 小数は repr（NaN・Infinity・-Infinity）。numpy のスカラーは tolist で Python の数にしてから）
fn key(k: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(s) = k.cast::<PyString>() {
        return Ok(s.to_string());
    }
    if k.is_none() {
        return Ok("null".into());
    }
    if let Ok(b) = k.cast::<PyBool>() {
        return Ok(if b.is_true() { "true" } else { "false" }.into());
    }
    if k.is_instance_of::<PyInt>() {
        return Ok(k.str()?.to_string());
    }
    if k.is_instance_of::<PyFloat>() {
        let x: f64 = k.extract()?;
        return Ok(if x.is_nan() {
            "NaN".into()
        } else if x.is_infinite() {
            if x > 0.0 { "Infinity" } else { "-Infinity" }.into()
        } else {
            k.repr()?.to_string()
        });
    }
    if k.hasattr("tolist")? {
        let v = k.call_method0("tolist")?;
        if !v.is(k) && !v.is_instance_of::<PyList>() {
            return key(&v);
        }
    }
    Err(PyTypeError::new_err(format!(
        "dict のキー {k} は文字列・数・bool・None だけ"
    )))
}

/// JSON の文字列（ファイルに書いたもの）→ Python の値。旧版が書いた NaN・Infinity（JSON でない）も
/// 読めるよう、serde で読めなければ json.loads で読む
pub fn loads<'py>(py: Python<'py>, t: &str) -> PyResult<Bound<'py, PyAny>> {
    match serde_json::from_str::<Value>(t) {
        Ok(v) => to_py(py, &v),
        Err(_) => py.import("json")?.call_method1("loads", (t,)),
    }
}

/// JSON の値 → Python の値（数は int か float、オブジェクトは dict）
pub fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any()
            } else if let Some(u) = n.as_u64() {
                u.into_pyobject(py)?.into_any()
            } else {
                n.as_f64().unwrap_or(f64::NAN).into_pyobject(py)?.into_any()
            }
        }
        Value::String(s) => PyString::new(py, s).into_any(),
        Value::Array(a) => {
            let l = PyList::empty(py);
            for x in a {
                l.append(to_py(py, x)?)?;
            }
            l.into_any()
        }
        Value::Object(o) => {
            let d = PyDict::new(py);
            for (k, x) in o {
                d.set_item(k, to_py(py, x)?)?;
            }
            d.into_any()
        }
    })
}

/// オブジェクトのキーを名前の順に並べ直す（json.dumps の sort_keys と同じ。serde_json の preserve_order が
/// 有効でも同じ並びにする）
pub fn sorted(v: Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut kv: Vec<(String, Value)> = o.into_iter().collect();
            kv.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(kv.into_iter().map(|(k, x)| (k, sorted(x))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        x => x,
    }
}
