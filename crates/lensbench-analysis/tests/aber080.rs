//! アフォーカル像空間の像面湾曲・色の焦点ずれ・グリッド歪曲と、瞳の収差の扇を OpticStudio 2026 R1.01 の出力
//! （tests/aber_os_080、refs/os080/aber_to_json.py で作る）と比べる

use lensbench_analysis::Analyses;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(p: &str) -> System {
    zmx::load_with(
        &root().join(p),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

fn refs(name: &str) -> Value {
    let p = root().join("tests/aber_os_080").join(name);
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

/// 差の最大を確かめる（出力の桁で丸めた値なので、許す差は桁の半分と計算の違いの分）
fn check(what: &str, pairs: impl Iterator<Item = (f64, f64)>, tol: f64) {
    let mut worst = (0.0_f64, 0.0, 0.0);
    for (a, b) in pairs {
        let d = (a - b).abs();
        assert!(d.is_finite(), "{what}: {a} vs {b}");
        if d > worst.0 {
            worst = (d, a, b);
        }
    }
    println!(
        "{what}: max diff {:.3e} ({} vs {})",
        worst.0, worst.1, worst.2
    );
    assert!(
        worst.0 <= tol,
        "{what}: {} vs {} (tol {tol})",
        worst.1,
        worst.2
    );
}

fn afocal() -> System {
    let s = load("tests/aber_os_080/afocal_tel.zmx");
    assert!(s.afocal);
    s
}

#[test]
fn afocal_field_curvature_matches_opticstudio() {
    let s = afocal();
    let os = refs("afocal_os.json");
    let fc = &os["field_curvature"];
    let out = run(&s, "field-curvature", json!({ "points": 101 }));
    assert_eq!(out["unit"], "diopters");
    for (k, w) in fc["waves"].as_array().unwrap().iter().enumerate() {
        let me = &out["waves"][k];
        let rows = w["rows"].as_array().unwrap();
        let col = |key: &str| rows.iter().map(|r| f(&r[key])).collect::<Vec<_>>();
        let get = |key: &str| {
            me[key]
                .as_array()
                .unwrap()
                .iter()
                .map(f)
                .collect::<Vec<_>>()
        };
        check(
            &format!("w{k} tan"),
            col("tan").into_iter().zip(get("tangential")),
            1e-6,
        );
        check(
            &format!("w{k} sag"),
            col("sag").into_iter().zip(get("sagittal")),
            1e-6,
        );
        check(
            &format!("w{k} dist"),
            col("distortion").into_iter().zip(get("distortion")),
            2e-8,
        );
    }
}

#[test]
fn afocal_focal_shift_matches_opticstudio() {
    let s = afocal();
    let os = &refs("afocal_os.json")["focal_shift"];
    let wl: Vec<f64> = os["wavelengths"]
        .as_array()
        .unwrap()
        .iter()
        .map(f)
        .collect();
    let out = run(&s, "focal-shift", json!({ "points": wl.len() }));
    assert_eq!(out["unit"], "diopters");
    let me = out["shift"].as_array().unwrap();
    check(
        "wavelength",
        wl.iter()
            .copied()
            .zip(out["wavelengths"].as_array().unwrap().iter().map(f)),
        5e-7,
    );
    check(
        "shift",
        os["shift"]
            .as_array()
            .unwrap()
            .iter()
            .map(f)
            .zip(me.iter().map(f)),
        2e-8,
    );
    assert_eq!(f(&out["diffraction_depth"]), 0.0);
}

#[test]
fn afocal_grid_distortion_matches_opticstudio() {
    let s = afocal();
    let os = &refs("afocal_os.json")["grid"];
    let out = run(&s, "grid-distortion", json!({ "grid": 13, "wave": 2 }));
    assert_eq!(out["unit"], "tan");
    let pts = out["points"].as_array().unwrap();
    let mut pairs = Vec::new();
    let mut pct = Vec::new();
    for o in os["points"].as_array().unwrap() {
        let (hx, hy) = (f(&o["x_field"]), f(&o["y_field"]));
        let m = pts
            .iter()
            .min_by(|a, b| {
                let d = |p: &Value| (f(&p["hx"]) - hx).hypot(f(&p["hy"]) - hy);
                d(a).total_cmp(&d(b))
            })
            .unwrap();
        assert!(
            (f(&m["hx"]) - hx).hypot(f(&m["hy"]) - hy) < 1e-5,
            "{hx} {hy}"
        );
        for i in 0..2 {
            pairs.push((f(&o["real"][i]), f(&m["real"][i])));
            pairs.push((f(&o["pred"][i]), f(&m["ideal"][i])));
        }
        pct.push((f(&o["distortion"]), f(&m["distortion"])));
    }
    check("grid points", pairs.into_iter(), 6e-7);
    check("grid distortion", pct.into_iter(), 1e-6);
}

#[test]
fn pupil_fan_matches_opticstudio() {
    let os = refs("paf_os.json");
    for (name, path) in [
        ("A0", "tests/refs/A0/system.zmx"),
        ("E0", "tests/refs/E0/system.zmx"),
        ("afocal_tel", "tests/aber_os_080/afocal_tel.zmx"),
    ] {
        let s = load(path);
        let out = run(&s, "pupil-fan", json!({ "rays": 20 }));
        assert_eq!(out["unit"], "%");
        let mut pairs = Vec::new();
        for fan in os["systems"][name].as_array().unwrap() {
            let field = fan["field"].as_u64().unwrap();
            let key = fan["kind"].as_str().unwrap();
            for w in 0..3 {
                let me = out["fans"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["field"] == field && r["wave"] == w + 1)
                    .unwrap();
                for (i, v) in fan["values"].as_array().unwrap().iter().enumerate() {
                    pairs.push((f(&v[w as usize]), f(&me[key][i])));
                }
            }
        }
        check(&format!("{name} pupil fan"), pairs.into_iter(), 6e-7);
    }
}

/// 像空間に平行に出る系（近軸レンズ 2 枚の望遠鏡、焦点が無限遠）は 0 視度（null にならない）
#[test]
fn collimated_afocal_is_zero_diopters() {
    let s = load("tests/aber_os_080/afocal_paraxial.zmx");
    assert!(s.afocal);
    let out = run(&s, "focal-shift", json!({ "points": 5 }));
    for v in out["shift"].as_array().unwrap() {
        assert_eq!(v.as_f64(), Some(0.0), "{out}");
    }
    let out = run(&s, "field-curvature", json!({ "points": 5 }));
    assert!(out["warnings"].as_array().unwrap().is_empty(), "{out}");
    for k in ["tangential", "sagittal"] {
        for v in out["waves"][0][k].as_array().unwrap() {
            let d = v.as_f64().unwrap();
            assert!(d.abs() < 1e-9, "{k}: {d}");
        }
    }
}
