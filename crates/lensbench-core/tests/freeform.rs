//! Zernike Standard・Fringe Sag と Grid Sag の面の矢高を OpticStudio の GetSag と比べる。
//!
//! 基準は tests/freeform（OpticStudio 26.1 の LDE.GetSag、非公開側で測定）。
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
        assert!(worst < 1e-12, "{name}: {worst:e}");
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
        let g = s.surfaces[1].grid.as_ref().unwrap();
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
        lensbench_core::zmx_write::save(&p, &a).unwrap();
        let b = zmx::load_with(&p, &opt).unwrap();
        for (j, (x, y)) in a.surfaces.iter().zip(&b.surfaces).enumerate() {
            assert_eq!(x.typ, y.typ, "{n} 面 {j}");
            assert_eq!(x.zern, y.zern, "{n} 面 {j}");
            assert_eq!(x.grid, y.grid, "{n} 面 {j}");
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
