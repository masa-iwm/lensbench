//! .zmx を zmx ブロック 1 つのプロジェクトに書き換えて作り直すと、元の系と同じになる（GUI が .zmx を組合せとして開く）

use std::path::Path;

#[test]
fn zmx_projects_rebuild_the_same_system() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let gd = vec![root.join("tests/glass")];
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests/refs"))
        .unwrap()
        .map(|e| e.unwrap().path().join("system.zmx"))
        .filter(|p| p.exists())
        .collect();
    files.sort();
    let (mut same, mut differ) = (Vec::new(), Vec::new());
    for f in &files {
        let o = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: gd.clone(),
            ..Default::default()
        };
        let s = lensbench_core::zmx::load_with(f, &o).unwrap();
        let p = lensbench_project::zmx_project(&s, &f.display().to_string()).unwrap();
        let (t, _) =
            lensbench_project::expand_json(&p.to_string(), Path::new("/"), &gd, None, None)
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let name = f.parent().unwrap().file_name().unwrap().to_string_lossy();
        match lensbench_project::same_system(&s, &t) {
            Ok(()) => same.push(name.to_string()),
            Err(e) => differ.push(format!("{name}: {e}")),
        }
    }
    // 像面を傾けた C0・C1 だけはプロジェクトで書けない（像面はブロックでない）
    assert_eq!(differ.len(), 2, "{differ:?}");
    assert!(
        differ
            .iter()
            .all(|d| (d.starts_with("C0") || d.starts_with("C1")) && d.contains("before")),
        "{differ:?}"
    );
    assert!(same.len() >= 30, "{same:?}");
    // GLRS・矩形の視野・ケラレ係数・環境を書いたもの
    for n in ["A0", "A6", "F2"] {
        assert!(same.iter().any(|s| s == n), "{n}: {differ:?}");
    }
}

#[test]
fn zmx_project_refuses_what_it_cannot_write() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = lensbench_core::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let mut m = s.clone();
    m.set_config_count(2).unwrap();
    let e = lensbench_project::zmx_project(&m, "x.zmx").unwrap_err();
    assert!(
        e.message().unwrap_or_default().contains("configurations"),
        "{e}"
    );
    s.field_type =
        lensbench_core::system::FieldType::Solve(lensbench_core::system::FieldCond::Position {
            surf: 3,
        });
    assert!(lensbench_project::zmx_project(&s, "x.zmx").is_err());
}

/// glrs はブロックの面・系の面の番号で書ける。ケラレ係数が視野より多ければ誤り
#[test]
fn glrs_and_vignetting_in_a_project() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = lensbench_project::Env {
        step_files: None,
        base: root.join("tests/refs"),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    const FIELDS: &str = r#""fields": {"type": "angle", "points": [[0, 0], [0, 5]], "norm": "rect",
        "vignetting": [[0, 0, 0, 0, 0], [0.1, -0.1, 0.05, 0.02, 0]]},"#;
    let make_with = |fields: &str, extra: &str| {
        let txt = format!(
            r#"{{ "wavelengths": [{{"um": 0.55}}], "aperture": {{"type": "epd", "value": 5}},
            {fields} {extra}
            "blocks": [{{"type": "stop", "semi": 2.5}}, {{"type": "air", "t": 2}},
              {{"type": "lens", "id": "L1", "source": {{"surfaces": [
                {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -50, "semi": 10}}]}}}},
              {{"type": "air", "t": 30}}] }}"#
        );
        let p = lensbench_project::Project::from_json(&txt)?;
        lensbench_project::expand(&p, &env)
    };
    let make = |extra: &str| make_with(FIELDS, extra);
    let s = make(r#""glrs": {"block": "L1", "surface": 1},"#).unwrap();
    assert!((s.surfaces[s.glrs].c + 0.02).abs() < 1e-12, "{}", s.glrs);
    assert_eq!(s.field_norm, lensbench_core::system::FieldNorm::Rect);
    assert_eq!((s.fields[1].vdx, s.fields[1].vcy), (0.1, 0.02));
    assert_eq!(make(r#""glrs": 2,"#).unwrap().glrs, 2);
    assert_eq!(make("").unwrap().glrs, 1);
    let e = make(r#""glrs": 99,"#).unwrap_err();
    assert!(e.message().unwrap_or_default().contains("glrs"), "{e}");
    let e = make_with(
        r#""fields": {"points": [[0, 0]], "vignetting": [[0, 0, 0, 0, 0], [0, 0, 0, 0, 0]]},"#,
        "",
    )
    .unwrap_err();
    assert!(
        e.message()
            .unwrap_or_default()
            .contains("2 vignetting rows for 1 fields"),
        "{e}"
    );
}
