//! 機構部品のケラレ: 系に置いた穴（円柱）で切れる光線を数える
use lensbench_core::mech::vignetting;
use lensbench_core::system::FieldSel;
use lensbench_project::{expand, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("examples/lenses/LBP"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// 平行光（EPD 10）が絞りから 40 mm 進んで単レンズに入る系。`mechanics` は置く機構部品、
/// `aperture` は絞りから 10 mm に置く円の開口の半径（比べる用）
fn system(mechanics: &str, aperture: Option<f64>) -> lensbench_core::System {
    let ap = match aperture {
        Some(r) => format!(
            r#"{{"type": "air", "t": 10}}, {{"type": "surfaces", "id": "ap", "surfaces": [{{"aperture": {{"type": "circular", "max": {r}}}}}]}}, {{"type": "air", "t": 30}}"#
        ),
        None => r#"{"type": "air", "t": 40}"#.into(),
    };
    let p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
        "fields": {{"points": [[0, 0], [0, 5]]}},
        "blocks": [{{"type": "stop", "id": "stop"}}, {ap},
          {{"type": "lens", "id": "L1", "source": {{"surfaces": [{{"radius": 50, "t": 4, "glass": "N-BK7"}}, {{"radius": -50}}]}}}},
          {{"type": "air", "t": 45}}],
        "mechanics": [{mechanics}]}}"#
    ))
    .unwrap();
    expand(&p, &env()).unwrap()
}

fn pupil(n: usize) -> (Vec<f64>, Vec<f64>) {
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for iy in 0..n {
        for ix in 0..n {
            let x = -1.0 + 2.0 * ix as f64 / (n - 1) as f64;
            let y = -1.0 + 2.0 * iy as f64 / (n - 1) as f64;
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// 長さ 0 の穴は、同じ位置の円の開口と同じ光線を切る
#[test]
fn thin_bore_matches_a_circular_aperture() {
    let (px, py) = pupil(31);
    for r in [3.5, 4.6] {
        let m = system(
            &format!(
                r#"{{"name": "ring", "mech": {{"bores": [{{"z0": 10, "z1": 10, "diameter": {}}}]}}, "at": {{"block": "stop"}}}}"#,
                2.0 * r
            ),
            None,
        );
        assert_eq!(m.mechanics.len(), 1);
        let a = system("", Some(r));
        let v = vignetting(&m, &m.mechanics, &[0, 1], &[0], &px, &py).unwrap();
        for (k, f) in [0usize, 1].iter().enumerate() {
            let t = a
                .try_trace(FieldSel::Index(*f), &px, &py, Some(0), None, false)
                .unwrap();
            let cut = t.rays.iter().filter(|x| x.vig != 0).count();
            assert!(cut > 0);
            assert_eq!(v[k].clipped[0], cut, "半径 {r} 視野 {f}");
            assert_eq!(v[k].first[0], cut);
            assert_eq!(v[k].passed, px.len() - cut);
            assert_eq!(v[k].optical, 0);
        }
    }
}

/// チューブ（長さのある穴）: 斜めの視野は、穴の出口の端で切れる。軸上は内径が光束より太ければ切れない
#[test]
fn tube_clips_the_oblique_field_at_its_exit() {
    let (px, py) = pupil(31);
    // 絞りから 5〜35 mm、内径 11（光束の半径 5、5° の視野は 35 mm で 3.06 mm ずれる）
    let m = system(
        r#"{"name": "tube", "mech": {"bores": [{"z0": 5, "z1": 35, "diameter": 11}], "length": 30, "outer_diameter": 14}, "at": {"block": "stop"}}"#,
        None,
    );
    let v = vignetting(&m, &m.mechanics, &[0, 1], &[0], &px, &py).unwrap();
    assert_eq!(v[0].clipped[0], 0);
    // 出口（35 mm）の位置の円の開口と同じ
    let thin = system(
        r#"{"mech": {"bores": [{"z0": 35, "z1": 35, "diameter": 11}]}, "at": {"block": "stop"}}"#,
        None,
    );
    let w = vignetting(&thin, &thin.mechanics, &[1], &[0], &px, &py).unwrap();
    assert!(v[1].clipped[0] > 0);
    assert_eq!(v[1].clipped[0], w[0].clipped[0]);
}

/// 2 つの部品: 最初に切った部品にだけ first を数える。ブロックの面と offset で置ける
#[test]
fn first_clip_goes_to_the_earlier_part() {
    let (px, py) = pupil(21);
    let m = system(
        r#"{"name": "front", "mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 8}]}, "at": {"block": "stop"}, "offset": [0, 0, 5]},
           {"name": "back", "mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 7}]}, "at": {"block": "L1", "surface": 1}, "offset": [0, 0, 2]}"#,
        None,
    );
    let v = vignetting(&m, &m.mechanics, &[0], &[0], &px, &py).unwrap();
    let r = &v[0];
    // 軸上の平行光（半径 5）は前の穴（半径 4）で切れ、レンズの後で光束は細くなる
    assert!(r.first[0] > 0);
    assert!(r.clipped[0] >= r.first[0]);
    assert_eq!(r.first[0] + r.first[1] + r.passed + r.optical, r.launched);
    assert!(r.clipped[1] >= r.first[1]);
    // 置いた面: stop は面 1、L1 の面 1（後面）は系の面 3
    assert_eq!(m.mechanics[0].surface, 1);
    assert_eq!(m.mechanics[1].surface, 3);
}

/// カタログの機構部品（LBP の例）と、誤りの書き方
#[test]
fn catalog_parts_and_errors() {
    let m = system(
        r#"{"catalog": "parts.json", "item": "TUBE-25-30", "at": {"block": "stop"}}"#,
        None,
    );
    assert_eq!(m.mechanics[0].name, "TUBE-25-30");
    assert!(!m.mechanics[0].bores.is_empty());
    for bad in [
        // 機構でない品
        r#"{"catalog": "parts.json", "item": "WIN-254-3", "at": {"block": "stop"}}"#,
        // 部品が無い
        r#"{"at": {"block": "stop"}}"#,
        // 内径が 0
        r#"{"mech": {"bores": [{"z0": 0, "z1": 1, "diameter": 0}]}, "at": {"surface": 1}}"#,
        // 面の番号が範囲外
        r#"{"mech": {"bores": [{"z0": 0, "z1": 1, "diameter": 5}]}, "at": {"surface": 99}}"#,
    ] {
        let p = Project::from_json(&format!(
            r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
            "blocks": [{{"type": "stop", "id": "stop"}}, {{"type": "air", "t": 40}}], "mechanics": [{bad}]}}"#
        ))
        .unwrap();
        assert!(expand(&p, &env()).is_err(), "{bad}");
    }
}

/// 配置の確認の系: 絞りから 20 mm に厚さ 5・半径 12.7 の単レンズ L1
fn fit_system(mechanics: &str) -> lensbench_core::System {
    let p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
        "blocks": [{{"type": "stop", "id": "stop"}}, {{"type": "air", "t": 20}},
          {{"type": "lens", "id": "L1", "source": {{"surfaces": [{{"radius": 60, "t": 5, "glass": "N-BK7", "semi": 12.7}}, {{"radius": -60, "semi": 12.7}}]}}}},
          {{"type": "air", "t": 50}}],
        "mechanics": [{mechanics}]}}"#
    ))
    .unwrap();
    expand(&p, &env()).unwrap()
}

fn kinds(s: &lensbench_core::System) -> Vec<(&'static str, usize, Option<usize>, f64)> {
    lensbench_core::mech::fit_check(s, &s.mechanics)
        .unwrap()
        .into_iter()
        .map(|f| (f.kind, f.part, f.other, f.value))
        .collect()
}

/// 部品の断面の輪: 穴の端で区切り、保持径があれば内側はそれと太い方
#[test]
fn profile_rings() {
    let s = fit_system(
        r#"{"mech": {"bores": [{"z0": 0, "z1": 3, "diameter": 20}, {"z0": 3, "z1": 10, "diameter": 24}], "length": 12, "outer_diameter": 30, "holds": {"diameter": 22}}, "at": {"block": "stop"}}"#,
    );
    let p = &s.mechanics[0];
    assert_eq!(
        p.profile(),
        vec![
            [0.0, 3.0, 11.0, 15.0],
            [3.0, 10.0, 12.0, 15.0],
            [10.0, 12.0, 0.0, 15.0]
        ]
    );
    // 外径なしは一番太い穴 + 1.5
    let t = fit_system(
        r#"{"mech": {"bores": [{"z0": 0, "z1": 3, "diameter": 20}]}, "at": {"block": "stop"}}"#,
    );
    assert_eq!(t.mechanics[0].outer(), 11.5);
}

#[test]
fn fit_collision_protrude_loose() {
    // チューブ（保持径 25.4）に収まる: 何もない
    let ok = fit_system(
        r#"{"catalog": "parts.json", "item": "TUBE-25-30", "at": {"block": "stop"}, "offset": [0, 0, 5], "holds": ["L1"]}"#,
    );
    assert_eq!(ok.mechanics[0].holds, vec![2, 3]);
    assert!(kinds(&ok).is_empty(), "{:?}", kinds(&ok));
    // 内径 20 の筒: レンズの縁（半径 12.7）が壁に 2.7 mm 当たる（前後の面）
    let hit = fit_system(
        r#"{"mech": {"bores": [{"z0": 0, "z1": 40, "diameter": 20}], "outer_diameter": 30}, "at": {"block": "stop"}}"#,
    );
    let k = kinds(&hit);
    assert_eq!(k.len(), 2);
    for (kind, _, _, v) in &k {
        assert_eq!(*kind, "collision");
        assert!((v - 2.7).abs() < 1e-9);
    }
    // 長さ 22 の筒が L1 を保持: L1 は 20〜25 なので後ろが 3 mm 余りはみ出す（縁の矢高の分だけ短い）
    let out = fit_system(
        r#"{"mech": {"bores": [{"z0": 0, "z1": 22, "diameter": 23}], "outer_diameter": 30, "holds": {"diameter": 25.4}}, "at": {"block": "stop"}, "holds": ["L1"]}"#,
    );
    let k = kinds(&out);
    assert_eq!(k.iter().filter(|x| x.0 == "protrude").count(), 1, "{k:?}");
    let v = k.iter().find(|x| x.0 == "protrude").unwrap().3;
    assert!((v - 3.0).abs() < 1e-9, "{v}");
    // 保持径 30 に 25.4 のレンズ: 細い（注意）
    let loose = fit_system(
        r#"{"mech": {"bores": [{"z0": 0, "z1": 40, "diameter": 28}], "outer_diameter": 34, "holds": {"diameter": 30}}, "at": {"block": "stop"}, "holds": ["L1"]}"#,
    );
    let f = lensbench_core::mech::fit_check(&loose, &loose.mechanics).unwrap();
    assert_eq!(f.len(), 2);
    assert!(f
        .iter()
        .all(|x| x.kind == "loose" && !x.error && (x.value - 2.3).abs() < 1e-9));
}

#[test]
fn fit_overlap_and_cage() {
    let tube = |z: f64, d: f64, o: f64| {
        format!(
            r#"{{"mech": {{"bores": [{{"z0": 0, "z1": 10, "diameter": {d}}}], "outer_diameter": {o}}}, "at": {{"block": "L1", "surface": 1}}, "offset": [0, 0, {z}]}}"#
        )
    };
    // 同じ位置に 2 本は重なる。端を突き合わせるのは重ならない。太い筒の穴の中の細い筒は重ならない
    let s = fit_system(&[tube(5.0, 20.0, 30.0), tube(8.0, 20.0, 30.0)].join(","));
    assert_eq!(kinds(&s), vec![("overlap", 0, Some(1), 0.0)]);
    let s = fit_system(&[tube(5.0, 20.0, 30.0), tube(15.0, 20.0, 30.0)].join(","));
    assert!(kinds(&s).is_empty());
    let s = fit_system(&[tube(5.0, 32.0, 40.0), tube(5.0, 20.0, 30.0)].join(","));
    assert!(kinds(&s).is_empty());
    // 横にずらした筒は重ならない
    let s = fit_system(&format!(
        "{},{}",
        tube(5.0, 20.0, 30.0),
        r#"{"mech": {"bores": [{"z0": 0, "z1": 10, "diameter": 20}], "outer_diameter": 30}, "at": {"block": "L1", "surface": 1}, "offset": [31, 0, 5]}"#
    ));
    assert!(kinds(&s).is_empty());
    // ケージ板 2 枚（間隔 30）: 板の前の端から後ろの端まで 8 + 30 + 8 = 46 mm のロッド
    let cage = |z: f64, sp: f64| {
        format!(
            r#"{{"mech": {{"bores": [{{"z0": 0, "z1": 8, "diameter": 22}}], "outer_diameter": 41, "rod_spacing": {sp}}}, "at": {{"block": "L1", "surface": 1}}, "offset": [0, 0, {z}]}}"#
        )
    };
    let s = fit_system(&[cage(5.0, 30.0), cage(43.0, 30.0)].join(","));
    let k = kinds(&s);
    assert_eq!(k.len(), 1);
    assert_eq!((k[0].0, k[0].1, k[0].2), ("rod", 0, Some(1)));
    assert!((k[0].3 - 46.0).abs() < 1e-9);
    let s = fit_system(&[cage(5.0, 30.0), cage(43.0, 60.0)].join(","));
    let k = kinds(&s);
    assert!(k.iter().any(|x| x.0 == "rod_mismatch"));
}

/// 枠付きのユーザーカタログの品（`frame`）を置くと、枠がガラスの最初の面に機構部品として付く。
/// 枠の座（0.4 mm）がガラスの前面に来て、縁（内径 21.2）が太い光束を切り、ガラスは枠に当たらない
#[test]
fn catalog_item_frame_is_placed_on_the_glass() {
    let dir = std::env::temp_dir().join(format!("lensbench-frame-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cat = dir.join("framed.json");
    std::fs::write(
        &cat,
        r#"{"version": 2, "vendor": "LB", "items": {"F25": {
            "glass_catalogs": ["SCHOTT"], "kind": "filter",
            "blocks": [{"type": "lens", "source": {"surfaces": [{"t": 2, "glass": "N-BK7", "semi": 11.95}, {"semi": 11.95}]}}],
            "frame": {"bores": [{"z0": 0, "z1": 0.4, "diameter": 21.2}, {"z0": 0.4, "z1": 3.5, "diameter": 24.08}],
                      "length": 3.5, "outer_diameter": 25, "holds": {"diameter": 24.08, "seat_z": 0.4}}}}}"#,
    )
    .unwrap();
    let sys = |epd: f64| {
        let p = Project::from_json(&format!(
            r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": {epd}}},
            "fields": {{"points": [[0, 0]]}},
            "blocks": [{{"type": "stop", "id": "stop"}}, {{"type": "air", "t": 10}},
              {{"type": "user", "id": "F", "catalog": {:?}, "item": "F25"}},
              {{"type": "air", "t": 20}}]}}"#,
            cat.to_str().unwrap()
        ))
        .unwrap();
        expand(&p, &env()).unwrap()
    };
    let s = sys(24.0);
    assert_eq!(s.mechanics.len(), 1);
    let m = &s.mechanics[0];
    assert_eq!(m.name, "F の枠");
    assert_eq!(m.offset, [0.0, 0.0, -0.4]);
    assert_eq!(m.holds.len(), 2);
    assert_eq!(m.surface, m.holds[0]);
    assert!(!s.surfaces[m.surface].mat.is_empty(), "ガラスの前面");
    let fit = lensbench_core::mech::fit_check(&s, &s.mechanics).unwrap();
    assert!(fit.is_empty(), "{fit:?}");
    let (px, py) = pupil(41);
    let v = vignetting(&s, &s.mechanics, &[0], &[0], &px, &py).unwrap();
    assert!(v[0].clipped[0] > 0);
    // 光束が縁より細ければ切れない
    let s = sys(20.0);
    let v = vignetting(&s, &s.mechanics, &[0], &[0], &px, &py).unwrap();
    assert_eq!(v[0].clipped[0], 0);
    // 逆向き（光は枠の z の大きい端から）: 縁は後ろの端に来て、座はガラスの前面（3.5 − 0.4 − 2 = 1.1）
    let txt = std::fs::read_to_string(&cat).unwrap().replace(
        r#""kind": "filter","#,
        r#""kind": "filter", "frame_reversed": true,"#,
    );
    std::fs::write(&cat, txt).unwrap();
    let s = sys(24.0);
    let m = &s.mechanics[0];
    assert!((m.offset[2] + 1.1).abs() < 1e-12, "{:?}", m.offset);
    let b: Vec<[f64; 3]> = m
        .bores
        .iter()
        .map(|b| [b.z0, b.z1, 2.0 * b.radius])
        .collect();
    assert!((b[0][1] - 3.1).abs() < 1e-12 && b[0][2] == 24.08, "{b:?}");
    assert!((b[1][0] - 3.1).abs() < 1e-12 && b[1][2] == 21.2, "{b:?}");
    let fit = lensbench_core::mech::fit_check(&s, &s.mechanics).unwrap();
    assert!(fit.is_empty(), "{fit:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `seat`: 部品の座（holds.seat_z）を、保持する最初の素子の前面に合わせて置く（at は省略できる）
#[test]
fn seat_places_the_part_on_the_held_element() {
    let s = fit_system(
        r#"{"name": "cell", "mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 22}, {"z0": 2, "z1": 12, "diameter": 25.6}], "outer_diameter": 30, "holds": {"diameter": 25.6, "seat_z": 2}}, "holds": ["L1"], "seat": true, "offset": [0, 0, 0.5]}"#,
    );
    let m = &s.mechanics[0];
    assert_eq!(m.surface, m.holds[0], "L1 の前面");
    assert_eq!(m.offset, [0.0, 0.0, -1.5]);
    assert!(kinds(&s).is_empty(), "{:?}", kinds(&s));
    // 座の位置の無い部品・保持する素子の無い部品・置く所（at も seat も）の無い部品は誤り
    for bad in [
        r#"{"mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 22}]}, "holds": ["L1"], "seat": true}"#,
        r#"{"mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 22}]}, "holds": ["L1"]}"#,
        r#"{"mech": {"bores": [{"z0": 0, "z1": 2, "diameter": 22}], "holds": {"diameter": 25.6, "seat_z": 2}}, "seat": true}"#,
    ] {
        let p = Project::from_json(&format!(
            r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
            "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 20}},
              {{"type": "lens", "id": "L1", "source": {{"surfaces": [{{"radius": 60, "t": 5, "glass": "N-BK7", "semi": 12.7}}, {{"radius": -60, "semi": 12.7}}]}}}},
              {{"type": "air", "t": 50}}], "mechanics": [{bad}]}}"#
        ))
        .unwrap();
        assert!(expand(&p, &env()).unwrap_err().contains("seat"), "{bad}");
    }
}

/// `after`: 前の部品の後ろの端につなぐ。オスのねじ（前の端）は前の部品のメスにねじの長さだけ重なる。
/// ねじが合わなければ注記して突き当てる
#[test]
fn after_chains_parts_with_threads() {
    let tube = |name: &str, after: &str, front_male: bool| {
        format!(
            r#"{{"name": "{name}", "mech": {{"bores": [{{"z0": 0, "z1": 10, "diameter": 26.3}}], "length": 10, "outer_diameter": 30.5,
            "front_thread": {{"thread": "SM1", "male": {front_male}, "length": 3}},
            "back_thread": {{"thread": "SM1", "male": false, "length": 7}}}}{after}}}"#
        )
    };
    let first = tube(
        "t1",
        r#", "at": {"block": "L1"}, "offset": [0, 0, -2]"#,
        true,
    );
    let second = tube("t2", r#", "after": "t1""#, true);
    let third = tube("t3", r#", "after": "t2", "offset": [0, 0, 1]"#, false);
    let p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
        "blocks": [{{"type": "stop", "id": "stop"}}, {{"type": "air", "t": 20}},
          {{"type": "lens", "id": "L1", "source": {{"surfaces": [{{"radius": 60, "t": 5, "glass": "N-BK7", "semi": 12.7}}, {{"radius": -60, "semi": 12.7}}]}}}},
          {{"type": "air", "t": 50}}],
        "mechanics": [{first}, {second}, {third}]}}"#
    ))
    .unwrap();
    let (s, warn) = lensbench_project::expand_report(&p, &env()).unwrap();
    let m = &s.mechanics;
    assert_eq!((m[1].surface, m[2].surface), (m[0].surface, m[0].surface));
    // t2 のオス（3 mm）が t1 のメスにねじ込む: -2 + 10 - 3
    assert!((m[1].offset[2] - 5.0).abs() < 1e-12, "{:?}", m[1].offset);
    // t3 の前はメスで t2 の後ろもメス: 合わないので突き当て（+ offset 1）
    assert!((m[2].offset[2] - 16.0).abs() < 1e-12, "{:?}", m[2].offset);
    assert!(
        warn.iter().any(|w| w.contains("ねじが合わない")),
        "{warn:?}"
    );
    // 前に無い部品・at との併用は誤り
    for bad in [
        tube("x", r#", "after": "nope""#, true),
        tube("x", r#", "after": "t1", "at": {"block": "L1"}"#, true),
    ] {
        let p = Project::from_json(&format!(
            r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 10}},
            "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 20}},
              {{"type": "lens", "id": "L1", "source": {{"surfaces": [{{"radius": 60, "t": 5, "glass": "N-BK7", "semi": 12.7}}, {{"radius": -60, "semi": 12.7}}]}}}},
              {{"type": "air", "t": 50}}], "mechanics": [{first}, {bad}]}}"#
        ))
        .unwrap();
        assert!(expand(&p, &env()).unwrap_err().contains("after"), "{bad}");
    }
}

/// `reversed`: 裏返して置く（両端のねじも入れ替わる）。裏返したチューブの後ろのオスに、前がメスのケージ板をつなぐ
#[test]
fn reversed_tube_connects_to_cage_plate() {
    let p = Project::from_json(
        r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 10},
        "blocks": [{"type": "stop", "id": "stop"}, {"type": "air", "t": 20},
          {"type": "lens", "id": "L1", "source": {"surfaces": [{"radius": 60, "t": 5, "glass": "N-BK7", "semi": 12.7}, {"radius": -60, "semi": 12.7}]}},
          {"type": "air", "t": 50}],
        "mechanics": [
          {"name": "tube", "reversed": true, "at": {"block": "L1"},
           "mech": {"bores": [{"z0": 0, "z1": 3, "diameter": 24}, {"z0": 3, "z1": 10, "diameter": 26.3}], "length": 10, "outer_diameter": 30.5,
             "front_thread": {"thread": "SM1", "male": true, "length": 3},
             "back_thread": {"thread": "SM1", "male": false, "length": 7}}},
          {"name": "plate", "after": "tube",
           "mech": {"bores": [{"z0": 0, "z1": 8, "diameter": 26.3}], "length": 8, "outer_diameter": 40.6, "rod_spacing": 30,
             "front_thread": {"thread": "SM1", "male": false, "length": 8}}}]}"#,
    )
    .unwrap();
    let (s, warn) = lensbench_project::expand_report(&p, &env()).unwrap();
    let m = &s.mechanics;
    // 裏返した穴: 細い穴（内径 24）が後ろに
    assert_eq!(
        m[0].bores.last().map(|b| (b.z0, b.z1, b.radius)),
        Some((7.0, 10.0, 12.0))
    );
    // ケージ板はチューブの後ろのオス（3 mm）に重なる: 10 - 3
    assert!((m[1].offset[2] - 7.0).abs() < 1e-12, "{:?}", m[1].offset);
    assert!(!warn.iter().any(|w| w.contains("ねじ")), "{warn:?}");
}

/// 簡易外形 `outline`: 角なら部品は正方形の柱（壁の中か は |x|・|y| の大きい方で見る）。省略は丸
#[test]
fn square_outline_reaches_corners() {
    let p = Project::from_json(
        r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 10},
        "blocks": [{"type": "stop", "id": "stop"}, {"type": "air", "t": 20}],
        "mechanics": [
          {"name": "plate", "at": {"surface": 1},
           "mech": {"bores": [{"z0": 0, "z1": 8, "diameter": 26}], "length": 8, "outer_diameter": 40, "outline": "square"}},
          {"name": "tube", "at": {"surface": 1}, "offset": [0, 0, 10],
           "mech": {"bores": [{"z0": 0, "z1": 8, "diameter": 26}], "length": 8, "outer_diameter": 40}}]}"#,
    )
    .unwrap();
    let (s, _) = lensbench_project::expand_report(&p, &env()).unwrap();
    let m = &s.mechanics;
    assert!(m[0].square && !m[1].square);
    // 角の近く（半径 26.9 > 外径の半径 20）は角の板の壁の中、丸のチューブの外
    let corner = [19.0, 19.0, 4.0];
    assert!(m[0].inside_wall(corner, 1e-9));
    assert!(!m[1].inside_wall(corner, 1e-9));
    // 辺の外は角でも外
    assert!(!m[0].inside_wall([20.5, 0.0, 4.0], 1e-9));
    // 書き戻し: 丸は outline を書かない
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["mechanics"][0]["mech"]["outline"], "square");
    assert!(v["mechanics"][1]["mech"].get("outline").is_none());
}
