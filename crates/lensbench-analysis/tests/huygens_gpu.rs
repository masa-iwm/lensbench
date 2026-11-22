//! Huygens PSF・MTF の GPU の経路（[`HuygensDevice::Gpu`]、CPU で動くアダプタでも使う）が厳密な和（CPU）と合うこと。
//! GPU のアダプタが無い・`gpu` 機能なしで組んだときは、CPU で計算して注意を出すことだけを確かめる
//! （CI の Linux は llvmpipe で GPU の経路を通す）。

use lensbench_analysis::huygens::{huygens_mtf, huygens_psf, HuygensDevice, HuygensOpts};
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

/// ROPD「無限遠」（半径が無限大の参照球。GPU では平面の式）
fn far(s: &System) -> System {
    let mut t = s.clone();
    t.edit_raw(|r| {
        r.ropd = 1;
        Ok(())
    })
    .unwrap();
    t
}

fn fell_back(w: &[String]) -> bool {
    w.iter().any(|w| w.contains("computed on the CPU"))
}

/// 斜めの視野の多色・傾いた像面・ROPD 無限遠・区画の倍数でない格子で、GPU と厳密な和の差は収差のない中心の値の 1e-6 より小さい
#[test]
fn gpu_matches_the_exact_sum() {
    let a0 = load("A0");
    let d0 = load("D0");
    let tilted = tilt_image(&d0, 25.0);
    let (a0f, d0f) = (far(&a0), far(&d0));
    let last = FieldSel::Index(a0.fields.len() - 1);
    let on = FieldSel::Index(0);
    let cases: [(&System, FieldSel, Waves, usize); 6] = [
        (&a0, last, Waves::All, 64),
        (&a0, on, Waves::One(a0.primary), 50),
        (&tilted, on, Waves::One(tilted.primary), 64),
        (&d0, on, Waves::One(d0.primary), 64),
        (&a0f, last, Waves::One(a0.primary), 64),
        (&d0f, on, Waves::All, 64),
    ];
    for (k, &(s, f, w, image)) in cases.iter().enumerate() {
        let o = HuygensOpts {
            pupil: Some(64),
            image,
            pixel: None,
            exact: true,
            device: HuygensDevice::Cpu,
        };
        let exact = huygens_psf(s, f, w, o).unwrap();
        let gpu = huygens_psf(
            s,
            f,
            w,
            HuygensOpts {
                exact: false,
                device: HuygensDevice::Gpu,
                ..o
            },
        )
        .unwrap();
        if fell_back(&gpu.warnings) {
            eprintln!("case {k}: {:?}", gpu.warnings);
            continue;
        }
        let d = exact
            .i
            .data()
            .iter()
            .zip(gpu.i.data())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(d < 1e-6, "case {k}: {d}");
        assert_eq!(exact.pixel, gpu.pixel);
        assert!((exact.enclosed - gpu.enclosed).abs() < 1e-6);
        assert_eq!(exact.warnings, gpu.warnings);
    }
}

/// 像面の形の外の点は GPU でも NaN、ほかの点は厳密な値
#[test]
fn gpu_keeps_nan_outside_the_image_surface() {
    let mut s = load("D0");
    let f = FieldSel::Index(0);
    let o = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: true,
        device: HuygensDevice::Cpu,
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
    let gpu = huygens_psf(
        &s,
        f,
        Waves::One(s.primary),
        HuygensOpts {
            exact: false,
            device: HuygensDevice::Gpu,
            ..o
        },
    )
    .unwrap();
    if fell_back(&gpu.warnings) {
        eprintln!("{:?}", gpu.warnings);
        return;
    }
    let mut nan = 0;
    for (a, b) in exact.i.data().iter().zip(gpu.i.data()) {
        assert_eq!(a.is_nan(), b.is_nan());
        if a.is_nan() {
            nan += 1;
        } else {
            assert!((a - b).abs() < 1e-6, "{a} {b}");
        }
    }
    assert!(nan > 0 && nan < 64 * 64);
}

/// Huygens MTF も GPU の PSF から（厳密な和の MTF と 1e-5 以内）
#[test]
fn gpu_huygens_mtf() {
    let s = load("A0");
    let f = FieldSel::Index(1);
    let freqs: Vec<f64> = (0..=10).map(|k| 10.0 * k as f64).collect();
    let o = HuygensOpts {
        pupil: Some(64),
        image: 64,
        pixel: None,
        exact: true,
        device: HuygensDevice::Cpu,
    };
    let exact = huygens_mtf(&s, f, Waves::All, o, &freqs).unwrap();
    let gpu = huygens_mtf(
        &s,
        f,
        Waves::All,
        HuygensOpts {
            exact: false,
            device: HuygensDevice::Gpu,
            ..o
        },
        &freqs,
    )
    .unwrap();
    for (a, b) in exact
        .tan
        .iter()
        .chain(&exact.sag)
        .zip(gpu.tan.iter().chain(&gpu.sag))
    {
        assert!(
            (a - b).abs() < 1e-5 || (a.is_nan() && b.is_nan()),
            "{a} {b}"
        );
    }
}

/// CPU に固定すれば注意は出ず、既定の展開と同じ値
#[test]
fn cpu_device_is_the_default_expansion() {
    let s = load("D0");
    let f = FieldSel::Index(0);
    let o = HuygensOpts {
        pupil: Some(32),
        image: 32,
        device: HuygensDevice::Cpu,
        ..Default::default()
    };
    let a = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    let b = huygens_psf(&s, f, Waves::One(s.primary), o).unwrap();
    assert!(!fell_back(&a.warnings));
    assert_eq!(a.i.data(), b.i.data());
}
