//! 物点・瞳座標を未知数にした光線の解きを、傾き偏心のある折返し系（P14: カタログレンズのダブルパス、
//! 傾けた球面鏡）で確かめる。

use lensbench_core::catalog::{LensCatalog, ZmxFileSource};
use lensbench_core::math::{dot, norm, sub};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::ObjSpec;
use lensbench_core::System;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// P14 に鏡の偏心も足した系。面: 1・2 レンズ（行き）、3 座標ブレーク、4 鏡、5 座標ブレーク（戻す）、
/// 6・7 レンズ（帰り）、8 物体面に戻ったところ
fn folded() -> System {
    let cat = LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap();
    let look = |v: &str, p: &str| cat.zmx(v, p);
    let s = lensbench_project::load(
        &root().join("examples/projects/P14_tilted_spherical_double_pass.json"),
        &[root().join("tests/glass")],
        Some(&look),
    )
    .unwrap();
    assert!(s.surfaces[4].mirror);
    assert_eq!((s.surfaces[3].cb_order, s.surfaces[5].cb_order), (0, 1));
    // 鏡を偏心させる（戻りの座標ブレークは逆の順で戻す）
    s.with_params(&[
        (s.param("3.dx").unwrap(), 0.3),
        (s.param("3.dy").unwrap(), -0.5),
        (s.param("5.dx").unwrap(), -0.3),
        (s.param("5.dy").unwrap(), 0.5),
    ])
    .unwrap()
}

/// 例 1: 物点（ファイバの XY）を動かして、主光線を鏡で法線入射させる（来た道を戻る）。
/// 面 1 の後（行き、ガラスの中）と面 6 の後（帰り、ガラスの中）の方向を逆向きにする。2 つの面の座標系は同じ向き
#[test]
fn chief_ray_returns_on_itself() {
    let s = folded();
    let w = s.primary;
    let pr = RayProblem::new(
        Unknown::Object,
        Residual::DirSum { s1: 1, s2: 6 },
        ObjSpec::Point([0.0, 0.0]),
        w,
    );
    let sol = s.solve_ray(&pr).unwrap();
    let r = sol.residual;
    assert!(r[0].abs().max(r[1].abs()) <= 1e-9, "残差 {r:?}");
    let ObjSpec::Point(obj) = sol.object else {
        unreachable!()
    };
    println!("物点 {obj:?}、反復 {}、残差 {r:?}", sol.iterations);
    // 解いた物点からの主光線を全面まで追い、本当に来た道を戻ることを確かめる
    let t = s.trace_spec(
        sol.object,
        &[0.0],
        &[0.0],
        w,
        lensbench_core::TraceOpts::new().vig([0.0; 5]).record(),
    );
    assert_eq!(t.rays[0].err, 0);
    for (a, b) in [(1, 7), (2, 6)] {
        let (ra, rb) = (t.rec_at(a).unwrap(), t.rec_at(b).unwrap());
        let d = norm(sub(ra.p[0], rb.p[0]));
        assert!(d < 1e-9, "面 {a} と {b} の交点が {d} ずれる");
    }
    // 物体面に戻る
    let back = t.rec_at(8).unwrap().xyz[0];
    assert!(
        (back[0] - obj[0]).abs() < 1e-8 && (back[1] - obj[1]).abs() < 1e-8,
        "{back:?} {obj:?}"
    );
    // 鏡への入射は法線方向: 鏡の後の方向（大域）が行きの逆
    let fr = s.frames();
    let k_in = t.rec_at(2).unwrap().lmn[0];
    let k_out = t.rec_at(4).unwrap().lmn[0];
    let (r2, r4) = (fr.surf[2].unwrap().r, fr.surf[4].unwrap().r);
    let g = |r: &[[f64; 3]; 3], v: [f64; 3]| {
        [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
    };
    let c = dot(g(&r2, k_in), g(&r4, k_out));
    assert!((c + 1.0).abs() < 1e-12, "cos {c}");
}

/// 例 2: 戻り光が指定した面の指定した点を通る物点（瞳座標は固定）
#[test]
fn return_ray_through_point() {
    let s = folded();
    let w = 0;
    for (surf, target, pupil) in [
        (8, [0.4, 1.3], [0.0, 0.0]),
        (7, [-1.0, 0.5], [0.5, -0.3]),
        (6, [0.2, -0.7], [-0.6, 0.4]),
    ] {
        let mut pr = RayProblem::new(
            Unknown::Object,
            Residual::Position { surf, target },
            ObjSpec::Point([0.0, 1.0]),
            w,
        );
        pr.pupil = pupil;
        let sol = s.solve_ray(&pr).unwrap();
        let r = sol.residual;
        assert!(r[0].abs().max(r[1].abs()) <= 1e-9, "面 {surf}: 残差 {r:?}");
        // 解いた物点で追い直しても同じ点を通る
        let t = s.trace_spec(
            sol.object,
            &[pupil[0]],
            &[pupil[1]],
            w,
            lensbench_core::TraceOpts::new().vig([0.0; 5]).record(),
        );
        let x = t.rec_at(surf).unwrap().xyz[0];
        assert!((x[0] - target[0]).abs() < 1e-9 && (x[1] - target[1]).abs() < 1e-9);
        println!(
            "面 {surf} の {target:?}: 物点 {:?}、反復 {}",
            sol.object, sol.iterations
        );
    }
}

/// 瞳座標を未知数に: 決めた物点から出て、戻り光が物体面の指定点を通る光線
#[test]
fn pupil_unknown() {
    let s = folded();
    let mut pr = RayProblem::new(
        Unknown::Pupil,
        Residual::Position {
            surf: 8,
            target: [0.3, 0.9],
        },
        ObjSpec::Point([0.0, 1.2]),
        s.primary,
    );
    pr.pupil = [0.1, 0.1];
    let sol = s.solve_ray(&pr).unwrap();
    let r = sol.residual;
    assert!(r[0].abs().max(r[1].abs()) <= 1e-9, "残差 {r:?}");
    println!("瞳 {:?}、反復 {}", sol.pupil, sol.iterations);
}

/// 方向余弦の差の残差（解いた光線を追い直して確かめる）と、解けない指定
#[test]
fn dir_diff_and_errors() {
    let s = folded();
    let w = s.primary;
    // 帰りの面 7 の後（物体空間）の方向を、行きの面 1 の前と同じにするのとは別に、ここでは
    // 面 2 の後（行き）と面 7 の後（帰り）の方向余弦をそろえる物点を解く
    let mut pr = RayProblem::new(
        Unknown::Object,
        Residual::DirDiff { s1: 2, s2: 7 },
        ObjSpec::Point([0.0, 0.0]),
        w,
    );
    pr.pupil = [0.3, 0.0];
    let sol = s.solve_ray(&pr).unwrap();
    let t = s.trace_spec(
        sol.object,
        &[0.3],
        &[0.0],
        w,
        lensbench_core::TraceOpts::new().vig([0.0; 5]).record(),
    );
    let (a, b) = (t.rec_at(2).unwrap().lmn[0], t.rec_at(7).unwrap().lmn[0]);
    assert!(
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
        "{a:?} {b:?}"
    );
    // 面の番号の範囲・無限遠物体で物点を未知数
    let bad = RayProblem::new(
        Unknown::Object,
        Residual::Position {
            surf: 99,
            target: [0.0, 0.0],
        },
        ObjSpec::Point([0.0, 0.0]),
        0,
    );
    assert!(s.solve_ray(&bad).is_err());
    let bad = RayProblem::new(
        Unknown::Object,
        Residual::Position {
            surf: 2,
            target: [0.0, 0.0],
        },
        ObjSpec::Dir([0.0, 0.0, 1.0]),
        0,
    );
    assert!(s.solve_ray(&bad).is_err());
}
