//! 光線を数える検出器（irradiance）と重み付きのスポット: 検出器の和が重みの和に合う、重みの無い追跡は本数を数える

use crate::common::root;
use lensbench_analysis::irradiance::{detect, spot_moments, DetectorOpts};
use lensbench_core::system::{FieldSel, Launch, Trace};
use lensbench_core::usersource::SourceRays;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};

fn load(name: &str) -> System {
    zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

/// 瞳の 21 × 21 の格子（縁の外も含み、一部は届かない）の光線を開始位置・方向で追い直したもの
fn rays(s: &System) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for j in 0..21 {
        for i in 0..21 {
            px.push(-1.2 + 2.4 * i as f64 / 20.0);
            py.push(-1.2 + 2.4 * j as f64 / 20.0);
        }
    }
    let t = s
        .try_trace(FieldSel::Index(1), &px, &py, Some(s.primary), None, false)
        .unwrap();
    (t.p0, t.k0)
}

fn traced(s: &System, w: Option<Vec<f64>>) -> Trace {
    let (p, k) = rays(s);
    let t = s.trace_launch(&Launch::Rays { p: &p, k: &k }, s.primary, None, false);
    match w {
        Some(w) => t.with_weights(w).unwrap(),
        None => t,
    }
}

fn weights(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.25 + (i % 7) as f64 * 0.5).collect()
}

/// 重み付きの光線: 画素の和 = 届いた光線の重みの和（検出面は届いた光線を全部覆う）、launched = 全部の重みの和
#[test]
fn the_detector_adds_up_the_weights() {
    let s = load("A0");
    let n = rays(&s).0.len();
    let w = weights(n);
    let t = traced(&s, Some(w.clone()));
    let ok = (0..n).filter(|&i| t.ok(i)).count();
    assert!(ok > 100, "{ok} of {n} rays arrive");
    let d = detect(&t, &DetectorOpts::default()).unwrap();
    let want: f64 = (0..n).filter(|&i| t.ok(i)).map(|i| w[i]).sum();
    assert!(
        (d.detected - want).abs() < 1e-12 * want,
        "{} {want}",
        d.detected
    );
    assert_eq!(d.detected, d.arrived);
    assert!((d.launched - w.iter().sum::<f64>()).abs() < 1e-12 * d.launched);
    assert_eq!(d.hits.data().iter().sum::<usize>(), ok);
    let e: f64 = d.irradiance.data().iter().sum::<f64>() * d.dx * d.dx;
    assert!((e - want).abs() < 1e-12 * want);
}

/// 重みの無い追跡は光線の本数を数える
#[test]
fn without_weights_the_detector_counts_rays() {
    let s = load("A0");
    let t = traced(&s, None);
    let ok = (0..t.rays.len()).filter(|&i| t.ok(i)).count();
    let d = detect(
        &t,
        &DetectorOpts {
            npx: 16,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(d.detected, ok as f64);
    assert_eq!(d.launched, t.rays.len() as f64);
}

/// 光源の全パワーを分けたパワーを掛けると、検出器の和は全パワー × 届いた重みの割合 [W]
#[test]
fn source_power_gives_watts() {
    let s = load("A0");
    let n = rays(&s).0.len();
    let w = weights(n);
    let src = SourceRays {
        p: vec![[0.0; 3]; n],
        k: vec![[0.0, 0.0, 1.0]; n],
        w: w.clone(),
    };
    let p = src.powers(2.0).unwrap();
    let t = traced(&s, None);
    let d = detect(
        &t,
        &DetectorOpts {
            scale: Some(p),
            ..Default::default()
        },
    )
    .unwrap();
    assert!((d.launched - 2.0).abs() < 1e-14);
    let frac: f64 = (0..n).filter(|&i| t.ok(i)).map(|i| w[i]).sum::<f64>() / w.iter().sum::<f64>();
    assert!((d.detected - 2.0 * frac).abs() < 1e-14);
}

/// 中心・幅を決めると外の光線は数えず、光線は中心から ±dx/2 の画素に入る
#[test]
fn a_fixed_area_bins_by_pixel_centres() {
    let s = load("A0");
    let t = traced(&s, Some(weights(441)));
    let d = detect(
        &t,
        &DetectorOpts {
            npx: 8,
            center: Some((0.0, 0.0)),
            width: Some(1e-3),
            scale: None,
        },
    )
    .unwrap();
    assert!(d.detected < d.arrived);
    assert!((d.dx - 1.25e-4).abs() < 1e-18);
    assert!((d.power.x()[0] - (-0.5e-3 + 0.625e-4)).abs() < 1e-15);
    for i in (0..t.rays.len()).filter(|&i| t.ok(i)) {
        let p = t.rays[i].xyz;
        let (u, v) = ((p[0] + 0.5e-3) / d.dx, (p[1] + 0.5e-3) / d.dx);
        if (0.0..8.0).contains(&u) && (0.0..8.0).contains(&v) {
            assert!(d.hits.data()[v as usize * 8 + u as usize] > 0);
        }
    }
    assert!(detect(
        &t,
        &DetectorOpts {
            npx: 0,
            ..Default::default()
        }
    )
    .is_err());
    let bad = DetectorOpts {
        scale: Some(vec![1.0; 3]),
        ..Default::default()
    };
    assert!(detect(&t, &bad).is_err());
}

/// 重み付きの重心・RMS: 重みが全部同じなら単純な平均、重みを定数倍しても同じ
#[test]
fn weighted_spot_moments() {
    let s = load("A0");
    let n = 441;
    let t1 = traced(&s, None);
    let ok: Vec<usize> = (0..n).filter(|&i| t1.ok(i)).collect();
    let m = ok.len() as f64;
    let cx = ok.iter().map(|&i| t1.rays[i].xyz[0]).sum::<f64>() / m;
    let cy = ok.iter().map(|&i| t1.rays[i].xyz[1]).sum::<f64>() / m;
    let rms = (ok
        .iter()
        .map(|&i| (t1.rays[i].xyz[0] - cx).powi(2) + (t1.rays[i].xyz[1] - cy).powi(2))
        .sum::<f64>()
        / m)
        .sqrt();
    let a = spot_moments(&t1).unwrap();
    assert!((a.centroid.0 - cx).abs() < 1e-12 && (a.centroid.1 - cy).abs() < 1e-12);
    assert!((a.rms - rms).abs() < 1e-12);
    assert_eq!(a.weight, m);
    let w = weights(n);
    let b = spot_moments(&traced(&s, Some(w.clone()))).unwrap();
    let c = spot_moments(&traced(&s, Some(w.iter().map(|x| 3.0 * x).collect()))).unwrap();
    assert!((b.centroid.1 - c.centroid.1).abs() < 1e-12 && (b.rms - c.rms).abs() < 1e-12);
    let sw: f64 = ok.iter().map(|&i| w[i]).sum();
    let wy = ok.iter().map(|&i| w[i] * t1.rays[i].xyz[1]).sum::<f64>() / sw;
    assert!((b.centroid.1 - wy).abs() < 1e-12);
    assert!(spot_moments(&traced(&s, Some(vec![0.0; n]))).is_none());
}
