//! OpticStudio の評価関数の Wizard（DMFS の行の後）が作る光線の行を、視野・構成ごとのスポット・波面の項にまとめる。
//!
//! Wizard は視野ごとに瞳の求積の点の光線の行（OPDX、TRAR・TRAC、TRAD・TRAE、TRCX・TRCY）を、求積の重み × 波長の重み ×
//! 視野の重み × Overall Wgt を重みにして並べる。行の重み付きの 2 乗和は、同じ点・重みで求めた RMS² に行の重みの和を
//! 掛けたものなので、視野ごとに重みが行の重みの和の項 1 つにすると評価関数の値が同じになる。x+y の行（TRAD と TRAE
//! など）は 1 本の光線で 2 行あり、重みの和も 2 回数えるので、x と y の 2 つの項にする（X Wgt・Y Wgt もそのまま）。
//! 求積の点の置き方は DMFS の次の BLNK の設定文（「GQ 3 rings 6 arms」・「RA 4 x 4 rays」）から読む。Wizard の設定は
//! .zmx にはこの文しか残らない（OpticStudio 26.1 で確かめた）。
//!
//! Ignore Lateral Color（波長ごとに PRIM の行で主波長を替える）は、波長ごとの単色の項にする。Assume Axial Symmetry の
//! Wizard は瞳の半分（軸上は腕 1 本）だけを並べるが、項は瞳全体の点を使う（回転対称な系では同じ値）。

use super::{data, is_length, INT2, TARGET, WEIGHT};
use lensbench_formats::zmx::merit::MeritRow;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// まとめる光線の演算子 → (波面か, 基準, 向き)
fn ray_op(op: &str) -> Option<(bool, &'static str, &'static str)> {
    Some(match op {
        "OPDX" => (true, "centroid", "xy"),
        "TRAR" => (false, "chief", "xy"),
        "TRAC" => (false, "centroid", "xy"),
        "TRAD" => (false, "chief", "x"),
        "TRAE" => (false, "chief", "y"),
        "TRCX" => (false, "centroid", "x"),
        "TRCY" => (false, "centroid", "y"),
        _ => return None,
    })
}

/// 設定文（「Sequential merit function: RMS spot x+y chief … RA 4 x 4 rays」）から求積の点の置き方のキー
fn quadrature(text: &str) -> Result<Map<String, Value>, String> {
    let body = text
        .strip_prefix("Sequential merit function:")
        .ok_or("no Wizard settings")?;
    let what = body.trim();
    let w: Vec<&str> = body.split_whitespace().collect();
    if w.first() != Some(&"RMS") {
        return Err(format!(
            "the Wizard's \"{what}\" is not supported (only RMS)"
        ));
    }
    let n = |i: usize| w.get(i).and_then(|s| s.parse::<usize>().ok());
    let mut m = Map::new();
    for i in 0..w.len() {
        match w[i] {
            "GQ" if w.get(i + 2) == Some(&"rings") && w.get(i + 4) == Some(&"arms") => {
                let (Some(r), Some(a)) = (n(i + 1), n(i + 3)) else {
                    break;
                };
                m.insert("quadrature".into(), json!("opticstudio"));
                m.insert("rings".into(), json!(r));
                m.insert("arms".into(), json!(a));
                return Ok(m);
            }
            "RA" if w.get(i + 2) == Some(&"x") => {
                let Some(k) = n(i + 1) else { break };
                m.insert("quadrature".into(), json!("grid"));
                m.insert("n".into(), json!(k));
                return Ok(m);
            }
            _ => {}
        }
    }
    Err(format!(
        "can't read the pupil sampling of the Wizard's \"{what}\""
    ))
}

/// まとめた結果: 行ごとにまとめたか、まとめた項を置く行（まとまりの最初の行）の項、注意
pub(super) struct Wizard {
    pub used: Vec<bool>,
    pub terms: BTreeMap<usize, Vec<Map<String, Value>>>,
    pub notes: Vec<String>,
}

/// まとまり 1 つ（構成・視野・波長（単色にするとき）・演算子が同じ行）
struct Group {
    first: usize,
    weight: f64,
    m: Map<String, Value>,
    /// 行の番号と (波長, Px, Py, 重み)
    rows: Vec<(usize, i64, f64, f64, f64)>,
}

/// 設定文の求積の瞳全体の点 (px, py, 重み)。GQ は r² の Gauss-Legendre の輪 × 0° からの腕、RA は n × n の升の中心
fn points(q: &Map<String, Value>) -> Vec<(f64, f64, f64)> {
    let get = |k: &str| q.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    let mut p = Vec::new();
    if q.get("quadrature") == Some(&json!("grid")) {
        let n = get("n");
        let c = |i: usize| (2 * i + 1) as f64 / n as f64 - 1.0;
        for i in 0..n {
            for k in 0..n {
                if c(k) * c(k) + c(i) * c(i) <= 1.0 + 1e-12 {
                    p.push((c(k), c(i), 1.0));
                }
            }
        }
    } else {
        let (rings, arms) = (get("rings"), get("arms"));
        let (u, wu) = lensbench_analysis::analysis::gauss_legendre(rings);
        for i in 0..rings {
            let r = ((u[i] + 1.0) / 2.0).sqrt();
            for a in 0..arms {
                let t = a as f64 * std::f64::consts::TAU / arms as f64;
                p.push((r * t.cos(), r * t.sin(), wu[i]));
            }
        }
    }
    p
}

/// 1 つの波長の行の点 (px, py, 重み) が、求積の点（瞳全体、x の正の半分（Assume Axial Symmetry）、軸上の
/// 視野で回転対称を使った 1 本の腕（GQ）・4 分の 1（RA））のどれかと、点も重みの比も同じか
fn matches(rows: &[(f64, f64, f64)], full: &[(f64, f64, f64)], grid: bool, on_axis: bool) -> bool {
    let e = 1e-9;
    let half: Vec<_> = full
        .iter()
        .filter(|p| p.0 > -e)
        .map(|&(x, y, w)| (x, y, if x > e { 2.0 * w } else { w }))
        .collect();
    let axis: Vec<_> = full
        .iter()
        .filter(|_| on_axis)
        .filter(|p| {
            if grid {
                p.0 > e && p.1 > e
            } else {
                p.0 > e && p.1.abs() < e
            }
        })
        .copied()
        .collect();
    let norm = |v: &[(f64, f64, f64)]| {
        let sw: f64 = v.iter().map(|p| p.2).sum();
        let mut v: Vec<_> = v.iter().map(|&(x, y, w)| (x, y, w / sw)).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    };
    let r = norm(rows);
    [full.to_vec(), half, axis].iter().any(|c| {
        let c = norm(c);
        c.len() == r.len()
            && c.iter().zip(&r).all(|(a, b)| {
                (a.0 - b.0).abs() < 1e-6
                    && (a.1 - b.1).abs() < 1e-6
                    && (a.2 - b.2).abs() < 1e-6 * a.2
            })
    })
}

/// rows の Wizard の光線の行をまとめる。row_config は行ごとの構成、unit はレンズ単位 [mm]
pub(super) fn group(rows: &[MeritRow], row_config: &[i64], unit: f64) -> Wizard {
    let mut out = Wizard {
        used: vec![false; rows.len()],
        terms: BTreeMap::new(),
        notes: Vec::new(),
    };
    let mut d = 0;
    while d < rows.len() {
        if rows[d].op != "DMFS" {
            d += 1;
            continue;
        }
        let end = (d + 1..rows.len())
            .find(|&k| rows[k].op == "DMFS")
            .unwrap_or(rows.len());
        let settings = rows.get(d + 1).filter(|r| r.op == "BLNK");
        match settings.map(|r| quadrature(&r.comment)) {
            Some(Ok(q)) => block(rows, row_config, unit, d + 2..end, &q, &mut out),
            Some(Err(e)) if settings.is_some_and(|r| r.comment.starts_with("Sequential")) => {
                out.notes.push(format!(
                    ".zmx merit function row {}: {e}; its rows are read one by one",
                    rows[d + 1].row
                ));
            }
            _ => {}
        }
        d = end;
    }
    out
}

/// DMFS の後の rows[range] の「Operands for field N.」の後の光線の行をまとめる
fn block(
    rows: &[MeritRow],
    row_config: &[i64],
    unit: f64,
    range: std::ops::Range<usize>,
    q: &Map<String, Value>,
    out: &mut Wizard,
) {
    // 視野の節の中の行（BLNK「Operands for field」から、光線・PRIM・CONF・BLNK でない行まで）
    let mut section = vec![false; rows.len()];
    let mut inside = false;
    for k in range.clone() {
        let r = &rows[k];
        if r.op == "BLNK" {
            inside = r.comment.starts_with("Operands for field");
        } else if ray_op(&r.op).is_none() && r.op != "PRIM" && r.op != "CONF" {
            inside = false;
        }
        section[k] = inside;
    }
    // Ignore Lateral Color: 波長ごとに PRIM で主波長を替えるので、波長ごとの単色の項にする
    let mono = range.clone().any(|k| section[k] && rows[k].op == "PRIM");
    let mut groups: Vec<(String, Group)> = Vec::new();
    for k in range {
        let r = &rows[k];
        let Some((wfe, reference, axis)) = ray_op(&r.op).filter(|_| section[k]) else {
            continue;
        };
        let (Ok(wave), Ok(hx), Ok(hy), Ok(px), Ok(py), Ok(t), Ok(w)) = (
            r.int(INT2),
            r.num(data(1)),
            r.num(data(2)),
            r.num(data(3)),
            r.num(data(4)),
            r.num(TARGET),
            r.num(WEIGHT),
        ) else {
            continue;
        };
        if t != 0.0 || w < 0.0 || wave < 1 {
            continue;
        }
        out.used[k] = true;
        let pt = (k, wave, px, py, w);
        let wave = if mono { Some(wave) } else { None };
        let key = format!("{} {hx:e} {hy:e} {wave:?} {}", row_config[k], r.op);
        let w = if is_length(&r.op) {
            w / (unit * unit)
        } else {
            w
        };
        if let Some((_, g)) = groups.iter_mut().find(|(kk, _)| *kk == key) {
            g.weight += w;
            g.rows.push(pt);
            continue;
        }
        let mut m = Map::new();
        m.insert("type".into(), json!(if wfe { "wavefront" } else { "spot" }));
        m.insert("field".into(), json!([hx, hy]));
        m.insert("wave".into(), json!(wave));
        m.insert("reference".into(), json!(reference));
        m.extend(q.clone());
        if axis != "xy" {
            m.insert("axis".into(), json!(axis));
        }
        if row_config[k] != 1 {
            m.insert("config".into(), json!(row_config[k]));
        }
        groups.push((
            key,
            Group {
                first: k,
                weight: w,
                m,
                rows: vec![pt],
            },
        ));
    }
    let full = points(q);
    let grid = q.get("quadrature") == Some(&json!("grid"));
    for (_, mut g) in groups {
        // 波長ごとの行の点・重みが設定文の求積と合わない（Wizard の後で行を消した・変えたなど）まとまりは、項にせず
        // 1 行ずつ読む
        let mut waves: Vec<i64> = g.rows.iter().map(|r| r.1).collect();
        waves.sort_unstable();
        waves.dedup();
        let ok = waves.iter().all(|&wv| {
            let pts: Vec<_> = g
                .rows
                .iter()
                .filter(|r| r.1 == wv)
                .map(|r| (r.2, r.3, r.4))
                .collect();
            let f = g.m["field"]
                .as_array()
                .map_or(vec![], |a| a.iter().filter_map(Value::as_f64).collect());
            matches(&pts, &full, grid, f.iter().all(|v| *v == 0.0))
        });
        if !ok {
            for r in &g.rows {
                out.used[r.0] = false;
            }
            out.notes.push(format!(
                ".zmx merit function row {}: the Wizard rows from here don't match the pupil sampling of its settings; they are read one by one",
                rows[g.first].row
            ));
            continue;
        }
        if g.weight > 0.0 {
            g.m.insert("weight".into(), json!(g.weight));
            out.terms.entry(g.first).or_default().push(g.m);
        }
    }
}
