//! serve の params の読み方（鍵の確かめ・パスの並び・数）

use std::path::PathBuf;

use serde_json::Value;

use super::RpcErr;

/// params の鍵が keys のどれかか（知らない鍵は誤り。打ち間違い・前の名前を黙って捨てない、XCT-34）
pub(super) fn known_keys(p: &Value, keys: &[&str]) -> Result<(), RpcErr> {
    let bad = p
        .as_object()
        .into_iter()
        .flatten()
        .find(|(k, _)| !keys.contains(&k.as_str()));
    match bad {
        Some((k, _)) => Err(RpcErr::params(format!(
            "unknown key {k} (one of {})",
            keys.join(", ")
        ))),
        None => Ok(()),
    }
}

pub(super) fn str_list(v: &Value, what: &str) -> Result<Vec<PathBuf>, RpcErr> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(vec![PathBuf::from(s)]),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| RpcErr::params(format!("{what} must be a list of strings")))
            })
            .collect(),
        _ => Err(RpcErr::params(format!("{what} must be a list of strings"))),
    }
}

pub(super) fn opt_usize(p: &Value, k: &str) -> Result<Option<usize>, RpcErr> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|x| Some(x as usize))
            .ok_or_else(|| RpcErr::params(format!("{k} must be an integer 0 or more"))),
    }
}

pub(super) fn opt_f64(p: &Value, k: &str) -> Result<Option<f64>, RpcErr> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| RpcErr::params(format!("{k} must be a number"))),
    }
}
