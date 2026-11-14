//! [`crate::combo`] の試験

use super::*;

/// ポータブル保存の相対パス: 機構部品のカタログのパスも写す
#[test]
fn map_paths_covers_mechanics() {
    let mut c = Combo {
        mechanics: vec![MechPick {
            catalog: "/a/items/tl.json".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    c.map_paths(&|p: &str| p.replace("/a/", ""));
    assert_eq!(c.mechanics[0].catalog, "items/tl.json");
}

#[test]
fn ids_and_blocks() {
    let mut c = Combo::default();
    let mut cat = Pick::new(ElemKind::Catalog);
    cat.vendor = "LB".into();
    cat.part = "PCX-100".into();
    c.picks = vec![
        cat.clone(),
        cat,
        Pick::new(ElemKind::Lens),
        Pick::new(ElemKind::Mirror),
        Pick::new(ElemKind::Lens),
        Pick::new(ElemKind::Splitter),
    ];
    assert_eq!(
        c.ids(),
        vec!["PCX-100", "PCX-100#2", "L1", "M1", "L2", "BS1"]
    );
    let p = c.project();
    let b = p["blocks"].as_array().unwrap();
    // 前の絞り・空気 + 要素ごとにブロックと空気
    assert_eq!(b.len(), 2 + 2 * 6);
    assert_eq!(b[6]["source"]["surfaces"][0]["glass"], "N-BK7");
    assert_eq!(b[8]["type"], "mirror");
    assert_eq!(b[8]["tilt"], json!([45.0, 0.0]));
    assert_eq!(b[12]["type"], "splitter");
    assert_eq!(b[12]["angle"], json!([45.0, 0.0]));
    c.picks[5].bs_flip = true;
    assert_eq!(c.project()["blocks"][12]["angle"], json!([-45.0, 0.0]));
    // 絞りの要素があれば前の絞りは置かない
    c.picks.insert(0, Pick::new(ElemKind::Stop));
    assert_eq!(c.project()["blocks"][0]["type"], "stop");
    assert_eq!(c.project()["blocks"][0]["id"], "STO1");
}

/// 間に差し込むと前の空気を半分ずつ分け、後ろに足すときは分けない
#[test]
fn insert_pick_splits_air() {
    let mut c = Combo {
        picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
        ..Default::default()
    };
    c.picks[0].air = 10.0;
    let mut p = Pick::new(ElemKind::Plate);
    p.air = 7.0;
    assert_eq!(insert_pick(&mut c, 1, p.clone()), 1);
    assert_eq!((c.picks[0].air, c.picks[1].air), (5.0, 5.0));
    assert_eq!(insert_pick(&mut c, 99, p), 3);
    assert_eq!(c.picks[3].air, 7.0);
}

/// 矢印キー: 光軸が斜め 45° でも ← → だけが光軸に沿い、↑ ↓ は偏心
#[test]
fn arrow_keys_follow_axis() {
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let v = View {
        axes: vec![Vec2::new(s, -s), Vec2::new(0.0, -1.0)],
        ..Default::default()
    };
    assert_eq!(v.along_key(0, Vec2::X), (1.0, 0.0));
    assert_eq!(v.along_key(0, -Vec2::X), (-1.0, 0.0));
    assert_eq!(v.along_key(0, -Vec2::Y), (0.0, 1.0));
    // 上向きの光軸は ↑ で進む、← → は偏心
    assert_eq!(v.along_key(1, -Vec2::Y), (1.0, 0.0));
    assert_eq!(v.along_key(1, Vec2::X).0, 0.0);
}

#[test]
fn old_pick_is_read() {
    let p: Pick =
        serde_json::from_str(r#"{"vendor":"LB","part":"X","reversed":true,"air":3.0}"#).unwrap();
    assert_eq!(p.kind, ElemKind::Catalog);
    assert!(p.reversed);
    assert_eq!(p.air, 3.0);
}

#[test]
fn focus_distance() {
    // 軸から 1 の高さで、傾き -0.1 の光線は 10 先で軸に届く
    let t = paraxial_focus(1.0, 0.0, -0.1, (1.0f64 - 0.01).sqrt()).unwrap();
    assert!((t - 9.9498743710662).abs() < 1e-9, "{t}");
    assert!(paraxial_focus(1.0, 0.0, 0.0, 1.0).is_none());
}

#[test]
fn shift_keeps_total() {
    let mut c = Combo {
        picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
        ..Default::default()
    };
    c.picks[0].air = 10.0;
    c.picks[1].air = 20.0;
    c.shift(1, 2.0);
    assert_eq!((c.picks[0].air, c.picks[1].air), (12.0, 18.0));
    c.shift(0, 1.0);
    assert_eq!((c.stop_gap, c.picks[0].air), (3.0, 11.0));
}

#[test]
fn point_in_polygon() {
    let sq = [
        Pos2::new(0.0, 0.0),
        Pos2::new(1.0, 0.0),
        Pos2::new(1.0, 1.0),
        Pos2::new(0.0, 1.0),
    ];
    assert!(inside(&sq, Pos2::new(0.5, 0.5)));
    assert!(!inside(&sq, Pos2::new(1.5, 0.5)));
}

#[test]
fn surface_edits_follow_the_element() {
    let mut c = Combo::default();
    let mut a = Pick::new(ElemKind::Lens);
    a.set_edit(1, "r", json!(-40.0));
    a.set_edit(1, "r", json!(-30.0));
    a.set_edit(0, "semi", json!(8.0));
    c.picks = vec![Pick::new(ElemKind::Lens), a];
    let v = c.project();
    assert_eq!(
        v["surface_edits"],
        json!([
            {"block": "L2", "surface": 1, "fields": {"r": -30.0}},
            {"block": "L2", "surface": 0, "fields": {"semi": 8.0}},
        ])
    );
    // 前に要素を足して id が変わっても、上書きは同じ要素に付いて行く
    c.picks.insert(0, Pick::new(ElemKind::Lens));
    assert_eq!(c.project()["surface_edits"][0]["block"], "L3");
    // 上書きが無ければ書かない
    c.picks[2].edits.clear();
    assert!(c.project().get("surface_edits").is_none());
}
/// 面の数・順・元が変わったら、面番号の上書き（edits・ops）を外す。値だけの変更では残す（GPN-02）
#[test]
fn stale_surface_edits_are_dropped() {
    let mut c = Combo::default();
    let mut a = Pick::new(ElemKind::Surfaces);
    a.surfs = vec![SurfRow::default(), SurfRow::default(), SurfRow::default()];
    a.set_edit(1, "r", json!(-40.0));
    a.ops.push(json!({ "op": "set_stop", "surface": 2 }));
    c.picks = vec![a];
    // 印の無い上書き（前の保存）は今の並びのものとみなす
    assert_eq!(c.drop_stale_edits(), 0);
    assert!(!c.picks[0].edits_for.is_empty());
    // 値だけ変える（面の数・種類は同じ）: 残す
    c.picks[0].surfs[0].radius = 12.0;
    assert_eq!(c.drop_stale_edits(), 0);
    assert_eq!(c.picks[0].edits.len(), 1);
    // 面を足す: 番号がずれるので外す
    c.picks[0].surfs.insert(1, SurfRow::default());
    assert_eq!(c.drop_stale_edits(), 2);
    assert!(c.picks[0].edits.is_empty() && c.picks[0].ops.is_empty());
    assert!(c.picks[0].edits_for.is_empty());
    // 反転も番号の向きが変わるので外す
    let mut b = Pick::new(ElemKind::Lens);
    b.set_edit(0, "semi", json!(8.0));
    c.picks.push(b);
    assert_eq!(c.drop_stale_edits(), 0);
    c.picks[1].reversed = true;
    assert_eq!(c.drop_stale_edits(), 1);
    // 消す操作の後の数
    assert_eq!(c.census(), [2, 0, 0]);
}

/// 絞り・ソルブ・ピックアップ: 要素の面はその要素の id に、絞りを面に置けば前の絞りは無くなる
#[test]
fn surface_ops_follow_the_element() {
    let mut c = Combo {
        picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
        ..Default::default()
    };
    assert!(c.lead_stop());
    c.picks[1].ops.push(json!({
        "op": "add_pickup", "dst": 1, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1
    }));
    c.set_stop_face(Some((vec![0], 1)));
    assert!(!c.lead_stop());
    assert_eq!(c.stop_face(), Some((vec![0], 1)));
    let v = c.project();
    assert_eq!(v["blocks"][0]["type"], "lens");
    assert_eq!(
        v["surface_ops"],
        json!([
            {"op": "set_stop", "surface": {"block": "L1", "surface": 1}},
            {"op": "add_pickup", "dst": {"block": "L2", "surface": 1}, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1},
        ])
    );
    c.set_stop_face(None);
    assert!(c.lead_stop());
    assert_eq!(c.project()["surface_ops"].as_array().map(Vec::len), Some(1));
}
/// ON/OFF・往路を戻る・状態
#[test]
fn on_off_back_and_states() {
    let mut c = Combo {
        picks: vec![
            Pick::new(ElemKind::Lens),
            Pick::new(ElemKind::Plate),
            Pick::new(ElemKind::Mirror),
        ],
        ..Default::default()
    };
    c.picks[1].on = false;
    c.picks[1].off_air = true;
    c.picks[2].back = true;
    let v = c.project();
    let b = &v["blocks"];
    assert_eq!((&b[4]["on"], &b[4]["off"]), (&json!(false), &json!("air")));
    assert_eq!(b[6]["back"], true);
    assert!(v.get("states").is_none());
    assert_eq!(c.span_ids(), vec!["L1", "", "M1"]);
    // 状態: P1 を入れ、L1 の後の空気を変える
    c.states.push("測定".into());
    c.state = Some("測定".into());
    c.picks[1].state_on.insert("測定".into(), true);
    *c.picks[0].air_mut(Some("測定")) = 12.0;
    assert_eq!(c.picks[0].air, 5.0);
    let v = c.project();
    assert_eq!(v["blocks"][3]["id"], "L1_air");
    assert_eq!(
        v["states"],
        json!([{ "name": "測定", "set": { "P1": { "on": true }, "L1_air": { "t": 12.0 } } }])
    );
    assert_eq!(c.span_ids(), vec!["L1", "P1", "M1"]);
    // 図の間隔を動かすと、状態の値が変わる
    c.shift(1, 1.0);
    assert_eq!(c.picks[0].air_in(Some("測定")), 13.0);
    assert_eq!(c.picks[0].air, 5.0);
    // 展開できる（基本は平板が空気、状態では平板が入る。鏡で往路を戻る）
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let pr = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
    let env = lensbench_project::Env {
        base: root.clone(),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    let n = |st: Option<&str>| {
        lensbench_project::expand_state(&pr, &env, st)
            .unwrap()
            .surfaces
            .len()
    };
    let (a, m) = (n(None), n(Some("測定")));
    assert_eq!(m, a + 2 * 2, "往復で平板の 2 面が 2 回");
}
/// 状態ごとの偏心・傾き（玉と鏡）・系の値・BS の経路がプロジェクトの状態になり、展開で当たる
#[test]
fn state_place_settings_and_path() {
    let mut c = Combo {
        picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Mirror)],
        states: vec!["S".into()],
        ..Default::default()
    };
    let st = Some("S");
    // 見るだけでは状態の値を作らない
    assert_eq!(c.picks[0].place_in(st).decenter, [0.0; 2]);
    assert!(!c.picks[0].has_state_values("S"));
    {
        let (d, t) = c.picks[0].place_mut(st);
        d[1] = 0.3;
        t[0] = 1.5;
    }
    c.picks[1].place_mut(st).1[1] = 2.0;
    assert_eq!(c.picks[0].decenter, [0.0; 2]);
    let mut s = c.snap();
    s.object_distance = Some(200.0);
    s.waves = vec![WavePt::um(0.55)];
    s.primary = 1;
    c.state_sys.insert("S".into(), s);
    c.state_path.insert("S".into(), "tr".into());
    let v = c.project();
    let ids = c.ids();
    let set = &v["states"][0]["set"];
    assert_eq!(set[&ids[0]]["decenter"], json!([0.0, 0.3]));
    assert_eq!(set[&ids[0]]["tilt"], json!([1.5, 0.0, 0.0]));
    assert_eq!(set[&ids[1]]["tilt"], json!([45.0, 2.0]));
    let pr = &v["states"][0]["project"];
    assert_eq!(pr["object_distance"], 200.0);
    assert_eq!(pr["wavelengths"], json!([{ "um": 0.55 }]));
    assert_eq!(pr["primary"], 1);
    assert_eq!(pr["path"], "TR");
    assert!(pr.get("aperture").is_none(), "変えていない項目は書かない");
    // プロジェクトとして読め、状態で値が変わる
    let p = lensbench_project::Project::from_json(&v.to_string()).unwrap();
    let q = p.with_state("S").unwrap();
    assert_eq!(q.object_distance, Some(200.0));
    assert_eq!(q.path.as_deref(), Some("TR"));
    // 状態の値をやめる
    c.picks[0].clear_state("S");
    assert!(!c.picks[0].has_state_values("S"));
    // 基本の値が無限遠で、状態で無限遠に戻すなど、書かない項目は既定の値で書く
    let mut c2 = c.clone();
    c2.object_distance = Some(50.0);
    c2.telecentric = true;
    c2.aiming = Aiming::Real;
    let mut s2 = c2.snap();
    s2.object_distance = None;
    s2.telecentric = false;
    s2.aiming = Aiming::Off;
    c2.state_sys.insert("S".into(), s2);
    let pr = c2.state_project("S");
    assert_eq!(pr["object_distance"], Value::Null);
    assert_eq!(pr["telecentric"], json!(false));
    assert_eq!(pr["aiming"], json!("off"));
    let v2 = c2.project();
    let p2 = lensbench_project::Project::from_json(&v2.to_string()).unwrap();
    assert_eq!(p2.with_state("S").unwrap().object_distance, None);
}

/// 群: 続く同じ群の要素を群のブロックに、状態で群を動かす
#[test]
fn groups_wrap_runs_and_move_by_state() {
    let mut c = Combo {
        picks: vec![
            Pick::new(ElemKind::Lens),
            Pick::new(ElemKind::Lens),
            Pick::new(ElemKind::Lens),
        ],
        ..Default::default()
    };
    c.picks[0].air = 10.0;
    c.picks[1].group = "G1".into();
    c.picks[2].group = "G1".into();
    c.picks[1].air = 3.0;
    c.picks[2].air = 30.0;
    c.states.push("tele".into());
    c.groups.push(GroupDef {
        name: "G1".into(),
        role: "zoom".into(),
        shift: 0.0,
        state_shift: [("tele".to_string(), 2.0)].into(),
    });
    let v = c.project();
    let b = &v["blocks"];
    // 絞り・空気、L1・空気、G1（L2・空気・L3）、空気
    assert_eq!(b[4]["type"], "group");
    assert_eq!(b[4]["role"], "zoom");
    assert_eq!(b[4]["blocks"].as_array().unwrap().len(), 3);
    assert_eq!(b[5]["t"], 30.0);
    assert_eq!(v["states"][0]["set"]["G1"]["shift"], 2.0);
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
    let env = lensbench_project::Env {
        base: root.clone(),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    let a = lensbench_project::expand_state(&pr, &env, None).unwrap();
    let t = lensbench_project::expand_state(&pr, &env, Some("tele")).unwrap();
    // L1 の後の空気が 2 増え、群の後の空気が 2 減る
    assert!((t.surfaces[3].t - a.surfaces[3].t - 2.0).abs() < 1e-12);
    let n = a.surfaces.len();
    assert!((t.surfaces[n - 2].t - a.surfaces[n - 2].t + 2.0).abs() < 1e-12);
}
/// 群: 離れた同じ名前は最初の続きだけ群に、要素の名前と同じ名前は群にしない
#[test]
fn groups_split_runs_and_element_names() {
    let mut c = Combo {
        picks: (0..4).map(|_| Pick::new(ElemKind::Lens)).collect(),
        ..Default::default()
    };
    c.picks[0].group = "G".into();
    c.picks[2].group = "G".into();
    let v = c.project();
    let types: Vec<&str> = v["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(types.iter().filter(|t| **t == "group").count(), 1);
    let id = c.ids()[1].clone();
    c.picks[2].group = String::new();
    c.picks[0].group = id;
    let v = c.project();
    assert!(!v["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["type"] == "group"));
}
/// 群: 要素から外した群・要素の名前と同じ群の状態の移動は書かない（状態で系が組める）
#[test]
fn stale_group_shift_is_not_written() {
    let mut c = Combo {
        picks: (0..2).map(|_| Pick::new(ElemKind::Lens)).collect(),
        ..Default::default()
    };
    c.states.push("tele".into());
    c.groups.push(GroupDef {
        name: "G1".into(),
        state_shift: [("tele".to_string(), 2.0)].into(),
        ..Default::default()
    });
    let id = c.ids()[0].clone();
    c.groups.push(GroupDef {
        name: id.clone(),
        state_shift: [("tele".to_string(), 1.0)].into(),
        ..Default::default()
    });
    c.picks[0].group = id;
    let v = c.project();
    assert!(v["states"][0]["set"].as_object().unwrap().is_empty());
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
    let env = lensbench_project::Env {
        base: root.clone(),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    lensbench_project::expand_state(&pr, &env, Some("tele")).unwrap();
}
/// BS: キューブの半分の三角形から正方形と分ける面
#[test]
fn cube_from_half_prism() {
    let [sq, diag] = cube_of_prism(&[[16.0, -6.35], [28.7, 6.35], [16.0, 6.35]]).unwrap();
    assert_eq!(diag, vec![[16.0, -6.35], [28.7, 6.35]]);
    let has = |q: [f64; 2]| {
        sq.iter()
            .any(|p| (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9)
    };
    assert!(has([28.7, -6.35]) && has([16.0, 6.35]) && sq.len() == 4);
    assert!(cube_of_prism(&[[0.0, 0.0], [2.0, 0.0], [1.0, 1.5]]).is_none());
    assert!(cube_of_prism(&[[0.0, 0.0], [1.0, 0.0]]).is_none());
}
/// 構成を重ねた layout では光線の色は構成ごと、1 つなら視野ごと
#[test]
fn ray_color_by_config() {
    let ray = |f: u64, c: u64| json!({ "field": f, "cfg": c, "pts": [[0.0, 0.0], [1.0, 0.0]] });
    let one = LayoutData::parse(&json!({ "rays": [ray(2, 0)], "configs": [] }));
    assert_eq!(one.rays[0].0, 2);
    let two = LayoutData::parse(&json!({
        "rays": [ray(2, 0), ray(2, 1)], "configs": ["TT", "RR"]
    }));
    assert_eq!(two.rays.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(two.configs, ["TT", "RR"]);
}
/// xz 面の図: plane で縦が x と分かり、矢印・ドラッグの偏心・傾きは x の向き
#[test]
fn xz_plane_moves_x() {
    let d = LayoutData::parse(&json!({ "plane": "xz" }));
    assert!(d.xz && !LayoutData::parse(&json!({})).xz);
    let mut p = Pick::new(ElemKind::Lens);
    let v = View {
        xz: true,
        ..Default::default()
    };
    v.nudge(&mut p, None, 0.5, 2.0);
    assert_eq!((p.decenter, p.tilt), ([0.5, 0.0], [0.0, -2.0, 0.0]));
    View::default().nudge(&mut p, None, 0.5, 2.0);
    assert_eq!((p.decenter, p.tilt), ([0.5, 0.5], [2.0, -2.0, 0.0]));
}
/// 名前の無い要素（BS）が続くとき、自動の玉を順に当てる
#[test]
fn spans_for_consecutive_splitters() {
    let d = LayoutData {
        vertices: vec![None; 17],
        plates: vec![("image".into(), 16, vec![])],
        units: vec![
            ("lens1".into(), vec![[2, 3]]),
            ("lens2".into(), vec![[4, 8]]),
            ("lens3".into(), vec![[10, 14]]),
            ("L1".into(), vec![[2, 3]]),
        ],
        ..Default::default()
    };
    let ids: Vec<String> = ["L1", "BS1", "BS2"].iter().map(|s| s.to_string()).collect();
    assert_eq!(
        d.spans_for(&ids, false),
        vec![Some([2, 3]), Some([4, 8]), Some([10, 14])]
    );
}

#[test]
fn cube_from_prism_with_repeated_corner() {
    let p = [
        [16.0, -39.05],
        [28.7, -26.35],
        [16.0, -26.35],
        [16.0, -26.35 + 1e-12],
    ];
    assert!(cube_of_prism(&p).is_some());
}
/// 開口・遮蔽: 空気中の面の開口になり、serve で系にできて図の要素に当たる
#[test]
fn aperture_block_builds() {
    let mut c = Combo {
        epd: 4.0,
        stop_gap: 2.0,
        ..Default::default()
    };
    let mut l = Pick::new(ElemKind::Lens);
    l.air = 10.0;
    c.picks.push(l);
    for (shape, dy) in [
        (ApShape::Obscuration, 0.0),
        (ApShape::Rect, 0.5),
        (ApShape::Circle, 0.0),
        (ApShape::Spider, 0.0),
    ] {
        let mut a = Pick::new(ElemKind::Aperture);
        a.ap = shape;
        a.semi = 1.0;
        a.ap_half = [3.0, 1.5];
        a.decenter[1] = dy;
        a.air = 5.0;
        c.picks.push(a);
    }
    let mut proj = c.project();
    let b = proj["blocks"].as_array().unwrap();
    let ap: Vec<&Value> = b.iter().filter(|x| x["type"] == "surfaces").collect();
    assert_eq!(ap.len(), 4);
    assert_eq!(ap[0]["surfaces"][0]["aperture"]["type"], "obscuration");
    let sp = &ap[3]["surfaces"][0]["aperture"];
    assert_eq!(sp["type"], "spider");
    assert_eq!(
        (sp["width"].as_f64(), sp["arms"].as_u64()),
        (Some(1.0), Some(4))
    );
    assert_eq!(ap[1]["surfaces"][0]["aperture"]["dy"], 0.5);
    assert_eq!(ap[1]["surfaces"][0]["aperture"]["hx"], 3.0);
    proj["glass_catalogs"] = json!(["SCHOTT"]);
    let f = std::env::temp_dir().join(format!("lensbench-ap-{}.json", std::process::id()));
    lensbench_core::vfs::write(&f, proj.to_string()).unwrap();
    let gd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass");
    let v = lensbench_engine::preview_layout(
        &json!({ "file": f.display().to_string(), "glass_dirs": [gd.display().to_string()] }),
        &json!({}),
    )
    .unwrap();
    lensbench_core::vfs::remove_file(&f).ok();
    let d = LayoutData::parse(&v);
    let spans = d.spans_for(&c.ids(), c.lead_stop());
    assert!(spans.iter().all(Option::is_some), "{spans:?}");
}

/// 組合せの要素で作れなかったブロック: 3 面の自作の玉（貼り合わせ）、面の並び（Jones 面・座標ブレーク）、
/// .zmx の面の範囲の取り込み。どれも系になり、要素ごとに図の面の範囲が当たる
#[test]
fn multi_surface_blocks_build() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut c = Combo {
        epd: 4.0,
        ..Default::default()
    };
    let mut l = Pick::new(ElemKind::Lens);
    l.r1 = 40.0;
    l.r2 = -30.0;
    l.t = 5.0;
    l.surfs = l.lens_rows();
    l.surfs[1].t = 2.0;
    l.surfs[1].glass = "F2".into();
    l.surfs.push(SurfRow {
        radius: -100.0,
        semi: 12.7,
        glass: "ignored".into(),
        t: 9.0,
        ..Default::default()
    });
    l.air = 10.0;
    let mut sf = Pick::new(ElemKind::Surfaces);
    sf.surfs = vec![
        SurfRow {
            kind: SurfKind::Jones,
            jones: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            t: 1.0,
            ..Default::default()
        },
        SurfRow {
            kind: SurfKind::Coordbrk,
            cb: [0.0, 0.0, 1.0, 0.0, 0.0],
            order: 1,
            ..Default::default()
        },
    ];
    sf.air = 10.0;
    let mut z = Pick::new(ElemKind::ZmxRange);
    z.path = root.join("tests/refs/A0/system.zmx").display().to_string();
    z.air = 30.0;
    c.picks = vec![l, sf, z];
    let mut proj = c.project();
    let b = proj["blocks"].as_array().unwrap();
    // .zmx の絞りを使うので、前に自動の絞りは無い
    assert!(!c.lead_stop());
    let lens = &b[0]["source"]["surfaces"];
    assert_eq!(lens.as_array().unwrap().len(), 3);
    assert_eq!(lens[1]["glass"], "F2");
    // 最後の面の後は空気（ガラス・厚さは書かない）
    assert!(lens[2]["glass"].is_null() && lens[2]["t"].is_null());
    assert_eq!(b[2]["surfaces"][0]["jones"][0], 1.0);
    assert_eq!(b[2]["surfaces"][1]["coordbrk"][2], 1.0);
    // 偏光子・波長板の行は project の書き方で読める
    let pol = SurfRow {
        kind: SurfKind::Polarizer,
        axis_deg: 30.0,
        extinction_ratio: 1000.0,
        ..Default::default()
    };
    let wp = SurfRow {
        kind: SurfKind::Waveplate,
        ..Default::default()
    };
    let none = SurfRow {
        kind: SurfKind::Polarizer,
        ..Default::default()
    };
    // 複屈折材の波長板（1 材・2 材の組）
    let mut quartz = SurfRow {
        kind: SurfKind::Waveplate,
        wp_material: "QUARTZ".into(),
        ..Default::default()
    };
    let q: lensbench_project::SurfSpec = serde_json::from_value(quartz.json()).unwrap();
    let w = q.waveplate.unwrap();
    assert_eq!(
        (w.material.as_deref(), w.thickness_mm),
        (Some("QUARTZ"), None)
    );
    quartz.wp_thickness = 0.4;
    quartz.wp_material2 = "MGF2".into();
    quartz.wp_thickness2 = 0.3;
    let q: lensbench_project::SurfSpec = serde_json::from_value(quartz.json()).unwrap();
    let w = q.waveplate.unwrap();
    assert!(w.material.is_none());
    assert_eq!(w.plates.len(), 2);
    assert_eq!(
        (w.plates[1].material.as_str(), w.plates[1].axis_deg),
        ("MGF2", 90.0)
    );
    assert_eq!(pol.json()["polarizer"]["extinction_ratio"], 1000.0);
    assert!(none.json()["polarizer"].get("extinction_ratio").is_none());
    for r in [&pol, &wp, &none] {
        let q: lensbench_project::SurfSpec = serde_json::from_value(r.json()).unwrap();
        assert!(
            q.polarizer.is_some() || q.waveplate.is_some(),
            "{:?}",
            r.json()
        );
    }
    assert_eq!(b[2]["surfaces"][1]["order"], 1);
    assert_eq!(b[4]["type"], "zmx");
    assert!(b[4]["from"].is_null() && b[4]["config"].is_null() && b[4]["stop"].is_null());
    // 範囲に .zmx の絞り（A0 は面 5）が無ければ、.zmx の絞りは使わず前に自動の絞りを置く（レビュー指摘）
    assert_eq!(zmx_stop_info(&c.picks[2].path).map(|x| x.0), Some(5));
    let mut short = c.clone();
    short.picks[2].range = [1, 3];
    assert!(!short.picks[2].range_has_stop() && short.lead_stop());
    let sp = short.project();
    assert_eq!(sp["blocks"][0]["type"], "stop");
    assert_eq!(sp["blocks"][6]["stop"], false);
    proj["glass_catalogs"] = json!(["SCHOTT"]);
    let f = std::env::temp_dir().join(format!("lensbench-multi-{}.json", std::process::id()));
    lensbench_core::vfs::write(&f, proj.to_string()).unwrap();
    let gd = root.join("tests/glass");
    let v = lensbench_engine::preview_layout(
        &json!({ "file": f.display().to_string(), "glass_dirs": [gd.display().to_string()] }),
        &json!({}),
    )
    .unwrap();
    lensbench_core::vfs::remove_file(&f).ok();
    let d = LayoutData::parse(&v);
    let spans = d.spans_for(&c.ids(), c.lead_stop());
    assert!(spans.iter().all(Option::is_some), "{spans:?} {:?}", d.units);
    let mut sp = short.project();
    sp["glass_catalogs"] = json!(["SCHOTT"]);
    lensbench_core::vfs::write(&f, sp.to_string()).unwrap();
    let v2 = lensbench_engine::preview_layout(
        &json!({ "file": f.display().to_string(), "glass_dirs": [gd.display().to_string()] }),
        &json!({}),
    );
    lensbench_core::vfs::remove_file(&f).ok();
    assert!(v2.is_ok(), "{v2:?}");
    // 貼り合わせの玉は 3 面
    let s0 = spans[0].unwrap();
    assert_eq!(s0[1] - s0[0], 2, "{spans:?}");
}
