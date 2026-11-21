//! 収差図・フットプリント・相対照度・けられの図を OpticStudio 26.1.1 の実測（tests/refs/<系>/aber_os.json。A0 とケラレ係数の
//! ある A6）と比べる。数値は OpticStudio の DataSeries（倍精度）か、テキストの出力（グリッド歪曲・ザイデル係数・フットプリント
//! は 6〜7 桁）。わかっている違い:
//! - 像面湾曲の子午: OpticStudio は主光線と瞳 +0.001 の片側の差分、lensbench は ±0.001 の中心差分（1e-4 mm ほど）
//! - 縦収差の瞳 0: OpticStudio の行は瞳 0.005 ほどの実光線、lensbench は近軸（1e-5 mm ほど）
//! - 相対照度の実効 F 値: OpticStudio の既定の標本（Ray Density 10）では数 % 大きく出る
//! - ケラレ係数のある系の相対照度・けられの図: OpticStudio の既定は係数を半直径の開口に置き換える（A6 は係数の効く 15° から先を比べない）

use crate::common::root;
use lensbench_analysis::Analyses;
use lensbench_core::system::VigInterp;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use serde_json::{json, Value};

const SYSTEMS: [&str; 2] = ["A0", "A6"];

/// ケラレ係数の補間は OpticStudio と同じ規則で（A6 の視野の間の点）
fn load(name: &str) -> System {
    let mut s = zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    s.edit_raw(|r| {
        r.vig_interp = VigInterp::OpticStudio;
        Ok(())
    })
    .unwrap();
    s
}

fn os(name: &str) -> Value {
    let p = root().join("tests/refs").join(name).join("aber_os.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn run(s: &System, name: &str, p: Value) -> Value {
    let cx = lensbench_analysis::Cx::new(s);
    Analyses::builtin()
        .get(name)
        .unwrap()
        .run_json(&cx, &p)
        .unwrap()
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn arr(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(f).collect()
}

fn near(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
}

#[test]
fn fans() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        // 光線収差は µm、OPD は波
        for (an, key, scale, tol) in [
            ("ray-fan", "ray_fan", 1e3, 1e-6),
            ("opd-fan", "opd_fan", 1.0, 1e-7),
        ] {
            let v = run(&s, an, json!({ "rays": 20 }));
            for row in v["fans"].as_array().unwrap() {
                let fi = row["field"].as_u64().unwrap() as usize;
                let w = row["wave"].as_u64().unwrap() as usize - 1;
                for (k, side) in ["tangential", "sagittal"].iter().enumerate() {
                    let r = &o[key][2 * (fi - 1) + k];
                    for (i, x) in arr(&row[*side]).iter().enumerate() {
                        let what = format!("{name} {an} field {fi} wave {} {side} {i}", w + 1);
                        near(x * scale, f(&r[i][w]), tol, &what);
                    }
                }
            }
        }
    }
}

#[test]
fn field_curvature_and_distortion() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        for (kind, key) in [
            ("f-tan", "field_curvature_ftan"),
            ("f-theta", "field_curvature_ftheta"),
        ] {
            let v = run(
                &s,
                "field-curvature",
                json!({ "points": 101, "distortion": kind }),
            );
            for (wi, wv) in v["waves"].as_array().unwrap().iter().enumerate() {
                let r = &o[key][wi];
                for i in 0..101 {
                    let what = format!("{name} {kind} wave {} point {i}", wi + 1);
                    near(f(&wv["tangential"][i]), f(&r[i][0]), 5e-4, &what);
                    near(f(&wv["sagittal"][i]), f(&r[i][1]), 1e-7, &what);
                    near(f(&wv["distortion"][i]), f(&r[i][4]), 1e-7, &what);
                }
            }
        }
    }
}

#[test]
fn longitudinal_lateral_color_and_focal_shift() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        let v = run(&s, "longitudinal", json!({ "points": 101 }));
        for (wi, wv) in v["waves"].as_array().unwrap().iter().enumerate() {
            for (i, z) in arr(&wv["z"]).iter().enumerate() {
                let tol = if i == 0 { 2e-5 } else { 1e-9 };
                near(
                    *z,
                    f(&o["longitudinal"][i][wi]),
                    tol,
                    &format!("{name} longitudinal {wi} {i}"),
                );
            }
        }
        // 主波長（列 1）は出さない。列 3・4 は Airy 半径
        let v = run(&s, "lateral-color", json!({ "points": 101 }));
        for wv in v["waves"].as_array().unwrap() {
            let w = wv["wave"].as_u64().unwrap() as usize - 1;
            for (i, d) in arr(&wv["d"]).iter().enumerate() {
                near(
                    d * 1e3,
                    f(&o["lateral_color"][i][w]),
                    1e-7,
                    &format!("{name} lateral color {w} {i}"),
                );
            }
        }
        near(
            f(&v["airy"]) * 1e3,
            f(&o["lateral_color"][0][3]),
            1e-9,
            "Airy radius",
        );
        let fs = &o["focal_shift"];
        let v = run(&s, "focal-shift", json!({ "points": 121 }));
        for (i, x) in arr(&v["shift"]).iter().enumerate() {
            near(
                *x,
                f(&fs["shift"][i]),
                1e-9,
                &format!("{name} focal shift {i}"),
            );
        }
        near(
            f(&v["range"]) * 1e3,
            f(&fs["range_um"]),
            1e-3,
            "focal shift range",
        );
        near(
            f(&v["diffraction_depth"]) * 1e3,
            f(&fs["diffraction_um"]),
            1e-3,
            "diffraction depth",
        );
    }
}

#[test]
fn grid_distortion() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        let v = run(&s, "grid-distortion", json!({ "grid": 9 }));
        let pts = v["points"].as_array().unwrap();
        assert_eq!(pts.len(), 81);
        for q in o["grid_distortion"]["points"].as_array().unwrap() {
            let (qx, qy) = (f(&q["predicted_x"]), f(&q["predicted_y"]));
            let p = pts
                .iter()
                .min_by(|a, b| {
                    let d = |p: &Value| (f(&p["ideal"][0]) - qx).hypot(f(&p["ideal"][1]) - qy);
                    d(a).total_cmp(&d(b))
                })
                .unwrap();
            let what = format!("{name} grid ({}, {})", q["i"], q["j"]);
            // A6 の (10.9°, −14.4°)・(14.4°, −14.4°) は視野 1 と 4 への L1 の距離が同じで、OpticStudio の補間（番号の小さい方）と
            // 丸めの差で選ぶ視野が分かれる
            let tie = name == "A6" && q["j"] == -4 && (q["i"] == 3 || q["i"] == 4);
            if tie {
                continue;
            }
            // テキストは 6 桁
            near(f(&p["ideal"][0]), qx, 1e-4, &what);
            near(f(&p["ideal"][1]), qy, 1e-4, &what);
            near(f(&p["real"][0]), f(&q["real_x"]), 1e-4, &what);
            near(f(&p["real"][1]), f(&q["real_y"]), 1e-4, &what);
            near(
                f(&p["distortion"]),
                f(&q["distortion_percent"]),
                1e-5,
                &what,
            );
        }
        near(
            f(&v["max_distortion"]),
            f(&o["grid_distortion"]["maximum_distortion_percent"]),
            1e-4,
            "max distortion",
        );
    }
}

#[test]
fn seidel_coefficients() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        let v = run(&s, "seidel", json!({}));
        let rows = &o["seidel"]["rows"];
        let keys = ["spha", "coma", "asti", "fcur", "dist", "cla", "ctr"];
        let mut n = 0;
        for r in v["surfaces"].as_array().unwrap() {
            let surf = r["surf"].to_string();
            // 平面の絞りなど、寄与の無い面は OpticStudio の表に無い
            let Some(q) = rows.get(&surf) else {
                assert!(
                    keys.iter().all(|k| f(&r[*k]).abs() < 1e-12),
                    "{name} surface {surf}"
                );
                continue;
            };
            for (i, k) in keys.iter().enumerate() {
                near(
                    f(&r[*k]),
                    f(&q[i]),
                    1.5e-6,
                    &format!("{name} surface {surf} {k}"),
                );
            }
            n += 1;
        }
        assert!(n >= 6);
        if let Some(q) = rows.get("TOT") {
            for (i, k) in keys.iter().enumerate() {
                near(
                    f(&v["sum"][*k]),
                    f(&q[i]),
                    1.5e-6,
                    &format!("{name} sum {k}"),
                );
            }
        }
    }
}

#[test]
fn footprint() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        for surf in [5, 7, 8] {
            // OpticStudio の Ray Density 20 は瞳の直径に 41 本の格子
            let v = run(&s, "footprint", json!({ "surface": surf, "rays": 41 }));
            for r in v["rows"].as_array().unwrap() {
                let key = format!("s{surf}_f{}_w{}", r["field"], r["wave"]);
                let q = &o["footprint_rd20"][&key];
                let what = format!("{name} footprint {key}");
                // テキストは 7 桁
                for (a, b) in [
                    (f(&r["x"][0]), "Ray X Minimum"),
                    (f(&r["x"][1]), "Ray X Maximum"),
                    (f(&r["y"][0]), "Ray Y Minimum"),
                    (f(&r["y"][1]), "Ray Y Maximum"),
                    (f(&r["max_radius"]), "Maximum Radius"),
                ] {
                    near(a, f(&q[b]), 1e-5, &format!("{what} {b}"));
                }
                near(
                    f(&r["through"]) * 100.0,
                    f(&q["rays_through_percent"]),
                    1e-9,
                    &what,
                );
            }
        }
    }
}

#[test]
fn relative_illumination_and_vignetting() {
    for name in SYSTEMS {
        let (s, o) = (load(name), os(name));
        // A6 は 15° から先でケラレ係数が効く（OpticStudio は係数を開口に置き換える）
        let upto = if name == "A6" { 15 } else { 21 };
        let ri = run(&s, "relative-illumination", json!({ "points": 21 }));
        let rp = run(
            &s,
            "relative-illumination",
            json!({ "points": 21, "polarization": true }),
        );
        let (q0, q1) = (
            &o["relative_illumination_pol0"],
            &o["relative_illumination_pol1"],
        );
        for i in 0..upto {
            let what = format!("{name} relative illumination {i}");
            // OpticStudio の標本（Ray Density 10）の揺れ
            near(f(&ri["relative"][i]), f(&q0[i][0]), 1e-3, &what);
            near(f(&rp["relative"][i]), f(&q1[i][0]), 1e-3, &what);
            // 実効 F 値は OpticStudio の標本で 5 % ほど大きく出る（標本を増やすと近づく）
            let (a, b) = (f(&ri["effective_fno"][i]), f(&q0[i][1]));
            assert!(a < b && b / a - 1.0 < 0.06, "{what} F/#: {a} vs {b}");
            // 偏光ありの実効 F 値は F/√T。透過率は OpticStudio の F/# の比の 2 乗と 0.3 % で合う（OpticStudio のガラスの内部透過の分）
            let t = f(&rp["transmission"].as_array().unwrap()[i]);
            near(t, (f(&q0[i][1]) / f(&q1[i][1])).powi(2), 3e-3, &what);
            near(f(&rp["effective_fno"][i]), a / t.sqrt(), 1e-12, &what);
        }
        if name == "A0" {
            let vg = run(&s, "vignetting", json!({ "points": 21 }));
            for (i, x) in arr(&vg["unvignetted"]).iter().enumerate() {
                near(
                    *x,
                    f(&o["vignetting_plot"][i]),
                    1e-12,
                    &format!("{name} vignetting {i}"),
                );
            }
        }
    }
}
