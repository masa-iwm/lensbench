//! 系を読んだときの注意が serve の load・reload・system の返事に届く（CLI-10。前は標準エラーに書いて捨てていた）

use lensbench_engine::Session;
use serde_json::{json, Value};

fn ok(sv: &Session, method: &str, p: Value) -> Value {
    sv.call(method, &p, Some(1))
        .unwrap_or_else(|e| panic!("{method}: {}", e.message))
}

fn texts(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("no notes: {v}"))
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn project_notes_reach_load_reload_and_system() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    // P0 の .zmx の範囲の外からのピックアップは値に固定する: プロジェクトを展開したときの注意
    let p = format!("{root}/tests/projects/P0/project.json");
    let glass = format!("{root}/tests/glass");
    let sv = Session::new();
    let r = ok(&sv, "load", json!({ "file": p, "glass_dirs": [glass] }));
    let notes = texts(&r["notes"]);
    let pick = |ns: &[String]| {
        ns.iter()
            .any(|n| n.contains("pickup from surface 2 outside the range"))
    };
    assert!(pick(&notes), "{notes:?}");
    assert_eq!(texts(&ok(&sv, "system", json!({}))["notes"]), notes);
    assert_eq!(texts(&ok(&sv, "reload", json!({}))["notes"]), notes);
}
