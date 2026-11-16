//! ブロックの面の絞り・ソルブ・ピックアップ（ブロックの surface_ops）

use lensbench_project::{expand, Env, ExpandOpts, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// ops はブロックの id と、そのブロックの surface_ops
fn project(
    stop_block: bool,
    ops: &[(&str, &str)],
) -> lensbench_core::Result<(lensbench_core::System, Vec<String>)> {
    let stop = if stop_block {
        r#"{"type": "stop", "semi": 2.5}, {"type": "air", "t": 2},"#
    } else {
        ""
    };
    let txt = format!(
        r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
        "aperture": {{"type": "epd", "value": 5}},
        "blocks": [{stop}
          {{"type": "lens", "id": "L1", "source": {{"surfaces": [
            {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -50, "semi": 10}}]}}}},
          {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L2", "source": {{"surfaces": [
            {{"radius": 40, "t": 3, "glass": "N-BK7", "semi": 10}}, {{"radius": -60, "semi": 10}}]}}}},
          {{"type": "air", "t": 30}}] }}"#
    );
    let txt = crate::put(&txt, "surface_ops", ops);
    expand(&Project::from_json(&txt)?, &env(), &ExpandOpts::default()).map(|x| (x.system, x.notes))
}

#[test]
fn stop_solve_pickup_by_block() {
    // 当てる面（surface・dst）の番号はそのブロックの面。src・other のほかのブロックの面は id で
    let ops = &[
        ("L1", r#"[{"op": "set_stop", "surface": 1}]"#),
        (
            "L2",
            r#"[{"op": "add_pickup", "dst": 1, "src": {"block": "L2", "surface": 0}, "what": "c", "scale": -1},
                {"op": "add_solve", "surface": 1, "kind": "marginal_height", "value": 0}]"#,
        ),
    ];
    // 絞りのブロックが無くても、面に置けばよい
    let (s, warn) = project(false, ops).unwrap();
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.stop(), 2);
    assert!((s.surfaces[4].c + s.surfaces[3].c).abs() < 1e-15);
    assert_eq!(s.solves.len(), 1);
    assert_eq!(s.solves[0].surf, 4);
    // 像面に周辺光線が集まる（厚さを解いた）
    assert!((s.surfaces[4].t - 30.0).abs() > 1e-6);
    // 絞りのブロックがあっても面に移る
    let (s, _) = project(true, ops).unwrap();
    assert_eq!(s.stop(), 3);
    assert_eq!(s.surfaces.iter().filter(|d| d.stop).count(), 1);
}

#[test]
fn missing_block_warns_and_bad_ops_fail() {
    // 参照するほかのブロックが無ければ注意
    let (_, warn) = project(
        true,
        &[(
            "L2",
            r#"[{"op": "add_pickup", "dst": 1, "src": {"block": "X", "surface": 0}, "what": "c"}]"#,
        )],
    )
    .unwrap();
    assert!(warn.iter().any(|w| w.contains("X")), "{warn:?}");
    let e = project(true, &[("L1", r#"[{"op": "set_stop", "surface": 5}]"#)]).unwrap_err();
    assert!(e.message().unwrap_or_default().contains("surface 5"), "{e}");
    let e = project(
        true,
        &[("L1", r#"[{"op": "delete_surface", "surface": 1}]"#)],
    )
    .unwrap_err();
    assert!(
        e.message().unwrap_or_default().contains("delete_surface"),
        "{e}"
    );
    // 誤りの文はブロックと surface_ops の何番目か（外す操作で束を分けてもずれない）
    let e = project(
        true,
        &[(
            "L2",
            r#"[{"op": "add_solve", "surface": 0},
                {"op": "remove_solve", "surface": 0},
                {"op": "add_solve", "surface": 1, "kind": "nonsense"}]"#,
        )],
    )
    .unwrap_err();
    assert!(
        e.message()
            .unwrap_or_default()
            .contains("block L2 surface_ops 3:"),
        "{e}"
    );
    // 絞りが無く set_stop も無ければ従来どおりのエラー
    assert!(project(false, &[])
        .unwrap_err()
        .to_string()
        .contains("stop"));
}

/// id の無いブロックにも持たせられ、状態で外したブロックの操作は当てない（注意も出さない）
#[test]
fn ops_follow_their_block() {
    let txt = r#"{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{"um": 0.5875618}],
        "aperture": {"type": "epd", "value": 5},
        "blocks": [
          {"type": "lens", "source": {"surfaces": [
            {"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}, {"radius": -50, "semi": 10}]},
           "surface_ops": [{"op": "set_stop", "surface": 1}]},
          {"type": "air", "t": 10},
          {"type": "group", "id": "G", "blocks": [
            {"type": "lens", "source": {"surfaces": [
              {"radius": 40, "t": 3, "glass": "N-BK7", "semi": 10}, {"radius": -60, "semi": 10}]},
             "surface_ops": [{"op": "add_solve", "surface": 1, "kind": "marginal_height", "value": 0}]}]},
          {"type": "air", "t": 30}],
        "states": [{"name": "off", "set": {"G": {"on": false, "off": "air"}}}] }"#;
    let p = Project::from_json(txt).unwrap();
    let x = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    assert_eq!(x.system.stop(), 2);
    assert_eq!(x.system.solves.len(), 1);
    assert_eq!(x.system.solves[0].surf, 4);
    let o = ExpandOpts {
        state: Some("off"),
        ..Default::default()
    };
    let x = expand(&p, &env(), &o).unwrap();
    assert_eq!(x.system.stop(), 2);
    assert!(x.system.solves.is_empty());
    assert!(x.notes.is_empty(), "{:?}", x.notes);
}

/// surface_ops は set_stop・add_solve・add_pickup・remove_solve・remove_pickup だけで、知らないキー・面の書き方の誤りは読むときに誤り（PRJ-11）
#[test]
fn surface_ops_are_typed() {
    for (ops, want) in [
        (
            r#"[{"op": "set_material", "surface": 1, "material": "N-BK7"}]"#,
            "set_material",
        ),
        (r#"[{"op": "set_stop", "surface": 1, "x": 1}]"#, "x"),
        (
            r#"[{"op": "set_stop", "surface": {"block": "L1", "face": 1}}]"#,
            "face",
        ),
        (
            r#"[{"op": "set_stop", "surface": -1}]"#,
            "surface number -1",
        ),
    ] {
        let e = project(false, &[("L1", ops)]).unwrap_err();
        assert!(e.message().unwrap_or_default().contains(want), "{ops}: {e}");
    }
    // 面の名前（展開した系の面のコメント）も前どおり書ける。書き直しても同じ形
    let p = lensbench_project::Project::from_json(
        r#"{"wavelengths": [{"um": 0.55}], "aperture": {"type": "epd", "value": 5},
        "blocks": [{"type": "stop", "semi": 2.5,
          "surface_ops": [{"op": "add_solve", "surface": 0, "other": {"block": "S", "surface": 0}},
                          {"op": "add_pickup", "dst": "IMG", "src": 1, "what": "t", "scale": -1, "from": null, "offset": 0}]},
          {"type": "air", "t": 30}]}"#,
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    assert_eq!(
        v["blocks"][0]["surface_ops"],
        serde_json::json!([
            {"op": "add_solve", "surface": 0, "other": {"block": "S", "surface": 0}},
            {"op": "add_pickup", "dst": "IMG", "src": 1, "what": "t", "scale": -1.0, "offset": 0.0}
        ])
    );
    // プロジェクトの直下には書けない
    let e = lensbench_project::Project::from_json(
        r#"{"wavelengths": [{"um": 0.55}], "aperture": {"type": "epd", "value": 5},
        "blocks": [{"type": "stop", "semi": 2.5}, {"type": "air", "t": 30}],
        "surface_ops": []}"#,
    )
    .unwrap_err();
    assert!(e.to_string().contains("surface_ops"), "{e}");
}

/// ブロックの元（.zmx）のソルブを remove_solve で外せる。外すものが無ければ当てずに注意（A0 の最後の厚さのソルブ）
#[test]
fn remove_solve_of_block_source() {
    let a0 = |ops: &str| {
        let txt = format!(
            r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
            "aperture": {{"type": "epd", "value": 10}},
            "blocks": [{{"type": "zmx", "id": "zmx", "path": "A0/system.zmx", "surface_ops": {ops}}},
              {{"type": "air", "t": 0}}] }}"#
        );
        expand(
            &Project::from_json(&txt).unwrap(),
            &env(),
            &ExpandOpts::default(),
        )
        .map(|x| (x.system, x.notes))
        .unwrap()
    };
    // 元の .zmx は面 7 の厚さに周辺光線の高さのソルブ（MAZH）を持つ
    let (s, warn) = a0("[]");
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.solves.iter().map(|q| q.surf).collect::<Vec<_>>(), [7]);
    let t = s.surfaces[7].t;
    let rm = r#"[{"op": "remove_solve", "surface": 6, "what": "t"}]"#;
    let (s, warn) = a0(rm);
    assert!(warn.is_empty(), "{warn:?}");
    assert!(s.solves.is_empty());
    // 外しても値はそのまま
    assert_eq!(s.surfaces[7].t, t);
    // 外したあとに置き直せる（順に当てる）
    let (s, _) = a0(&format!(
        "[{}, {}]",
        &rm[1..rm.len() - 1],
        r#"{"op": "add_solve", "surface": 6, "kind": "marginal_height", "value": 0.5}"#
    ));
    assert_eq!(s.solves.len(), 1);
    assert_eq!(s.solves[0].kind.value(), 0.5);
    // 外すものが無い面・曲率は注意だけ
    for ops in [
        r#"[{"op": "remove_solve", "surface": 2}]"#,
        r#"[{"op": "remove_solve", "surface": 6, "what": "c"}]"#,
        r#"[{"op": "remove_pickup", "dst": 6, "what": "t"}]"#,
    ] {
        let (s, warn) = a0(ops);
        assert_eq!(s.solves.len(), 1, "{ops}");
        assert!(
            warn.iter().any(|w| w.contains("nothing to remove")),
            "{ops}: {warn:?}"
        );
    }
}
