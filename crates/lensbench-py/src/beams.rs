//! プロジェクトの光束（`beams`）: 一覧・光束ごとの系・光束の表（`System.beams`・`System.load_beams`・`System.beam_table`）

use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use serde_json::{json, Value};

use crate::errors::err;

/// プロジェクトファイルを読む（光束の名前の誤りも確かめる）
fn project(path: &Path) -> lensbench_core::Result<lensbench_project::Project> {
    let txt =
        std::fs::read_to_string(path).map_err(lensbench_core::Error::io_at(path.display()))?;
    let p = lensbench_project::Project::from_json(&txt)?;
    p.check_beams()?;
    Ok(p)
}

/// 光束の一覧（ファイルに書いたままの欄）
pub(crate) fn beams_of(py: Python<'_>, path: PathBuf) -> PyResult<Bound<'_, PyAny>> {
    let p = py.detach(|| project(&path)).map_err(err)?;
    let v: Vec<Value> = p
        .beams
        .iter()
        .map(|b| {
            json!({
                "name": b.name, "path": b.path, "state": b.state, "role": b.role,
                "turn": b.turn, "off": b.off,
            })
        })
        .collect();
    crate::pyjson::to_py(py, &Value::from(v))
}

/// 読む光束の名前: names を与えればその順（無い名前は誤り）、なければ off でない光束（all なら全部）
pub(crate) fn beam_names(
    path: &Path,
    names: Option<Vec<String>>,
    all: bool,
) -> lensbench_core::Result<Vec<String>> {
    let p = project(path)?;
    if p.beams.is_empty() {
        return Err(lensbench_core::Error::Invalid(
            "the project has no beams".into(),
        ));
    }
    match names {
        Some(v) if v.is_empty() => Err(lensbench_core::Error::Invalid(
            "beams is empty (leave beams out for the beams that are not off)".into(),
        )),
        Some(v) => {
            for n in &v {
                if p.beam(n).is_none() {
                    return Err(lensbench_core::Error::Invalid(format!(
                        "no beam {n} ({})",
                        p.beam_names().join(", ")
                    ))
                    .with_code(lensbench_core::error::code::STATE_NOT_FOUND));
                }
            }
            Ok(v)
        }
        None => Ok(p
            .beams
            .iter()
            .filter(|b| all || !b.off)
            .map(|b| b.name.clone())
            .collect()),
    }
}

/// 光束の表の設定
pub(crate) struct TableArgs {
    pub field: usize,
    pub wave: Option<usize>,
    pub grid: usize,
    pub reference: Option<String>,
    pub all: bool,
    pub glass_dirs: Vec<PathBuf>,
    pub lens_catalogs: Vec<PathBuf>,
    pub threads: Option<usize>,
}

/// 光束の表（解析 `beams`、CLI の `lensbench beams` と同じ）
pub(crate) fn beam_table(
    py: Python<'_>,
    path: PathBuf,
    a: TableArgs,
) -> PyResult<Bound<'_, PyAny>> {
    let reg = lensbench_engine::analyses();
    let an = reg
        .get("beams")
        .cloned()
        .ok_or_else(|| crate::errors::value_err("the beams analysis is not registered"))?;
    let params = json!({
        "field": a.field, "wave": a.wave, "grid": a.grid, "reference": a.reference, "all": a.all,
    });
    let spec = lensbench_engine::SysSpec {
        file: path,
        glass_dir: a.glass_dirs,
        lens_catalog: a.lens_catalogs,
        cfg: 1,
        names: lensbench_project::open::ArgNames::PYTHON,
        ..Default::default()
    };
    let v = py
        .detach(|| {
            lensbench_core::with_threads(a.threads, || -> lensbench_core::Result<Value> {
                let s = lensbench_engine::load::load_file(&spec)?;
                let src = lensbench_engine::source::FileSource::new(&spec, None);
                let cx = lensbench_analysis::Cx {
                    system: &s,
                    source: Some(&src),
                };
                an.run_json(&cx, &params)
            })
        })
        .map_err(err)?;
    crate::pyjson::to_py(py, &v)
}

/// 読み方の引数（glass_dirs, lens_catalogs, coatings, coating_file）
pub(crate) type LoadArgs = (
    Option<Vec<PathBuf>>,
    Option<Vec<PathBuf>>,
    Option<Vec<PathBuf>>,
    Option<PathBuf>,
);

/// 光束ごとの系（`System.load_beams`）: 光束の名前 → `load(path, state=名前)` の系
pub(crate) fn load_beams(
    py: Python<'_>,
    path: PathBuf,
    beams: Option<Vec<String>>,
    all: bool,
    o: LoadArgs,
) -> PyResult<Bound<'_, pyo3::types::PyDict>> {
    let names = py.detach(|| beam_names(&path, beams, all)).map_err(err)?;
    let d = pyo3::types::PyDict::new(py);
    for n in names {
        let s = crate::PySystem::load(
            py,
            path.clone(),
            o.0.clone(),
            1,
            Some(n.clone()),
            o.1.clone(),
            o.2.clone(),
            o.3.clone(),
            None,
            None,
        )?;
        d.set_item(n, Py::new(py, s)?)?;
    }
    Ok(d)
}
