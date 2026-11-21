//! 収差図（ray-fan・opd-fan・field-curvature・longitudinal・lateral-color・focal-shift・grid-distortion・seidel）の値が、
//! 同じ量の評価関数の項（TRAY・TRAX・OPDC・FCGT・FCGS・LONA・AXCL・DISG・SPHA ほか）の値と同じであること（0 の符号は問わない）。

use crate::common::root;
use lensbench_analysis::Analyses;
use lensbench_core::seidel::DistComp;
use lensbench_core::{FieldSel, System};
use lensbench_formats::zmx::{self, LoadOptions};
use lensbench_optim::{DistMode, Problem, RayQ, SeidelQ, Term, TermKind};
use serde_json::{json, Value};

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

/// 項 kind の値（目標 0）
fn op(s: &System, kind: TermKind) -> f64 {
    let mut p = Problem::single(s.clone());
    p.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind,
    });
    p.term_values(&[]).unwrap()[0].value()
}

fn run(s: &System, name: &str, p: Value) -> Value {
    let cx = lensbench_analysis::Cx::new(s);
    Analyses::builtin()
        .get(name)
        .unwrap()
        .run_json(&cx, &p)
        .unwrap()
}

/// 同じ値（0 の符号は問わない）
fn same(a: f64, b: f64, what: &str) {
    assert!(a == b, "{what}: {a} vs {b}");
}

#[test]
fn ray_and_opd_fans_are_the_operands() {
    let s = load("A0");
    let v = run(&s, "ray-fan", json!({ "field": 3, "rays": 2 }));
    let o = run(&s, "opd-fan", json!({ "field": 3, "rays": 2 }));
    for (k, row) in v["fans"].as_array().unwrap().iter().enumerate() {
        let w = row["wave"].as_u64().unwrap() as usize - 1;
        for (i, p) in row["p"].as_array().unwrap().iter().enumerate() {
            let p = p.as_f64().unwrap();
            let ray = |px: f64, py: f64, q: RayQ| {
                op(
                    &s,
                    TermKind::Ray {
                        field: FieldSel::Index(2),
                        wave: Some(w),
                        px,
                        py,
                        surf: None,
                        q,
                        target: 0.0,
                    },
                )
            };
            let tan = row["tangential"][i].as_f64().unwrap();
            let sag = row["sagittal"][i].as_f64().unwrap();
            same(tan, ray(0.0, p, RayQ::TraY), "TRAY");
            same(sag, ray(p, 0.0, RayQ::TraX), "TRAX");
            let opd = |px: f64, py: f64| {
                op(
                    &s,
                    TermKind::Opd {
                        field: FieldSel::Index(2),
                        wave: Some(w),
                        px,
                        py,
                        target: 0.0,
                    },
                )
            };
            let ot = o["fans"][k]["tangential"][i].as_f64().unwrap();
            let os = o["fans"][k]["sagittal"][i].as_f64().unwrap();
            same(ot, opd(0.0, p), "OPDC tangential");
            same(os, opd(p, 0.0), "OPDC sagittal");
        }
    }
}

#[test]
fn field_curvature_and_distortion_are_the_operands() {
    let s = load("A0");
    let v = run(&s, "field-curvature", json!({ "points": 3 }));
    let h = v["h"].as_array().unwrap();
    for wv in v["waves"].as_array().unwrap() {
        let w = wv["wave"].as_u64().unwrap() as usize - 1;
        for (i, hh) in h.iter().enumerate() {
            let hh = hh.as_f64().unwrap();
            let f = FieldSel::Norm(0.0, hh);
            let fc = |q| {
                op(
                    &s,
                    TermKind::Ray {
                        field: f,
                        wave: Some(w),
                        px: 0.0,
                        py: 0.0,
                        surf: None,
                        q,
                        target: 0.0,
                    },
                )
            };
            same(
                wv["tangential"][i].as_f64().unwrap(),
                fc(RayQ::FcgT),
                "FCGT",
            );
            same(wv["sagittal"][i].as_f64().unwrap(), fc(RayQ::FcgS), "FCGS");
            if hh > 0.0 {
                let d = op(
                    &s,
                    TermKind::Distortion {
                        wave: Some(w),
                        absolute: false,
                        field: Some(f),
                        reference: None,
                        mode: DistMode::At,
                        abcd: None,
                        component: DistComp::Radial,
                        target: 0.0,
                    },
                );
                same(wv["distortion"][i].as_f64().unwrap(), d, "DISG");
            }
        }
    }
    let g = run(&s, "grid-distortion", json!({ "grid": 3 }));
    for p in g["points"].as_array().unwrap() {
        let (hx, hy) = (p["hx"].as_f64().unwrap(), p["hy"].as_f64().unwrap());
        if hx == 0.0 && hy == 0.0 {
            continue;
        }
        let d = op(
            &s,
            TermKind::Distortion {
                wave: None,
                absolute: false,
                field: Some(FieldSel::Norm(hx, hy)),
                reference: None,
                mode: DistMode::At,
                abcd: None,
                component: DistComp::Radial,
                target: 0.0,
            },
        );
        same(p["distortion"].as_f64().unwrap(), d, "DISG grid");
    }
}

#[test]
fn longitudinal_color_and_seidel_are_the_operands() {
    let s = load("A0");
    let v = run(&s, "longitudinal", json!({ "points": 3 }));
    let zones = v["zones"].as_array().unwrap();
    for wv in v["waves"].as_array().unwrap() {
        let w = wv["wave"].as_u64().unwrap() as usize - 1;
        for (i, z) in zones.iter().enumerate() {
            let l = op(
                &s,
                TermKind::Longitudinal {
                    wave: Some(w),
                    wave2: None,
                    zone: z.as_f64().unwrap(),
                    target: 0.0,
                },
            );
            same(wv["z"][i].as_f64().unwrap(), l, "LONA");
        }
    }
    // 色の焦点ずれの系の波長での値は AXCL（主波長から）
    let fs = run(&s, "focal-shift", json!({ "points": 2 }));
    let lo = s.wavelengths.iter().copied().fold(f64::INFINITY, f64::min);
    let wlo = s.wavelengths.iter().position(|&l| l == lo).unwrap();
    let axcl = op(
        &s,
        TermKind::Longitudinal {
            wave: Some(s.primary),
            wave2: Some(wlo),
            zone: 0.0,
            target: 0.0,
        },
    );
    let got = fs["shift"][0].as_f64().unwrap();
    assert!((got - axcl).abs() <= 1e-12, "AXCL {got} vs {axcl}");
    let sd = run(&s, "seidel", json!({}));
    for row in sd["waves"].as_array().unwrap() {
        let j = row["surf"].as_u64().unwrap() as usize;
        for (key, q) in [
            ("w040", SeidelQ::Spha),
            ("w131", SeidelQ::Coma),
            ("w222", SeidelQ::Asti),
            ("w220", SeidelQ::Fcur),
            ("w311", SeidelQ::Dist),
        ] {
            let x = op(
                &s,
                TermKind::Seidel {
                    surf: j,
                    q,
                    wave: None,
                    target: 0.0,
                },
            );
            same(row[key].as_f64().unwrap(), x, key);
        }
    }
    // 倍率色収差の視野 1 の値（実主光線）は 0、近軸の LACL とは視野の端で近い値
    let lc = run(&s, "lateral-color", json!({ "points": 2 }));
    for wv in lc["waves"].as_array().unwrap() {
        let w = wv["wave"].as_u64().unwrap() as usize - 1;
        assert_eq!(wv["d"][0].as_f64().unwrap(), 0.0, "lateral color on axis");
        let lacl = op(
            &s,
            TermKind::LateralColor {
                wave: Some(s.primary),
                wave2: w,
                target: 0.0,
            },
        );
        let real = wv["d"][1].as_f64().unwrap();
        assert!(
            (real - lacl).abs() < 0.5 * lacl.abs() + 1e-4,
            "lateral color {real} vs LACL {lacl}"
        );
    }
}

/// 相対照度・実効 F 値の図は RELI・EFNO の項と同じ。フットプリントの最大の半径は光線の交点から、けられの割合は 0〜1
#[test]
fn illumination_and_footprint() {
    let s = load("A0");
    let v = run(&s, "relative-illumination", json!({ "points": 3 }));
    for (i, hh) in v["h"].as_array().unwrap().iter().enumerate() {
        let f = FieldSel::Norm(0.0, hh.as_f64().unwrap());
        let term = |relative| {
            op(
                &s,
                TermKind::Illum {
                    field: f,
                    wave: None,
                    n: None,
                    relative,
                    target: 0.0,
                },
            )
        };
        same(v["relative"][i].as_f64().unwrap(), term(true), "RELI");
        same(v["effective_fno"][i].as_f64().unwrap(), term(false), "EFNO");
    }
    let fp = run(&s, "footprint", json!({ "surface": 5, "rays": 7 }));
    assert_eq!(fp["surface"], 5);
    for r in fp["rows"].as_array().unwrap() {
        let m = r["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[0].as_f64().unwrap().hypot(p[1].as_f64().unwrap()))
            .fold(0.0, f64::max);
        same(r["max_radius"].as_f64().unwrap(), m, "footprint radius");
        assert_eq!(r["through"], 1.0);
    }
    let vg = run(&s, "vignetting", json!({ "points": 3 }));
    for u in vg["unvignetted"].as_array().unwrap() {
        assert_eq!(u.as_f64().unwrap(), 1.0);
    }
}

/// 面に小さい開口を付けると、その面でけられる光線が出て、フットプリントの割合・けられの図が下がる
#[test]
fn footprint_counts_rays_clipped_at_an_aperture() {
    let mut s = load("A0");
    s.edit_raw(|rx| {
        rx.surfaces[7].ap = lensbench_core::surface::Aperture::Circular {
            min: 0.0,
            max: 5.0,
            dx: 0.0,
            dy: 0.0,
        };
        Ok(())
    })
    .unwrap();
    let fp = run(
        &s,
        "footprint",
        json!({ "surface": 7, "field": 3, "wave": 2, "rays": 15 }),
    );
    let r = &fp["rows"][0];
    let (through, clipped) = (
        r["through"].as_f64().unwrap(),
        r["clipped"].as_f64().unwrap(),
    );
    assert!(clipped > 0.0 && through < 1.0, "{through} {clipped}");
    // ほかの面でけられないので、通る割合とこの面でけられる割合の和は 1
    assert!(
        (through + clipped - 1.0).abs() < 1e-12,
        "{through} {clipped}"
    );
    assert!(fp["aperture"][0].as_array().unwrap().len() > 10);
    let vg = run(
        &s,
        "vignetting",
        json!({ "points": 2, "rays": 15, "no_vignetting_factors": true }),
    );
    assert!(vg["unvignetted"][1].as_f64().unwrap() < 1.0);
}
