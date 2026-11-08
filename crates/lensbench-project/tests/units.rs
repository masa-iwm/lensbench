//! ブロック・群の id が玉・群のパラメータになる

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::{expand, expand_report, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

const HEAD: &str = r#""glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "epd", "value": 10},
  "fields": {"type": "angle", "points": [[0, 0], [0, 14]]},"#;

fn project(blocks: &str) -> System {
    let p = Project::from_json(&format!("{{ {HEAD} \"blocks\": {blocks} }}")).unwrap();
    expand(&p, &env()).unwrap()
}

fn a0() -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

fn a0_blocks(a: f64, b: f64, group: &str) -> String {
    let s = a0();
    let t: Vec<f64> = s.surfaces.iter().map(|d| d.t).collect();
    format!(
        r#"[{{"type": "lens", "id": "L1", "source": {{"zmx": "A0/system.zmx", "lens": 1}}}}, {{"type": "air", "id": "d1", "t": {a}}},
            {{"type": "group", "id": "g2", "blocks": [
               {{"type": "lens", "id": "L2", "source": {{"zmx": "A0/system.zmx", "lens": 2}}}}]{group}}},
            {{"type": "air", "id": "d2", "t": {b}}},
            {{"type": "stop", "semi": {}}}, {{"type": "air", "t": {}}},
            {{"type": "lens", "id": "L3", "source": {{"zmx": "A0/system.zmx", "lens": 3}}}}, {{"type": "air", "t": {}}},
            {{"type": "surfaces", "surfaces": [{{"t": {}}}]}}]"#,
        s.surfaces[5].semi, t[5], t[7], t[8]
    )
}

fn same_surfaces(a: &System, b: &System) {
    assert_eq!(a.surfaces.len(), b.surfaces.len());
    for (j, (x, y)) in a.surfaces.iter().zip(&b.surfaces).enumerate() {
        assert_eq!(format!("{x:?}"), format!("{y:?}"), "面 {j}");
    }
}

#[test]
fn block_ids_become_units() {
    let a = a0();
    let (t2, t4) = (a.surfaces[2].t, a.surfaces[4].t);
    let s = project(&a0_blocks(t2, t4, ""));
    let names: Vec<&str> = s.units.iter().map(|u| u.name.as_str()).collect();
    // 空気（面がない）は玉・群にしない
    assert_eq!(names, ["lens1", "lens2", "lens3", "L1", "L2", "g2", "L3"]);
    let u = |n: &str| s.units[s.find_unit(n).unwrap()].spans.clone();
    assert_eq!(u("L1"), vec![[1, 2]]);
    assert_eq!(u("g2"), vec![[3, 4]]);
    // 群の移動のパラメータ = プロジェクトの shift（面の列がビットまで同じ）
    let sh = 1.25;
    let by_param = s
        .with_params(&[(s.param("g2.shift").unwrap(), sh)])
        .unwrap();
    let by_block = project(&a0_blocks(t2, t4, &format!(r#", "shift": {sh}"#)));
    same_surfaces(&by_param, &by_block);
    // 偏心・傾きもプロジェクトの place と同じ位置へ（展開は座標ブレークを入れる）
    let m = s
        .with_params(&[
            (s.param("L2.dy").unwrap(), 0.1),
            (s.param("L2.tx").unwrap(), 0.5),
        ])
        .unwrap();
    let placed = project(&a0_blocks(t2, t4, "").replace(
        r#""id": "L2","#,
        r#""id": "L2", "decenter": [0, 0.1], "tilt": [0.5, 0, 0],"#,
    ));
    let (fm, fp) = (m.frames(), placed.frames());
    // 玉 2 の面（展開では座標ブレークの次）と、その後の面
    for (j, k) in [(3, 4), (4, 5), (5, 7), (8, 10)] {
        let (x, y) = (fm.surf[j].unwrap(), fp.surf[k].unwrap());
        for i in 0..3 {
            assert!(
                (x.o[i] - y.o[i]).abs() < 1e-12,
                "{j} {k}: {:?} {:?}",
                x.o,
                y.o
            );
        }
    }
}

#[test]
fn a_lens_in_a_double_pass_is_one_unit_with_two_spans() {
    let s = project(
        r#"[{"type": "stop", "semi": 5}, {"type": "air", "t": 20},
            {"type": "group", "id": "G", "blocks": [{"type": "lens", "id": "L", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}}]},
            {"type": "air", "t": 30}, {"type": "mirror", "id": "M", "back": true}]"#,
    );
    let u = s.find_unit("L").unwrap();
    assert_eq!(s.units[u].spans, vec![[2, 3], [5, 6]]);
    assert_eq!(s.units[s.find_unit("M").unwrap()].spans, vec![[4, 4]]);
    let m = s
        .with_params(&[
            (s.param("G.dy").unwrap(), 0.05),
            (s.param("L.tx").unwrap(), 0.2),
        ])
        .unwrap();
    let f = m.frames();
    for (x, y) in [(2, 6), (3, 5)] {
        let (a, b) = (f.surf[x].unwrap(), f.surf[y].unwrap());
        for i in 0..3 {
            assert!((a.o[i] - b.o[i]).abs() < 1e-12);
        }
    }
    // 群の移動は往復とも
    let m = s
        .with_params(&[(s.param("G.shift").unwrap(), 2.5)])
        .unwrap();
    let t: Vec<f64> = m.surfaces.iter().map(|d| d.t).collect();
    assert_eq!(&t[1..7], &[22.5, 4.0, 27.5, -27.5, -4.0, -22.5]);
}

#[test]
fn ids_that_cannot_be_units_are_warned() {
    // id「3」（数字だけ）は玉・群の名前にできない
    let p = Project::from_json(&format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "id": "3"}}, {{"type": "air", "t": 10}}] }}"#
    ))
    .unwrap();
    let (s, warn) = expand_report(&p, &env()).unwrap();
    assert!(s.find_unit("3").is_none());
    assert!(warn.iter().any(|w| w.contains("ブロック 3")), "{warn:?}");
}
