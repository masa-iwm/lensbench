//! Huygens PSF・Huygens MTFの性質: 軸上・斜めの視野で FFT PSF（像面の上）と一致、収差のない系で回折限界、傾いた像面で
//! 像面の上の座標（主光線に垂直な面の 1/cos 倍に伸びる）になること。OpticStudio との照合は huygens_refs.rs。

use lensbench_analysis::analysis;
use lensbench_analysis::huygens::{huygens_mtf, huygens_mtf_axes, huygens_psf, HuygensOpts};
use lensbench_analysis::psf::{self, diffraction_limit, PsfOpts};
use lensbench_core::grid::Grid2;
use lensbench_core::surface::Surface;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::Waves;
use lensbench_formats::zmx::{self, LoadOptions};
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
    t.insert_surface(
        at,
        Surface {
            form: lensbench_core::surface::Form::CoordBrk {
                cb: [0.0, 0.0, deg, 0.0, 0.0],
                order: 0,
            },
            ..Default::default()
        },
    )
    .unwrap();
    t
}

fn max_abs_diff(a: &Grid2, b: &Grid2) -> f64 {
    a.data()
        .iter()
        .zip(b.data())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn on_axis_matches_fft_psf() {
    // 軸上では主光線に垂直な面 = 像面なので、FFT PSF と同じ格子で比べられる（差は瞳から像への写像の一次近似の分）
    for (name, tol) in [("A0", 1e-3), ("B0", 1e-3), ("D0", 2e-3), ("D1", 2e-3)] {
        let s = load(name);
        for w in [Some(s.primary), None] {
            let f = psf::psf(&s, FieldSel::Index(0), Waves::all_or(w), PsfOpts::default()).unwrap();
            let h = huygens_psf(
                &s,
                FieldSel::Index(0),
                Waves::all_or(w),
                HuygensOpts::default(),
            )
            .unwrap();
            assert_eq!(f.i.x(), h.i.x());
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
    // 斜めの視野でも FFT PSF の既定は像面の上なので、同じ格子で比べられる（主光線に垂直な面の格子は
    // 1/cos だけ細かく、差が大きい）
    let s = load("A0");
    for fi in 1..s.fields.len() {
        let f = psf::psf(&s, FieldSel::Index(fi), Waves::All, PsfOpts::default()).unwrap();
        let h = huygens_psf(&s, FieldSel::Index(fi), Waves::All, HuygensOpts::default()).unwrap();
        assert_eq!(f.i.x(), h.i.x());
        assert_eq!(f.i.y(), h.i.y());
        let d = max_abs_diff(&f.i, &h.i);
        assert!(d < 2e-3, "A0 視野 {}: {d:.2e}", fi + 1);
        assert_eq!(h.center, f.center);
        assert_eq!(f.plane, psf::PsfPlane::Image);
        let perp = PsfOpts {
            plane: psf::PsfPlane::ChiefPerp,
            ..Default::default()
        };
        let fp = psf::psf(&s, FieldSel::Index(fi), Waves::All, perp).unwrap();
        assert!(fp.pixel.1 < f.pixel.1);
    }
}

#[test]
fn perfect_lens_gives_diffraction_limit() {
    // D0 は理想レンズ（F/5）。正規化瞳で振幅が一様なので、瞳の縁の向きの余弦が tan の分だけ詰まり、
    // MTF は回折限界から 2e-3 ほど動く（FFT の MTF は一次近似なので回折限界そのもの）
    let s = load("D0");
    let f = FieldSel::Index(0);
    let c = analysis::mtf_cutoff(&s, f, s.primary).unwrap().sag;
    let fs: Vec<f64> = (0..=10).map(|k| 0.95 * c * k as f64 / 10.0).collect();
    let o = HuygensOpts {
        pupil: Some(128),
        image: 256,
        pixel: None,
        exact: false,
    };
    let h = huygens_mtf(&s, f, Waves::One(s.primary), o, &fs).unwrap();
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
    // 中心の行（x）は FFT PSF（主光線に垂直な面）と同じ間隔、中心の列（y）は像面の Δ が垂直な面の Δ cos に当たる
    let perp = PsfOpts {
        plane: psf::PsfPlane::ChiefPerp,
        ..Default::default()
    };
    let fp = psf::psf(&s, f, Waves::One(s.primary), perp).unwrap();
    let d = fp.pixel.0;
    let fq = psf::psf(
        &s,
        f,
        Waves::One(s.primary),
        PsfOpts {
            pixel: Some(d * cs),
            ..perp
        },
    )
    .unwrap();
    let o = HuygensOpts {
        pupil: Some(64),
        image: 128,
        pixel: Some(d),
        exact: false,
    };
    let hp = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let c0 = 64;
    let (mut ex, mut ey): (f64, f64) = (0.0, 0.0);
    for m in 0..128 {
        ex = ex.max((hp.i[c0][m] - fp.i[c0][m]).abs());
        ey = ey.max((hp.i[m][c0] - fq.i[m][c0]).abs());
    }
    assert!(ex < 3e-3 && ey < 3e-3, "x {ex:.2e} y {ey:.2e}");
    // 既定の FFT PSF（像面の上）は Huygens PSF と同じ格子で全体が合う
    let fi = psf::psf(
        &s,
        f,
        Waves::One(s.primary),
        PsfOpts {
            pixel: Some(d),
            ..Default::default()
        },
    )
    .unwrap();
    let e = max_abs_diff(&fi.i, &hp.i);
    assert!(e < 3e-3, "像面の上 {e:.2e}");
    // MTF: 像面の上の周波数 ν は主光線に垂直な面の ν / cos（子午）
    let c = analysis::mtf_cutoff(&s0, f, s.primary).unwrap().sag;
    let fs: Vec<f64> = (0..=8).map(|k| 0.9 * c * cs * k as f64 / 8.0).collect();
    let o = HuygensOpts {
        pupil: Some(128),
        image: 256,
        pixel: None,
        exact: false,
    };
    let h = huygens_mtf(&s, f, Waves::One(s.primary), o, &fs).unwrap();
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
        pupil: Some(2),
        image: 16,
        pixel: None,
        exact: false,
    };
    assert!(huygens_psf(&s, FieldSel::Index(0), Waves::All, o).is_err());
    // 像の格子が瞳の標本の周期を超える
    let o = HuygensOpts {
        pupil: Some(16),
        image: 64,
        pixel: None,
        exact: false,
    };
    assert!(huygens_psf(&s, FieldSel::Index(0), Waves::All, o).is_err());
}

/// 像の格子のナイキスト 1/(2Δ) を超える周波数は、折り返した値でなく NaN
#[test]
fn huygens_mtf_above_nyquist_is_nan() {
    let s = load("D0");
    let f = FieldSel::Index(0);
    let o = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: false,
    };
    let p = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let ny = 0.5 / p.pixel.0;
    let h = huygens_mtf(&s, f, Waves::One(s.primary), o, &[0.5 * ny, 1.5 * ny]).unwrap();
    assert!(h.sag[0].is_finite() && h.tan[0].is_finite());
    assert!(h.sag[1].is_nan() && h.tan[1].is_nan(), "{:?}", h.sag);
}

/// 像面の形の外（矢高が求まらない）格子の点は平面として計算せず、強度を NaN にして注意を出す
#[test]
fn huygens_points_outside_the_image_surface_are_nan() {
    let mut s = load("D0");
    let f = FieldSel::Index(0);
    let o = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: false,
    };
    let p0 = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    // 半径が格子の幅の 1/4 の球面の像面: 格子の外側の点は球の外
    let r = 16.0 * p0.pixel.0;
    let img = s.image();
    s.edit_raw(|rx| {
        rx.surfaces[img].c = 1.0 / r;
        Ok(())
    })
    .unwrap();
    let p = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let rows: Vec<&[f64]> = p.i.rows().collect();
    assert!(rows[0][0].is_nan(), "{}", rows[0][0]);
    assert!(rows[32][32].is_finite() && p.peak > 0.0);
    assert!(
        p.warnings.iter().any(|w| w.contains("intensity is NaN")),
        "{:?}",
        p.warnings
    );
    assert!(p.enclosed.is_finite());
    // その PSF の MTF はどの周波数も決まらない: 変換は全部 NaN（調和周波数の間も 0 にしない）、
    // huygens_mtf は理由つきの誤り
    let ny = 0.5 / p.pixel.0;
    let fs = [0.0, 0.1 * ny, 0.37 * ny, 0.5 * ny];
    let m = lensbench_analysis::huygens::psf_mtf(&p, &fs);
    assert!(
        m.tan.iter().chain(&m.sag).all(|v| v.is_nan()),
        "{:?} {:?}",
        m.tan,
        m.sag
    );
    let e = huygens_mtf(&s, f, Waves::One(s.primary), o, &fs)
        .unwrap_err()
        .to_string();
    assert!(e.contains("undefined at"), "{e}");
    // NaN の無い PSF では有限（間の周波数も）
    let m0 = lensbench_analysis::huygens::psf_mtf(&p0, &fs);
    assert!(m0.tan.iter().chain(&m0.sag).all(|v| v.is_finite()));
}

/// 理想レンズ（f 100、入射瞳径 10、λ 0.55 µm）で視野 (x, y)° の系
fn ideal_lens_field(x: f64, y: f64) -> System {
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 {x}\nYFLN 0 {y}\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ 100\nSURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap()
}

#[test]
fn skew_fields_match_huygens() {
    // x・y の両方に角度のある視野では J = ∂(L, M)/∂(px, py) に非対角の項が出る。像面の上の FFT PSF は瞳の格子を
    // 写してそれを位相に入れる（psf::image_shear）。同じ画角の y だけの視野と同じくらい Huygens に合う
    let o = PsfOpts {
        pupil: Some(64),
        image: 64,
        ..Default::default()
    };
    let ho = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: false,
    };
    let f1 = FieldSel::Index(1);
    let mut worst = Vec::new();
    for (x, y) in [(20.0, 20.0), (14.6, 14.6), (0.0, 28.0), (-12.0, 17.0)] {
        let s = ideal_lens_field(x, y);
        let f = psf::psf(&s, f1, Waves::All, o).unwrap();
        let h = huygens_psf(&s, f1, Waves::All, ho).unwrap();
        assert_eq!(f.i.x(), h.i.x());
        assert_eq!(f.i.y(), h.i.y());
        let d = max_abs_diff(&f.i, &h.i);
        let dp = (f.peak - h.peak).abs() / h.peak;
        worst.push((x, y, d, dp));
    }
    for (x, y, d, dp) in worst {
        assert!(d < 2e-3 && dp < 2e-3, "({x}, {y}): {d:.2e} {dp:.2e}");
    }
    // 公開の D1 の視野 (3, −4) もほかの視野と同じくらい
    let s = load("D1");
    for fi in 0..s.fields.len() {
        let f = psf::psf(&s, FieldSel::Index(fi), Waves::All, PsfOpts::default()).unwrap();
        let h = huygens_psf(&s, FieldSel::Index(fi), Waves::All, HuygensOpts::default()).unwrap();
        let d = max_abs_diff(&f.i, &h.i);
        assert!(d < 2e-3, "D1 視野 {}: {d:.2e}", fi + 1);
    }
}

/// 子午・球欠の向き: 既定（視野の向き）では、回転対称な系の Huygens MTF は視野の方位によらず、回折 MTF との
/// 差も y の視野と同じ（斜めの視野でも）
#[test]
fn huygens_mtf_follows_the_field_direction() {
    let mut s = load("A0");
    let h: f64 = 14.0;
    let d = (h.to_radians().tan() / 2f64.sqrt()).atan().to_degrees();
    s.edit_raw(|rx| {
        let f0 = rx.fields[0];
        rx.fields = [(0.0, h), (d, d), (h, 0.0)]
            .iter()
            .map(|&(x, y)| {
                let mut f = f0;
                (f.x, f.y) = (x, y);
                f
            })
            .collect();
        Ok(())
    })
    .unwrap();
    let fr = [10.0, 30.0, 60.0];
    let o = HuygensOpts {
        image: 256,
        ..Default::default()
    };
    let hm = |i: usize| {
        let f = FieldSel::Index(i);
        huygens_mtf_axes(&s, f, Waves::All, o, &fr, None, Default::default()).unwrap()
    };
    // 回折 MTF との差（像の格子の大きさ・像面の上か瞳の座標かの違い。y の視野でも 5e-3 ほど）も方位によらない
    let dm = |i: usize| analysis::mtf(&s, FieldSel::Index(i), Waves::All, None, &fr).unwrap();
    let (y, dy) = (hm(0), dm(0));
    for i in 1..3 {
        let (m, d) = (hm(i), dm(i));
        for k in 0..fr.len() {
            let e = [m.tan[k] - y.tan[k], m.sag[k] - y.sag[k]];
            assert!(
                e[0].abs() < 3e-3 && e[1].abs() < 3e-3,
                "field {} {}: {e:?}",
                i + 1,
                fr[k]
            );
            let e = [
                (m.tan[k] - d.tan[k]) - (y.tan[k] - dy.tan[k]),
                (m.sag[k] - d.sag[k]) - (y.sag[k] - dy.sag[k]),
            ];
            assert!(
                e[0].abs() < 1.5e-3 && e[1].abs() < 1.5e-3,
                "field {} {}: {e:?}",
                i + 1,
                fr[k]
            );
        }
    }
}

/// 既定の計算（位相・振幅を区画ごとに 2 次式で）は厳密な和（exact）と 1e-6 より近い。斜めの視野の多色、傾いた像面、
/// 区画の倍数でない格子、粗い間隔（区画が小さくなる）で
#[test]
fn expand_is_close_to_the_exact_sum() {
    let a0 = load("A0");
    let tilted = tilt_image(&load("D0"), 25.0);
    let last = FieldSel::Index(a0.fields.len() - 1);
    let on = FieldSel::Index(0);
    let cases: [(&System, FieldSel, Waves, usize, Option<f64>); 4] = [
        (&a0, last, Waves::All, 64, None),
        (&a0, on, Waves::One(a0.primary), 50, None),
        (&tilted, on, Waves::One(tilted.primary), 64, None),
        (&a0, on, Waves::One(a0.primary), 37, Some(0.004)),
    ];
    for (k, &(s, f, w, image, pixel)) in cases.iter().enumerate() {
        let o = HuygensOpts {
            pupil: Some(64),
            image,
            pixel,
            exact: true,
        };
        let exact = huygens_psf(s, f, w, o).unwrap();
        let fast = huygens_psf(s, f, w, HuygensOpts { exact: false, ..o }).unwrap();
        let d = max_abs_diff(&exact.i, &fast.i);
        assert!(d < 1e-6, "case {k}: {d}");
        assert!(d > 0.0, "case {k}: the default took the exact path");
        assert_eq!(exact.pixel, fast.pixel);
        assert!((exact.enclosed - fast.enclosed).abs() < 1e-6);
    }
}

/// 既定の計算でも、像面の形の外の点は NaN のまま、その区画のほかの点は厳密な値
#[test]
fn expand_keeps_nan_outside_the_image_surface() {
    let mut s = load("D0");
    let f = FieldSel::Index(0);
    let o = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: true,
    };
    let p0 = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let r = 16.0 * p0.pixel.0;
    let img = s.image();
    s.edit_raw(|rx| {
        rx.surfaces[img].c = 1.0 / r;
        Ok(())
    })
    .unwrap();
    let exact = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let fast = huygens_psf(
        &s,
        f,
        Waves::One(s.primary),
        HuygensOpts { exact: false, ..o },
    )
    .unwrap();
    let mut nan = 0;
    for (a, b) in exact.i.data().iter().zip(fast.i.data()) {
        assert_eq!(a.is_nan(), b.is_nan());
        if a.is_nan() {
            nan += 1;
        } else {
            assert!((a - b).abs() < 1e-6, "{a} {b}");
        }
    }
    assert!(nan > 0 && nan < 64 * 64);
    assert_eq!(exact.warnings, fast.warnings);
}
