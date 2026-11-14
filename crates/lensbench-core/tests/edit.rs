//! 系の編集: 面の挿入・削除・種類と材料の変更、ピックアップとソルブの付け外し、多重構成の表。

use crate::common::root;
use lensbench_core::edit::surf_type_parse;
use lensbench_core::param::{Kind, Param, PickKind, Pickup, SolveKind, ThickSolve};
use lensbench_core::surface::{Aperture, SurfType, Surface};
use lensbench_core::system::{Aiming, ApertureKind, Field, FieldNorm, FieldSel, FieldType};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::collections::HashMap;

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
    for i in 0..7 {
        for j in 0..7 {
            let (x, y) = (i as f64 / 3.0 - 1.0, j as f64 / 3.0 - 1.0);
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// 全視野・全波長の像面での光線（位置・方向・光路長・止まり）
fn rays(s: &System) -> Vec<f64> {
    let (px, py) = grid();
    let mut out = Vec::new();
    for f in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for r in &t.rays {
                out.extend(r.xyz.iter().chain(&r.lmn));
                out.push(r.opl);
                out.push(r.err as f64);
                out.push(r.vig as f64);
            }
        }
    }
    out
}

fn same_bits(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn close(a: &[f64], b: &[f64], tol: f64) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut m: f64 = 0.0;
    for (x, y) in a.iter().zip(b) {
        if x.is_nan() && y.is_nan() {
            continue;
        }
        m = m.max((x - y).abs());
    }
    assert!(m <= tol, "最大差 {m:e} > {tol:e}");
    m
}

fn surf(t: f64) -> Surface {
    Surface {
        t,
        n: Vec::new(),
        ..Default::default()
    }
}

/// 物体（無限遠）・絞り・像面だけの系
fn empty() -> System {
    let mut sf = vec![surf(f64::INFINITY), surf(10.0), surf(0.0)];
    sf[1].stop = true;
    for d in sf.iter_mut() {
        d.n = vec![1.0; 3];
    }
    System::new(
        sf,
        vec![0.4861, 0.5876, 0.6563],
        vec![1.0; 3],
        1,
        vec![Field::xy(0.0, 0.0), Field::xy(0.0, 1.0)],
        FieldType::Angle,
        FieldNorm::Radial,
        (ApertureKind::Epd, 10.0),
        false,
        Aiming::Off,
        HashMap::new(),
    )
    .unwrap()
}

/// 何も変えない編集は、読み込んだ系の追跡を変えない（全部の照合系）
#[test]
fn noop_edit_keeps_bits() {
    let mut n = 0;
    for e in std::fs::read_dir(root().join("tests/refs")).unwrap() {
        let name = e.unwrap().file_name().into_string().unwrap();
        if !root()
            .join("tests/refs")
            .join(&name)
            .join("system.zmx")
            .exists()
        {
            continue;
        }
        let s = load(&name);
        let mut t = s.clone();
        t.edit(|_| Ok(())).unwrap();
        for (j, (a, b)) in s.surfaces.iter().zip(&t.surfaces).enumerate() {
            assert!(
                same_bits(&a.n, &b.n),
                "{name} 面 {j}: 屈折率 {:?} {:?}",
                a.n,
                b.n
            );
        }
        assert!(same_bits(&rays(&s), &rays(&t)), "{name}");
        n += 1;
    }
    assert!(n >= 30, "{n}");
}

/// 空気中に厚さ 0 の平面を入れても追跡は変わらず、消すと元とビット一致
#[test]
fn insert_and_delete_dummy() {
    for name in ["A0", "A3", "B0", "C0"] {
        let s = load(name);
        let r0 = rays(&s);
        // 後ろが空気の面の後に入れる
        let j = (1..s.image())
            .find(|&j| s.surfaces[j].n.iter().all(|&x| x == 1.0) && !s.surfaces[j].mirror)
            .unwrap();
        let mut t = s.clone();
        t.insert_surface(j + 1, surf(0.0)).unwrap();
        assert_eq!(t.surfaces.len(), s.surfaces.len() + 1);
        assert_eq!(t.stop(), s.stop() + usize::from(s.stop() > j));
        close(&r0, &rays(&t), 1e-9);
        let notes = t.delete_surface(j + 1).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!(same_bits(&r0, &rays(&t)), "{name}");
    }
}

/// 何もない系に単レンズを組む: 近軸の焦点距離がレンズの式と合い、厚さのソルブで像面が近軸の焦点に来る
#[test]
fn build_singlet() {
    let mut s = empty();
    // 絞りの後ろに 2 面（前面 R=50、後面 R=-50、厚さ 5、n=1.5 分散なし）
    let mut a = surf(5.0);
    a.c = 1.0 / 50.0;
    a.mat = "1.5,0".into();
    a.semi = 8.0;
    let mut b = surf(40.0);
    b.c = -1.0 / 50.0;
    s.insert_surface(2, a).unwrap();
    s.insert_surface(3, b).unwrap();
    assert_eq!(s.surfaces.len(), 5);
    assert_eq!(s.surfaces[2].n, vec![1.5; 3]);
    assert_eq!(s.surfaces[3].n, vec![1.0; 3]);
    let (n, r1, r2, d) = (1.5_f64, 50.0, -50.0, 5.0);
    let p = (n - 1.0) * (1.0 / r1 - 1.0 / r2 + (n - 1.0) * d / (n * r1 * r2));
    let efl = s.paraxial().efl;
    assert!((efl - 1.0 / p).abs() < 1e-9, "{efl} {}", 1.0 / p);
    // 像面の手前の厚さを近軸のマージナル光線の高さ 0 で解く
    s.add_solve(ThickSolve {
        surf: 3,
        height: 0.0,
        kind: SolveKind::MarginalHeight,
    })
    .unwrap();
    let t = s
        .try_trace(FieldSel::Index(0), &[0.0], &[0.1], Some(1), None, false)
        .unwrap();
    let y = t.rays[0].xyz[1];
    assert!(y.abs() < 1e-3, "{y}");
    // 解いた厚さはパラメータで変えられない、外せば変えられる
    let th = Param::new(3, Kind::Thick);
    assert!(s.with_params(&[(th, 40.0)]).is_err());
    s.remove_solve(3, Kind::Thick).unwrap();
    s.set_param(th, 40.0).unwrap();
    // 材料をガラス名に: 無ければエラー、カタログを足せば読める
    assert!(s.set_material(2, "N-BK7").is_err());
    let miss = s.add_glass_catalogs(&["SCHOTT".into()], &[root().join("tests/glass")]);
    assert!(miss.is_empty(), "{miss:?}");
    s.set_material(2, "N-BK7").unwrap();
    assert!(
        (s.surfaces[2].n[1] - 1.5168).abs() < 1e-3,
        "{:?}",
        s.surfaces[2].n
    );
    // 鏡にすると後ろの媒質は前の媒質（空気）
    s.set_material(3, "MIRROR").unwrap();
    assert!(s.surfaces[3].mirror);
    assert_eq!(s.surfaces[3].n, s.surfaces[2].n);
    s.set_material(3, "").unwrap();
    assert_eq!(s.surfaces[3].n, vec![1.0; 3]);
    // 開口・膜
    s.set_aperture(
        2,
        Aperture::Circular {
            min: 0.0,
            max: 3.0,
            dx: 0.0,
            dy: 0.0,
        },
    )
    .unwrap();
    let t = s
        .try_trace(FieldSel::Index(0), &[0.0], &[1.0], Some(1), None, false)
        .unwrap();
    assert!(t.rays[0].vig != 0 || t.rays[0].err != 0);
    s.set_coating(2, "AR").unwrap();
    assert_eq!(s.surfaces[2].coat, "AR");
}

/// ピックアップを付けると先が元に追従し、元の面を消すと外れて説明が返る
#[test]
fn pickups_follow_and_renumber() {
    let mut s = empty();
    let mut a = surf(5.0);
    a.c = 0.02;
    a.mat = "1.5,0".into();
    s.insert_surface(2, a).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    s.add_pickup(Pickup::new(3, 2, PickKind::Curv, -1.0, 0.0))
        .unwrap();
    assert_eq!(s.surfaces[3].c, -0.02);
    s.set_param(Param::new(2, Kind::Curv), 0.03).unwrap();
    assert_eq!(s.surfaces[3].c, -0.03);
    // 前に面を入れると番号が付け替わる
    s.insert_surface(2, surf(1.0)).unwrap();
    assert_eq!((s.pickups[0].src, s.pickups[0].dst), (3, 4));
    s.set_param(Param::new(3, Kind::Curv), 0.01).unwrap();
    assert_eq!(s.surfaces[4].c, -0.01);
    // 外すと値は残り、もう追従しない
    s.remove_pickup(4, PickKind::Curv).unwrap();
    s.set_param(Param::new(3, Kind::Curv), 0.02).unwrap();
    assert_eq!(s.surfaces[4].c, -0.01);
    // 付け直して元を消す
    s.add_pickup(Pickup::new(4, 3, PickKind::Curv, 1.0, 0.0))
        .unwrap();
    let notes = s.delete_surface(3).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(s.pickups.is_empty());
    // 絞り・物体・像面は消せない
    assert!(s.delete_surface(s.stop()).is_err());
    assert!(s.delete_surface(0).is_err());
    assert!(s.delete_surface(s.image()).is_err());
}

/// 面の前の傾き偏心（before0〜4）をピックアップの先にできる（倍率つき）。先に傾き偏心がなければ作り、
/// 面の後は「前を戻す」になる。パラメータで元を動かすと先も動き、微分の連鎖にも入る
#[test]
fn pickup_before_tilt() {
    let mut s = empty();
    s.insert_surface(2, surf(5.0)).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    // 元に値がないうちは先に作らない
    for i in 0..5u8 {
        s.add_pickup(Pickup::new(3, 2, PickKind::Before(i), -1.0, 0.0))
            .unwrap();
    }
    assert!(s.surfaces[3].before.is_none());
    s.set_param(Param::new(2, Kind::Dx), 0.01).unwrap();
    s.set_param(Param::new(2, Kind::Ty), 0.5).unwrap();
    let b = s.surfaces[3].before.unwrap();
    assert_eq!(b.d, [-0.01, 0.0, 0.0, -0.5, 0.0]);
    assert_eq!(s.surfaces[3].after, lensbench_core::surface::After::Reverse);
    // with_params（元の系は変わらない）
    let t = s
        .with_params(&[
            (Param::new(2, Kind::Dy), 0.02),
            (Param::new(2, Kind::Tz), 1.0),
        ])
        .unwrap();
    assert_eq!(
        t.surfaces[3].before.unwrap().d,
        [-0.01, -0.02, 0.0, -0.5, -1.0]
    );
    assert_eq!(s.surfaces[3].before.unwrap().d[1], 0.0);
    // 先のパラメータはピックアップで決まる
    assert!(s.pickup_of(Param::new(3, Kind::Dx)).is_some());
    // 後の傾き偏心（after）も先にできる（面 4 の後 = 面 2 の前）
    s.insert_surface(4, surf(10.0)).unwrap();
    s.add_pickup(Pickup {
        from: Some(PickKind::Before(0)),
        ..Pickup::new(4, 2, PickKind::After(0), 1.0, 0.0)
    })
    .unwrap();
    s.set_param(Param::new(2, Kind::Dx), 0.03).unwrap();
    assert_eq!(s.surfaces[3].before.unwrap().d[0], -0.03);
    match s.surfaces[4].after {
        lensbench_core::surface::After::Tilt(t) => assert_eq!(t.d[0], 0.03),
        a => panic!("{a:?}"),
    }
}

/// 同じ面の前と後の両方をピックアップの先にしても、前を作るときに後を「前を戻す」にしない（後は後のピックアップで決まる）
#[test]
fn pickup_before_and_after_together() {
    use lensbench_core::surface::{After, Tilt};
    let mut s = empty();
    s.insert_surface(2, surf(5.0)).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    s.surfaces[2].before = Some(Tilt {
        d: [0.0; 5],
        order: 0,
    });
    s.surfaces[2].after = After::Tilt(Tilt {
        d: [0.0; 5],
        order: 1,
    });
    s.add_pickup(Pickup::new(3, 2, PickKind::Before(0), 1.0, 0.0))
        .unwrap();
    s.add_pickup(Pickup::new(3, 2, PickKind::After(0), 1.0, 0.0))
        .unwrap();
    s.set_param(Param::new(2, Kind::Dx), 0.01).unwrap();
    assert_eq!(s.surfaces[3].before.unwrap().d[0], 0.01);
    // 面 2 の後は 0 のまま（値 0 のうちは作らない）
    assert_eq!(s.surfaces[3].after, After::None);
    if let After::Tilt(t) = &mut s.surfaces[2].after {
        t.d[0] = -0.02;
    }
    s.resolve_pickups().unwrap();
    match s.surfaces[3].after {
        After::Tilt(t) => assert_eq!(t.d[0], -0.02),
        a => panic!("{a:?}"),
    }
}

/// 面の種類: 座標ブレークにすると材料・曲率が消え、媒質は前を引き継ぐ。戻すと材料（空気）から作る
#[test]
fn surface_type_change() {
    let mut s = empty();
    let mut a = surf(5.0);
    a.c = 0.02;
    a.mat = "1.5,0".into();
    s.insert_surface(2, a).unwrap();
    s.insert_surface(3, surf(2.0)).unwrap();
    s.set_surface_type(3, surf_type_parse("COORDBRK").unwrap())
        .unwrap();
    assert_eq!(s.surfaces[3].n, vec![1.5; 3]);
    s.set_param(Param::new(3, Kind::Tx), 1.0).unwrap();
    s.set_surface_type(3, SurfType::Standard).unwrap();
    assert_eq!(s.surfaces[3].cb, [0.0; 5]);
    assert_eq!(s.surfaces[3].n, vec![1.0; 3]);
    // 形のデータの要る種類はエラー（.zmx から読む）
    assert!(s.set_surface_type(3, SurfType::ZernikeStd).is_err());
    // 失敗した編集で系は変わらない
    assert_eq!(s.surfaces[3].typ, SurfType::Standard);
    assert!(s.set_surface_type(s.stop(), SurfType::CoordBrk).is_err());
}

/// 多重構成の表: 構成ごとの値を入れた系、面の挿入で行の面番号が付け替わる、消した面の行は外れる
#[test]
fn configs_table() {
    let mut s = empty();
    let mut a = surf(5.0);
    a.c = 0.02;
    a.mat = "1.5,0".into();
    s.insert_surface(2, a).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    let th = Param::new(3, Kind::Thick);
    assert!(s.set_config_row(th, vec![40.0, 45.0]).is_err());
    s.set_config_count(3).unwrap();
    s.set_config_row(th, vec![40.0, 45.0, 50.0]).unwrap();
    s.set_config_row(Param::new(2, Kind::Curv), vec![0.02, 0.02, 0.025])
        .unwrap();
    let c3 = s.config(3).unwrap();
    assert_eq!(c3.surfaces[3].t, 50.0);
    assert_eq!(c3.surfaces[2].c, 0.025);
    assert_eq!(c3.configs().unwrap().active, 3);
    assert_eq!(c3.config(1).unwrap().surfaces[3].t, 40.0);
    assert!(s.config(4).is_err());
    // 面を前に入れると行の面が付け替わる
    s.insert_surface(1, surf(1.0)).unwrap();
    let rows = &s.configs().unwrap().rows;
    assert_eq!(rows[0].param, Param::new(4, Kind::Thick));
    assert_eq!(s.config(2).unwrap().surfaces[4].t, 45.0);
    // 面を消すとその行は外れる
    let notes = s.delete_surface(4).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(s.configs().unwrap().rows.len(), 1);
    // 今の構成が無くなるまで減らすと構成 1 の値が入る
    let mut c3 = s.config(3).unwrap();
    assert_eq!(c3.surfaces[3].c, 0.025);
    c3.set_config_count(2).unwrap();
    assert_eq!(c3.configs().unwrap().active, 1);
    assert_eq!(c3.surfaces[3].c, 0.02);
    // 構成の数を減らす・1 にすると表が消える
    s.set_config_count(2).unwrap();
    assert_eq!(s.configs().unwrap().rows[0].values.len(), 2);
    s.set_config_count(1).unwrap();
    assert!(s.configs().is_none());
}

/// 名前で面の値を変える: ピックアップ・ソルブで決まる値はエラー、値の読み出しと往復
#[test]
fn surface_fields_by_name() {
    use lensbench_core::edit::SurfValue::*;
    let mut s = empty();
    s.insert_surface(2, surf(5.0)).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    s.set_surface_fields(
        2,
        &[
            ("r".into(), Num(50.0)),
            ("material".into(), Text("1.6,0".into())),
            ("comment".into(), Text("L1 front".into())),
            ("semi".into(), Num(6.0)),
        ],
    )
    .unwrap();
    assert_eq!(s.surfaces[2].c, 0.02);
    assert_eq!(s.surfaces[2].n, vec![1.6; 3]);
    assert_eq!(s.find_surface("L1 front").unwrap(), 2);
    s.set_surface_fields(2, &[("dn".into(), Num(0.01))])
        .unwrap();
    assert!((s.surfaces[2].n[0] - 1.61).abs() < 1e-15);
    // 材料を変えても dn は残る
    s.set_surface_fields(2, &[("material".into(), Text("1.5,0".into()))])
        .unwrap();
    assert!((s.surfaces[2].n[0] - 1.51).abs() < 1e-15);
    let f = s.surface_fields(2).unwrap();
    assert!(f.contains(&("material", Text("1.5,0".into()))));
    assert!(s
        .set_surface_fields(2, &[("nope".into(), Num(1.0))])
        .is_err());
    s.add_pickup(Pickup::new(3, 2, PickKind::Curv, -1.0, 0.0))
        .unwrap();
    assert!(s.set_surface_fields(3, &[("c".into(), Num(0.1))]).is_err());
    // dn・semi・面タイプのパラメータもピックアップ先なら拒む
    s.add_pickup(Pickup::new(3, 2, PickKind::Semi, 1.0, 0.0))
        .unwrap();
    assert!(s
        .set_surface_fields(3, &[("semi".into(), Num(4.0))])
        .is_err());
    assert!(s.set_surface_fields(3, &[("dn".into(), Num(0.0))]).is_ok());
    // 種類を変えて座標ブレークの値を入れる
    s.insert_surface(4, surf(0.0)).unwrap();
    s.set_surface_fields(
        4,
        &[
            ("type".into(), Text("COORDBRK".into())),
            ("cb".into(), Nums(vec![0.0, 0.1, 1.0, 0.0, 0.0])),
        ],
    )
    .unwrap();
    assert_eq!(s.surfaces[4].typ, SurfType::CoordBrk);
    assert_eq!(s.surfaces[4].cb[1], 0.1);
}

/// 値が有限にならないピックアップ（無限遠の物体距離から）は付けられない
#[test]
fn pickup_must_be_finite() {
    let mut s = empty();
    let p = Pickup::new(1, 0, PickKind::Thick, 0.0, 2.0);
    assert!(s.add_pickup(p).is_err());
    assert!(s.pickups.is_empty());
}

/// 曲率のソルブ: 別の面の番号も付け替わり、曲率は名前でも変えられない。厚さと曲率のソルブは別に持てる
#[test]
fn curvature_solve_renumber() {
    let mut s = empty();
    let mut a = surf(5.0);
    a.c = 0.02;
    a.mat = "1.5,0".into();
    s.insert_surface(2, a).unwrap();
    s.insert_surface(3, surf(40.0)).unwrap();
    // 面 3 の曲率中心を面 2 の頂点に
    s.add_solve(ThickSolve {
        surf: 3,
        height: 0.0,
        kind: SolveKind::ConcentricSurface(2),
    })
    .unwrap();
    assert!(
        (s.surfaces[3].c - (-1.0 / 5.0)).abs() < 1e-12,
        "{}",
        s.surfaces[3].c
    );
    s.add_solve(ThickSolve {
        surf: 3,
        height: 45.0,
        kind: SolveKind::Position(2),
    })
    .unwrap();
    assert_eq!(s.solves.len(), 2);
    assert!((s.surfaces[3].t - 40.0).abs() < 1e-12);
    use lensbench_core::edit::SurfValue::Num;
    assert!(s.set_surface_fields(3, &[("c".into(), Num(0.1))]).is_err());
    s.insert_surface(1, surf(1.0)).unwrap();
    let k: Vec<_> = s.solves.iter().map(|q| (q.surf, q.kind)).collect();
    assert!(k.contains(&(4, SolveKind::ConcentricSurface(3))), "{k:?}");
    assert!(k.contains(&(4, SolveKind::Position(3))), "{k:?}");
    s.remove_solve(4, Kind::Thick).unwrap();
    assert_eq!(s.solves.len(), 1);
    // 参照先の面を消すとそのソルブは外れる
    let notes = s.delete_surface(3).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(s.solves.is_empty());
}
