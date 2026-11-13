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
    assert!(e.contains("知らないブロックの種類"), "{e}");
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

#[test]
fn swap_accepts_schema_key() {
    let v = validator("swap");
    let x: Value = serde_json::from_str(
        r#"{"$schema": "s", "slots": [{"block": "A", "filter": {"efl": [40, null]}}], "vars": ["d.t"]}"#,
    )
    .unwrap();
    check(&v, "swap", &x);
    lensbench_project::swap::SwapSpec::from_json(&x.to_string()).unwrap();
    let mut bad = x.clone();
    bad["slot"] = Value::Null;
    assert!(!v.is_valid(&bad));
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
    assert!(e.contains("blocks/2/reversd"), "{e}");
    let e =
        lensbench_project::Project::from_json(&base(r#""object_distanse": 100,"#, "")).unwrap_err();
    assert!(e.contains("object_distanse"), "{e}");
    // 面の中（untagged の出どころの中）も
    let t = base("", "").replace(r#""glass": "N-BK7""#, r#""glas": "N-BK7""#);
    let e = lensbench_project::Project::from_json(&t).unwrap_err();
    assert!(e.contains("glas"), "{e}");
    // 既定値をわざわざ書いたもの（書き戻しでは省かれる）は通す
    lensbench_project::Project::from_json(&base(
        r#""telecentric": false, "surface_edits": [], "path": null,"#,
        r#""reversed": false, "on": true, "decenter": [0, 0],"#,
    ))
    .unwrap();
}
