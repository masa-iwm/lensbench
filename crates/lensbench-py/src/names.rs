//! Python の警告（展開の注意・旧名の DeprecationWarning、CLI-17）と、BS の経路の一覧

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use pyo3::prelude::*;

use crate::errors::{self, err};

/// プロジェクトファイルの BS の経路の一覧（`System.beam_paths`）
pub(crate) fn beam_paths_of(
    py: Python<'_>,
    path: PathBuf,
    glass_dirs: Option<Vec<PathBuf>>,
    state: Option<String>,
    lens_catalogs: Option<Vec<PathBuf>>,
) -> PyResult<Vec<String>> {
    let glass_dirs = glass_dirs.unwrap_or_default();
    let lens_catalogs = lens_catalogs.unwrap_or_default();
    py.detach(|| -> Result<Vec<String>, String> {
        let txt = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut p = lensbench_project::Project::from_json(&txt)?;
        if let Some(st) = &state {
            p = p.with_state(st)?;
        }
        let cat = LensCatalog::load_registered(&lens_catalogs)?;
        let look = |v: &str, q: &str| cat.zmx(v, q);
        let env = lensbench_project::Env {
            base: path.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
            glass_dirs,
            catalog: Some(&look),
        };
        lensbench_project::paths(&p, &env)
    })
    .map_err(err)
}

/// 新しい名前か旧名の引数（CLI-17 で名前をそろえた。旧名は DeprecationWarning、両方は誤り）
pub(crate) fn renamed<T>(
    py: Python<'_>,
    new: Option<T>,
    old: Option<T>,
    nn: &str,
    on: &str,
) -> PyResult<Option<T>> {
    match (new, old) {
        (Some(_), Some(_)) => Err(errors::InvalidError::new_err(format!(
            "{nn} と {on}（旧名）の両方は渡せない"
        ))),
        (None, Some(o)) => {
            deprecated(py, &format!("{on} は {nn} の旧名（次の major 版で消す）"))?;
            Ok(Some(o))
        }
        (n, None) => Ok(n),
    }
}

/// DeprecationWarning を出す（呼んだ Python の行を指す）
pub(crate) fn deprecated(py: Python<'_>, msg: &str) -> PyResult<()> {
    let m = std::ffi::CString::new(msg).unwrap_or_default();
    PyErr::warn(
        py,
        &py.get_type::<pyo3::exceptions::PyDeprecationWarning>(),
        &m,
        1,
    )
}

/// 展開の説明を Python の警告（UserWarning）にする
pub(crate) fn warn_all(py: Python<'_>, warn: &[String]) -> PyResult<()> {
    if warn.is_empty() {
        return Ok(());
    }
    let w = py.import("warnings")?;
    for m in warn {
        w.call_method1("warn", (m.as_str(),))?;
    }
    Ok(())
}
