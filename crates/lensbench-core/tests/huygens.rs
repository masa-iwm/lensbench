//! Huygens PSF・Huygens MTFの性質: 軸上で FFT PSF と一致、収差のない系で回折限界、傾いた像面で
//! 像面の上の座標（主光線に垂直な面の 1/cos 倍に伸びる）になること。OpticStudio との照合は mtf_family_refs.rs。

use lensbench_core::analysis;
use lensbench_core::huygens::{huygens_mtf, huygens_psf, HuygensOpts};
use lensbench_core::psf::{self, diffraction_limit, PsfOpts};
use lensbench_core::surface::{SurfType, Surface};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
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

/// 像面の前に x 軸まわりに deg 傾ける座標ブレークを入れる
fn tilt_image(s: &System, deg: f64) -> System {
    let mut t = s.clone();
    let at = t.image();
    let n = t.surfaces[at - 1].n.clone();
    t.insert_surface(
        at,
        Surface {
            typ: SurfType::CoordBrk,
            cb: [0.0, 0.0, deg, 0.0, 0.0],
            n,
            ..Default::default()
        },
    )
    .unwrap();
    t
}

fn max_abs_diff(a: &[Vec<f64>], b: &[Vec<f64>]) -> f64 {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn on_axis_matches_fft_psf() {
    // 軸上では主光線に垂直な面 = 像面なので、FFT PSF と同じ格子で比べられる（差は瞳から像への写像の一次近似の分）
    for (name, tol) in [("A0", 1e-3), ("B0", 1e-3), ("D0", 2e-3), ("D1", 2e-3)] {
        let s = load(name);
        for w in [Some(s.primary), None] {
            let f = psf::psf(&s, FieldSel::Index(0), w, PsfOpts::default()).unwrap();
            let h = huygens_psf(&s, FieldSel::Index(0), w, HuygensOpts::default()).unwrap();
            assert_eq!(f.x, h.x);
            let d = max_abs_diff(&f.i, &h.i);
            assert!(d < tol, "{name} {w:?}: {d:.2e}");
            assert!(
                (f.peak - h.peak).abs() < tol,
                "{name}: {} {}",
                f.peak,
                h.peak
            );
        }
    }
}

#[test]
fn off_axis_close_to_fft_psf() {
    // 斜めの視野では像面上の座標なので 1/cos だけ違う（A0 の視野は小さく、差は数 1e-3）
    let s = load("A0");
    for fi in 1..s.fields.len() {
        let f = psf::psf(&s, FieldSel::Index(fi), None, PsfOpts::default()).unwrap();
        let h = huygens_psf(&s, FieldSel::Index(fi), None, HuygensOpts::default()).unwrap();
        let d = max_abs_diff(&f.i, &h.i);
        assert!(d < 1e-2, "A0 視野 {}: {d:.2e}", fi + 1);
        assert_eq!(h.center, f.center);
    }
}

#[test]
fn perfect_lens_gives_diffraction_limit() {
    // D0 は理想レンズ（F/5）。正規化瞳で振幅が一様なので、瞳の縁の向きの余弦が tan の分だけ詰まり、
    // MTF は回折限界から 2e-3 ほど動く（FFT の MTF は一次近似なので回折限界そのもの）
    let s = load("D0");
    let f = FieldSel::Index(0);
    let m = analysis::mtf(&s, f, Some(s.primary), None, &[]).unwrap();
    let c = m.cutoff.0;
    let fs: Vec<f64> = (0..=10).map(|k| 0.95 * c * k as f64 / 10.0).collect();
    let o = HuygensOpts {
        pupil: 128,
        image: 256,
        delta: None,
    };
    let h = huygens_mtf(&s, f, Some(s.primary), o, &fs).unwrap();
    assert!((h.tan[0] - 1.0).abs() < 1e-12 && (h.sag[0] - 1.0).abs() < 1e-12);
    for (k, &q) in fs.iter().enumerate() {
        let dl = diffraction_limit(q / c);
        assert!((h.tan[k] - dl).abs() < 3e-3, "T {q}: {} {dl}", h.tan[k]);
        assert!((h.sag[k] - dl).abs() < 3e-3, "S {q}: {} {dl}", h.sag[k]);
    }
}

#[test]
fn tilted_image_plane_stretches_by_one_over_cos() {
    let s0 = load("D0");
    let deg: f64 = 25.0;
    let s = tilt_image(&s0, deg);
    let cs = deg.to_radians().cos();
    let f = FieldSel::Index(0);
    // 中心の行（x）は FFT PSF と同じ間隔、中心の列（y）は像面の Δ が垂直な面の Δ cos に当たる
    let fp = psf::psf(&s, f, Some(s.primary), PsfOpts::default()).unwrap();
    let d = fp.delta.0;
    let fq = psf::psf(
        &s,
        f,
        Some(s.primary),
        PsfOpts {
            delta: Some(d * cs),
            ..Default::default()
        },
    )
    .unwrap();
    let o = HuygensOpts {
        pupil: 64,
        image: 128,
        delta: Some(d),
    };
    let hp = huygens_psf(&s, f, Some(s.primary), o).unwrap();
    let c0 = 64;
    let (mut ex, mut ey): (f64, f64) = (0.0, 0.0);
    for m in 0..128 {
        ex = ex.max((hp.i[c0][m] - fp.i[c0][m]).abs());
        ey = ey.max((hp.i[m][c0] - fq.i[m][c0]).abs());
    }
    assert!(ex < 3e-3 && ey < 3e-3, "x {ex:.2e} y {ey:.2e}");
    // MTF: 像面の上の周波数 ν は主光線に垂直な面の ν / cos（子午）
    let m = analysis::mtf(&s0, f, Some(s.primary), None, &[]).unwrap();
    let c = m.cutoff.0;
    let fs: Vec<f64> = (0..=8).map(|k| 0.9 * c * cs * k as f64 / 8.0).collect();
    let o = HuygensOpts {
        pupil: 128,
        image: 256,
        delta: None,
    };
    let h = huygens_mtf(&s, f, Some(s.primary), o, &fs).unwrap();
    for (k, &q) in fs.iter().enumerate() {
        let t = diffraction_limit(q / (c * cs));
        let sg = diffraction_limit(q / c);
        assert!((h.tan[k] - t).abs() < 6e-3, "T {q}: {} {t}", h.tan[k]);
        assert!((h.sag[k] - sg).abs() < 6e-3, "S {q}: {} {sg}", h.sag[k]);
    }
}

#[test]
fn rejects_bad_grids() {
    let s = load("A0");
    let o = HuygensOpts {
        pupil: 2,
        image: 16,
        delta: None,
    };
    assert!(huygens_psf(&s, FieldSel::Index(0), None, o).is_err());
    // 像の格子が瞳の標本の周期を超える
    let o = HuygensOpts {
        pupil: 16,
        image: 64,
        delta: None,
    };
    assert!(huygens_psf(&s, FieldSel::Index(0), None, o).is_err());
}
