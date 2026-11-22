//! 膜の一覧を Python から登録する（拡張の表の膜の一覧。Rust の `Extensions::coating_catalog` と同じ表に入る）

use std::path::PathBuf;

use lensbench_core::usercoating::{check_coatings, CoatingSet};
use pyo3::prelude::*;

use crate::err;
use crate::errors::value_err;

/// Register a coating catalog (replaces one with the same name). `files` are coating model (.lbcm, .lbcb), measured table (.lbct)
/// or coating file (COATING.DAT format: MATE, COAT, TABLE) files, `dat` is text in the COATING.DAT format. The coatings are looked up by
/// the surface's coating name before the coating file of the .zmx and the coating folder. Returns the number of coatings.
/// A coating with the same name in another registered catalog is an error. Register before loading the systems that use it
/// (a system reads its coatings once, at the first polarization trace)
#[pyfunction]
#[pyo3(signature = (name, files=Vec::new(), dat=None))]
fn register_coating_catalog(
    name: String,
    files: Vec<PathBuf>,
    dat: Option<String>,
) -> PyResult<usize> {
    let mut set = CoatingSet::new();
    for f in &files {
        set.add_file(f).map_err(err)?;
    }
    if let Some(t) = dat {
        let d = CoatingSet::parse_dat(&t).map_err(err)?;
        set.mate.extend(d.mate);
        set.coat.extend(d.coat);
        set.table.extend(d.table);
    }
    check_coatings(&set).map_err(err)?;
    let names = set.names();
    if names.is_empty() {
        return Err(value_err(format!("coating catalog {name} has no coatings")));
    }
    lensbench_core::ext::update_default(|e| {
        for c in &names {
            if let Some(o) = e
                .find_coating(c)
                .filter(|o| !o.name.eq_ignore_ascii_case(&name))
            {
                return Err(value_err(format!(
                    "coating {c} is in both coating catalogs {} and {name}; give one of them another name",
                    o.name
                )));
            }
        }
        e.add_coating_catalog(&name, set);
        Ok(names.len())
    })
}

/// Remove a registered coating catalog (True if there was one)
#[pyfunction]
fn unregister_coating_catalog(name: &str) -> bool {
    lensbench_core::ext::update_default(|e| e.remove_coating_catalog(name))
}

/// Registered coating catalogs: {catalog name: [coating names]}
#[pyfunction]
fn coating_catalogs() -> std::collections::BTreeMap<String, Vec<String>> {
    let e = lensbench_core::ext::default_ext();
    e.coating_catalogs()
        .into_iter()
        .filter_map(|c| {
            let names = e.coating_catalog(&c)?.set().names();
            Some((c, names))
        })
        .collect()
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(register_coating_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(unregister_coating_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(coating_catalogs, m)?)?;
    Ok(())
}
