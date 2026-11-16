//! ズーム軌跡（CLI の `zoom`・serve の `zoom`）

use crate::session::views::{ZoomPoint, ZoomView};
use lensbench_core::System;
pub use lensbench_design::dls_spec::{DlsChoice, ZoomParams};
use lensbench_optim::zoom;
use serde_json::{json, Value};

fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

/// 読んだ状態の系でズーム軌跡を解く（CLI の zoom・serve の zoom。設定は Python の zoom と同じ型）
pub fn zoom_systems(
    cs: &[System],
    names: &[String],
    a: &ZoomParams,
) -> lensbench_core::Result<ZoomView> {
    let sp = a.spec(cs, names)?;
    let q = sp.q;
    let r = zoom::solve(cs, &sp)?;
    let vnames: Vec<String> = sp.moving.iter().map(|&p| cs[0].param_name(p)).collect();
    let path = r
        .path
        .iter()
        .map(|p| {
            let mut extra = serde_json::Map::new();
            extra.insert(q.name().into(), num(p.value));
            for (k, nm) in vnames.iter().enumerate() {
                extra.insert(nm.clone(), json!(p.x[k]));
            }
            ZoomPoint {
                pos: p.pos,
                state: p.state.map(|k| names[k].clone()),
                target: p.target,
                merit: p.merit,
                stop: p.stop.as_str().into(),
                error: p.stop.error().map(String::from),
                iterations: p.iterations,
                extra,
            }
        })
        .collect();
    Ok(ZoomView {
        quantity: q.name().into(),
        merit_kind: a.merit.as_str().into(),
        vars: vnames,
        path,
        warnings: r.warnings,
        notes: Vec::new(),
    })
}
