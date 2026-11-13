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
            s.surfaces[j].typ == lensbench_core::surface::SurfType::Standard
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
) -> Result<System, String> {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    let p = Project::from_json(&txt).unwrap();
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let e = Env {
        base: root().join("examples/lenses/LB"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: Some(&look),
    };
    expand(&p, &e)
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
    assert!(e.contains("NONE"), "{e}");
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
    let m0 = s0.surfaces.iter().position(|d| d.mirror).unwrap();
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
    let m1 = s1.surfaces.iter().position(|d| d.mirror).unwrap();
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
            let s = lensbench_project::load_state(
                &p,
                &[root().join("tests/glass")],
                Some(&look),
                st.as_deref(),
            )
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

fn try_project(blocks: &str) -> Result<System, String> {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    expand(&Project::from_json(&txt).unwrap(), &env())
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
    assert_eq!(j.surfaces[k].typ, SurfType::Jones);
    assert_eq!(
        j.surfaces[k].jones,
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
    assert_eq!(c.surfaces[k].typ, SurfType::CoordBrk);
    assert_eq!(
        (c.surfaces[k].cb, c.surfaces[k].cb_order),
        ([0.1, 0.0, 0.0, 0.0, 0.0], 1)
    );
    let (px, py) = ([0.0, 0.4], [0.3, -0.6]);
    let t0 = base.trace(FieldSel::Index(3), &px, &py, Some(1), None, false);
    let t1 = c.trace(FieldSel::Index(3), &px, &py, Some(1), None, false);
    for (a, b) in t0.rays.iter().zip(&t1.rays) {
        assert!((a.xyz[0] - 0.1 - b.xyz[0]).abs() < 1e-12);
        assert!((a.xyz[1] - b.xyz[1]).abs() < 1e-12);
    }
    // 小さな円の開口で光線がけられる
    let v = with(r#"{"aperture": {"type": "circular", "max": 0.001}}"#);
    let t = v.trace(FieldSel::Index(1), &px, &py, Some(1), None, false);
    assert!(t.rays.iter().all(|r| r.vig == k));
    // 書けない組み合わせ
    for (s, msg) in [
        (
            r#"{"jones": [1, 0, 0, 0, 0, 0, 1, 0], "coordbrk": [0, 0, 0, 0, 0]}"#,
            "2 つ以上",
        ),
        (
            r#"{"jones": [1, 0, 0, 0, 0, 0, 1, 0], "radius": 20}"#,
            "曲率",
        ),
        (
            r#"{"coordbrk": [0, 0, 1, 0, 0], "glass": "N-BK7"}"#,
            "ガラス",
        ),
        (
            r#"{"coordbrk": [0, 0, 1, 0, 0], "aperture": {"type": "floating"}}"#,
            "開口",
        ),
        (r#"{"coordbrk": [0, 0, 1, 0, 0], "order": 2}"#, "順序"),
    ] {
        let e = try_project(&format!(
            r#"[{zmx}, {{"type": "surfaces", "surfaces": [{s}]}}]"#
        ))
        .unwrap_err();
        assert!(e.contains(msg), "{s}: {e}");
    }
}

/// zmx ブロックの厚さのソルブ（A0 の面 7、像面の焦点）は、次の面まで取り込めば残り、set_param で解き直す
#[test]
fn thickness_solves_follow_set_param() {
    let txt =
        format!("{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": \"A0/system.zmx\"}}] }}");
    let (s, warn) =
        lensbench_project::expand_report(&Project::from_json(&txt).unwrap(), &env()).unwrap();
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
    let (s, warn) =
        lensbench_project::expand_report(&Project::from_json(&txt).unwrap(), &env()).unwrap();
    assert!(s.solves.is_empty());
    assert!(warn.iter().any(|w| w.contains("ソルブ")), "{warn:?}");
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
    let m = s.surfaces.iter().find(|q| q.mirror).unwrap();
    assert!(
        matches!(m.ap, lensbench_core::surface::Aperture::Rect { hx, hy, .. } if hx == 5.0 && hy == 10.0),
        "{:?}",
        m.ap
    );
}

/// zmx ブロックは lensbench の系（系ファイル・プロジェクトの .json）も取り込む（#132）
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
    assert!(e.contains("構成の番号"), "{e}");
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
        let s = expand(&Project::from_json(&txt).unwrap(), &env()).unwrap();
        let t = s.trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, true);
        let m = (1..s.surfaces.len())
            .position(|j| s.surfaces[j].mirror)
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
