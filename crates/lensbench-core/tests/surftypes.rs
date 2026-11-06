//! Fresnel（FRESNELS）・Cylinder Fresnel（CFRESNEL）・Binary Optic 2（BINARY_2）の面を OpticStudio と比べる。
//!
//! 基準は tests/surftypes（独自処方の合成系、OpticStudio 26.1 の OPTH・REAX… と OPDC、非公開側で測定）。
//! - F1: 平面の窓 + Fresnel（R −40、k −1.2、r^4 3e-6）、視野 y 0・5・8°
//! - B2: R50 のレンズ + Binary 2（基板 R −80、r^4 1e-6、M 1、正規化半径 6、ρ^2・ρ^4・ρ^6 の係数）
//! - C1: 平面の窓 + Cylinder Fresnel（平面の基板、c −0.03、k −0.5、y^4 2e-6）、視野 (0,0)・(0,3)・(3,0)°
//! - C2: C1 の基板を R −100 にしたもの（OpticStudio は OPD を出さない。光路長は参考）
//!
//! 光線ごと・面ごとの交点・方向・光路長、EFL、OPDC を見る。

use std::path::PathBuf;

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::zmx_write;
use lensbench_core::{analysis, FieldSel, System};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(name: &str) -> (System, Value) {
    let dir = root().join("tests/surftypes").join(name);
    let s = zmx::load_with(
        &dir.join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("rays.json")).unwrap()).unwrap();
    (s, v)
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// 光線ごと・面ごとの差の最大（位置 mm、方向、光路長 mm（面 1 からの差））
fn ray_diff(name: &str) -> (f64, f64, f64, usize) {
    let (s, v) = load(name);
    let (mut dp, mut dd, mut dl, mut n) = (0.0f64, 0.0f64, 0.0f64, 0);
    for r in v["rays"].as_array().unwrap() {
        let w = r["wave"].as_u64().unwrap() as usize;
        let (hx, hy) = (f(&r["Hx"]), f(&r["Hy"]));
        let t = s.trace(
            FieldSel::Norm(hx, hy),
            &[f(&r["Px"])],
            &[f(&r["Py"])],
            Some(w),
            None,
            true,
        );
        let surfs = r["surfaces"].as_array().unwrap();
        let ok = surfs.last().unwrap()["err"].as_f64().unwrap() == 0.0;
        assert_eq!(ok, t.rays[0].err == 0, "{name}: {r}");
        if !ok {
            continue;
        }
        let o1 = t.rec[0].opl[0];
        let z1 = f(&surfs[0]["opth"]);
        for (rs, z) in t.rec.iter().zip(surfs) {
            assert_eq!(rs.surf as u64, z["surf"].as_u64().unwrap());
            let e = rs.entry(0);
            for (a, key) in ["x", "y", "z"].iter().enumerate() {
                dp = dp.max((e.xyz[a] - f(&z[key])).abs());
            }
            for (a, key) in ["l", "m", "n"].iter().enumerate() {
                dd = dd.max((rs.parity * e.lmn[a] - f(&z[key])).abs());
            }
            dl = dl.max(((e.opl - o1) - (f(&z["opth"]) - z1)).abs());
        }
        n += 1;
    }
    println!("{name}: {n} 本、位置 {dp:.1e}、方向 {dd:.1e}、光路長 {dl:.1e}");
    (dp, dd, dl, n)
}

#[test]
fn fresnel_rays() {
    let (dp, dd, dl, n) = ray_diff("F1");
    assert!(n >= 80);
    assert!(
        dp < 1e-11 && dd < 1e-13 && dl < 1e-10,
        "{dp:e} {dd:e} {dl:e}"
    );
}

#[test]
fn binary2_rays() {
    let (dp, dd, dl, n) = ray_diff("B2");
    assert!(n >= 80);
    assert!(
        dp < 1e-11 && dd < 1e-13 && dl < 1e-10,
        "{dp:e} {dd:e} {dl:e}"
    );
}

#[test]
fn cyl_fresnel_rays() {
    let (dp, dd, dl, n) = ray_diff("C1");
    assert!(n >= 80);
    assert!(
        dp < 1e-11 && dd < 1e-13 && dl < 1e-10,
        "{dp:e} {dd:e} {dl:e}"
    );
    // 曲がった基板: 交点・方向は一致。光路長は OpticStudio も正しくない（OPD を出さない）ので緩く
    let (dp, dd, dl, n) = ray_diff("C2");
    assert!(n >= 80);
    assert!(dp < 1e-9 && dd < 1e-11 && dl < 2e-5, "{dp:e} {dd:e} {dl:e}");
}

#[test]
fn paraxial_efl() {
    for (name, efl) in [
        ("F1", 77.39938),
        ("B2", 58.05061),
        ("C1", 64.49948),
        ("C2", 64.49948),
    ] {
        let (s, _) = load(name);
        let e = s.paraxial().efl;
        println!("{name}: EFL {e} (OS {efl})");
        assert!((e - efl).abs() < 1e-5, "{name}: {e}");
    }
}

/// OPDC（主光線基準、波長単位）。C2 は OpticStudio が出さない
#[test]
fn opd_matches_opdc() {
    for name in ["F1", "B2", "C1"] {
        let (s, _) = load(name);
        let dir = root().join("tests/surftypes").join(name);
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opd.json")).unwrap()).unwrap();
        let px: Vec<f64> = v["pupil_px"].as_array().unwrap().iter().map(f).collect();
        let py: Vec<f64> = v["pupil_py"].as_array().unwrap().iter().map(f).collect();
        let fi = v["field"].as_u64().unwrap() as usize;
        let o = analysis::opd(&s, FieldSel::Index(fi), &px, &py, None, true).unwrap();
        let mut worst = 0.0f64;
        for (k, r) in v["opd_waves_OPDC"].as_array().unwrap().iter().enumerate() {
            worst = worst.max((o.opd[k] - f(r)).abs());
        }
        println!("{name}: OPDC の差 {worst:.1e} 波");
        assert!(worst < 1e-6, "{name}: {worst:e}");
    }
}

/// 書き出した .zmx を読み直すと同じ追跡になる
#[test]
fn write_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("lb_st_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    for name in ["F1", "B2", "C1", "C2"] {
        let (s, _) = load(name);
        let p = tmp.join(format!("{name}.zmx"));
        std::fs::write(&p, zmx_write::write(&s).unwrap()).unwrap();
        let (s2, _) = (
            zmx::load_with(
                &p,
                &LoadOptions {
                    cfg: 1,
                    catalog_dirs: vec![root().join("tests/glass")],
                    ..Default::default()
                },
            )
            .unwrap(),
            (),
        );
        let px = [0.0, 0.3, -0.7, 1.0];
        let py = [0.0, 0.9, 0.2, -0.4];
        for fi in 0..s.fields.len() {
            let a = s.trace(FieldSel::Index(fi), &px, &py, None, None, false);
            let b = s2.trace(FieldSel::Index(fi), &px, &py, None, None, false);
            for (x, y) in a.rays.iter().zip(&b.rays) {
                assert_eq!(x.xyz, y.xyz, "{name}");
                assert_eq!(x.lmn, y.lmn, "{name}");
                assert_eq!(x.opl.to_bits(), y.opl.to_bits(), "{name}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
