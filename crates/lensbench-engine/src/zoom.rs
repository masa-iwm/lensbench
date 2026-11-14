//! ズーム軌跡（CLI の `zoom`・serve の `zoom`）

use lensbench_core::optim::{DefaultMerit, DlsOpts, ParaxQ, Stop};
use lensbench_core::sweep::RmsRef;
use lensbench_core::zoom::{self, ZoomSpec};
use lensbench_core::System;
use serde_json::{json, Value};

/// ズーム軌跡の設定（CLI の旗・serve の params と同じ意味）
#[derive(Clone, Debug)]
pub struct ZoomOpts {
    /// 状態ごとに解くパラメータ（"G2.shift"・"7.t"、範囲は "G2.shift=-5:20"）
    pub vars: Vec<String>,
    /// 状態ごとの焦点距離の目標（カンマで、状態の数だけ。"-" は目標なし）
    pub efl: Option<String>,
    /// 状態ごとの近軸倍率の目標
    pub pmag: Option<String>,
    /// 隣り合う状態の間にとる点の数
    pub steps: usize,
    /// 像の評価関数: spot か wavefront
    pub merit: String,
    /// 基準: centroid か chief
    pub reference: String,
    pub rings: usize,
    /// 1 点の反復の上限
    pub iters: usize,
    /// 目標との差の許し（目標に対する比）
    pub tol: f64,
}

fn stop_name(s: Stop) -> &'static str {
    s.as_str()
}

fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

fn parse_var(s: &System, v: &str) -> Result<(lensbench_core::param::Param, (f64, f64)), String> {
    let (spec, range) = match v.split_once('=') {
        Some((a, b)) => (a.trim(), Some(b)),
        None => (v.trim(), None),
    };
    let p = s.param(spec).map_err(|e| e.to_string())?;
    let mut b = (f64::NEG_INFINITY, f64::INFINITY);
    if let Some(r) = range {
        let (lo, hi) = r
            .split_once(':')
            .ok_or_else(|| format!("--var {v} の範囲は 下限:上限"))?;
        let num = |t: &str, d: f64| -> Result<f64, String> {
            if t.trim().is_empty() {
                Ok(d)
            } else {
                t.trim()
                    .parse()
                    .map_err(|_| format!("--var {v} の範囲 {t}"))
            }
        };
        b = (num(lo, b.0)?, num(hi, b.1)?);
    }
    Ok((p, b))
}

/// 読んだ状態の系でズーム軌跡を解く（CLI の zoom・serve の zoom）
pub fn zoom_systems(cs: &[System], names: &[String], a: &ZoomOpts) -> Result<Value, String> {
    let n = cs.len();
    let (q, txt) = match (&a.efl, &a.pmag) {
        (Some(t), _) => (ParaxQ::Efl, Some(t)),
        (None, Some(t)) => (ParaxQ::Pmag, Some(t)),
        (None, None) => (ParaxQ::Efl, None),
    };
    let targets: Vec<Option<f64>> = match txt {
        None => vec![None; n],
        Some(t) => t
            .split(',')
            .map(|x| {
                let x = x.trim();
                if x == "-" || x.is_empty() {
                    Ok(None)
                } else {
                    x.parse()
                        .map(Some)
                        .map_err(|_| format!("目標 {x} が読めない"))
                }
            })
            .collect::<Result<_, String>>()?,
    };
    if targets.len() != n {
        return Err(format!(
            "目標が {} 個（状態は {n} 個: {}）",
            targets.len(),
            names.join(", ")
        ));
    }
    let mut moving = Vec::new();
    let mut bounds = Vec::new();
    for v in &a.vars {
        let (p, b) = parse_var(&cs[0], v)?;
        moving.push(p);
        bounds.push(b);
    }
    let mut sp = ZoomSpec::new(moving, targets);
    sp.bounds = bounds;
    sp.q = q;
    sp.steps = a.steps;
    sp.rings = a.rings;
    sp.target_tol = a.tol;
    sp.merit = match a.merit.to_ascii_lowercase().as_str() {
        "spot" => DefaultMerit::Spot,
        "wavefront" | "wfe" => DefaultMerit::Wavefront,
        m => return Err(format!("--merit {m}（spot・wavefront）")),
    };
    sp.reference = match a.reference.to_ascii_lowercase().as_str() {
        "centroid" => RmsRef::Centroid,
        "chief" => RmsRef::Chief,
        r => return Err(format!("--ref {r}（centroid・chief）")),
    };
    sp.opts = DlsOpts {
        max_iter: a.iters,
        ..Default::default()
    };
    let r = zoom::solve(cs, &sp).map_err(|e| e.to_string())?;
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
                "stop": stop_name(p.stop),
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
        "merit_kind": a.merit,
        "vars": vnames,
        "path": path,
        "warnings": r.warnings,
    }))
}
