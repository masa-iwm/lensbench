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
    let e = terms(serde_json::json!({"type": "spot", "field": 1, "min": 1})).unwrap_err();
    assert!(e.contains("min・max は使えない"), "{e}");
}

/// 公差の設定: 例がスキーマに合い、知らないキーはスキーマでも読みでも誤りで、読みの誤りには欄の道が付く（PRJ-11）
#[test]
fn tolerance_schema_matches_reader() {
    let v = jsonschema::validator_for(&schema::schema("tolerance").unwrap()).unwrap();
    let ok = serde_json::json!({
        "$schema": "s", "trials": 10, "dist": "uniform", "radial": "normal:3", "wedge_face": "front",
        "catalog_tols": {"EDMUND": {"thickness": 0.1, "rules": [{"cemented": false, "dia_le": 12, "thickness": 0.05}]}, "*": "precision"},
        "lenses": {"L1": {"tilt": 0.02}, "L2": null, "L3": "high"},
        "groups": {"G1": {"decenter": 0.05}},
        "tols": [{"param": "3.t", "pm": 0.1}, {"params": ["2.r", "3.r"], "lo": -1, "hi": 1, "scale": [1, -1]}],
        "compensators": ["9.t", {"param": "G1.shift", "lo": -1, "hi": 1, "configs": [1]}],
        "merit": "spot", "reference": "chief", "percentiles": [50, 90], "samples": true
    });
    let errs: Vec<String> = v.iter_errors(&ok).map(|e| e.to_string()).collect();
    assert!(errs.is_empty(), "{}", errs.join("\n"));
    lensbench_design::tolerance::read_spec(&ok).unwrap();
    for (bad, path) in [
        (
            serde_json::json!({"trails": 10}),
            "tolerance: unknown field `trails`",
        ),
        (
            serde_json::json!({"lenses": {"L1": {"tlit": 1}}}),
            "tolerance.lenses.L1: unknown field `tlit`",
        ),
        (
            serde_json::json!({"catalog_tols": {"X": {"rules": [{"dia_lx": 1}]}}}),
            "tolerance.catalog_tols.X: rules[0]: unknown field `dia_lx`",
        ),
        (
            serde_json::json!({"tols": [{"param": "3.t", "pm": 0.1}, {"parm": "3.t"}]}),
            "tolerance.tols[1]: unknown field `parm`",
        ),
    ] {
        assert!(!v.is_valid(&bad), "{bad}");
        let e = lensbench_design::tolerance::read_spec(&bad).unwrap_err();
        assert!(e.starts_with(path), "{e}");
    }
}
