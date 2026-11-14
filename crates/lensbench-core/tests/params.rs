//! 系を作り直さずにパラメータを変える: 変えた系の追跡が、同じ値の .zmx を読み直した系の追跡とビット一致する。

use crate::common::root;
use lensbench_core::param::{Kind, Param};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{zmx_write, System};
use std::time::Instant;

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

/// .zmx に書いて同じガラスで読み直す
fn reload(s: &System) -> System {
    let txt = zmx_write::write(s).unwrap();
    let raw = zmx::parse_text(&txt).unwrap();
    zmx::build(
        &raw,
        &LoadOptions {
            cfg: 1,
            glasses: Some((*s.glasses).clone()),
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap()
}

fn grid() -> (Vec<f64>, Vec<f64>) {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for i in 0..9 {
        for j in 0..9 {
            let (x, y) = (i as f64 / 4.0 - 1.0, j as f64 / 4.0 - 1.0);
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// 全視野・全波長の追跡のビット列
fn bits(s: &System) -> Vec<u64> {
    let (px, py) = grid();
    let mut out = Vec::new();
    for f in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for r in &t.rays {
                out.extend(r.xyz.iter().chain(&r.lmn).map(|v| v.to_bits()));
                out.push(r.opl.to_bits());
                out.push(r.err as u64);
                out.push(r.vig as u64);
            }
        }
    }
    out
}

fn same(a: &System, b: &System, what: &str) {
    let (x, y) = (bits(a), bits(b));
    assert_eq!(x.len(), y.len());
    let diff = x.iter().zip(&y).filter(|(p, q)| p != q).count();
    assert_eq!(diff, 0, "{what}: {diff} 個の値が違う");
}

fn changes(s: &System) -> Vec<Vec<(Param, f64)>> {
    let p = |t: &str| s.param(t).unwrap();
    vec![
        vec![(p("2.c"), -3.1e-3)],
        vec![(p("4.r"), 20.5)],
        vec![(p("1.k"), -0.3)],
        vec![(p("1.a4"), 2.5e-6), (p("1.a6"), -1e-8)],
        vec![(p("3.t"), 1.25)],
        vec![(p("7.t"), 19.7)],
        // 傾き偏心: 既にある面（前・後とも）、まだない面、鏡
        vec![(p("2.dy"), 0.35), (p("2.tx"), 1.2)],
        vec![(p("3.tz"), 2.5), (p("3.dx"), 0.1)],
        vec![(p("4.dx"), 0.05), (p("4.ty"), -0.3)],
        vec![(p("8.tx"), 30.2)],
        vec![(p("6.tz"), 1.0), (p("7.dy"), -0.02), (p("2.c"), -2.5e-3)],
    ]
}

#[test]
fn changed_system_traces_bit_identical_to_reloaded_zmx() {
    for name in ["C0", "C1", "A0", "E1"] {
        let base = load(name);
        let list = if name.starts_with('C') {
            changes(&base)
        } else {
            let p = |t: &str| base.param(t).unwrap();
            vec![
                vec![(p("2.c"), base.surfaces[2].c * 1.01)],
                vec![(p("3.t"), base.surfaces[3].t + 0.1)],
            ]
            .into_iter()
            // 厚さのソルブで決まる厚さは変えられない（E1 の面 3）
            .filter(|ch| {
                ch.iter().all(|(q, _)| {
                    base.pickup_of(*q).is_none()
                        && !(q.name().ends_with(".t")
                            && base.solves.iter().any(|z| z.surf == q.surf))
                })
            })
            .chain([vec![(p("1.t"), base.surfaces[1].t + 0.1)]])
            .chain([vec![(p("2.dx"), 0.05), (p("2.tx"), 0.2)]])
            .collect()
        };
        // 追跡してキャッシュを作ってから変える（残したキャッシュが正しいことも確かめる）
        bits(&base);
        for ch in list {
            let names: Vec<String> = ch
                .iter()
                .map(|(p, v)| format!("{}={v}", p.name()))
                .collect();
            let what = format!("{name} {}", names.join(" "));
            let a = base.with_params(&ch).unwrap();
            let r = reload(&a);
            same(&a, &r, &what);
            let mut b = base.clone();
            bits(&b);
            b.set_params(&ch).unwrap();
            same(&b, &r, &format!("{what}（set）"));
            // 値を読み返せる
            for (p, v) in &ch {
                let got = a.get_param(*p).unwrap();
                assert!(
                    (got - v).abs() <= 1e-15 * v.abs(),
                    "{what}: {} = {got}",
                    p.name()
                );
            }
        }
    }
}

#[test]
fn surfaces_by_name_and_errors() {
    let mut s = load("C0");
    s.surfaces[3].comment = "L1 back".into();
    s.surfaces[6].comment = "dup".into();
    s.surfaces[7].comment = "DUP".into();
    assert_eq!(s.find_surface("3").unwrap(), 3);
    assert_eq!(s.find_surface(" l1 BACK ").unwrap(), 3);
    assert_eq!(s.param("L1 back.t").unwrap(), Param::new(3, Kind::Thick));
    assert!(s.find_surface("dup").is_err());
    assert!(s.find_surface("99").is_err());
    assert!(s.param("3.xx").is_err());
    assert!(s.param("2.a4").is_err(), "標準面に非球面係数");
    assert!(s.param("8.dn").is_err(), "鏡の媒質");
    assert_eq!(s.param("objx").unwrap(), Param::obj_x());
    assert!(s.set_param(Param::obj_x(), 1.0).is_err());
    // 途中にエラーがあれば何も変えない（キャッシュも元のまま使える）
    let before = bits(&s);
    let t3 = s.surfaces[3].t;
    assert!(s
        .set_params(&[
            (s.param("3.t").unwrap(), t3 + 1.0),
            (s.param("2.c").unwrap(), f64::NAN)
        ])
        .is_err());
    assert_eq!(s.surfaces[3].t, t3);
    assert_eq!(bits(&s), before);
}

#[test]
fn index_offsets() {
    let base = load("C0");
    let wp = base.primary;
    // 面 1 の後の媒質（N-SK16）
    let a = base
        .with_params(&[
            (base.param("1.dn").unwrap(), 1e-3),
            (base.param("1.disp").unwrap(), 0.02),
        ])
        .unwrap();
    for w in 0..base.wavelengths.len() {
        let b0 = base.surfaces[1].n[w];
        let want = b0 + 1e-3 + 0.02 * (b0 - base.surfaces[1].n[wp]);
        assert_eq!(a.surfaces[1].n[w], want);
        assert_eq!(a.base_index(1, w), b0);
    }
    // 追跡は変わる
    assert_ne!(bits(&a), bits(&base));
    // 0 に戻すと元とビット一致
    let mut b = a.clone();
    b.set_params(&[
        (b.param("1.dn").unwrap(), 0.0),
        (b.param("1.disp").unwrap(), 0.0),
    ])
    .unwrap();
    assert!(b.surfaces[1].n_base.is_empty());
    same(&b, &base, "dn を 0 に戻す");
    // 鏡の後は前の媒質を引き継ぐ: 鏡の前の空気（面 7 の後）にずれを与えると鏡（面 8）の n も変わる
    let c = base
        .with_params(&[(base.param("7.dn").unwrap(), 2e-4)])
        .unwrap();
    assert_eq!(c.surfaces[8].n, c.surfaces[7].n);
    assert_eq!(c.surfaces[8].n[wp], base.surfaces[8].n[wp] + 2e-4);
    // .zmx には書けない
    assert!(zmx_write::write(&c).is_err());
}

/// 変更 1 回のコスト（変更＋最初の追跡に要る前処理）。値は表示のみ
#[test]
fn change_cost() {
    let base = load("C1");
    let (px, py) = grid();
    base.try_trace(FieldSel::Index(1), &px, &py, None, None, false)
        .unwrap();
    let n = 2000;
    for (label, p) in [
        ("曲率", base.param("2.c").unwrap()),
        ("厚さ", base.param("3.t").unwrap()),
        ("傾き", base.param("4.tx").unwrap()),
        ("dn", base.param("1.dn").unwrap()),
    ] {
        let v0 = base.get_param(p).unwrap();
        let t0 = Instant::now();
        for i in 0..n {
            let s = base.with_params(&[(p, v0 + 1e-6 * i as f64)]).unwrap();
            std::hint::black_box(s.frames());
        }
        let dt = t0.elapsed().as_secs_f64() / n as f64;
        let t1 = Instant::now();
        for i in 0..n / 10 {
            let s = base.with_params(&[(p, v0 + 1e-6 * i as f64)]).unwrap();
            std::hint::black_box(
                s.try_trace(FieldSel::Index(1), &px[..1], &py[..1], None, None, false)
                    .unwrap(),
            );
        }
        let dt1 = t1.elapsed().as_secs_f64() / (n / 10) as f64;
        println!(
            "{label}: 複製＋変更＋座標系 {:.1} µs、主光線 1 本まで（エイミング込み） {:.1} µs",
            dt * 1e6,
            dt1 * 1e6
        );
    }
}

/// 曲率を変えると面の交点の求め方（平面・曲面）も変わる: 平面にして追跡してから曲面に戻した系の偏光追跡
/// （座標系のキャッシュを引き継ぐ経路）が、キャッシュを捨てた系とビット一致する（のレビュー）
#[test]
fn surface_shape_cache_follows_curvature_changes() {
    use lensbench_core::coating::{Coatings, C64};
    let one = C64::new(1.0, 0.0);
    let zero = C64::new(0.0, 0.0);
    let pols = [[one, zero], [zero, one]];
    let (px, py) = (vec![0.0, 0.3, -0.5, 0.7], vec![0.0, 0.6, 0.2, -0.4]);
    for name in ["C0", "C1"] {
        let base = load(name);
        let p = Param::new(2, Kind::Curv);
        let c0 = base.get_param(p).unwrap();
        assert!(c0 != 0.0, "{name}: 面 2 が平面");
        let flat = base.with_params(&[(p, 0.0)]).unwrap();
        let coat = Coatings::for_system(&flat).unwrap_or_default();
        let f = FieldSel::Index(1.min(flat.fields.len() - 1));
        flat.try_trace_pol(f, &px, &py, None, &pols, &coat).unwrap();
        let back = flat.with_params(&[(p, c0)]).unwrap();
        let mut fresh = back.clone();
        fresh.changed().unwrap();
        let a = back.try_trace_pol(f, &px, &py, None, &pols, &coat).unwrap();
        let b = fresh
            .try_trace_pol(f, &px, &py, None, &pols, &coat)
            .unwrap();
        for (i, (ra, rb)) in a.trace.rays.iter().zip(&b.trace.rays).enumerate() {
            for k in 0..3 {
                assert_eq!(
                    ra.xyz[k].to_bits(),
                    rb.xyz[k].to_bits(),
                    "{name} 光線 {i}: {:?} と {:?}",
                    ra.xyz,
                    rb.xyz
                );
            }
        }
    }
}
