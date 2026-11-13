//! 着地点の微分 d(x, y, l, m)/dpと、検出面のずらし dzを中心差分と照合する。
//! 差分側は、パラメータを変えた系で同じ開始位置・方向（物体の座標系）の光線を追う。
//! dz つきの dOPL/dp は、開始を固定した光路長の差分から、着地点の動きの分 n (l dx + m dy) を引いたものと比べる
//! （端点を固定した微分 = 開始を固定した微分 − ∂V/∂E · dE/dp）。
#![allow(clippy::type_complexity, clippy::needless_range_loop)]

use crate::common::root;
use lensbench_core::math::*;
use lensbench_core::param::{Kind, Param};
use lensbench_core::surface::SurfType;
use lensbench_core::system::{Launch, Trace};
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

fn ps(s: &System, names: &[&str]) -> Vec<Param> {
    names.iter().map(|n| s.param(n).unwrap()).collect()
}

fn step(p: Param) -> f64 {
    match p.kind {
        Kind::Curv => 2e-6,
        Kind::Conic => 2e-3,
        Kind::Asph(i) => 1e-5 / 10f64.powi(2 * i as i32 + 2),
        Kind::Dn | Kind::Disp => 1e-5,
        Kind::Radius => 2e-3,
        _ => 1e-4,
    }
}

/// 物体の座標系での開始位置・方向で、系 s を面 last まで追う（dz があれば最後の面からずらす）
fn launch(s: &System, p: &[V3], k: &[V3], w: usize, last: usize, dz: Option<f64>) -> Trace {
    let f = s.object_frame();
    let pg: Vec<V3> = p.iter().map(|&x| add(f.o, mv(&f.r, x))).collect();
    let kg: Vec<V3> = k.iter().map(|&x| mv(&f.r, x)).collect();
    let mut t = s.trace_launch(&Launch::Rays { p: &pg, k: &kg }, w, Some(last), false);
    if let Some(z) = dz {
        s.shift_end(&mut t, w, z).unwrap();
    }
    t
}

/// 系 s の視野 fi の数本の光線で、着地点の微分と dz つき dOPL/dp を差分と比べる。最大の相対誤差を返す
fn check(
    label: &str,
    s: &System,
    fi: usize,
    w: usize,
    last: usize,
    dz: Option<f64>,
    params: &[Param],
) -> f64 {
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
    let np = params.len();
    let lg = s.land_grad(&t, w, params, dz).unwrap();
    let mut og = s.opl_grad(&t, w, params).unwrap();
    if let Some(z) = dz {
        s.opl_grad_shift(&t, w, params, z, &mut og).unwrap();
    }
    // 開始位置・方向を物体の座標系に
    let f = s.object_frame();
    let p0: Vec<V3> = t.p0.iter().map(|&x| mtv(&f.r, sub(x, f.o))).collect();
    let k0: Vec<V3> = t.k0.iter().map(|&x| mtv(&f.r, x)).collect();
    // ずらしの前後で、開始を固定した追跡が元の追跡を再現する
    let base = launch(s, &p0, &k0, w, last, dz);
    let mut tb = t.clone();
    if let Some(z) = dz {
        s.shift_end(&mut tb, w, z).unwrap();
    }
    for i in 0..px.len() {
        assert_eq!(t.rays[i].err, 0, "{label}: 光線 {i} が止まる");
        for c in 0..3 {
            assert!(
                (base.rays[i].xyz[c] - tb.rays[i].xyz[c]).abs() < 1e-10,
                "{label}: 再現"
            );
        }
    }
    let nlast = {
        let d = &s.surfaces[last];
        if d.mirror || d.typ == SurfType::CoordBrk {
            s.index_before(last, w)
        } else {
            d.n[w]
        }
    };
    let mut worst: f64 = 0.0;
    let mut bad = 0;
    for (q, &p) in params.iter().enumerate() {
        // 5 点の中心差分（刻みは 3 通りから光線・量ごとに最もよく合うもの）
        let at = |d: f64| -> Trace {
            match p.kind {
                Kind::ObjX | Kind::ObjY => {
                    let c = if p.kind == Kind::ObjX { 0 } else { 1 };
                    let pp: Vec<V3> = p0
                        .iter()
                        .map(|&x| {
                            let mut x = x;
                            x[c] += d;
                            x
                        })
                        .collect();
                    launch(s, &pp, &k0, w, last, dz)
                }
                _ => {
                    let v = s.get_param(p).unwrap();
                    let s2 = s.with_params(&[(p, v + d)]).unwrap();
                    launch(&s2, &p0, &k0, w, last, dz)
                }
            }
        };
        let hs = [0.25, 1.0, 4.0].map(|m| step(p) * m);
        let tr: Vec<[Trace; 4]> = hs
            .iter()
            .map(|&h| [at(h), at(-h), at(2.0 * h), at(-2.0 * h)])
            .collect();
        let fd = |val: &dyn Fn(&Trace) -> f64, hi: usize| -> f64 {
            let [a, b, c, d] = &tr[hi];
            let h = hs[hi];
            (8.0 * (val(a) - val(b)) - (val(c) - val(d))) / (12.0 * h)
        };
        for i in 0..px.len() {
            let r = &tb.rays[i];
            // x, y, l, m と、開始を固定した光路長
            let vals: [Box<dyn Fn(&Trace) -> f64>; 5] = [
                Box::new(move |t: &Trace| t.rays[i].xyz[0]),
                Box::new(move |t: &Trace| t.rays[i].xyz[1]),
                Box::new(move |t: &Trace| t.rays[i].lmn[0]),
                Box::new(move |t: &Trace| t.rays[i].lmn[1]),
                Box::new(move |t: &Trace| t.rays[i].opl),
            ];
            let g4 = &lg[(i * np + q) * 4..(i * np + q) * 4 + 4];
            // 端点の動き dE = (dx, dy, z_x dx + z_y dy)（dz なしで最後の面が曲面のとき。dz ありは平面）
            let kz = match dz {
                Some(_) => [r.lmn[0], r.lmn[1]],
                None => {
                    let d = &s.surfaces[last];
                    let x = r.xyz;
                    let (_, z1) =
                        lensbench_core::trace::sag(d.c, d.k, &d.asph, x[0] * x[0] + x[1] * x[1]);
                    [
                        r.lmn[0] + r.lmn[2] * 2.0 * z1 * x[0],
                        r.lmn[1] + r.lmn[2] * 2.0 * z1 * x[1],
                    ]
                }
            };
            let ana = [
                g4[0],
                g4[1],
                g4[2],
                g4[3],
                og[i * np + q] + nlast * (kz[0] * g4[0] + kz[1] * g4[1]),
            ];
            for (c, val) in vals.iter().enumerate() {
                let a = ana[c];
                let mut best = (f64::INFINITY, 0.0, 0.0);
                for hi in 0..hs.len() {
                    let d = fd(val.as_ref(), hi);
                    if (a - d).abs() < best.0 {
                        best = ((a - d).abs(), d, hs[hi]);
                    }
                }
                let (diff, d, h) = best;
                let err = diff / a.abs().max(d.abs()).max(1e-300);
                // 合格: 相対 1e-7、または差分の丸め誤差以下（値の差 1e-13 程度 / 刻み）
                if !(err < 1e-7 || diff * h < 1e-12) {
                    println!(
                        "NG {label} 光線 {i} {} 成分 {c}: 解析 {a:e} 差分 {d:e}（相対 {err:e}、刻み {h:e}）",
                        p.name()
                    );
                    bad += 1;
                }
                if err.is_finite() {
                    worst = worst.max(err.min(1.0));
                }
            }
        }
    }
    println!(
        "{label}: {np} パラメータ × {} 本: 外れ {bad}（最大の相対誤差 {worst:.1e}）",
        px.len()
    );
    assert_eq!(bad, 0, "{label}");
    worst
}

/// 傾き偏心・鏡・非球面の系（無限遠物体）
#[test]
fn tilted_folded_infinite() {
    let names = [
        "1.c", "1.k", "1.a4", "1.a6", "2.c", "2.r", "1.t", "3.t", "7.t", "9.t", "2.dx", "2.dy",
        "2.tx", "2.ty", "3.tz", "3.dx", "4.dx", "4.ty", "6.tx", "8.tx", "8.ty", "8.dx", "10.tx",
        "1.dn", "1.disp", "3.dn", "7.dn", "8.t", "8.c",
    ];
    for name in ["C0", "C1"] {
        let s = load(name);
        let p = ps(&s, &names);
        check(name, &s, 1, s.primary, s.image(), None, &p);
        check(&format!("{name} dz"), &s, 2, 0, s.image(), Some(0.37), &p);
    }
}

/// 有限物体: 物点の移動・物体距離も。途中の面（曲面）で止め、dz も
#[test]
fn finite_object() {
    let s = load("B0");
    // 像面までの厚さはソルブ: 微分はソルブで動く厚さの分も含む（差分側は with_params で解き直す）
    assert_eq!(s.solves.len(), 1);
    let mut names: Vec<String> = vec!["objx".into(), "objy".into(), "0.t".into(), "0.dn".into()];
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
    check("B0 面 5", &s, 1, s.primary, 5, None, &p);
    check("B0 面 4（曲面）", &s, 1, s.primary, 4, None, &p);
    check("B0 面 4 dz", &s, 1, s.primary, 4, Some(-1.5), &p);
    check("B0 像面 dz", &s, 1, s.primary, s.image(), Some(0.2), &p);
}

/// 非球面の多い系とトロイダル面
#[test]
fn aspheric_and_toroidal() {
    let s = load("E0");
    let p: Vec<Param> = (1..s.image())
        .flat_map(|j| {
            ["c", "k", "a4", "t", "dy", "tx", "dn"]
                .iter()
                // 厚さのソルブで決まる厚さは変えられない（その厚さの変化は他のパラメータの微分に入る）
                .filter(|&&k| !(k == "t" && s.solves.iter().any(|q| q.surf == j)))
                .filter_map(|k| s.param(&format!("{j}.{k}")).ok())
                .collect::<Vec<_>>()
        })
        .collect();
    check("E0", &s, 1, s.primary, s.image(), Some(0.05), &p);

    let mut s = load("A0");
    let d = &mut s.surfaces[1];
    d.typ = SurfType::Toroidal;
    d.cx = d.c * 0.7;
    d.asph = vec![1e-5, -2e-7];
    // 面を直接書き換えたので、像面の厚さのソルブ（面 7）を解き直す（with_params の系と揃える）
    s.solve_thickness().unwrap();
    let p = ps(&s, &["1.t", "1.dy", "1.tx", "1.ty", "2.c", "1.dn", "3.dx"]);
    check("A0+トロイダル", &s, 1, s.primary, s.image(), None, &p);
    // トロイダル面の形のパラメータは未対応
    let t = s.trace_spec(
        s.object_spec(0.0, 0.0).unwrap(),
        &[0.1],
        &[0.2],
        s.primary,
        None,
        None,
        true,
        None,
    );
    assert!(s
        .land_grad(&t, s.primary, &[Param::new(1, Kind::Curv)], None)
        .is_err());
}

/// Zernike・Grid Sag の面
#[test]
fn freeform_surfaces() {
    use lensbench_core::freeform::{GridSag, ZernSag};
    use std::sync::Arc;
    let mut s = load("A0");
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
    // 面を直接書き換えたので、像面の厚さのソルブを解き直す
    s.solve_thickness().unwrap();
    let p = ps(
        &s,
        &[
            "1.c", "1.k", "1.z4", "1.z9", "3.z5", "3.z8", "3.z22", "3.c", "3.a4", "3.t", "6.c",
            "6.a4", "6.dx", "6.t", "2.dn",
        ],
    );
    check("A0+Zernike・Grid", &s, 1, s.primary, s.image(), None, &p);
    check("A0+Zernike・Grid dz", &s, 3, 0, s.image(), Some(0.3), &p);
}

/// 記録なしの追跡はエラー、dz のずらしは光路長と大域の位置も直す
#[test]
fn shift_end_and_errors() {
    let s = load("A0");
    let spec = s.object_spec(0.0, 1.0).unwrap();
    let t = s.trace_spec(spec, &[0.3], &[-0.2], s.primary, None, None, false, None);
    assert!(s.land_grad(&t, s.primary, &ps(&s, &["1.c"]), None).is_err());
    let mut t2 = t.clone();
    let tt = s.shift_end(&mut t2, s.primary, 1.25).unwrap();
    let r0 = &t.rays[0];
    let r1 = &t2.rays[0];
    assert!((r1.xyz[2] - 1.25).abs() < 1e-15);
    assert!((tt[0] - (1.25 - r0.xyz[2]) / r0.lmn[2]).abs() < 1e-15);
    let n = s.surfaces[s.image()].n[s.primary];
    assert!((r1.opl - r0.opl - n * tt[0]).abs() < 1e-12);
    let dp = sub(r1.p, r0.p);
    let dk = scale(r0.k, tt[0]);
    for c in 0..3 {
        assert!((dp[c] - dk[c]).abs() < 1e-12);
    }
    assert!(s.shift_end(&mut t2, s.primary, f64::NAN).is_err());
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
    assert!(!s.pickups.is_empty());
    let p = ps(
        &s,
        &[
            "1.t", "2.k", "2.a4", "3.dx", "3.ty", "2.t", "1.c", "1.r", "5.c", "4.dn",
        ],
    );
    check("ピックアップ", &s, 1, s.primary, 7, None, &p);
    check("ピックアップ dz", &s, 1, s.primary, 7, Some(0.4), &p);
    // 最後の面の形がピックアップ先（面 5 の a4 ← 面 2 の a4）: dz つき dOPL/dp の形の項も追従する
    check(
        "ピックアップ 面 5 で dz",
        &s,
        1,
        s.primary,
        5,
        Some(-0.3),
        &p,
    );
}
