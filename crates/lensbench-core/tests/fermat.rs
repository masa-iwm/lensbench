//! dOPL/dpを中心差分と照合する。差分側は、パラメータを変えた系で始点と最後の面での交点（局所 x・y）を
//! 同じにした光線を解いて（の解き）光路長を求める。5 点の中心差分（刻みは 4 通りから最もよく合うもの）。
//!
//! 合格: 相対 1e-9 以内、または差のずれが差分の丸め誤差以内（|解析 − 差分|·刻み < 1e-12 mm）。
//! 微分が小さい量（光線の高さが小さいときの円錐係数など）は、差分の側の丸め誤差で相対 1e-9 に届かない。

use crate::common::root;
use lensbench_core::param::{Kind, Param};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::{FieldSel, ObjSpec};
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

/// 始点 spec・最後の面の交点 target を固定した光線の光路長（瞳座標 pupil から解く）
fn opl_fixed(
    s: &System,
    spec: ObjSpec,
    pupil: [f64; 2],
    last: usize,
    target: [f64; 2],
    w: usize,
) -> f64 {
    let mut pr = RayProblem::new(
        Unknown::Pupil,
        Residual::Position { surf: last, target },
        spec,
        w,
    );
    pr.pupil = pupil;
    pr.tol = 1e-11;
    // 大きい刻みでは光線が通らないこともある（その刻みは使わない）
    let Ok(sol) = s.solve_ray(&pr) else {
        return f64::NAN;
    };
    // 残った交点のずれの 1 次の分を除く（最後の面は平面）
    assert_eq!(s.surfaces[last].c, 0.0);
    let r = sol.trace.rec.last().unwrap();
    let (x, k) = (r.xyz[0], r.lmn[0]);
    let n = s.index_before(last, w);
    sol.trace.rays[0].opl - n * (k[0] * (x[0] - target[0]) + k[1] * (x[1] - target[1]))
}

fn step(p: Param) -> f64 {
    match p.kind {
        Kind::Curv => 2e-5,
        Kind::Conic => 2e-2,
        Kind::Asph(i) => 1e-4 / 10f64.powi(2 * i as i32 + 2),
        Kind::Dn | Kind::Disp => 1e-4,
        Kind::Radius => 2e-2,
        Kind::Zern(_) => 1e-5,
        _ => 1e-3,
    }
}

/// 系 s の視野 fi・波長 w の数本の光線を面 last（平面）まで追い、params の dOPL/dp を差分と比べる。
/// 最後の面が像面（結像している）だと交点を固定した光線が瞳座標で決まらないので、その前の面で止める
fn check(label: &str, s: &System, fi: usize, w: usize, last: usize, params: &[Param]) -> f64 {
    let px = [0.0, 0.6, -0.3, 0.2, -0.7];
    let py = [0.0, 0.1, 0.5, -0.8, -0.4];
    let (hx, hy) = s.hxhy(fi);
    let spec = s.object_spec(hx, hy).unwrap();
    let t = s.trace_spec(
        spec,
        &px,
        &py,
        w,
        Some([0.0; 5]),
        Some(last),
        true,
        Some((hx, hy)),
    );
    let g = s.opl_grad(&t, w, params).unwrap();
    let rl = t.rec.last().unwrap();
    let np = params.len();
    let mut worst: f64 = 0.0;
    let mut bad = 0;
    let (mut n_all, mut n_rel, mut n_round) = (0, 0, 0);
    for i in 0..px.len() {
        assert_eq!(t.rays[i].err, 0, "{label}: 光線 {i} が止まる");
        let x = rl.xyz[i];
        let target = [x[0], x[1]];
        let opl0 = t.rays[i].opl;
        // 解きで元の光線を再現できる
        let o = opl_fixed(s, spec, [px[i], py[i]], last, target, w);
        assert!((o - opl0).abs() < 1e-11, "{label}: 再現 {o} {opl0}");
        for (q, &p) in params.iter().enumerate() {
            let a = g[i * np + q];
            // 刻みを 3 通り試し、最もよく合うものを使う（差分の打ち切り誤差と丸め誤差の兼ね合いは量ごとに違う）
            let mut best = (f64::INFINITY, 0.0, 0.0);
            for m in [0.25, 1.0, 4.0, 16.0] {
                let h = step(p) * m;
                let at = |d: f64| -> f64 {
                    match p.kind {
                        Kind::ObjX | Kind::ObjY => {
                            let ObjSpec::Point(mut pt) = spec else {
                                unreachable!()
                            };
                            pt[if p.kind == Kind::ObjX { 0 } else { 1 }] += d;
                            opl_fixed(s, ObjSpec::Point(pt), [px[i], py[i]], last, target, w)
                        }
                        _ => {
                            let v = s.get_param(p).unwrap();
                            let s2 = s.with_params(&[(p, v + d)]).unwrap();
                            opl_fixed(&s2, spec, [px[i], py[i]], last, target, w)
                        }
                    }
                };
                let fd = (8.0 * (at(h) - at(-h)) - (at(2.0 * h) - at(-2.0 * h))) / (12.0 * h);
                if (a - fd).abs() < best.0 {
                    best = ((a - fd).abs(), fd, h);
                }
            }
            let (diff, fd, h) = best;
            let err = diff / a.abs().max(fd.abs()).max(1e-300);
            n_all += 1;
            if err < 1e-9 {
                n_rel += 1;
            } else if diff * h < 1e-12 {
                // 差分の丸め誤差以下（光路長の差が 1e-12 mm。光路長 〜100 mm の 1e-14 相対で、追跡の丸め誤差の大きさ）
                n_round += 1;
            } else {
                println!(
                    "NG {label} 光線 {i} {}: 解析 {a:e} 差分 {fd:e}（相対 {err:e}、刻み {h:e}）",
                    p.name()
                );
                bad += 1;
            }
            worst = worst.max(err);
        }
    }
    println!(
        "{label}: {np} パラメータ × {} 本 = {n_all}: 相対 1e-9 以内 {n_rel}、差分の丸め誤差以内 {n_round}、外れ {bad}（最大の相対誤差 {worst:.1e}）",
        px.len()
    );
    assert_eq!(bad, 0, "{label}");
    worst
}

fn ps(s: &System, names: &[&str]) -> Vec<Param> {
    names.iter().map(|n| s.param(n).unwrap()).collect()
}

/// 傾き偏心・折返し（鏡）・非球面の系（無限遠物体、エイミングなし・あり）
#[test]
fn tilted_folded_infinite() {
    let names = [
        "1.c", "1.k", "1.a4", "1.a6", "2.c", "2.r", "1.t", "3.t", "7.t", "9.t", "2.dx", "2.dy",
        "2.tx", "2.ty", "3.tz", "3.dx", "4.dx", "4.ty", "6.tx", "8.tx", "8.ty", "8.dx", "10.tx",
        "1.dn", "1.disp", "3.dn", "7.dn", "8.t",
    ];
    for name in ["C0", "C1"] {
        let s = load(name);
        let p = ps(&s, &names);
        check(name, &s, 1, s.primary, s.image(), &p);
        check(&format!("{name} 波長 0"), &s, 2, 0, s.image(), &p);
    }
}

/// Zernike・Grid Sag の面: 形の係数・Zernike の項の係数・基準の面・間隔
#[test]
fn freeform_surfaces() {
    use lensbench_core::freeform::{GridSag, ZernSag};
    use lensbench_core::surface::SurfType;
    use std::sync::Arc;
    let mut s = load("A0");
    // 像面を焦点から十分に離す（交点を固定した光線が瞳座標で決まるように）。像のソルブは外す
    s.solves.clear();
    s.surfaces[7].t = 10.0;
    let d = &mut s.surfaces[3];
    d.typ = SurfType::ZernikeStd;
    d.zern = Some(Arc::new(ZernSag::new(
        false,
        6.0,
        (1..=37).map(|i| 1e-4 * (i as f64 * 0.7).sin()).collect(),
        true,
    )));
    let d = &mut s.surfaces[1];
    d.typ = SurfType::ZernikeFringe;
    d.k = -0.4;
    d.zern = Some(Arc::new(ZernSag::new(
        true,
        9.0,
        (1..=37).map(|i| 1e-4 * (i as f64 * 1.3).cos()).collect(),
        true,
    )));
    let (nx, ny) = (21, 21);
    let d = &mut s.surfaces[6];
    d.typ = SurfType::GridSag;
    d.grid = Some(Arc::new(GridSag {
        nx,
        ny,
        dx: 0.8,
        dy: 0.8,
        xdec: 0.0,
        ydec: 0.1,
        z: (0..nx * ny)
            .map(|k| {
                let (x, y) = ((k % nx) as f64 * 0.8 - 8.0, (k / nx) as f64 * 0.8 - 7.9);
                [
                    1e-3 * (0.3 * x + 0.2 * y).sin(),
                    3e-4 * (0.3 * x + 0.2 * y).cos(),
                    2e-4 * (0.3 * x + 0.2 * y).cos(),
                    -6e-5 * (0.3 * x + 0.2 * y).sin(),
                ]
            })
            .collect(),
        nodata: vec![false; nx * ny],
        linear: false,
    }));
    let p = ps(
        &s,
        &[
            "1.c", "1.k", "1.z4", "1.z9", "1.z12", "3.z1", "3.z5", "3.z8", "3.z22", "3.z37", "3.c",
            "3.a4", "3.t", "6.c", "6.k", "6.a4", "6.dx", "6.t",
        ],
    );
    check("A0+Zernike・Grid", &s, 1, s.primary, s.image(), &p);
    check("A0+Zernike・Grid 視野 3", &s, 3, 0, s.image(), &p);
}

/// 有限物体: 物点の移動・物体距離も
#[test]
fn finite_object() {
    let s = load("B0");
    let mut names: Vec<String> = vec!["objx".into(), "objy".into(), "0.t".into(), "0.dn".into()];
    // 絞り（面 5、平面）まで
    for j in 1..5 {
        names.push(format!("{j}.t"));
        names.push(format!("{j}.dy"));
        names.push(format!("{j}.tx"));
        if s.param(&format!("{j}.c")).is_ok() {
            names.push(format!("{j}.c"));
        }
        if s.param(&format!("{j}.dn")).is_ok() {
            names.push(format!("{j}.dn"));
        }
    }
    let n: Vec<&str> = names.iter().map(|x| x.as_str()).collect();
    let p = ps(&s, &n);
    check("B0", &s, 1, s.primary, 5, &p);
    // 傾いた系を有限物体にしたもの
    let c = load("C0");
    let c = c.with_params(&[(c.param("0.t").unwrap(), 150.0)]).unwrap();
    let p = ps(
        &c,
        &[
            "objx", "objy", "0.t", "1.c", "2.tx", "8.tx", "8.dy", "3.t", "1.dn", "0.dn",
        ],
    );
    check("C0 有限", &c, 1, c.primary, c.image(), &p);
}

/// trace_grad は trace と同じ光線を返す
#[test]
fn trace_grad_matches_trace() {
    let s = load("C1");
    let px = [0.1, -0.4];
    let py = [0.3, 0.2];
    let p = ps(&s, &["2.c", "8.tx"]);
    let (t, g) = s
        .trace_grad(FieldSel::Index(1), &px, &py, None, None, &p)
        .unwrap();
    let t2 = s.trace(FieldSel::Index(1), &px, &py, None, None, false);
    for i in 0..2 {
        assert_eq!(t.rays[i].opl.to_bits(), t2.rays[i].opl.to_bits());
    }
    assert_eq!(g.len(), 4);
    assert!(g.iter().all(|v| v.is_finite()));
}

/// ピックアップ元のパラメータ: ピックアップ先が一緒に動いた分を含む（差分側も with_params で先が追従する）
#[test]
fn pickup_sources() {
    let head = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 1\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";
    let t = format!(
        "{head}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE EVENASPH\n  CURV -0.01\n  CONI -0.5\n  PARM 1 0\n  PARM 2 1e-6\n  DISZ 3\n\
SURF 3\n  TYPE COORDBRK\n  PARM 1 0.2\n  PARM 4 1.5\n  DISZ 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.01 4 1 -1 0 \"\"\n  PKUP 2 -1 0\n  PZUP 1 1.5 0.2 0\n  DISZ 0\n  GLAS ___BLANK 1 0 1.6 0 0 0 0 0 0\n\
SURF 5\n  TYPE EVENASPH\n  CURV -0.015\n  PPAR 2 2 -1 0 0\n  PZUP 4 0.5 1 0\n  DISZ 0\n\
SURF 6\n  TYPE COORDBRK\n  PPAR 1 3 -1 0 0\n  PPAR 4 3 -1 0 0\n  DISZ 20\n\
SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 15\n\
SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let raw = zmx::parse_text(&t).unwrap();
    let s = zmx::build(&raw, &Default::default(), &[], None).unwrap();
    // 1.t → 4.t → 5.t と連なる
    let p = ps(
        &s,
        &[
            "1.t", "2.k", "2.a4", "3.dx", "3.ty", "2.t", "1.c", "1.r", "5.c",
        ],
    );
    check("ピックアップ", &s, 1, s.primary, 7, &p);
    // 連なりの分が入っている（ピックアップを外した系とは違う）
    let mut free = s.clone();
    free.pickups.clear();
    let tr = s.trace_spec(
        s.object_spec(0.0, 1.0).unwrap(),
        &[0.3],
        &[0.2],
        0,
        Some([0.0; 5]),
        Some(7),
        true,
        Some((0.0, 1.0)),
    );
    let a = s.opl_grad(&tr, 0, &p[..1]).unwrap()[0];
    let b = free.opl_grad(&tr, 0, &p[..1]).unwrap()[0];
    assert!((a - b).abs() > 1e-3, "{a} {b}");
}
