//! JSON スキーマ: docs/schema のファイルが serde の型から作ったものと同じか、例題・書き出した JSON が
//! スキーマに合うか。スキーマを作り直すには `LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project --test it schema::`

use lensbench_project::{schema, sysfile, Project, UserCatalog};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn schema_path(name: &str) -> PathBuf {
    root().join(format!("docs/schema/{name}.schema.json"))
}

fn validator(name: &str) -> jsonschema::Validator {
    let v = schema::schema(name).unwrap();
    jsonschema::validator_for(&v).unwrap_or_else(|e| panic!("{name} のスキーマ: {e}"))
}

fn check(v: &jsonschema::Validator, what: &str, x: &Value) {
    let errs: Vec<String> = v
        .iter_errors(x)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errs.is_empty(), "{what}: {}", errs.join("\n"));
}

#[test]
fn schema_files_match_types() {
    let update = std::env::var_os("LENSBENCH_UPDATE_SCHEMA").is_some();
    let mut stale = Vec::new();
    // 組み込みだけ（detector.rs が同じ exe で登録する試しの種類を入れない、XCT-32）
    for (name, v) in schema::all_with(&[]) {
        let p = schema_path(name);
        let want = schema::to_text(&v);
        if update {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &want).unwrap();
        } else if std::fs::read_to_string(&p)
            .ok()
            .as_deref()
            .map(|t| t.replace("\r\n", "\n"))
            != Some(want)
        {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "docs/schema の {} が型とずれている。LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project --test it schema:: で作り直す",
        stale.join(", ")
    );
}

#[test]
fn example_projects_match_schema() {
    let v = validator("project");
    let mut n = 0;
    for dir in ["examples/projects", "tests/projects/P0"] {
        for e in std::fs::read_dir(root().join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "json") && !p.ends_with("os_rays.json") {
                let t = std::fs::read_to_string(&p).unwrap();
                let x: Value = serde_json::from_str(&t).unwrap();
                check(&v, &p.display().to_string(), &x);
                // 書き直したものも合う
                let back: Value =
                    serde_json::from_str(&Project::from_json(&t).unwrap().to_json()).unwrap();
                check(&v, &format!("{} (書き直し)", p.display()), &back);
                n += 1;
            }
        }
    }
    assert!(n >= 18, "{n}");
}

#[test]
fn project_schema_rejects_mistakes() {
    let v = validator("project");
    let ok: Value = serde_json::from_str(
        r#"{"$schema": "x", "wavelengths": [{"um": 0.55}], "aperture": {"type": "epd", "value": 10},
            "blocks": [{"type": "air", "t": 5}]}"#,
    )
    .unwrap();
    assert!(v.is_valid(&ok));
    // ブロックの種類の誤り・必須の項目なし。組み込みでない種類はスキーマでは検出器（何でも書ける）で、
    // 読むときに登録していない種類として誤り
    let mut bad = ok.clone();
    bad["blocks"][0]["type"] = 5.into();
    assert!(!v.is_valid(&bad));
    let mut typo = ok.clone();
    typo["blocks"][0]["type"] = "glass".into();
    assert!(v.is_valid(&typo));
    let e = Project::from_json(&typo.to_string()).unwrap_err();
    assert!(e.to_string().contains("unknown block type"), "{e}");
    let mut bad = ok.clone();
    bad["blocks"][0].as_object_mut().unwrap().remove("t");
    assert!(!v.is_valid(&bad));
    // 検出器のブロック（登録した種類の項目はスキーマに載らない）
    let mut det = ok.clone();
    det["blocks"] = serde_json::from_str(r#"[{"type": "camera", "id": "S", "pixel": 5}]"#).unwrap();
    check(&v, "detector", &det);
}

#[test]
fn user_catalog_matches_schema() {
    let v = validator("user-catalog");
    let p = root().join("examples/lenses/LBP/parts.json");
    let t = std::fs::read_to_string(&p).unwrap();
    check(&v, "parts.json", &serde_json::from_str(&t).unwrap());
    let c = UserCatalog::from_json(&t).unwrap();
    check(&v, "書き直し", &serde_json::from_str(&c.to_json()).unwrap());
}

#[test]
fn system_file_matches_schema() {
    let v = validator("system");
    let glass = vec![root().join("tests/glass")];
    let mut n = 0;
    for e in std::fs::read_dir(root().join("examples/projects")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        // カタログのレンズを使うものは読めないので飛ばす
        let Ok(sys) = lensbench_project::load(&p, &glass, None) else {
            continue;
        };
        let txt = sysfile::to_string(&sys, &[]).unwrap();
        check(
            &v,
            &p.display().to_string(),
            &serde_json::from_str(&txt).unwrap(),
        );
        n += 1;
    }
    assert!(n >= 5, "{n}");
}

/// 知らないキー（綴りの違い）は黙って捨てずに誤りにする。書き戻しで省かれる既定値を書いたものは通す
#[test]
fn unknown_keys_are_errors() {
    let base = |extra: &str, block: &str| {
        format!(
            r#"{{ "wavelengths": [{{"um": 0.55}}], "aperture": {{"type": "epd", "value": 5}}, {extra}
            "blocks": [{{"type": "stop", "semi": 2.5}}, {{"type": "air", "t": 2}},
              {{"type": "lens", "id": "L1", {block} "source": {{"surfaces": [
                {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -50, "semi": 10}}]}}}},
              {{"type": "air", "t": 30}}] }}"#
        )
    };
    let e = lensbench_project::Project::from_json(&base("", r#""reversd": true,"#)).unwrap_err();
    assert!(e.to_string().contains("blocks/2/reversd"), "{e}");
    let e =
        lensbench_project::Project::from_json(&base(r#""object_distanse": 100,"#, "")).unwrap_err();
    assert!(e.to_string().contains("object_distanse"), "{e}");
    // 面の中（untagged の出どころの中）も
    let t = base("", "").replace(r#""glass": "N-BK7""#, r#""glas": "N-BK7""#);
    let e = lensbench_project::Project::from_json(&t).unwrap_err();
    assert!(e.to_string().contains("glas"), "{e}");
    // 中身がすべて既定の欄の中の綴りの違いも
    let e =
        lensbench_project::Project::from_json(&base(r#""environment": {"temprature_c": 25},"#, ""))
            .unwrap_err();
    assert!(e.to_string().contains("environment/temprature_c"), "{e}");
    // 既定値をわざわざ書いたもの（書き戻しでは省かれる）は通す
    lensbench_project::Project::from_json(&base(
        r#""telecentric": false, "surface_edits": [], "path": null,
            "environment": {"temperature_c": 20, "pressure_atm": 1, "adjust_index": false},"#,
        r#""reversed": false, "on": true, "decenter": [0, 0],"#,
    ))
    .unwrap();
}

/// 系の編集の操作（PRJ-11）: どの操作もスキーマに合い、読める。知らない操作・キー・型の違いはスキーマでも読みでも誤り
#[test]
fn edit_ops_schema_matches_reader() {
    use lensbench_project::edit_ops::{self, EditOp};
    let v = validator("edit-ops");
    let ok: Value = serde_json::from_str(
        r#"[
        {"op": "insert_surface", "at": 1, "surface": {"r": 50, "t": 5, "material": "N-BK7"}},
        {"op": "insert_surface", "at": 1},
        {"op": "delete_surface", "surface": "L1 front"},
        {"op": "set_surface", "surface": 2, "fields": {"semi": 8, "aperture": null}},
        {"op": "set_surface_type", "surface": 2, "type": "evenasph"},
        {"op": "set_material", "surface": 2, "material": "N-SF11"},
        {"op": "set_stop", "surface": 1},
        {"op": "add_pickup", "dst": 4, "src": 3, "what": "c", "scale": -1, "offset": {"$f": "inf"}, "from": null},
        {"op": "remove_pickup", "dst": 4, "what": "c"},
        {"op": "add_solve", "surface": 3, "kind": "marginal_height", "value": 0, "other": null, "param": 1},
        {"op": "add_solve", "surface": 3},
        {"op": "remove_solve", "surface": 3, "what": "c"},
        {"op": "remove_solve", "surface": 3},
        {"op": "set_surface_data", "surface": 2, "data": {"k": -1}},
        {"op": "set_param", "param": "T2", "value": 3.5},
        {"op": "set_system_data", "data": {"wavelengths": [0.55]}},
        {"op": "define_unit", "name": "G1", "spans": [[1, 2]], "pivot": [0, 0, 1]},
        {"op": "set_unit", "unit": 0, "name": "G2"},
        {"op": "remove_unit", "unit": "G2"},
        {"op": "add_mech", "part": {"name": "tube", "surface": 2, "bores": [{"z0": 0, "z1": 10, "radius": 15}]}},
        {"op": "set_mech", "mech": "tube", "part": {"tilt": [1, 0]}},
        {"op": "remove_mech", "mech": 0},
        {"op": "set_config_count", "n": 2},
        {"op": "set_config_row", "param": "T2", "values": [1, 2]},
        {"op": "add_config_row", "param": "T3"},
        {"op": "remove_config_row", "param": "T3"},
        {"op": "set_config", "k": 1},
        {"op": "insert_config", "at": 2, "pickup": true},
        {"op": "remove_config", "k": 2},
        {"op": "set_config_pickup", "param": "T2", "k": 2, "from": 1, "scale": 2},
        {"op": "remove_config_pickup", "param": "T2", "k": 2}
    ]"#,
    )
    .unwrap();
    check(&v, "ok", &ok);
    let ops = edit_ops::parse(&ok).unwrap();
    assert_eq!(ops.len(), ok.as_array().unwrap().len());
    // 書き直したものも合い、読み戻すと同じ
    let back = serde_json::to_value(&ops).unwrap();
    check(&v, "書き直し", &back);
    assert_eq!(edit_ops::parse(&back).unwrap(), ops);
    assert!(matches!(&ops[7], EditOp::AddPickup(p) if p.offset.unwrap().0 == f64::INFINITY));
    for bad in [
        r#"{"op": "nope"}"#,
        r#"{"surface": 1}"#,
        r#"{"op": "set_stop", "surfce": 1}"#,
        r#"{"op": "set_stop", "surface": -1}"#,
        r#"{"op": "add_pickup", "dst": 4, "src": 3, "what": "c", "scale": "x"}"#,
        r#"{"op": "add_solve", "surface": 3, "valu": 1}"#,
        r#"{"op": "remove_solve", "surface": 3, "what": "x"}"#,
        r#"{"op": "set_param", "param": "T2", "value": {"$f": "big"}}"#,
        r#"{"op": "define_unit", "name": "G", "spans": [[1, 2, 3]]}"#,
        r#"{"op": "set_config", "k": 1, "n": 2}"#,
    ] {
        let x: Value = serde_json::from_str(&format!("[{bad}]")).unwrap();
        assert!(!v.is_valid(&x), "スキーマが通した: {bad}");
        let e = edit_ops::parse(&x).unwrap_err();
        assert!(
            e.message().unwrap_or_default().starts_with("operation 1"),
            "{e}"
        );
    }
}
