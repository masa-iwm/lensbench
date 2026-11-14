//! JSON スキーマ（差し替え探索の設定）: docs/schema/swap.schema.json が型から作ったものと同じか、設定がスキーマに合うか。
//! 作り直すには `LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-design --test it schema::`

use lensbench_design::schema;
use serde_json::Value;
use std::path::Path;

#[test]
fn schema_files_match_types() {
    let update = std::env::var_os("LENSBENCH_UPDATE_SCHEMA").is_some();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schema");
    for (name, v) in schema::all() {
        let p = dir.join(format!("{name}.schema.json"));
        let want = lensbench_project::schema::to_text(&v);
        if update {
            std::fs::write(&p, &want).unwrap();
        } else {
            let have = std::fs::read_to_string(&p)
                .unwrap_or_default()
                .replace("\r\n", "\n");
            assert!(
                have == want,
                "docs/schema の {name} が型とずれている。LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-design --test it schema:: で作り直す"
            );
        }
    }
}

#[test]
fn swap_accepts_schema_key() {
    let v = jsonschema::validator_for(&schema::schema("swap").unwrap()).unwrap();
    let x: Value = serde_json::from_str(
        r#"{"$schema": "s", "slots": [{"block": "A", "filter": {"efl": [40, null]}}], "vars": ["d.t"]}"#,
    )
    .unwrap();
    let errs: Vec<String> = v.iter_errors(&x).map(|e| e.to_string()).collect();
    assert!(errs.is_empty(), "{}", errs.join("\n"));
    lensbench_design::swap::SwapSpec::from_json(&x.to_string()).unwrap();
    let mut bad = x.clone();
    bad["slot"] = Value::Null;
    assert!(!v.is_valid(&bad));
}
