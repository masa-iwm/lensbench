//! 分散式の有効範囲（AGF の LD）の外の波長の警告。式は延ばして計算する（OpticStudio は屈折率 1.0 にする）
use lensbench_core::zmx::{self, LoadOptions};
use std::path::PathBuf;

fn load(wavm: &str) -> lensbench_core::system::System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let txt = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nGCAT SCHOTT\nFTYP 0 0 1 2 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\n{wavm}PWAV 1\n\
SURF 0\n  CURV 0.0\n  DISZ INFINITY\nSURF 1\n  STOP\n  CURV 0.02\n  DISZ 4\n  GLAS N-SK16\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  CURV -0.01\n  DISZ 3\n  GLAS F2\n  DIAM 6 1 0 0 1 \"\"\nSURF 3\n  CURV 0.0\n  DISZ 50\nSURF 4\n  CURV 0.0\n  DISZ 0\n"
    );
    let raw = zmx::parse_text(&txt).unwrap();
    let opt = LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::build(&raw, &opt, &opt.catalog_dirs, None).unwrap()
}

#[test]
fn warns_outside_formula_range_and_extrapolates() {
    let s = load("WAVM 1 0.5876 1\nWAVM 2 2.0 1\n");
    assert!(s.dispersion_warnings().is_empty());
    // N-SK16（0.31〜2.5 µm）と F2（0.32〜2.5 µm）: 3 µm は両方の範囲の外、0.315 µm は F2 だけ
    let s = load("WAVM 1 0.315 1\nWAVM 2 3.0 1\n");
    let w = s.dispersion_warnings();
    assert_eq!(w.len(), 3, "{w:?}");
    assert!(w.iter().any(|x| x.contains("N-SK16") && x.contains("3 µm")));
    assert!(w.iter().any(|x| x.contains("F2") && x.contains("0.315")));
    // 屈折率は式を延ばした値（1.0 にはしない）
    assert!(s.surfaces[1].n[1] > 1.5 && s.surfaces[2].n[1] > 1.5);
}
