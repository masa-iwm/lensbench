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
        let txt =
            std::fs::read_to_string(&path).map_err(lensbench_core::Error::io_at(path.display()))?;
        let mut p = lensbench_project::Project::from_json(&txt)?;
        if let Some(st) = &state {
            p = p.with_state(st)?;
        }
        let cat = LensCatalog::load_registered(&lens_catalogs)?;
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

/// Read the systems of several configurations of a multi-configuration .zmx (`System.load_configs`)
pub(crate) fn load_configs_of(
    py: Python<'_>,
    path: PathBuf,
    configs: Option<Vec<usize>>,
    glass_dirs: Option<Vec<PathBuf>>,
    coatings: Option<Vec<PathBuf>>,
    coating_file: Option<PathBuf>,
) -> PyResult<Vec<crate::PySystem>> {
    use crate::errors::value_err;
    use crate::PySystem;
    use lensbench_formats::zmx;
    if path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        return Err(value_err(
            "load_configs is only for .zmx (load project files per state with load(state=...))",
        ));
    }
    let opt = zmx::LoadOptions {
        catalog_dirs: glass_dirs.unwrap_or_default(),
        ..Default::default()
    };
    // 空の list は全部（None）と取り違えやすいので断る
    if configs.as_ref().is_some_and(|c| c.is_empty()) {
        return Err(value_err(
            "configs is empty (leave configs out for all configurations)",
        ));
    }
    let cfgs = configs.unwrap_or_default();
    let v = py
        .detach(|| zmx::load_configs(&path, &opt, &cfgs))
        .map_err(err)?;
    let coat = coatings.unwrap_or_else(crate::env_coatings);
    v.into_iter()
        .map(|mut s| {
            if coating_file.is_some() {
                s.edit_raw(|r| {
                    r.coating_file = coating_file.clone();
                    Ok(())
                })
                .map_err(err)?;
            }
            Ok(PySystem::new(s, coat.clone(), Default::default()))
        })
        .collect()
}

/// 膜を使う計算（偏光の追跡・干渉・系を渡したプラグインなど）で、系の膜の注意（同じ名前の膜のどれを使ったかなど、
/// [`lensbench_core::coating::Coatings::notes`]）を Python の警告にする。膜なし（None）なら何もしない
pub(crate) fn warn_coat(
    py: Python<'_>,
    coat: Option<&lensbench_core::coating::Coatings>,
) -> PyResult<()> {
    coat.map_or(Ok(()), |c| warn_all(py, &c.notes))
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
