use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::{expand, Env, Project};
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

fn project(blocks: &str) -> System {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    let p = Project::from_json(&txt).unwrap();
    expand(&p, &env()).unwrap()
}

/// 像面の光線（局所座標）を比べる
fn same_rays(a: &System, b: &System, tol: f64) {
    let (px, py) = (vec![0.0, 0.7, -0.5, 0.3], vec![0.0, 0.2, 0.6, -0.9]);
    for f in 0..a.fields.len() {
        for w in 0..a.wavelengths.len() {
            let t0 = a
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            let t1 = b
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for i in 0..px.len() {
                let (r0, r1) = (&t0.rays[i], &t1.rays[i]);
                assert_eq!(r0.err, r1.err);
                for k in 0..3 {
                    assert!(
                        (r0.xyz[k] - r1.xyz[k]).abs() < tol,
                        "{f} {w} {i} {:?} {:?}",
                        r0.xyz,
                        r1.xyz
                    );
                }
            }
        }
    }
}

#[test]
fn a_whole_zmx_block_is_the_system() {
    let s = project(r#"[{"type": "zmx", "path": "A0/system.zmx"}]"#);
    let a = a0();
    assert_eq!(s.surfaces.len(), a.surfaces.len());
    same_rays(&s, &a, 1e-12);
}

#[test]
fn lens_blocks_rebuild_the_triplet() {
    // A0: 玉 3 つ、絞りは 2 つ目と 3 つ目の間の面、像の手前にダミー面
    let a = a0();
    let t: Vec<f64> = a.surfaces.iter().map(|s| s.t).collect();
    let blocks = format!(
        r#"[{{"type": "lens", "source": {{"zmx": "A0/system.zmx", "lens": 1}}}}, {{"type": "air", "t": {}}},
            {{"type": "lens", "source": {{"zmx": "A0/system.zmx", "lens": 2}}}}, {{"type": "air", "t": {}}},
            {{"type": "stop", "semi": {}}}, {{"type": "air", "t": {}}},
            {{"type": "lens", "source": {{"zmx": "A0/system.zmx", "lens": 3}}}}, {{"type": "air", "t": {}}},
            {{"type": "surfaces", "surfaces": [{{"t": {}}}]}}]"#,
        t[2], t[4], a.surfaces[5].semi, t[5], t[7], t[8]
    );
    let s = project(&blocks);
    assert_eq!(s.surfaces.len(), a.surfaces.len());
    same_rays(&s, &a, 1e-12);
}

#[test]
fn a_tilted_lens_returns_to_the_axis() {
    let lens = r#"{"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}"#;
    let mk = |place: &str| {
        project(&format!(
            r#"[{{"type": "stop"}}, {{"type": "air", "t": 5}}, {lens}{place}}}, {{"type": "air", "t": 50}}]"#
        ))
    };
    let s0 = mk("");
    let s1 = mk(r#", "decenter": [0.3, -0.2], "tilt": [2, -1, 5]"#);
    // 傾けた玉の後ろの面（像面）の座標系は、傾けないときと同じ
    let (f0, f1) = (s0.frames(), s1.frames());
    let (i0, i1) = (s0.image(), s1.image());
    let (a, b) = (f0.surf[i0].unwrap(), f1.surf[i1].unwrap());
    for k in 0..3 {
        assert!((a.o[k] - b.o[k]).abs() < 1e-12, "{:?} {:?}", a.o, b.o);
        for m in 0..3 {
            assert!((a.r[k][m] - b.r[k][m]).abs() < 1e-14);
        }
    }
    // 玉の前面は偏心した点
    let j = (0..s1.surfaces.len())
        .find(|&j| s1.surfaces[j].mat == "N-BK7")
        .unwrap();
    let fj = f1.surf[j].unwrap();
    assert!((fj.o[0] - 0.3).abs() < 1e-12 && (fj.o[1] + 0.2).abs() < 1e-12);
    // OFF: 空気で残すと像面は同じ位置、詰めると玉の厚さだけ手前
    let s2 = mk(r#", "on": false, "off": "air""#);
    let s3 = mk(r#", "on": false"#);
    let z = |s: &System| s.frames().surf[s.image()].unwrap().o[2];
    assert!((z(&s2) - z(&s0)).abs() < 1e-12);
    assert!((z(&s0) - z(&s3) - 4.0).abs() < 1e-12);
}

#[test]
fn a_lens_after_a_fold_mirror_works_like_the_unfolded_one() {
    let lens = r#"{"type": "lens", "source": {"surfaces": [{"radius": 50, "t": 5, "glass": "N-BK7"}, {"radius": -50}]}}"#;
    let unf = project(&format!(
        r#"[{{"type": "stop"}}, {{"type": "air", "t": 30}}, {lens}, {{"type": "air", "t": 45}}]"#
    ));
    let fold = project(&format!(
        r#"[{{"type": "stop"}}, {{"type": "air", "t": 10}}, {{"type": "mirror"}}, {{"type": "air", "t": 20}}, {lens}, {{"type": "air", "t": 45}}]"#
    ));
    // 鏡のあとは厚さが負
    assert!(fold.surfaces.iter().skip(4).all(|s| s.t <= 0.0));
    assert!(
        (unf.paraxial().efl - fold.paraxial().efl).abs() < 1e-9,
        "{} {}",
        unf.paraxial().efl,
        fold.paraxial().efl
    );
    // 像面の光線: x は同じ、y は同じ（平面鏡は z だけ返す）
    same_rays_xy(&unf, &fold);
}

fn same_rays_xy(a: &System, b: &System) {
    let (px, py) = (vec![0.0, 0.7, -0.5, 0.3], vec![0.0, 0.2, 0.6, -0.9]);
    for f in 0..a.fields.len() {
        let t0 = a
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        let t1 = b
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        for i in 0..px.len() {
            for k in 0..2 {
                assert!(
                    (t0.rays[i].xyz[k] - t1.rays[i].xyz[k]).abs() < 1e-11,
                    "{:?} {:?}",
                    t0.rays[i].xyz,
                    t1.rays[i].xyz
                );
            }
        }
    }
}
