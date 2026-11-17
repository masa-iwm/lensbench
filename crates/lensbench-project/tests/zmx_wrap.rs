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
        let p = lensbench_project::zmx_project(std::slice::from_ref(&s), &f.display().to_string())
            .unwrap();
        let env = lensbench_project::Env {
            base: "/".into(),
            glass_dirs: gd.clone(),
            catalog: None,
            step_files: None,
        };
        let t = lensbench_project::Project::from_json(&p.to_string())
            .and_then(|q| {
                lensbench_project::expand(&q, &env, &Default::default())
                    .map_err(lensbench_core::Error::from)
            })
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()))
            .system;
        let name = f.parent().unwrap().file_name().unwrap().to_string_lossy();
        match lensbench_project::same_system(&s, &t) {
            Ok(()) => same.push(name.to_string()),
            Err(e) => differ.push(format!("{name}: {e}")),
        }
    }
    // 像面を傾けた C0・C1 も、プロジェクトの像面（image）で書ける
    assert!(differ.is_empty(), "{differ:?}");
    assert!(same.iter().any(|s| s == "C0"), "{same:?}");
    assert!(same.len() >= 30, "{same:?}");
    // GLRS・矩形の視野・ケラレ係数・環境を書いたもの
    for n in ["A0", "A6", "F2"] {
        assert!(same.iter().any(|s| s == n), "{n}: {differ:?}");
    }
}

#[test]
fn zmx_project_writes_multi_config_and_field_solve() {
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
    // 多重構成の表を持つ系 1 つからも、構成ごとの状態を作る
    let mut m = s.clone();
    m.set_config_count(3).unwrap();
    let p = lensbench_project::zmx_project(std::slice::from_ref(&m), "x.zmx").unwrap();
    assert_eq!(p["states"].as_array().map(Vec::len), Some(3), "{p}");
    assert_eq!(p["states"][2]["set"]["zmx"]["config"], 3, "{p}");
    s.edit(|rx| {
        rx.field_type =
            lensbench_core::system::FieldType::Solve(lensbench_core::system::FieldCond::Position {
                surf: 3,
            });
        Ok(())
    })
    .unwrap();
    // 条件で定める視野は、条件の面を系の面の番号で書く。作り直した系は元と同じ
    let p = lensbench_project::zmx_project(std::slice::from_ref(&s), "A0/system.zmx").unwrap();
    assert_eq!(p["fields"]["type"], "solve", "{p}");
    assert_eq!(p["fields"]["solve"]["surface"], "3", "{p}");
    let env = lensbench_project::Env {
        base: root.join("tests/refs"),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
        step_files: None,
    };
    let t = lensbench_project::Project::from_json(&p.to_string())
        .and_then(|q| lensbench_project::expand(&q, &env, &Default::default()).map_err(Into::into))
        .unwrap()
        .system;
    lensbench_project::same_system(&s, &t).unwrap();
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
        lensbench_project::expand(&p, &env, &lensbench_project::ExpandOpts::default())
            .map(|x| x.system)
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

/// 多重構成の .zmx は、構成ごとの状態「Config k」を持つプロジェクトになり、状態 k を当てて作り直すと構成 k の系と同じ
#[test]
fn multi_config_zmx_becomes_states() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // tests/bs_configs の .zmx はガラスのカタログ LBSCH（試験の SCHOTT.AGF と同じもの）を使う
    let lb = Path::new(env!("CARGO_TARGET_TMPDIR")).join("zmx_wrap_lbsch");
    std::fs::create_dir_all(&lb).unwrap();
    std::fs::copy(root.join("tests/glass/SCHOTT.AGF"), lb.join("LBSCH.AGF")).unwrap();
    let gd = vec![root.join("tests/glass"), lb];
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests/bs_configs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "zmx"))
        .collect();
    files.sort();
    assert!(files.len() >= 8, "{files:?}");
    let env = lensbench_project::Env {
        base: "/".into(),
        glass_dirs: gd.clone(),
        catalog: None,
        step_files: None,
    };
    let mut differ = Vec::new();
    let mut multi = 0;
    for f in &files {
        let o = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: gd.clone(),
            ..Default::default()
        };
        let all = lensbench_core::zmx::load_configs(f, &o, &[]).unwrap();
        let n = all.len();
        let p = lensbench_project::zmx_project(&all, &f.display().to_string())
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let p = lensbench_project::Project::from_json(&p.to_string()).unwrap();
        if n == 1 {
            assert!(p.states.is_empty());
            continue;
        }
        multi += 1;
        assert_eq!(p.state_names().len(), n, "{}", f.display());
        for (k, sk) in all.iter().enumerate() {
            let q = p.with_state(&format!("Config {}", k + 1)).unwrap();
            let t = lensbench_project::expand(&q, &env, &Default::default())
                .unwrap_or_else(|e| panic!("{} config {}: {e}", f.display(), k + 1))
                .system;
            if let Err(e) = lensbench_project::same_system(sk, &t) {
                differ.push(format!("{} config {}: {e}", f.display(), k + 1));
            }
        }
    }
    assert!(differ.is_empty(), "{differ:#?}");
    assert!(multi >= 6, "{multi}");
}

/// 取り込んだ .zmx の膜ファイルは、展開した系の膜ファイルになる（面の膜の名前はそのファイルで引く）
#[test]
fn zmx_block_brings_its_coating_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let f = root.join("tests/birefringent/b2_coat.zmx");
    let gd = vec![root.join("tests/glass")];
    let s = lensbench_core::zmx::load_with(
        &f,
        &lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: gd.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.coating_file.is_some());
    let p = lensbench_project::zmx_project(std::slice::from_ref(&s), "b2_coat.zmx").unwrap();
    let env = lensbench_project::Env {
        base: f.parent().unwrap().into(),
        glass_dirs: gd,
        catalog: None,
        step_files: None,
    };
    let x = lensbench_project::Project::from_json(&p.to_string())
        .and_then(|q| lensbench_project::expand(&q, &env, &Default::default()).map_err(Into::into))
        .unwrap();
    assert_eq!(x.system.coating_file, s.coating_file);
    lensbench_project::same_system(&s, &x.system).unwrap();
}
