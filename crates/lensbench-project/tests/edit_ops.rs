//! JSON の操作の列での系の編集（serve の edit）。

use lensbench_formats::zmx::{self, LoadOptions};
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
    let notes = edit_ops::apply_json(
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
    let r = edit_ops::apply_json(
        &mut s,
        &json!([
            {"op": "insert_surface", "at": 1},
            {"op": "set_material", "surface": 1, "material": "NO-SUCH"},
        ]),
    );
    assert!(r
        .unwrap_err()
        .message()
        .unwrap_or_default()
        .starts_with("operation 2:"));
    assert_eq!(s.surfaces.len(), before);
    assert!(edit_ops::apply_json(&mut s, &json!([{"op": "nope"}])).is_err());
}

/// ソルブの種類を名前で付け・外す（別の面を参照する種類は面の番号か名前）
#[test]
fn solve_ops() {
    let mut s = a0();
    s.edit_raw(|rx| {
        rx.solves.clear();
        Ok(())
    })
    .unwrap();
    let j = s.image() - 1;
    edit_ops::apply_json(
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
        .any(|q| q.kind == lensbench_core::solves::SolveKind::ConcentricSurface { other: j - 1 }));
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "remove_solve", "surface": j, "what": "c"}]),
    )
    .unwrap();
    assert_eq!(s.solves.len(), 1);
    // 別の面が要る種類で other なし・知らない量の from はエラー
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "add_solve", "surface": j, "kind": "position"}])
    )
    .is_err());
    assert!(edit_ops::apply_json(
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
    let cb = s.surfaces.iter().position(|d| d.is_coord_break()).unwrap();
    assert_eq!(s.surfaces[cb].mat, "-");
    let semi = s.surfaces[1].semi + 1.0;
    edit_ops::apply_json(
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
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_surface_data", "surface": 2, "data": {"typ": "EvenAsph", "k": -0.5, "asph": [0.0, 1e-6], "semi": 9.5}},
                {"op": "add_solve", "surface": 2, "kind": "marginal_height"}]),
    )
    .unwrap();
    let d2 = &s.surfaces[2];
    assert_eq!(d2.typ(), lensbench_core::surface::SurfType::EvenAsph);
    assert_eq!((d2.k, d2.semi), (-0.5, 9.5));
    assert_eq!(d2.asph(), vec![0.0, 1e-6]);
    assert_eq!(d2.mat, mat.as_str());
    assert!(!s.n(2).is_empty());
    let v = edit_ops::surface_data(&s, 2).unwrap();
    assert_eq!(v["solves"][0]["kind"], "marginal_height");
    assert_eq!(v["solves"][0]["what"], "t");
    // 変えない項目・無い項目は断る（系はそのまま）
    // 種類に無い欄に書いた既定でない値（偶数次非球面の cb、標準面の係数）も断る
    for bad in [
        json!({"stop": true}),
        json!({"zz": 1}),
        json!({"k": "x"}),
        json!({"cb": [0.0, 0.1, 0.0, 0.0, 0.0]}),
        json!({"typ": "Standard", "asph": [0.0, 1e-6]}),
    ] {
        let e = edit_ops::apply_json(
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
    edit_ops::apply_json(&mut s, &json!([{"op": "set_system_data", "data": d}])).unwrap();
    assert_eq!(lensbench_project::sysfile::system_hash(&s).unwrap(), hash);

    // 波長を 3 つに（整数の重みも数として読む）、温度を変える
    let n_before = s.n(1).to_vec();
    edit_ops::apply_json(
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
    assert_eq!(s.n(1).len(), 3);
    assert_ne!(s.n(1), n_before);
    // 視野・開口
    edit_ops::apply_json(
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
        let r = edit_ops::apply_json(&mut s, &json!([{"op": "set_system_data", "data": bad}]));
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
    edit_ops::apply_json(
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
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_unit", "unit": "G1", "spans": [[a, b]], "pivot": [0, 0, 1]}]),
    )
    .unwrap();
    let g = s.find_unit("G1").unwrap();
    assert_eq!(
        (s.units[g].spans.clone(), s.units[g].pivot),
        (vec![[a, b]], [0.0, 0.0, 1.0])
    );
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_param", "param": "G1.dx", "value": 0.1}]),
    )
    .unwrap();
    let e =
        edit_ops::apply_json(&mut s, &json!([{"op": "remove_unit", "unit": "G1"}])).unwrap_err();
    assert!(e.message().unwrap_or_default().contains("is moved"), "{e}");
    edit_ops::apply_json(
        &mut s,
        &json!([
            {"op": "set_param", "param": "G1.dx", "value": 0.0},
            {"op": "remove_unit", "unit": "G1"},
        ]),
    )
    .unwrap();
    assert!(s.find_unit("G1").is_none());
    // 一部だけ重なる範囲・数字だけの名前は誤り
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "define_unit", "name": "X", "spans": [[b, last + 1]]}])
    )
    .is_err());
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "define_unit", "name": "12", "spans": [[a, b]]}])
    )
    .is_err());
}

/// 機構部品を置く・変える・外す。面を足す・消すと置く面の番号も付け替える
#[test]
fn mech_ops() {
    let mut s = a0();
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "add_mech", "part": {
            "name": "tube", "surface": 2, "offset": [0, 0, -1], "bores": [{"z0": 0, "z1": 10, "radius": 15}], "holds": [2, 3]
        }}]),
    )
    .unwrap();
    assert_eq!(s.mechanics[0].bores[0].radius, 15.0);
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_mech", "mech": "tube", "part": {"tilt": [1, 0], "surface": 3}}]),
    )
    .unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].tilt),
        (3, [1.0, 0.0])
    );
    // 前に面を足すと番号がずれる、保持する面を消すと外れる
    edit_ops::apply_json(&mut s, &json!([{"op": "insert_surface", "at": 1}])).unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].holds.clone()),
        (4, vec![3, 4])
    );
    edit_ops::apply_json(&mut s, &json!([{"op": "delete_surface", "surface": 3}])).unwrap();
    assert_eq!(
        (s.mechanics[0].surface, s.mechanics[0].holds.clone()),
        (3, vec![3])
    );
    let notes =
        edit_ops::apply_json(&mut s, &json!([{"op": "delete_surface", "surface": 3}])).unwrap();
    assert!(
        s.mechanics.is_empty() && notes.iter().any(|n| n.contains("tube")),
        "{notes:?}"
    );
    // 誤り: 名前の重なり・面の範囲・穴の寸法・無い項目
    let part = json!({"name": "m", "surface": 1, "bores": []});
    edit_ops::apply_json(&mut s, &json!([{"op": "add_mech", "part": part}])).unwrap();
    assert!(edit_ops::apply_json(&mut s, &json!([{"op": "add_mech", "part": part}])).is_err());
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_mech", "mech": 0, "part": {"surface": 999}}])
    )
    .is_err());
    assert!(edit_ops::apply_json(&mut s, &json!([{"op": "set_mech", "mech": 0, "part": {"bores": [{"z0": 1, "z1": 0, "radius": 1}]}}])).is_err());
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_mech", "mech": "m", "part": {"what": 1}}])
    )
    .is_err());
    edit_ops::apply_json(&mut s, &json!([{"op": "remove_mech", "mech": "m"}])).unwrap();
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
    edit_ops::apply_json(
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
    edit_ops::apply_json(
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
    edit_ops::apply_json(&mut w, &json!([{"op": "set_system_data", "data": {"wavelengths": [0.6328], "weights": [1.0], "primary": 0}}])).unwrap();
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

/// 構成の挿入・削除（ピックアップの番号の付け替え）と構成のピックアップ
#[test]
fn config_insert_remove_and_pickups() {
    let mut s = a0();
    let t = |s: &lensbench_core::System| s.configs().unwrap().rows[0].values.clone();
    let picks = |s: &lensbench_core::System| {
        s.configs().unwrap().rows[0]
            .picks
            .iter()
            .map(|p| (p.k, p.from, p.scale, p.offset))
            .collect::<Vec<_>>()
    };
    edit_ops::apply_json(
        &mut s,
        &json!([
            {"op": "set_config_count", "n": 2},
            {"op": "set_config_row", "param": "1.t", "values": [5.0, 6.0]},
            // ピックアップ付きで最後に（構成 1 から）
            {"op": "insert_config", "at": 3, "pickup": true},
        ]),
    )
    .unwrap();
    assert_eq!(t(&s), vec![5.0, 6.0, 5.0]);
    assert_eq!(picks(&s), vec![(3, 1, 1.0, 0.0)]);
    // 構成 1 の値を変えると構成 3 も追従（今の構成 1 を系で変えても）
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_param", "param": "1.t", "value": 7.0}]),
    )
    .unwrap();
    s.sync_active_config();
    assert_eq!(t(&s), vec![7.0, 6.0, 7.0]);
    // 構成 1 の前に挿入: ピックアップの番号がずれる、今の構成は 2 に
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "insert_config", "at": 1, "from": 2}]),
    )
    .unwrap();
    assert_eq!(t(&s), vec![6.0, 7.0, 6.0, 7.0]);
    assert_eq!(picks(&s), vec![(4, 2, 1.0, 0.0)]);
    assert_eq!(s.configs().unwrap().active, 2);
    // 倍率・ずれ付きのピックアップ、今の構成にすると系に入る
    edit_ops::apply_json(
        &mut s,
        &json!([
            {"op": "set_config_pickup", "param": "1.t", "k": 1, "from": 2, "scale": 2.0, "offset": 1.0},
            {"op": "set_config", "k": 1},
        ]),
    )
    .unwrap();
    assert_eq!(t(&s), vec![15.0, 7.0, 6.0, 7.0]);
    assert_eq!(s.surfaces[1].t, 15.0);
    // 循環は不可、ピックアップの構成の値を直接変えるとピックアップは外れる
    assert!(edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_config_pickup", "param": "1.t", "k": 2, "from": 1}])
    )
    .is_err());
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_config_row", "param": "1.t", "values": [15.0, 7.0, 6.0, 9.0]}]),
    )
    .unwrap();
    assert_eq!(picks(&s), vec![(1, 2, 2.0, 1.0)]);
    // 元の構成 2 を消す: ピックアップは値を残して外れる。今の構成 1 はそのまま
    edit_ops::apply_json(&mut s, &json!([{"op": "remove_config", "k": 2}])).unwrap();
    assert_eq!(t(&s), vec![15.0, 6.0, 9.0]);
    assert!(picks(&s).is_empty());
    assert_eq!(s.configs().unwrap().active, 1);
    // 今の構成を消すと次の構成が系に入る、1 つになると表が消えて残りの値が系に
    edit_ops::apply_json(&mut s, &json!([{"op": "remove_config", "k": 1}])).unwrap();
    assert_eq!(s.configs().unwrap().active, 1);
    assert_eq!(s.surfaces[1].t, 6.0);
    edit_ops::apply_json(&mut s, &json!([{"op": "remove_config", "k": 1}])).unwrap();
    assert!(s.configs().is_none());
    assert_eq!(s.surfaces[1].t, 9.0);
}

/// 操作の列は自動の半直径を最後に 1 回だけ求め直す。結果は 1 つずつ当てたときと同じ。
/// 半直径から曲率を取るピックアップがある（半直径が追跡に効く）ときは操作ごとに求め直すので、それも同じ
/// 別の行からの構成のピックアップ（OpticStudio の元の行）: 同じ量の種類の行から、同じ構成からでもよい。元の行の値に追従し、
/// 循環・違う種類の行は不可。面を挿入すると元の行の面の番号も付け替え、元の行を消すと値を残して外れる
#[test]
fn config_pickup_from_another_row() {
    let mut s = a0();
    let vals = |s: &lensbench_core::System, p: &str| {
        let p = s.param(p).unwrap();
        let c = s.configs().unwrap();
        c.rows.iter().find(|r| r.param == p).unwrap().values.clone()
    };
    edit_ops::apply_json(
        &mut s,
        &json!([
            {"op": "set_config_count", "n": 2},
            {"op": "set_config_row", "param": "1.t", "values": [5.0, 6.0]},
            {"op": "set_config_row", "param": "2.t", "values": [1.0, 1.0]},
            {"op": "set_config_row", "param": "2.c", "values": [0.01, 0.02]},
            {"op": "set_config_pickup", "param": "2.t", "k": 1, "row": "1.t", "from": 1, "scale": 2.0, "offset": 1.0},
            {"op": "set_config_pickup", "param": "2.t", "k": 2, "row": "1.t", "from": 2},
        ]),
    )
    .unwrap();
    assert_eq!(vals(&s, "2.t"), vec![11.0, 6.0]);
    assert_eq!(s.surfaces[2].t, 11.0);
    // 元の行の値を変えると追従する
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_config_row", "param": "1.t", "values": [4.0, 7.0]}]),
    )
    .unwrap();
    assert_eq!(vals(&s, "2.t"), vec![9.0, 7.0]);
    // 今の構成（1）の元の行の値を系で変えると、今の構成のピックアップの先の値も系に入り、次に合わせても外れない
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_param", "param": "1.t", "value": 3.0}]),
    )
    .unwrap();
    s.sync_active_config();
    assert_eq!(vals(&s, "2.t"), vec![7.0, 7.0]);
    assert_eq!(s.surfaces[2].t, 7.0);
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_param", "param": "3.t", "value": 3.0}]),
    )
    .unwrap();
    s.sync_active_config();
    assert_eq!(vals(&s, "2.t"), vec![7.0, 7.0]);
    let p2 = s.param("2.t").unwrap();
    let n_picks = |s: &lensbench_core::System| {
        let c = s.configs().unwrap();
        c.rows.iter().find(|r| r.param == p2).unwrap().picks.len()
    };
    assert_eq!(n_picks(&s), 2);
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "set_config_row", "param": "1.t", "values": [4.0, 7.0]}]),
    )
    .unwrap();
    // 循環（1.t 構成 1 ← 2.t 構成 1 ← 1.t 構成 1）・違う種類の行・表に無い行は不可
    for bad in [
        json!({"op": "set_config_pickup", "param": "1.t", "k": 1, "row": "2.t", "from": 1}),
        json!({"op": "set_config_pickup", "param": "2.t", "k": 1, "row": "2.c", "from": 1}),
        json!({"op": "set_config_pickup", "param": "2.t", "k": 1, "row": "3.t", "from": 1}),
    ] {
        assert!(
            edit_ops::apply_json(&mut s, &json!([bad])).is_err(),
            "{bad}"
        );
    }
    // 面 1 の前に面を挿入すると、行も元の行も面の番号がずれる
    let mut t = s.clone();
    t.insert_surface(1, lensbench_core::surface::Surface::default())
        .unwrap();
    let c = t.configs().unwrap();
    let r = c
        .rows
        .iter()
        .find(|r| r.param == t.param("3.t").unwrap())
        .unwrap();
    assert!(
        r.picks
            .iter()
            .all(|q| q.row == Some(t.param("2.t").unwrap())),
        "{:?}",
        r.picks
    );
    // 元の行を消すと値を残して外れる
    edit_ops::apply_json(
        &mut s,
        &json!([{"op": "remove_config_row", "param": "1.t"}]),
    )
    .unwrap();
    let c = s.configs().unwrap();
    let r = c
        .rows
        .iter()
        .find(|r| r.param == s.param("2.t").unwrap())
        .unwrap();
    assert!(r.picks.is_empty());
    assert_eq!(r.values, vec![9.0, 7.0]);
}

#[test]
fn ops_semi_once_matches_one_by_one() {
    let s0 = a0();
    assert!(s0.surfaces.iter().any(|d| d.semi_auto));
    let n = s0.surfaces.len();
    let mut ops: Vec<serde_json::Value> = (1..n - 1)
        .map(|j| match j % 3 {
            0 => json!({"op": "set_surface", "surface": j, "fields": {"t": s0.surfaces[j].t * 1.01 + 0.1}}),
            1 => json!({"op": "set_surface", "surface": j, "fields": {"comment": "x"}}),
            _ => json!({"op": "set_surface", "surface": j, "fields": {"c": s0.surfaces[j].c * 0.99}}),
        })
        .collect();
    let surfs = |s: &lensbench_core::System| format!("{:?}", s.surfaces);
    let one_by_one = |ops: &[serde_json::Value]| {
        let mut b = a0();
        for o in ops {
            edit_ops::apply_json(&mut b, &json!([o])).unwrap();
        }
        b
    };
    let mut a = a0();
    edit_ops::apply_json(&mut a, &json!(ops)).unwrap();
    let b = one_by_one(&ops);
    assert_eq!(surfs(&a), surfs(&b));
    assert_ne!(surfs(&a), surfs(&s0));

    // 最後の面の曲率を、自動の半直径の面 1 から取る
    let last = n - 2;
    assert!(s0.surfaces[1].semi_auto);
    ops.insert(
        0,
        json!({"op": "add_pickup", "dst": last, "src": 1, "what": "c", "from": "semi", "scale": -0.001}),
    );
    ops.retain(|o| o["surface"] != json!(last));
    let mut a = a0();
    edit_ops::apply_json(&mut a, &json!(ops)).unwrap();
    let b = one_by_one(&ops);
    assert_eq!(surfs(&a), surfs(&b));

    // 変えた後に今の半直径を読む操作: 自動を外して固定する、半直径から曲率を取るピックアップを足す
    let t1 = s0.surfaces[1].t;
    for tail in [
        json!({"op": "set_surface", "surface": 1, "fields": {"semi_auto": false}}),
        json!({"op": "add_pickup", "dst": last, "src": 1, "what": "c", "from": "semi", "scale": -0.001}),
    ] {
        let ops = vec![
            json!({"op": "set_surface", "surface": 1, "fields": {"t": t1 + 3.0}}),
            json!({"op": "set_surface", "surface": 2, "fields": {"comment": "y"}}),
            tail,
        ];
        let mut a = a0();
        edit_ops::apply_json(&mut a, &json!(ops)).unwrap();
        assert_eq!(surfs(&a), surfs(&one_by_one(&ops)), "{ops:?}");
    }

    // 開口が絞りの大きさで決まる系で、変えた後に絞りを自動の半直径の面へ移す（その面の今の半直径が開口になる）
    let float = |ops: &[serde_json::Value]| {
        let mut s = a0();
        let mut ap = edit_ops::system_data(&s).unwrap()["aperture"].clone();
        ap[0] = json!("float");
        ap[1] = json!(s.surfaces[s.stop()].semi);
        edit_ops::apply_json(
            &mut s,
            &json!([{"op": "set_system_data", "data": {"aperture": ap}}]),
        )
        .unwrap();
        assert_eq!(s.aperture.0, lensbench_core::system::ApertureKind::Float);
        let mut a = s.clone();
        edit_ops::apply_json(&mut a, &json!(ops)).unwrap();
        for o in ops {
            edit_ops::apply_json(&mut s, &json!([o])).unwrap();
        }
        (a, s)
    };
    let ops = vec![
        json!({"op": "set_surface", "surface": 1, "fields": {"t": t1 + 3.0}}),
        json!({"op": "set_surface", "surface": 2, "fields": {"c": s0.surfaces[2].c * 0.9}}),
        json!({"op": "set_stop", "surface": 4}),
    ];
    let (a, b) = float(&ops);
    assert_eq!(surfs(&a), surfs(&b));
}

/// 熱の構成の操作は make_thermal と同じ系を作り、その注意を返す
#[test]
fn make_thermal_op_matches_core() {
    use lensbench_core::thermal::{make_thermal, ThermalSpec};
    let s0 = a0();
    let spec = ThermalSpec {
        temps: vec![-20.0, 60.0],
        pressures: vec![1.0, 0.5],
    };
    let want = make_thermal(&s0, 1, &spec).unwrap();
    let mut s = s0.clone();
    let notes = edit_ops::apply_json(
        &mut s,
        &json!([{"op": "make_thermal", "temps": [-20, 60], "pressures": [1, 0.5]}]),
    )
    .unwrap();
    assert_eq!(notes, want.warnings);
    let (a, b) = (s.configs().unwrap(), want.system.configs().unwrap());
    assert_eq!((a.n, a.active), (3, 1));
    assert_eq!(a.rows.len(), b.rows.len());
    for (x, y) in a.rows.iter().zip(&b.rows) {
        assert_eq!((x.param, &x.values), (y.param, &y.values));
    }
    assert!(s.adjust_index);
    // 温度が無ければ断り、系は変えない
    let mut s = s0.clone();
    assert!(edit_ops::apply_json(&mut s, &json!([{"op": "make_thermal", "temps": []}])).is_err());
    assert!(s.configs().is_none());
}
