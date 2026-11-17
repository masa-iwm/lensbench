//! Zernike Standard・Fringe Sag と Grid Sag の面の矢高を OpticStudio の GetSag と比べる。
//!
//! 基準は tests/freeform（OpticStudio 26.1 の LDE.GetSag で測定）。
//! 面 1 だけの系で、x・y が ±7 mm の格子と数点。Zernike は 37・231 項と外挿の有無、Grid Sag は双三次・双一次、
//! 微分あり・なし（OpticStudio が差分で求めた微分は .zmx の GARR に入る）、格子の外、Grid Sag の Zernike の項。

use std::path::PathBuf;

use lensbench_core::freeform::{sag_xy, GridSag};
use lensbench_core::zmx;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/freeform")
}

#[test]
fn sag_matches_getsag() {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("getsag.json")).unwrap()).unwrap();
    let mut n = 0;
    for (name, t) in v["sag"].as_object().unwrap() {
        let s = zmx::load(&dir().join(format!("sag_{name}.zmx"))).unwrap();
        let d = &s.surfaces[1];
        let mut worst: f64 = 0.0;
        for (p, r) in t["pts"]
            .as_array()
            .unwrap()
            .iter()
            .zip(t["sag"].as_array().unwrap())
        {
            let (x, y) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
            let r = r.as_f64().unwrap();
            let z = sag_xy(d, x, y).0;
            // 正規化半径の外へ外挿した高次の項は値が大きく（ρ ≈ 2 で 1e3 mm 程度）、丸めの差もその相対で効く
            worst = worst.max((z - r).abs() / r.abs().max(1.0));
            n += 1;
        }
        println!("{name}: 最大の差 {worst:.2e}（mm、1 mm を超える値は相対）");
        // 外挿した 231 項は値が ρ^20 で大きく、項どうしの打ち消しの丸めが残る（漸化式で 1.3e-12、前の ρ² の
        // べきの多項式で 7e-13。正規化半径の中は漸化式で 9e-16、前は 9e-14）
        let extrapolated = name.contains("_231_ext1") || name.contains("_231_extNone");
        let tol = if extrapolated { 2e-12 } else { 1e-12 };
        assert!(worst < tol, "{name}: {worst:e}");
    }
    assert!(n > 3000);
}

/// .DAT の読込（微分なしは差分で求める）が、OpticStudio が .zmx に入れた格子（GARR）と同じになる
#[test]
fn dat_matches_garr() {
    for name in [
        "grid_d_cubic",
        "grid_nod_cubic",
        "grid_zonly_cubic",
        "grid_d_linear",
    ] {
        let s = zmx::load(&dir().join(format!("sag_{name}.zmx"))).unwrap();
        let g = s.surfaces[1].grid().unwrap();
        let dat = std::fs::read_to_string(dir().join(format!("lb_{name}.dat"))).unwrap();
        let h = GridSag::from_dat(&dat).unwrap();
        assert_eq!((h.nx, h.ny), (g.nx, g.ny));
        let mut worst: f64 = 0.0;
        for (a, b) in h.z.iter().zip(&g.z) {
            for c in 0..4 {
                worst = worst.max((a[c] - b[c]).abs());
            }
        }
        assert!(worst < 1e-15, "{name}: {worst:e}");
        assert_eq!((h.dx, h.dy, h.xdec, h.ydec), (g.dx, g.dy, g.xdec, g.ydec));
    }
}

/// 書き出した .zmx を読み直すと同じ面になり、追跡がビット一致する（Z0〜Z4・G0〜G3）
#[test]
fn write_roundtrip() {
    use lensbench_core::zmx::LoadOptions;
    use lensbench_core::FieldSel;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("lb_ff_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    for n in ["Z0", "Z1", "Z2", "Z3", "Z4", "G0", "G1", "G2", "G3"] {
        let a = zmx::load_with(&root.join(format!("tests/refs/{n}/system.zmx")), &opt).unwrap();
        let p = tmp.join(format!("{n}.zmx"));
        lensbench_formats::zmx_write::save(&p, &a).unwrap();
        let b = zmx::load_with(&p, &opt).unwrap();
        for (j, (x, y)) in a.surfaces.iter().zip(&b.surfaces).enumerate() {
            assert_eq!(x.typ(), y.typ(), "{n} 面 {j}");
            assert_eq!(x.zern(), y.zern(), "{n} 面 {j}");
            assert_eq!(x.grid(), y.grid(), "{n} 面 {j}");
            assert_eq!(x.asph, y.asph, "{n} 面 {j}");
            assert_eq!(x.t.to_bits(), y.t.to_bits(), "{n} 面 {j}");
        }
        let px = [0.0, 0.3, -0.7, 0.5];
        let py = [0.0, 0.8, 0.2, -0.6];
        let ta = a
            .try_trace(FieldSel::Index(1), &px, &py, None, None, false)
            .unwrap();
        let tb = b
            .try_trace(FieldSel::Index(1), &px, &py, None, None, false)
            .unwrap();
        for (r, q) in ta.rays.iter().zip(&tb.rays) {
            assert_eq!(r.xyz.map(f64::to_bits), q.xyz.map(f64::to_bits), "{n}");
            assert_eq!(r.opl.to_bits(), q.opl.to_bits(), "{n}");
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
}

/// sag_xy は追跡と同じ形（Shape::sag）: トロイダル面・Cylinder Fresnel の基板・Fresnel（平らな基板）でも、追跡した
/// 光線の面の上の交点（局所座標）の z と合う（以前は回転対称でない面のほかを偶数次非球面として求めていた）
#[test]
fn sag_xy_is_the_traced_shape() {
    use lensbench_core::FieldSel;
    let head = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 12\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 2\nYFLN 0 3\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 2\n";
    let tail = "  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.2 0 0 0 0 0\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 20\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let build = |surf: &str| {
        let txt = format!("{head}SURF 2\n  {surf}{tail}");
        zmx::build(
            &zmx::parse_text(&txt).unwrap(),
            &Default::default(),
            &[],
            None,
        )
        .unwrap()
    };
    for (label, surf) in [
        (
            "トロイダル面",
            "TYPE TOROIDAL\n  CURV 0.02\n  PARM 1 30\n  PARM 2 2e-3\n  PARM 3 -1e-6\n",
        ),
        ("Fresnel", "TYPE FRESNELS\n  CURV 0.03\n  PARM 1 1e-5\n"),
        (
            "Cylinder Fresnel",
            "TYPE CFRESNEL\n  CURV 0.015\n  PARM 1 1e-5\n",
        ),
    ] {
        let s = build(surf);
        let d = &s.surfaces[2];
        let px = [0.0, 0.5, -0.8, 0.3, 0.7];
        let py = [0.0, 0.6, 0.2, -0.9, -0.5];
        let t = s
            .try_trace(FieldSel::Index(1), &px, &py, None, None, true)
            .unwrap();
        let k = t.rec.iter().position(|r| r.surf == 2).unwrap();
        let mut moved: f64 = 0.0;
        for i in 0..px.len() {
            assert!(t.rays[i].status.is_ok(), "{label}: 光線 {i}");
            let q = t.rec[k].xyz[i];
            let z = sag_xy(d, q[0], q[1]).0;
            assert!(
                (z - q[2]).abs() < 1e-12,
                "{label} 光線 {i}: 矢高 {z} 交点 {}",
                q[2]
            );
            // 以前の求め方（c・k・asph の偶数次非球面）との差
            let r2 = q[0] * q[0] + q[1] * q[1];
            let old = d.c * r2 / (1.0 + (1.0 - (1.0 + d.k) * d.c * d.c * r2).sqrt())
                + d.asph
                    .iter()
                    .enumerate()
                    .map(|(j, a)| a * r2.powi(j as i32 + 1))
                    .sum::<f64>();
            moved = moved.max((old - z).abs());
        }
        println!("{label}: 以前の矢高との差 最大 {moved:e} mm");
        assert!(moved > 1e-3, "{label}: 以前の矢高との差 {moved}");
        if label == "Fresnel" {
            // Fresnel の面は平らな基板（曲率は屈折の傾きだけ）
            assert_eq!(sag_xy(d, 3.0, -2.0), (0.0, 0.0, 0.0));
        }
    }
    // トロイダル面の x 方向は回転の円（半径 30）: 点 (x, 0) の矢高は x² / (R + √(R² − x²))
    let s = build("TYPE TOROIDAL\n  CURV 0.02\n  PARM 1 30\n  PARM 2 2e-3\n");
    let x: f64 = 4.0;
    let want = x * x / (30.0 + (900.0 - x * x).sqrt());
    let z = sag_xy(&s.surfaces[2], x, 0.0).0;
    assert!((z - want).abs() < 1e-15, "{z} {want}");
}
