//! serve の optimize: CLI の optimize と同じ結果、apply で読み込んだ系に当たること、構成の並び。
//! 1 つの場でなので、別のテストの実行ファイルにしている（並列のテストと系を取り合わない）。

use lensbench_engine::Session;
use serde_json::{json, Value};
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn call(sv: &Session, m: &str, p: Value) -> Value {
    sv.call(m, &p, None)
        .unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn load(sv: &Session, file: &str, state: Option<&str>) {
    call(
        sv,
        "load",
        json!({
            "file": Path::new(ROOT).join(file).display().to_string(),
            "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
            "state": state,
        }),
    );
}

#[test]
fn optimize_serve() {
    let sv = Session::new();
    // 1 つのテストの中で順に（1 つの場で）
    load(&sv, "tests/refs/A0/system.zmx", None);
    call(&sv, "set", json!({ "values": { "8.t": 0.3 } }));
    let t = json!({"type": "ray", "field": 1, "py": 0.7, "q": "y"});
    let p = json!({
        "vars": ["8.t", {"name": "6.c", "lo": 0.0, "hi": 0.05}],
        "merit": "spot",
        "terms": [t],
    });
    let r = call(&sv, "optimize", p.clone());
    assert!(r["merit"].as_f64().unwrap() < r["merit0"].as_f64().unwrap());
    assert_eq!(r["names"], json!(["8.t", "6.c"]));
    assert_eq!(r["applied"], json!(false));
    // 進み具合は終わると消える
    assert!(sv.optimize_progress().is_none());
    // 足した項の値（既定の項の後）
    let terms = r["terms"].as_array().unwrap();
    assert_eq!(terms.last().unwrap()["default"], json!(false));
    assert!(terms[0]["default"].as_bool().unwrap());
    // 系は変えていない
    assert_eq!(
        call(&sv, "get", json!({"param": "8.t"}))["value"],
        json!(0.3)
    );
    // apply で当てる
    let mut pa = p.clone();
    pa["apply"] = json!(true);
    let ra = call(&sv, "optimize", pa);
    assert_eq!(ra["applied"], json!(true));
    assert_eq!(ra["x"], r["x"]);
    assert_eq!(
        call(&sv, "get", json!({"param": "8.t"}))["value"],
        r["x"][0]
    );
    // 間違い
    for bad in [
        json!({"vars": ["8.t"], "terms": [{"type": "ray", "q": "y"}]}),
        json!({"vars": ["8.t"], "merit": "foo"}),
        json!({"vars": [{"name": "8.t", "configs": [3]}]}),
    ] {
        assert!(
            lensbench_engine::call("optimize", &bad, None).is_err(),
            "{bad}"
        );
    }

    // 構成の並び（プロジェクトの状態）: 2 つの状態の既定の項（視野 4 つずつ）。読み込んだ状態（far）に効かない変数は
    // 当てない
    load(
        &sv,
        "examples/projects/P6_states_focus_group.json",
        Some("far"),
    );
    let r = call(
        &sv,
        "optimize",
        json!({
            "vars": [{"name": "front.shift", "configs": [1]}, {"name": "front.shift", "configs": [2]}],
            "configs": ["far", "near"],
            "iters": 3,
            "apply": true,
        }),
    );
    assert_eq!(r["terms"].as_array().unwrap().len(), 8);
    assert_eq!(r["applicable"], json!([true, false]));
    assert_eq!(r["applied"], json!(true));
    assert_eq!(
        call(&sv, "get", json!({"param": "front.shift"}))["value"],
        r["x"][0]
    );
}
