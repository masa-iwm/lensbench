//! ユーザーカタログ（玉・ブロックの保存と他のプロジェクトでの再利用）の自己照合

use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::{block_item, expand, lens_item, Env, Project, UserCatalog};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env_at(base: &Path) -> Env<'static> {
    Env {
        base: base.to_path_buf(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn a0() -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lb-cat-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 像面の光線を比べる
fn same_rays(a: &System, b: &System, tol: f64) {
    let (px, py) = (vec![0.0, 0.7, -0.5, 0.3], vec![0.0, 0.2, 0.6, -0.9]);
    for f in 0..a.fields.len() {
        let t0 = a
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        let t1 = b
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        for i in 0..px.len() {
            assert_eq!(t0.rays[i].err, t1.rays[i].err);
            for k in 0..3 {
                assert!(
                    (t0.rays[i].xyz[k] - t1.rays[i].xyz[k]).abs() < tol,
                    "{f} {i} {:?} {:?}",
                    t0.rays[i].xyz,
                    t1.rays[i].xyz
                );
            }
        }
    }
}

const HEAD: &str = r#""glass_catalogs": [],
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "epd", "value": 10},
  "fields": {"type": "angle", "points": [[0, 0], [0, 14], [0, 20], [10, 10]]},"#;

#[test]
fn saved_lenses_rebuild_the_triplet_in_another_project() {
    let a = a0();
    let dir = tmp("lens");
    let mut cat = UserCatalog::default();
    for k in 1..=3 {
        cat.items
            .insert(format!("A0 lens {k}"), lens_item(&a, k, "").unwrap());
    }
    assert_eq!(cat.items["A0 lens 1"].glass_catalogs, ["SCHOTT"]);
    cat.save(&dir.join("my.json")).unwrap();
    // 読み戻しても同じ
    let back = UserCatalog::load(&dir.join("my.json")).unwrap();
    assert_eq!(back.to_json(), cat.to_json());
    let t: Vec<f64> = a.surfaces.iter().map(|s| s.t).collect();
    // プロジェクトのガラスカタログは空: 項目のカタログ（SCHOTT）が足される
    let txt = format!(
        r#"{{ {HEAD} "blocks": [
            {{"type": "user", "catalog": "my.json", "item": "A0 lens 1"}}, {{"type": "air", "t": {}}},
            {{"type": "user", "catalog": "my.json", "item": "A0 lens 2"}}, {{"type": "air", "t": {}}},
            {{"type": "stop", "semi": {}}}, {{"type": "air", "t": {}}},
            {{"type": "user", "catalog": "my.json", "item": "A0 lens 3"}}, {{"type": "air", "t": {}}},
            {{"type": "surfaces", "surfaces": [{{"t": {}}}]}}] }}"#,
        t[2], t[4], a.surfaces[5].semi, t[5], t[7], t[8]
    );
    let s = expand(&Project::from_json(&txt).unwrap(), &env_at(&dir)).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(s.surfaces.len(), a.surfaces.len());
    same_rays(&s, &a, 1e-12);
}

#[test]
fn a_saved_group_works_in_another_folder() {
    // P6 の前の群（.zmx の玉 2 つ）を保存し、別のフォルダのプロジェクトで使う
    let ex = root().join("examples/projects");
    let p6 = Project::from_json(
        &std::fs::read_to_string(ex.join("P6_states_focus_group.json")).unwrap(),
    )
    .unwrap();
    let item = block_item(&p6, &env_at(&ex), "front", "front group").unwrap();
    // .zmx の玉は面を直接書いた玉に（.zmx のパスが残らない）
    let js = serde_json::to_string(&item).unwrap();
    assert!(!js.contains("system.zmx"), "{js}");
    let dir = tmp("group");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let mut cat = UserCatalog::default();
    cat.items.insert("front".into(), item);
    cat.save(&dir.join("sub/cat.json")).unwrap();
    // P6 の前の群をユーザーカタログ（別のフォルダ、絶対パス）の参照に置き換える
    let mut v: serde_json::Value = serde_json::to_value(&p6).unwrap();
    let cp = dir.join("sub/cat.json").to_string_lossy().into_owned();
    v["blocks"][0] =
        serde_json::json!({"type": "user", "id": "front", "catalog": cp, "item": "front"});
    let q: Project = serde_json::from_value(v).unwrap();
    let s = expand(&q, &env_at(&ex)).unwrap();
    let s0 = expand(&p6, &env_at(&ex)).unwrap();
    // 状態で ON/OFF・偏心もできる（id は参照のブロックのもの）
    let mut q2 = q.clone();
    q2.states = vec![lensbench_project::State {
        name: "off".into(),
        set: [(
            "front".to_string(),
            serde_json::json!({"on": false, "off": "air"})
                .as_object()
                .unwrap()
                .clone(),
        )]
        .into(),
        ..Default::default()
    }];
    let off = lensbench_project::expand_state(&q2, &env_at(&ex), Some("off")).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    same_rays(&s, &s0, 1e-12);
    assert!(off.surfaces.len() < s.surfaces.len());
}

#[test]
fn a_saved_lens_in_a_double_pass() {
    let dir = tmp("dp");
    let lens = r#"{"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}}"#;
    let cat =
        format!(r#"{{"items": {{"L": {{"glass_catalogs": ["SCHOTT"], "blocks": [{lens}]}}}}}}"#);
    std::fs::write(dir.join("c.json"), cat).unwrap();
    let mk = |b: &str| {
        let txt = format!(
            r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {b},
                {{"type": "air", "t": 30}}, {{"type": "mirror", "radius": -80, "back": true}}] }}"#
        );
        expand(&Project::from_json(&txt).unwrap(), &env_at(&dir))
    };
    let s = mk(r#"{"type": "user", "catalog": "c.json", "item": "L"}"#).unwrap();
    let txt = format!(r#"{{"type": "group", "blocks": [{lens}]}}"#);
    let mut p: serde_json::Value = serde_json::from_str(&format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {txt},
            {{"type": "air", "t": 30}}, {{"type": "mirror", "radius": -80, "back": true}}] }}"#
    ))
    .unwrap();
    p["glass_catalogs"] = serde_json::json!(["SCHOTT"]);
    let s0 = expand(&serde_json::from_value(p).unwrap(), &env_at(&dir)).unwrap();
    let e = mk(r#"{"type": "user", "catalog": "c.json", "item": "nope"}"#).unwrap_err();
    let e2 = mk(r#"{"type": "user", "catalog": "none.json", "item": "L"}"#).unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(format!("{:?}", s.surfaces), format!("{:?}", s0.surfaces));
    assert!(e.contains("nope"), "{e}");
    assert!(e2.contains("none.json"), "{e2}");
}

#[test]
fn a_catalog_item_referring_to_itself_is_an_error() {
    let dir = tmp("cycle");
    // a → a、b → c → b（c は別のファイルから相対パスで）
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("c.json"),
        r#"{"items": {"a": {"blocks": [{"type": "user", "catalog": "c.json", "item": "a"}]},
                      "b": {"blocks": [{"type": "user", "catalog": "sub/d.json", "item": "c"}]}}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("sub/d.json"),
        r#"{"items": {"c": {"blocks": [{"type": "user", "catalog": "../c.json", "item": "b"}]}}}"#,
    )
    .unwrap();
    let mk = |item: &str| {
        let txt = format!(
            r#"{{ {HEAD} "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 5}},
                {{"type": "user", "catalog": "c.json", "item": "{item}"}}, {{"type": "air", "t": 5}}] }}"#
        );
        expand(&Project::from_json(&txt).unwrap(), &env_at(&dir))
    };
    let (ea, eb) = (mk("a").unwrap_err(), mk("b").unwrap_err());
    // 同じ項目を 2 回並べるのは循環ではない
    std::fs::write(
        dir.join("e.json"),
        r#"{"items": {"x": {"blocks": [{"type": "air", "t": 1}]},
                      "y": {"blocks": [{"type": "user", "catalog": "e.json", "item": "x"}, {"type": "user", "catalog": "e.json", "item": "x"}]}}}"#,
    )
    .unwrap();
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop"}}, {{"type": "user", "catalog": "e.json", "item": "y"}}] }}"#
    );
    let ok = expand(&Project::from_json(&txt).unwrap(), &env_at(&dir));
    std::fs::remove_dir_all(&dir).ok();
    assert!(ea.contains("循環"), "{ea}");
    assert!(eb.contains("循環"), "{eb}");
    assert_eq!(ok.unwrap().surfaces[1].t, 2.0);
}

/// 組合せの要素・範囲の保存（GUI）: 面の表の変更（surface_edits）を焼き込み、範囲は間の空気も入れる
#[test]
fn blocks_item_bakes_surface_edits_and_ranges() {
    use lensbench_project::blocks_item;
    let head = HEAD.replace(r#""glass_catalogs": []"#, r#""glass_catalogs": ["SCHOTT"]"#);
    let txt = format!(
        r#"{{ {head} "blocks": [
            {{"type": "stop", "semi": 5}}, {{"type": "air", "t": 2}},
            {{"type": "lens", "id": "L1", "reversed": true, "decenter": [0.1, 0],
              "source": {{"surfaces": [
                {{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -80, "semi": 8}}]}}}},
            {{"type": "air", "t": 3}},
            {{"type": "lens", "id": "L2", "source": {{"surfaces": [
                {{"radius": -40, "t": 2, "glass": "F2", "semi": 8}}, {{"radius": 200, "semi": 8}}]}}}},
            {{"type": "air", "t": 50}}],
          "surface_edits": [{{"block": "L1", "surface": 1, "fields": {{"r": 70.0, "semi": 7.5}}}}] }}"#
    );
    let p = Project::from_json(&txt).unwrap();
    let dir = tmp("bake");
    let env = env_at(&dir);
    let s0 = expand(&p, &env).unwrap();
    // 1 つ: 変更が焼き込まれ、反転は面の向きに入る
    let one = blocks_item(&p, &env, &["L1".into()], "edited").unwrap();
    assert_eq!(one.glass_catalogs, ["SCHOTT"]);
    let js = serde_json::to_value(&one.blocks[0]).unwrap();
    assert_eq!(js["reversed"], false, "{js}");
    assert_eq!(js["decenter"], serde_json::json!([0.1, 0.0]), "{js}");
    assert_eq!(js["source"]["surfaces"][1]["radius"], 70.0, "{js}");
    assert_eq!(js["source"]["surfaces"][1]["semi"], 7.5, "{js}");
    // 範囲: L1・空気・L2
    let range = blocks_item(&p, &env, &["L1".into(), "L2".into()], "").unwrap();
    assert_eq!(range.blocks.len(), 3);
    let mut cat = UserCatalog::default();
    cat.items.insert("pair".into(), range);
    cat.save(&dir.join("my.json")).unwrap();
    // 同じ系を、保存した範囲（変更なし）で組み直すと光線が同じ
    let txt2 = format!(
        r#"{{ {HEAD} "blocks": [
            {{"type": "stop", "semi": 5}}, {{"type": "air", "t": 2}},
            {{"type": "user", "catalog": "my.json", "item": "pair"}},
            {{"type": "air", "t": 50}}] }}"#
    );
    let s1 = expand(&Project::from_json(&txt2).unwrap(), &env).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    same_rays(&s1, &s0, 1e-12);
    // 玉でないブロックの変更は焼き込めない
    let mut q = p.clone();
    q.blocks[0] =
        serde_json::from_value(serde_json::json!({"type": "stop", "id": "S", "semi": 5})).unwrap();
    q.surface_edits = serde_json::from_value(
        serde_json::json!([{"block": "S", "surface": 0, "fields": {"semi": 4.0}}]),
    )
    .unwrap();
    let e = blocks_item(&q, &env, &["S".into(), "L1".into()], "").unwrap_err();
    assert!(e.contains("玉だけ"), "{e}");
}

/// 焼き込みで面の開口・玉の中の座標ブレークが残り、書けない値（面の前後の傾き偏心）は誤り（のレビュー）
#[test]
fn baking_keeps_aperture_and_inner_coordbrk() {
    use lensbench_core::surface::Aperture;
    use lensbench_project::blocks_item;
    let head = HEAD.replace(r#""glass_catalogs": []"#, r#""glass_catalogs": ["SCHOTT"]"#);
    let proj = |edits: &str| {
        format!(
            r#"{{ {head} "blocks": [
            {{"type": "stop", "semi": 5}}, {{"type": "air", "t": 2}},
            {{"type": "lens", "id": "L1", "decenter": [0, 0.05], "source": {{"surfaces": [
                {{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8,
                  "aperture": {{"type": "circular", "max": 4.5}}}},
                {{"coordbrk": [0, 0.02, 0.1, 0, 0]}},
                {{"radius": -80, "semi": 8}}]}}}},
            {{"type": "air", "t": 50}}],
          "surface_edits": {edits} }}"#
        )
    };
    let p = Project::from_json(&proj(
        r#"[{"block": "L1", "surface": 1, "fields": {"r": -70.0}}]"#,
    ))
    .unwrap();
    let dir = tmp("bake2");
    let env = env_at(&dir);
    let s0 = expand(&p, &env).unwrap();
    let item = blocks_item(&p, &env, &["L1".into()], "").unwrap();
    let js = serde_json::to_value(&item.blocks[0]).unwrap();
    let sf = &js["source"]["surfaces"];
    assert_eq!(sf.as_array().unwrap().len(), 3, "{js}");
    assert_eq!(sf[0]["aperture"]["max"], 4.5, "{js}");
    assert_eq!(
        sf[1]["coordbrk"],
        serde_json::json!([0.0, 0.02, 0.1, 0.0, 0.0]),
        "{js}"
    );
    assert_eq!(sf[2]["radius"], -70.0, "{js}");
    let mut cat = UserCatalog::default();
    cat.items.insert("L1".into(), item);
    cat.save(&dir.join("my.json")).unwrap();
    let txt2 = format!(
        r#"{{ {HEAD} "blocks": [
            {{"type": "stop", "semi": 5}}, {{"type": "air", "t": 2}},
            {{"type": "user", "catalog": "my.json", "item": "L1"}},
            {{"type": "air", "t": 50}}] }}"#
    );
    let s1 = expand(&Project::from_json(&txt2).unwrap(), &env).unwrap();
    same_rays(&s1, &s0, 1e-12);
    let aps0: Vec<&Aperture> = s0.surfaces.iter().map(|x| &x.ap).collect();
    let aps1: Vec<&Aperture> = s1.surfaces.iter().map(|x| &x.ap).collect();
    assert_eq!(aps0, aps1);
    // 面の前後の傾き偏心は書けないので誤り
    let q = Project::from_json(&proj(
        r#"[{"block": "L1", "surface": 0, "fields": {"before": {"d": [0.5, 0, 0, 0, 0], "order": 0}}}]"#,
    ))
    .unwrap();
    let e = blocks_item(&q, &env, &["L1".into()], "").unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert!(e.contains("before"), "{e}");
}

/// 標準面でない面（Odd Asphere・トロイダル・Fresnel）の玉も、面を直接書いた玉に保存して同じ光線になる
#[test]
fn saved_lens_keeps_odd_asphere_toroidal_and_fresnel() {
    const ZHEAD: &str =
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 4 -3\nYFLN 0 2 5\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";
    let surf1 = [
        // アキシコン風の Odd Asphere（β1 の円錐 + β3）
        "  TYPE ODDASPHE\n  CURV 0.02\n  PARM 1 -0.05\n  PARM 2 0\n  PARM 3 1e-5\n",
        // 円柱面（回転半径 0）と、x にも曲率のあるトロイダル面
        "  TYPE TOROIDAL\n  CURV 0.04\n  CONI -0.5\n  PARM 1 0\n  PARM 2 0\n  PARM 3 2e-6\n",
        "  TYPE TOROIDAL\n  CURV 0.04\n  PARM 1 60\n",
        // Fresnel（平面の上で、傾きは偶数次非球面の面）
        "  TYPE FRESNELS\n  CURV 0.05\n  CONI -1\n  PARM 1 0\n  PARM 2 1e-6\n",
    ];
    let dir = tmp("shape");
    for (n, s1) in surf1.iter().enumerate() {
        let txt = format!(
            "{ZHEAD}SURF 1\n  STOP\n{s1}  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 40\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        let raw = zmx::parse_text(&txt).unwrap();
        let a = zmx::build(
            &raw,
            &LoadOptions {
                cfg: 1,
                ..Default::default()
            },
            &[],
            None,
        )
        .unwrap();
        let item = lens_item(&a, 1, "").unwrap();
        let js = serde_json::to_string(&item).unwrap();
        let want = ["odd_asph", "toroidal", "toroidal", "fresnel"][n];
        assert!(js.contains(&format!("\"shape\":\"{want}\"")), "{js}");
        assert_eq!(js.contains("radius_x"), n == 2, "{js}");
        let mut cat = UserCatalog::default();
        cat.items.insert("L".into(), item);
        cat.save(&dir.join("s.json")).unwrap();
        let p = r#"{ "glass_catalogs": [],
          "wavelengths": [{"um": 0.5876}], "primary": 1,
          "aperture": {"type": "epd", "value": 10},
          "fields": {"type": "angle", "points": [[0, 0], [4, 2], [-3, 5]]},
          "blocks": [{"type": "stop"}, {"type": "user", "catalog": "s.json", "item": "L"},
            {"type": "air", "t": 40}, {"type": "surfaces", "surfaces": [{"t": 0}]}] }"#;
        let s = expand(&Project::from_json(p).unwrap(), &env_at(&dir)).unwrap();
        same_rays(&s, &a, 1e-12);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 古い書式の .zmx（WAVM が無く WAVL・WWGT に波長と重み）
#[test]
fn old_zmx_wavelength_lines() {
    let txt = "VERS 31125 33\nUNIT MM X W X CM\nENPD 4\nWAVL 0.486 0.5876 0.656\nWWGT 1 2 1\n\
WAVN 0.486 0.5876 0.656 0.55 0.55 0.55 0.55 0.55 0.55 0.55 0.55 0.55\nPWAV 2\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.1\n  DISZ 2\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = zmx::parse_text(txt).unwrap();
    let s = zmx::build(
        &raw,
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap();
    assert_eq!(s.wavelengths, vec![0.486, 0.5876, 0.656]);
    assert_eq!(s.primary, 1);
}
