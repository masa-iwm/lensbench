//! 偏光追跡の性質の確認（公開のテスト光学系 A0）。OpticStudio の偏光追跡との照合は、市販の膜・ガラスを
//! 含む非公開の基準（Jones 行列、薄膜の鏡、表、暗号化された膜の実測表）で行う。

use lensbench_core::coating::{Coatings, Measured, C64};
use lensbench_core::surface::SurfType;
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{Error, System};
use std::path::PathBuf;

fn a0() -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

const X: [C64; 2] = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];

/// 屈折率の変わる面（境界）
fn interfaces(s: &System, w: usize) -> Vec<(usize, f64, f64)> {
    (1..s.surfaces.len())
        .filter(|&j| !s.surfaces[j].ignore && s.surfaces[j].typ != SurfType::CoordBrk)
        .map(|j| (j, s.index_before(j, w), s.surfaces[j].n[w]))
        .filter(|&(_, a, b)| a != b)
        .collect()
}

#[test]
fn axial_ray_has_the_normal_incidence_fresnel_transmission() {
    let s = a0();
    let w = s.primary;
    let c = Coatings::for_system(&s).unwrap();
    let t = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    let ifs = interfaces(&s, w);
    assert!(ifs.len() >= 4);
    let amp: f64 = ifs
        .iter()
        .map(|&(_, a, b)| 2.0 * (a * b).sqrt() / (a + b))
        .product();
    let e = t.e[0][0];
    let kw = 2.0 * std::f64::consts::PI / (s.wavelengths[w] * 1e-3);
    let ph = C64::new(0.0, -kw * t.trace.rays[0].opl).exp();
    assert!((e[0] - ph * amp).norm() < 1e-14, "{e:?} {amp}");
    assert!(e[1].norm() < 1e-15 && e[2].norm() < 1e-15);
}

#[test]
fn ideal_coatings_scale_every_ray_alike() {
    let mut s = a0();
    let w = s.primary;
    let ifs = interfaces(&s, w);
    for &(j, _, _) in &ifs {
        s.surfaces[j].coat = "I.95".into();
    }
    let c = Coatings::for_system(&s).unwrap();
    let px = [0.0, 0.7, -0.3, 0.0];
    let py = [0.0, 0.0, 0.5, -0.9];
    let ys = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
    let t = s
        .try_trace_pol(FieldSel::Index(2), &px, &py, None, &[X, ys], &c)
        .unwrap();
    let amp = 0.95f64.sqrt().powi(ifs.len() as i32);
    for i in 0..px.len() {
        assert!(t.trace.ok(i));
        for m in 0..2 {
            let n2: f64 = t.e[i][m].iter().map(|v| v.norm_sqr()).sum();
            // 入力は光線に垂直な面へ射影しただけ（規格化しない）なので、斜めの光線では 1 より少し小さい
            let k0 = t.trace.k0[i];
            let inp = 1.0 - k0[m] * k0[m];
            assert!((n2 - amp * amp * inp).abs() < 1e-13, "{i} {m} {n2}");
        }
    }
}

#[test]
fn absorption_follows_the_internal_transmittance() {
    let mut s = a0();
    let w = s.primary;
    let wl = s.wavelengths[w];
    let c = Coatings::for_system(&s).unwrap();
    let t0 = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    // 最初のガラスに、厚さ 10 mm で内部透過率 0.9 のデータを足す
    let j = interfaces(&s, w)[0].0;
    let mat = s.surfaces[j].mat.clone();
    // 同じガラスの玉すべての軸上の厚さ
    let thick: f64 = (1..s.surfaces.len())
        .filter(|&q| s.surfaces[q].mat == mat && !s.surfaces[q].mirror)
        .map(|q| s.surfaces[q].t)
        .sum();
    s.glasses.get_mut(&mat).unwrap().it = vec![[wl - 0.1, 0.9, 10.0], [wl + 0.1, 0.9, 10.0]];
    let t1 = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    let ratio = t1.e[0][0][0].norm() / t0.e[0][0][0].norm();
    let expect = 0.9f64.powf(thick / 10.0 / 2.0);
    assert!((ratio - expect).abs() < 1e-14, "{ratio} {expect}");
}

#[test]
fn quarter_wave_jones_surface_makes_circular_polarization() {
    let mut s = a0();
    let img = s.image();
    // 像面を λ/4（速軸 x）の Jones 行列面にする
    s.surfaces[img].typ = SurfType::Jones;
    s.surfaces[img].jones = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let c = Coatings::for_system(&s).unwrap();
    let d45 = [C64::new(0.5f64.sqrt(), 0.0), C64::new(0.5f64.sqrt(), 0.0)];
    let t = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[d45], &c)
        .unwrap();
    let e = t.e[0][0];
    let r = e[1] / e[0];
    assert!((r - C64::new(0.0, 1.0)).norm() < 1e-14, "{r}");
}

#[test]
fn measured_tables_are_used_for_unknown_coatings() {
    let mut s = a0();
    let w = s.primary;
    let (j, _, _) = interfaces(&s, w)[0];
    s.surfaces[j].coat = "ZEC1".into();
    let mut c = Coatings::for_system(&s).unwrap();
    // 表がなければ止める
    let r = s.try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c);
    assert!(matches!(r, Err(Error::NotImplemented(_))), "{r:?}");
    let key = format!("ZEC1|AIR|{}", s.surfaces[j].mat);
    let ang: Vec<f64> = (0..=60).map(|a| a as f64).collect();
    let v = [C64::new(0.0, 0.9), C64::new(0.8, 0.0)];
    c.add_measured(
        &key,
        Measured {
            wavelengths: vec![s.wavelengths[w]],
            angles_deg: ang.clone(),
            data: vec![vec![v; ang.len()]],
        },
    )
    .unwrap();
    let t = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    // 面 j だけ表の係数、他は膜なしのフレネル。軸上では s 方向が x̂ × k の向き（y）になるので、x 偏光は p
    let mut s0 = a0();
    s0.surfaces[j].coat = String::new();
    let c0 = Coatings::for_system(&s0).unwrap();
    let t0 = s0
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c0)
        .unwrap();
    let (n1, n2) = (s.index_before(j, w), s.surfaces[j].n[w]);
    let fres = 2.0 * (n1 * n2).sqrt() / (n1 + n2);
    let r = t.e[0][0][0] / t0.e[0][0][0];
    assert!((r - C64::new(0.8 / fres, 0.0)).norm() < 1e-13, "{r}");
    // 表にない波長では止める
    let other =
        (0..s.wavelengths.len()).find(|&v| (s.wavelengths[v] - s.wavelengths[w]).abs() > 1e-6);
    if let Some(o) = other {
        let r = s.try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], Some(o), &[X], &c);
        assert!(matches!(r, Err(Error::NotImplemented(_))));
    }
}

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

#[test]
fn ideal_lens_keeps_the_field_transverse() {
    for name in ["D0", "D2"] {
        let s = load(name);
        let c = Coatings::for_system(&s).unwrap();
        let px = [0.0, 0.8, 0.0, -0.6];
        let py = [0.0, 0.0, 0.9, -0.6];
        let ys = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
        let nf = s.fields.len();
        let t = s
            .try_trace_pol(FieldSel::Index(nf - 1), &px, &py, None, &[X, ys], &c)
            .unwrap();
        for i in 0..px.len() {
            let k = t.trace.rays[i].lmn;
            for m in 0..2 {
                let e = t.e[i][m];
                let ek = e[0] * k[0] + e[1] * k[1] + e[2] * k[2];
                assert!(ek.norm() < 1e-12, "{name} {i} {m} {ek}");
            }
        }
    }
}

#[test]
fn vignetted_rays_out_of_a_table_do_not_stop_the_trace() {
    let mut s = a0();
    let w = s.primary;
    let (j, _, _) = interfaces(&s, w)[0];
    s.surfaces[j].coat = "ZEC1".into();
    s.surfaces[j].ap = lensbench_core::surface::Aperture::Circular {
        min: 0.0,
        max: 2.0,
        dx: 0.0,
        dy: 0.0,
    };
    let mut c = Coatings::for_system(&s).unwrap();
    let key = format!("ZEC1|AIR|{}", s.surfaces[j].mat);
    let ang: Vec<f64> = (0..=8).map(|a| a as f64).collect();
    let v = [C64::new(0.9, 0.0), C64::new(0.9, 0.0)];
    c.add_measured(
        &key,
        Measured {
            wavelengths: vec![s.wavelengths[w]],
            angles_deg: ang.clone(),
            data: vec![vec![v; ang.len()]],
        },
    )
    .unwrap();
    let t = s
        .try_trace_pol(
            FieldSel::Norm(0.0, 0.0),
            &[0.0, 0.0],
            &[0.1, 1.0],
            None,
            &[X],
            &c,
        )
        .unwrap();
    assert!(t.trace.ok(0) && t.trace.rays[1].vig == j);
    assert!(t.e[0][0][0].norm().is_finite());
    assert!(t.e[1][0][0].re.is_nan());
    // 後ろの面（像面の手前）でけられる光線も止めない
    s.surfaces[j].ap = lensbench_core::surface::Aperture::None;
    let img = s.image();
    s.surfaces[img - 1].ap = lensbench_core::surface::Aperture::Circular {
        min: 0.0,
        max: 1e-3,
        dx: 0.0,
        dy: 0.0,
    };
    let t = s
        .try_trace_pol(
            FieldSel::Norm(0.0, 0.0),
            &[0.0, 0.0],
            &[0.1, 1.0],
            None,
            &[X],
            &c,
        )
        .unwrap();
    assert!(t.trace.rays[1].vig == img - 1 && t.trace.rays[1].err == 0);
    assert!(t.e[1][0][0].re.is_nan());
    s.surfaces[img - 1].ap = lensbench_core::surface::Aperture::None;
    // けられていない光線が範囲外なら止める
    s.surfaces[j].ap = lensbench_core::surface::Aperture::None;
    let r = s.try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[1.0], None, &[X], &c);
    assert!(matches!(r, Err(Error::NotImplemented(_))), "{r:?}");
}
