//! ザイデル収差係数（seidel::seidel）を平行平板・二次曲面の式と比べる

use lensbench_core::seidel::{longitudinal, seidel};
use lensbench_core::system::System;
use lensbench_formats::zmx;

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
YFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n";

fn sys(body: &str) -> System {
    let raw = zmx::parse_text(&format!("{HEAD}{body}")).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn close(a: f64, b: f64, what: &str) {
    assert!(
        (a - b).abs() < 1e-12 * (1.0 + b.abs()),
        "{what}: {a} != {b}"
    );
}

/// 収束光の中の平行平板（厚さ d、屈折率 n）: S_I = −d u⁴ (n² − 1)/n³（u は平板の前の傾き）
#[test]
fn plate_spherical() {
    let (d, n, l): (f64, f64, f64) = (10.0, 1.5, 100.0);
    let s = sys(&format!(
        "SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {l}\nSURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {d}\n  GLAS ___BLANK 1 0 {n} 0 0 0 0 0 0\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    ));
    let u: f64 = 5.0 / l;
    let sd = seidel(&s, 0).unwrap();
    close(
        sd.at(0)[0],
        -d * u.powi(4) * (n * n - 1.0) / n.powi(3),
        "S_I",
    );
    close(sd.at(0)[3], 0.0, "平面の S_IV");
}

/// 二次曲面の球面収差: 円錐係数 k の分だけ S_I が k c³ y⁴ (n′ − n) 増える
#[test]
fn conic_spherical() {
    let (c, n, k): (f64, f64, f64) = (0.02, 1.5, -0.6);
    let body = |k: f64| {
        format!(
            "SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\nSURF 1\n  STOP\n  TYPE STANDARD\n  CURV {c}\n  CONI {k}\n  DISZ 100\n  GLAS ___BLANK 1 0 {n} 0 0 0 0 0 0\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        )
    };
    let s0 = sys(&body(0.0));
    let s1 = sys(&body(k));
    let y: f64 = 5.0;
    let d = seidel(&s1, 0).unwrap().at(1)[0] - seidel(&s0, 0).unwrap().at(1)[0];
    close(d, k * c.powi(3) * y.powi(4) * (n - 1.0), "ΔS_I");
    // 波の数: W040 = S_I/8λ
    let sd = seidel(&s1, 0).unwrap();
    close(sd.waves(0)[0], sd.at(0)[0] / (8.0 * 0.5876e-3), "W040");
}

/// ミラーのある系は誤り
#[test]
fn mirror_rejected() {
    let s = sys("SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\nSURF 1\n  STOP\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -50\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n");
    assert!(seidel(&s, 0).is_err());
}

/// 曲率の中心に物点がある面（周辺光線が面に垂直に入る A = 0）: S_I・S_II は 0、S_V も有限
#[test]
fn concentric_surface() {
    let l: f64 = 100.0;
    // 視野 2 つ（0°・5°）、主光線が 0 でないように
    let head = HEAD
        .replace("FTYP 0 0 1 1", "FTYP 0 0 2 1")
        .replace("YFLN 0\nFWGN 1", "YFLN 0 5\nFWGN 1 1");
    let raw = zmx::parse_text(&format!(
        "{head}SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {l}\nSURF 1\n  STOP\n  TYPE STANDARD\n  CURV {}\n  DISZ 10\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        -1.0 / l
    ))
    .unwrap();
    let s = zmx::build(&raw, &Default::default(), &[], None).unwrap();
    let v = seidel(&s, 0).unwrap().at(1);
    for (i, x) in v[..2].iter().enumerate() {
        assert!(x.abs() < 1e-15, "S_{} = {x}", i + 1);
    }
    assert!(v[4].is_finite() && v[3] != 0.0, "{v:?}");
}

/// 実光線の縦収差は像面の頂点から測る: 像面を曲げても（光線は同じ直線なので）同じ値
#[test]
fn longitudinal_curved_image() {
    let body = |ci: f64| {
        format!(
            "SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\nSURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 95\nSURF 3\n  TYPE STANDARD\n  CURV {ci}\n  DISZ 0\n"
        )
    };
    let flat = longitudinal(&sys(&body(0.0)), 0, 1.0).unwrap();
    let curved = longitudinal(&sys(&body(-0.02)), 0, 1.0).unwrap();
    close(curved, flat, "LONA");
    assert!(flat.abs() > 1e-3, "{flat}");
}
