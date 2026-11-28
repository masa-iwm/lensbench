//! 合流（`join`）: マッハツェンダーの 2 本の腕の光束が 2 つめの BS で重なる、鏡を傾けるとその腕の光束だけが動く

use lensbench_core::math::{dot, norm, sub, V3};
use lensbench_core::system::FieldSel;
use lensbench_core::System;
use lensbench_project::scene::{expansions, Scene};
use lensbench_project::{expand, Env, ExpandOpts, Project};
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

/// 理想の BS 2 つと 45° の鏡 2 枚のマッハツェンダー。BS2 は BS1 の透過の腕に置き、反射の腕は鏡 M2 の後で BS2 の口 D へ合流する。
/// m2 は鏡 M2 の傾き [deg]（−45 で BS2 の口 D へ向かう）、form は BS の形
fn mz(m2: f64, form: &str) -> Project {
    mz_at(45.0, m2, form, "")
}

/// m1 は鏡 M1 の傾き [deg]、at は BS2 の置き方（空なら透過の腕の並びで置く）
fn mz_at(m1: f64, m2: f64, form: &str, at: &str) -> Project {
    let bs = |id: &str, branches: &str| {
        format!(
            r#"{{"type": "splitter", "id": "{id}", "form": "{form}", "size": 10, "thickness": 3, "glass": "N-BK7", "semi": 12 {branches}}}"#
        )
    };
    let bs2 = bs(
        "BS2",
        &format!(r#", "t": [{{"type": "air", "t": 30}}], "r": [{{"type": "air", "t": 30}}] {at}"#),
    );
    let arms = format!(
        r#", "t": [{{"type": "air", "t": 50}}, {{"type": "mirror", "id": "M1", "tilt": [{m1}, 0], "semi": 15}},
                  {{"type": "air", "t": 50}}, {bs2}],
            "r": [{{"type": "air", "t": 50}}, {{"type": "mirror", "id": "M2", "tilt": [{m2}, 0], "semi": 15}},
                  {{"type": "air", "t": 50}}, {{"type": "join", "splitter": "BS2", "port": "d"}}]"#
    );
    let txt = format!(
        r#"{{"wavelengths": [{{"um": 0.6328}}], "glass_catalogs": ["SCHOTT"],
          "aperture": {{"type": "epd", "value": 4}},
          "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {}]}}"#,
        bs("BS1", &arms)
    );
    Project::from_json(&txt).unwrap()
}

fn sys(p: &Project, path: &str) -> System {
    expand(
        p,
        &env(),
        &ExpandOpts {
            path: Some(path),
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{path}: {e}"))
    .system
}

/// 主光線の像面の点と向き（大域）
fn chief(s: &System) -> (V3, V3) {
    let t = s
        .try_trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, false)
        .unwrap();
    assert!(t.traced(0), "{:?}", t.rays[0].status);
    (t.rays[0].p, t.rays[0].k)
}

fn close(a: V3, b: V3, tol: f64) -> bool {
    norm(sub(a, b)) < tol
}

/// 4 本の経路がとれ、TT と RR、TR と RT が同じ口から同じ所・同じ向きに出る（2 光束が BS2 で重なる）
#[test]
fn mach_zehnder_beams_overlap() {
    for form in ["ideal", "cube", "plate"] {
        let p = mz(-45.0, form);
        let mut ps = lensbench_project::paths(&p, &env()).unwrap();
        ps.sort();
        assert_eq!(ps, ["RR", "RT", "TR", "TT"], "{form}");
        for (a, b) in [("TT", "RR"), ("TR", "RT")] {
            let (pa, ka) = chief(&sys(&p, a));
            let (pb, kb) = chief(&sys(&p, b));
            assert!(close(pa, pb, 1e-9), "{form} {a} {b}: {pa:?} {pb:?}");
            assert!(close(ka, kb, 1e-12), "{form} {a} {b}: {ka:?} {kb:?}");
        }
    }
}

/// 鏡 M2（反射の腕）を傾けると、反射の腕の光束だけが傾きの 2 倍の角で動き、BS2 と透過の腕は動かない
#[test]
fn tilting_a_mirror_moves_its_arm_only() {
    let (p0, p1) = (mz(-45.0, "ideal"), mz(-45.1, "ideal"));
    let (tt0, tt1) = (chief(&sys(&p0, "TT")), chief(&sys(&p1, "TT")));
    assert!(close(tt0.0, tt1.0, 1e-12) && close(tt0.1, tt1.1, 1e-15));
    let (rr0, rr1) = (chief(&sys(&p0, "RR")), chief(&sys(&p1, "RR")));
    let ang = dot(rr0.1, rr1.1).clamp(-1.0, 1.0).acos().to_degrees();
    assert!((ang - 0.2).abs() < 1e-9, "{ang}");
    assert!(!close(rr0.0, rr1.0, 1e-3));
}

/// 場面では BS2 は 1 つ: 両方の腕の BS2 の面が同じ面にまとまる
#[test]
fn the_joined_bs_is_one_object() {
    let p = mz(-45.0, "cube");
    let xs = expansions(&p, &env()).unwrap();
    assert_eq!(xs.len(), 4);
    let sc = Scene::from_project(&p, &env()).unwrap();
    let split: Vec<_> = sc
        .faces
        .iter()
        .filter(|f| {
            f.refs
                .iter()
                .any(|r| xs[r.system].1.map[r.surf].blocks.iter().any(|b| b == "BS2"))
        })
        .collect();
    assert!(!split.is_empty());
    // BS2 の斜面は 4 本の経路のどれからも来る
    assert!(split.iter().any(|f| {
        let mut v: Vec<usize> = f.refs.iter().map(|r| r.system).collect();
        v.sort();
        v.dedup();
        v.len() == 4
    }));
}

/// 合流の誤り: 並びの最後でない、無い BS、口へ向かわない
#[test]
fn join_errors() {
    let one = |blocks: &str| {
        let txt = format!(
            r#"{{"wavelengths": [{{"um": 0.6328}}], "aperture": {{"type": "epd", "value": 4}},
              "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 20}}, {blocks}]}}"#
        );
        let p = Project::from_json(&txt).unwrap();
        expand(&p, &env(), &ExpandOpts::default())
            .map(|_| ())
            .unwrap_err()
            .to_string()
    };
    let e = one(r#"{"type": "join", "splitter": "X", "port": "a"}, {"type": "air", "t": 5}"#);
    assert!(e.contains("last block"), "{e}");
    let e = one(r#"{"type": "join", "splitter": "X", "port": "a"}"#);
    assert!(e.contains("no BS X"), "{e}");
    // 鏡 M2 の傾きの向きが逆だと、反射の腕は BS2 の口 D の外から入らない
    let p = mz(45.0, "ideal");
    let e = expand(
        &p,
        &env(),
        &ExpandOpts {
            path: Some("RR"),
            ..Default::default()
        },
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(e.contains("BS2: the beam doesn't head into port D"), "{e}");
}

/// BS2 の大域の置き方（透過の腕の並びで置いたときと同じ所・同じ向き。キューブは BS1 の長さだけ先）
fn at(form: &str) -> String {
    let z = if form == "cube" { 80 } else { 70 };
    format!(r#", "at": {{"o": [0, 50, {z}], "tilt": [-90, 0, 180]}}"#)
}

/// BS2 を大域の座標に置いても、並びで置いたときと同じ所に出る。鏡 M1（BS2 を置いた透過の腕）を傾けると、
/// 並びで置いた BS2 は M1 と一緒に動くが、大域に置いた BS2 は動かず、透過の腕の光束だけが動く
#[test]
fn a_placed_bs_stays_when_a_mirror_tilts() {
    for form in ["ideal", "cube"] {
        let (p0, p1) = (mz(-45.0, form), mz_at(45.0, -45.0, form, &at(form)));
        for q in ["TT", "TR", "RT", "RR"] {
            let (a, b) = (chief(&sys(&p0, q)), chief(&sys(&p1, q)));
            assert!(
                close(a.0, b.0, 1e-9) && close(a.1, b.1, 1e-12),
                "{form} {q}: {a:?} {b:?}"
            );
        }
    }
    let p0 = mz_at(45.0, -45.0, "ideal", &at("ideal"));
    let p1 = mz_at(45.1, -45.0, "ideal", &at("ideal"));
    let (rr0, rr1) = (chief(&sys(&p0, "RR")), chief(&sys(&p1, "RR")));
    assert!(close(rr0.0, rr1.0, 1e-12) && close(rr0.1, rr1.1, 1e-15));
    let (tt0, tt1) = (chief(&sys(&p0, "TT")), chief(&sys(&p1, "TT")));
    let ang = dot(tt0.1, tt1.1).clamp(-1.0, 1.0).acos().to_degrees();
    assert!((ang - 0.2).abs() < 1e-9, "{ang}");
    // 並びで置いた BS2 は M1 と動くので、合流の腕の光束も動く
    let (q0, q1) = (mz(-45.0, "ideal"), mz_at(45.1, -45.0, "ideal", ""));
    assert!(!close(
        chief(&sys(&q0, "RR")).1,
        chief(&sys(&q1, "RR")).1,
        1e-6
    ));
    // at は偏心・傾きと一緒に使えない
    let p = mz_at(
        45.0,
        -45.0,
        "ideal",
        &format!(r#"{}, "tilt": [1, 0, 0]"#, at("ideal")),
    );
    let e = expand(
        &p,
        &env(),
        &ExpandOpts {
            path: Some("TT"),
            ..Default::default()
        },
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(e.contains("at can't be used"), "{e}");
}

/// 合流の先の BS の置き方は展開の間で使い回す（[`lensbench_project::SourceCache`]）が、置き方を決めたファイルが
/// 変われば集め直す: BS2 の前の空気を .zmx から取り込み、その厚さを書き換えると合流の経路も新しい置き方になる
/// （覚えた置き方と覚えずに集めた置き方で同じ）
#[test]
fn join_places_follow_a_changed_file() {
    let dir = std::env::temp_dir().join(format!("lb_join_places_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zmx = dir.join("gap.zmx");
    let write = |t: &str| {
        let txt = format!(
            "MODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nWAVM 1 0.6328 1\nSURF 0\n  TYPE STANDARD\n  DISZ INFINITY\nSURF 1\n  TYPE STANDARD\n  DISZ {t}\nSURF 2\n  TYPE STANDARD\n  DISZ 0\n"
        );
        std::fs::write(&zmx, txt).unwrap();
    };
    let mut p = mz(-45.0, "cube");
    let gap: lensbench_project::Block = serde_json::from_str(&format!(
        r#"{{"type": "zmx", "path": {:?}, "stop": false}}"#,
        zmx.to_string_lossy()
    ))
    .unwrap();
    match &mut p.blocks[2] {
        lensbench_project::Block::Splitter { spec, .. } => spec.t.insert(3, gap),
        _ => panic!("BS1 is the third block"),
    }
    let cache = lensbench_project::SourceCache::default();
    let at = |q: &str, cache: Option<&lensbench_project::SourceCache>| {
        let s = expand(
            &p,
            &env(),
            &ExpandOpts {
                path: Some(q),
                cache,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{q}: {e}"))
        .system;
        chief(&s)
    };
    let mut rr = Vec::new();
    for t in ["10", "25.5"] {
        write(t);
        for q in ["TT", "RR", "RT"] {
            let (a, b) = (at(q, Some(&cache)), at(q, None));
            assert!(
                close(a.0, b.0, 1e-12) && close(a.1, b.1, 1e-12),
                "{t} {q}: {a:?} {b:?}"
            );
        }
        rr.push(at("RR", Some(&cache)));
    }
    // 合流の経路は BS2 の新しい置き方で出る（前の置き方のままではない）
    assert!(!close(rr[0].0, rr[1].0, 1.0), "{rr:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 光束の `turn` で BS2 の手前で戻る光束があっても、合流の腕の光束は BS2 の置き方を引ける（置き方は turn を外して
/// 集める。光束ごとの展開は置き方を使い回す）
#[test]
fn a_turned_beam_does_not_hide_the_join_target() {
    let mut p = mz(-45.0, "cube");
    let lens: lensbench_project::Block = serde_json::from_str(
        r#"{"type": "lens", "id": "L", "source": {"surfaces": [
             {"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}}"#,
    )
    .unwrap();
    match &mut p.blocks[2] {
        lensbench_project::Block::Splitter { spec, .. } => spec.t.insert(1, lens),
        _ => panic!("BS1 is the third block"),
    }
    p.beams = serde_json::from_str(
        r#"[{"name": "g", "path": "T", "turn": {"block": "L", "surface": 1}},
            {"name": "r", "path": "RR"}]"#,
    )
    .unwrap();
    let xs = expansions(&p, &env()).unwrap();
    assert_eq!(xs.len(), 2);
}
