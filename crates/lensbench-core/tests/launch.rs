//! 光線を直接渡す追跡: 開始位置と方向（Launch::Rays）、絞り面の実座標（Launch::Stop）。

use crate::common::root;
use lensbench_core::param::{Kind, Param};
use lensbench_core::system::{Aiming, FieldSel, Launch, ObjSpec};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;

fn load(name: &str) -> System {
    zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

fn grid() -> (Vec<f64>, Vec<f64>) {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for i in -4..=4 {
        for j in -4..=4 {
            let (x, y) = (i as f64 / 4.0, j as f64 / 4.0);
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// 瞳座標の追跡の開始位置・方向をそのまま渡すと、同じ光線になる
#[test]
fn rays_reproduce_pupil_trace() {
    for name in ["A0", "A2", "B0", "B1", "D2"] {
        let s = load(name);
        let (px, py) = grid();
        for w in 0..s.wavelengths.len() {
            let t = s.trace(
                FieldSel::Index(s.fields.len() - 1),
                &px,
                &py,
                Some(w),
                None,
                true,
            );
            let r = s.trace_launch(&Launch::Rays { p: &t.p0, k: &t.k0 }, w, None, true);
            assert_eq!(r.rec.len(), t.rec.len());
            for (i, (a, b)) in t.rays.iter().zip(&r.rays).enumerate() {
                assert_eq!(a.err, b.err, "{name} {i}");
                if a.err != 0 {
                    continue;
                }
                for c in 0..3 {
                    assert!((a.xyz[c] - b.xyz[c]).abs() < 1e-11, "{name} {i}");
                    assert!((a.lmn[c] - b.lmn[c]).abs() < 1e-13, "{name} {i}");
                }
                assert!((a.opl - b.opl).abs() < 1e-11, "{name} {i}");
            }
        }
    }
}

/// 絞り面の実座標で渡すと、その点を通る。実光線エイミングの系では、瞳座標 × 絞りの半径と同じ光線
#[test]
fn stop_points_hit_the_stop() {
    for name in ["A0", "A2", "B0", "B1", "C1", "D2", "D4"] {
        let s = load(name);
        let st = s.stop();
        let (px, py) = grid();
        let (rx, ry) = s.stop_radius();
        let xs: Vec<f64> = px.iter().map(|v| v * rx).collect();
        let ys: Vec<f64> = py.iter().map(|v| v * ry).collect();
        let (hx, hy) = s.hxhy(s.fields.len() - 1);
        let spec = s.object_spec(hx, hy).unwrap();
        let w = s.primary;
        let r = s.trace_launch(
            &Launch::Stop {
                spec,
                x: &xs,
                y: &ys,
            },
            w,
            None,
            true,
        );
        let rs = r.rec.iter().find(|q| q.surf == st).unwrap();
        for i in 0..xs.len() {
            if r.rays[i].err != 0 {
                continue;
            }
            let e = (rs.xyz[i][0] - xs[i])
                .abs()
                .max((rs.xyz[i][1] - ys[i]).abs());
            assert!(e < 1e-9, "{name} {i}: {e}");
        }
        let ok = r.rays.iter().filter(|q| q.err == 0).count();
        assert!(ok * 10 >= xs.len() * 9, "{name}: {ok}/{}", xs.len());
        if s.aiming == Aiming::Real {
            let t = s.trace_launch(
                &Launch::Pupil {
                    spec,
                    signed: false,
                    px: &px,
                    py: &py,
                    vig: Some([0.0; 5]),
                    hxy: None,
                },
                w,
                None,
                false,
            );
            for i in 0..xs.len() {
                if t.rays[i].err != 0 || r.rays[i].err != 0 {
                    continue;
                }
                for c in 0..3 {
                    assert!(
                        (t.rays[i].xyz[c] - r.rays[i].xyz[c]).abs() < 1e-8,
                        "{name} {i}"
                    );
                }
            }
        }
    }
}

/// 物体側テレセントリックの系（理想レンズ f 50 の後側焦点に絞り）。絞りより前の厚さを少し変えても、
/// 絞り面の点で渡した光線は同じ側を通る（瞳座標では向きが反転しうる）
#[test]
fn stop_points_are_continuous_near_telecentric() {
    let t = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nFLOA\nFTYP 1 0 1 1 0 0 0\n\
             XFLN 0\nYFLN 1\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
             SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
             SURF 1\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
             SURF 2\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n  DIAM 2 1 0 0 1 \"\"\n\
             SURF 3\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
             SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = zmx::parse_text(t).unwrap();
    let s0 = zmx::build(&raw, &Default::default(), &[], None).unwrap();
    let (xs, ys) = (vec![1.0, 0.0, -0.5], vec![0.0, 1.0, 0.5]);
    for dt in [0.0, 1e-5, -1e-5, 1e-9, -1e-9] {
        let s = s0
            .with_params(&[(Param::new(1, Kind::Thick), 50.0 + dt)])
            .unwrap();
        let spec = ObjSpec::Point([0.3, -0.2]);
        let r = s.trace_launch(
            &Launch::Stop {
                spec,
                x: &xs,
                y: &ys,
            },
            0,
            Some(2),
            true,
        );
        for i in 0..xs.len() {
            assert_eq!(r.rays[i].err, 0, "{dt} {i}");
            let q = r.rays[i].xyz;
            assert!(
                (q[0] - xs[i]).abs() < 1e-9 && (q[1] - ys[i]).abs() < 1e-9,
                "{dt} {i}: {q:?}"
            );
        }
    }
}

/// 理想レンズ f 50 の 2 枚（物体距離 50、絞りは半径 2 で第 1 レンズの t1 後ろ）
fn two_lens(t1: f64, aim: u8) -> System {
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nFLOA\nRAIM 0 {aim} 1 1 0 0 0 0 0 1\n\
         FTYP 1 0 2 1 0 0 0\nXFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
         SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
         SURF 1\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ {t1}\n\
         SURF 2\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n  DIAM 2 1 0 0 1 \"\"\n\
         SURF 3\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
         SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let raw = zmx::parse_text(&t).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

/// OpticStudio 26.1 の実測（2026-11-07）: 入射瞳が物体より前（ENPP < 0）だと px = 1 は絞りの −x 側、
/// 物体空間の L は負。近軸エイミングでも同じ。テレセントリック（t1 = 50）の前後で反転し、t1 = 50 は ENPP = +1e10。
/// signed の瞳座標では常に +x 側
#[test]
fn pupil_orientation_matches_opticstudio_and_signed_is_continuous() {
    let l_os = 0.03996803835;
    let cases: [(f64, u8, f64); 9] = [
        (100.0, 0, -1.0),
        (100.0, 1, -1.0),
        (50.0, 0, 1.0),
        (50.0 + 1e-5, 0, -1.0),
        (50.0 - 1e-5, 0, 1.0),
        (50.0 + 1e-3, 0, -1.0),
        (50.0 - 1e-3, 0, 1.0),
        (50.1, 0, -1.0),
        (49.9, 0, 1.0),
    ];
    for (t1, aim, sx) in cases {
        let s = two_lens(t1, aim);
        assert_eq!(s.pupil_reversed(), sx < 0.0, "{t1} {aim}");
        let spec = s.object_spec(0.0, 0.0).unwrap();
        let px = [1.0, 0.0];
        let py = [0.0, 1.0];
        for signed in [false, true] {
            let t = s.trace_launch(
                &Launch::Pupil {
                    spec,
                    signed,
                    px: &px,
                    py: &py,
                    vig: Some([0.0; 5]),
                    hxy: None,
                },
                0,
                Some(2),
                false,
            );
            let want = if signed { 1.0 } else { sx };
            let (a, b) = (t.rays[0].xyz, t.rays[1].xyz);
            assert!(
                (a[0] - 2.0 * want).abs() < 1e-9 && a[1].abs() < 1e-9,
                "{t1} {aim} {signed} {a:?}"
            );
            assert!(
                (b[1] - 2.0 * want).abs() < 1e-9 && b[0].abs() < 1e-9,
                "{t1} {aim} {signed} {b:?}"
            );
            assert!(
                (t.k0[0][0] - want * l_os).abs() < 1e-10,
                "{t1} {aim} {signed} {:?}",
                t.k0[0]
            );
        }
    }
}
