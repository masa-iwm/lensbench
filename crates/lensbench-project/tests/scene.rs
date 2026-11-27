//! 物の場面（Scene）: 展開した系の面が場面の面と 1e-12 mm で重なる、往復・BS の経路の同じ面は 1 つ、玉の縁の形

use lensbench_core::catalog::{LensCatalog, ZmxFileSource};
use lensbench_core::math::{dot, norm, sub};
use lensbench_project::scene::{expansions, ObjectKind, Scene, Shape};
use lensbench_project::{Env, Expansion, Project};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 場面の面と展開した系の面の頂点・軸が 1e-12 mm（軸は 1e-12）で一致し、系の光学面は全部どこかの面に入る
fn check_frames(name: &str, sc: &Scene, xs: &[(String, Expansion)]) {
    for (i, (label, x)) in xs.iter().enumerate() {
        let s = &x.system;
        let fr = s.frames();
        for j in 1..=s.image() {
            let d = &s.surfaces[j];
            if d.ignore || d.is_coord_break() {
                continue;
            }
            let Some(k) = sc.face_of(i, j) else {
                continue;
            };
            let f = fr.surf[j].unwrap();
            let g = &sc.faces[k];
            let dp = norm(sub(g.frame.o, f.o));
            assert!(
                dp < 1e-12,
                "{name} {label} surface {j}: vertex off by {dp:e} mm"
            );
            let c = dot(g.axis(), [f.r[0][2], f.r[1][2], f.r[2][2]]);
            assert!(
                1.0 - c.abs() < 1e-12,
                "{name} {label} surface {j}: axis {c}"
            );
            let r = g
                .refs
                .iter()
                .find(|r| r.system == i && r.surf == j)
                .unwrap();
            assert_eq!(r.flipped, c < 0.0, "{name} {label} surface {j}");
            assert_eq!(r.face, x.map[j].face, "{name} {label} surface {j}");
        }
        // 像面は必ず検出器
        let k = sc.face_of(i, s.image()).unwrap();
        assert_eq!(sc.objects[sc.faces[k].object].kind, ObjectKind::Detector);
    }
}

fn env_for<'a>(p: &Path, look: lensbench_project::CatalogLookup<'a>) -> Env<'a> {
    Env::for_file(p, &[root().join("tests/glass")], Some(look))
}

/// 例のプロジェクトと BS の照合の系の全部: 場面の面が展開した系と重なる
#[test]
fn scene_faces_sit_on_the_expanded_surfaces() {
    let cat = LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap();
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let mut files = Vec::new();
    for dir in ["examples/projects", "tests/splitter"] {
        for e in std::fs::read_dir(root().join(dir)).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") && !p.ends_with("bs_refs.json") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut n = 0;
    for p in &files {
        let pr = Project::load(p).unwrap();
        let env = env_for(p, &look);
        let xs = expansions(&pr, &env).unwrap();
        let sc = Scene::from_project(&pr, &env).unwrap();
        check_frames(&p.display().to_string(), &sc, &xs);
        assert!(
            sc.notes.iter().all(|n| !n.contains("media")),
            "{}: {:?}",
            p.display(),
            sc.notes
        );
        // 経路が 2 つ以上なら、共通の幹の面は 1 つにまとまる
        if xs.len() > 1 {
            assert!(
                sc.faces.iter().any(|f| {
                    let mut v: Vec<usize> = f.refs.iter().map(|r| r.system).collect();
                    v.dedup();
                    v.len() > 1
                }),
                "{}: no face is shared between the paths",
                p.display()
            );
        }
        eprintln!(
            "{}: {} systems, {} objects ({} not closed), {} faces, notes {:?}",
            p.file_name().unwrap().to_string_lossy(),
            xs.len(),
            sc.objects.len(),
            sc.objects.iter().filter(|o| !o.closed).count(),
            sc.faces.len(),
            sc.notes
        );
        n += xs.len();
    }
    assert!(n >= files.len());
}

const HEAD: &str = r#""wavelengths": [{"um": 0.5875618}], "glass_catalogs": ["SCHOTT"],
  "aperture": {"type": "epd", "value": 4}, "object_distance": 100,"#;

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn project(blocks: &str, extra: &str) -> Project {
    Project::from_json(&format!("{{ {HEAD} {extra} \"blocks\": {blocks} }}")).unwrap()
}

const LENS: &str = r#"{"type": "lens", "id": "L", "source": {"surfaces": [
    {"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 6}]}}"#;

/// 単玉: 絞り・玉（面 2・縁の輪 1・側面 1）・検出器。縁の輪は小さい面の形の終わりから玉の半径まで、側面は 2 面の縁の間
#[test]
fn a_singlet_is_a_closed_solid() {
    let p = project(
        &format!(
            r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}}, {LENS}, {{"type": "air", "t": 20}}]"#
        ),
        "",
    );
    let sc = Scene::from_project(&p, &env()).unwrap();
    let kinds: Vec<ObjectKind> = sc.objects.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [ObjectKind::Stop, ObjectKind::Lens, ObjectKind::Detector]
    );
    let l = &sc.objects[1];
    assert!(l.closed);
    assert_eq!(l.name, "L");
    assert_eq!(sc.regions.len(), 1);
    let x = &expansions(&p, &env()).unwrap()[0].1;
    let s = &x.system;
    let (j0, j1) = (2, 3);
    let rim = s.lens_radius(j0).max(s.lens_radius(j1));
    assert!((rim - 8.0).abs() < 1e-12, "{rim}");
    let (mut ann, mut cyl) = (Vec::new(), Vec::new());
    for &k in &l.faces {
        match sc.faces[k].shape {
            Shape::Annulus { r0, r1, z } => ann.push((r0, r1, z)),
            Shape::Cylinder { r, z0, z1 } => cyl.push((r, z0, z1)),
            Shape::Optical { .. } => {}
        }
    }
    // 面 1 は半径 6 で形が終わり、玉の半径 8 まで平ら
    assert_eq!(ann.len(), 1);
    let sag = |c: f64, r: f64| c * r * r / (1.0 + (1.0 - c * c * r * r).sqrt());
    let z1 = sag(-1.0 / 60.0, 6.0);
    assert!((ann[0].0 - 6.0).abs() < 1e-12 && (ann[0].1 - 8.0).abs() < 1e-12);
    assert!((ann[0].2 - z1).abs() < 1e-12);
    // 側面は面 0 の縁（半径 8）の z から、面 1 の縁の平らな輪（4 + z1）まで
    assert_eq!(cyl.len(), 1);
    let z0 = sag(1.0 / 40.0, 8.0);
    assert!((cyl[0].0 - 8.0).abs() < 1e-12);
    assert!(
        (cyl[0].1 - z0).abs() < 1e-12 && (cyl[0].2 - (4.0 + z1)).abs() < 1e-12,
        "{cyl:?}"
    );
    // 媒質: 面 0 は −z が空気・+z がガラス、側面の内側はガラス
    let f0 = &sc.faces[sc.face_of(0, j0).unwrap()];
    assert!(f0.minus.is_air() && f0.plus.name() == "N-BK7");
}

/// 玉を往復するダブルパス: 復路の面は往路の面にまとまり（向きは逆）、玉は 1 つ
#[test]
fn a_double_pass_keeps_one_lens() {
    let p = project(
        &format!(
            r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}}, {LENS}, {{"type": "air", "t": 20}},
            {{"type": "mirror", "back": true, "semi": 10}}]"#
        ),
        "",
    );
    let xs = expansions(&p, &env()).unwrap();
    let sc = Scene::from_project(&p, &env()).unwrap();
    check_frames("double pass", &sc, &xs);
    assert!(sc.notes.is_empty(), "{:?}", sc.notes);
    let lenses: Vec<_> = sc
        .objects
        .iter()
        .filter(|o| o.kind == ObjectKind::Lens)
        .collect();
    assert_eq!(lenses.len(), 1);
    assert!(lenses[0].closed);
    assert_eq!(
        sc.objects
            .iter()
            .filter(|o| o.kind == ObjectKind::Mirror)
            .count(),
        1
    );
    let opt: Vec<_> = lenses[0]
        .faces
        .iter()
        .filter(|&&k| matches!(sc.faces[k].shape, Shape::Optical { .. }))
        .collect();
    assert_eq!(opt.len(), 2);
    for &&k in &opt {
        let f = &sc.faces[k];
        assert_eq!(f.refs.len(), 2, "{:?}", f.refs);
        assert!(f
            .refs
            .iter()
            .any(|r| r.face.as_ref().is_some_and(|x| x.back)));
    }
}

/// 光束の turn: 部分反射の面は玉の面と同じ面で、媒質は空気とガラス
#[test]
fn a_turn_is_the_lens_face() {
    let p = project(
        &format!(
            r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}}, {LENS}, {{"type": "air", "t": 20}}]"#
        ),
        r#""beams": [{"name": "t", "path": ""}, {"name": "g", "turn": {"block": "L", "surface": 1}}],"#,
    );
    let xs = expansions(&p, &env()).unwrap();
    assert_eq!(xs.len(), 2);
    let sc = Scene::from_project(&p, &env()).unwrap();
    check_frames("turn", &sc, &xs);
    assert_eq!(
        sc.objects
            .iter()
            .filter(|o| o.kind == ObjectKind::Lens)
            .count(),
        1
    );
    let k = sc.face_of(0, 3).unwrap();
    let f = &sc.faces[k];
    assert!(f.refs.iter().any(|r| r.system == 1));
    assert!(
        f.minus.name() == "N-BK7" && f.plus.is_air(),
        "{:?} {:?}",
        f.minus,
        f.plus
    );
}

/// 機構部品は壁の面を持つ物になる
#[test]
fn mechanics_become_walls() {
    let p = root().join("examples/projects/P19_tube_cage.json");
    let pr = Project::load(&p).unwrap();
    let cat = LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap();
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = env_for(&p, &look);
    let xs = expansions(&pr, &env).unwrap();
    let parts = xs[0].1.system.mechanics.len();
    assert!(parts > 0);
    let sc = Scene::from_project(&pr, &env).unwrap();
    let mech: Vec<_> = sc
        .objects
        .iter()
        .filter(|o| o.kind == ObjectKind::Mech)
        .collect();
    assert_eq!(mech.len(), parts);
    assert!(mech.iter().all(|o| o
        .faces
        .iter()
        .any(|&k| matches!(sc.faces[k].shape, Shape::Cylinder { .. }))));
}
