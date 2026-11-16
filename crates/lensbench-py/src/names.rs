//! Python の警告（展開の注意）と、BS の経路の一覧

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use pyo3::prelude::*;

use crate::errors::err;

/// The beam splitter paths of a project file (`System.beam_paths`)
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
        let cat = LensCatalog::load_registered(&lens_catalogs).map_err(|e| e.to_string())?;
        let look = |v: &str, q: &str| cat.zmx(v, q);
        let env = lensbench_project::Env {
            step_files: None,
            base: path.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
            glass_dirs,
            catalog: Some(&look),
        };
        Ok(lensbench_project::paths(&p, &env)?)
    })
    .map_err(err)
}

/// Turn the notes of the expansion into Python warnings (UserWarning)
pub(crate) fn warn_all(py: Python<'_>, warn: &[String]) -> PyResult<()> {
    if warn.is_empty() {
        return Ok(());
    }
    let w = py.import("warnings")?;
    for m in warn {
        w.call_method1("warn", (lensbench_core::i18n::tr(m),))?;
    }
    Ok(())
}
