//! serve の元に戻す段の名前・一覧（serve.rs から分けた）
use super::{Change, Served, Snap};
use serde_json::{json, Value};

/// 段の変更の短い記述（GUI の一覧の名前）: set は名前の並び、edit は操作ごとの op と面・名前
pub(super) fn change_label(c: Option<&Change>) -> Value {
    match c {
        Some(Change::Set(v)) => json!({ "set": v.iter().map(|r| r.0.clone()).collect::<Vec<_>>() }),
        Some(Change::Edit(ops)) => {
            let ops: Vec<Value> = ops
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    let mut m = serde_json::Map::new();
                    for k in ["op", "surface", "param", "at", "k", "name"] {
                        if let Some(x) = o.get(k) {
                            m.insert(k.into(), x.clone());
                        }
                    }
                    // 変えた項目の名前（値は出さない）
                    if let Some(f) = o.get("fields").and_then(Value::as_object) {
                        m.insert("fields".into(), json!(f.keys().collect::<Vec<_>>()));
                    }
                    Value::Object(m)
                })
                .collect();
            json!({ "edit": ops })
        }
        None => Value::Null,
    }
}

/// 元に戻せる段・やり直せる段の数と、その変更（`undo_list` は新しい順、`redo_list` は次にやり直す順）
pub(super) fn history_json(x: &Served) -> Value {
    let list = |v: &[Snap]| v.iter().rev().map(|s| s.what.clone()).collect::<Vec<_>>();
    json!({
        "undo": x.undo.len(),
        "redo": x.redo.len(),
        "undo_list": list(&x.undo),
        "redo_list": list(&x.redo),
    })
}

/// 今戻した・やり直した段が開く・読み直すだったか（系の概要を返して GUI の表を読み直させる）
pub(super) fn snap_was_load(v: &Value, back: bool) -> bool {
    // 戻したら、その段はやり直す段の先頭に、やり直したら元に戻す段の先頭にある
    let top = if back {
        &v["redo_list"][0]
    } else {
        &v["undo_list"][0]
    };
    top.get("open").is_some() || top.get("reload").is_some()
}

/// 構成の切り替えだけの edit の段の印（set のパラメータの名前と重ならない）
pub(super) const CONFIG_KEY: &str = "\u{1}set_config";

/// edit の ops が構成の切り替え（`set_config`）だけか。続けて切り替えても元に戻す段は 1 つにする（GAR-28: Ctrl+A で
/// 構成を順に見ても、段が切り替えの数だけ積まれない）
pub(super) fn config_only(ops: &Value) -> bool {
    ops.as_array()
        .is_some_and(|a| !a.is_empty() && a.iter().all(|o| o["op"] == "set_config"))
}
