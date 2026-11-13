//! ビームスプリッター: 手で組んだ同じ系との一致、経路の列挙、往き透過・戻り反射、膜の係数。

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::FieldSel;
use lensbench_core::System;
use lensbench_project::{expand_path, paths, Env, Project};
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

const HEAD: &str = r#""glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "na", "value": 0.05},
  "fields": {"type": "object_height", "points": [[0, 0], [0, 1.5], [1, -1]]},
  "object_distance": 50,"#;

const LENS: &str = r#"{"type": "lens", "id": "L", "source": {"surfaces": [
   {"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}}"#;

fn proj(blocks: &str) -> Project {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    Project::from_json(&txt).unwrap()
}

fn sys(blocks: &str, path: Option<&str>) -> System {
    expand_path(&proj(blocks), &env(), path).unwrap().0
}

const PX: [f64; 5] = [0.0, 0.7, -0.5, 0.3, 0.0];
const PY: [f64; 5] = [0.0, 0.2, 0.6, -0.9, -1.0];

/// 像面の光線（大域の位置・方向と光路長）を比べる
fn same_global(a: &System, b: &System, tol: f64) {
    same_global_opt(a, b, tol, false)
}

/// same_global と同じ。allow_vig なら、両方で同じにけられた光線は比べない
fn same_global_opt(a: &System, b: &System, tol: f64, allow_vig: bool) {
    for f in 0..a.fields.len() {
        for w in 0..a.wavelengths.len() {
            let t0 = a
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            let t1 = b
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            for i in 0..PX.len() {
                let (r0, r1) = (&t0.rays[i], &t1.rays[i]);
                if allow_vig {
                    assert_eq!((r0.err, r0.vig), (r1.err, r1.vig), "{} {f} {w} {i}", a.name);
                    if r0.err != 0 || r0.vig != 0 {
                        continue;
                    }
                }
                assert_eq!((r0.err, r0.vig), (0, 0), "{f} {w} {i}");
                assert_eq!((r1.err, r1.vig), (0, 0), "{f} {w} {i}");
                for k in 0..3 {
                    assert!(
                        (r0.p[k] - r1.p[k]).abs() < tol && (r0.k[k] - r1.k[k]).abs() < tol,
                        "{f} {w} {i} {:?} {:?} / {:?} {:?}",
                        r0.p,
                        r1.p,
                        r0.k,
                        r1.k
                    );
                }
                assert!(
                    (r0.opl - r1.opl).abs() < tol,
                    "{f} {w} {i} {} {}",
                    r0.opl,
                    r1.opl
                );
            }
        }
    }
}

#[test]
fn plate_transmission_is_a_tilted_plate() {
    let bs = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "plate", "thickness": 3, "glass": "N-BK7", "angle": [45, 0]}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
        ),
        Some("T"),
    );
    let hand = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "lens", "source": {{"surfaces": [{{"t": 3, "glass": "N-BK7"}}, {{}}]}}, "tilt": [45, 0, 0]}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
        ),
        None,
    );
    same_global(&bs, &hand, 1e-11);
}

#[test]
fn ideal_reflection_is_a_fold_mirror() {
    for ang in ["[45, 0]", "[0, -30]", "[20, 35]"] {
        let bs = sys(
            &format!(
                r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
               {{"type": "splitter", "id": "BS", "form": "ideal", "angle": {ang}}},
               {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
            ),
            Some("R"),
        );
        let hand = sys(
            &format!(
                r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
               {{"type": "mirror", "tilt": {ang}}},
               {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
            ),
            None,
        );
        same_global(&bs, &hand, 1e-11);
        // 2 軸の傾きでも反射した軸上の主光線は後ろの面の (0, 0) を通る
        let t = bs.trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, true);
        for j in bs.surfaces.len() - 3..bs.surfaces.len() {
            if let Some(r) = t.rec_at(j) {
                assert!(
                    r.xyz[0][0].abs().max(r.xyz[0][1].abs()) < 1e-9,
                    "{ang} 面 {j}"
                );
            }
        }
        // ideal の透過はそのまま
        let t = sys(
            &format!(
                r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
               {{"type": "splitter", "id": "BS", "form": "ideal", "angle": {ang}}},
               {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
            ),
            Some("T"),
        );
        let straight = sys(
            &format!(
                r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 30}}, {LENS}, {{"type": "air", "t": 60}}]"#
            ),
            None,
        );
        same_global(&t, &straight, 1e-11);
    }
}

#[test]
fn cube_transmission_is_a_glass_block() {
    let bs = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "cube", "size": 12, "glass": "N-BK7"}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
        ),
        None,
    );
    let hand = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "lens", "source": {{"surfaces": [{{"t": 12, "glass": "N-BK7"}}, {{}}]}}}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
        ),
        None,
    );
    same_global(&bs, &hand, 1e-11);
}

/// キューブの反射は、ガラスの中で 45° の鏡で折り曲げた系（手で面を並べた .zmx）と同じ
#[test]
fn cube_reflection_is_a_fold_in_glass() {
    let bs = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "cube", "size": 12, "glass": "N-BK7", "angle": [45, 0]}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
        ),
        Some("R"),
    );
    // 面 1 絞り(10) / 面 2 入口 N-BK7 (6) / CB 45 / 鏡 / CB 45 (-6) / 面 出口 (−20) / 玉 …
    let zmx = "VERS 260101 1 0\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 1\nFLOA\nGCAT SCHOTT\n\
FTYP 0 0 1 3 0 0 0\nWAVM 1 0.4861327 1\nWAVM 2 0.5875618 1\nWAVM 3 0.6562725 1\nPWAV 2\n\
SURF 0\n  TYPE STANDARD\n  CURV 0\n  DISZ 50\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0\n  DISZ 10\n\
SURF 2\n  TYPE STANDARD\n  CURV 0\n  GLAS N-BK7 0 0 1.5 40\n  DISZ 6\n\
SURF 3\n  TYPE COORDBRK\n  PARM 1 0\n  PARM 2 0\n  PARM 3 45\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0\n  GLAS MIRROR\n  DISZ 0\n\
SURF 5\n  TYPE COORDBRK\n  PARM 1 0\n  PARM 2 0\n  PARM 3 45\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ -6\n\
SURF 6\n  TYPE STANDARD\n  CURV 0\n  DISZ -20\n\
SURF 7\n  TYPE STANDARD\n  CURV -0.025\n  GLAS N-BK7 0 0 1.5 40\n  DISZ -4\n\
SURF 8\n  TYPE STANDARD\n  CURV 0.016666666666666666\n  DISZ -60\n\
SURF 9\n  TYPE STANDARD\n  CURV 0\n  DISZ 0\n";
    let dir = std::env::temp_dir().join("lensbench_splitter_test");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("cube_r.zmx");
    std::fs::write(&f, zmx).unwrap();
    let mut hand = lensbench_core::zmx::load_with(
        &f,
        &lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    // 開口・視野をプロジェクトと同じに
    hand.fields = bs.fields.clone();
    hand.field_type = bs.field_type;
    hand.aperture = bs.aperture;
    hand.changed().unwrap();
    same_global(&bs, &hand, 1e-11);
}

const ARM: &str = r#"{"type": "air", "t": 15}, {"type": "lens", "source": {"surfaces": [
   {"radius": 30, "t": 3, "glass": "N-BK7", "semi": 8}, {"radius": -30, "semi": 8}]}},
   {"type": "air", "t": 25}"#;

/// 往きは透過、戻りは反射（Twyman-Green の被検の腕）。分岐を書かない形
#[test]
fn double_pass_turns_at_the_splitter() {
    let blocks = format!(
        r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "cube", "size": 10, "glass": "N-BK7", "ar": "I.s=1,p=1"}},
           {ARM}, {{"type": "mirror", "back": true, "coat": "I.s=0,p=0"}}, {{"type": "air", "t": 30}}]"#
    );
    let p = proj(&blocks);
    assert_eq!(paths(&p, &env()).unwrap(), vec!["TT", "TR", "RT", "RR"]);
    let tr = sys(&blocks, Some("TR"));
    let tt = sys(&blocks, Some("TT"));
    // TR: 主光線は D の口（−dC = +y）へ出る。TT: A の口（−z）へ戻る
    let chief = |s: &System| {
        s.try_trace(FieldSel::Index(0), &[0.0], &[0.0], Some(1), None, false)
            .unwrap()
            .rays[0]
    };
    let (a, b) = (chief(&tr), chief(&tt));
    assert_eq!((a.err, a.vig, b.err, b.vig), (0, 0, 0, 0));
    let dc = {
        // 傾き [45, 0] の面での +z の反射
        let r = lensbench_core::math::rot(0, 45.0);
        let n = lensbench_core::math::col(&r, 2);
        [
            -2.0 * n[2] * n[0],
            -2.0 * n[2] * n[1],
            1.0 - 2.0 * n[2] * n[2],
        ]
    };
    for k in 0..3 {
        assert!((a.k[k] + dc[k]).abs() < 1e-12, "{:?} {dc:?}", a.k);
    }
    assert!((b.k[2] + 1.0).abs() < 1e-12, "{:?}", b.k);
    // 理想の 50:50: TR の電場は √0.5 · (−√0.5) = −0.5（主光線、AR は理想の全透過。腕は空気だけにする）
    let tr = sys(
        r#"[{"type": "stop", "semi": 3}, {"type": "air", "t": 10},
           {"type": "splitter", "id": "BS", "form": "cube", "size": 10, "glass": "N-BK7", "ar": "I.s=1,p=1"},
           {"type": "air", "t": 40}, {"type": "mirror", "back": true, "coat": "I.s=0,p=0"}, {"type": "air", "t": 30}]"#,
        Some("TR"),
    );
    let c = Coatings::for_system(&tr).unwrap();
    let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
    let pt = tr
        .try_trace_pol_opts(FieldSel::Index(0), &[0.0], &[0.0], Some(1), &[x], &c, false)
        .unwrap();
    let e = pt.e[0][0];
    let amp = (e[0].norm_sqr() + e[1].norm_sqr() + e[2].norm_sqr()).sqrt();
    assert!((amp - 0.5).abs() < 1e-12, "{amp}");
}

/// Michelson: 2 つの腕（分岐 t・r）から戻った光が D の口で重なる。光路差は腕の長さの差の 2 倍
#[test]
fn michelson_arms_meet_at_port_d() {
    let blocks = |dl: f64| {
        format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "cube", "size": 30, "glass": "N-BK7", "ratio": 0.4,
             "t": [{{"type": "air", "t": 40}}, {{"type": "mirror", "back": true, "coat": "I.s=0,p=0"}}],
             "r": [{{"type": "air", "t": {}}}, {{"type": "mirror", "back": true, "coat": "I.s=0,p=0"}}],
             "d": [{{"type": "air", "t": 30}}]}}]"#,
            40.0 + dl
        )
    };
    let p = proj(&blocks(0.25));
    assert_eq!(paths(&p, &env()).unwrap(), vec!["TT", "TR", "RT", "RR"]);
    let tr = sys(&blocks(0.25), Some("TR"));
    let rt = sys(&blocks(0.25), Some("RT"));
    for f in 0..3 {
        let a = tr
            .try_trace(FieldSel::Index(f), &PX, &PY, Some(1), None, false)
            .unwrap();
        let b = rt
            .try_trace(FieldSel::Index(f), &PX, &PY, Some(1), None, false)
            .unwrap();
        for i in 0..PX.len() {
            let (ra, rb) = (a.rays[i], b.rays[i]);
            assert_eq!((ra.err, ra.vig, rb.err, rb.vig), (0, 0, 0, 0));
            // 同じ方向で D の口へ
            for k in 0..3 {
                assert!((ra.k[k] - rb.k[k]).abs() < 1e-12);
            }
            // 平面鏡の腕なので、光路差は (腕の差 × 2) を光線の向きの cos で割ったもの … 軸上の主光線で確かめる
            if f == 0 && i == 0 {
                assert!(
                    ((rb.opl - ra.opl) - 0.5).abs() < 1e-11,
                    "{}",
                    rb.opl - ra.opl
                );
            }
        }
    }
    // 分岐比 0.4: TR の強度 0.4 · 0.6、RT も 0.6 · 0.4（膜なしの面のフレネルを除いて比べる）
    let c = Coatings::for_system(&tr).unwrap();
    let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
    let ia = |s: &System| {
        let pt = s
            .try_trace_pol_opts(FieldSel::Index(0), &[0.0], &[0.0], Some(1), &[x], &c, false)
            .unwrap();
        let e = pt.e[0][0];
        e[0].norm_sqr() + e[1].norm_sqr() + e[2].norm_sqr()
    };
    let (a, b) = (ia(&tr), ia(&rt));
    assert!((a - b).abs() < 1e-14, "{a} {b}");
    let n = tr.surfaces[3].n[1];
    let fr = 4.0 * n / ((n + 1.0) * (n + 1.0)); // 空気とガラスの面の垂直入射の透過率
    assert!((a - 0.24 * fr.powi(4)).abs() < 1e-12, "{a}");
}

/// 理想 PBS: p（分ける面の入射面の中の偏光、傾き x なら y）は透過、s は反射。消光比 ts
#[test]
fn ideal_pbs_splits_s_and_p() {
    let blocks = r#"[{"type": "stop", "semi": 3}, {"type": "air", "t": 10},
           {"type": "splitter", "id": "BS", "form": "cube", "size": 10, "glass": "N-BK7", "ar": "I.s=1,p=1",
            "pbs": {"tp": 0.98, "ts": 0.001}},
           {"type": "air", "t": 30}]"#;
    let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
    let y = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
    for (path, ix, iy) in [("T", 0.001, 0.98), ("R", 0.999, 0.02)] {
        let s = sys(blocks, Some(path));
        let c = Coatings::for_system(&s).unwrap();
        let pt = s
            .try_trace_pol_opts(
                FieldSel::Index(0),
                &[0.0],
                &[0.0],
                Some(1),
                &[x, y],
                &c,
                false,
            )
            .unwrap();
        let i = |e: [C64; 3]| e[0].norm_sqr() + e[1].norm_sqr() + e[2].norm_sqr();
        let (a, b) = (i(pt.e[0][0]), i(pt.e[0][1]));
        assert!(
            (a - ix).abs() < 1e-12 && (b - iy).abs() < 1e-12,
            "{path}: {a} {b}"
        );
    }
}

#[test]
fn off_splitter_is_removed_and_path_letters_are_checked() {
    let on = format!(
        r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "ideal", "on": false}},
           {{"type": "air", "t": 20}}, {LENS}, {{"type": "air", "t": 60}}]"#
    );
    let straight = format!(
        r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 30}}, {LENS}, {{"type": "air", "t": 60}}]"#
    );
    same_global(&sys(&on, None), &sys(&straight, None), 1e-12);
    assert!(paths(&proj(&on), &env()).unwrap().is_empty());
    let e = expand_path(&proj(&on), &env(), Some("T")).unwrap_err();
    assert!(e.contains("文字が多い"), "{e}");
    let dup = r#"[{"type": "stop"}, {"type": "splitter", "id": "BS", "form": "ideal"},
        {"type": "splitter", "id": "BS", "form": "ideal"}, {"type": "air", "t": 5}]"#;
    assert!(expand_path(&proj(dup), &env(), None)
        .unwrap_err()
        .contains("2 つ以上"));
}

/// 状態で経路を選ぶ
#[test]
fn a_state_selects_the_path() {
    let txt = format!(
        r#"{{ {HEAD} "blocks": [{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "ideal"}}, {{"type": "air", "t": 20}}],
           "states": [{{"name": "refl", "project": {{"path": "R"}}}}] }}"#
    );
    let p = Project::from_json(&txt).unwrap();
    let r = p.with_state("refl").unwrap();
    let (s, _) = expand_path(&r, &env(), None).unwrap();
    assert!(s.surfaces.iter().any(|d| d.mirror));
    let (s, _) = expand_path(&p, &env(), None).unwrap();
    assert!(!s.surfaces.iter().any(|d| d.mirror));
}

/// くさびの平板: 垂直に置いた板の裏面を α 傾けると、主光線は asin(n sin α) − α 曲がる
#[test]
fn wedge_plate_deviates_the_beam() {
    let a = 0.5f64;
    let s = sys(
        &format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "plate", "thickness": 4, "glass": "N-BK7",
             "angle": [0, 0], "wedge": [{a}, 0]}}, {{"type": "air", "t": 20}}]"#
        ),
        Some("T"),
    );
    let n = s.surfaces[3].n[1];
    let r = s
        .try_trace(FieldSel::Index(0), &[0.0], &[0.0], Some(1), None, false)
        .unwrap()
        .rays[0];
    let ar = a.to_radians();
    let dev = (n * ar.sin()).asin() - ar;
    let got = r.k[1].atan2(r.k[2]).abs();
    assert!((got - dev).abs() < 1e-12, "{got} {dev}");
}

/// 平板の裏で分ける: 反射は表で分けるときと同じ向き（板の中を往復するぶん横にずれる）
#[test]
fn plate_split_on_the_back_reflects_in_the_same_direction() {
    let b = |face: &str| {
        format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}},
           {{"type": "splitter", "id": "BS", "form": "plate", "thickness": 4, "glass": "N-BK7",
             "split_face": "{face}"}}, {{"type": "air", "t": 20}}]"#
        )
    };
    let chief = |s: &System| {
        s.try_trace(FieldSel::Index(1), &[0.0], &[0.0], Some(1), None, false)
            .unwrap()
            .rays[0]
    };
    let (f, k) = (
        chief(&sys(&b("front"), Some("R"))),
        chief(&sys(&b("back"), Some("R"))),
    );
    for i in 0..3 {
        assert!((f.k[i] - k.k[i]).abs() < 1e-12, "{:?} {:?}", f.k, k.k);
    }
    assert!((f.p[2] - k.p[2]).abs() > 1.0);
    // 裏で分けると往きも戻りも板を通る: T・R とも 1 回だけ BS を通る経路
    assert_eq!(paths(&proj(&b("back")), &env()).unwrap(), vec!["T", "R"]);
}

fn splitter_dir() -> PathBuf {
    root().join("tests/splitter")
}

/// tests/splitter の照合の系（examples/bs_cases.rs で作った .json と、その経路を書き出した .zmx）
const OS_CASES: [&str; 8] = [
    "cube_T",
    "cube_R",
    "plate_T",
    "plate_R",
    "plate_back_R",
    "tg_TR",
    "mich_TR",
    "mich_RT",
];

fn os_case(name: &str) -> System {
    let txt = std::fs::read_to_string(splitter_dir().join(format!("{name}.json"))).unwrap();
    let p = Project::from_json(&txt).unwrap();
    expand_path(&p, &env(), None).unwrap().0
}

/// 書き出した .zmx（反射の面は膜なしの鏡）を読み直すと、プロジェクトの経路と同じ光線
#[test]
fn written_paths_read_back_as_the_same_system() {
    for name in OS_CASES {
        let s = os_case(name);
        let z = lensbench_core::zmx::load_with(
            &splitter_dir().join(format!("{name}.zmx")),
            &lensbench_core::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root().join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        same_global_opt(&s, &z, 1e-9, true);
    }
}

/// OpticStudio 26.1 との照合（tests/splitter/bs_refs.json、make_bs_refs.py で作成）: lensbench が書いた各経路の
/// .zmx（反射の面は膜なしの鏡）を OpticStudio で追跡した像面の局所の位置・方向・光路長・けられの面と、
/// cube_T・plate_T（透過だけの経路）の偏光の電場（膜 BS3・AR2、LBBS.DAT）
#[test]
fn paths_match_opticstudio() {
    use lensbench_core::coating::CoatingFile;
    let refs: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(splitter_dir().join("bs_refs.json")).unwrap(),
    )
    .unwrap();
    let c = Coatings {
        file: CoatingFile::load(&splitter_dir().join("LBBS.DAT")).unwrap(),
        ..Default::default()
    };
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    let (mut wp, mut wd, mut wl, mut we) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for name in OS_CASES {
        let s = os_case(name);
        let case = &refs["cases"][name];
        for r in case["unpol"].as_array().unwrap() {
            let w = r["wave"].as_u64().unwrap() as usize - 1;
            let tr = s
                .try_trace(
                    FieldSel::Norm(f(&r["hx"]), f(&r["hy"])),
                    &[f(&r["px"])],
                    &[f(&r["py"])],
                    Some(w),
                    None,
                    false,
                )
                .unwrap();
            let t = tr.rays[0];
            let (err, vig) = (
                r["err"].as_u64().unwrap() as usize,
                r["vig"].as_u64().unwrap() as usize,
            );
            assert_eq!((t.err, t.vig), (err, vig), "{name} {r}");
            if err != 0 || vig != 0 {
                continue;
            }
            for k in 0..3 {
                wp = wp.max((t.xyz[k] - f(&r["xyz"][k])).abs());
                // OpticStudio の方向余弦は鏡の数の偶奇（parity）を掛けた値
                wd = wd.max((tr.parity * t.lmn[k] - f(&r["lmn"][k])).abs());
            }
            wl = wl.max((t.opl - f(&r["opl"])).abs());
        }
        let Some(pol) = case["pol"].as_array() else {
            continue;
        };
        for r in pol {
            let w = r["wave"].as_u64().unwrap() as usize - 1;
            let inp = if r["in"].as_u64().unwrap() == 0 {
                [C64::new(1.0, 0.0), C64::new(0.0, 0.0)]
            } else {
                [C64::new(0.0, 0.0), C64::new(1.0, 0.0)]
            };
            let pt = s
                .try_trace_pol(
                    FieldSel::Norm(f(&r["hx"]), f(&r["hy"])),
                    &[f(&r["px"])],
                    &[f(&r["py"])],
                    Some(w),
                    &[inp],
                    &c,
                )
                .unwrap();
            let e = pt.e[0][0];
            for k in 0..3 {
                let z = C64::new(f(&r["E"][k][0]), f(&r["E"][k][1]));
                we = we.max((e[k] - z).norm());
            }
        }
    }
    eprintln!("位置 {wp:.1e} mm、方向 {wd:.1e}、光路長 {wl:.1e} mm、電場 {we:.1e}");
    assert!(
        wp < 1e-9 && wd < 1e-11 && wl < 1e-8 && we < 1e-6,
        "{wp} {wd} {wl} {we}"
    );
}

/// 分ける面の膜 BS3 の反射（硝材どうし N-BK7|N-BK7 と空気から AIR|N-BK7）を OpticStudio の
/// Analyze > Coatings > Reflection vs Angle（主波長）と比べる。R はそのまま、位相は膜の側から入るとき
/// 基準面の違い exp(2 i k n1 cosθ Σd)（ゴーストと同じ約束、tests/ghost.rs）を戻して比べる
#[test]
fn split_coating_reflection_matches_opticstudio() {
    use lensbench_core::coating::CoatingFile;
    use std::f64::consts::PI;
    let c = Coatings {
        file: CoatingFile::load(&splitter_dir().join("LBBS.DAT")).unwrap(),
        ..Default::default()
    };
    let s = os_case("cube_T");
    let wl = s.wavelengths[s.primary];
    let n = s.surfaces[2].n[s.primary];
    let k = 2.0 * PI / wl;
    let d = 0.0655 + 0.0996 + 0.0655;
    let (mut wr, mut wph) = (0.0f64, 0.0f64);
    for (file, key, n1, n2, front) in [
        ("reflect_cube_T.txt", "BS3|N-BK7|N-BK7|R", n, n, true),
        ("reflect_plate_T.txt", "BS3|AIR|N-BK7|R", 1.0, n, true),
    ] {
        let fi = c.prepare(&s, key, wl, n1, n2).unwrap();
        let txt = std::fs::read_to_string(splitter_dir().join(file)).unwrap();
        let mut rows = 0;
        for line in txt.lines() {
            let v: Vec<f64> = line
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            // Angle, Rs, Rp, Ts, Tp, As, Ap, 位相 T s・p, 位相 R s・p, …
            if v.len() != 15 {
                continue;
            }
            rows += 1;
            let th = v[0].to_radians();
            let mut r = fi.eval(th).unwrap();
            if front {
                let e = C64::new(0.0, -2.0 * k * n1 * th.cos() * d).exp();
                r = r.map(|z| z * e);
            }
            for p in 0..2 {
                wr = wr.max((r[p].norm_sqr() - v[1 + p]).abs());
                let mut dd = r[p].arg().to_degrees() - v[9 + p];
                dd -= 360.0 * (dd / 360.0).round();
                wph = wph.max(dd.abs());
            }
        }
        assert!(rows > 40, "{file}: {rows} 行");
    }
    eprintln!("R {wr:.1e}, 位相 {wph:.1e} deg");
    assert!(wr < 1e-6 && wph < 1e-3, "R {wr:.1e} 位相 {wph:.1e}");
}
