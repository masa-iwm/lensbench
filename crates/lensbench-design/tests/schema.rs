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

/// 評価関数の項: 例の並びがスキーマに合い、知らないキー・種類はスキーマでも読みでも誤り（PRJ-11）
#[test]
fn terms_schema_matches_reader() {
    let v = jsonschema::validator_for(&schema::schema("terms").unwrap()).unwrap();
    let ok: Value = serde_json::json!([
        {"type": "ray", "field": 1, "py": 1, "q": "y", "target": 0.1, "weight": 2},
        {"type": "mtf", "field": [0, 0.5], "freq": 30, "dir": "tan", "min": 0.3, "config": 1},
        {"type": "zplm", "macro": 7, "data": 2, "macro_dirs": ["."]},
        {"type": "paraxial", "q": "efl", "min": 351},
        {"type": "plugin", "name": "rms", "field": 1, "output": "wfe", "args": {"rings": 4}}
    ]);
    let errs: Vec<String> = v.iter_errors(&ok).map(|e| e.to_string()).collect();
    assert!(errs.is_empty(), "{}", errs.join("\n"));
    assert!(v.is_valid(&ok[0]));
    for bad in [
        serde_json::json!({"type": "ray", "field": 1, "q": "y", "taget": 1}),
        serde_json::json!({"type": "nope"}),
        serde_json::json!({"type": "mtf", "field": 1}),
    ] {
        assert!(!v.is_valid(&bad), "{bad}");
    }
    let s = lensbench_core::zmx::load_with(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/refs/A0/system.zmx"),
        &lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let e = lensbench_design::merit::term_from_json(
        &serde_json::json!({"type": "ray", "field": 1, "q": "y", "taget": 1}),
        std::slice::from_ref(&s),
    )
    .unwrap_err();
    assert!(e.starts_with("ray の項: unknown field `taget`"), "{e}");
    // 値の綴りの誤りは種類の誤りにしない
    let terms = |v: Value| lensbench_design::merit::term_from_json(&v, std::slice::from_ref(&s));
    let e = terms(serde_json::json!({"type": "mtf", "field": 1, "freq": 30, "dir": "TAN"}))
        .unwrap_err();
    assert!(e.starts_with("mtf の項: unknown variant `TAN`"), "{e}");
    let e = terms(serde_json::json!({"type": "nope"})).unwrap_err();
    assert!(e.starts_with("type nope（spot・"), "{e}");
    assert!(terms(serde_json::json!({"type": "paraxial", "q": "efl"})).is_err());
    terms(serde_json::json!({"type": "paraxial", "q": "efl", "min": 351})).unwrap();
}
