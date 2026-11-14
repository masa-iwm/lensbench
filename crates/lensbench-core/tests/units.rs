//! 玉・群の単位のパラメータ: 面ごとに同じ変更を与えた系と一致し、dOPL/dp・着地点の微分が中心差分と一致する。

use crate::common::root;
use lensbench_core::param::{Kind, Param};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::{FieldSel, ObjSpec};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{zmx_write, System};

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

fn text(t: &str) -> System {
    let raw = zmx::parse_text(t).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
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

/// 全視野・全波長の、像面での光線（位置・方向・光路長・止まり）
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

fn same_bits(a: &System, b: &System, what: &str) {
    let (x, y) = (rays(a), rays(b));
    assert_eq!(x.len(), y.len());
    let diff = x
        .iter()
        .zip(&y)
        .filter(|(p, q)| p.to_bits() != q.to_bits())
        .count();
    assert_eq!(diff, 0, "{what}: {diff} 個の値が違う");
}

/// 丸め誤差の範囲で一致（最大の差を返す）
fn close(a: &System, b: &System, tol: f64, what: &str) -> f64 {
    let (x, y) = (rays(a), rays(b));
    assert_eq!(x.len(), y.len());
    let mut worst: f64 = 0.0;
    for (p, q) in x.iter().zip(&y) {
        if p.is_nan() && q.is_nan() {
            continue;
        }
        let d = (p - q).abs();
        assert!(d <= tol * (1.0 + p.abs()), "{what}: {p} と {q}");
        worst = worst.max(d);
    }
    println!("{what}: 最大の差 {worst:.1e}");
    worst
}

#[test]
fn lenses_are_found_on_load() {
    let s = load("A0");
    let names: Vec<&str> = s.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["lens1", "lens2", "lens3"]);
    assert_eq!(s.units[0].spans, vec![[1, 2]]);
    let p = s.param("lens2.tx").unwrap();
    assert_eq!(s.param_name(p), "lens2.tx");
    assert_eq!(s.get_param(p).unwrap(), 0.0);
    assert!(s.param("lens9.tx").is_err());
    assert!(s.param("lens1.c").is_err());
}

/// shift・dn・disp は面ごとのパラメータの組合せそのもの（ビット一致）
#[test]
fn linear_unit_params_equal_per_surface_changes() {
    for name in ["A0", "B0", "C0"] {
        let s = load(name);
        let mut g = s.clone();
        // 群: 玉 2 と玉 3（面 3〜7、絞りを含む）
        let lens2 = g.units[1].spans[0];
        let last = g.units.iter().find(|u| u.name == "lens3").unwrap().spans[0];
        g.define_unit("G", &[[lens2[0], last[1]]], [0.0; 3])
            .unwrap();
        let p = |t: &str| g.param(t).unwrap();
        let (a, b) = (lens2[0], last[1]);
        type Changes = Vec<(Param, f64)>;
        let cases: Vec<(Changes, Changes)> = vec![
            (
                vec![(p("G.shift"), 0.37)],
                // 後の間隔が厚さのソルブならソルブに任せる
                [(a - 1, 0.37), (b, -0.37)]
                    .into_iter()
                    .filter(|&(j, _)| !g.solves.iter().any(|q| q.surf == j))
                    .map(|(j, d)| (p(&format!("{j}.t")), g.surfaces[j].t + d))
                    .collect(),
            ),
            (
                vec![(p("lens2.dn"), 1.5e-4)],
                (lens2[0]..lens2[1])
                    .map(|j| (p(&format!("{j}.dn")), 1.5e-4))
                    .collect(),
            ),
            (
                vec![(p("lens1.disp"), -0.02), (p("lens1.dn"), 2e-5)],
                vec![(p("1.disp"), -0.02), (p("1.dn"), 2e-5)],
            ),
        ];
        for (u, f) in cases {
            let x = g.with_params(&u).unwrap();
            let y = g.with_params(&f).unwrap();
            same_bits(&x, &y, &format!("{name} {:?}", u));
            for &(q, v) in &u {
                assert_eq!(x.get_param(q).unwrap(), v);
            }
        }
        // 移動を 2 回（2 回目は差の分だけ動く）
        let x = g.with_params(&[(p("G.shift"), 0.2)]).unwrap();
        let x = x.with_params(&[(p("G.shift"), -0.1)]).unwrap();
        assert!((x.surfaces[a - 1].t - (g.surfaces[a - 1].t - 0.1)).abs() < 1e-14);
        if !g.solves.iter().any(|q| q.surf == b) {
            assert!((x.surfaces[b].t - (g.surfaces[b].t + 0.1)).abs() < 1e-14);
        }
    }
}

/// 剛体の偏心・傾き: 座標ブレークを入れた系（と、それを .zmx に書いて読み直した系）と丸め誤差で一致
#[test]
fn rigid_motion_equals_coordinate_breaks() {
    for name in ["A0", "B0", "C0"] {
        let mut s = load(name);
        let lens3 = s.units[2].spans[0];
        let l2 = s.units[1].spans[0];
        s.define_unit("G", &[[l2[0], lens3[1]]], [0.0; 3]).unwrap();
        let g = s.find_unit("G").unwrap();
        s.set_unit_pivot(g, [0.1, -0.2, 2.5]).unwrap();
        let p = |t: &str| s.param(t).unwrap();
        let cases = vec![
            vec![(p("lens1.dy"), 0.05)],
            vec![(p("lens1.tx"), 0.3), (p("lens1.dx"), -0.02)],
            vec![(p("lens2.tz"), 3.0), (p("lens2.ty"), -0.2)],
            // 回転の中心がある群と、その中の玉（入れ子）
            vec![(p("G.tx"), 0.25), (p("G.dy"), 0.03)],
            vec![
                (p("G.tx"), 0.25),
                (p("lens3.ty"), 0.4),
                (p("lens3.dx"), 0.01),
            ],
            vec![(p("G.shift"), 0.3), (p("G.ty"), 0.2), (p("lens2.tx"), -0.1)],
        ];
        for ch in cases {
            // C0 の玉 1 は中に戻さない傾きがあり、書き出しの座標ブレークに厚さが付く（近軸量が変わる）: 下の試験で
            if name == "C0"
                && ch
                    .iter()
                    .any(|(q, _)| s.param_name(*q).starts_with("lens1"))
            {
                continue;
            }
            let what = format!(
                "{name} {}",
                ch.iter()
                    .map(|(q, v)| format!("{}={v}", s.param_name(*q)))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let a = s.with_params(&ch).unwrap();
            assert!(!a.symmetric());
            let f = a.flatten_units().unwrap();
            assert!(f.surfaces.len() > a.surfaces.len());
            close(&a, &f, 1e-12, &what);
            same_frames(&a, &f, &what);
            // 書き出しても同じ（.zmx の数値の桁で）
            let txt = zmx_write::write(&a).unwrap();
            let raw = zmx::parse_text(&txt).unwrap();
            let r = zmx::build(
                &raw,
                &LoadOptions {
                    cfg: 1,
                    glasses: Some((*a.glasses).clone()),
                    ..Default::default()
                },
                &[],
                None,
            )
            .unwrap();
            close(&f, &r, 1e-12, &format!("{what}（読み直し）"));
            // 動きを 0 に戻すと元の系とビット一致
            let back: Vec<(Param, f64)> = ch.iter().map(|&(q, _)| (q, 0.0)).collect();
            // （移動は厚さの足し引きなので丸め誤差の範囲）
            let z = a.with_params(&back).unwrap();
            if ch.iter().any(|(q, _)| s.param_name(*q).ends_with(".shift")) {
                close(&z, &s, 1e-12, &format!("{what}（戻す）"));
            } else {
                same_bits(&z, &s, &format!("{what}（戻す）"));
            }
        }
    }
}

/// 座標ブレークでない面の座標系を順に比べる
fn same_frames(a: &System, b: &System, what: &str) {
    let pick = |s: &System| -> Vec<lensbench_core::system::Frame> {
        let f = s.frames();
        (1..s.surfaces.len())
            .filter(|&j| s.surfaces[j].typ != lensbench_core::surface::SurfType::CoordBrk)
            .filter_map(|j| f.surf[j])
            .collect()
    };
    let (x, y) = (pick(a), pick(b));
    assert_eq!(x.len(), y.len());
    for (k, (p, q)) in x.iter().zip(&y).enumerate() {
        for i in 0..3 {
            assert!(
                (p.o[i] - q.o[i]).abs() < 1e-12,
                "{what}: {k} {:?} {:?}",
                p.o,
                q.o
            );
            for j in 0..3 {
                assert!((p.r[i][j] - q.r[i][j]).abs() < 1e-14, "{what}: {k}");
            }
        }
    }
}

/// 中に戻さない傾きがある玉（C0 の玉 1）: 座標ブレークにした系の面の位置・向きが一致
#[test]
fn rigid_motion_of_a_lens_with_inner_tilt() {
    let s = load("C0");
    let p = |t: &str| s.param(t).unwrap();
    for ch in [
        vec![(p("lens1.dy"), 0.05)],
        vec![
            (p("lens1.tx"), 0.3),
            (p("lens1.dx"), -0.02),
            (p("lens1.tz"), 1.0),
        ],
    ] {
        let a = s.with_params(&ch).unwrap();
        same_frames(&a, &a.flatten_units().unwrap(), &format!("{ch:?}"));
    }
}

/// 剛体の動きは玉の面々を空間でまとめて動かす: 頂点の位置と向きを直接確かめる
#[test]
fn rigid_motion_moves_vertices_about_the_pivot() {
    let mut s = load("A0");
    let u = s.find_unit("lens2").unwrap();
    let [a, b] = s.units[u].spans[0];
    s.set_unit_pivot(u, [0.0, 0.0, 1.0]).unwrap();
    let m = s
        .with_params(&[
            (Param::unit(u, lensbench_core::units::UnitKind::Tx), 2.0),
            (Param::unit(u, lensbench_core::units::UnitKind::Dy), 0.1),
        ])
        .unwrap();
    let (f0, f1) = (s.frames(), m.frames());
    let o_entry = f0.surf[a].unwrap().o;
    let th = 2f64.to_radians();
    for j in a..=b {
        let p = f0.surf[j].unwrap().o;
        let q = f1.surf[j].unwrap().o;
        // 入口（面 a の頂点、傾きなし）の座標で: x → P + D + Rx (x − P)
        let (y, z) = (p[1] - o_entry[1], p[2] - o_entry[2] - 1.0);
        let want = [
            p[0],
            o_entry[1] + 0.1 + y * th.cos() - z * th.sin(),
            o_entry[2] + 1.0 + y * th.sin() + z * th.cos(),
        ];
        for i in 0..3 {
            assert!((q[i] - want[i]).abs() < 1e-12, "面 {j}: {q:?} {want:?}");
        }
    }
    // 範囲の外の面は動かない
    for j in (1..a).chain(b + 1..s.surfaces.len()) {
        assert_eq!(f0.surf[j].unwrap().o, f1.surf[j].unwrap().o, "面 {j}");
    }
}

/// ダブルパス: 往きと戻りの 2 つの範囲を同じ玉として、空間で同じだけ動かす
fn double_pass() -> System {
    text(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 1 0 0 0\n\
         XFLN 0 0\nYFLN 0 1.5\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
         SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
         SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n\
         SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 30\n\
         SURF 4\n  TYPE STANDARD\n  CURV -0.004\n  DISZ -30\n  GLAS MIRROR\n\
         SURF 5\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -4\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n\
         SURF 6\n  TYPE STANDARD\n  CURV 0.02\n  DISZ -5\n\
         SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
    )
}

#[test]
fn double_pass_lens_moves_both_passes() {
    let mut s = double_pass();
    s.define_unit("L", &[[2, 3], [5, 6]], [0.0, 0.0, 2.0])
        .unwrap();
    let p = |t: &str| s.param(t).unwrap();
    for ch in [
        vec![(p("L.dy"), 0.04)],
        vec![(p("L.tx"), 0.3), (p("L.dx"), 0.02), (p("L.tz"), 5.0)],
    ] {
        let a = s.with_params(&ch).unwrap();
        // 往きと戻りの同じ面（2 と 6、3 と 5）が同じ位置・向き
        let f = a.frames();
        for (x, y) in [(2, 6), (3, 5)] {
            let (fx, fy) = (f.surf[x].unwrap(), f.surf[y].unwrap());
            for i in 0..3 {
                assert!(
                    (fx.o[i] - fy.o[i]).abs() < 1e-12,
                    "{x} {y} {:?} {:?}",
                    fx.o,
                    fy.o
                );
                for k in 0..3 {
                    assert!((fx.r[i][k] - fy.r[i][k]).abs() < 1e-14);
                }
            }
        }
        let fl = a.flatten_units().unwrap();
        close(&a, &fl, 1e-12, &format!("ダブルパス {:?}", ch));
    }
    // 移動: 往きは前 +、後 −。戻り（厚さが負）も同じ式で空間の同じ向きに動く
    let a = s.with_params(&[(p("L.shift"), 0.5)]).unwrap();
    let t: Vec<f64> = a.surfaces.iter().map(|d| d.t).collect();
    assert_eq!(&t[1..7], &[10.5, 4.0, 29.5, -29.5, -4.0, -5.5]);
    let f = a.frames();
    assert!((f.surf[2].unwrap().o[2] - f.surf[6].unwrap().o[2]).abs() < 1e-12);
}

/// 前の間隔がピックアップ先（戻りの厚さが往きをピックアップ）でも、元と一緒に動けば変えられる
#[test]
fn shift_follows_pickups() {
    let t = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
         XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
         SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
         SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n\
         SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 30\n\
         SURF 4\n  TYPE STANDARD\n  CURV -0.004\n  DISZ -30\n  GLAS MIRROR\n  PZUP 3 -1 0\n\
         SURF 5\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -4\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n  PZUP 2 -1 0\n\
         SURF 6\n  TYPE STANDARD\n  CURV 0.02\n  DISZ -5\n\
         SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let mut s = text(t);
    assert!(!s.pickups.is_empty());
    s.define_unit("L", &[[2, 3], [5, 6]], [0.0; 3]).unwrap();
    let a = s
        .with_params(&[(s.param("L.shift").unwrap(), 0.5)])
        .unwrap();
    let t: Vec<f64> = a.surfaces.iter().map(|d| d.t).collect();
    assert_eq!(&t[1..7], &[10.5, 4.0, 29.5, -29.5, -4.0, -5.5]);
}

#[test]
fn bad_units_are_rejected() {
    let mut s = load("A0");
    assert!(s.define_unit("x", &[[0, 2]], [0.0; 3]).is_err());
    assert!(s.define_unit("x", &[[3, 2]], [0.0; 3]).is_err());
    assert!(s.define_unit("x", &[[1, s.image()]], [0.0; 3]).is_err());
    assert!(s.define_unit("x.y", &[[1, 2]], [0.0; 3]).is_err());
    assert!(s.define_unit("3", &[[1, 2]], [0.0; 3]).is_err());
    // 玉 1（面 1〜2）と一部だけ重なる
    assert!(s.define_unit("x", &[[2, 4]], [0.0; 3]).is_err());
    s.define_unit("x", &[[1, 4]], [0.0; 3]).unwrap();
    assert!(s.define_unit("x", &[[1, 4]], [0.0; 3]).is_err());
    // 自動の玉は同じ名前で置き換えられる
    s.define_unit("lens1", &[[1, 2]], [0.0, 0.0, 1.0]).unwrap();
    assert_eq!(s.units.iter().filter(|u| u.name == "lens1").count(), 1);
    // 面の名前と重なる名前は unit: / surf: で区別
    s.surfaces[5].comment = "x".into();
    assert!(s.param("x.dx").is_err());
    assert!(s.param("unit:x.dx").unwrap().kind.is_unit());
    assert!(!s.param("surf:x.dx").unwrap().kind.is_unit());
    // 媒質のない群の dn
    s.define_unit("air", &[[5, 5]], [0.0; 3]).ok();
}

// ---- dOPL/dp と中心差分 ----

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
    let Ok(sol) = s.solve_ray(&pr) else {
        return f64::NAN;
    };
    assert_eq!(s.surfaces[last].c, 0.0);
    let r = sol.trace.rec.last().unwrap();
    let (x, k) = (r.xyz[0], r.lmn[0]);
    let n = s.index_before(last, w);
    sol.trace.rays[0].opl - n * (k[0] * (x[0] - target[0]) + k[1] * (x[1] - target[1]))
}

fn step(p: Param) -> f64 {
    use lensbench_core::units::UnitKind as U;
    match p.kind {
        Kind::Unit(U::Dn) | Kind::Unit(U::Disp) => 1e-4,
        Kind::Unit(U::Tx) | Kind::Unit(U::Ty) | Kind::Unit(U::Tz) => 1e-3,
        _ => 1e-3,
    }
}

/// 面 last（平面）まで追った数本の光線の dOPL/dp を差分と比べる
fn check(label: &str, s: &System, fi: usize, w: usize, last: usize, params: &[Param]) {
    let px = [0.0, 0.6, -0.3, 0.2, -0.7];
    let py = [0.0, 0.1, 0.5, -0.8, -0.4];
    let (hx, hy) = s.hxhy(fi);
    let spec = s.object_spec(hx, hy).unwrap();
    let t = s.trace_spec(
        spec,
        &px,
        &py,
        w,
        lensbench_core::TraceOpts::new()
            .vig([0.0; 5])
            .upto(last)
            .record()
            .hxy((hx, hy)),
    );
    let g = s.opl_grad(&t, w, params).unwrap();
    let rl = t.rec.last().unwrap();
    let np = params.len();
    let mut worst: f64 = 0.0;
    let mut noise = 0;
    for i in 0..px.len() {
        assert_eq!(t.rays[i].err, 0, "{label}: 光線 {i} が止まる");
        let x = rl.xyz[i];
        let target = [x[0], x[1]];
        for (q, &p) in params.iter().enumerate() {
            let a = g[i * np + q];
            let mut best = (f64::INFINITY, 0.0, 0.0);
            for m in [0.25, 1.0, 4.0, 16.0] {
                let h = step(p) * m;
                let at = |d: f64| -> f64 {
                    let v = s.get_param(p).unwrap();
                    let s2 = s.with_params(&[(p, v + d)]).unwrap();
                    opl_fixed(&s2, spec, [px[i], py[i]], last, target, w)
                };
                let fd = (8.0 * (at(h) - at(-h)) - (at(2.0 * h) - at(-2.0 * h))) / (12.0 * h);
                if (a - fd).abs() < best.0 {
                    best = ((a - fd).abs(), fd, h);
                }
            }
            let (diff, fd, h) = best;
            let err = diff / a.abs().max(fd.abs()).max(1e-300);
            assert!(
                err < 1e-8 || diff * h < 1e-12,
                "{label} 光線 {i} {}: 解析 {a:e} 差分 {fd:e}（相対 {err:e}）",
                s.param_name(p)
            );
            if err < 1e-8 {
                worst = worst.max(err);
            } else {
                noise += 1;
            }
        }
    }
    println!(
        "{label}: {} 個のうち相対 1e-8 以内 {}（最大 {worst:.1e}）、差分の丸め誤差以内 {noise}",
        px.len() * np,
        px.len() * np - noise
    );
}

/// 最後に平面を足す（像面の手前で止めるため）。像面の前の面の厚さを分ける（その厚さのソルブは外す）
fn with_probe(s: &System) -> (System, usize) {
    let mut s = s.clone();
    let ima = s.image();
    let mut probe = s.surfaces[ima].clone();
    probe.t = 0.0;
    probe.c = 0.0;
    // 最後の玉のすぐ後（瞳座標で交点がよく動く所）
    let t = s.surfaces[ima - 1].t;
    s.surfaces[ima - 1].t = 1.0;
    probe.t = t - 1.0;
    probe.ap = lensbench_core::surface::Aperture::None;
    s.solves.retain(|q| q.surf != ima - 1);
    probe.n = s.surfaces[ima - 1].n.clone();
    s.surfaces.insert(ima, probe);
    s.changed().unwrap();
    (s, ima)
}

#[test]
fn opl_grad_of_unit_params_matches_differences() {
    for name in ["A0", "B0"] {
        let base = load(name);
        let (mut s, last) = with_probe(&base);
        s.units = s.lens_units();
        let l2 = s.units[1].spans[0];
        let l3 = s.units[2].spans[0];
        s.define_unit("G", &[[l2[0], l3[1]]], [0.0, 0.3, 1.5])
            .unwrap();
        s.set_unit_pivot(0, [0.0, 0.0, 1.0]).unwrap();
        let names = [
            "lens1.dx",
            "lens1.dy",
            "lens1.tx",
            "lens1.ty",
            "lens1.tz",
            "lens2.dn",
            "lens2.disp",
            "G.tx",
            "G.dy",
            "G.shift",
            "lens3.ty",
        ];
        let ps: Vec<Param> = names.iter().map(|n| s.param(n).unwrap()).collect();
        // 動かしていない系と、動かした系（入れ子の群と玉）
        check(name, &s, 1, 0, last, &ps);
        let m = s
            .with_params(&[
                (s.param("G.tx").unwrap(), 0.2),
                (s.param("lens3.ty").unwrap(), -0.3),
                (s.param("lens1.dy").unwrap(), 0.02),
            ])
            .unwrap();
        check(&format!("{name} 動かした系"), &m, 1, 0, last, &ps);
    }
}

#[test]
fn opl_grad_double_pass() {
    let base = double_pass();
    let (mut s, last) = with_probe(&base);
    s.define_unit("L", &[[2, 3], [5, 6]], [0.0, 0.0, 2.0])
        .unwrap();
    let ps: Vec<Param> = ["L.dx", "L.dy", "L.tx", "L.ty", "L.dn", "L.shift"]
        .iter()
        .map(|n| s.param(n).unwrap())
        .collect();
    check("ダブルパス", &s, 1, 0, last, &ps);
}
