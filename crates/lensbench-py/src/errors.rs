//! Python の例外: lensbench.Error（ValueError の子）と、core の誤りの種類ごとの子クラス

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

pyo3::create_exception!(
    lensbench,
    Error,
    PyValueError,
    "lensbench の誤りの基底（ValueError の子なので except ValueError でも拾える）"
);
pyo3::create_exception!(lensbench, InvalidError, Error, "値や構成が不正");
pyo3::create_exception!(
    lensbench,
    UnsupportedError,
    Error,
    "対応していない機能を含む"
);
pyo3::create_exception!(lensbench, ReadError, Error, "ファイルの読込の誤り");
pyo3::create_exception!(lensbench, CancelledError, Error, "計算を途中でやめた");

/// 誤りの元（core の誤りは種類ごとの例外に、文字列などは [`Error`] に）
pub enum ErrSrc {
    Core(lensbench_core::Error),
    Text(String),
}

impl From<lensbench_core::Error> for ErrSrc {
    fn from(e: lensbench_core::Error) -> Self {
        ErrSrc::Core(e)
    }
}

impl From<String> for ErrSrc {
    fn from(e: String) -> Self {
        ErrSrc::Text(e)
    }
}

impl From<&str> for ErrSrc {
    fn from(e: &str) -> Self {
        ErrSrc::Text(e.to_string())
    }
}

impl From<&String> for ErrSrc {
    fn from(e: &String) -> Self {
        ErrSrc::Text(e.clone())
    }
}

impl From<serde_json::Error> for ErrSrc {
    fn from(e: serde_json::Error) -> Self {
        ErrSrc::Text(e.to_string())
    }
}

impl From<numpy::ndarray::ShapeError> for ErrSrc {
    fn from(e: numpy::ndarray::ShapeError) -> Self {
        ErrSrc::Text(e.to_string())
    }
}

/// 誤りを lensbench の例外に（core の誤りは種類ごとのクラス、ほかは基底の [`Error`]。どれも ValueError の子）
pub fn err<E: Into<ErrSrc>>(e: E) -> PyErr {
    match e.into() {
        ErrSrc::Core(e) => {
            let m = e.to_string();
            match e {
                lensbench_core::Error::Invalid(_) => InvalidError::new_err(m),
                lensbench_core::Error::NotImplemented(_) => UnsupportedError::new_err(m),
                lensbench_core::Error::Io(_) => ReadError::new_err(m),
                lensbench_core::Error::Cancelled => CancelledError::new_err(m),
            }
        }
        // 文字列の誤りは英語で書いてあるので、表示の言語に訳す（XCT-20）
        ErrSrc::Text(m) => Error::new_err(lensbench_core::i18n::tr(&m)),
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("Error", py.get_type::<Error>())?;
    m.add("InvalidError", py.get_type::<InvalidError>())?;
    m.add("UnsupportedError", py.get_type::<UnsupportedError>())?;
    m.add("ReadError", py.get_type::<ReadError>())?;
    m.add("CancelledError", py.get_type::<CancelledError>())?;
    Ok(())
}

/// Python の組み込みの例外を、英語の文を表示の言語に訳して作る（XCT-20）
pub fn value_err(m: impl AsRef<str>) -> PyErr {
    PyValueError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// [`value_err`] の IndexError
pub fn index_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyIndexError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// [`value_err`] の TypeError
pub fn type_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyTypeError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// [`value_err`] の RuntimeError
pub fn runtime_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}
