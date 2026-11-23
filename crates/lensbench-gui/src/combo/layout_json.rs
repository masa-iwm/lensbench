//! serve の layout・load の返事から読む小さな部品

use serde_json::Value;

/// serve の `blocks`（load・layout の返事）: id のあるブロック・群と、その往路の面の範囲
pub(crate) fn block_spans(v: &Value) -> Vec<(String, Vec<[usize; 2]>)> {
    runs_of(v, "forward")
}

/// serve の `blocks` の、鏡の `back`・光束の `turn` で戻る復路の面の範囲
pub(crate) fn block_back_spans(v: &Value) -> Vec<(String, Vec<[usize; 2]>)> {
    runs_of(v, "back")
}

fn runs_of(v: &Value, key: &str) -> Vec<(String, Vec<[usize; 2]>)> {
    let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
    v["blocks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    (
                        b["id"].as_str().unwrap_or_default().to_string(),
                        b[key]
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

/// 要素ごとの面の範囲（[`super::LayoutData::spans_for`]）。blocks は serve の `blocks`、stop は絞りの面（表の行から
/// 引けるので、レイアウト図の結果を作らずに引ける）
pub(crate) fn spans_in(
    blocks: &[(String, Vec<[usize; 2]>)],
    stop: Option<usize>,
    ids: &[String],
    lead_stop: bool,
) -> Vec<Option<[usize; 2]>> {
    let mut v: Vec<Option<[usize; 2]>> = Vec::with_capacity(ids.len());
    let mut prev = if lead_stop { stop } else { None };
    for id in ids {
        // 空の id（OFF の要素）は系に無い
        let s = (!id.is_empty())
            .then(|| super::LayoutData::block_after(blocks, id, prev, ids))
            .flatten();
        if let Some(s) = s {
            prev = Some(s[1]);
        }
        v.push(s);
    }
    v
}
