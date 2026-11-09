//! 複屈折材の板（OpticStudio の Birefringent In / Out、Mode 2 の波長板と Mode 0）を OpticStudio 26.1 と照合する。
//! 系（tests/birefringent）: 絞り → 空気 5 → 板 → 空気 20 → 像面、波長 0.55・0.6328・0.8 µm、視野 角度 (0,0)・(0,3)・(2,−2)°。
//! b1 は水晶 1 mm（多次、結晶軸 30°）、b2 は水晶 0.0175 mm（633 nm で λ/4）、b3 は水晶 0.0175 mm（30°）と MgF₂ 0.0141 mm
//! （120°）の貼り合わせ、b4_m0 は b1 の Mode 0（常光の成分だけ）。基準は Normalized Polarized の像面の電場（入力 x・y）
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/birefringent")
}

fn load(name: &str) -> System {
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![dir().join("../glass")],
        ..Default::default()
    };
    zmx::load_with(&dir().join(format!("{name}.zmx")), &opt).unwrap()
}

/// 波長ごとに (光線ごとの複素の比 s を除いた残りの最大, |s| − 1 の最大)。比を除くのは、OS の光路の位相の数え方
/// （軸外の視野）の違いのため（tests/projects/A3 の照合と同じ）
fn pol_diff(s: &System, name: &str) -> Vec<(f64, f64)> {
    let txt = std::fs::read_to_string(dir().join(format!("os_pol_{name}.json"))).unwrap();
    let os: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let coat = Coatings::for_system(s).unwrap();
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let nw = os["wavelengths_um"].as_array().unwrap().len();
    let mut worst = vec![(0.0f64, 0.0f64); nw];
    for fd in os["fields"].as_array().unwrap() {
        let v = |k: &str| -> Vec<f64> {
            fd[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        let (hx, hy) = (fd["hx"].as_f64().unwrap(), fd["hy"].as_f64().unwrap());
        let (px, py) = (v("px"), v("py"));
        for (w, wv) in worst.iter_mut().enumerate() {
            let t = s
                .try_trace_pol(FieldSel::Norm(hx, hy), &px, &py, Some(w), &pols, &coat)
                .unwrap();
            for (i, e) in t.e.iter().enumerate() {
                let o = |m: usize, c: usize| {
                    C64::new(
                        fd["re"][w][m][i][c].as_f64().unwrap(),
                        fd["im"][w][m][i][c].as_f64().unwrap(),
                    )
                };
                let mc = || (0..2).flat_map(|m| (0..3).map(move |c| (m, c)));
                let (mut num, mut den) = (C64::new(0.0, 0.0), 0.0);
                for (m, c) in mc() {
                    num += o(m, c).conj() * e[m][c];
                    den += o(m, c).norm_sqr();
                }
                let sc = num / den;
                for (m, c) in mc() {
                    let d = (e[m][c] - sc * o(m, c)).norm();
                    assert!(d.is_finite(), "{name} 波長 {w} 光線 {i}");
                    wv.0 = wv.0.max(d);
                }
                wv.1 = wv.1.max((sc.norm() - 1.0).abs());
            }
        }
    }
    worst
}

#[test]
fn birefringent_plates_match_opticstudio() {
    for name in ["b1", "b2", "b3", "b4_m0"] {
        let s = load(name);
        let d = pol_diff(&s, name);
        println!("{name}（波長ごとの 比を除いた残り, |s| − 1） {d:?}");
        // Mode 0（常光の成分だけ）は 1e-12 で合う。Mode 2 の残り 2〜3e-4 は軸上でも同じ大きさで、OS が面に垂直な光線の
        // s・p を決めるときの小さな二色性（Fresnel の扱いを振っても消えない）（以前の偏光の照合で既知の差とした、直入射付近の s・p の扱いと同じ種類）
        let tol = if name == "b4_m0" {
            (1e-10, 1e-10)
        } else {
            (5e-4, 2e-3)
        };
        assert!(
            d.iter().all(|&(r, a)| r < tol.0 && a < tol.1),
            "{name} {d:?}"
        );
    }
}

/// 書き出して読み直しても同じ板（種類・Mode・結晶軸・材料・厚さ）
#[test]
fn birefringent_zmx_roundtrip() {
    for name in ["b1", "b3", "b4_m0"] {
        let s = load(name);
        let txt = zmx_write::write(&s).unwrap();
        assert!(txt.contains("TYPE BIRE__IN \"\"") && txt.contains("TYPE BIRE_OUT"));
        let p = std::env::temp_dir().join(format!("lb_bire_{name}_{}.zmx", std::process::id()));
        std::fs::write(&p, &txt).unwrap();
        let opt = zmx::LoadOptions {
            catalog_dirs: vec![dir().join("../glass")],
            ..Default::default()
        };
        let r = zmx::load_with(&p, &opt).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(r.surfaces.len(), s.surfaces.len());
        for (a, b) in s.surfaces.iter().zip(&r.surfaces) {
            assert_eq!(a.bire, b.bire);
            assert_eq!(a.mat, b.mat);
            assert_eq!(a.t, b.t);
        }
    }
}
