//! 曲率のソルブと残りの厚さのソルブ、別の列・半直径・面の後の傾き偏心のピックアップ。
//! .zmx の書き方と読込で解き直すことは OpticStudio 2026 R1.01 で実測した（非公開側の照合データ 123 ケースで一致を確認）。
//! ここでは定義どおりに解けること、保存値が古くても解き直すこと、書き出し・読み戻し・パラメータの変更を確かめる。

use lensbench_core::param::{PickKind, SolveKind};
use lensbench_core::surface::After;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 14 20\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

const G1: &str = "  GLAS ___BLANK 1 0 1.6204 60.3 0 0 0 0 0\n";
const G2: &str = "  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n";

/// 三枚玉（絞りは面 5）。lines[k] は面 k に足す行（CURV・DISZ の行を置き換えるなら curv[k]・disz[k]）
fn triplet(curv: &[(usize, &str)], extra: &[(usize, &str)]) -> String {
    let base: [(f64, &str, &str); 7] = [
        (1.0 / 23.6, "3.5", G1),
        (-1.0 / 360.0, "5.8", ""),
        (-1.0 / 23.4, "1.2", G2),
        (1.0 / 21.5, "2.3", ""),
        (0.0, "2.6", ""),
        (1.0 / 88.0, "3.2", G1),
        (-1.0 / 19.1, "45.4", ""),
    ];
    let mut t = HEAD.to_string();
    for (i, (c, d, g)) in base.iter().enumerate() {
        let k = i + 1;
        t += &format!("SURF {k}\n");
        if k == 5 {
            t += "  STOP\n";
        }
        t += "  TYPE STANDARD\n";
        match curv.iter().find(|x| x.0 == k) {
            Some((_, l)) => t += &format!("  {l}\n"),
            None => t += &format!("  CURV {c:e}\n"),
        }
        t += &format!("  DISZ {d}\n{g}  DIAM 5 1 0 0 1 \"\"\n");
        for (_, l) in extra.iter().filter(|x| x.0 == k) {
            t += &format!("  {l}\n");
        }
    }
    t += "SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    t
}

fn build(txt: &str) -> System {
    let raw = zmx::parse_text(txt).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn bits(s: &System) -> Vec<u64> {
    let mut v = Vec::new();
    for f in 0..s.fields.len() {
        let t = s.trace(
            FieldSel::Index(f),
            &[0.0, 0.3, -0.5],
            &[0.0, 0.6, 0.2],
            None,
            None,
            false,
        );
        for r in &t.rays {
            // 極端な解（強い曲率など）では通らない光線もある: 通るかどうかも比べる
            v.push(r.err as u64);
            if r.err == 0 {
                v.extend([r.xyz[0], r.xyz[1], r.lmn[0], r.lmn[1]].map(f64::to_bits));
            }
        }
    }
    v
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

/// 面 6・7 に曲率のソルブを置いた系が定義どおりに解け、保存値（0.5）が古くても同じ系になる
#[test]
fn curvature_solves_meet_their_definition() {
    let fmax = 20f64.to_radians().tan();
    let cases: [(usize, &str, SolveKind); 9] = [
        (6, "2 -0.1", SolveKind::MarginalAngle),
        (7, "3 -0.1", SolveKind::ChiefAngle),
        (6, "5 0", SolveKind::MarginalNormal),
        (6, "6 0", SolveKind::ChiefNormal),
        (6, "7 0", SolveKind::Aplanatic),
        (7, "8 0.02", SolveKind::ElementPower),
        (6, "9 3", SolveKind::ConcentricSurface(3)),
        (7, "10 6", SolveKind::ConcentricRadius(6)),
        (6, "11 4.5", SolveKind::FNumber),
    ];
    for (k, code, kind) in cases {
        let line = format!("CURV 0.5 {code} 0 0 \"\"");
        let s = build(&triplet(&[(k, &line)], &[]));
        assert_eq!(s.solves.iter().filter(|q| q.surf == k).count(), 1);
        assert_eq!(s.solves.iter().find(|q| q.surf == k).unwrap().kind, kind);
        let par = s.paraxial();
        let (ym, um) = &par.marginal;
        let (yc, uc) = &par.chief;
        let d = &s.surfaces[k];
        let n = s.index_before(k, 0);
        let n2 = d.n[0];
        let ok = match kind {
            SolveKind::MarginalAngle => close(um[k], -0.1, 1e-13),
            SolveKind::ChiefAngle => close(uc[k] * fmax, -0.1, 1e-13),
            SolveKind::MarginalNormal => close(um[k - 1] + ym[k] * d.c, 0.0, 1e-13),
            SolveKind::ChiefNormal => close(uc[k - 1] + yc[k] * d.c, 0.0, 1e-13),
            SolveKind::Aplanatic => close(um[k] / n2, um[k - 1] / n, 1e-13),
            SolveKind::ElementPower => {
                let a = &s.surfaces[k - 1];
                let ng = a.n[0];
                let (fa, fb) = (a.c * (ng - 1.0), d.c * (1.0 - ng));
                close(fa + fb - a.t / ng * fa * fb, 0.02, 1e-13)
            }
            // 面 6 から面 3 の頂点まで −(2.3 + 2.6 + 1.2)
            SolveKind::ConcentricSurface(_) => close(1.0 / d.c, -6.1, 1e-13),
            SolveKind::ConcentricRadius(_) => close(1.0 / d.c, -3.2 + 88.0, 1e-13),
            SolveKind::FNumber => close(um[k], -1.0 / 9.0, 1e-13),
            _ => unreachable!(),
        };
        assert!(ok, "{kind:?}: c = {}", d.c);
        // 解いた値を保存値にしても同じ（解き直しで値を変えない）、書き出して読み戻してもソルブが残る
        let s2 = build(&triplet(
            &[(k, &format!("CURV {:e} {code} 0 0 \"\"", d.c))],
            &[],
        ));
        assert_eq!(s2.surfaces[k].c, d.c, "{kind:?}");
        let b = build(&zmx_write::write(&s).unwrap());
        assert_eq!(b.solves, s.solves, "{kind:?}");
        assert_eq!(bits(&b), bits(&s), "{kind:?}");
    }
}

/// 厚さのソルブの残り（主光線の高さ・瞳の位置・コバ厚・位置・補償・曲率中心・実光線の高さ）
#[test]
fn thickness_solves_meet_their_definition() {
    let fmax = 20f64.to_radians().tan();
    let cases: [(usize, &str, SolveKind); 7] = [
        (6, "CHZH 0.5 0", SolveKind::ChiefHeight),
        (7, "TPUP 0 0", SolveKind::PupilPosition),
        (6, "EDGE 1.5 4", SolveKind::Edge(4.0)),
        (6, "TOLE 5 10", SolveKind::Position(5)),
        (4, "TCOM 2 8", SolveKind::Compensator(2)),
        (6, "TCOC 6 0", SolveKind::CenterOfCurvature(6)),
        (7, "MAZH 0.2 0.7", SolveKind::MarginalHeightZone(0.7)),
    ];
    for (k, line, kind) in cases {
        let s = build(&triplet(&[], &[(k, line)]));
        assert_eq!(s.solves.iter().find(|q| q.surf == k).unwrap().kind, kind);
        let par = s.paraxial();
        let t = s.surfaces[k].t;
        let ok = match kind {
            SolveKind::ChiefHeight => close(par.chief.0[k + 1] * fmax, 0.5, 1e-13),
            SolveKind::PupilPosition => par.chief.0[k + 1].abs() < 1e-12,
            SolveKind::Edge(r) => {
                let (a, b) = (&s.surfaces[k], &s.surfaces[k + 1]);
                let sag = |c: f64| c * r * r / (1.0 + (1.0 - c * c * r * r).sqrt());
                close(t + sag(b.c) - sag(a.c), 1.5, 1e-13)
            }
            SolveKind::Position(_) => close(s.surfaces[5].t + t, 10.0, 1e-13),
            SolveKind::Compensator(_) => close(s.surfaces[2].t + t, 8.0, 1e-13),
            SolveKind::CenterOfCurvature(_) => close(t, 88.0, 1e-13),
            SolveKind::MarginalHeightZone(z) => {
                let r = s.trace(
                    FieldSel::Norm(0.0, 0.0),
                    &[0.0],
                    &[z],
                    None,
                    Some(k + 1),
                    false,
                );
                // 像面（平面）での実光線の高さ
                close(r.rays[0].xyz[1], 0.2, 1e-12)
            }
            _ => unreachable!(),
        };
        assert!(ok, "{kind:?}: t = {t}");
        let b = build(&zmx_write::write(&s).unwrap());
        assert_eq!(b.solves, s.solves, "{kind:?}");
        assert_eq!(bits(&b), bits(&s), "{kind:?}");
    }
}

/// パラメータを変えるとソルブを解き直し、同じ値を書いた .zmx を読んだ系と同じになる。ソルブで決まる値は変えられない
#[test]
fn solves_follow_parameter_changes() {
    let txt = triplet(&[(6, "CURV 0.5 11 4.5 0 0 \"\"")], &[(7, "MAZH 0 0")]);
    let s = build(&txt);
    let c = s.with_params(&[(s.param("3.t").unwrap(), 1.6)]).unwrap();
    assert_ne!(c.surfaces[6].c, s.surfaces[6].c);
    let fresh = build(&txt.replace("  DISZ 1.2\n", "  DISZ 1.6\n"));
    assert_eq!(c.surfaces[6].c, fresh.surfaces[6].c);
    assert_eq!(c.surfaces[7].t, fresh.surfaces[7].t);
    assert!(close(c.paraxial().marginal.1[6], -1.0 / 9.0, 1e-13));
    let e = s.with_params(&[(s.param("6.r").unwrap(), 50.0)]);
    assert!(e.unwrap_err().to_string().contains("ソルブ"));
}

/// 別の列からのピックアップ（OpticStudio の実測どおり: 曲率は c = 倍率 / 元の値、ほかは倍率 × 元の値 + オフセットで
/// 元が半径の列なら半径の値）と半直径のピックアップ
#[test]
fn pickups_from_other_columns() {
    let txt = triplet(
        &[(6, "CURV 0.3 4 2 2 2 \"\"")],
        &[
            (4, "PZUP 1 2 0.5 1"),
            (3, "CONI 0"),
            (3, "PKUP 1 2 1"),
            (2, "DIAM 1 2 1 2 2 \"\""),
        ],
    );
    // DIAM の行が 2 つ（後のものが勝つ）にならないよう、面 2 の既定の行を消す
    let txt = txt.replacen("  DISZ 5.8\n  DIAM 5 1 0 0 1 \"\"\n", "  DISZ 5.8\n", 1);
    let s = build(&txt);
    let sf = &s.surfaces;
    assert_eq!(sf[6].c, 2.0 / 5.8);
    assert_eq!(sf[4].t, 2.0 * 23.6 + 0.5);
    assert_eq!(sf[3].k, 2.0 * 23.6);
    assert_eq!(sf[2].semi, 2.0 * 3.5);
    let from = |d: usize, w: PickKind| {
        s.pickups
            .iter()
            .find(|q| q.dst == d && q.what == w)
            .unwrap()
            .from
    };
    assert_eq!(from(6, PickKind::Curv), Some(PickKind::Thick));
    assert_eq!(from(4, PickKind::Thick), Some(PickKind::Radius));
    assert_eq!(from(2, PickKind::Semi), Some(PickKind::Thick));
    // 元を変えると追従し、同じ値で読み直した系と同じ
    let c = s
        .with_params(&[
            (s.param("2.t").unwrap(), 6.0),
            (s.param("1.r").unwrap(), 25.0),
        ])
        .unwrap();
    assert_eq!(c.surfaces[6].c, 2.0 / 6.0);
    assert!(close(c.surfaces[4].t, 50.5, 1e-14));
    assert!(close(c.surfaces[3].k, 50.0, 1e-14));
    // 書き出して読み戻してもつながりが残る
    let b = build(&zmx_write::write(&s).unwrap());
    assert_eq!(b.pickups, s.pickups);
    assert_eq!(bits(&b), bits(&s));
}

/// 面の後の傾き偏心のピックアップ（SCBD 2 のモード 2·(先 − 元) + 1、+2 は反転）は元の面の前の値と順序を取る
#[test]
fn after_tilt_pickups() {
    let src = "SCBD 1 0 0 0.1 0.2 1 -0.5 2";
    let src_after = "SCBD 2 1 0 -0.3 0.05 -0.4 0.6 -1.5";
    for (mode, rev) in [(7, false), (8, true)] {
        let line = format!("SCBD 2 0 {mode} 9 9 9 9 9");
        let s = build(&triplet(&[], &[(3, src), (3, src_after), (6, &line)]));
        let After::Tilt(t) = s.surfaces[6].after else {
            panic!("{:?}", s.surfaces[6].after)
        };
        let b = [0.1, 0.2, 1.0, -0.5, 2.0];
        if rev {
            assert_eq!(t.d, b.map(|v: f64| -v));
            assert_eq!(t.order, 1);
        } else {
            assert_eq!(t.d, b);
            assert_eq!(t.order, 0);
        }
        // 元の面の前の傾きを変えると追従する
        let c = s.with_params(&[(s.param("3.tx").unwrap(), 1.5)]).unwrap();
        let After::Tilt(t2) = c.surfaces[6].after else {
            panic!()
        };
        assert_eq!(t2.d[2], if rev { -1.5 } else { 1.5 });
        let w = build(&zmx_write::write(&s).unwrap());
        assert_eq!(w.pickups, s.pickups);
        assert_eq!(bits(&w), bits(&s));
    }
    // モード 1: この面の前と同じ
    let s = build(&triplet(
        &[],
        &[
            (6, "SCBD 1 1 0 0.05 -0.02 0.3 0.1 0"),
            (6, "SCBD 2 0 1 0 0 0 0 0"),
        ],
    ));
    let After::Tilt(t) = s.surfaces[6].after else {
        panic!()
    };
    assert_eq!(t.d, [0.05, -0.02, 0.3, 0.1, 0.0]);
    assert_eq!(t.order, 1);
}

/// パラメータを変えて戻した系、ファイルの値（解かれる値）だけ違う .zmx を読んだ系が、元の系とビット一致する。
/// ソルブは前の値によらず同じ出発点から解く
#[test]
fn solve_independent_of_previous_value() {
    type Lines<'a> = &'a [(usize, &'a str)];
    let cases: [(Lines, Lines); 5] = [
        (&[], &[(7, "MAZH 0 0")]),
        (&[], &[(7, "MAZH 0 1")]),
        (&[], &[(7, "CHZH 0.5 0")]),
        (&[], &[(6, "TPUP 0 0")]),
        (&[(6, "CURV 0.0 2 -0.1 0 0 \"\"")], &[(7, "MAZH 0 0")]),
    ];
    for (curv, extra) in cases {
        let a = build(&triplet(curv, extra));
        // 解かれる厚さのファイルの値を変えたもの
        let mut txt = triplet(curv, extra);
        if extra[0].0 == 7 {
            txt = txt.replace("DISZ 45.4", "DISZ 44.9");
        } else {
            txt = txt.replace("DISZ 3.2", "DISZ 3.1");
        }
        if !curv.is_empty() {
            txt = txt.replace("CURV 0.0 2", "CURV 0.01 2");
        }
        let b = build(&txt);
        let p = a.param("3.t").unwrap();
        let t0 = a.get_param(p).unwrap();
        let c = a
            .with_params(&[(p, t0 + 0.4)])
            .unwrap()
            .with_params(&[(p, t0)])
            .unwrap();
        for (name, s) in [("ファイルの値", &b), ("変えて戻す", &c)] {
            for k in 0..a.surfaces.len() {
                assert_eq!(
                    s.surfaces[k].t.to_bits(),
                    a.surfaces[k].t.to_bits(),
                    "{extra:?} {name}: 面 {k} の厚さ {} {}",
                    s.surfaces[k].t,
                    a.surfaces[k].t
                );
                assert_eq!(s.surfaces[k].c.to_bits(), a.surfaces[k].c.to_bits());
            }
            assert_eq!(bits(s), bits(&a), "{extra:?} {name}");
        }
    }
}

/// ZPL マクロの曲率のソルブ（CURV c 12 … "マクロ名"）は解けないので、保存された曲率で固定して読む
#[test]
fn zpl_macro_curvature_solve_keeps_saved_value() {
    let s = build(&triplet(&[(6, "CURV 0.0125 12 0 0 0 \"MYMACRO\"")], &[]));
    assert_eq!(s.surfaces[6].c, 0.0125);
    assert!(s.solves.iter().all(|q| q.surf != 6));
}
