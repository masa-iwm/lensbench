//! Python の例外: lensbench.Error（ValueError の子）と、core の誤りの種類ごとの子クラス

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

pyo3::create_exception!(
    lensbench,
    Error,
    PyValueError,
    "Base of lensbench errors (a child of ValueError, so except ValueError also catches it)"
);
pyo3::create_exception!(
    lensbench,
    InvalidError,
    Error,
    "Invalid value or configuration"
);
pyo3::create_exception!(
    lensbench,
    UnsupportedError,
    Error,
    "Uses an unsupported feature"
);
pyo3::create_exception!(lensbench, ReadError, Error, "Error reading a file");
pyo3::create_exception!(
    lensbench,
    CancelledError,
    Error,
    "The calculation was cancelled"
);

/// Where an error comes from (core errors become exceptions per kind, strings and the like [`Error`])
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

/// An error into a lensbench exception (core errors into classes per kind, others into the base [`Error`]; all are children of ValueError)
pub fn err<E: Into<ErrSrc>>(e: E) -> PyErr {
    match e.into() {
        ErrSrc::Core(e) => {
            let m = e.to_string();
            let pe = match e.base() {
                lensbench_core::Error::NotImplemented(_) => UnsupportedError::new_err(m),
                lensbench_core::Error::Io(_) => ReadError::new_err(m),
                lensbench_core::Error::Cancelled => CancelledError::new_err(m),
                _ => InvalidError::new_err(m),
            };
            // 機械で見分ける符号（lensbench_core::error::code、無ければ種類の名前）を例外の code に
            Python::attach(|py| {
                let _ = pe.value(py).setattr("code", e.code());
            });
            pe
        }
        // 文字列の誤りは英語で書いてあるので、表示の言語に訳す
        ErrSrc::Text(m) => Error::new_err(lensbench_core::i18n::tr(&m)),
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // 誤りの文・注意の訳の表（例外の文を訳す）
    lensbench_lang::register();
    let py = m.py();
    m.add("Error", py.get_type::<Error>())?;
    m.add("InvalidError", py.get_type::<InvalidError>())?;
    m.add("UnsupportedError", py.get_type::<UnsupportedError>())?;
    m.add("ReadError", py.get_type::<ReadError>())?;
    m.add("CancelledError", py.get_type::<CancelledError>())?;
    Ok(())
}

/// Make a Python built-in exception, translating the English message into the display language
pub(crate) fn value_err(m: impl AsRef<str>) -> PyErr {
    PyValueError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// IndexError of [`value_err`]
pub(crate) fn index_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyIndexError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// TypeError of [`value_err`]
pub(crate) fn type_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyTypeError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}

/// RuntimeError of [`value_err`]
pub(crate) fn runtime_err(m: impl AsRef<str>) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(lensbench_core::i18n::tr(m.as_ref()))
}
