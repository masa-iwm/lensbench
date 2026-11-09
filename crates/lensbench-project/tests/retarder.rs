//! 偏光子・波長板の面: 波長ごとの Jones 行列、偏光の追跡、面の書き方の往復

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::FieldSel;
use lensbench_project::{expand, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn system(surfs: &str) -> lensbench_core::System {
    try_system(surfs).unwrap()
}

fn try_system(surfs: &str) -> Result<lensbench_core::System, String> {
    let p = Project::from_json(&format!(
        r#"{{"glass_catalogs": ["SCHOTT", "LBCRYSTAL"], "wavelengths": [{{"um": 0.5}}, {{"um": 0.633}}, {{"um": 1.0}}], "primary": 2,
        "aperture": {{"type": "epd", "value": 4}}, "fields": {{"points": [[0, 0]]}},
        "blocks": [{{"type": "stop"}}, {{"type": "air", "t": 5}},
          {{"type": "surfaces", "surfaces": [{surfs}]}}, {{"type": "air", "t": 10}}]}}"#
    ))?;
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    expand(&p, &env)
}

/// 軸上の光線の、入力 x・y の偏光に対する出口の電場 (Ex, Ey)
fn fields(s: &lensbench_core::System, w: usize) -> [[C64; 2]; 2] {
    let coat = Coatings::for_system(s).unwrap();
    let one = C64::new(1.0, 0.0);
    let zero = C64::new(0.0, 0.0);
    let t = s
        .try_trace_pol_opts(
            FieldSel::Index(0),
            &[0.0],
            &[0.0],
            Some(w),
            &[[one, zero], [zero, one]],
            &coat,
            false,
        )
        .unwrap();
    let e = &t.e[0];
    [[e[0][0], e[0][1]], [e[1][0], e[1][1]]]
}

#[test]
fn polarizer_extinction_and_axis() {
    let s = system(
        r#"{"polarizer": {"axis_deg": 0, "extinction_ratio": 100, "transmittance": 0.81}, "t": 1}"#,
    );
    let f = fields(&s, 1);
    assert!((f[0][0].norm_sqr() - 0.81).abs() < 1e-12, "{f:?}");
    assert!((f[1][1].norm_sqr() - 0.0081).abs() < 1e-12, "{f:?}");
    assert!(f[0][1].norm() < 1e-12 && f[1][0].norm() < 1e-12);
    // 45° の透過軸: x の入力は強度 1/2 で 45° の向きに
    let s = system(r#"{"polarizer": {"axis_deg": 45}}"#);
    let f = fields(&s, 0);
    assert!((f[0][0].norm_sqr() - 0.25).abs() < 1e-12, "{f:?}");
    assert!((f[0][0] - f[0][1]).norm() < 1e-12, "{f:?}");
}

#[test]
fn waveplate_by_wavelength() {
    // 633 nm の λ/4 板、速い軸 45°: x の直線偏光は円偏光（|Ex| = |Ey|、位相差 90°）
    let s =
        system(r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 45}}"#);
    let f = fields(&s, 1);
    let (ex, ey) = (f[0][0], f[0][1]);
    assert!((ex.norm() - ey.norm()).abs() < 1e-12, "{f:?}");
    let d = (ey / ex).arg().abs();
    assert!((d - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{d}");
    // 1 µm では遅延 0.25·0.633 波長
    let f = fields(&s, 2);
    let d = (f[0][1] / f[0][0]).arg().abs();
    // 速い軸 45° の板に x を入れると Ey/Ex = (a−b)/(a+b) = −i·tan(φ/2)
    let phi = 2.0 * std::f64::consts::PI * 0.25 * 0.633 / 1.0;
    assert!((f[0][1].norm() / f[0][0].norm() - (phi / 2.0).tan()).abs() < 1e-12);
    assert!((d - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{d}");
    // λ/2 板、速い軸 22.5°: x は 45° の直線偏光に
    let s =
        system(r#"{"waveplate": {"retardance": 0.5, "wavelength_um": 0.633, "axis_deg": 22.5}}"#);
    let f = fields(&s, 1);
    assert!((f[0][0].norm() - f[0][1].norm()).abs() < 1e-12, "{f:?}");
    assert!((f[0][1] / f[0][0]).im.abs() < 1e-12, "{f:?}");
    // 多次（order 2）は設計波長で同じ、ほかの波長で遅延が大きく変わる
    let m = system(
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 45, "order": 2}}"#,
    );
    let z =
        system(r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 45}}"#);
    let (fm, fz) = (fields(&m, 1), fields(&z, 1));
    assert!((fm[0][1] / fm[0][0] - fz[0][1] / fz[0][0]).norm() < 1e-9);
    let (fm, fz) = (fields(&m, 0), fields(&z, 0));
    assert!((fm[0][1] / fm[0][0] - fz[0][1] / fz[0][0]).norm() > 0.1);
}

#[test]
fn spec_round_trip_and_errors() {
    let s = system(
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 45, "order": 1}}"#,
    );
    let j = s.surfaces.iter().find(|d| d.retarder.is_some()).unwrap();
    // jones は設計波長の値
    let r = j.retarder.clone().unwrap();
    assert_eq!(j.jones, r.jones8(0.633));
    for bad in [
        r#"{"polarizer": {"extinction_ratio": 0.5}}"#,
        r#"{"polarizer": {"transmittance": 1.5}}"#,
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0}}"#,
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.6}, "jones": [1,0,0,0,0,0,1,0]}"#,
        r#"{"polarizer": {}, "glass": "N-BK7"}"#,
    ] {
        let p = Project::from_json(&format!(
            r#"{{"wavelengths": [{{"um": 0.6}}], "aperture": {{"type": "epd", "value": 4}}, "fields": {{"points": [[0, 0]]}},
            "blocks": [{{"type": "stop"}}, {{"type": "surfaces", "surfaces": [{bad}]}}, {{"type": "air", "t": 10}}]}}"#
        ));
        let env = Env {
            base: root(),
            glass_dirs: vec![root().join("tests/glass")],
            catalog: None,
        };
        assert!(p.and_then(|p| expand(&p, &env)).is_err(), "{bad}");
    }
}

/// くさびのウィンドウ（面の tilt）: 薄いくさびの振れ角は (n − 1)·α に近く、後の座標は元の軸のまま
#[test]
fn wedge_window_deviation() {
    let p = Project::from_json(
        r#"{"glass_catalogs": ["SCHOTT"], "wavelengths": [{"um": 0.5875618}],
        "aperture": {"type": "epd", "value": 2}, "fields": {"points": [[0, 0]]},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 5},
          {"type": "lens", "source": {"surfaces": [
            {"t": 3, "glass": "N-BK7", "semi": 12.7}, {"tilt": [1.0, 0], "semi": 12.7}]}},
          {"type": "air", "t": 100}]}"#,
    )
    .unwrap();
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    let s = expand(&p, &env).unwrap();
    let t = s.trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, false);
    let l = t.rays[0].lmn;
    let n = s.surfaces.iter().find(|d| d.mat == "N-BK7").unwrap().n[0];
    let dev = l[1].atan2(l[2]).to_degrees().abs();
    let thin = (n - 1.0) * 1.0;
    assert!(
        (dev - thin).abs() < 0.01 * thin,
        "振れ {dev}° 薄いくさび {thin}°"
    );
    // 書き戻すと tilt のまま
    let j = s.surfaces.iter().position(|d| d.before.is_some()).unwrap();
    assert_eq!(s.surfaces[j].after, lensbench_core::surface::After::Reverse);
}

/// 波長 wl の、遅延 r 波長・速い軸 axis の理想の板の行列（比べる元）
fn ideal(r: f64, axis: f64, wl: f64) -> [C64; 4] {
    lensbench_core::surface::Retarder {
        axis_deg: axis,
        tx: 1.0,
        ty: 1.0,
        retardance: r,
        order: 0,
        wl0: wl,
        plates: Vec::new(),
    }
    .jones_at(wl)
}

/// 出口の電場 f が、行列 m に複素数 c を 1 つ掛けたものと合うか（|c| − 1 は loss まで）
fn same_up_to_phase(f: [[C64; 2]; 2], m: [C64; 4], tol: f64) -> bool {
    same_scaled(f, m, tol, 1e-7)
}

fn same_scaled(f: [[C64; 2]; 2], m: [C64; 4], tol: f64, loss: f64) -> bool {
    let g = [f[0][0], f[1][0], f[0][1], f[1][1]];
    let (mut num, mut den) = (C64::new(0.0, 0.0), 0.0);
    for i in 0..4 {
        num += m[i].conj() * g[i];
        den += m[i].norm_sqr();
    }
    let c = num / den;
    (c.norm() - 1.0).abs() < loss && (0..4).all(|i| (g[i] - c * m[i]).norm() < tol)
}

/// 厚さのある結晶板の行列は、理想の板と同じ（OpticStudio と同じ位相の約束、遅い軸が e^{−iφ}）
fn crystal(r: f64, axis: f64, wl: f64) -> [C64; 4] {
    ideal(r, axis, wl)
}

/// 結晶板の入口の面（厚さ = 板の厚さ）
fn plate_thicknesses(s: &lensbench_core::System) -> Vec<(String, f64)> {
    s.surfaces
        .iter()
        .filter(|d| matches!(d.bire, Some(lensbench_core::surface::Bire::In { .. })))
        .map(|d| (d.mat.clone(), d.t))
        .collect()
}

/// 複屈折材の波長板: 厚さのある結晶板（入口・出口の面）になり、遅延は Δn(λ)·d/λ（水晶は Ghosh の常光・異常光の式）
#[test]
fn birefringent_waveplate_by_wavelength() {
    let s = system(
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 45, "material": "QUARTZ"}}"#,
    );
    let g = |n: &str| s.glasses[n].clone();
    let dn = |l: f64| g("QUARTZ-E").n_cat(l) - g("QUARTZ").n_cat(l);
    // 厚さは設計波長の遅延 1/4 から。板の後ろの空気は元の面の厚さ（0）
    let pl = plate_thicknesses(&s);
    assert_eq!(pl.len(), 1);
    let d = pl[0].1;
    assert_eq!(pl[0].0, "QUARTZ");
    assert!((d - 0.25 * 0.633e-3 / dn(0.633)).abs() < 1e-15, "{d}");
    assert!((dn(0.633) - 0.009045).abs() < 1e-5);
    assert!(s.surfaces.iter().all(|d| d.retarder.is_none()));
    // 結晶軸は面内で遅い軸（水晶は ne > no なので速い軸 45° + 90°）
    let ax = s
        .surfaces
        .iter()
        .find_map(|d| match &d.bire {
            Some(lensbench_core::surface::Bire::In { axis, mode, .. }) => Some((*axis, *mode)),
            _ => None,
        })
        .unwrap();
    assert_eq!(ax.1, 2);
    assert!((ax.0[0] + 0.5f64.sqrt()).abs() < 1e-15 && (ax.0[1] - 0.5f64.sqrt()).abs() < 1e-15);
    for (w, l) in [0.5, 0.633, 1.0].into_iter().enumerate() {
        let want = crystal(dn(l) * d * 1e3 / l, 45.0, l);
        assert!(same_up_to_phase(fields(&s, w), want, 1e-7), "{l}");
    }
    // 設計波長では円偏光
    let f = fields(&s, 1);
    assert!((f[0][0].norm() - f[0][1].norm()).abs() < 1e-7);

    // 厚さを書いた多次の板: 1 mm の水晶は 633 nm で約 14.3 波長
    let s = system(
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "thickness_mm": 1}, "t": 2}"#,
    );
    assert_eq!(plate_thicknesses(&s), vec![("QUARTZ".to_string(), 1.0)]);
    // 板の後ろの空気は 2 mm（と後ろのブロックの空気 10 mm。板の厚さの分だけ系は長くなる）
    let out = s
        .surfaces
        .iter()
        .find(|d| d.bire == Some(lensbench_core::surface::Bire::Out))
        .unwrap();
    assert_eq!(out.t, 12.0);
    for (w, l) in [0.5, 0.633, 1.0].into_iter().enumerate() {
        let want = crystal(dn(l) * 1e3 / l, 0.0, l);
        assert!(same_up_to_phase(fields(&s, w), want, 1e-7), "{l}");
    }
    // 透過率 0.81 は外側の 2 面の理想膜（1 面 0.9）
    let s = system(
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "thickness_mm": 1, "transmittance": 0.81}}"#,
    );
    let f = fields(&s, 1);
    assert!(
        (f[0][0].norm_sqr() + f[1][0].norm_sqr() - 0.81).abs() < 1e-12,
        "{f:?}"
    );
}

/// アクロマティック波長板（2 材の組）: 水晶と MgF₂ の速い軸を直交させ、2 波長で 1/4 になる厚さ
#[test]
fn achromatic_waveplate_two_materials() {
    let probe = system(
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "thickness_mm": 1}}"#,
    );
    let g = |n: &str| probe.glasses[n].clone();
    let dq = |l: f64| g("QUARTZ-E").n_cat(l) - g("QUARTZ").n_cat(l);
    let dm = |l: f64| g("MGF2-E").n_cat(l) - g("MGF2").n_cat(l);
    // dm·d1 − dq·d2 = λ/4（λ = 0.5, 1.0 µm、d は µm。水晶の方が分散が大きいので引く側）
    let (l1, l2) = (0.5, 1.0);
    let (a, b, c, e) = (dm(l1), -dq(l1), dm(l2), -dq(l2));
    let det = a * e - b * c;
    let d1 = (0.25 * l1 * e - b * 0.25 * l2) / det * 1e-3;
    let d2 = (a * 0.25 * l2 - c * 0.25 * l1) / det * 1e-3;
    assert!(d1 > 0.0 && d2 > 0.0, "{d1} {d2}");
    let s = system(&format!(
        r#"{{"waveplate": {{"wavelength_um": 0.633, "axis_deg": 45, "plates": [
          {{"material": "MGF2", "thickness_mm": {d1}}}, {{"material": "QUARTZ", "thickness_mm": {d2}, "axis_deg": 90}}]}}}}"#
    ));
    // 板 2 枚は 入口(MGF2) → 出口(後ろ QUARTZ、厚さ 0) → 入口(QUARTZ) → 出口(空気)
    assert_eq!(
        plate_thicknesses(&s),
        vec![("MGF2".to_string(), d1), ("QUARTZ".to_string(), d2)]
    );
    for (w, l) in [0.5, 0.633, 1.0].into_iter().enumerate() {
        let net = (dm(l) * d1 - dq(l) * d2) * 1e3 / l;
        if w != 1 {
            assert!((net - 0.25).abs() < 1e-12, "{l} {net}");
        } else {
            assert!((net - 0.25).abs() < 0.05, "{l} {net}");
        }
        // 速い軸が直交する 2 枚は、差の遅延の 1 枚と同じ行列（貼り合わせの面の Fresnel で強度 0.3 % 減る）
        let f = fields(&s, w);
        assert!(
            same_scaled(f, crystal(net, 45.0, l), 1e-12, 5e-3),
            "{l} {f:?}"
        );
    }
}

/// 結晶板の面は .zmx の BIRE__IN・BIRE_OUT に書き出し、読み直しても同じ偏光
#[test]
fn crystal_plate_zmx_roundtrip() {
    let s = system(
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 30, "material": "QUARTZ"}, "t": 3}"#,
    );
    let txt = lensbench_core::zmx_write::write(&s).unwrap();
    assert!(
        txt.contains("TYPE BIRE__IN") && txt.contains("TYPE BIRE_OUT"),
        "{txt}"
    );
    assert!(txt.contains("COAT I.99999999"));
    let p = std::env::temp_dir().join(format!("lb_crystal_{}.zmx", std::process::id()));
    std::fs::write(&p, &txt).unwrap();
    let opt = lensbench_core::zmx::LoadOptions {
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let r = lensbench_core::zmx::load_with(&p, &opt).unwrap();
    std::fs::remove_file(&p).ok();
    for w in 0..3 {
        let (a, b) = (fields(&s, w), fields(&r, w));
        for i in 0..2 {
            for j in 0..2 {
                assert!((a[i][j] - b[i][j]).norm() < 1e-12, "{w} {a:?} {b:?}");
            }
        }
    }
}

/// 結晶板を面で直接書く（OpticStudio の Birefringent In / Out と同じ並び）
#[test]
fn crystal_plate_surfaces_directly() {
    let a = system(
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "thickness_mm": 1, "axis_deg": 60}, "t": 3}"#,
    );
    let b = system(
        r#"{"glass": "QUARTZ", "t": 1, "coat": "I.99999999", "birefringent": {"in": {"axis": [-0.8660254037844386, 0.5, 0]}}},
           {"t": 3, "coat": "I.99999999", "birefringent": "out"}"#,
    );
    for w in 0..3 {
        let (x, y) = (fields(&a, w), fields(&b, w));
        for i in 0..2 {
            for j in 0..2 {
                assert!((x[i][j] - y[i][j]).norm() < 1e-12, "{w} {x:?} {y:?}");
            }
        }
    }
    for bad in [
        r#"{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [1, 0, 0]}}}"#,
        r#"{"t": 1, "birefringent": {"in": {"axis": [1, 0, 0]}}}, {"birefringent": "out"}"#,
        r#"{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [0, 0, 0]}}}, {"birefringent": "out"}"#,
        r#"{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [1, 0, 0], "mode": 1}}}, {"birefringent": "out"}"#,
        r#"{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [1, 0, 0]}}}, {"t": 1}, {"birefringent": "out"}"#,
        r#"{"birefringent": "out"}"#,
    ] {
        assert!(try_system(bad).is_err(), "{bad}");
    }
}

#[test]
fn birefringent_waveplate_errors() {
    for bad in [
        // 異常光のガラスが無い
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "material": "N-BK7"}}"#,
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "material": "NOPE"}}"#,
        // 厚さも遅延も無い
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ"}}"#,
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "thickness_mm": -1}}"#,
        r#"{"waveplate": {"wavelength_um": 0.633, "thickness_mm": 1}}"#,
        r#"{"waveplate": {"wavelength_um": 0.633, "material": "QUARTZ", "plates": [{"material": "MGF2", "thickness_mm": 1}]}}"#,
        r#"{"waveplate": {"wavelength_um": 0.633, "plates": [{"material": "MGF2", "thickness_mm": 0}]}}"#,
    ] {
        assert!(try_system(bad).is_err(), "{bad}");
    }
}

/// 理想の λ/4 と同じ向きの結晶の λ/4 を重ねると λ/2（位相の向きがそろっている）
#[test]
fn ideal_and_crystal_quarter_waves_add() {
    let s = system(
        r#"{"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 30}, "t": 1},
           {"waveplate": {"retardance": 0.25, "wavelength_um": 0.633, "axis_deg": 30, "material": "QUARTZ"}}"#,
    );
    let g = |n: &str| s.glasses[n].clone();
    let dn = |l: f64| g("QUARTZ-E").n_cat(l) - g("QUARTZ").n_cat(l);
    let d = 0.25 * 0.633e-3 / dn(0.633);
    let r = 0.25 + dn(0.633) * d * 1e3 / 0.633;
    assert!((r - 0.5).abs() < 1e-12);
    assert!(same_up_to_phase(
        fields(&s, 1),
        crystal(0.5, 30.0, 0.633),
        1e-7
    ));
}
