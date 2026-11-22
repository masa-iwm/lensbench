//! 光束の組（`beams`）: 名前で経路・状態を選ぶ、名前の誤り、戻りの面の参照（`back`・`pass`）。

use lensbench_core::system::FieldSel;
use lensbench_core::System;
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

/// tests/splitter の照合の系に欄を足して読む
fn case(name: &str, extra: &str) -> Project {
    let txt = std::fs::read_to_string(root().join(format!("tests/splitter/{name}.json"))).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let x: serde_json::Value = serde_json::from_str(extra).unwrap();
    for (k, val) in x.as_object().unwrap() {
        v[k] = val.clone();
    }
    Project::from_json(&v.to_string()).unwrap()
}

fn sys(p: &Project, state: Option<&str>, path: Option<&str>) -> System {
    expand(
        p,
        &env(),
        &ExpandOpts {
            state,
            path,
            ..Default::default()
        },
    )
    .unwrap()
    .system
}

const PX: [f64; 5] = [0.0, 0.7, -0.5, 0.3, 0.0];
const PY: [f64; 5] = [0.0, 0.2, 0.6, -0.9, -1.0];

/// 全視野・全波長の光線（像面の位置・方向・光路長・けられ）がそろって同じ
fn identical(a: &System, b: &System) {
    assert_eq!(a.surfaces.len(), b.surfaces.len());
    for f in 0..a.fields.len() {
        for w in 0..a.wavelengths.len() {
            let ta = a
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            let tb = b
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
                assert_eq!((ra.status, ra.vig), (rb.status, rb.vig));
                assert_eq!((ra.p, ra.k), (rb.p, rb.k));
                assert_eq!(ra.opl.to_bits(), rb.opl.to_bits());
            }
        }
    }
}

/// 照合済みのマイケルソン・トワイマングリーンの経路を光束として選ぶと、経路を直に選んだ系と同じ
#[test]
fn beams_select_the_checked_paths() {
    for (name, paths) in [
        ("mich_RT", ["RT", "TR"]),
        ("mich_TR", ["TR", "RT"]),
        ("tg_TR", ["TR", "RT"]),
    ] {
        let p = case(
            name,
            &format!(
                r#"{{"beams": [{{"name": "a", "path": "{}", "role": "sample"}},
                              {{"name": "b", "path": "{}", "role": "reference", "off": true}}]}}"#,
                paths[0], paths[1]
            ),
        );
        assert_eq!(p.beam_names(), ["a", "b"]);
        for (beam, path) in ["a", "b"].into_iter().zip(paths) {
            identical(&sys(&p, Some(beam), None), &sys(&p, None, Some(path)));
        }
        // 読み書きで変わらない
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(q.beams, p.beams);
    }
}

/// 光束の状態を当て、経路を省けば状態・プロジェクトの経路
#[test]
fn a_beam_applies_its_state() {
    let p = case(
        "mich_RT",
        r#"{"states": [{"name": "near", "project": {"object_distance": 40, "path": "TR"}}],
            "beams": [{"name": "ref", "path": "RT", "state": "near"},
                      {"name": "near_only", "state": "near"}]}"#,
    );
    let r = p.with_state("ref").unwrap();
    assert_eq!(
        (r.object_distance, r.path.as_deref()),
        (Some(40.0), Some("RT"))
    );
    let n = p.with_state("near_only").unwrap();
    assert_eq!(n.path.as_deref(), Some("TR"));
    identical(
        &sys(&p, Some("ref"), None),
        &sys(&p.with_state("near").unwrap(), None, Some("RT")),
    );
    // 状態・光束の一覧は残る
    assert_eq!(r.beams.len(), 2);
    assert_eq!(r.states.len(), 1);
}

/// 名前の誤りは読むときに誤り。無い名前の誤りは光束の名前も挙げる
#[test]
fn beam_names_are_checked() {
    let head = std::fs::read_to_string(root().join("tests/splitter/mich_RT.json")).unwrap();
    let with = |extra: &str| {
        let mut v: serde_json::Value = serde_json::from_str(&head).unwrap();
        let x: serde_json::Value = serde_json::from_str(extra).unwrap();
        for (k, val) in x.as_object().unwrap() {
            v[k] = val.clone();
        }
        Project::from_json(&v.to_string())
            .map(|_| ())
            .unwrap_err()
            .to_string()
    };
    for (extra, want) in [
        (
            r#"{"beams": [{"name": "a"}, {"name": "a"}]}"#,
            "two or more beams named a",
        ),
        (r#"{"beams": [{"name": " "}]}"#, "beam 1: the name is empty"),
        (
            r#"{"states": [{"name": "a"}], "beams": [{"name": "a"}]}"#,
            "beam a: a state has the same name",
        ),
        (
            r#"{"beams": [{"name": "a", "state": "x"}]}"#,
            "beam a: no state x",
        ),
        (r#"{"beams": [{"name": "a", "path": "TX"}]}"#, "beam a:"),
        (r#"{"beams": [{"name": "a", "turn": []}]}"#, "turn"),
    ] {
        let e = with(extra);
        assert!(e.contains(want), "{extra}: {e}");
    }
    let p = case("mich_RT", r#"{"beams": [{"name": "ref", "path": "RT"}]}"#);
    let e = p.with_state("x").unwrap_err().to_string();
    assert!(e.contains("no state or beam x (beams: ref)"), "{e}");
    // 状態で光束の組は変えられない
    let p = case(
        "mich_RT",
        r#"{"states": [{"name": "s", "project": {"beams": []}}], "beams": [{"name": "ref"}]}"#,
    );
    let e = p.with_state("s").unwrap_err().to_string();
    assert!(e.contains("beams can't be changed"), "{e}");
    // 経路を持つ光束に別の経路は渡せない
    let p = case("mich_RT", r#"{"beams": [{"name": "ref", "path": "RT"}]}"#);
    let e = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("ref"),
            path: Some("TR"),
            ..Default::default()
        },
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(e.contains("beam ref has its own path"), "{e}");
}

const HEAD: &str = r#""wavelengths": [{"um": 0.5875618}], "glass_catalogs": ["SCHOTT"],
  "aperture": {"type": "epd", "value": 4}, "object_distance": 100,"#;

/// 玉 L を往復するダブルパス（L の面 0 は R 40、面 1 は R −60）
fn double_pass(glrs: &str, ops: &str) -> Result<lensbench_project::Expansion, String> {
    let txt = format!(
        r#"{{ {HEAD} {glrs} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L", "surface_ops": {ops}, "source": {{"surfaces": [
             {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
          {{"type": "air", "t": 20}}, {{"type": "mirror", "back": true, "semi": 10}}]}}"#
    );
    let p = Project::from_json(&txt).map_err(|e| e.to_string())?;
    expand(&p, &env(), &ExpandOpts::default()).map_err(|e| e.to_string())
}

/// 面の参照の back で復路の面を指す（面の番号は往路と同じで、復路は逆の順）
#[test]
fn back_refers_to_the_return_faces() {
    let x = double_pass(
        r#""glrs": {"block": "L", "surface": 0, "back": true},"#,
        "[]",
    )
    .unwrap();
    let s = &x.system;
    let g = s.glrs;
    assert!(x.map[g].back, "{g}");
    // 面 0（R 40）を鏡の後で通る（鏡の後も面の形の曲率は同じ符号）
    assert!(
        (s.surfaces[g].c - 1.0 / 40.0).abs() < 1e-15,
        "{}",
        s.surfaces[g].c
    );
    let fwd = double_pass(r#""glrs": {"block": "L", "surface": 0},"#, "[]").unwrap();
    assert!(!fwd.map[fwd.system.glrs].back);
    assert!(fwd.system.glrs < g);

    // 復路の面へのピックアップ: 復路の面 0 の曲率を往路の面 1 から
    let x = double_pass(
        "",
        r#"[{"op": "add_pickup", "dst": {"block": "L", "surface": 0, "back": true},
             "src": {"block": "L", "surface": 1}, "what": "c"}]"#,
    )
    .unwrap();
    let s = &x.system;
    let j = s.pickups[0].dst;
    assert!(x.map[j].back);
    assert!(
        (s.surfaces[j].c + 1.0 / 60.0).abs() < 1e-15,
        "{}",
        s.surfaces[j].c
    );

    // 範囲の外の pass・面、復路の座標ブレークは誤り
    for (glrs, want) in [
        (
            r#""glrs": {"block": "L", "surface": 0, "back": true, "pass": 1},"#,
            "block L pass 1 (1 return passes)",
        ),
        (
            r#""glrs": {"block": "L", "surface": 2, "back": true},"#,
            "block L surface 2 (2 surfaces in the pass)",
        ),
        (
            r#""glrs": {"block": "L", "surface": 0, "back": true, "coordbrk": true},"#,
            "a coordinate break of the return path can't be chosen",
        ),
    ] {
        let e = double_pass(glrs, "[]").map(|_| ()).unwrap_err();
        assert!(e.contains(want), "{glrs}: {e}");
    }
    // pass 0・back なしは書き出さない（今のファイルと同じ）
    let p = Project::from_json(&format!(
        r#"{{ {HEAD} "glrs": {{"block": "L", "surface": 1, "pass": 0}}, "blocks": [{{"type": "stop", "semi": 2}}]}}"#
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    assert_eq!(v["glrs"], serde_json::json!({"block": "L", "surface": 1}));
}

/// 書き戻し: 復路の面へのソルブ・ピックアップは、ブロックの面の back（往路と同じ番号）で書く。復路の面の値の上書きは
/// 往路の面で（書かずに説明を返す）
#[test]
fn write_back_puts_solves_and_pickups_on_return_faces() {
    use lensbench_project::edit_spec::EditOp;
    let x = double_pass("", "[]").unwrap();
    let faces: Vec<_> = x.map.iter().map(|o| o.face.clone()).collect();
    // 復路の L の面 0（復路では L の 2 つ目の面）と往路の L の面 1
    let back0 = (0..faces.len())
        .find(|&j| {
            faces[j]
                .as_ref()
                .is_some_and(|f| f.back && !f.cb && f.index == 1)
        })
        .unwrap();
    let fwd1 = (0..faces.len())
        .find(|&j| {
            faces[j]
                .as_ref()
                .is_some_and(|f| !f.back && !f.cb && f.index == 1)
        })
        .unwrap();
    let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
        { "op": "add_pickup", "dst": back0, "src": fwd1, "what": "c" },
        { "op": "add_solve", "surface": back0, "kind": "marginal_height", "value": 0.0 },
        { "op": "set_surface", "surface": back0, "fields": { "r": 30.0 } },
    ]))
    .unwrap();
    let mut p = Project::from_json(&format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L", "source": {{"surfaces": [
             {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
          {{"type": "air", "t": 20}}, {{"type": "mirror", "back": true, "semi": 10}}]}}"#
    ))
    .unwrap();
    let ed = lensbench_project::writeback::Edited {
        system: &x.system,
        faces: &faces,
        before: &x.system,
    };
    let notes = lensbench_project::writeback::write_back(&mut p, ed, &ops, &|_| Ok(()));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("edit 3") && notes[0].contains("later pass"),
        "{notes:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    let ops = &v["blocks"][2]["surface_ops"];
    let back = serde_json::json!({"block": "L", "surface": 0, "back": true});
    assert_eq!(ops[0]["dst"], back, "{ops}");
    assert_eq!(
        ops[0]["src"],
        serde_json::json!({"block": "L", "surface": 1}),
        "{ops}"
    );
    assert_eq!(ops[1]["surface"], back, "{ops}");
    // 展開すると復路の面 0 にピックアップとソルブ
    let y = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    assert_eq!(y.system.pickups.len(), 1);
    assert_eq!(y.system.pickups[0].dst, back0);
    assert!(
        y.system.solves.iter().any(|q| q.surf == back0),
        "{:?}",
        y.system.solves
    );
}
