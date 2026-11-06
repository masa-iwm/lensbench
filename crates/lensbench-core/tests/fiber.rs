//! ファイバ結合（fiber::fiber_coupling）をガウスの式と比べる。物体側・像側とも f 50 の近軸レンズの 4f 系
//! （1:1 の結像で方向余弦がそのまま移る、収差なし）

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::fiber::{fiber_coupling, FiberOpts};
use lensbench_core::system::System;
use lensbench_core::zmx;

fn relay(defocus: f64) -> System {
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nOBNA 0.3 0\nFTYP 0 0 1 1 0 0 0\nYFLN 0\nFWGN 1\n\
WAVM 1 0.6 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 100\n\
SURF 2\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ {}\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        50.0 + defocus
    );
    let raw = zmx::parse_text(&t).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn opts(a: f64, b: f64) -> FiberOpts {
    FiberOpts {
        na_source: a,
        na_receiver: b,
        grid: 151,
        ..Default::default()
    }
}

#[test]
fn matched_mismatched_and_offset() {
    let s = relay(0.0);
    let c = fiber_coupling(&s, 0, &opts(0.1, 0.1), None).unwrap();
    assert!((c.eta - 1.0).abs() < 5e-5, "{}", c.eta);
    assert!((c.system - 1.0).abs() < 1e-8, "{}", c.system - 1.0);
    // NA の違い: η = (2ab/(a² + b²))²
    let (a, b) = (0.1, 0.05);
    let c = fiber_coupling(&s, 0, &opts(a, b), None).unwrap();
    let want = (2.0 * a * b / (a * a + b * b)).powi(2);
    assert!((c.eta - want).abs() < 5e-5, "{} {want}", c.eta);
    // 受け側の横ずれ Δ: η = exp(−Δ²/w0²)、w0 = λ/(π NA)
    let mut o = opts(0.1, 0.1);
    o.receiver = [0.001, 0.0];
    let c = fiber_coupling(&s, 0, &o, None).unwrap();
    let w0 = 0.6e-3 / (std::f64::consts::PI * 0.1);
    let want = (-(0.001f64 / w0).powi(2)).exp();
    assert!((c.eta - want).abs() < 5e-5, "{} {want}", c.eta);
    // 送り側の横ずれも同じ（1:1 の結像で像は反対向きにずれるので、受け側を合わせると 1）
    let mut o = opts(0.1, 0.1);
    o.source = [0.0, 0.002];
    o.receiver = [0.0, -0.002];
    let c = fiber_coupling(&s, 0, &o, None).unwrap();
    assert!((c.eta - 1.0).abs() < 5e-5, "{}", c.eta);
}

/// 焦点ずれ Δz: 方向余弦の空間で位相 exp(ik Δz cos θ) の重なり（数値積分）と比べる
#[test]
fn defocus() {
    let dz = 0.01;
    let s = relay(dz);
    let na = 0.1;
    let c = fiber_coupling(&s, 0, &opts(na, na), None).unwrap();
    let k = 2.0 * std::f64::consts::PI / 0.6e-3;
    // 極座標の 1 次元の積分: η = |∫ e^{−2ρ²/a²} e^{ik Δz √(1−ρ²)} 2πρ dρ|² / (π a²/2)²
    let n = 200_000;
    let rmax = 3.0 * na;
    let mut z = C64::new(0.0, 0.0);
    for i in 0..n {
        let r = (i as f64 + 0.5) * rmax / n as f64;
        z += C64::from_polar(
            (-2.0 * r * r / (na * na)).exp() * 2.0 * std::f64::consts::PI * r,
            k * dz * (1.0 - r * r).sqrt(),
        );
    }
    z *= rmax / n as f64;
    let want = z.norm_sqr() / (std::f64::consts::PI * na * na / 2.0).powi(2);
    assert!((c.eta - want).abs() < 2e-5, "{} {want}", c.eta);
    assert!(c.eta < 0.99);
}

/// 偏光込み: 膜のない系では C は単位行列に近く、どの入射偏光でも η とほぼ同じ
#[test]
fn jones() {
    let s = relay(0.0);
    let coat = Coatings::for_system(&s).unwrap();
    let c = fiber_coupling(&s, 0, &opts(0.1, 0.1), Some(&coat)).unwrap();
    let j = c.jones.unwrap();
    // 出射電場の z 成分（sin θ 程度）を受け側の x・y に射影しない分だけ 1 より小さい（〜NA²/4）
    assert!(
        (j[0][0].norm() - 1.0).abs() < 5e-3 && j[0][1].norm() < 1e-6,
        "{j:?}"
    );
    assert!((j[1][1] - j[0][0]).norm() < 1e-9);
    for e in [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.6, 0.0), C64::new(0.0, 0.8)],
    ] {
        let p = c.eta_pol(e).unwrap();
        assert!((p - c.eta).abs() < 6e-3, "{p} {}", c.eta);
    }
}

#[test]
fn errors() {
    let s = relay(0.0);
    let mut o = opts(0.1, 0.1);
    o.grid = 100;
    assert!(fiber_coupling(&s, 0, &o, None).is_err());
    assert!(fiber_coupling(&s, 3, &opts(0.1, 0.1), None).is_err());
    assert!(fiber_coupling(&s, 0, &opts(0.0, 0.1), None).is_err());
}

/// 系の絞り: 絞り面（物点から 50 mm の理想レンズ）の半径 R で切ると、方向余弦の円 ρ ≤ c（c = sin atan(R/50)）の
/// 外が損失になる。整合したガウスでは system = 1 − e^{−2c²/a²}、η = system²（誤差は格子の刻みの 2 乗）
#[test]
fn stop_clips() {
    let a = 0.1;
    for c in [0.05, 0.1, 0.15] {
        let r = 50.0 * (c / (1.0f64 - c * c).sqrt());
        let t = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nFLOA\nFTYP 0 0 1 1 0 0 0\nYFLN 0\nFWGN 1\n\
WAVM 1 0.6 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 100\n  DIAM {r} 1 0 0 1 \"\"\n\
SURF 2\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        let s = zmx::build(
            &zmx::parse_text(&t).unwrap(),
            &Default::default(),
            &[],
            None,
        )
        .unwrap();
        let want = 1.0 - (-2.0 * c * c / (a * a)).exp();
        for (grid, tol) in [(151, 2e-4), (301, 6e-5)] {
            let mut o = opts(a, a);
            o.grid = grid;
            let k = fiber_coupling(&s, 0, &o, None).unwrap();
            assert!((k.system - want).abs() < tol, "{} {want}", k.system);
            assert!(
                (k.eta - want * want).abs() < 2.0 * tol,
                "{} {}",
                k.eta,
                want * want
            );
            assert!((k.receiver - want).abs() < 2.0 * tol);
        }
        // 絞りを無視すれば 1
        let mut o = opts(a, a);
        o.clip_stop = false;
        let k = fiber_coupling(&s, 0, &o, None).unwrap();
        assert!((k.eta - 1.0).abs() < 5e-5 && (k.system - 1.0).abs() < 1e-12);
    }
}
