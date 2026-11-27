//! 有限距離の物体・角度の視野の系（B3）の歪曲とザイデル係数を OpticStudio 26.1.1 の実測（tests/refs/B3/dist_os.json）と比べる。
//! 前は近軸主光線の倍率に物体距離が掛からず、グリッド歪曲・像面湾曲の歪曲が 2 万 %、ザイデル係数が桁で小さく出ていた

use crate::common::root;
use lensbench_analysis::Analyses;
use lensbench_core::seidel;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use serde_json::{json, Value};

fn load() -> System {
    zmx::load_with(
        &root().join("tests/refs/B3/system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

fn os() -> Value {
    let p = root().join("tests/refs/B3/dist_os.json");
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
    v.as_f64().unwrap()
}

fn near(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
}

/// 主波長（0 から数えて 1）
const W: usize = 1;

#[test]
fn grid_distortion() {
    let (s, o) = (load(), os());
    let v = run(&s, "grid-distortion", json!({ "grid": 9 }));
    let pts = v["points"].as_array().unwrap();
    let refs = o["grid"].as_array().unwrap();
    assert_eq!(pts.len(), refs.len());
    for r in refs {
        let what = format!("grid ({}, {})", f(&r["hx"]), f(&r["hy"]));
        // 格子の点の並びは OpticStudio と違うので、視野で引く
        let p = pts
            .iter()
            .find(|p| {
                (f(&p["hx"]) - f(&r["hx"])).abs() < 1e-12
                    && (f(&p["hy"]) - f(&r["hy"])).abs() < 1e-12
            })
            .unwrap_or_else(|| panic!("{what}: no point"));
        for k in 0..2 {
            near(f(&p["ideal"][k]), f(&r["ideal"][k]), 1e-9, &what);
            near(f(&p["real"][k]), f(&r["real"][k]), 1e-9, &what);
        }
        near(f(&p["distortion"]), f(&r["distortion"]), 1e-8, &what);
    }
    // 軸上の基準の A・D（OpticStudio の ABCD）
    let r = seidel::distortion_ref(&s, W, None).unwrap();
    near(r.j[0][0], f(&o["abcd_axis"][0]), 1e-8, "ABCD A");
    near(r.j[1][1], f(&o["abcd_axis"][3]), 1e-8, "ABCD D");
}

#[test]
fn distortion_operands_and_field_curvature() {
    let (s, o) = (load(), os());
    // DISG の基準の視野の番号（1 は軸上）の正規化視野。視野は (0, 0)・(0, 5)・(0, 8)・(3, 3) 度
    let ref_field = |k: u64| match k {
        1 => None,
        2 => Some((0.0, 5.0 / 8.0)),
        _ => unreachable!(),
    };
    for d in o["disg"].as_array().unwrap() {
        let h = (f(&d["hx"]), f(&d["hy"]));
        let r = ref_field(d["ref_field"].as_u64().unwrap());
        let (_, pct) = seidel::distortion_at(&s, W, h, r).unwrap();
        // 視野 2 を基準にした x の向きの微分（ABCD の A）は OpticStudio と相対 7e-8 違う（差分の刻みの違いとみる。
        // D と軸上の基準は 1e-10 で合う）ので、x の成分のある点は 1e-5 % まで
        let tol = if r.is_some() && h.0 != 0.0 {
            1e-5
        } else {
            1e-9
        };
        near(pct, f(&d["value"]), tol, &format!("DISG {r:?} {h:?}"));
    }
    near(
        seidel::distortion_max(&s, W, (0.0, 1.0), false).unwrap(),
        f(&o["dimx_pct"]),
        1e-9,
        "DIMX %",
    );
    near(
        seidel::distortion_max(&s, W, (0.0, 1.0), true).unwrap(),
        f(&o["dimx_abs"]),
        1e-10,
        "DIMX mm",
    );
    let (da, dp) = seidel::ftheta_distortion(&s, W).unwrap();
    near(da, f(&o["disc_abs"]), 1e-10, "DISC mm");
    near(dp, f(&o["disc_pct"]), 1e-9, "DISC %");
    let v = run(&s, "field-curvature", json!({ "points": 101 }));
    let wv = &v["waves"][W];
    for (i, r) in o["fc_distortion"]["distortion"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        near(
            f(&wv["distortion"][i]),
            f(r),
            1e-7,
            &format!("field curvature distortion {i}"),
        );
    }
}

#[test]
fn seidel_sums() {
    let (s, o) = (load(), os());
    let z = seidel::seidel(&s, W).unwrap();
    let sum = z.waves(0);
    for (k, key) in ["SPHA", "COMA", "ASTI", "FCUR"].iter().enumerate() {
        near(sum[k], f(&o["seidel_waves_sum"][key]), 1e-6, key);
    }
    for r in o["seidel_waves_surfaces"].as_array().unwrap() {
        let j = r["surface"].as_u64().unwrap() as usize;
        let w = z.waves(j);
        for (k, key) in ["SPHA", "COMA", "ASTI", "FCUR", "DIST"].iter().enumerate() {
            near(w[k], f(&r[key]), 1e-6, &format!("{key} surface {j}"));
        }
    }
}
