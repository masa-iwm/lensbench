//! 分散式の有効範囲（AGF の LD）の外の波長の警告。式は延ばして計算する（OpticStudio は屈折率 1.0 にする）
use lensbench_formats::zmx::{self, LoadOptions};
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
    assert!(s.n(1)[1] > 1.5 && s.n(2)[1] > 1.5);
}

/// 面の硝材の nd・vd のずれ（dnd・dvd、OpticStudio の TIND・TABB）: モデルガラスの式の差を波長ごとに足す
#[test]
fn glass_nd_vd_offsets_add_model_glass_difference() {
    use lensbench_core::glass::model_index;
    use lensbench_core::surface::Material;
    let mut s = load("WAVM 1 0.4861 1\nWAVM 2 0.5876 1\nWAVM 3 0.6563 1\n");
    let n0: Vec<f64> = s.n(2).to_vec();
    let (nd, vd, dpgf) = s.glass_nd_vd(&s.surfaces[2].mat).unwrap();
    assert!((vd - 36.37).abs() < 0.05, "F2 vd {vd}");
    s.edit(|rx| {
        rx.surfaces[2].dvd = 5.0;
        rx.surfaces[2].dnd = 1e-3;
        Ok(())
    })
    .unwrap();
    for (w, &l) in s.wavelengths.clone().iter().enumerate() {
        let d = model_index(nd + 1e-3, vd + 5.0, dpgf, l) - model_index(nd, vd, dpgf, l);
        assert!((s.n(2)[w] - n0[w] - d).abs() < 1e-15);
    }
    // モデルガラスでは nd・vd を直接変えたのと同じ
    let mut m = s.clone();
    m.edit(|rx| {
        rx.surfaces[2].mat = Material::Model {
            nd: 1.6,
            vd: 40.0,
            dpgf: 0.0,
        };
        rx.surfaces[2].dnd = 0.0;
        rx.surfaces[2].dvd = -2.0;
        Ok(())
    })
    .unwrap();
    for (w, &l) in m.wavelengths.clone().iter().enumerate() {
        assert!((m.n(2)[w] - model_index(1.6, 38.0, 0.0, l)).abs() < 1e-15);
    }
    // 屈折率だけの媒質には使えない
    let mut x = s.clone();
    let e = x
        .edit(|rx| {
            rx.surfaces[2].mat = Material::Index(1.5);
            Ok(())
        })
        .unwrap_err();
    assert!(e.to_string().contains("nd and vd offsets"), "{e}");
}
