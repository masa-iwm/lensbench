//! 定義した視野以外の点のケラレ係数（公開 #226）。OpticStudio 互換（VigInterp::OpticStudio）は OpticStudio 2026 R1.01 を
//! ZOS-API で測った値と同じ（25 系 5010 点。ここはその代表値）。
//! 既定の連続（VigInterp::Continuous）は視野の間で段差がない

use lensbench_core::system::{Field, FieldNorm, FieldSel, VigInterp};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::path::PathBuf;

fn with_fields(fields: &[(f64, f64, [f64; 5])], norm: FieldNorm) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let mut s = zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
    s.fields = fields
        .iter()
        .map(|&(x, y, v)| Field {
            vdx: v[0],
            vdy: v[1],
            vcx: v[2],
            vcy: v[3],
            van: v[4],
            ..Field::xy(x, y)
        })
        .collect();
    s.field_norm = norm;
    s.vig_interp = VigInterp::OpticStudio;
    s.changed().unwrap();
    s
}

fn near(a: [f64; 5], b: [f64; 5]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12)
}

const F2: [f64; 5] = [0.05, 0.10, 0.10, 0.20, 5.0];
const F3: [f64; 5] = [0.10, 0.30, 0.20, 0.50, 12.0];

/// y だけの視野（同じ符号）: r² = X² + Y² の区分線形で補間する。外側・負の側は L1 で最も近い視野
#[test]
fn y_only_fields_interpolate_in_r_squared() {
    for norm in [FieldNorm::Radial, FieldNorm::Rect] {
        let s = with_fields(
            &[(0.0, 0.0, [0.0; 5]), (0.0, 5.0, F2), (0.0, 10.0, F3)],
            norm,
        );
        let v = |hx, hy| s.vig_factors(hx, hy).unwrap();
        // OS の実測: Hy 0.3 → VDY 0.036・VCY 0.072・VAN 1.8、Hy 0.7 → 0.164・0.296・7.24
        let a = v(0.0, 0.3);
        assert!(
            (a[1] - 0.036).abs() < 1e-12
                && (a[3] - 0.072).abs() < 1e-12
                && (a[4] - 1.8).abs() < 1e-12,
            "{a:?}"
        );
        let b = v(0.0, 0.7);
        assert!(
            (b[1] - 0.164).abs() < 1e-12
                && (b[3] - 0.296).abs() < 1e-12
                && (b[4] - 7.24).abs() < 1e-12,
            "{b:?}"
        );
        // 負の Hy は正の Hy の鏡像
        assert!(near(v(0.0, -0.7), b));
        // 外側は L1 で最も近い視野（端の値に止めない）
        assert!(near(v(0.0, 1.2), F3));
        assert!(near(v(0.0, -1.1), [0.0; 5]));
        assert!(near(
            v(1.2, 0.5),
            if norm == FieldNorm::Radial {
                F2
            } else {
                v(0.0, 0.5)
            }
        ));
        if norm == FieldNorm::Radial {
            // 半径で補間するので Hx も効く
            assert!(near(v(0.7, 0.0), b));
        } else {
            // Rectangular で x の最大が 0 なら Hx は効かない
            assert!(near(v(0.7, 0.3), a));
        }
    }
}

/// x と y の両方の視野: 補間せず、物理座標の L1 距離で最も近い視野（同じ距離なら番号の小さい視野）
#[test]
fn xy_fields_take_the_nearest_field() {
    let f3 = [0.2, 0.0, 0.4, 0.0, 0.0];
    let f4 = [0.0, 0.15, 0.0, 0.25, -8.0];
    let s = with_fields(
        &[
            (0.0, 0.0, [0.0; 5]),
            (0.0, 10.0, F3),
            (10.0, 0.0, f3),
            (7.0, 7.0, f4),
        ],
        FieldNorm::Radial,
    );
    let v = |hx, hy| s.vig_factors(hx, hy).unwrap();
    assert!(near(v(0.3, 0.2), [0.0; 5]));
    assert!(near(v(0.0, 0.5), [0.0; 5]));
    assert!(near(v(0.7, 0.2), f3));
    assert!(near(v(0.7, 0.3), f4));
    // 係数つきの系でも、定義した視野以外の点を追える（以前は誤り）
    assert!(s
        .try_trace(
            FieldSel::Norm(0.3, 0.45),
            &[0.0, 1.0],
            &[0.0, 0.0],
            None,
            None,
            false
        )
        .is_ok());
}

/// 既定の連続な補間: 定義した視野で値が一致し、視野の間で段差がない（L1 最近傍の段差の場所をまたいでも連続）
#[test]
fn continuous_is_continuous_and_matches_fields() {
    let f3 = [0.2, 0.0, 0.4, 0.0, 0.0];
    let f4 = [0.0, 0.15, 0.0, 0.25, -8.0];
    let fields = [
        (0.0, 0.0, [0.0; 5]),
        (0.0, 10.0, F3),
        (10.0, 0.0, f3),
        (7.0, 7.0, f4),
    ];
    let mut s = with_fields(&fields, FieldNorm::Radial);
    s.vig_interp = VigInterp::Continuous;
    s.changed().unwrap();
    let r = 10.0;
    for &(x, y, v) in &fields {
        assert!(near(s.vig_factors(x / r, y / r).unwrap(), v));
    }
    // 細かく刻んで隣り合う値の差が小さい（段差がない）
    let n = 400;
    for row in [0.2, 0.5, 0.75, -0.3] {
        let mut prev: Option<[f64; 5]> = None;
        for i in 0..=n {
            let hx = -0.5 + 1.8 * i as f64 / n as f64;
            let v = s.vig_factors(hx, row).unwrap();
            if let Some(p) = prev {
                // 係数は 0.5 以下、VAN は 12° 以下。刻み 0.0045 で段差がなければ差は傾き × 刻み
                let d = (0..4).map(|k| (p[k] - v[k]).abs()).fold(0.0, f64::max);
                let da = (p[4] - v[4]).abs();
                assert!(d < 0.01 && da < 0.3, "hy {row} hx {hx}: {p:?} → {v:?}");
            }
            prev = Some(v);
        }
    }
    // 三角形 (0,0)-(10,0)-(7,7) の重心は 3 視野の平均
    let c = s.vig_factors(17.0 / 30.0, 7.0 / 30.0).unwrap();
    let mean: [f64; 5] = std::array::from_fn(|k| (f3[k] + f4[k]) / 3.0);
    assert!(near(c, mean), "{c:?} {mean:?}");
    // y だけの視野は OpticStudio と同じ区間は同じ値、外は最も外の視野の値
    let mut y = with_fields(
        &[(0.0, 0.0, [0.0; 5]), (0.0, 5.0, F2), (0.0, 10.0, F3)],
        FieldNorm::Radial,
    );
    let os = y.vig_factors(0.0, 0.7).unwrap();
    y.vig_interp = VigInterp::Continuous;
    assert!(near(y.vig_factors(0.0, 0.7).unwrap(), os));
    assert!(near(y.vig_factors(0.0, 1.3).unwrap(), F3));
}

/// 同じ円に 4 点以上が乗る視野（x・y の格子、正方形の 4 隅）でも、連続モードは対角線で飛ばない
#[test]
fn continuous_on_cocircular_grids() {
    let v = |x: f64, y: f64| {
        [
            0.02 * x + 0.01 * y,
            0.03 * y,
            0.01 * x * y / 10.0,
            0.0,
            x - y,
        ]
    };
    let mut grid = Vec::new();
    for x in [-10.0, 0.0, 10.0] {
        for y in [-10.0, 0.0, 10.0] {
            grid.push((x, y, v(x, y)));
        }
    }
    let square: Vec<_> = [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)]
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            (
                x,
                y,
                [
                    0.1 * i as f64,
                    0.05 * (i % 2) as f64,
                    0.0,
                    0.0,
                    3.0 * i as f64,
                ],
            )
        })
        .collect();
    for (fields, norm) in [(grid, FieldNorm::Rect), (square, FieldNorm::Rect)] {
        let mut s = with_fields(&fields, norm);
        s.vig_interp = VigInterp::Continuous;
        s.changed().unwrap();
        for &(x, y, f) in &fields {
            assert!(near(s.vig_factors(x / 10.0, y / 10.0).unwrap(), f));
        }
        // 縦・横・斜めに刻み 0.004 で走査して段差がない
        let n = 500;
        for (dir, off) in [
            ((1.0, 0.0), 0.37),
            ((0.0, 1.0), 0.61),
            ((1.0, 1.0), 0.0),
            ((1.0, -1.0), 0.13),
        ] {
            let mut prev: Option<[f64; 5]> = None;
            for i in 0..=n {
                let t = -1.0 + 2.0 * i as f64 / n as f64;
                let (hx, hy) = if dir.1 == 0.0 {
                    (t, off)
                } else if dir.0 == 0.0 {
                    (off, t)
                } else {
                    (t, dir.1 * t + off)
                };
                let v = s.vig_factors(hx, hy).unwrap();
                if let Some(p) = prev {
                    let d = (0..4).map(|k| (p[k] - v[k]).abs()).fold(0.0, f64::max);
                    let da = (p[4] - v[4]).abs();
                    assert!(d < 0.005 && da < 0.2, "{dir:?} t {t}: {p:?} → {v:?}");
                }
                prev = Some(v);
            }
        }
    }
}

/// 輪の上に並ぶ視野（cos・sin で作るので、同じ円に乗るかが丸め誤差で揺れる）でも、穴があかず段差がない
#[test]
fn continuous_on_rings() {
    use std::f64::consts::PI;
    let v = |x: f64, y: f64| {
        [
            0.02 * x + 0.01 * y + 0.2,
            0.015 * y + 0.2,
            0.0,
            0.0,
            x - 0.5 * y,
        ]
    };
    let ring = |n: usize, r: f64, ph: f64| -> Vec<(f64, f64)> {
        (0..n)
            .map(|i| {
                let a = 2.0 * PI * i as f64 / n as f64 + ph;
                (r * a.cos(), r * a.sin())
            })
            .collect()
    };
    let one: Vec<(f64, f64)> = ring(8, 10.0, 0.0);
    let mut two = ring(12, 10.0, 0.0);
    two.extend(ring(6, 5.0, 0.3));
    for pts in [one, two] {
        let fields: Vec<_> = pts.iter().map(|&(x, y)| (x, y, v(x, y))).collect();
        let mut s = with_fields(&fields, FieldNorm::Radial);
        s.vig_interp = VigInterp::Continuous;
        s.changed().unwrap();
        // 中は平面の値（三角形の線形で厳密に再現）、外も縁で連続
        let n = 600;
        for (row, horiz) in [
            (0.0, true),
            (0.37, true),
            (-0.81, true),
            (0.5, false),
            (-0.2, false),
        ] {
            let mut prev: Option<[f64; 5]> = None;
            for i in 0..=n {
                let t = -1.3 + 2.6 * i as f64 / n as f64;
                let (hx, hy) = if horiz { (t, row) } else { (row, t) };
                let f = s.vig_factors(hx, hy).unwrap();
                if hx.hypot(hy) < 0.9 {
                    let w = v(10.0 * hx, 10.0 * hy);
                    let e = f
                        .iter()
                        .zip(&w)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f64::max);
                    assert!(e < 1e-9, "({hx}, {hy}): {f:?} {w:?}");
                }
                if let Some(p) = prev {
                    let d = (0..4).map(|k| (p[k] - f[k]).abs()).fold(0.0, f64::max);
                    let da = (p[4] - f[4]).abs();
                    assert!(d < 0.005 && da < 0.2, "({hx}, {hy}): {p:?} → {f:?}");
                }
                prev = Some(f);
            }
        }
    }
}
