use lensbench_core::system::FieldSel;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
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
    expand(&p, &env(), &ExpandOpts::default())
        .map(|x| x.system)
        .unwrap()
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
                assert_eq!(r0.status, r1.status);
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
fn vig_interp_sets_the_system() {
    use lensbench_core::system::VigInterp;
    let blocks = r#"[{"type": "zmx", "path": "A0/system.zmx"}]"#;
    assert_eq!(project(blocks).vig_interp, VigInterp::Continuous);
    let txt = format!("{{ {HEAD} \"vig_interp\": \"opticstudio\", \"blocks\": {blocks} }}");
    let p = Project::from_json(&txt).unwrap();
    let s = expand(&p, &env(), &ExpandOpts::default()).unwrap().system;
    assert_eq!(s.vig_interp, VigInterp::OpticStudio);
}

#[test]
fn a_zmx_range_keeps_only_the_air_right_after_it() {
    // A0 の面 1〜2（1 枚目の玉）: 玉の後ろの空気 5.8 だけが付き、範囲の外の空気は足さない
    let a = a0();
    let s = project(
        r#"[{"type": "zmx", "path": "A0/system.zmx", "from": 1, "to": 2}, {"type": "stop", "semi": 3}]"#,
    );
    let t: Vec<f64> = s.surfaces.iter().map(|d| d.t).collect();
    // 物体、玉の 2 面、絞り、像面
    assert_eq!(s.surfaces.len(), 5, "{t:?}");
    assert_eq!(s.surfaces[1].c, a.surfaces[1].c);
    assert_eq!(s.surfaces[2].c, a.surfaces[2].c);
    assert_eq!(t[1], a.surfaces[1].t);
    assert_eq!(t[2], a.surfaces[2].t, "{t:?}");
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
    same_rays_xy_within(a, b, 1e-11);
}

fn same_rays_xy_within(a: &System, b: &System, tol: f64) {
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
                    (t0.rays[i].xyz[k] - t1.rays[i].xyz[k]).abs() < tol,
                    "{:?} {:?}",
                    t0.rays[i].xyz,
                    t1.rays[i].xyz
                );
            }
        }
    }
}

#[test]
fn a_double_pass_returns_to_the_start() {
    // 絞り → 20 → 偏心・傾けた玉 → 30 → 平面鏡（往路を戻る）→ 復路で玉を逆に通って絞りの面へ
    let lens = r#"{"type": "lens", "decenter": [0.2, -0.1], "tilt": [1.5, -0.7, 3], "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}}"#;
    let s = project(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {lens}, {{"type": "air", "t": 30}}, {{"type": "mirror", "back": true}}]"#
    ));
    let f = s.frames();
    // 復路の最後の面（絞りの面に戻ったダミー）は、絞りの面と同じ座標系
    let ret = s
        .surfaces
        .iter()
        .rposition(|d| d.comment == "STOP (return)")
        .unwrap();
    let st = s.stop();
    let (a, b) = (f.surf[st].unwrap(), f.surf[ret].unwrap());
    for k in 0..3 {
        assert!((a.o[k] - b.o[k]).abs() < 1e-12, "{:?} {:?}", a.o, b.o);
        for m in 0..3 {
            assert!((a.r[k][m] - b.r[k][m]).abs() < 1e-14);
        }
    }
    // 復路の玉の面は往路と同じ面（同じ座標系・同じ曲率）
    let glass: Vec<usize> = (0..s.surfaces.len())
        .filter(|&j| {
            s.surfaces[j].typ() == lensbench_core::surface::SurfType::Standard
                && s.surfaces[j].c != 0.0
        })
        .collect();
    assert_eq!(glass.len(), 4);
    for (p, q) in [(glass[0], glass[3]), (glass[1], glass[2])] {
        assert_eq!(s.surfaces[p].c, s.surfaces[q].c);
        let (x, y) = (f.surf[p].unwrap(), f.surf[q].unwrap());
        for k in 0..3 {
            assert!(
                (x.o[k] - y.o[k]).abs() < 1e-12,
                "{p} {q} {:?} {:?}",
                x.o,
                y.o
            );
        }
    }
    // 光線は往復して像面（絞りの面の位置）に届く
    let t = s
        .try_trace(
            FieldSel::Index(0),
            &[0.0, 0.5],
            &[0.0, 0.3],
            None,
            None,
            false,
        )
        .unwrap();
    assert!(t.ok(0) && t.ok(1));
}

fn lb_catalog() -> lensbench_core::catalog::LensCatalog {
    use lensbench_core::catalog::{LensCatalog, ZmxFileSource};
    LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap()
}

fn project_with(
    blocks: &str,
    cat: &lensbench_core::catalog::LensCatalog,
) -> lensbench_core::Result<System> {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    let p = Project::from_json(&txt).unwrap();
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let e = Env {
        step_files: None,
        base: root().join("examples/lenses/LB"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: Some(&look),
    };
    expand(&p, &e, &ExpandOpts::default())
        .map_err(lensbench_core::Error::from)
        .map(|x| x.system)
}

#[test]
fn catalog_lenses_are_the_lenses_of_their_files() {
    // 自作のカタログ（examples/lenses/LB、1 ファイル 1 レンズ）の 2〜4 枚の組（空気間隔、反転）が、
    // 同じ .zmx の玉を直接読んで組んだ系と光線まで同じ
    let cat = lb_catalog();
    assert_eq!(cat.len(), 5);
    let combos: [&[(&str, bool)]; 3] = [
        &[("PCX-100", false), ("ACH-45", true)],
        &[("BCX-50", false), ("BCC-32", true), ("MEN-75", false)],
        &[
            ("PCX-100", true),
            ("ACH-45", false),
            ("ACH-45", true),
            ("BCX-50", false),
        ],
    ];
    for c in combos {
        let mk = |by_catalog: bool| {
            let mut b = vec![
                r#"{"type": "stop", "semi": 4}"#.to_string(),
                r#"{"type": "air", "t": 5}"#.into(),
            ];
            for (k, (part, rev)) in c.iter().enumerate() {
                let src = if by_catalog {
                    // 大文字・小文字は区別しない
                    let v = if k % 2 == 0 { "LB" } else { "lb" };
                    format!(r#"{{"vendor": "{v}", "part": "{part}"}}"#)
                } else {
                    format!(r#"{{"zmx": "{part}.zmx", "lens": 1}}"#)
                };
                b.push(format!(
                    r#"{{"type": "lens", "source": {src}, "reversed": {rev}}}"#
                ));
                b.push(format!(r#"{{"type": "air", "t": {}}}"#, 3 + 2 * k));
            }
            b.push(r#"{"type": "air", "t": 40}"#.into());
            project_with(&format!("[{}]", b.join(", ")), &cat).unwrap()
        };
        let (a, b) = (mk(true), mk(false));
        assert_eq!(a.surfaces.len(), b.surfaces.len());
        same_rays(&a, &b, 0.0 + f64::MIN_POSITIVE);
    }
    // 無い型番はエラー
    let e = project_with(
        r#"[{"type": "stop"}, {"type": "air", "t": 1}, {"type": "lens", "source": {"vendor": "LB", "part": "NONE"}}, {"type": "air", "t": 1}]"#,
        &cat,
    )
    .unwrap_err();
    assert!(e.message().unwrap_or_default().contains("NONE"), "{e}");
}

#[test]
fn a_tilted_return_mirror_tilts_only_the_mirror() {
    // 往路を戻る鏡を傾けても、復路の面は往路と同じ位置。光線は鏡の傾きの 2 倍の角で戻る
    let lens = r#"{"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}}"#;
    let mk = |tilt: &str| {
        project(&format!(
            r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {lens}, {{"type": "air", "t": 30}}, {{"type": "mirror", "back": true, "tilt": {tilt}}}]"#
        ))
    };
    let (s0, s1) = (mk("[0, 0]"), mk("[0.7, -0.4]"));
    let (f0, f1) = (s0.frames(), s1.frames());
    // s1 は鏡の前後に座標ブレークが 2 つ多い。鏡の後の面を後ろから順に対応させる
    let (n0, n1) = (s0.surfaces.len(), s1.surfaces.len());
    assert_eq!(n1, n0 + 2);
    let m0 = s0.surfaces.iter().position(|d| d.is_mirror()).unwrap();
    for k in 1..(n0 - m0) {
        let (a, b) = (f0.surf[n0 - k].unwrap(), f1.surf[n1 - k].unwrap());
        for i in 0..3 {
            assert!((a.o[i] - b.o[i]).abs() < 1e-12, "{k} {:?} {:?}", a.o, b.o);
            for j in 0..3 {
                assert!((a.r[i][j] - b.r[i][j]).abs() < 1e-14);
            }
        }
    }
    // 鏡そのものは傾いている
    let m1 = s1.surfaces.iter().position(|d| d.is_mirror()).unwrap();
    let r = f1.surf[m1].unwrap().r;
    assert!(
        (r[2][2] - (0.7f64.to_radians().cos() * 0.4f64.to_radians().cos())).abs() < 1e-12,
        "{r:?}"
    );
}

#[test]
fn a_tilted_flat_return_mirror_sends_the_ray_back_at_twice_the_angle() {
    // 絞り → 10 → 平面鏡（θ 傾けて往路を戻る）→ 絞りの面: 軸上の主光線は y = −10 tan 2θ（x 軸まわりの傾き）
    let th: f64 = 1.0;
    let s = project(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 10}}, {{"type": "mirror", "back": true, "tilt": [{th}, 0], "semi": 5}}]"#
    ));
    let t = s
        .try_trace(FieldSel::Index(0), &[0.0], &[0.0], Some(1), None, false)
        .unwrap();
    assert!(t.ok(0));
    let y = t.rays[0].xyz[1];
    let want = 10.0 * (2.0 * th).to_radians().tan();
    assert!((y.abs() - want).abs() < 1e-12, "{y} {want}");
    assert!(t.rays[0].xyz[0].abs() < 1e-15);
}

#[test]
fn example_projects_expand_and_all_rays_pass() {
    // 全部の例を、既定の状態と全部の状態で展開し、7×7 の瞳の光線が通ることを見る
    let cat = lb_catalog();
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let dir = root().join("examples/projects");
    let g: Vec<f64> = (0..7).map(|i| -1.0 + i as f64 / 3.0).collect();
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for &b in &g {
        for &a in &g {
            if a * a + b * b <= 1.0 + 1e-12 {
                px.push(a);
                py.push(b);
            }
        }
    }
    let (mut n, mut n_states) = (0, 0);
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let pr = Project::from_json(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let mut states: Vec<Option<String>> = vec![None];
        states.extend(pr.state_names().into_iter().map(|x| Some(x.to_string())));
        for st in &states {
            let s = lensbench_project::Project::load(&p)
                .and_then(|q| {
                    lensbench_project::expand(
                        &q,
                        &lensbench_project::Env::for_file(
                            &p,
                            &[root().join("tests/glass")],
                            Some(&look),
                        ),
                        &lensbench_project::ExpandOpts {
                            state: st.as_deref(),
                            ..Default::default()
                        },
                    )
                    .map_err(lensbench_core::Error::from)
                })
                .map(|x| x.system)
                .unwrap_or_else(|e| panic!("{} {st:?}: {e}", p.display()));
            for f in 0..s.fields.len() {
                for w in 0..s.wavelengths.len() {
                    let t = s
                        .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                        .unwrap();
                    for i in 0..px.len() {
                        assert!(
                            t.ok(i),
                            "{} {st:?} 視野 {f} 波長 {w} 瞳 ({}, {})",
                            p.display(),
                            px[i],
                            py[i]
                        );
                    }
                }
            }
            n_states += usize::from(st.is_some());
        }
        n += 1;
    }
    assert!(n >= 14, "{n}");
    assert!(n_states >= 5, "{n_states}");
}

#[test]
fn a_zmx_lens_file_takes_its_vendor_from_the_folder_even_by_a_bare_name() {
    use lensbench_core::catalog::{LensCatalogSource, ZmxFileSource};
    let dir = root().join("examples/lenses/LB");
    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let l = ZmxFileSource.load(std::path::Path::new("BCX-50.zmx"));
    std::env::set_current_dir(cwd).unwrap();
    let l = l.unwrap();
    assert_eq!((l[0].vendor.as_str(), l[0].part.as_str()), ("LB", "BCX-50"));
}

fn try_project(blocks: &str) -> lensbench_core::Result<System> {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .map_err(lensbench_core::Error::from)
    .map(|x| x.system)
}

/// 面の並びに Jones 行列・座標ブレーク・開口の面を書ける
#[test]
fn surfaces_block_writes_jones_coordbrk_and_apertures() {
    use lensbench_core::surface::{Aperture, SurfType};
    let zmx = r#"{"type": "zmx", "path": "A0/system.zmx"}"#;
    let base = project(&format!("[{zmx}]"));
    let with = |s: &str| {
        project(&format!(
            r#"[{zmx}, {{"type": "surfaces", "surfaces": [{s}]}}]"#
        ))
    };
    // 恒等の Jones 行列と大きな矩形の開口は光線を変えない
    let j = with(
        r#"{"jones": [1, 0, 0.5, 0, 0, 0, 1, 0], "aperture": {"type": "rect", "hx": 50, "hy": 40, "dx": 1}}"#,
    );
    let k = j.image() - 1;
    assert_eq!(j.surfaces[k].typ(), SurfType::Jones);
    assert_eq!(
        j.surfaces[k].jones(),
        [1.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0, 0.0]
    );
    assert_eq!(
        j.surfaces[k].ap,
        Aperture::Rect {
            hx: 50.0,
            hy: 40.0,
            dx: 1.0,
            dy: 0.0
        }
    );
    same_rays(&base, &j, 1e-12);
    // 座標ブレークで dx だけずらすと、像面の局所 x が dx だけ減る
    let c = with(r#"{"coordbrk": [0.1, 0, 0, 0, 0], "order": 1}"#);
    assert_eq!(c.surfaces[k].typ(), SurfType::CoordBrk);
    assert_eq!(
        (c.surfaces[k].cb(), c.surfaces[k].cb_order()),
        ([0.1, 0.0, 0.0, 0.0, 0.0], 1)
    );
    // 座標ブレークにも浮動の開口を付けられる（半直径の円）
    let f = with(r#"{"coordbrk": [0, 0, 0, 0, 0], "semi": 2.5, "aperture": {"type": "floating"}}"#);
    assert_eq!(f.surfaces[k].typ(), SurfType::CoordBrk);
    assert_eq!(f.surfaces[k].ap, Aperture::Floating { semi: 2.5 });
    let (px, py) = ([0.0, 0.4], [0.3, -0.6]);
    let t0 = base
        .try_trace(FieldSel::Index(3), &px, &py, Some(1), None, false)
        .unwrap();
    let t1 = c
        .try_trace(FieldSel::Index(3), &px, &py, Some(1), None, false)
        .unwrap();
    for (a, b) in t0.rays.iter().zip(&t1.rays) {
        assert!((a.xyz[0] - 0.1 - b.xyz[0]).abs() < 1e-12);
        assert!((a.xyz[1] - b.xyz[1]).abs() < 1e-12);
    }
    // 小さな円の開口で光線がけられる
    let v = with(r#"{"aperture": {"type": "circular", "max": 0.001}}"#);
    let t = v
        .try_trace(FieldSel::Index(1), &px, &py, Some(1), None, false)
        .unwrap();
    assert!(t.rays.iter().all(|r| r.vig == k));
    // 書けない組み合わせ
    for (s, msg) in [
        (
            r#"{"jones": [1, 0, 0, 0, 0, 0, 1, 0], "coordbrk": [0, 0, 0, 0, 0]}"#,
            "two or more",
        ),
        (
            r#"{"jones": [1, 0, 0, 0, 0, 0, 1, 0], "radius": 20}"#,
            "curvature",
        ),
        (
            r#"{"coordbrk": [0, 0, 1, 0, 0], "glass": "N-BK7"}"#,
            "no glass after",
        ),
        (r#"{"coordbrk": [0, 0, 1, 0, 0], "order": 2}"#, "order"),
    ] {
        let e = try_project(&format!(
            r#"[{zmx}, {{"type": "surfaces", "surfaces": [{s}]}}]"#
        ))
        .unwrap_err();
        assert!(e.message().unwrap_or_default().contains(msg), "{s}: {e}");
    }
}

/// surface_edits の coordbrk はブロックの面の座標ブレークだけを数える（偏心・傾きの入口・戻りの座標ブレークは数えない）
#[test]
fn coordbrk_edits_skip_placement_breaks() {
    let s = try_project(
        r#"[{"type": "zmx", "path": "A0/system.zmx"},
            {"type": "surfaces", "id": "S", "decenter": [0.1, 0],
             "surfaces": [{"t": 1}, {"coordbrk": [0, 0, 0, 0, 0]}],
             "surface_edits": [{"surface": 0, "coordbrk": true, "fields": {"ignore": true}}]}]"#,
    )
    .unwrap();
    let cbs: Vec<usize> = (0..s.surfaces.len())
        .filter(|&j| s.surfaces[j].is_coord_break())
        .collect();
    let ign: Vec<bool> = cbs.iter().map(|&j| s.surfaces[j].ignore).collect();
    // 入口・面の座標ブレーク・戻り
    assert_eq!(ign[ign.len() - 3..], [false, true, false], "{cbs:?}");
    // 番号が座標ブレークの数を超えれば誤り
    let e = try_project(
        r#"[{"type": "zmx", "path": "A0/system.zmx"},
            {"type": "surfaces", "id": "S", "decenter": [0.1, 0], "surfaces": [{"t": 1}],
             "surface_edits": [{"surface": 0, "coordbrk": true, "fields": {"ignore": true}}]}]"#,
    )
    .unwrap_err();
    assert!(e.to_string().contains("coordinate break"), "{e}");
}

/// zmx ブロックの厚さのソルブ（A0 の面 7、像面の焦点）は、次の面まで取り込めば残り、set_param で解き直す
#[test]
fn thickness_solves_follow_set_param() {
    let txt =
        format!("{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": \"A0/system.zmx\"}}] }}");
    let (s, warn) = lensbench_project::expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &lensbench_project::ExpandOpts::default(),
    )
    .map(|x| (x.system, x.notes))
    .unwrap();
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.solves.len(), 1);
    let k = s.solves[0].surf;
    let a = s
        .with_params(&[(s.param("2.c").unwrap(), s.surfaces[2].c * 1.02)])
        .unwrap();
    assert!((a.surfaces[k].t - s.surfaces[k].t).abs() > 1e-4);
    assert!(a.paraxial().marginal.0[k + 1].abs() < 1e-12);
    // 次の面（面 8）を取り込まないと値で固定
    let txt = format!(
        "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": \"A0/system.zmx\", \"to\": 7}}, {{\"type\": \"air\", \"t\": 1}}] }}"
    );
    let (s, warn) = lensbench_project::expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &lensbench_project::ExpandOpts::default(),
    )
    .map(|x| (x.system, x.notes))
    .unwrap();
    assert!(s.solves.is_empty());
    assert!(warn.iter().any(|w| w.text.contains("solve")), "{warn:?}");
}

#[test]
fn semi_auto_lens_surfaces_follow_the_rays() {
    // 自作の玉の面の "semi": "auto"は光線から求め、固定の面はそのまま。カタログの玉は固定
    let s = project(
        r#"[{"type": "stop", "semi": 5}, {"type": "air", "t": 10},
            {"type": "lens", "source": {"surfaces": [
                {"radius": 40, "t": 4, "glass": "N-BK7", "semi": "auto"}, {"radius": -40, "semi": 9}]}},
            {"type": "air", "t": 50}]"#,
    );
    let j = (0..s.surfaces.len())
        .find(|&j| s.surfaces[j].mat == "N-BK7")
        .unwrap();
    assert!(s.surfaces[j].semi_auto && !s.surfaces[j + 1].semi_auto);
    assert_eq!(s.surfaces[j + 1].semi, 9.0);
    let h = s.ray_heights();
    assert!((s.surfaces[j].semi - h[j].unwrap()).abs() < 1e-12);
    assert!(s.surfaces[j].semi > 5.0, "{}", s.surfaces[j].semi);
    let cat = lb_catalog();
    let c = project_with(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 5},
            {"type": "lens", "source": {"vendor": "LB", "part": "BCC-32"}}, {"type": "air", "t": 40}]"#,
        &cat,
    )
    .unwrap();
    assert!(c.surfaces.iter().all(|d| !d.semi_auto));
}

#[test]
fn a_mirror_block_can_have_a_rect_aperture() {
    // 四角い鏡（補ったカタログ品など）: 矩形の開口が鏡の面に付く
    let s = project(
        r#"[{"type": "stop"}, {"type": "air", "t": 10}, {"type": "mirror", "semi": 11.18, "aperture": {"type": "rect", "hx": 5, "hy": 10}}, {"type": "air", "t": 20}]"#,
    );
    let m = s.surfaces.iter().find(|q| q.is_mirror()).unwrap();
    assert!(
        matches!(m.ap, lensbench_core::surface::Aperture::Rect { hx, hy, .. } if hx == 5.0 && hy == 10.0),
        "{:?}",
        m.ap
    );
}

/// zmx ブロックは lensbench の系（系ファイル・プロジェクトの .json）も取り込む
#[test]
fn a_zmx_block_takes_a_lensbench_system_json() {
    let a = a0();
    // 系ファイル
    let dir = std::env::temp_dir().join(format!("lb-json-block-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a0.json");
    lensbench_project::sysfile::save(&f, &a, &[]).unwrap();
    let s = project(&format!(
        r#"[{{"type": "zmx", "path": {}}}]"#,
        serde_json::to_string(&f.display().to_string()).unwrap()
    ));
    assert_eq!(s.surfaces.len(), a.surfaces.len());
    same_rays(&s, &a, 1e-12);
    // プロジェクト（中の zmx ブロックの相対パスはそのプロジェクトのフォルダから）
    let p = dir.join("p.json");
    std::fs::write(
        &p,
        format!(
            "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": {}}}] }}",
            serde_json::to_string(
                &root()
                    .join("tests/refs/A0/system.zmx")
                    .display()
                    .to_string()
            )
            .unwrap()
        ),
    )
    .unwrap();
    let s = project(&format!(
        r#"[{{"type": "zmx", "path": {}, "from": 1, "to": 2}}, {{"type": "stop", "semi": 3}}]"#,
        serde_json::to_string(&p.display().to_string()).unwrap()
    ));
    assert_eq!(s.surfaces[1].c, a.surfaces[1].c);
    assert_eq!(s.surfaces[2].c, a.surfaces[2].c);
    // 構成の番号は 1 だけ
    let e = try_project(&format!(
        r#"[{{"type": "zmx", "path": {}, "config": 2}}]"#,
        serde_json::to_string(&f.display().to_string()).unwrap()
    ))
    .unwrap_err();
    assert!(
        e.message()
            .unwrap_or_default()
            .contains("only configuration 1"),
        "{e}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ブロックの中で起きた誤りは、そのブロックの位置と名前を持つ（群の中の id の無いブロックは「#番号」）
#[test]
fn expand_errors_carry_the_block() {
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop"}},
            {{"type": "group", "id": "G1", "blocks": [{{"type": "air", "t": 1}}, {{"type": "zmx", "path": "no-such-file.zmx"}}]}},
            {{"type": "air", "t": 5}}] }}"#
    );
    let e = expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .unwrap_err();
    let at = e.block.clone().expect("the block of the error");
    assert_eq!(at.name, "G1/#2");
    assert_eq!(
        at.node,
        lensbench_project::NodePath(vec![
            lensbench_project::NodeStep::Block(1),
            lensbench_project::NodeStep::Block(1)
        ])
    );
    assert!(
        e.message().unwrap_or_default().contains("no-such-file.zmx"),
        "{e}"
    );
    // lensbench_core::Error に直すと名前が文の前に付く
    let core: lensbench_core::Error = e.into();
    assert!(
        core.message()
            .unwrap_or_default()
            .starts_with("block G1/#2: "),
        "{core}"
    );
    // 和訳はブロックの名前の前置きにも当たる（後ろの文が「パス: 文」でも）
    let ja = lensbench_core::i18n::translate(core.message().unwrap_or_default());
    assert!(ja.starts_with("ブロック G1/#2: "), "{ja}");
    // ブロックの外の誤り（絞りが無い）は位置を持たない
    let txt = format!(r#"{{ {HEAD} "blocks": [{{"type": "air", "t": 5}}] }}"#);
    let e = expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .unwrap_err();
    assert!(e.block.is_none(), "{e}");
}

/// 取り込んだプロジェクトの循環は道を並べて止め、循環でない深い入れ子は通す。状態・BS の経路を選べる
#[test]
fn imported_projects_find_loops_and_take_a_state() {
    let dir = std::env::temp_dir().join(format!("lb-json-loop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let put = |name: &str, rest: &str| {
        std::fs::write(dir.join(name), format!("{{ {HEAD} {rest} }}")).unwrap();
    };
    put(
        "leaf.json",
        r#""blocks": [{"type": "stop", "semi": 5}, {"type": "air", "id": "d1", "t": 5},
            {"type": "lens", "source": {"surfaces": [
              {"radius": 50, "t": 4, "glass": "N-BK7", "semi": 6}, {"radius": -50, "semi": 6}]}},
            {"type": "air", "t": 40}],
          "states": [{"name": "far", "set": {"d1": {"t": 9}}}]"#,
    );
    // 10 段の入れ子（循環ではない）
    for i in 0..10 {
        let next = if i == 9 {
            "leaf.json".to_string()
        } else {
            format!("c{}.json", i + 1)
        };
        put(
            &format!("c{i}.json"),
            &format!(r#""blocks": [{{"type": "zmx", "path": "{next}"}}]"#),
        );
    }
    let at = |name: &str| serde_json::to_string(&dir.join(name).display().to_string()).unwrap();
    let len = |s: &System| {
        s.surfaces[1..s.surfaces.len() - 1]
            .iter()
            .map(|q| q.t)
            .sum::<f64>()
    };
    let deep = project(&format!(
        r#"[{{"type": "zmx", "path": {}}}]"#,
        at("c0.json")
    ));
    let base = project(&format!(
        r#"[{{"type": "zmx", "path": {}}}]"#,
        at("leaf.json")
    ));
    assert_eq!(len(&deep), len(&base));
    // 状態を選ぶ
    let far = project(&format!(
        r#"[{{"type": "zmx", "path": {}, "state": "far"}}]"#,
        at("leaf.json")
    ));
    assert!((len(&far) - len(&base) - 4.0).abs() < 1e-12);
    // x → y → x
    put("x.json", r#""blocks": [{"type": "zmx", "path": "y.json"}]"#);
    put("y.json", r#""blocks": [{"type": "zmx", "path": "x.json"}]"#);
    let e = try_project(&format!(r#"[{{"type": "zmx", "path": {}}}]"#, at("x.json")))
        .unwrap_err()
        .to_string();
    assert!(e.contains("a loop of imports"), "{e}");
    assert!(
        e.contains("x.json → ") && e.contains("y.json → ") && e.ends_with("x.json"),
        "{e}"
    );
    // 状態は .zmx には付けられない
    let e = try_project(&format!(
        r#"[{{"type": "zmx", "path": {}, "state": "far"}}]"#,
        serde_json::to_string(
            &root()
                .join("tests/refs/A0/system.zmx")
                .display()
                .to_string()
        )
        .unwrap()
    ))
    .unwrap_err()
    .to_string();
    assert!(e.contains("for a project file"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 折り曲げの鏡を 2 軸で傾けても、鏡の後の軸は反射した光軸に乗る（軸上の主光線が鏡の後の全部の面で (0, 0) を
/// 通り、方向は局所の ∓z）
#[test]
fn two_axis_fold_mirror_keeps_axis() {
    for tilt in ["[30, 20]", "[30, 0]", "[0, 20]", "[-12, 37]"] {
        let txt = format!(
            r#"{{ "wavelengths": [{{"um": 0.55}}], "aperture": {{"type": "epd", "value": 2}},
            "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
              {{"type": "mirror", "tilt": {tilt}, "semi": 10}}, {{"type": "air", "t": 50}},
              {{"type": "lens", "source": {{"surfaces": [
                {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 6}}, {{"radius": -50, "semi": 6}}]}}}},
              {{"type": "air", "t": 20}}] }}"#
        );
        let s = expand(
            &Project::from_json(&txt).unwrap(),
            &env(),
            &ExpandOpts::default(),
        )
        .map(|x| x.system)
        .unwrap();
        let t = s
            .try_trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, true)
            .unwrap();
        let m = (1..s.surfaces.len())
            .position(|j| s.surfaces[j].is_mirror())
            .unwrap()
            + 1;
        for j in m + 2..s.surfaces.len() {
            let Some(r) = t.rec_at(j) else { continue };
            let [x, y, _] = r.xyz[0];
            let [l, mm, _] = r.lmn[0];
            assert!(x.abs().max(y.abs()) < 1e-9, "{tilt} 面 {j}: ({x}, {y})");
            assert!(
                l.abs().max(mm.abs()) < 1e-9,
                "{tilt} 面 {j}: 方向 ({l}, {mm})"
            );
        }
    }
}

/// 環境（温度・気圧）: プロジェクトの environment で屈折率を計算し、取り込んだ .zmx の環境と違えば注意を出す
#[test]
fn environment_sets_indices_and_warns() {
    let dir = std::env::temp_dir().join(format!("lb-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hot = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nENVD 4.0E+1 1.2 1\n\
GCAT SCHOTT\nFTYP 0 0 1 3 0 0 0\nXFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.4861327 1\nWAVM 2 0.5875618 1\n\
WAVM 3 0.6562725 1\nPWAV 2\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5 40 0 0 0 0 0 0\n  DIAM 10 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.02\n  DISZ 45\n  DIAM 10 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    std::fs::write(dir.join("hot.zmx"), hot).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let z = zmx::load_with(&dir.join("hot.zmx"), &opt).unwrap();
    assert_eq!(
        (z.temperature, z.pressure, z.adjust_index),
        (40.0, 1.2, true)
    );
    let env = Env {
        step_files: None,
        base: dir.clone(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    let run = |extra: &str| {
        let t = format!(
            "{{ {HEAD} {extra} \"blocks\": [{{\"type\": \"zmx\", \"path\": \"hot.zmx\"}}] }}"
        );
        lensbench_project::expand(
            &Project::from_json(&t).unwrap(),
            &env,
            &lensbench_project::ExpandOpts::default(),
        )
        .map(|x| (x.system, x.notes))
        .unwrap()
    };
    // 既定の環境（20 °C）: 注意が出て、屈折率は .zmx を単体で読んだのと違う
    let (s, warn) = run("");
    assert!(
        warn.iter().any(|w| w.text.contains("environment")),
        "{warn:?}"
    );
    assert!(s.n(1)[1] != z.n(1)[1]);
    // 同じ環境にすれば注意はなく、屈折率も同じ
    let (s, warn) =
        run(r#""environment": {"temperature_c": 40, "pressure_atm": 1.2, "adjust_index": true},"#);
    assert!(
        !warn.iter().any(|w| w.text.contains("environment")),
        "{warn:?}"
    );
    for j in 1..s.surfaces.len() {
        assert_eq!(s.n(j), z.n(j), "面 {j}");
    }
    assert_eq!((s.temperature, s.pressure), (40.0, 1.2));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 使い回し: 同じファイルは読み直さず、書き換えれば読み直す。入れ子のプロジェクトは中の葉だけ覚える
#[test]
fn source_cache_reuses_files_until_they_change() {
    let dir = std::env::temp_dir().join(format!("lb-src-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a0.json");
    let a = a0();
    lensbench_project::sysfile::save(&f, &a, &[]).unwrap();
    let txt = format!(
        "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": {}}}] }}",
        serde_json::to_string(&f.display().to_string()).unwrap()
    );
    let p = Project::from_json(&txt).unwrap();
    let cache = lensbench_project::SourceCache::default();
    let o = ExpandOpts {
        cache: Some(&cache),
        ..Default::default()
    };
    let s1 = expand(&p, &env(), &o).unwrap().system;
    assert_eq!(cache.file_count(), 1);
    let s2 = expand(&p, &env(), &o).unwrap().system;
    assert_eq!(cache.file_count(), 1);
    same_rays(&s1, &s2, 1e-15);
    same_rays(&s1, &a, 1e-12);
    // 書き換えると読み直す（大きさが変わるように面を 1 つ減らす）
    let mut fewer = a.clone();
    fewer
        .edit(|r| {
            r.surfaces.remove(2);
            Ok(())
        })
        .unwrap();
    lensbench_project::sysfile::save(&f, &fewer, &[]).unwrap();
    let s3 = expand(&p, &env(), &o).unwrap().system;
    assert_eq!(s3.surfaces.len(), fewer.surfaces.len());
    // 入れ子のプロジェクト: 中の .zmx は覚え、プロジェクトの系は覚えない
    let q = dir.join("p.json");
    std::fs::write(
        &q,
        format!(
            "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": {}}}] }}",
            serde_json::to_string(
                &root()
                    .join("tests/refs/A0/system.zmx")
                    .display()
                    .to_string()
            )
            .unwrap()
        ),
    )
    .unwrap();
    let outer = Project::from_json(&format!(
        "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": {}}}] }}",
        serde_json::to_string(&q.display().to_string()).unwrap()
    ))
    .unwrap();
    let cache = lensbench_project::SourceCache::default();
    let o = ExpandOpts {
        cache: Some(&cache),
        ..Default::default()
    };
    let n1 = expand(&outer, &env(), &o).unwrap().system;
    assert_eq!(cache.file_count(), 1);
    let n2 = expand(&outer, &env(), &o).unwrap().system;
    same_rays(&n1, &n2, 1e-15);
    // 環境が変われば捨てる
    cache.scope("other");
    assert_eq!(cache.file_count(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// 展開の結果は面ごとの出どころ（ブロック・群、往路・復路、裏返した玉）を持つ
#[test]
fn the_expansion_maps_each_surface_to_its_blocks() {
    let lens = |id: &str, rev: bool| {
        format!(
            r#"{{"type": "lens", "id": "{id}", "reversed": {rev}, "decenter": [0.1, 0], "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7"}}, {{"radius": -60}}]}}}}"#
        )
    };
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}},
            {{"type": "group", "id": "G", "blocks": [{}, {{"type": "air", "t": 2}}, {}]}},
            {{"type": "air", "t": 30}}, {{"type": "mirror", "id": "M", "back": true}}] }}"#,
        lens("A", false),
        lens("B", true)
    );
    let p = Project::from_json(&txt).unwrap();
    let x = expand(&p, &env(), &ExpandOpts::default()).unwrap();
    let s = &x.system;
    assert_eq!(x.map.len(), s.surfaces.len());
    let runs = |id: &str, back| lensbench_project::block_runs(&x.map, id, back);
    // 玉 A・B は群 G の中。往路と復路で 1 つずつ
    let (a, b, g) = (
        runs("A", Some(false)),
        runs("B", Some(false)),
        runs("G", Some(false)),
    );
    assert_eq!((a.len(), b.len(), g.len()), (1, 1, 1));
    assert_eq!((g[0][0], g[0][1]), (a[0][0], b[0][1]));
    assert_eq!(runs("A", Some(true)).len(), 1);
    assert_eq!(runs("G", None).len(), 2);
    // ブロックごとの範囲の一覧: 群が中の玉より先、往路と復路を分ける
    let bs = x.block_spans();
    let ids: Vec<&str> = bs.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["G", "A", "B", "M"]);
    assert_eq!(bs[0].forward, g);
    assert_eq!(bs[0].back, runs("G", Some(true)));
    assert_eq!(bs[1].forward, a);
    assert!(bs[3].back.is_empty() && bs[3].forward == runs("M", None));
    // 偏心の座標ブレークもブロックの面。中の面は 2 つ
    let faces = (a[0][0]..=a[0][1])
        .filter(|&j| !s.surfaces[j].is_coord_break())
        .count();
    assert_eq!(faces, 2);
    // 復路の面は back、鏡は鏡の面だけ
    let m = runs("M", None);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0][0], m[0][1]);
    assert!(s.surfaces[m[0][0]].is_mirror());
    assert!(x.map[m[0][0] + 1..s.surfaces.len() - 1]
        .iter()
        .any(|o| o.back));
    // 裏返した玉 B の往路の面だけ flip、玉・群のパラメータは flipped
    assert!(x
        .map
        .iter()
        .filter(|o| o.flip.is_some())
        .all(|o| o.is_in("B") && !o.back));
    let ua = s.units.iter().find(|u| u.name == "A").unwrap();
    assert!(!ua.flipped);
    // 偏心つきの裏返した玉 B も flipped（範囲は偏心の座標ブレークを含む）。B を含む群 G は立たない
    let u = s.units.iter().find(|u| u.name == "B").unwrap();
    assert!(u.flipped, "{:?}", u.spans);
    assert!(!s.units.iter().find(|u| u.name == "G").unwrap().flipped);
}

/// 復路の面の並び（空気中）に理想レンズを置ける。往路と復路で同じ焦点距離の理想レンズを 2 回通る
#[test]
fn a_paraxial_surface_works_on_the_return_path() {
    let pl = r#"{"type": "surfaces", "surfaces": [{"row": {"t": 10, "action": {"paraxial": {"f": 50}}}}]}"#;
    let s = project(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {pl}, {{"type": "air", "t": 30}}, {{"type": "mirror", "back": true}}]"#
    ));
    let par: Vec<usize> = (0..s.surfaces.len())
        .filter(|&j| s.surfaces[j].typ() == lensbench_core::surface::SurfType::Paraxial)
        .collect();
    assert_eq!(par.len(), 2, "往路と復路の 1 つずつ");
    let (a, b) = (&s.surfaces[par[0]], &s.surfaces[par[1]]);
    assert_eq!((a.focal(), b.focal()), (50.0, 50.0));
}

/// 鏡の後（向きが逆の所）に置いた面の行の Tilted は、tan の符号も変えて今の向きに直す
#[test]
fn a_tilted_row_after_a_mirror_flips_its_tan() {
    let row = r#"{"type": "surfaces", "surfaces": [{"row": {"t": 5, "shape": {"tilted": {"tan": [0, 0.02]}}}}]}"#;
    let tans = |blocks: &str| -> Vec<Vec<f64>> {
        let s = project(blocks);
        (0..s.surfaces.len())
            .filter(|&j| s.surfaces[j].typ() == lensbench_core::surface::SurfType::TiltSurf)
            .map(|j| s.surfaces[j].asph().to_vec())
            .collect()
    };
    let fwd = tans(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(fwd, vec![vec![0.0, 0.02]]);
    let back = tans(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {{"type": "mirror"}}, {{"type": "air", "t": 10}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(back, vec![vec![0.0, -0.02]]);
}

/// 鏡の後のトロイダル面は c と同じく cx の符号も変える（光の進む向きで書いた形を今の向きに）
#[test]
fn a_toroidal_surface_after_a_mirror_flips_its_cx() {
    let row = r#"{"type": "surfaces", "surfaces": [{"shape": "toroidal", "radius": 50, "radius_x": 80, "t": 5}]}"#;
    let curv = |blocks: &str| -> Vec<(f64, f64)> {
        let s = project(blocks);
        (0..s.surfaces.len())
            .filter(|&j| s.surfaces[j].typ() == lensbench_core::surface::SurfType::Toroidal)
            .map(|j| (s.surfaces[j].c, s.surfaces[j].cx()))
            .collect()
    };
    let fwd = curv(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(fwd, vec![(0.02, 0.0125)]);
    let back = curv(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {{"type": "mirror"}}, {{"type": "air", "t": 10}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(back, vec![(-0.02, -0.0125)]);
}

/// 鏡の後の Irregular は傾きと Zs・Za・Zc の符号を変え、偏心と項の角度はそのまま
#[test]
fn an_irregular_row_after_a_mirror_flips_only_its_tilt_and_terms() {
    let row = r#"{"type": "surfaces", "surfaces": [{"row": {"t": 5, "semi": 4, "shape": {"irregular": {"c": 0.01, "decenter": [0.1, 0.2], "tilt": [1, 2], "zs": 0.001, "za": 0.002, "zc": 0.003, "angle": 30}}}}]}"#;
    let vals = |blocks: &str| -> Vec<(f64, Vec<f64>)> {
        let s = project(blocks);
        (0..s.surfaces.len())
            .filter(|&j| s.surfaces[j].typ() == lensbench_core::surface::SurfType::Irregular)
            .map(|j| (s.surfaces[j].c, s.surfaces[j].asph().to_vec()))
            .collect()
    };
    let fwd = vals(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(
        fwd,
        vec![(0.01, vec![0.1, 0.2, 1.0, 2.0, 0.001, 0.002, 0.003, 30.0])]
    );
    let back = vals(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {{"type": "mirror"}}, {{"type": "air", "t": 10}}, {row}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(
        back,
        vec![(
            -0.01,
            vec![0.1, 0.2, -1.0, -2.0, -0.001, -0.002, -0.003, 30.0]
        )]
    );
}

/// 鏡の後の利用者定義の面: 裏返しに対応するモデル（ideal_mapping）はそのまま置け、対応しないモデルは面の番号と
/// モデルの名前つきの誤り
#[test]
fn a_user_surface_after_a_mirror_needs_a_flip() {
    use lensbench_core::plugin::{Config, ConfigReader};
    use lensbench_core::usersurf::{SurfaceInstance, SurfaceModel};
    use std::sync::Arc;
    let row = |model: &str| {
        format!(
            r#"{{"type": "surfaces", "surfaces": [{{"row": {{"t": 5, "action": {{"user": {{"model": "{model}", "config": {{"f": 40}}}}}}}}}}]}}"#
        )
    };
    let blocks = |model: &str| {
        format!(
            r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {{"type": "mirror"}}, {{"type": "air", "t": 10}}, {}, {{"type": "air", "t": 30}}]"#,
            row(model)
        )
    };
    let s = project(&blocks("ideal_mapping"));
    assert!(s
        .surfaces
        .iter()
        .any(|d| d.typ() == lensbench_core::surface::SurfType::User));

    struct NoFlip;
    struct Flat;
    impl SurfaceInstance for Flat {
        fn deflect(
            &self,
            _: [f64; 3],
            k: [f64; 3],
            _: f64,
            _: f64,
            _: f64,
        ) -> Option<([f64; 3], f64)> {
            Some((k, 0.0))
        }
    }
    impl SurfaceModel for NoFlip {
        fn name(&self) -> &str {
            "lbtest_noflip"
        }
        fn prepare(&self, config: &Config) -> lensbench_core::Result<Arc<dyn SurfaceInstance>> {
            ConfigReader::new(config).num("f", 0.0)?;
            Ok(Arc::new(Flat))
        }
    }
    let mut ext = lensbench_core::ext::Ext::default();
    ext.add_surface_model(Arc::new(NoFlip));
    lensbench_core::ext::with(Arc::new(ext), || {
        let e = try_project(&blocks("lbtest_noflip")).unwrap_err();
        let m = e.to_string();
        assert!(
            m.contains("after a mirror") && m.contains("lbtest_noflip"),
            "{m}"
        );
        // 鏡の前なら置ける
        try_project(&format!(
            r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {}, {{"type": "air", "t": 30}}]"#,
            row("lbtest_noflip")
        ))
        .unwrap();
    });
}

/// Gradium の玉の段（分布の軸の偏心・傾き dec、reversed なら反転）
fn grin_lens(dec: [f64; 4], reversed: bool) -> String {
    use lensbench_core::grin::{GradiumProfile, GrinKind, Sgrin};
    let kind = GrinKind::Gradium {
        prof: GradiumProfile {
            name: "LBTG1".into(),
            family: "LBTGSF".into(),
            zmax: 6.0,
            coef: vec![
                1.75, -0.12, 0.03, -0.01, 0.004, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
            file: "LBTEST.GRD".into(),
            flipped: false,
        },
        disp: Sgrin {
            name: "LBTGSF".into(),
            lmin: 0.4,
            lmax: 1.0,
            lref: 0.55,
            k: [
                vec![0.0331, 0.0, -0.0331],
                vec![0.5, -0.2, 0.0],
                vec![0.0, 0.0, 0.0],
            ],
            l: [vec![0.0095, 0.0003], vec![100.0, 0.0], vec![1.0, 0.0]],
        },
        dz: 1.0,
        dec,
        cap: 1,
    };
    let prof = serde_json::to_string(&kind).unwrap();
    format!(
        r#"{{"type": "lens", "reversed": {reversed}, "source": {{"surfaces": [{{"row": {{"t": 4, "shape": {{"conic": {{"c": 0.02}}}}, "medium": {{"grin": {{"name": "LBTG1", "profile": {prof}, "dt": 0.5}}}}}}}}, {{"row": {{"shape": {{"conic": {{"c": -0.01}}}}}}}}]}}}}"#
    )
}

/// GRIN の玉（Gradium、分布の軸を傾け偏心）はミラーの後に置いても、展開した光学系のように働く
#[test]
fn a_grin_lens_after_a_fold_mirror_works_like_the_unfolded_one() {
    let lens = grin_lens([0.1, -0.05, 0.01, 0.02], false);
    let unf = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 30}}, {lens}, {{"type": "air", "t": 45}}]"#
    ));
    let fold = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}}, {{"type": "mirror"}}, {{"type": "air", "t": 20}}, {lens}, {{"type": "air", "t": 45}}]"#
    ));
    assert!(fold.surfaces.iter().any(|d| d.grin().is_some()));
    assert!(
        (unf.paraxial().efl - fold.paraxial().efl).abs() < 1e-9,
        "{} {}",
        unf.paraxial().efl,
        fold.paraxial().efl
    );
    same_rays_xy(&unf, &fold);
}

/// 行で書いた鏡（radius・conic で書けない形）: 鏡より前なら行のまま、鏡の後（向きが逆の所）なら裏返して置き、
/// 鏡の後で向きが戻る。欄と行を一緒に書く・鏡でない行・厚さのある行は誤り
#[test]
fn a_mirror_written_as_row_flips_after_a_mirror() {
    let m = r#"{"type": "mirror", "row": {"semi": 6, "action": "reflect", "shape": {"irregular": {"c": 0.01, "tilt": [1, 2], "zs": 0.001}}}}"#;
    let mirrors = |blocks: &str| -> Vec<(f64, f64, f64)> {
        let s = project(blocks);
        s.surfaces
            .iter()
            .filter(|f| f.typ() == lensbench_core::surface::SurfType::Irregular)
            .inspect(|f| assert!(f.is_mirror() && f.semi == 6.0))
            .map(|f| (f.c, f.asph()[2], f.t))
            .collect()
    };
    let fwd = mirrors(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {m}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(fwd, vec![(0.01, 1.0, -30.0)]);
    let back = mirrors(&format!(
        r#"[{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {{"type": "mirror"}}, {{"type": "air", "t": 10}}, {m}, {{"type": "air", "t": 30}}]"#
    ));
    assert_eq!(back, vec![(-0.01, -1.0, 30.0)]);
    for bad in [
        r#"{"type": "mirror", "radius": 50, "row": {"action": "reflect"}}"#,
        r#"{"type": "mirror", "semi": 5, "row": {"action": "reflect"}}"#,
        r#"{"type": "mirror", "row": {"semi": 5}}"#,
        r#"{"type": "mirror", "row": {"t": 5, "action": "reflect"}}"#,
    ] {
        let txt = format!(
            r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 5}}, {{"type": "air", "t": 20}}, {bad}, {{"type": "air", "t": 30}}] }}"#
        );
        let p = Project::from_json(&txt).unwrap();
        assert!(expand(&p, &env(), &ExpandOpts::default()).is_err(), "{bad}");
    }
}

/// 反転した GRIN の玉: 焦点距離は変わらず、平面鏡で往復する光学系は、展開して 2 つ目を反転した玉にした光学系と
/// 同じに働く（分布の軸の偏心・傾きは y だけにして、鏡映と 180° 回転を同じにする）
#[test]
fn a_reversed_grin_lens_works_like_the_turned_one() {
    let sys = |lens: &str| {
        project(&format!(
            r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 30}}, {lens}, {{"type": "air", "t": 45}}]"#
        ))
    };
    for dec in [[0.0; 4], [0.1, -0.05, 0.01, 0.02]] {
        let (f, r) = (sys(&grin_lens(dec, false)), sys(&grin_lens(dec, true)));
        let (ef, er) = (f.paraxial().efl, r.paraxial().efl);
        assert!((ef - er).abs() < 1e-9 * ef.abs(), "{dec:?} {ef} {er}");
    }
    let dec = [0.0, -0.05, 0.0, 0.02];
    let fold = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {}, {{"type": "air", "t": 30}}, {{"type": "mirror", "back": true}}]"#,
        grin_lens(dec, false)
    ));
    let unf = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {}, {{"type": "air", "t": 60}}, {}, {{"type": "air", "t": 20}}]"#,
        grin_lens(dec, false),
        grin_lens(dec, true)
    ));
    assert_eq!(
        fold.surfaces.iter().filter(|d| d.grin().is_some()).count(),
        2
    );
    // 分布の軸を傾けた媒質は一般の積分（1 歩の許容 1e-8）で追うので、向きの違う 2 つの経路の差はその程度
    same_rays_xy_within(&unf, &fold, 1e-8);
}

/// 傾けた Gradient 9 の平面と標準面の玉: 反転すると傾けた平面は Tilted になって媒質の次に来る。平面鏡で往復する
/// 光学系は、展開して 2 つ目を反転した玉にした光学系と同じに働く（傾きは y だけにして鏡映と 180° 回転を同じにする）
#[test]
fn a_reversed_gradient9_lens_with_a_tilted_plane_traces() {
    use lensbench_core::grin::{Grad9Mat, GrinKind};
    let kind = GrinKind::Grad9 {
        mat: Grad9Mat {
            b: 1.6,
            c: 0.01,
            k0: 0.05,
            k1: 0.004,
            k2: 0.0002,
        },
        tan: [0.0, -0.02],
    };
    let prof = serde_json::to_string(&kind).unwrap();
    let lens = |reversed: bool| {
        format!(
            r#"{{"type": "lens", "reversed": {reversed}, "source": {{"surfaces": [{{"row": {{"t": 5, "medium": {{"grin": {{"name": "LBG9", "profile": {prof}, "dt": 0.5}}}}}}}}, {{"row": {{"shape": {{"conic": {{"c": -0.02}}}}}}}}]}}}}"#
        )
    };
    let fold = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {}, {{"type": "air", "t": 30}}, {{"type": "mirror", "back": true}}]"#,
        lens(false)
    ));
    let unf = project(&format!(
        r#"[{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {}, {{"type": "air", "t": 60}}, {}, {{"type": "air", "t": 20}}]"#,
        lens(false),
        lens(true)
    ));
    for s in [&fold, &unf] {
        assert!(s
            .surfaces
            .iter()
            .any(|d| d.typ() == lensbench_core::surface::SurfType::TiltSurf));
    }
    same_rays_xy_within(&unf, &fold, 1e-8);
}

/// 玉の中の鏡（裏面鏡）: 鏡の後の面は鏡の後の向きで置き、厚さは負になる。最初・最後の面の鏡と、面の並びの鏡は誤り
#[test]
fn a_mirror_inside_a_lens_folds_the_rest_of_it() {
    let lens = |ss: &str| {
        format!(
            r#"[{{"type": "stop", "semi": 4}}, {{"type": "air", "t": 10}},
            {{"type": "lens", "source": {{"surfaces": [{ss}]}}}}, {{"type": "air", "t": 30}}]"#
        )
    };
    let mangin = r#"{"radius": 0, "t": 3, "glass": "N-BK7", "semi": 5},
        {"row": {"shape": {"conic": {"c": -0.01}}, "action": "reflect", "semi": 5.0, "t": 3}},
        {"radius": 0, "semi": 5}"#;
    let s = project(&lens(mangin));
    let t: Vec<f64> = s.surfaces.iter().map(|x| x.t).collect();
    assert_eq!(&t[2..5], &[3.0, -3.0, -30.0], "{t:?}");
    assert!(s.surfaces[3].is_mirror());
    assert_eq!(s.surfaces[3].c, -0.01);
    let txt = |b: String| format!("{{ {HEAD} \"blocks\": {b} }}");
    let first = r#"{"row": {"action": "reflect", "t": 3}}, {"radius": 0, "semi": 5}"#;
    let p = Project::from_json(&txt(lens(first))).unwrap();
    let e = expand(&p, &env(), &ExpandOpts::default()).err().unwrap();
    assert!(e.to_string().contains("can't be mirrors"), "{e}");
    let list = r#"[{"type": "surfaces", "surfaces": [{"row": {"action": "reflect"}}]}, {"type": "air", "t": 5}]"#;
    let p = Project::from_json(&txt(list.into())).unwrap();
    let e = expand(&p, &env(), &ExpandOpts::default()).err().unwrap();
    assert!(
        e.to_string().contains("surface list can't have a mirror"),
        "{e}"
    );
}

/// 空気の間隔の medium: 前の面の後をその媒質にする（液浸）。最後の間隔なら像空間。鏡の後・ガラスの後は誤り
#[test]
fn a_gap_can_be_filled_with_a_medium() {
    let s = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 10},
        {"type": "lens", "source": {"surfaces": [{"radius": 20, "t": 3, "glass": "N-BK7", "semi": 5}, {"radius": 0, "semi": 5}]}},
        {"type": "air", "t": 2, "medium": "1.515"}]"#,
    );
    let k = s.image() - 1;
    assert_eq!(
        s.surfaces[k].mat,
        lensbench_core::material::Material::Index(1.515)
    );
    assert_eq!(s.surfaces[k].t, 2.0);
    let txt = format!(
        "{{ {HEAD} \"blocks\": [{{\"type\": \"stop\"}}, {{\"type\": \"air\", \"t\": 10}}, {{\"type\": \"mirror\"}}, {{\"type\": \"air\", \"t\": 5, \"medium\": \"N-BK7\"}}] }}"
    );
    let e = expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .err()
    .unwrap();
    assert!(e.to_string().contains("is not air"), "{e}");
}

/// プロジェクトのマクロ（名前と文）を、ブロックの surface_ops の macro_curv のソルブが番号で指して解く。
/// マクロのファイルは読まない
#[test]
fn a_macro_solve_uses_the_project_macros() {
    let txt = format!(
        r#"{{ {HEAD} "macros": [{{"name": "MIRR", "source": "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n"}}],
        "blocks": [{{"type": "stop", "semi": 4}}, {{"type": "air", "t": 10}},
        {{"type": "lens", "id": "L", "source": {{"surfaces": [{{"radius": 50, "t": 3, "glass": "N-BK7", "semi": 5}}, {{"radius": 0, "semi": 5}}]}},
          "surface_ops": [{{"op": "add_solve", "surface": 1, "kind": "macro_curv", "param": 0}}]}},
        {{"type": "air", "t": 20}}] }}"#
    );
    let e = expand(
        &Project::from_json(&txt).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .unwrap();
    let s = e.system;
    // レンズの後の面の曲率 = −前の面の曲率
    let j = s.image() - 1;
    assert!(
        (s.surfaces[j].c + 0.02).abs() < 1e-15,
        "{}",
        s.surfaces[j].c
    );
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    // 円錐係数のマクロのソルブ（add_macro_solve）
    let k = txt.replace(
        r#""kind": "macro_curv", "param": 0}]"#,
        r#""kind": "macro_curv", "param": 0}, {"op": "add_macro_solve", "surface": 0, "macro": 1, "what": "k"}]"#,
    ).replace(
        r#""source": "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n"}]"#,
        r#""source": "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n"}, {"name": "K", "source": "SOLVERETURN -0.7\n"}]"#,
    );
    assert!(k.contains("add_macro_solve") && k.contains(r#""K""#), "{k}");
    let s = expand(
        &Project::from_json(&k).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .unwrap()
    .system;
    assert_eq!(s.surfaces[s.image() - 2].k, -0.7);
    assert_eq!(s.macro_parms.len(), 1);
    // 名前でも指せる（大文字・小文字は区別しない）。並べ替えても同じマクロを指す
    let named = k
        .replace(r#""param": 0}"#, r#""macro": "mirr"}"#)
        .replace(r#""macro": 1,"#, r#""macro": "K","#);
    let named = named.replace(
        r#"[{"name": "MIRR", "source": "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n"}, {"name": "K", "source": "SOLVERETURN -0.7\n"}]"#,
        r#"[{"name": "K", "source": "SOLVERETURN -0.7\n"}, {"name": "MIRR", "source": "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n"}]"#,
    );
    assert!(
        named.contains(r#""macro": "mirr""#) && named.contains(r#"[{"name": "K""#),
        "{named}"
    );
    let s = expand(
        &Project::from_json(&named).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .unwrap()
    .system;
    assert_eq!(s.surfaces[s.image() - 2].k, -0.7);
    assert!((s.surfaces[s.image() - 1].c + 0.02).abs() < 1e-15);
    // 同じ名前のマクロは 2 つ置けない
    let dup = named.replace(r#"{"name": "K","#, r#"{"name": "mirr","#);
    let err = expand(
        &Project::from_json(&dup).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("used twice"), "{err}");
    // 無い名前は誤り
    let bad = named.replace(r#""macro": "mirr""#, r#""macro": "NONE""#);
    let err = expand(
        &Project::from_json(&bad).unwrap(),
        &env(),
        &ExpandOpts::default(),
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("no macro named NONE"), "{err}");
    // 無い番号のマクロは誤り
    let bad = txt.replace(r#""param": 0"#, r#""param": 1"#);
    assert!(expand(
        &Project::from_json(&bad).unwrap(),
        &env(),
        &ExpandOpts::default()
    )
    .is_err());
}
