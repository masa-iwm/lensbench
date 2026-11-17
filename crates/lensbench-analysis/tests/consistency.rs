//! 解析どうしの整合の試験。OpticStudio の基準値に頼らず、物理の関係だけで確かめる。
//! - 回折 MTF = |PSF のフーリエ変換| / PSF の総和（軸上と、像面の上の PSF（既定）なら斜めの視野も）
//! - 中心遮蔽の系でも Huygens PSF と FFT PSF が合う（以前は Huygens が遮蔽の系で止まっていた）
//! - 中心遮蔽の系の PSF の MTF は輪帯瞳の式と合う
//! - 瞳の標本不足・像の格子の外の光量を PSF・Huygens PSF が知らせ、自動の格子では知らせない

use lensbench_analysis::huygens::{huygens_psf, HuygensOpts};
use lensbench_core::analysis;
use lensbench_core::psf::{self, Psf, PsfOpts};
use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::Waves;
use std::f64::consts::PI;
use std::path::PathBuf;

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join("tests/refs").join(name).join("system.zmx"), &opt).unwrap()
}

/// 理想レンズ（f 100、入射瞳径 10、λ 0.55 µm）の前の平行光の中に半径 obsc の遮蔽を置く（obsc_mtf.rs と同じ系）
fn ideal(obsc: f64) -> System {
    ideal_defocus(obsc, 0.0)
}

/// [`ideal`] の像面を dz [mm] ずらす（焦点ずれ W020 ≈ dz·NA² / 2λ、NA 0.05 で dz 1 mm ≈ 2.3 波）
fn ideal_defocus(obsc: f64, dz: f64) -> System {
    let t = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 2\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ {z}\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let t = t.replace("{z}", &(100.0 + dz).to_string());
    let mut s = zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap();
    if obsc > 0.0 {
        s.edit_raw(|rx| {
            rx.surfaces[1].ap = Aperture::Obsc {
                min: 0.0,
                max: obsc,
                dx: 0.0,
                dy: 0.0,
            };
            Ok(())
        })
        .unwrap();
    }
    s
}

/// PSF の格子の上で直接フーリエ変換した OTF の絶対値を全光量で割る（x 向き = 球欠、y 向き = 子午）。
/// 全光量は格子の総和 / 格子に入った割合（格子の外の裾は 0 以外の周波数ではほぼ打ち消し合う）。
/// 格子の座標は PSF の面の上（既定は像面、軸上ではどちらの面も同じ）
fn psf_mtf(p: &Psf, f: f64) -> (f64, f64) {
    let (mut sx, mut sy, mut tot) = ((0.0, 0.0), (0.0, 0.0), 0.0);
    for (iy, row) in p.i.rows().enumerate() {
        for (ix, &v) in row.iter().enumerate() {
            let (ax, ay) = (2.0 * PI * f * p.i.x()[ix], 2.0 * PI * f * p.i.y()[iy]);
            sx.0 += v * ax.cos();
            sx.1 += v * ax.sin();
            sy.0 += v * ay.cos();
            sy.1 += v * ay.sin();
            tot += v;
        }
    }
    let tot = tot / p.enclosed;
    (f64::hypot(sx.0, sx.1) / tot, f64::hypot(sy.0, sy.1) / tot)
}

/// 収差のある実際の系の軸上で、PSF のフーリエ変換が回折 MTF と合う（単色・多色）。
/// 差は瞳の格子の離散化と格子の外の裾の分（実測で 3e-4 以下）
#[test]
fn psf_transform_is_mtf() {
    for (name, tol) in [("A0", 1e-3), ("B0", 1e-3), ("D1", 1e-3)] {
        let s = load(name);
        let f0 = FieldSel::Index(0);
        for w in [Some(s.primary), None] {
            let o = PsfOpts {
                image: 256,
                ..Default::default()
            };
            let p = psf::psf(&s, f0, Waves::all_or(w), o).unwrap();
            assert!(p.warnings.is_empty(), "{name}: {:?}", p.warnings);
            let cut = analysis::mtf_cutoff(&s, f0, s.primary).unwrap().sag;
            let fr: Vec<f64> = (1..10).map(|k| cut * k as f64 / 10.0).collect();
            let m = analysis::mtf(&s, f0, Waves::all_or(w), None, &fr).unwrap();
            for (k, &q) in fr.iter().enumerate() {
                let (ps, pt) = psf_mtf(&p, q);
                assert!(
                    (ps - m.sag[k]).abs() < tol,
                    "{name} {w:?} S {q:.1}: {ps} {}",
                    m.sag[k]
                );
                assert!(
                    (pt - m.tan[k]).abs() < tol,
                    "{name} {w:?} T {q:.1}: {pt} {}",
                    m.tan[k]
                );
            }
        }
    }
}

/// 斜めの視野: 既定（像面の上）の PSF のフーリエ変換は回折 MTF と合い、主光線に垂直な面の PSF は子午の周波数が
/// 1/cos ずれる（E1 の視野 3 は主光線の N = 0.8835）
#[test]
fn psf_transform_is_mtf_off_axis() {
    for (name, fi, tol) in [("A0", 2, 1e-3), ("B0", 2, 1e-3), ("D1", 2, 1e-3)] {
        let s = load(name);
        let f = FieldSel::Index(fi);
        let w = Some(s.primary);
        let o = PsfOpts {
            image: 256,
            ..Default::default()
        };
        let p = psf::psf(&s, f, Waves::all_or(w), o).unwrap();
        let q = psf::psf(
            &s,
            f,
            Waves::all_or(w),
            PsfOpts {
                plane: psf::PsfPlane::ChiefPerp,
                ..o
            },
        )
        .unwrap();
        let cut = analysis::mtf_cutoff(&s, f, s.primary).unwrap().tan;
        let fr: Vec<f64> = (1..10).map(|k| cut * k as f64 / 10.0).collect();
        let m = analysis::mtf(&s, f, Waves::all_or(w), None, &fr).unwrap();
        let (mut e, mut ep): (f64, f64) = (0.0, 0.0);
        for (k, &v) in fr.iter().enumerate() {
            let (ps, pt) = psf_mtf(&p, v);
            e = e.max((ps - m.sag[k]).abs()).max((pt - m.tan[k]).abs());
            ep = ep.max((psf_mtf(&q, v).1 - m.tan[k]).abs());
        }
        eprintln!(
            "{name} 視野 {}: 像面 {e:.2e} 垂直な面（子午）{ep:.2e}",
            fi + 1
        );
        assert!(e < tol, "{name} 視野 {}: {e:.2e}", fi + 1);
        // 子午の間隔の比が主光線の向きの余弦（垂直な面の方が細かい）
        let c = s
            .try_trace(f, &[0.0], &[0.0], Some(s.primary), None, false)
            .unwrap()
            .rays[0]
            .lmn[2];
        assert!(
            (q.pixel.1 / p.pixel.1 - c).abs() < 1e-3,
            "{name}: {} {c}",
            q.pixel.1 / p.pixel.1
        );
    }
}

/// 中心遮蔽の系で Huygens PSF と FFT PSF が同じ格子で合い、どちらのフーリエ変換も輪帯瞳の MTF の式と合う
#[test]
fn obscured_huygens_matches_fft_psf() {
    let e = 0.4;
    let s = ideal(5.0 * e);
    let f0 = FieldSel::Index(0);
    let f = psf::psf(&s, f0, Waves::All, PsfOpts::default()).unwrap();
    let h = huygens_psf(&s, f0, Waves::All, HuygensOpts::default()).unwrap();
    assert_eq!(f.i.x(), h.i.x());
    let d =
        f.i.data()
            .iter()
            .zip(h.i.data())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
    assert!(d < 2e-3, "{d:.2e}");
    // 収差がないのでストレール比は 1（遮蔽込みの収差のない同じ瞳で割る）
    assert!((f.peak - 1.0).abs() < 2e-3 && (h.peak - 1.0).abs() < 2e-3);
    let cut = 10.0 / (0.55e-3 * 100.0);
    for k in 1..10 {
        let q = cut * k as f64 / 10.0;
        let a = annular(e, q / cut);
        for (what, p) in [("fft", &f), ("huygens", &h)] {
            let (ps, pt) = psf_mtf(p, q);
            assert!(
                (ps - a).abs() < 7e-3 && (pt - a).abs() < 7e-3,
                "{what} {q:.1}: {ps} {pt} {a}"
            );
        }
    }
    // 遮蔽のない同じ系とは MTF の中域が違う（遮蔽が効いている）
    let p0 = psf::psf(&ideal(0.0), f0, Waves::All, PsfOpts::default()).unwrap();
    let mid = cut * 0.5;
    assert!((psf_mtf(&p0, mid).0 - psf_mtf(&f, mid).0).abs() > 0.05);
}

/// 焦点のずれた系を小さな瞳の格子で計算すると位相の標本不足を、小さな像の格子では格子の外の光量を知らせる。
/// 自動の瞳の格子なら知らせず、隣り合う標本の位相差は π/2 以下
#[test]
fn undersampling_is_reported() {
    let s = ideal_defocus(0.0, 1.0);
    let f0 = FieldSel::Index(0);
    let fixed = |pupil| PsfOpts {
        pupil: Some(pupil),
        image: 2 * pupil,
        pixel: None,
        ..Default::default()
    };
    let hfixed = |pupil| HuygensOpts {
        pupil: Some(pupil),
        image: 32,
        pixel: None,
    };
    let coarse = psf::psf(&s, f0, Waves::All, fixed(16)).unwrap();
    let hc = huygens_psf(&s, f0, Waves::All, hfixed(16)).unwrap();
    for (what, p) in [("fft", &coarse), ("huygens", &hc)] {
        assert!(
            p.phase_step > psf::PHASE_STEP_WARN,
            "{what}: {}",
            p.phase_step
        );
        assert!(!p.warnings.is_empty(), "{what}");
    }
    let auto = psf::psf(&s, f0, Waves::All, PsfOpts::default()).unwrap();
    let ha = huygens_psf(&s, f0, Waves::All, HuygensOpts::default()).unwrap();
    for (what, p) in [("fft", &auto), ("huygens", &ha)] {
        assert!(
            p.phase_step <= psf::PHASE_STEP_WARN,
            "{what}: {}",
            p.phase_step
        );
        assert!(p.warnings.is_empty(), "{what}: {:?}", p.warnings);
    }
    // 像の格子が PSF を覆えないと（焦点ずれで広がった像に 16 × 16）光量の割合が下がり、知らせる
    let small = PsfOpts {
        image: 16,
        ..Default::default()
    };
    let p = psf::psf(&s, f0, Waves::All, small).unwrap();
    assert!(p.enclosed < psf::ENCLOSED_WARN, "{}", p.enclosed);
    assert!(!p.warnings.is_empty());
}

/// 半径 a・b の円の中心が d 離れたときの重なりの面積
fn lens_area(a: f64, b: f64, d: f64) -> f64 {
    if d >= a + b {
        return 0.0;
    }
    if d <= (a - b).abs() {
        return PI * a.min(b).powi(2);
    }
    let ca = ((d * d + a * a - b * b) / (2.0 * d * a))
        .clamp(-1.0, 1.0)
        .acos();
    let cb = ((d * d + b * b - a * a) / (2.0 * d * b))
        .clamp(-1.0, 1.0)
        .acos();
    a * a * (ca - ca.sin() * ca.cos()) + b * b * (cb - cb.sin() * cb.cos())
}

/// 無収差の輪帯瞳（外径 1、内径 e）の MTF
fn annular(e: f64, v: f64) -> f64 {
    let d = 2.0 * v;
    let ov = lens_area(1.0, 1.0, d) - 2.0 * lens_area(1.0, e, d) + lens_area(e, e, d);
    ov / (PI * (1.0 - e * e))
}
