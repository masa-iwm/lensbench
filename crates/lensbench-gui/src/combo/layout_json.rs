//! serve の layout・load の返事から読む小さな部品

use serde_json::Value;

/// serve の `blocks`（load・layout の返事）: id のあるブロック・群と、その往路の面の範囲
pub(crate) fn block_spans(v: &Value) -> Vec<(String, Vec<[usize; 2]>)> {
    let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
    v["blocks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    (
                        b["id"].as_str().unwrap_or_default().to_string(),
                        b["forward"]
                            .as_array()
                            .map(|a| a.iter().map(|x| [us(&x[0]), us(&x[1])]).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn pts(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                .collect()
        })
        .unwrap_or_default()
}
