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
            "beams": [{"name": "refb", "path": "RT", "state": "near"},
                      {"name": "near_only", "state": "near"}]}"#,
    );
    let r = p.with_state("refb").unwrap();
    assert_eq!(
        (r.object_distance, r.path.as_deref()),
        (Some(40.0), Some("RT"))
    );
    let n = p.with_state("near_only").unwrap();
    assert_eq!(n.path.as_deref(), Some("TR"));
    identical(
        &sys(&p, Some("refb"), None),
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
            r#"{"beams": [{"name": "ref"}]}"#,
            "beam ref: 0, ref and reference mean the reference sphere in interference",
        ),
        (
            r#"{"states": [{"name": "a"}], "beams": [{"name": "a"}]}"#,
            "beam a: a state has the same name",
        ),
        (
            r#"{"beams": [{"name": "a", "state": "x"}]}"#,
            "beam a: no state x",
        ),
        (r#"{"beams": [{"name": "a", "path": "TX"}]}"#, "beam a:"),
        (
            r#"{"beams": [{"name": "a", "turn": {"block": "L", "surface": 0, "back": true}}]}"#,
            "beam a: turn takes a lens surface as numbered on the way out",
        ),
    ] {
        let e = with(extra);
        assert!(e.contains(want), "{extra}: {e}");
    }
    let p = case("mich_RT", r#"{"beams": [{"name": "refb", "path": "RT"}]}"#);
    let e = p.with_state("x").unwrap_err().to_string();
    assert!(e.contains("no state or beam x (beams: refb)"), "{e}");
    // 状態で光束の組は変えられない
    let p = case(
        "mich_RT",
        r#"{"states": [{"name": "s", "project": {"beams": []}}], "beams": [{"name": "refb"}]}"#,
    );
    let e = p.with_state("s").unwrap_err().to_string();
    assert!(e.contains("beams can't be changed"), "{e}");
    // 経路を持つ光束に別の経路は渡せない
    let p = case("mich_RT", r#"{"beams": [{"name": "refb", "path": "RT"}]}"#);
    let e = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("refb"),
            path: Some("TR"),
            ..Default::default()
        },
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(e.contains("beam refb has its own path"), "{e}");
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

/// 玉 L（面 0 は R 40、面 1 は R −60、N-BK7）を通り、光束 g の turn で折り返す系。other は L の後のブロック
fn turned(turn: &str, other: &str) -> Result<System, String> {
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L", "source": {{"surfaces": [
             {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
          {{"type": "air", "t": 20}} {other}],
          "beams": [{{"name": "g", "turn": {turn}}}]}}"#
    );
    let p = Project::from_json(&txt).map_err(|e| e.to_string())?;
    expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("g"),
            ..Default::default()
        },
    )
    .map(|x| x.system)
    .map_err(|e| e.to_string())
}

/// 垂直入射の境界の反射率
fn fresnel(n: f64) -> f64 {
    ((n - 1.0) / (n + 1.0)).powi(2)
}

/// 並べた turn がたどれなければ何番目の折り返しかを言う（面 1 で戻った後の復路に面 1 は無い・面 0 で進んだ後の往路に面 0 は無い）
#[test]
fn unreached_turns_are_named() {
    let e = turned(
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 1}]"#,
        "",
    )
    .unwrap_err();
    assert!(
        e.contains("turn 2: block L surface 1 (pass 0) is not on the way back"),
        "{e}"
    );
    let e = turned(
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 0}, {"block": "L", "surface": 0}]"#,
        "",
    )
    .unwrap_err();
    assert!(e.contains("turn 3:") && e.contains("on the way out"), "{e}");
    // 玉の中の往路の続きの折り返しの面が玉の外（面 2 枚の玉の面 2）: 1 つ目と同じ誤り（落ちない）
    let e = turned(
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 0}, {"block": "L", "surface": 2}]"#,
        "",
    )
    .unwrap_err();
    assert!(
        e.contains("turn: surface 2 (2 surfaces in the block)"),
        "{e}"
    );
    // 2 回で往路に戻れば元の像面まで進む
    let s = turned(
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 0}]"#,
        "",
    )
    .unwrap();
    assert_eq!(s.surfaces.iter().filter(|d| d.is_mirror()).count(), 2);
}

/// turn で玉の面 0 で折り返すと、同じ形の鏡で戻る系と光線の道は同じで、強さは境界の反射率
#[test]
fn turn_reflects_at_a_lens_face() {
    use lensbench_analysis::defs::mean_transmission;
    let s = turned(r#"{"block": "L", "surface": 0}"#, "").unwrap();
    let m = Project::from_json(&format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
          {{"type": "mirror", "radius": 40, "back": true, "semi": 8}}]}}"#
    ))
    .unwrap();
    let m = expand(&m, &env(), &ExpandOpts::default()).unwrap().system;
    assert_eq!(s.surfaces.len(), m.surfaces.len());
    let j = s.surfaces.iter().position(|d| d.is_mirror()).unwrap();
    let r = s.surfaces[j].reflect.as_ref().unwrap();
    assert_eq!(r.mat, "N-BK7");
    let nbk7 = r.n[0];
    assert!((nbk7 - 1.5168).abs() < 1e-3, "{nbk7}");
    let ta = s
        .try_trace(FieldSel::Index(0), &PX, &PY, Some(0), None, false)
        .unwrap();
    let tb = m
        .try_trace(FieldSel::Index(0), &PX, &PY, Some(0), None, false)
        .unwrap();
    for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
        assert_eq!(ra.status, rb.status);
        for i in 0..3 {
            assert!((ra.p[i] - rb.p[i]).abs() < 1e-12 && (ra.k[i] - rb.k[i]).abs() < 1e-12);
        }
        assert!((ra.opl - rb.opl).abs() < 1e-9);
    }
    let t = mean_transmission(&s, FieldSel::Index(0), 0).unwrap();
    assert!((t - fresnel(nbk7)).abs() < 2e-4, "{t}");

    // 面 1（後ろは空気）で折り返すと、玉の中を戻って面 0 を出る: 入る・出るの透過と面 1 の反射
    let s = turned(r#"{"block": "L", "surface": 1}"#, "").unwrap();
    let j = s.surfaces.iter().position(|d| d.is_mirror()).unwrap();
    assert_eq!(s.surfaces[j].reflect.as_ref().unwrap().r#mat, "AIR");
    assert_eq!(s.surfaces[j].reflect.as_ref().unwrap().n, vec![1.0]);
    let t = mean_transmission(&s, FieldSel::Index(0), 0).unwrap();
    let r = fresnel(nbk7);
    assert!((t - r * (1.0 - r).powi(2)).abs() < 3e-4, "{t}");
    // 戻りで面 0 を出た後は空気、像は物体の側
    let last = &s.surfaces[s.surfaces.len() - 2];
    assert!(last.mat.glass().is_none(), "{:?}", last.mat);
}

/// turn の誤り: 面の番号の外、経路に無いブロック、群の中、OFF
#[test]
fn turn_errors() {
    for (turn, want) in [
        (
            r#"{"block": "L", "surface": 2}"#,
            "turn: surface 2 (2 surfaces in the block)",
        ),
        (
            r#"{"block": "X", "surface": 0}"#,
            "turn: block X (pass 0) is not on the path",
        ),
        (
            r#"{"block": "L", "surface": 0, "pass": 1}"#,
            "turn: block L (pass 1) is not on the path",
        ),
    ] {
        let e = turned(turn, "").map(|_| ()).unwrap_err();
        assert!(e.contains(want), "{turn}: {e}");
    }
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "group", "blocks": [{{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L", "source": {{"surfaces": [{{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}},
          {{"radius": -60, "semi": 8}}]}}}}]}}, {{"type": "air", "t": 20}}],
          "beams": [{{"name": "g", "turn": {{"block": "L", "surface": 0}}}}]}}"#
    );
    let p = Project::from_json(&txt).unwrap();
    let e = expand(
        &p,
        &env(),
        &ExpandOpts {
            state: Some("g"),
            ..Default::default()
        },
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(e.contains("turn: a lens in a group"), "{e}");
}

/// BS の腕の玉で turn すると、戻りは BS で経路の文字の口へ出る。同じ所の平面鏡で戻る系と光線の道が同じ
#[test]
fn turn_returns_through_a_bs() {
    let txt = std::fs::read_to_string(root().join("tests/splitter/mich_RT.json")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["blocks"][2]["t"] = serde_json::json!([
        {"type": "air", "t": 10},
        {"type": "lens", "id": "W", "source": {"surfaces": [
            {"radius": 0, "t": 2, "glass": "N-BK7", "semi": 8}, {"radius": 0, "semi": 8}]}},
        {"type": "air", "t": 18},
        {"type": "mirror", "back": true, "semi": 12}
    ]);
    v["beams"] =
        serde_json::json!([{"name": "g", "path": "TT", "turn": {"block": "W", "surface": 0}}]);
    let p = Project::from_json(&v.to_string()).unwrap();
    let s = sys(&p, Some("g"), None);
    v["blocks"][2]["t"] = serde_json::json!([
        {"type": "air", "t": 10},
        {"type": "mirror", "back": true, "semi": 12}
    ]);
    v["beams"] = serde_json::json!([]);
    let m = Project::from_json(&v.to_string()).unwrap();
    let m = sys(&m, None, Some("TT"));
    assert_eq!(s.surfaces.len(), m.surfaces.len());
    let ta = s
        .try_trace(FieldSel::Index(0), &PX, &PY, Some(0), None, false)
        .unwrap();
    let tb = m
        .try_trace(FieldSel::Index(0), &PX, &PY, Some(0), None, false)
        .unwrap();
    for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
        assert_eq!(ra.status, rb.status);
        for i in 0..3 {
            assert!((ra.p[i] - rb.p[i]).abs() < 1e-12 && (ra.k[i] - rb.k[i]).abs() < 1e-12);
        }
        assert!((ra.opl - rb.opl).abs() < 1e-9);
    }
}

/// 面の並びの Jones 面と z 軸まわりの回転の座標ブレークは往路を戻れる。復路は逆の順で、回転は逆、Jones の行列は
/// 転置（相反性）。旋光子（回転の行列）は往復で打ち消し、像面の電場は旋光子なしと同じ
#[test]
fn jones_and_z_rotations_return() {
    use lensbench_core::coating::{Coatings, C64};
    use lensbench_core::surface::SurfType;
    use lensbench_core::system::PolIn;
    let (c, s) = (0.3f64.cos(), 0.3f64.sin());
    let run = |plate: &str| {
        let txt = format!(
            r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
              {{"type": "surfaces", "surfaces": [{plate}]}}, {{"type": "air", "t": 20}},
              {{"type": "mirror", "back": true, "semi": 10, "coat": "I.s=0,p=0"}}]}}"#
        );
        let p = Project::from_json(&txt).unwrap();
        expand(&p, &env(), &ExpandOpts::default()).unwrap().system
    };
    let plain = run(r#"{"t": 0}"#);
    let rot = run(&format!(
        r#"{{"coordbrk": [0, 0, 0, 0, 25]}}, {{"jones": [{c}, 0, {}, 0, {s}, 0, {c}, 0]}},
           {{"coordbrk": [0, 0, 0, 0, -25]}}"#,
        -s
    ));
    // 復路の面: 回転 25°、転置した行列、回転 −25°
    let kinds: Vec<usize> = (0..rot.surfaces.len())
        .filter(|&j| matches!(rot.surfaces[j].typ(), SurfType::Jones | SurfType::CoordBrk))
        .collect();
    assert_eq!(kinds.len(), 6, "{kinds:?}");
    let back = &kinds[3..];
    assert_eq!(rot.surfaces[back[0]].cb()[4], 25.0);
    assert_eq!(
        rot.surfaces[back[1]].jones(),
        [c, 0.0, s, 0.0, -s, 0.0, c, 0.0]
    );
    assert_eq!(rot.surfaces[back[2]].cb()[4], -25.0);
    let e = |sys: &System| {
        let co = Coatings::for_system(sys).unwrap();
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        sys.try_trace_pol(
            FieldSel::Index(0),
            &[0.0, 0.5],
            &[0.0, 0.3],
            None,
            PolIn::new(&[x], &co).no_phase(),
        )
        .unwrap()
        .e
    };
    let (a, b) = (e(&plain), e(&rot));
    for i in 0..2 {
        for k in 0..3 {
            assert!(
                (a[i][0][k] - b[i][0][k]).norm() < 1e-12,
                "{:?} {:?}",
                a[i][0],
                b[i][0]
            );
        }
    }
}

/// 媒質で満たす間隔と、z 軸まわりの回転・Jones 面を含む玉は往路を戻れる。ミラーで折り返した系は、同じ媒質を
/// 逆の順に並べた展開した系と、像面の x・y・方向・光路長が同じ
#[test]
fn media_return() {
    let sys = |blocks: &str| {
        let txt = format!(
            r#"{{ {HEAD} "fields": {{"type": "angle", "points": [[0, 0], [0, 2]]}}, "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}}, {blocks}]}}"#
        );
        let p = Project::from_json(&txt).unwrap();
        expand(&p, &env(), &ExpandOpts::default()).unwrap().system
    };
    let folded = sys(
        r#"{"type": "surfaces", "surfaces": [{"t": 0}]}, {"type": "air", "t": 2, "medium": "F2"},
           {"type": "lens", "id": "L", "source": {"surfaces": [{"t": 0, "glass": "N-BK7", "semi": 8},
             {"coordbrk": [0, 0, 0, 0, 25]}, {"jones": [1, 0, 0.2, 0, 0, 0, 1, 0], "glass": "N-BK7", "t": 3},
             {"coordbrk": [0, 0, 0, 0, -25]}, {"semi": 8}]}},
           {"type": "air", "t": 20}, {"type": "mirror", "back": true, "semi": 10}"#,
    );
    let open = sys(
        r#"{"type": "lens", "id": "A", "source": {"surfaces": [{"t": 2, "glass": "F2", "semi": 8},
             {"t": 3, "glass": "N-BK7", "semi": 8}, {"semi": 8}]}}, {"type": "air", "t": 40},
           {"type": "lens", "id": "B", "source": {"surfaces": [{"t": 3, "glass": "N-BK7", "semi": 8},
             {"t": 2, "glass": "F2", "semi": 8}, {"semi": 8}]}}, {"type": "air", "t": 10}"#,
    );
    let jones: Vec<[f64; 8]> = folded
        .surfaces
        .iter()
        .filter(|s| s.typ() == lensbench_core::surface::SurfType::Jones)
        .map(|s| s.jones())
        .collect();
    assert_eq!(
        jones,
        [
            [1.0, 0.0, 0.2, 0.0, 0.0, 0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0, 0.0, 0.2, 0.0, 1.0, 0.0]
        ]
    );
    for f in 0..2 {
        let ta = folded
            .try_trace(FieldSel::Index(f), &PX, &PY, None, None, false)
            .unwrap();
        let tb = open
            .try_trace(FieldSel::Index(f), &PX, &PY, None, None, false)
            .unwrap();
        for (a, b) in ta.rays.iter().zip(&tb.rays) {
            assert_eq!(a.status, b.status);
            for k in 0..2 {
                assert!((a.p[k] - b.p[k]).abs() < 1e-9, "{:?} {:?}", a.p, b.p);
                assert!((a.k[k] - b.k[k]).abs() < 1e-12, "{:?} {:?}", a.k, b.k);
            }
            assert!((a.opl - b.opl).abs() < 1e-9, "{} {}", a.opl, b.opl);
        }
    }
}
