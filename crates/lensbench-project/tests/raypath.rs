//! 光線の経路（RayPath）: 光束の turn・ゴーストの (a, b) を出来事の列に直す

use lensbench_project::raypath::{EventKind, RayPath};
use lensbench_project::{expand, Env, ExpandOpts, Expansion, Project};
use std::path::PathBuf;

fn env() -> Env<'static> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Env {
        step_files: None,
        base: root.join("tests/refs"),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    }
}

/// 絞り・玉 L（面 0 は R 40、面 1 は R −60）・空気。光束 g は L の面 turn で折り返す
fn expanded(turn: usize, state: Option<&str>) -> Expansion {
    let txt = format!(
        r#"{{"wavelengths": [{{"um": 0.5875618}}], "glass_catalogs": ["SCHOTT"],
          "aperture": {{"type": "epd", "value": 4}}, "object_distance": 100,
          "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L", "source": {{"surfaces": [
             {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
          {{"type": "air", "t": 20}}],
          "beams": [{{"name": "g", "turn": {{"block": "L", "surface": {turn}}}}}]}}"#
    );
    let p = Project::from_json(&txt).unwrap();
    expand(
        &p,
        &env(),
        &ExpandOpts {
            state,
            ..Default::default()
        },
    )
    .unwrap()
}

/// 経路の無い系: 面ごとに透過し、像面で検出。面の名前はプロジェクトの面
#[test]
fn a_plain_system_transmits_every_face() {
    let x = expanded(0, None);
    let p = RayPath::sequential(&x.system, Some(&x.map)).unwrap();
    assert_eq!(p.code(), "T T T D");
    assert_eq!(p.reflections(), 0);
    let l: Vec<usize> = p.0[1..3]
        .iter()
        .map(|e| e.face.as_ref().unwrap().index)
        .collect();
    assert_eq!(l, [0, 1]);
    assert!(p.0[3].face.is_none());
    assert_eq!(p.0.last().unwrap().kind, EventKind::Detect);
}

/// turn の光束: 折り返す面で 1 回反射し、往路の面を戻る（戻りの面は back の名前）
#[test]
fn a_turned_beam_reflects_once() {
    for turn in [0, 1] {
        let x = expanded(turn, Some("g"));
        let p = RayPath::sequential(&x.system, Some(&x.map)).unwrap();
        assert_eq!(p.reflections(), 1, "{turn}: {}", p.code());
        let r =
            p.0.iter()
                .position(|e| e.kind == EventKind::Reflect)
                .unwrap();
        let f = p.0[r].face.as_ref().unwrap();
        assert_eq!((f.index, f.back), (turn, false), "{turn}");
        assert!(p.0[r + 1..p.len() - 1]
            .iter()
            .all(|e| e.kind == EventKind::Transmit && e.face.as_ref().is_some_and(|f| f.back)));
        assert_eq!(p.0.last().unwrap().kind, EventKind::Detect);
    }
}

/// ゴースト (a, b) = (L の面 1, L の面 0): 面 1 で反射して戻り、面 0 で反射して像面へ
#[test]
fn a_ghost_path() {
    let x = expanded(0, None);
    let s = &x.system;
    let (a, b) = (3, 2);
    let p = RayPath::ghost(s, Some(&x.map), a, b).unwrap();
    assert_eq!(p.code(), "T T R R T D");
    let idx: Vec<Option<usize>> =
        p.0.iter()
            .map(|e| e.face.as_ref().map(|f| f.index))
            .collect();
    assert_eq!(idx, [Some(0), Some(0), Some(1), Some(0), Some(1), None]);
    assert_eq!(
        p.0.iter().map(|e| e.surf).collect::<Vec<_>>(),
        [1, 2, 3, 2, 3, 4]
    );
    // 像面で反射するゴースト、面の名前なし
    let q = RayPath::ghost(s, None, 4, 2).unwrap();
    assert_eq!(q.code(), "T T T R T R T D");
    assert!(q.0.iter().all(|e| e.face.is_none()));
    assert!(RayPath::ghost(s, None, 2, 3).is_err());
    assert!(RayPath::ghost(s, None, 5, 2).is_err());
    assert!(RayPath::sequential(s, Some(&x.map[1..])).is_err());
}
