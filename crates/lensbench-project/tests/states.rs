//! 状態（多重構成に当たる）と群（ズーム群・フォーカス群）の展開の自己照合

use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::{expand, expand_state, Env, Project};
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
    expand(&parse(&format!("\"blocks\": {blocks}")), &env()).unwrap()
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
    same_surfaces(&expand_state(&p, &env(), None).unwrap(), &project(&blocks));
    // near: 物体距離と群の移動
    let near = expand_state(&p, &env(), Some("near")).unwrap();
    let mut q = parse(&format!(
        "\"object_distance\": 2000, \"blocks\": {}",
        a0_blocks(t2 - 0.5, t4 + 0.5, "")
    ));
    q.name = p.name.clone();
    same_surfaces(&near, &expand(&q, &env()).unwrap());
    // no L3: 玉 3 を空気で残し、間隔 d1 を 7 に
    let s = expand_state(&p, &env(), Some("no L3")).unwrap();
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
        expand_state(&p, &env(), Some("x")).unwrap_err()
    };
    assert!(err(r#""set": {"nope": {"t": 1}}"#).contains("nope"));
    assert!(err(r#""set": {"d1": {"thickness": 1}}"#).contains("thickness"));
    assert!(err(r#""set": {"L1": {"type": "air"}}"#).contains("type"));
    assert!(err(r#""set": {"d1": {"t": "x"}}"#).contains("状態 x"));
    assert!(err(r#""project": {"blocks": []}"#).contains("blocks"));
    let p = parse(&format!("\"blocks\": {blocks}"));
    assert!(expand_state(&p, &env(), Some("y")).is_err());
    // 同じ id が 2 つ
    let dup = blocks.replace(r#""id": "L3""#, r#""id": "L1""#);
    let p = parse(&format!(
        "\"blocks\": {dup}, \"states\": [{{\"name\": \"x\", \"set\": {{\"L1\": {{\"on\": false}}}}}}]"
    ));
    assert!(expand_state(&p, &env(), Some("x")).is_err());
}

#[test]
fn a_group_first_in_the_system_moves_against_the_rest() {
    let mk = |obj: &str| {
        expand(
            &parse(&format!(
                r#"{obj} "blocks": [{{"type": "group", "shift": 1, "blocks": [{{"type": "stop"}}]}}, {{"type": "air", "t": 10}}]"#
            )),
            &env(),
        )
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
    b.surfaces[2].t += 1.25;
    b.surfaces[4].t -= 1.25;
    // 像面までの厚さ（面 7）はソルブ（近軸の焦点）なので、OpticStudio と同じく解き直される
    assert_eq!(b.solves.len(), 1);
    b.solve_thickness().unwrap();
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
    for (x, y) in blk.surfaces.iter_mut().zip(&z.surfaces) {
        x.comment = y.comment.clone();
        x.semi = y.semi;
        x.semi_auto = y.semi_auto;
    }
    // zmx ブロックでは像面までの厚さのソルブが残り、展開した系で解き直される（玉のブロックは値）
    assert_eq!(z.solves.len(), 1);
    let k = z.solves[0].surf;
    blk.surfaces[k].t = z.surfaces[k].t;
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
    let e = |groups: &str| {
        expand(
            &parse(&format!(
                r#""blocks": [{{"type": "zmx", "path": "A0/system.zmx", "groups": [{groups}]}}]"#
            )),
            &env(),
        )
        .unwrap_err()
    };
    assert!(
        e(r#"{"name": "a", "from": 1, "to": 4}, {"name": "b", "from": 3, "to": 7}"#)
            .contains("重なる")
    );
    assert!(e(r#"{"name": "a", "from": 4, "to": 3}"#).contains("逆"));
    assert!(e(r#"{"name": "a", "from": 20, "to": 30}"#).contains("ない"));
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
    let s1 = expand_state(&p, &env, None).unwrap();
    let s2 = expand_state(&p, &env, Some("c2")).unwrap();
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
        expand(
            &parse(&format!(
                r#""blocks": [{{"type": "stop"}}, {{"type": "air", "t": 10}},
                    {{"type": "group", "shift": {sh}, "blocks": [{{"type": "mirror"}}]}}, {{"type": "air", "t": 20}},
                    {{"type": "lens", "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7"}}, {{"radius": -60}}]}}}},
                    {{"type": "air", "t": 30}}]"#
            )),
            &env(),
        )
    };
    assert!(mk(0.0).is_ok());
    let e = mk(2.0).unwrap_err();
    assert!(e.contains("鏡"), "{e}");
}

#[test]
fn a_group_with_two_mirrors_cannot_move() {
    // 鏡 2 枚で parity が元に戻っても、群の中に鏡があれば動かせない。
    // 30° の折り曲げを 2 回すると出ていく軸が入ってくる軸から傾き、間隔の補正では表せない
    let mk = |sh: f64| {
        expand(
            &parse(&format!(
                r#""blocks": [{{"type": "stop"}}, {{"type": "air", "t": 10}},
                    {{"type": "group", "shift": {sh}, "blocks": [
                        {{"type": "mirror", "tilt": [15, 0]}}, {{"type": "air", "t": 20}},
                        {{"type": "mirror", "tilt": [15, 0]}}]}},
                    {{"type": "air", "t": 20}},
                    {{"type": "lens", "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7"}}, {{"radius": -60}}]}}}},
                    {{"type": "air", "t": 30}}]"#
            )),
            &env(),
        )
    };
    assert!(mk(0.0).is_ok());
    let e = mk(2.0).unwrap_err();
    assert!(e.contains("鏡"), "{e}");
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
    )
    .unwrap();
    assert!(s.afocal);
    assert_eq!(s.afocal_unit, AfocalUnit::ArcSeconds);
    let e = expand(
        &parse(&format!(r#""afocal": "xx", "blocks": {blocks}"#)),
        &env(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("afocal"), "{e}");
    let mut p = parse(&format!(
        r#""object_distance": 200, "telecentric": true, "blocks": {blocks}"#
    ));
    p.fields.kind = lensbench_project::FieldKind::ObjectHeight;
    p.fields.points = vec![[0.0, 0.0], [0.0, 3.0]];
    p.aperture.kind = lensbench_project::ApertureType::Na;
    p.aperture.value = 0.01;
    let s = expand(&p, &env()).unwrap();
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
    let s = expand_state(&p, &env(), Some("afocal")).unwrap();
    assert!(s.afocal);
    assert_eq!(s.afocal_unit, AfocalUnit::Degrees);
}
