//! 状態（多重構成に当たる）と群（ズーム群・フォーカス群）の展開の自己照合

use lensbench_core::system::FieldSel;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use lensbench_project::{expand, Env, ExpandOpts, Project};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env_at(base: &Path) -> Env<'static> {
    Env {
        step_files: None,
        base: base.to_path_buf(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn env() -> Env<'static> {
    env_at(&root().join("tests/refs"))
}

fn a0() -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

const HEAD: &str = r#""glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "epd", "value": 10},
  "fields": {"type": "angle", "points": [[0, 0], [0, 14], [0, 20], [10, 10]]},"#;

fn parse(rest: &str) -> Project {
    Project::from_json(&format!("{{ {HEAD} {rest} }}")).unwrap()
}

fn project(blocks: &str) -> System {
    expand(
        &parse(&format!("\"blocks\": {blocks}")),
        &env(),
        &ExpandOpts::default(),
    )
    .map(|x| x.system)
    .unwrap()
}

/// 面の列がビットまで同じ
fn same_surfaces(a: &System, b: &System) {
    assert_eq!(a.surfaces.len(), b.surfaces.len());
    for (j, (x, y)) in a.surfaces.iter().zip(&b.surfaces).enumerate() {
        assert_eq!(format!("{x:?}"), format!("{y:?}"), "面 {j}");
    }
}

/// 像面の光線（局所座標）を比べる
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
            assert_eq!(t0.rays[i].status, t1.rays[i].status);
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

fn z_image(s: &System) -> f64 {
    s.frames().surf[s.image()].unwrap().o[2]
}

/// A0 を玉 3 つのブロックで。玉 2 を群 g2（mid の書き方）で包み、前後の空気は a・b
fn a0_blocks(a: f64, b: f64, group: &str) -> String {
    let s = a0();
    let t: Vec<f64> = s.surfaces.iter().map(|d| d.t).collect();
    format!(
        r#"[{{"type": "lens", "id": "L1", "source": {{"zmx": "A0/system.zmx", "lens": 1}}}}, {{"type": "air", "id": "d1", "t": {a}}},
            {{"type": "group", "id": "g2", "name": "G2", "role": "focus"{group}, "blocks": [
               {{"type": "lens", "id": "L2", "source": {{"zmx": "A0/system.zmx", "lens": 2}}}}]}},
            {{"type": "air", "id": "d2", "t": {b}}},
            {{"type": "stop", "semi": {}}}, {{"type": "air", "t": {}}},
            {{"type": "lens", "id": "L3", "source": {{"zmx": "A0/system.zmx", "lens": 3}}}}, {{"type": "air", "t": {}}},
            {{"type": "surfaces", "surfaces": [{{"t": {}}}]}}]"#,
        s.surfaces[5].semi, t[5], t[7], t[8]
    )
}

#[test]
fn a_shifted_group_moves_only_the_group() {
    let a = a0();
    let (t2, t4) = (a.surfaces[2].t, a.surfaces[4].t);
    let sh = 1.25;
    let moved = project(&a0_blocks(t2, t4, &format!(r#", "shift": {sh}"#)));
    let by_hand = project(&a0_blocks(t2 + sh, t4 - sh, ""));
    same_surfaces(&moved, &by_hand);
    // 群の外の合計長は変わらない
    let still = project(&a0_blocks(t2, t4, ""));
    assert!((z_image(&moved) - z_image(&still)).abs() < 1e-12);
    same_rays(&still, &a, 1e-12);
}

#[test]
fn a_state_applies_its_changes() {
    let a = a0();
    let (t2, t4) = (a.surfaces[2].t, a.surfaces[4].t);
    let blocks = a0_blocks(t2, t4, "");
    let states = r#""states": [
        {"name": "near", "project": {"object_distance": 2000}, "set": {"g2": {"shift": -0.5}}},
        {"name": "no L3", "set": {"L3": {"on": false, "off": "air"}, "d1": {"t": 7}}}
      ]"#;
    let p = parse(&format!("\"blocks\": {blocks}, {states}"));
    assert_eq!(p.state_names(), ["near", "no L3"]);
    // 状態なしは書いたまま
    same_surfaces(
        &expand(
            &p,
            &env(),
            &ExpandOpts {
                state: None,
                ..Default::default()
            },
        )
        .map(|x| x.system)
        .unwrap(),
        &project(&blocks),
    );
    // near: 物体距離と群の移動
    let near = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("near"),
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .unwrap();
    let mut q = parse(&format!(
        "\"object_distance\": 2000, \"blocks\": {}",
        a0_blocks(t2 - 0.5, t4 + 0.5, "")
    ));
    q.name = p.name.clone();
    same_surfaces(
        &near,
        &expand(&q, &env(), &ExpandOpts::default())
            .map(|x| x.system)
            .unwrap(),
    );
    // no L3: 玉 3 を空気で残し、間隔 d1 を 7 に
    let s = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("no L3"),
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .unwrap();
    let hand = a0_blocks(7.0, t4, "").replace(
        r#""id": "L3","#,
        r#""id": "L3", "on": false, "off": "air","#,
    );
    same_surfaces(&s, &project(&hand));
    assert!((z_image(&s) - z_image(&project(&blocks)) - (7.0 - t2)).abs() < 1e-12);
}

#[test]
fn bad_states_are_errors() {
    let a = a0();
    let blocks = a0_blocks(a.surfaces[2].t, a.surfaces[4].t, "");
    let err = |set: &str| {
        let p = parse(&format!(
            "\"blocks\": {blocks}, \"states\": [{{\"name\": \"x\", {set}}}]"
        ));
        expand(
            &p,
            &env(),
            &ExpandOpts {
                state: Some("x"),
                ..Default::default()
            },
        )
        .map(|x| x.system)
        .unwrap_err()
    };
    assert!(err(r#""set": {"nope": {"t": 1}}"#)
        .to_string()
        .contains("nope"));
    assert!(err(r#""set": {"d1": {"thickness": 1}}"#)
        .to_string()
        .contains("thickness"));
    assert!(err(r#""set": {"L1": {"type": "air"}}"#)
        .to_string()
        .contains("type"));
    assert!(err(r#""set": {"d1": {"t": "x"}}"#)
        .to_string()
        .contains("state x"));
    assert!(err(r#""project": {"blocks": []}"#)
        .to_string()
        .contains("blocks"));
    let p = parse(&format!("\"blocks\": {blocks}"));
    assert!(expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("y"),
            ..Default::default()
        }
    )
    .map(|x| x.system)
    .is_err());
    // 同じ id が 2 つ
    let dup = blocks.replace(r#""id": "L3""#, r#""id": "L1""#);
    let p = parse(&format!(
        "\"blocks\": {dup}, \"states\": [{{\"name\": \"x\", \"set\": {{\"L1\": {{\"on\": false}}}}}}]"
    ));
    assert!(expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("x"),
            ..Default::default()
        }
    )
    .map(|x| x.system)
    .is_err());
}

#[test]
fn a_group_first_in_the_system_moves_against_the_rest() {
    let mk = |obj: &str| {
        expand(&parse(&format!(
                r#"{obj} "blocks": [{{"type": "group", "shift": 1, "blocks": [{{"type": "stop"}}]}}, {{"type": "air", "t": 10}}]"#
            )), &env(), &ExpandOpts::default()).map(|x| x.system)
        .unwrap()
    };
    // 無限遠の物体: 前の間隔は表さないので、後ろの間隔が縮むだけ
    let s = mk("");
    assert_eq!(s.surfaces[1].t, 9.0);
    // 有限の物体なら物体距離が延びる
    let s = mk(r#""object_distance": 100,"#);
    assert_eq!(s.surfaces[0].t, 101.0);
    assert_eq!(s.surfaces[1].t, 9.0);
}

#[test]
fn zmx_groups_move_and_tilt_lenses() {
    let a = a0();
    let zb = |groups: &str| {
        project(&format!(
            r#"[{{"type": "zmx", "path": "A0/system.zmx", "groups": [{groups}]}}]"#
        ))
    };
    // 玉 2（面 3〜4）を 1.25 動かす = 面 2 の厚さに足し、面 4 の厚さから引く
    let s = zb(r#"{"id": "g2", "name": "G2", "role": "zoom", "from": 3, "to": 4, "shift": 1.25}"#);
    let mut b = a.clone();
    b.edit_raw(|rx| {
        rx.surfaces[2].t += 1.25;
        rx.surfaces[4].t -= 1.25;
        Ok(())
    })
    .unwrap();
    // 像面までの厚さ（面 7）はソルブ（近軸の焦点）なので、OpticStudio と同じく解き直される
    assert_eq!(b.solves.len(), 1);
    b.resolve_links().unwrap();
    assert!((b.surfaces[7].t - a.surfaces[7].t).abs() > 1e-6);
    same_rays(&s, &b, 1e-12);
    for j in 0..a.surfaces.len() {
        assert_eq!(s.surfaces[j].t, b.surfaces[j].t, "面 {j}");
    }
    // 偏心・傾けた群はブロックの群と同じ面の列
    let tilt = r#", "decenter": [0.1, -0.2], "tilt": [0.5, 1, 2]"#;
    let z = zb(&format!(r#"{{"id": "g2", "from": 3, "to": 4{tilt}}}"#));
    let mut blk = project(&a0_blocks(a.surfaces[2].t, a.surfaces[4].t, tilt));
    // 注記と、ダミー面の半径だけ違う（ブロックの絞りは「STOP」、面のブロックは半径 0 の固定、zmx の面は自動）
    blk.edit_raw(|rx| {
        for (x, y) in rx.surfaces.iter_mut().zip(&z.surfaces) {
            x.comment = y.comment.clone();
            x.semi = y.semi;
            x.semi_auto = y.semi_auto;
        }
        Ok(())
    })
    .unwrap();
    // zmx ブロックでは像面までの厚さのソルブが残り、展開した系で解き直される（玉のブロックは値）
    assert_eq!(z.solves.len(), 1);
    let k = z.solves[0].surf;
    blk.edit_raw(|rx| {
        rx.surfaces[k].t = z.surfaces[k].t;
        Ok(())
    })
    .unwrap();
    same_surfaces(&z, &blk);
    // 群の外の座標系は元の系と同じ（像面は焦点のソルブで動くので、ソルブの面で比べる）
    let plain = zb("");
    let (f0, f1) = (plain.frames(), z.frames());
    let (o0, o1) = (
        f0.surf[plain.solves[0].surf].unwrap().o,
        f1.surf[k].unwrap().o,
    );
    for k in 0..3 {
        assert!((o0[k] - o1[k]).abs() < 1e-12, "{o0:?} {o1:?}");
    }
    // 玉 2・玉 3 をそれぞれ群にして、OFF で空気に
    let off = zb(
        r#"{"from": 3, "to": 4, "on": false, "off": "air"}, {"from": 6, "to": 7, "on": false, "off": "air"}"#,
    );
    assert!((z_image(&off) - z_image(&plain)).abs() < 1e-12);
    assert!(off.surfaces.len() < a.surfaces.len());
}

#[test]
fn bad_zmx_groups_are_errors() {
    let e =
        |groups: &str| {
            expand(&parse(&format!(
                r#""blocks": [{{"type": "zmx", "path": "A0/system.zmx", "groups": [{groups}]}}]"#
            )), &env(), &ExpandOpts::default()).map(|x| x.system)
        .unwrap_err()
        };
    assert!(
        e(r#"{"name": "a", "from": 1, "to": 4}, {"name": "b", "from": 3, "to": 7}"#)
            .to_string()
            .contains("overlap")
    );
    assert!(e(r#"{"name": "a", "from": 4, "to": 3}"#)
        .to_string()
        .contains("reverse"));
    assert!(e(r#"{"name": "a", "from": 20, "to": 30}"#)
        .to_string()
        .contains("no "));
}

#[test]
fn a_state_can_pick_a_zmx_configuration() {
    // A0 の写しに 2 つ目の構成（面 4 の厚さを変える）を足す
    let dir = std::env::temp_dir().join(format!("lb-states-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = a0();
    let t4 = a.surfaces[4].t;
    // 元は UTF-16 LE（BOM つき）。UTF-8 にして足す
    let raw = std::fs::read(root().join("tests/refs/A0/system.zmx")).unwrap();
    let u: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u).unwrap();
    let txt = &txt[..txt.find("MNUM").unwrap()];
    let mce = format!(
        "MNUM 2 1\nTHIC 4 1 {t4} 0 0 0 1 1 0\nTHIC 4 2 {} 0 0 0 1 1 0\n",
        t4 + 2.0
    );
    std::fs::write(dir.join("mc.zmx"), format!("{txt}{mce}")).unwrap();
    let p = Project::from_json(&format!(
        r#"{{ {HEAD} "blocks": [{{"type": "zmx", "id": "z", "path": "mc.zmx"}}],
             "states": [{{"name": "c2", "set": {{"z": {{"config": 2}}}}}}] }}"#
    ))
    .unwrap();
    let env = env_at(&dir);
    let s1 = expand(
        &p,
        &env,
        &ExpandOpts {
            state: None,
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .unwrap();
    let s2 = expand(
        &p,
        &env,
        &ExpandOpts {
            state: Some("c2"),
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(s1.surfaces[4].t, t4);
    assert_eq!(s2.surfaces[4].t, t4 + 2.0);
}

#[test]
fn a_shifted_group_in_a_double_pass() {
    // 絞り → 20 → 群（玉）→ 30 → 平面鏡（往路を戻る）。群を動かすと往復とも同じだけ動く
    let lens = r#"{"type": "lens", "tilt": [1, 0, 0], "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}}"#;
    let mk = |a: f64, b: f64, sh: f64| {
        project(&format!(
            r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": {a}}}, {{"type": "group", "shift": {sh}, "blocks": [{lens}]}},
                {{"type": "air", "t": {b}}}, {{"type": "mirror", "back": true}}]"#
        ))
    };
    let moved = mk(20.0, 30.0, 2.5);
    let by_hand = mk(22.5, 27.5, 0.0);
    same_surfaces(&moved, &by_hand);
}

#[test]
fn a_group_with_a_mirror_cannot_move() {
    let mk = |sh: f64| {
        expand(&parse(&format!(
                r#""blocks": [{{"type": "stop"}}, {{"type": "air", "t": 10}},
                    {{"type": "group", "shift": {sh}, "blocks": [{{"type": "mirror"}}]}}, {{"type": "air", "t": 20}},
                    {{"type": "lens", "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7"}}, {{"radius": -60}}]}}}},
                    {{"type": "air", "t": 30}}]"#
            )), &env(), &ExpandOpts::default()).map(|x| x.system)
    };
    assert!(mk(0.0).is_ok());
    let e = mk(2.0).unwrap_err();
    assert!(e.message().unwrap_or_default().contains("mirror"), "{e}");
}

#[test]
fn a_group_with_two_mirrors_cannot_move() {
    // 鏡 2 枚で parity が元に戻っても、群の中に鏡があれば動かせない。
    // 30° の折り曲げを 2 回すると出ていく軸が入ってくる軸から傾き、間隔の補正では表せない
    let mk = |sh: f64| {
        expand(&parse(&format!(
                r#""blocks": [{{"type": "stop"}}, {{"type": "air", "t": 10}},
                    {{"type": "group", "shift": {sh}, "blocks": [
                        {{"type": "mirror", "tilt": [15, 0]}}, {{"type": "air", "t": 20}},
                        {{"type": "mirror", "tilt": [15, 0]}}]}},
                    {{"type": "air", "t": 20}},
                    {{"type": "lens", "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7"}}, {{"radius": -60}}]}}}},
                    {{"type": "air", "t": 30}}]"#
            )), &env(), &ExpandOpts::default()).map(|x| x.system)
    };
    assert!(mk(0.0).is_ok());
    let e = mk(2.0).unwrap_err();
    assert!(e.message().unwrap_or_default().contains("mirror"), "{e}");
}

/// 像空間のアフォーカル（`afocal` に角度の単位）と物体側テレセントリック。状態でも変えられる
#[test]
fn afocal_and_telecentric_settings() {
    use lensbench_core::system::AfocalUnit;
    let a = a0();
    let blocks = a0_blocks(a.surfaces[2].t, a.surfaces[4].t, "");
    let s = project(&blocks);
    assert!(!s.afocal && !s.telecentric);
    let s = expand(
        &parse(&format!(r#""afocal": "as", "blocks": {blocks}"#)),
        &env(),
        &ExpandOpts::default(),
    )
    .map(|x| x.system)
    .unwrap();
    assert!(s.afocal);
    assert_eq!(s.afocal_unit, AfocalUnit::ArcSeconds);
    let e = expand(
        &parse(&format!(r#""afocal": "xx", "blocks": {blocks}"#)),
        &env(),
        &ExpandOpts::default(),
    )
    .map(|x| x.system)
    .unwrap_err();
    assert!(e.message().unwrap_or_default().contains("afocal"), "{e}");
    let mut p = parse(&format!(
        r#""object_distance": 200, "telecentric": true, "blocks": {blocks}"#
    ));
    p.fields.kind = lensbench_project::FieldKind::ObjectHeight;
    p.fields.points = vec![[0.0, 0.0], [0.0, 3.0]];
    p.aperture.kind = lensbench_core::system::ApertureKind::Na;
    p.aperture.value = 0.01;
    let s = expand(&p, &env(), &ExpandOpts::default())
        .map(|x| x.system)
        .unwrap();
    assert!(s.telecentric);
    // 書いて読み直しても同じ
    let p = parse(&format!(r#""afocal": "mr", "blocks": {blocks}"#));
    let q = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(q.afocal.as_deref(), Some("mr"));
    assert!(!p.to_json().contains("telecentric"));
    // 状態で変える
    let p = parse(&format!(
        r#""blocks": {blocks}, "states": [{{"name": "afocal", "project": {{"afocal": "dg"}}}}]"#
    ));
    let s = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("afocal"),
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .unwrap();
    assert!(s.afocal);
    assert_eq!(s.afocal_unit, AfocalUnit::Degrees);
}

/// 書き出しで省かれる項目（鏡の aperture、系の coatings・mechanics）も状態で変えられる。型が読まない項目は誤り
#[test]
fn states_change_values_that_are_left_out_when_default() {
    let p = parse(
        r#""blocks": [{"type": "air", "t": 10}, {"type": "mirror", "id": "M1", "semi": 5}, {"type": "air", "t": 10}],
        "states": [
          {"name": "rect", "set": {"M1": {"aperture": {"type": "rect", "hx": 4, "hy": 3}}},
           "project": {"coatings": {"ND1": {"type": "nd", "od": 1, "absorptive": true}}}},
          {"name": "bad", "set": {"M1": {"nope": 1}}},
          {"name": "bad_root", "project": {"nope": 1}}
        ]"#,
    );
    assert!(!p.to_json().contains("aperture\": {\"type\": \"rect"));
    let q = p.with_state("rect").unwrap();
    assert!(q.to_json().contains("\"hx\""), "{}", q.to_json());
    assert!(q.coatings.contains_key("ND1"));
    // 状態の一覧と、変えないブロックは残る
    assert_eq!(q.states.len(), 3);
    assert_eq!(q.blocks.len(), 3);
    let e = p.with_state("bad").unwrap_err().to_string();
    assert!(
        e.contains("state bad") && e.contains("nope of block M1"),
        "{e}"
    );
    let e = p.with_state("bad_root").unwrap_err().to_string();
    assert!(e.contains("system value nope"), "{e}");
}

/// 面の名前（FaceId）: 状態で玉を外しても、ほかのブロックの面は同じ名前で見つかる。復路の面は back、
/// 偏心の座標ブレークは番号なし
#[test]
fn faces_keep_their_names_across_states() {
    let lens = |id: &str, extra: &str| {
        format!(
            r#"{{"type": "lens", "id": "{id}"{extra}, "source": {{"surfaces": [{{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}}"#
        )
    };
    let p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 4}},
        "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 5}}, {l1}, {{"type": "air", "t": 5}},
          {{"type": "group", "id": "G", "blocks": [{l2}, {{"type": "air", "t": 3}}, {l3}]}},
          {{"type": "air", "t": 20}}, {{"type": "mirror", "back": true}}],
        "states": [{{"name": "no L2", "set": {{"L2": {{"on": false, "off": "air"}}}}}}]}}"#,
        l1 = lens("L1", r#", "decenter": [0.2, 0]"#),
        l2 = lens("L2", ""),
        l3 = lens("L3", ""),
    ))
    .unwrap();
    let all = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let st = ExpandOpts {
        state: Some("no L2"),
        ..Default::default()
    };
    let no2 = expand(&p, &env(), &st).unwrap();
    assert_eq!(
        all.map.len(),
        no2.map.len() + 4,
        "L2 の往路・復路の 2 面ずつ"
    );
    let n = all.map.len();
    assert!(all.map[0].face.is_none() && all.map[n - 1].face.is_none());
    let mut found = 0;
    for (j, o) in all.map.iter().enumerate().take(n - 1).skip(1) {
        let f = o.face.as_ref().expect("面の名前");
        let cb = all.system.surfaces[j].is_coord_break();
        assert_eq!(f.cb, cb, "surface {j}");
        match no2.find_face(f) {
            Some(k) => {
                found += 1;
                assert!(!o.is_in("L2"), "surface {j}");
                let (a, b) = (&all.system.surfaces[j], &no2.system.surfaces[k]);
                assert_eq!(
                    (a.c, a.mat.name(), o.back),
                    (b.c, b.mat.name(), no2.map[k].back)
                );
            }
            None => assert!(o.is_in("L2"), "surface {j}"),
        }
    }
    assert_eq!(found, n - 2 - 4);
    // 往路と復路の L3 の面は別の名前、復路の範囲は 1 回目
    let l3: Vec<&lensbench_project::FaceId> = all
        .map
        .iter()
        .filter(|o| o.is_in("L3"))
        .filter_map(|o| o.face.as_ref())
        .collect();
    assert_eq!(l3.len(), 4);
    assert!(!l3[0].back && l3[3].back && l3[3].pass == 0);
    assert_eq!(l3[0].node, l3[3].node);
    assert_ne!(l3[0], l3[3]);
    // 偏心した L1 の前後の座標ブレークは別の名前
    let cbs: Vec<&lensbench_project::FaceId> = all
        .map
        .iter()
        .filter(|o| o.is_in("L1"))
        .filter_map(|o| o.face.as_ref())
        .filter(|f| f.cb && !f.back)
        .collect();
    assert_eq!(cbs.len(), 2);
    assert_ne!(cbs[0], cbs[1]);
}

/// 当て直し（Retarget）: 前の座標ブレークは別の状態でも前の座標ブレークへ。状態がブロックの中身を替えて面の数が
/// 変わったら、そのブロックの面は「無い」（同じ番号の別の面に当てない）
#[test]
fn retarget_keeps_cbs_and_skips_resized_blocks() {
    use lensbench_project::edit_spec::{EditOp, Ref};
    use lensbench_project::retarget::Retarget;
    let two =
        r#"[{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]"#;
    let three = r#"[{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -40, "t": 2, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]"#;
    let p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 4}},
        "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 5}},
          {{"type": "lens", "id": "L0", "source": {{"surfaces": {two}}}}}, {{"type": "air", "t": 5}},
          {{"type": "lens", "id": "L1", "decenter": [0.2, 0], "source": {{"surfaces": {two}}}}}, {{"type": "air", "t": 5}},
          {{"type": "lens", "id": "L2", "source": {{"surfaces": {two}}}}}, {{"type": "air", "t": 20}}],
        "states": [{{"name": "s", "set": {{"L0": {{"on": false, "off": "air"}}, "L2": {{"source": {{"surfaces": {three}}}}}}}}}]}}"#
    ))
    .unwrap();
    let a = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let st = ExpandOpts {
        state: Some("s"),
        ..Default::default()
    };
    let b = expand(&p, &env(), &st).unwrap();
    let faces = |x: &lensbench_project::Expansion| -> Vec<Option<lensbench_project::FaceId>> {
        x.map.iter().map(|o| o.face.clone()).collect()
    };
    let mut r = Retarget::new(&faces(&a), &faces(&b));
    let set = |j: usize| EditOp::SetSurface {
        surface: Ref::Index(j),
        fields: Default::default(),
    };
    let cb = |x: &lensbench_project::Expansion| -> Vec<usize> {
        (0..x.map.len())
            .filter(|&j| x.map[j].is_in("L1") && x.system.surfaces[j].is_coord_break())
            .collect()
    };
    let (ca, cb_) = (cb(&a), cb(&b));
    for k in 0..2 {
        match r.op(&set(ca[k])).unwrap() {
            EditOp::SetSurface {
                surface: Ref::Index(j),
                ..
            } => assert_eq!(j, cb_[k]),
            o => panic!("{o:?}"),
        }
    }
    let l2 = (0..a.map.len()).find(|&j| a.map[j].is_in("L2")).unwrap();
    assert!(r.op(&set(l2)).is_err(), "L2 は面の数が違う");
}

/// 展開した系にした面の編集をブロックへ書き戻すと、書き戻したプロジェクトの展開が編集した系と同じ。
/// 書き戻せない編集（ブロックの最後の面の厚さ・座標ブレーク・面を入れる）は注意
#[test]
fn surface_edits_write_back_to_blocks() {
    use lensbench_project::edit_spec::EditOp;
    let lens = |id: &str, extra: &str| {
        format!(
            r#"{{"type": "lens", "id": "{id}"{extra}, "source": {{"surfaces": [{{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}}"#
        )
    };
    let mut p = Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.5876}}], "aperture": {{"type": "epd", "value": 4}},
        "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 5}}, {a}, {{"type": "air", "t": 5}},
          {{"type": "group", "id": "G", "blocks": [{b}]}}, {{"type": "air", "t": 30}}]}}"#,
        a = lens("A", r#", "decenter": [0.1, 0]"#),
        b = lens("B", ""),
    ))
    .unwrap();
    let x = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let j = |id: &str, k: usize| -> usize {
        (0..x.map.len())
            .filter(|&j| x.map[j].is_in(id) && !x.system.surfaces[j].is_coord_break())
            .nth(k)
            .unwrap()
    };
    let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
        { "op": "set_surface", "surface": j("A", 0), "fields": { "r": 50.0, "t": 5.0 } },
        { "op": "set_material", "surface": j("B", 0), "material": "F2" },
        { "op": "add_pickup", "dst": j("B", 1), "src": j("A", 1), "what": "c", "scale": -1.0 },
        { "op": "set_surface", "surface": j("B", 1), "fields": { "t": 20.0 } },
        { "op": "set_surface", "surface": j("A", 0) - 1, "fields": { "c": 0.0 } },
        { "op": "insert_surface", "at": 2 },
    ]))
    .unwrap();
    let mut s = x.system.clone();
    lensbench_project::edit_ops::apply(&mut s, &ops[..3]).unwrap();
    let faces: Vec<_> = x.map.iter().map(|o| o.face.clone()).collect();
    let ed = lensbench_project::writeback::Edited {
        system: &x.system,
        faces: &faces,
        before: &x.system,
    };
    let notes = lensbench_project::writeback::write_back(&mut p, ed, &ops, &|_| Ok(()));
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(
        notes[0].contains("edit 4") && notes[0].contains("air"),
        "{notes:?}"
    );
    assert!(notes[1].contains("edit 5") && notes[1].contains("coordinate break"));
    assert!(notes[2].contains("edit 6") && notes[2].contains("insert_surface"));
    let y = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    assert_eq!(
        format!("{:?}", y.system.surfaces),
        format!("{:?}", s.surfaces)
    );
    assert_eq!(y.system.pickups, s.pickups);
}

/// 往復するブロックの書き戻し: 最後の面の厚み・材料は書かない（後ろが戻しの座標ブレークでも）、面の値は復路にも
/// 当たるので注意を出す
#[test]
fn round_trip_blocks_write_back_with_notes() {
    use lensbench_project::edit_spec::EditOp;
    let mut p = Project::from_json(
        r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 4},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 5},
          {"type": "lens", "id": "L", "decenter": [0.2, 0], "source": {"surfaces": [{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}},
          {"type": "air", "t": 20}, {"type": "mirror", "back": true}]}"#,
    )
    .unwrap();
    let x = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let j: Vec<usize> = (0..x.map.len())
        .filter(|&j| x.map[j].is_in("L") && !x.map[j].back)
        .filter(|&j| !x.system.surfaces[j].is_coord_break())
        .collect();
    assert_eq!(j.len(), 2);
    assert!(
        x.system.surfaces[j[1] + 1].is_coord_break(),
        "最後の面の後ろは戻しの座標ブレーク"
    );
    let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
        { "op": "set_surface", "surface": j[1], "fields": { "t": 1.0 } },
        { "op": "set_material", "surface": j[1], "material": "F2" },
        { "op": "set_surface", "surface": j[0], "fields": { "r": 50.0 } },
    ]))
    .unwrap();
    let faces: Vec<_> = x.map.iter().map(|o| o.face.clone()).collect();
    let ed = lensbench_project::writeback::Edited {
        system: &x.system,
        faces: &faces,
        before: &x.system,
    };
    let notes = lensbench_project::writeback::write_back(&mut p, ed, &ops, &|_| Ok(()));
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes[0].contains("edit 1") && notes[0].contains("air"));
    assert!(notes[1].contains("edit 2") && notes[1].contains("air"));
    assert!(notes[2].contains("edit 3") && notes[2].contains("return path"));
    let y = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let r: Vec<f64> = (0..y.map.len())
        .filter(|&k| y.map[k].is_in("L") && !y.system.surfaces[k].is_coord_break())
        .map(|k| y.system.surfaces[k].c)
        .collect();
    assert_eq!(r.len(), 4);
    assert!(
        (r[0] - 1.0 / 50.0).abs() < 1e-12 && (r[3] - 1.0 / 50.0).abs() < 1e-12,
        "{r:?}"
    );
}

/// 復路・往路の 2 回目以降の面の値をブロックの面へ書き戻す: 復路の面の形の値は同じ面、面の後ろの間隔（符号が逆）・
/// 媒質は往路の 1 つ前の面へ。復路の最初の面（ブロックの面 0）の後ろはブロックの前なので書かない
#[test]
fn later_pass_faces_write_back_to_blocks() {
    use lensbench_project::edit_spec::EditOp;
    let mut p = Project::from_json(
        r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 4},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 5},
          {"type": "lens", "id": "L", "source": {"surfaces": [{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8},
             {"radius": -40, "t": 2, "glass": "F2", "semi": 8}, {"radius": -90, "semi": 8}]}},
          {"type": "air", "t": 20}, {"type": "mirror", "back": true}]}"#,
    )
    .unwrap();
    let x = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    // L の面: 往路 0・1・2、復路 2・1・0
    let l = |back: bool| -> Vec<usize> {
        (0..x.map.len())
            .filter(|&j| x.map[j].is_in("L") && x.map[j].back == back)
            .filter(|&j| !x.system.surfaces[j].is_coord_break())
            .collect()
    };
    let (f, b) = (l(false), l(true));
    assert_eq!((f.len(), b.len()), (3, 3));
    let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
        { "op": "set_surface", "surface": b[1], "fields": { "r": -30.0, "k": -1.0 } },
        { "op": "set_surface", "surface": b[0], "fields": { "t": -3.0 } },
        { "op": "set_material", "surface": b[0], "material": "N-BK7" },
        { "op": "set_surface", "surface": b[2], "fields": { "t": -6.0 } },
    ]))
    .unwrap();
    let faces: Vec<_> = x.map.iter().map(|o| o.face.clone()).collect();
    let ed = lensbench_project::writeback::Edited {
        system: &x.system,
        faces: &faces,
        before: &x.system,
    };
    let notes = lensbench_project::writeback::write_back(&mut p, ed, &ops, &|_| Ok(()));
    assert_eq!(notes.len(), 4, "{notes:?}");
    for k in 0..3 {
        assert!(notes[k].contains("return path"), "{notes:?}");
    }
    assert!(
        notes[3].contains("edit 4") && notes[3].contains("before the block"),
        "{notes:?}"
    );
    let y = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let s = &y.system.surfaces;
    assert_eq!(y.map.len(), x.map.len());
    for j in [f[1], b[1]] {
        assert!((s[j].c + 1.0 / 30.0).abs() < 1e-15, "{j}: {}", s[j].c);
        assert_eq!(s[j].k, -1.0);
    }
    assert_eq!((s[f[1]].t, s[b[0]].t), (3.0, -3.0));
    for j in [f[1], b[0]] {
        assert_eq!(format!("{:?}", s[j].mat), r#"Glass("N-BK7")"#, "{j}");
    }
    // 面 0 は変わらない
    assert_eq!(s[f[0]].t, 4.0);
    assert!((s[b[2]].c - 1.0 / 60.0).abs() < 1e-15);

    // 往路の 2 回目以降（同じ向きにブロックを 2 回通る）の面は 1 回目と同じ番号の面
    let mut p2 = Project::from_json(&p.to_json()).unwrap();
    let x2 = expand(&p2, &env(), &ExpandOpts::default()).unwrap();
    let mut faces2: Vec<_> = x2.map.iter().map(|o| o.face.clone()).collect();
    // 復路の範囲を往路の 2 回目に見立てる（並びは往路と同じ順）
    for (k, &j) in b.iter().enumerate() {
        let g = faces2[j].as_mut().unwrap();
        (g.back, g.pass, g.index) = (false, 1, k);
    }
    let ed = lensbench_project::writeback::Edited {
        system: &x2.system,
        faces: &faces2,
        before: &x2.system,
    };
    let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
        { "op": "set_surface", "surface": b[0], "fields": { "semi": 7.0 } },
        { "op": "set_stop", "surface": b[0] },
    ]))
    .unwrap();
    let notes = lensbench_project::writeback::write_back(&mut p2, ed, &ops, &|_| Ok(()));
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].contains("other passes"), "{notes:?}");
    assert!(
        notes[1].contains("edit 2") && notes[1].contains("first forward pass"),
        "{notes:?}"
    );
    let y = expand(&p2, &env(), &ExpandOpts::default()).unwrap();
    assert_eq!(y.system.surfaces[f[0]].semi, 7.0);
}

/// プロジェクトファイルの形式の印と版: 書くときに付き、読み戻せる。新しい版・違う印は誤り、省略は今の版
#[test]
fn project_files_carry_format_and_version() {
    let txt = r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 4},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 10}]}"#;
    let p = Project::from_json(txt).unwrap();
    let out = p.to_json();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["format"], "lensbench-project");
    assert_eq!(v["format_version"], 1);
    Project::from_json(&out).unwrap();
    let newer = out.replace("\"format_version\": 1", "\"format_version\": 2");
    let e = Project::from_json(&newer).unwrap_err().to_string();
    assert!(e.contains("newer"), "{e}");
    let other = out.replace("lensbench-project", "lensbench-system");
    assert!(Project::from_json(&other).is_err());
}
