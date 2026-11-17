//! 形のパラメータ（c・R・k・asph・Zernike の項）の微分を、形ごとに本当の矢高・光路長の中心差分と照合する。
//! asph の意味は形ごとに違う（Odd Asphere の β、Tilted の tan、Irregular の偏心・傾き・Zs・Za・Zc・角度）。
//! 形に効かない値（Tilted の c・k、ほかの型の面に残った Zernike の項など）はパラメータにできないことも確かめる。
//! 曲率のソルブがトロイダル面の y² の項を近軸の追跡と同じに扱うこともここで見る。

use crate::common::root;
use lensbench_core::freeform::ZernSag;
use lensbench_core::param::{Kind, Param};
use lensbench_core::surface::SurfType;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::sync::Arc;

/// 半直径を今の値に固定する（Irregular の正規化半径は半直径なので、自動の半直径だとパラメータを変えた系で
/// 形の正規化も変わる。その分は微分に入れていない）
fn fix_semi(mut s: System) -> System {
    s.edit_raw(|p| {
        for d in &mut p.surfaces {
            d.semi_auto = false;
        }
        Ok(())
    })
    .unwrap();
    s
}

/// tests/surftypes_os の系（面 2 が試す面）
fn surftypes(name: &str) -> System {
    fix_semi(
        zmx::load_with(
            &root()
                .join("tests/surftypes_os")
                .join(name)
                .join("system.zmx"),
            &LoadOptions {
                cfg: 1,
                ..Default::default()
            },
        )
        .unwrap(),
    )
}

/// tests/refs の A0 に、偶数次非球面（面 2）・Zernike Fringe（面 1）・Zernike Standard（面 3）・Grid Sag なしの
/// 標準面に残った Zernike の項（面 4）を置いた系
fn a0_mixed() -> System {
    let mut s = fix_semi(
        zmx::load_with(
            &root().join("tests/refs/A0/system.zmx"),
            &LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root().join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap(),
    );
    s.edit_raw(|p| {
        p.solves.clear();
        Ok(())
    })
    .unwrap();
    let img = s.image();
    s.edit_raw(|p| {
        p.surfaces[img - 1].t += 10.0;
        Ok(())
    })
    .unwrap();
    let zern = |fringe: bool, norm: f64, a: f64| {
        Some(Arc::new(ZernSag::new(
            fringe,
            norm,
            (1..=37).map(|i| 1e-4 * (i as f64 * a).sin()).collect(),
            true,
        )))
    };
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[1];
    d.typ = SurfType::ZernikeFringe;
    d.k = -0.4;
    d.zern = zern(true, 9.0, 1.3);
    let s = rx.build_raw().unwrap();
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[2];
    d.typ = SurfType::EvenAsph;
    d.k = 0.3;
    d.asph = vec![1e-5, -2e-7, 3e-10];
    let s = rx.build_raw().unwrap();
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[3];
    d.typ = SurfType::ZernikeStd;
    d.zern = zern(false, 6.0, 0.7);
    let s = rx.build_raw().unwrap();
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[4];
    d.typ = SurfType::Standard;
    d.asph.clear();
    d.zern = zern(false, 6.0, 0.4);
    rx.build_raw().unwrap()
}

/// A0 の面 3 を surftypes_os の系 name の面 2（Tilted・Odd Asphere・Irregular）の形にした系（像面は焦点から離す。
/// 交点を固定した光線が瞳座標で決まるように）
fn a0_with(name: &str) -> System {
    let src = surftypes(name).surfaces[2].clone();
    let mut s = fix_semi(
        zmx::load_with(
            &root().join("tests/refs/A0/system.zmx"),
            &LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root().join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap(),
    );
    s.edit_raw(|p| {
        p.solves.clear();
        Ok(())
    })
    .unwrap();
    let img = s.image();
    s.edit_raw(|p| {
        p.surfaces[img - 1].t += 10.0;
        Ok(())
    })
    .unwrap();
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[3];
    d.typ = src.typ;
    d.c = src.c;
    d.k = src.k;
    d.asph = src.asph;
    rx.build_raw().unwrap()
}

/// asph が偶数次の係数でない面の刻み（fermat・landing の差分と共通）
pub(crate) fn asph_step(s: &System, p: Param) -> Option<f64> {
    let Kind::Asph(i) = p.kind else {
        return None;
    };
    match s.surfaces.get(p.surf)?.typ {
        SurfType::TiltSurf => Some(1e-4),
        SurfType::OddAsph => Some(1e-4 / 10f64.powi(i as i32 + 1)),
        // 偏心 [mm]・傾き [deg]、Zs・Za・Zc [mm]、角度 [deg]
        SurfType::Irregular => Some(match i {
            0..=3 => 1e-3,
            7 => 1e-2,
            _ => 1e-4,
        }),
        _ => None,
    }
}

fn step(s: &System, p: Param) -> f64 {
    asph_step(s, p).unwrap_or(match p.kind {
        Kind::Curv => 2e-5,
        Kind::Conic => 2e-2,
        Kind::Radius => 2e-2,
        Kind::Asph(i) => 1e-4 / 10f64.powi(2 * i as i32 + 2),
        _ => 1e-5,
    })
}

const KINDS: [Kind; 15] = [
    Kind::Curv,
    Kind::Radius,
    Kind::Conic,
    Kind::Asph(0),
    Kind::Asph(1),
    Kind::Asph(2),
    Kind::Asph(3),
    Kind::Asph(4),
    Kind::Asph(5),
    Kind::Asph(6),
    Kind::Asph(7),
    Kind::Zern(1),
    Kind::Zern(4),
    Kind::Zern(8),
    Kind::Zern(12),
];

/// 面 j の値を、パラメータ kind の値として直接 d だけ変えた系（パラメータ・系の検査を通さない。形を見るだけ）
fn nudge(s: &System, j: usize, kind: Kind, d: f64) -> lensbench_core::Prescription {
    let mut rx = s.prescription().clone();
    let f = &mut rx.surfaces[j];
    match kind {
        Kind::Curv | Kind::Radius => f.c += d,
        Kind::Conic => f.k += d,
        Kind::Asph(i) => {
            let i = i as usize;
            if f.asph.len() <= i {
                f.asph.resize(i + 1, 0.0);
            }
            f.asph[i] += d;
        }
        Kind::Zern(n) => {
            let mut z = f
                .zern
                .as_deref()
                .cloned()
                .unwrap_or_else(|| ZernSag::new(false, 5.0, vec![0.0; 37], true));
            z.coef[n as usize - 1] += d;
            z.rebuild();
            f.zern = Some(Arc::new(z));
        }
        _ => unreachable!(),
    }
    rx
}

/// 形のパラメータの矢高・勾配の微分（Shape::dsag）が、パラメータを変えた系の本当の矢高（Shape::sag）の中心差分と
/// 合う。パラメータにできない値は、変えても矢高が変わらない
#[test]
fn sag_derivatives_follow_the_real_shape() {
    let mut cases: Vec<(String, System, usize)> = ["T1", "O1", "O3", "O5", "I4", "I7"]
        .iter()
        .map(|n| (n.to_string(), surftypes(n), 2))
        .collect();
    let a = a0_mixed();
    for j in 1..=4 {
        cases.push((format!("A0 面 {j}"), a.clone(), j));
    }
    let mut n_checked = 0;
    let mut n_rejected = 0;
    for (label, s, j) in &cases {
        let j = *j;
        let semi = s.surfaces[j].semi;
        let pts: Vec<(f64, f64)> = [(0.31, 0.12), (-0.5, 0.4), (0.07, -0.66), (0.6, 0.35)]
            .iter()
            .map(|&(x, y)| (x * semi, y * semi))
            .collect();
        for kind in KINDS {
            let p = Param::new(j, kind);
            let shape = s.surfaces[j].shape();
            // 平面の曲率半径は無限大（0 と書く）で、差分がとれない
            if kind == Kind::Radius && s.surfaces[j].c == 0.0 {
                continue;
            }
            match s.check_param(p) {
                Ok(()) => {
                    for &(x, y) in &pts {
                        let a = shape
                            .dsag(kind, x, y)
                            .unwrap_or_else(|| panic!("{label} {}: 微分がない", p.name()));
                        let v = s.get_param(p).unwrap();
                        let mut best = [f64::INFINITY; 3];
                        let mut ok = [false; 3];
                        for m in [0.25, 1.0, 4.0, 16.0] {
                            let h = step(s, p) * m;
                            let at = |d: f64| {
                                let s2 = s.with_params(&[(p, v + d)]).unwrap();
                                let r = s2.surfaces[j].shape().sag(x, y);
                                [r.0, r.1, r.2]
                            };
                            let (p1, m1, p2, m2) = (at(h), at(-h), at(2.0 * h), at(-2.0 * h));
                            for c in 0..3 {
                                let fd = (8.0 * (p1[c] - m1[c]) - (p2[c] - m2[c])) / (12.0 * h);
                                let diff = (a[c] - fd).abs();
                                let err = diff / a[c].abs().max(fd.abs()).max(1e-300);
                                best[c] = best[c].min(err);
                                // 相対 1e-8、または差が差分の丸め誤差（矢高の差 1e-14 mm）以下
                                ok[c] |= err < 1e-8 || diff * h < 1e-14;
                            }
                        }
                        assert!(
                            ok.iter().all(|&o| o),
                            "{label} {} ({x}, {y}): 解析 {a:?}、差分との相対誤差 {best:?}",
                            p.name()
                        );
                        n_checked += 1;
                    }
                }
                Err(_) => {
                    assert!(
                        shape.dsag(kind, 0.1, 0.2).is_none(),
                        "{label} {}: 断るパラメータに微分がある",
                        p.name()
                    );
                    let s2 = nudge(s, j, kind, 1e-3);
                    for &(x, y) in &pts {
                        let (a, b) = (shape.sag(x, y), s2.surfaces[j].shape().sag(x, y));
                        assert_eq!(
                            [a.0, a.1, a.2].map(f64::to_bits),
                            [b.0, b.1, b.2].map(f64::to_bits),
                            "{label} {}: 断ったパラメータで形が変わる",
                            p.name()
                        );
                    }
                    n_rejected += 1;
                }
            }
        }
    }
    println!("矢高の微分: {n_checked} 点、断ったパラメータ {n_rejected}");
    assert!(n_checked > 300 && n_rejected > 40);
}

/// 形に効かない値はパラメータにできず、その理由を言う
#[test]
fn values_that_do_not_change_the_shape_are_rejected() {
    let t = surftypes("T1");
    for spec in ["2.c", "2.r", "2.k", "2.a6"] {
        let p = Param::new(2, t.param(spec).map(|p| p.kind).unwrap_or(Kind::Asph(2)));
        let e = t
            .check_param(p)
            .unwrap_err()
            .message()
            .unwrap_or_default()
            .to_string();
        assert!(e.contains("doesn't change the shape"), "{spec}: {e}");
    }
    assert!(t.check_param(Param::new(2, Kind::Asph(1))).is_ok());
    let a = a0_mixed();
    let e = a
        .check_param(Param::new(4, Kind::Zern(4)))
        .unwrap_err()
        .to_string();
    assert!(e.contains("doesn't change the shape"), "{e}");
    assert!(a.check_param(Param::new(3, Kind::Zern(4))).is_ok());
    let i7 = surftypes("I7");
    assert!(i7.check_param(Param::new(2, Kind::Asph(7))).is_ok());
}

/// 形のパラメータごとに、本当に変えた系の光路長・着地点の差分と合う（Odd Asphere・Tilted・Irregular）
#[test]
fn opl_and_landing_derivatives_on_every_shape() {
    for name in ["T1", "O3", "I4", "I7"] {
        let s = a0_with(name);
        let p: Vec<Param> = KINDS
            .iter()
            .map(|&k| Param::new(3, k))
            .filter(|&p| s.check_param(p).is_ok())
            .collect();
        assert!(!p.is_empty());
        crate::fermat::check(name, &s, 1, s.primary, s.image(), &p);
        crate::landing::check(name, &s, 1, s.primary, s.image(), None, &p);
    }
}

/// トロイダル面の y² の項がある面の曲率のソルブ（周辺光線の角度）が、近軸の追跡で目標に丸め誤差まで届く
#[test]
fn toroid_curvature_solve_meets_its_target() {
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 2\n\
SURF 2\n  TYPE TOROIDAL\n  CURV 0.01 2 -0.05 0 0 \"\"\n  PARM 1 40\n  PARM 2 4e-3\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.2 0 0 0 0 0\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 30\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = zmx::parse_text(txt).unwrap();
    let s = zmx::build(&raw, &Default::default(), &[], None).unwrap();
    let um = &s.paraxial().marginal.1;
    let d = &s.surfaces[2];
    assert_eq!(d.asph.first().copied(), Some(4e-3));
    // 以前の規則（y² の項を引かない c = 目標の近軸の曲率）の系
    let mut old = s.clone();
    old.edit_raw(|p| {
        p.solves.clear();
        p.surfaces[2].c = d.shape().paraxial_curv();
        Ok(())
    })
    .unwrap();
    let u_old = old.paraxial().marginal.1[2];
    println!(
        "トロイダル面の曲率のソルブ: c = {}（以前の規則 {}）、面 2 の後の u = {:e}（以前の規則 {:e}、目標 −0.05）",
        d.c, old.surfaces[2].c, um[2], u_old
    );
    assert!((um[2] + 0.05).abs() < 1e-15, "u = {}", um[2]);
    assert!((u_old + 0.05).abs() > 1e-3, "以前の規則 u = {u_old}");
}

/// 曲率のソルブは標準面・偶数次非球面・トロイダル面だけ。Odd Asphere（paraxial_extra に 2β₂ が入る）は
/// 解かずに断るので、値は変わらない
#[test]
fn odd_asphere_curvature_solve_is_rejected() {
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 2\n\
SURF 2\n  TYPE ODDASPHE\n  CURV 0.01 2 -0.05 0 0 \"\"\n  PARM 1 0\n  PARM 2 4e-3\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.2 0 0 0 0 0\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 30\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = zmx::parse_text(txt).unwrap();
    let e = zmx::build(&raw, &Default::default(), &[], None).unwrap_err();
    assert!(e.to_string().contains("curvature solve"), "{e}");
}
