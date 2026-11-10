//! JSON の操作の列での系の編集（serve の edit）。

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_project::edit_ops;
use serde_json::json;
use std::path::PathBuf;

fn a0() -> lensbench_core::System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn ops_apply_in_order_and_all_or_nothing() {
    let mut s = a0();
    let n0 = s.surfaces.len();
    let notes = edit_ops::apply(
        &mut s,
        &json!([
            {"op": "insert_surface", "at": 1, "surface": {"t": 0.0, "comment": "D", "aperture": {"type": "circular", "max": 20}}},
            {"op": "set_surface", "surface": "D", "fields": {"semi": 5.0}},
            {"op": "add_pickup", "dst": 1, "src": 2, "what": "t", "scale": 0.0, "offset": 1.0},
            {"op": "set_config_count", "n": 2},
            {"op": "set_config_row", "param": "D.c", "values": [0.0, 0.001]},
            {"op": "remove_config_row", "param": "D.c"},
            {"op": "remove_pickup", "dst": "D", "what": "t"},
            {"op": "delete_surface", "surface": "D"},
        ]),
    )
    .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(s.surfaces.len(), n0);
    let f = edit_ops::surf_fields_json(&s, 0).unwrap();
    assert_eq!(f["t"], json!({"$f": "inf"}));
    // 途中で失敗すると何も変わらない
    let before = s.surfaces.len();
    let r = edit_ops::apply(
        &mut s,
        &json!([
            {"op": "insert_surface", "at": 1},
            {"op": "set_material", "surface": 1, "material": "NO-SUCH"},
        ]),
    );
    assert!(r.unwrap_err().starts_with("操作 2:"));
    assert_eq!(s.surfaces.len(), before);
    assert!(edit_ops::apply(&mut s, &json!([{"op": "nope"}])).is_err());
}

/// ソルブの種類を名前で付け・外す（別の面を参照する種類は面の番号か名前）
#[test]
fn solve_ops() {
    let mut s = a0();
    s.solves.clear();
    let j = s.image() - 1;
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "add_solve", "surface": j, "kind": "marginal_height", "value": 0.0},
            {"op": "add_solve", "surface": j, "kind": "concentric_surface", "other": j - 1},
        ]),
    )
    .unwrap();
    assert_eq!(s.solves.len(), 2);
    assert!(s
        .solves
        .iter()
        .any(|q| q.kind == lensbench_core::param::SolveKind::ConcentricSurface(j - 1)));
    edit_ops::apply(
        &mut s,
        &json!([{"op": "remove_solve", "surface": j, "what": "c"}]),
    )
    .unwrap();
    assert_eq!(s.solves.len(), 1);
    // 別の面が要る種類で other なし・知らない量の from はエラー
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "add_solve", "surface": j, "kind": "position"}])
    )
    .is_err());
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "add_pickup", "dst": 1, "src": 2, "what": "c", "from": "nope"}])
    )
    .is_err());
}

/// 座標ブレークのある系（読込で材料が "-"）も編集できる（で見つけた）
#[test]
fn edit_with_coordinate_breaks() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = zmx::load_with(
        &root.join("tests/projects/P0/template.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let cb = s
        .surfaces
        .iter()
        .position(|d| d.typ == lensbench_core::surface::SurfType::CoordBrk)
        .unwrap();
    assert_eq!(s.surfaces[cb].mat, "-");
    let semi = s.surfaces[1].semi + 1.0;
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_surface", "surface": 1, "fields": {"semi": semi}}]),
    )
    .unwrap();
    assert_eq!(s.surfaces[1].semi, semi);
}

/// 面のデータを全部の項目で読み、書いた項目だけ変える（GUI の面の設定）
#[test]
fn surface_data_roundtrip() {
    let mut s = a0();
    let v = edit_ops::surface_data(&s, 2).unwrap();
    let d = &v["data"];
    for k in [
        "typ", "t", "c", "k", "asph", "semi", "ap", "cb", "before", "after", "mat",
    ] {
        assert!(d.get(k).is_some(), "{k}: {d}");
    }
    assert!(d.get("n").is_none());
    let mat = d["mat"].as_str().unwrap().to_string();
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_surface_data", "surface": 2, "data": {"k": -0.5, "asph": [0.0, 1e-6], "semi": 9.5}},
                {"op": "add_solve", "surface": 2, "kind": "marginal_height"}]),
    )
    .unwrap();
    let d2 = &s.surfaces[2];
    assert_eq!((d2.k, d2.semi), (-0.5, 9.5));
    assert_eq!(d2.asph, vec![0.0, 1e-6]);
    assert_eq!(d2.mat, mat);
    assert!(!d2.n.is_empty());
    let v = edit_ops::surface_data(&s, 2).unwrap();
    assert_eq!(v["solves"][0]["kind"], "marginal_height");
    assert_eq!(v["solves"][0]["what"], "t");
    // 変えない項目・無い項目は断る（系はそのまま）
    for bad in [json!({"stop": true}), json!({"zz": 1}), json!({"k": "x"})] {
        let e = edit_ops::apply(
            &mut s,
            &json!([{"op": "set_surface_data", "surface": 2, "data": bad}]),
        );
        assert!(e.is_err(), "{bad}");
    }
    assert_eq!(s.surfaces[2].k, -0.5);
}

/// 系の設定の読み書き: 書いた項目だけ変わり、屈折率は波長・温度から作り直す
#[test]
fn system_data_roundtrip() {
    let mut s = a0();
    let d = edit_ops::system_data(&s).unwrap();
    for k in edit_ops::SYSTEM_KEYS {
        assert!(d.get(*k).is_some(), "{k} が無い: {d}");
    }
    // 読んだものをそのまま書いても変わらない
    let hash = lensbench_project::sysfile::system_hash(&s).unwrap();
    edit_ops::apply(&mut s, &json!([{"op": "set_system_data", "data": d}])).unwrap();
    assert_eq!(lensbench_project::sysfile::system_hash(&s).unwrap(), hash);

    // 波長を 3 つに（整数の重みも数として読む）、温度を変える
    let n_before = s.surfaces[1].n.clone();
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_system_data", "data": {
            "wavelengths": [0.486, 0.5876, 0.656], "weights": [1, 2, 1], "primary": 1,
            "temperature": 40, "adjust_index": true,
        }}]),
    )
    .unwrap();
    assert_eq!(s.wavelengths, vec![0.486, 0.5876, 0.656]);
    assert_eq!(s.weights, vec![1.0, 2.0, 1.0]);
    assert_eq!((s.primary, s.temperature), (1, 40.0));
    assert_eq!(s.surfaces[1].n.len(), 3);
    assert_ne!(s.surfaces[1].n, n_before);
    // 視野・開口
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_system_data", "data": {
            "fields": [{"x": 0, "y": 0, "w": 1, "vdx": 0, "vdy": 0, "vcx": 0, "vcy": 0, "van": 0},
                       {"x": 0, "y": 3, "w": 1, "vdx": 0, "vdy": 0, "vcx": 0, "vcy": 0, "van": 0}],
            "aperture": d["aperture"],
        }}]),
    )
    .unwrap();
    assert_eq!(s.fields.len(), 2);
    assert_eq!(s.fields[1].y, 3.0);

    // 誤り: 項目が無い・面の項目・重みの数・主波長の範囲。どれも系を変えない
    let h = lensbench_project::sysfile::system_hash(&s).unwrap();
    for bad in [
        json!({"surfaces": []}),
        json!({"weights": [1, 1]}),
        json!({"primary": 3}),
        json!({"wavelengths": []}),
        json!({"glrs": 999}),
    ] {
        let r = edit_ops::apply(&mut s, &json!([{"op": "set_system_data", "data": bad}]));
        assert!(r.is_err(), "{bad} が通った");
    }
    assert_eq!(lensbench_project::sysfile::system_hash(&s).unwrap(), h);
}

/// 玉・群の定義・範囲・名前・外す（範囲を変える・外すのは動かしていないときだけ）
#[test]
fn unit_ops() {
    let mut s = a0();
    let auto: Vec<String> = s.units.iter().map(|u| u.name.clone()).collect();
    assert!(auto.len() >= 2, "{auto:?}");
    let [a, b] = s.units[0].spans[0];
    let last = s.units[1].spans[0][1];
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "define_unit", "name": "G1", "spans": [[a, last]]},
            {"op": "set_unit", "unit": "lens1", "name": "L1"},
        ]),
    )
    .unwrap();
    let g = s.find_unit("G1").unwrap();
    assert_eq!(s.units[g].spans, vec![[a, last]]);
    assert!(s.find_unit("L1").is_some_and(|i| !s.units[i].auto));
    // 範囲を縮める・動かした群は範囲を変えられない
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_unit", "unit": "G1", "spans": [[a, b]], "pivot": [0, 0, 1]}]),
    )
    .unwrap();
    let g = s.find_unit("G1").unwrap();
    assert_eq!(
        (s.units[g].spans.clone(), s.units[g].pivot),
        (vec![[a, b]], [0.0, 0.0, 1.0])
    );
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_param", "param": "G1.dx", "value": 0.1}]),
    )
    .unwrap();
    let e = edit_ops::apply(&mut s, &json!([{"op": "remove_unit", "unit": "G1"}])).unwrap_err();
    assert!(e.contains("動か"), "{e}");
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "set_param", "param": "G1.dx", "value": 0.0},
            {"op": "remove_unit", "unit": "G1"},
        ]),
    )
    .unwrap();
    assert!(s.find_unit("G1").is_none());
    // 一部だけ重なる範囲・数字だけの名前は誤り
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "define_unit", "name": "X", "spans": [[b, last + 1]]}])
    )
    .is_err());
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "define_unit", "name": "12", "spans": [[a, b]]}])
    )
    .is_err());
}

/// 機構部品を置く・変える・外す。面を足す・消すと置く面の番号も付け替える
#[test]
fn mech_ops() {
    let mut s = a0();
    edit_ops::apply(
        &mut s,
        &json!([{"op": "add_mech", "part": {
            "name": "tube", "surface": 2, "offset": [0, 0, -1], "bores": [{"z0": 0, "z1": 10, "radius": 15}], "holds": [2, 3]
        }}]),
    )
    .unwrap();
    assert_eq!(s.mechanics[0].bores[0].radius, 15.0);
    edit_ops::apply(
        &mut s,
        &json!([{"op": "set_mech", "mech": "tube", "part": {"tilt": [1, 0], "surface": 3}}]),
    )
    .unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].tilt),
        (3, [1.0, 0.0])
    );
    // 前に面を足すと番号がずれる、保持する面を消すと外れる
    edit_ops::apply(&mut s, &json!([{"op": "insert_surface", "at": 1}])).unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].holds.clone()),
        (4, vec![3, 4])
    );
    edit_ops::apply(&mut s, &json!([{"op": "delete_surface", "surface": 3}])).unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].holds.clone()),
        (3, vec![3])
    );
    let notes = edit_ops::apply(&mut s, &json!([{"op": "delete_surface", "surface": 3}])).unwrap();
    assert!(
        s.mechanics.is_empty() && notes.iter().any(|n| n.contains("tube")),
        "{notes:?}"
    );
    // 誤り: 名前の重なり・面の範囲・穴の寸法・無い項目
    let part = json!({"name": "m", "surface": 1, "bores": []});
    edit_ops::apply(&mut s, &json!([{"op": "add_mech", "part": part}])).unwrap();
    assert!(edit_ops::apply(&mut s, &json!([{"op": "add_mech", "part": part}])).is_err());
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "set_mech", "mech": 0, "part": {"surface": 999}}])
    )
    .is_err());
    assert!(edit_ops::apply(&mut s, &json!([{"op": "set_mech", "mech": 0, "part": {"bores": [{"z0": 1, "z1": 0, "radius": 1}]}}])).is_err());
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "set_mech", "mech": "m", "part": {"what": 1}}])
    )
    .is_err());
    edit_ops::apply(&mut s, &json!([{"op": "remove_mech", "mech": "m"}])).unwrap();
    assert!(s.mechanics.is_empty());
}

/// 面の複屈折（birefringent の入口・出口）を面のデータで付け・外す。異常光の屈折率は材料（名前-E）から作る
#[test]
fn bire_by_surface_data() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let s0 = zmx::load_with(&root.join("tests/birefringent/b2.zmx"), &opt).unwrap();
    let jin = s0
        .surfaces
        .iter()
        .position(|d| matches!(d.bire, Some(lensbench_core::surface::Bire::In { .. })))
        .unwrap();
    let jout = s0
        .surfaces
        .iter()
        .position(|d| matches!(d.bire, Some(lensbench_core::surface::Bire::Out)))
        .unwrap();
    let din = edit_ops::surface_data(&s0, jin).unwrap()["data"]["bire"].clone();
    assert!(din["In"]["axis"].is_array(), "{din}");
    let mut s = s0.clone();
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "set_surface_data", "surface": jin, "data": {"bire": null}},
            {"op": "set_surface_data", "surface": jout, "data": {"bire": null}},
        ]),
    )
    .unwrap();
    assert!(s.surfaces[jin].bire.is_none());
    // 付け直す（ne は書かなくても材料から）
    let mut d = din.clone();
    d["In"].as_object_mut().unwrap().remove("ne");
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "set_surface_data", "surface": jin, "data": {"bire": d}},
            {"op": "set_surface_data", "surface": jout, "data": {"bire": "Out"}},
        ]),
    )
    .unwrap();
    assert_eq!(s.surfaces[jin].bire, s0.surfaces[jin].bire);
    assert_eq!(s.surfaces[jout].bire, s0.surfaces[jout].bire);
    // 波長を変えると異常光の屈折率も作り直す
    let mut w = s.clone();
    edit_ops::apply(&mut w, &json!([{"op": "set_system_data", "data": {"wavelengths": [0.6328], "weights": [1.0], "primary": 0}}])).unwrap();
    match (&w.surfaces[jin].bire, &s0.surfaces[jin].bire) {
        (
            Some(lensbench_core::surface::Bire::In { ne, .. }),
            Some(lensbench_core::surface::Bire::In { ne: ne0, .. }),
        ) => {
            assert_eq!(ne.len(), 1);
            let k = s0
                .wavelengths
                .iter()
                .position(|&l| (l - 0.6328).abs() < 1e-9)
                .unwrap();
            assert!((ne[0] - ne0[k]).abs() < 1e-12, "{ne:?} {ne0:?}");
        }
        x => panic!("{x:?}"),
    }
}
