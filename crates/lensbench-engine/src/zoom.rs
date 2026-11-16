//! ズーム軌跡（CLI の `zoom`・serve の `zoom`）

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
) -> lensbench_core::Result<Value> {
    let sp = a.spec(cs, names)?;
    let q = sp.q;
    let r = zoom::solve(cs, &sp)?;
    let vnames: Vec<String> = sp.moving.iter().map(|&p| cs[0].param_name(p)).collect();
    let path: Vec<Value> = r
        .path
        .iter()
        .map(|p| {
            let mut o = json!({
                "pos": p.pos,
                "state": p.state.map(|k| names[k].clone()),
                "target": p.target,
                q.name(): num(p.value),
                "merit": num(p.merit),
                "stop": p.stop.as_str(),
                "error": p.stop.error(),
                "iterations": p.iterations,
            });
            for (k, nm) in vnames.iter().enumerate() {
                o[nm] = json!(p.x[k]);
            }
            o
        })
        .collect();
    Ok(json!({
        "quantity": q.name(),
        "merit_kind": a.merit.as_str(),
        "vars": vnames,
        "path": path,
        "warnings": r.warnings,
    }))
}
