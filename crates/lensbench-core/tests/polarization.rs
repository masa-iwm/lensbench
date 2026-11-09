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
    std::sync::Arc::make_mut(&mut s.glasses)
        .get_mut(&mat)
        .unwrap()
        .it = vec![[wl - 0.1, 0.9, 10.0], [wl + 0.1, 0.9, 10.0]];
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
    // 表がなければ止める。エラーには面の番号・種類・材料・膜の名前が入る
    let r = s.try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c);
    assert!(matches!(r, Err(Error::NotImplemented(_))), "{r:?}");
    let label = format!("面 {j}(STANDARD, {}, 膜 ZEC1)", s.surfaces[j].mat);
    assert_eq!(s.surf_label(j), label);
    assert!(r.unwrap_err().to_string().contains(&label));
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
    // 光線ごとのエラーにも面の名札が付く
    let m = r.unwrap_err().to_string();
    assert!(
        m.contains(&format!("面 {j}(STANDARD, ")) && m.contains("膜 ZEC1"),
        "{m}"
    );
}

#[test]
fn coating_file_stacks_follow_the_opticstudio_conventions() {
    use lensbench_core::coating::{stack, CoatKind, CoatingFile};
    use std::f64::consts::PI;
    let s = a0();
    let txt = "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\nMATE M\n0.4 0.5 3.0\n1.2 0.5 3.0\n\
               COAT AR2\nL 0.11 1\nH 0.06 1\nCOAT MIR\nL 0.1 1\nM 1.0 1\n";
    let c = Coatings {
        file: CoatingFile::parse(txt).unwrap(),
        ..Default::default()
    };
    assert_eq!(c.kind("AR2|AIR|1.5,0.0"), CoatKind::Stack);
    let wl = 0.55;
    let k = 2.0 * PI / wl;
    let (n1, n2) = (1.0, 1.5);
    let layers = [(C64::new(1.38, 0.0), 0.11), (C64::new(2.1, 0.0), 0.06)];
    // 空気から硝材: 層は外側から順、位相 exp(i k Σ d (n1 / cosθ1 − β² / q))
    let f = c.prepare(&s, "AR2|AIR|1.5,0.0", wl, n1, n2).unwrap();
    for deg in [0.0f64, 30.0, 60.0] {
        let th = deg.to_radians();
        let (_, _, ts, tp) = stack(n1, &layers, C64::new(n2, 0.0), wl, th);
        let b = th.sin();
        let mut ph = 0.17 / th.cos();
        for (n, d) in layers {
            let q = (n * n - b * b).sqrt().re;
            ph -= d * b * b / q;
        }
        let e = C64::new(0.0, k * ph).exp();
        let v = f.eval(th).unwrap();
        assert!(
            (v[0] - ts * e).norm() < 1e-13 && (v[1] - tp * e).norm() < 1e-13,
            "{deg}"
        );
    }
    // 硝材から空気: 基板は前の媒質（層を逆に通る）。透過率は同じ β の空気から硝材と同じ（相反性）
    let g = c.prepare(&s, "AR2|1.5,0.0|AIR", wl, n2, n1).unwrap();
    for deg in [0.0f64, 20.0, 35.0] {
        let th = deg.to_radians();
        let ta = f.eval((n2 * th.sin()).asin()).unwrap();
        let tg = g.eval(th).unwrap();
        for m in 0..2 {
            assert!((ta[m].norm() - tg[m].norm()).abs() < 1e-13, "{deg} {m}");
        }
    }
    // 鏡: 基板は最後の層の材料、位相 exp(2 i k n1 cosθ1 Σ d)（最後の層は数えない）
    let h = c.prepare(&s, "MIR|AIR|MIRROR", wl, n1, 0.0).unwrap();
    let th = 0.4f64;
    let (rs, rp, _, _) = stack(
        n1,
        &[(C64::new(1.38, 0.0), 0.1)],
        C64::new(0.5, 3.0),
        wl,
        th,
    );
    let e = C64::new(0.0, 2.0 * k * th.cos() * 0.1).exp();
    let v = h.eval(th).unwrap();
    assert!((v[0] - rs * e).norm() < 1e-13 && (v[1] - rp * e).norm() < 1e-13);
}

/// 層 [(n, d)] の特性行列（外側から順）→ (M00, M10, M01, M11)
fn char_matrix(layers: &[(C64, f64)], wl: f64, b: f64, pol: usize) -> [C64; 4] {
    use std::f64::consts::PI;
    let one = C64::new(1.0, 0.0);
    let i = C64::new(0.0, 1.0);
    let mut m = [[one, C64::new(0.0, 0.0)], [C64::new(0.0, 0.0), one]];
    for &(n, d) in layers {
        let q = (n * n - b * b).sqrt();
        let e = if pol == 0 { q } else { n * n / q };
        let dl = q * (2.0 * PI * d / wl);
        let l = [[dl.cos(), i * dl.sin() / e], [i * e * dl.sin(), dl.cos()]];
        m = [
            [
                m[0][0] * l[0][0] + m[0][1] * l[1][0],
                m[0][0] * l[0][1] + m[0][1] * l[1][1],
            ],
            [
                m[1][0] * l[0][0] + m[1][1] * l[1][0],
                m[1][0] * l[0][1] + m[1][1] * l[1][1],
            ],
        ];
    }
    [m[0][0], m[1][0], m[0][1], m[1][1]]
}

#[test]
fn coating_model_reproduces_the_stack_it_was_made_from() {
    use lensbench_core::coating::{parse_models, stack, CoatKind, CoatingFile};
    use std::fmt::Write;
    let s = a0();
    let txt = "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\nMATE M\n0.4 0.5 3.0\n1.2 0.5 3.0\n\
               COAT AR2\nL 0.11 1\nH 0.06 1\nCOAT MIR\nL 0.1 1\nM 1.0 1\n";
    let layers = [(C64::new(1.38, 0.0), 0.11), (C64::new(2.1, 0.0), 0.06)];
    // 膜のモデルを層から作る: M̃ = 特性行列、ψ = Σ d_j Re(β²/q_j)（OpticStudio の位相の層の項）
    let wls: Vec<f64> = (0..=20).map(|i| 0.50 + 0.005 * i as f64).collect();
    let mut m = String::from("LBCM 2\nCOAT ZAR\nD 0.17\nBMAX 1.3\n");
    for &wl in &wls {
        writeln!(m, "WAVE {wl:?}").unwrap();
        for j in 0..=85 {
            let b = 0.02 * j as f64;
            // OpticStudio と同じく Re をとる（β が層の屈折率を超えると 0。BMAX より先は使われない）
            let psi: f64 = layers
                .iter()
                .map(|(n, d)| d * (b * b / C64::new(n.re * n.re - b * b, 0.0).sqrt()).re)
                .sum();
            write!(m, "{b:?}").unwrap();
            for pol in 0..2 {
                for z in char_matrix(&layers, wl, b, pol) {
                    write!(m, " {:?} {:?}", z.re, z.im).unwrap();
                }
                write!(m, " {psi:?}").unwrap();
            }
            m.push('\n');
        }
    }
    // 鏡: 基準 2.05 から見た r（位相を外したもの）
    let n_ref = 2.05;
    m.push_str("COAT ZMIR\nMIRROR 2.05 0.1\n");
    for &wl in &wls {
        writeln!(m, "WAVE {wl:?}").unwrap();
        for j in 0..=200 {
            let b = 0.01 * j as f64;
            let (rs, rp, _, _) = stack(
                n_ref,
                &[(C64::new(1.38, 0.0), 0.1)],
                C64::new(0.5, 3.0),
                wl,
                (b / n_ref).asin(),
            );
            writeln!(m, "{b:?} {:?} {:?} {:?} {:?}", rs.re, rs.im, rp.re, rp.im).unwrap();
        }
    }
    let mut c = Coatings {
        file: CoatingFile::parse(txt).unwrap(),
        ..Default::default()
    };
    c.models.extend(parse_models(&m).unwrap());
    assert_eq!(c.kind("ZAR|AIR|1.5,0.0"), CoatKind::Model);
    assert_eq!(c.kind("ZAR|AIR|MIRROR"), CoatKind::Missing);
    assert_eq!(c.kind("ZMIR|1.6,0.0|MIRROR"), CoatKind::Model);
    let wl = 0.5525; // 波長の格子の間
    let mut worst = 0.0f64;
    // (膜ファイルの鍵, モデルの鍵, n1, n2, 入射角の上限, 許容)。β が層の屈折率（1.38）に近づくと ψ が急に
    // 変わり、鏡は β の大きい側で r が速く変わるので、β の格子 0.02・0.01 では補間の差が 1e-5 程度になる
    for (k, km, n1, n2, amax, tol) in [
        ("AR2|AIR|1.5,0.0", "ZAR|AIR|1.5,0.0", 1.0, 1.5, 80.0, 2e-6),
        ("AR2|1.5,0.0|AIR", "ZAR|1.5,0.0|AIR", 1.5, 1.0, 40.0, 2e-6),
        (
            "AR2|1.7,0.0|1.5,0.0",
            "ZAR|1.7,0.0|1.5,0.0",
            1.7,
            1.5,
            47.0,
            1e-4,
        ),
        ("MIR|AIR|MIRROR", "ZMIR|AIR|MIRROR", 1.0, 0.0, 80.0, 2e-6),
        (
            "MIR|1.6,0.0|MIRROR",
            "ZMIR|1.6,0.0|MIRROR",
            1.6,
            0.0,
            80.0,
            1e-4,
        ),
    ] {
        let f = c.prepare(&s, k, wl, n1, n2).unwrap();
        let g = c.prepare(&s, km, wl, n1, n2).unwrap();
        for i in 0..=40 {
            let th = (amax * i as f64 / 40.0f64).to_radians();
            let (a, b) = (f.eval(th).unwrap(), g.eval(th).unwrap());
            let d = (a[0] - b[0]).norm().max((a[1] - b[1]).norm());
            assert!(d < tol, "{km} {th}: {a:?} {b:?}");
            worst = worst.max(d);
        }
    }
    eprintln!("max |model − stack| {worst:.1e}");
    // 屈折面での反射（ゴースト）: モデルの M から求めた r が層から求めた r と合う（ψ は打ち消し合う）。
    // 硝材から空気は 41.8° より先で全反射（|r| = 1）
    for (k, km, n1, n2, amax) in [
        ("AR2|AIR|1.5,0.0|R", "ZAR|AIR|1.5,0.0|R", 1.0, 1.5, 80.0),
        ("AR2|1.5,0.0|AIR|R", "ZAR|1.5,0.0|AIR|R", 1.5, 1.0, 70.0),
        (
            "AR2|1.7,0.0|1.5,0.0|R",
            "ZAR|1.7,0.0|1.5,0.0|R",
            1.7,
            1.5,
            70.0,
        ),
    ] {
        assert_eq!(c.kind(km), CoatKind::Model);
        let f = c.prepare(&s, k, wl, n1, n2).unwrap();
        let g = c.prepare(&s, km, wl, n1, n2).unwrap();
        for i in 0..=40 {
            let th = (amax * i as f64 / 40.0f64).to_radians();
            let (a, b) = (f.eval(th).unwrap(), g.eval(th).unwrap());
            let d = (a[0] - b[0]).norm().max((a[1] - b[1]).norm());
            assert!(d < 2e-6, "{km} {th}: {a:?} {b:?}");
        }
    }
    // 透過の実測表だけでは反射を求めない
    let mut c2 = Coatings::default();
    c2.models.clear();
    assert_eq!(c2.kind("ZAR|AIR|1.5,0.0|R"), CoatKind::Missing);
    assert!(matches!(
        c2.prepare(&s, "ZAR|AIR|1.5,0.0|R", wl, 1.0, 1.5),
        Err(Error::NotImplemented(_))
    ));
    // BMAX を超える β と、波長の範囲外は止める
    let g = c.prepare(&s, "ZAR|1.7,0.0|1.5,0.0", wl, 1.7, 1.5).unwrap();
    assert!(matches!(
        g.eval(55f64.to_radians()),
        Err(Error::NotImplemented(_))
    ));
    let r = c.prepare(&s, "ZAR|AIR|1.5,0.0", 0.7, 1.0, 1.5);
    assert!(matches!(r, Err(Error::NotImplemented(_))));
    // 物理的な位相: 層ごとの項を外す。膜ファイルの膜とモデルが一致し、BMAX を超えても計算する
    c.phase = lensbench_core::coating::PhaseConvention::Physical;
    let f = c.prepare(&s, "AR2|1.7,0.0|1.5,0.0", wl, 1.7, 1.5).unwrap();
    let g = c.prepare(&s, "ZAR|1.7,0.0|1.5,0.0", wl, 1.7, 1.5).unwrap();
    for deg in [0.0f64, 30.0, 55.0, 60.0] {
        let th = deg.to_radians();
        let (a, b) = (f.eval(th).unwrap(), g.eval(th).unwrap());
        assert!(
            (a[0] - b[0]).norm().max((a[1] - b[1]).norm()) < 1e-5,
            "{deg}: {a:?} {b:?}"
        );
        // OpticStudio の約束とは s・p 共通の位相だけ違う
        let o = Coatings {
            file: c.file.clone(),
            ..Default::default()
        }
        .prepare(&s, "AR2|1.7,0.0|1.5,0.0", wl, 1.7, 1.5)
        .unwrap()
        .eval(th)
        .unwrap();
        let (r0, r1) = (o[0] / a[0], o[1] / a[1]);
        assert!(
            (r0.norm() - 1.0).abs() < 1e-12 && (r0 - r1).norm() < 1e-12,
            "{deg}"
        );
    }
}

#[test]
fn table_coating_uses_p_at_normal_incidence() {
    use lensbench_core::coating::CoatingFile;
    let s = a0();
    let c = Coatings {
        file: CoatingFile::parse(
            "TABLE T\nANGL 0\nWAVE 0.55 0.1 0.2 0.9 0.8\nANGL 30\nWAVE 0.55 0.15 0.25 0.85 0.75\n",
        )
        .unwrap(),
        ..Default::default()
    };
    let f = c.prepare(&s, "T|AIR|1.5,0.0", 0.55, 1.0, 1.5).unwrap();
    // 垂直入射は s・p の区別がないので p の値（OpticStudio と同じ）
    let v = f.eval(0.0).unwrap();
    assert!((v[0].norm() - 0.8f64.sqrt()).abs() < 1e-15 && v[0] == v[1]);
    // 垂直入射とみなすのは 1 − cosθ < 1e-8 だけ（OpticStudio 26.1 で 0.008102° は p、0.008104° は表の s）
    let v = f.eval(0.008102f64.to_radians()).unwrap();
    assert!((v[0].norm() - 0.8f64.sqrt()).abs() < 1e-8);
    let v = f.eval(0.008104f64.to_radians()).unwrap();
    assert!((v[0].norm() - 0.9f64.sqrt()).abs() < 1e-8);
    // 斜め入射は表の s・p を cosθ について線形に補間（OpticStudio の 15° の値 0.887283・0.787283）
    let v = f.eval(15f64.to_radians()).unwrap();
    assert!((v[0].norm_sqr() - 0.887283).abs() < 1e-6, "{v:?}");
    assert!((v[1].norm_sqr() - 0.787283).abs() < 1e-6, "{v:?}");
    // 段差なし: 0° の行の s を p に置き換えて補間（0° から連続）
    let c = Coatings {
        table_normal: lensbench_core::coating::TableNormal::Continuous,
        ..c
    };
    let f = c.prepare(&s, "T|AIR|1.5,0.0", 0.55, 1.0, 1.5).unwrap();
    for deg in [0.0f64, 0.009, 15.0] {
        let th = deg.to_radians();
        let v = f.eval(th).unwrap();
        // s は 0° で 0.8、30° で 0.85（cosθ で線形）、p は表のまま
        let t = (1.0 - th.cos()) / (1.0 - 30f64.to_radians().cos());
        assert!((v[0].norm_sqr() - (0.8 + 0.05 * t)).abs() < 1e-12, "{deg}");
        assert!((v[1].norm_sqr() - (0.8 - 0.05 * t)).abs() < 1e-12, "{deg}");
    }
}

#[test]
fn coating_model_fitted_from_model_glass_tables_matches_the_stack() {
    use lensbench_core::coat_fit::{fit_model, format_models, FitOptions};
    use lensbench_core::coating::{parse_models, CoatingFile, Measured};
    let s = a0();
    let txt = "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\nMATE M\n0.4 0.5 3.0\n1.2 0.5 3.0\n\
               COAT AR2\nL 0.11 1\nH 0.06 1\nCOAT MIR\nL 0.1 1\nM 1.0 1\n";
    let c = Coatings {
        file: CoatingFile::parse(txt).unwrap(),
        ..Default::default()
    };
    // OpticStudio の代わりに、膜ファイルの膜（OpticStudio の約束で計算）でモデルガラスの組を「測る」
    let wls: Vec<f64> = (0..=40).map(|i| 1.0 / (2.0 - 0.0125 * i as f64)).collect();
    let n = |m: &str| {
        if m == "AIR" {
            1.0
        } else {
            m.split(',').next().unwrap().parse::<f64>().unwrap()
        }
    };
    let meas = |coat: &str, m1: &str, m2: &str| -> Measured {
        let n1 = n(m1);
        let mirror = m2 == "MIRROR";
        let mut b: Vec<f64> = (0..=40)
            .map(|i| (2.0 * i as f64).to_radians().sin())
            .collect();
        if n1 > 1.0 {
            let db = if mirror { 0.005 } else { 0.01 };
            let mut x = 1.0 + db;
            while x < n1 * 80f64.to_radians().sin() {
                b.push(x);
                x += db;
            }
        }
        let angles_deg: Vec<f64> = b.iter().map(|b| (b / n1).asin().to_degrees()).collect();
        let data = wls
            .iter()
            .map(|&wl| {
                let f = c
                    .prepare(
                        &s,
                        &format!("{coat}|{m1}|{m2}"),
                        wl,
                        n1,
                        if mirror { 0.0 } else { n(m2) },
                    )
                    .unwrap();
                angles_deg
                    .iter()
                    .map(|a| f.eval(a.to_radians()).unwrap())
                    .collect()
            })
            .collect();
        Measured {
            wavelengths: wls.clone(),
            angles_deg,
            data,
        }
    };
    let mut tabs = Vec::new();
    for m1 in ["AIR", "1.85,0.0", "2.05,0.0"] {
        for m2 in ["1.45,0.0", "1.65,0.0", "1.85,0.0", "2.05,0.0", "2.3,0.0"] {
            if m1 == "AIR" && m2 == "2.3,0.0" {
                continue;
            }
            tabs.push((format!("AR2|{m1}|{m2}"), meas("AR2", m1, m2)));
        }
    }
    let t: Vec<(&str, &Measured)> = tabs.iter().map(|(k, m)| (k.as_str(), m)).collect();
    let rep = fit_model(&t, &FitOptions::default()).unwrap();
    assert!((rep.model.d - 0.17).abs() < 1e-6, "D {}", rep.model.d);
    assert!(rep.residual_max < 1e-6, "{}", rep.residual_max);
    // ψ は層の屈折率 1.38 で発散するので、BMAX はその手前
    assert!(
        rep.model.bmax > 1.2 && rep.model.bmax < 1.38,
        "BMAX {}",
        rep.model.bmax
    );
    let mt = vec![
        ("MIR|AIR|MIRROR".to_string(), meas("MIR", "AIR", "MIRROR")),
        (
            "MIR|2.05,0.0|MIRROR".to_string(),
            meas("MIR", "2.05,0.0", "MIRROR"),
        ),
    ];
    let t: Vec<(&str, &Measured)> = mt.iter().map(|(k, m)| (k.as_str(), m)).collect();
    // 鏡だけの表（透過の組がない）は当てはめられない
    assert!(fit_model(&t, &FitOptions::default()).is_err());
    let mut all = tabs.clone();
    all.extend(mt);
    let t: Vec<(&str, &Measured)> = all.iter().map(|(k, m)| (k.as_str(), m)).collect();
    let rep = fit_model(&t, &FitOptions::default()).unwrap();
    let mr = rep.model.mirror.as_ref().unwrap();
    assert!((mr.d - 0.1).abs() < 1e-6, "mirror D {}", mr.d);
    // 書き出して読み直し、膜ファイルの膜と比べる（AR2 と MIR は同じ表から作った 1 つのモデル ZX）
    let txt = format_models(&[("ZX", &rep.model)]);
    let mut c2 = Coatings {
        file: c.file.clone(),
        ..Default::default()
    };
    c2.models.extend(parse_models(&txt).unwrap());
    let wl = 0.6123;
    let mut worst = 0.0f64;
    for (k, km, n1, n2, amax) in [
        ("AR2|AIR|1.5,0.0", "ZX|AIR|1.5,0.0", 1.0, 1.5, 80.0),
        ("AR2|1.5,0.0|AIR", "ZX|1.5,0.0|AIR", 1.5, 1.0, 40.0),
        ("AR2|1.7,0.0|1.5,0.0", "ZX|1.7,0.0|1.5,0.0", 1.7, 1.5, 40.0),
        ("MIR|AIR|MIRROR", "ZX|AIR|MIRROR", 1.0, 0.0, 80.0),
        ("MIR|1.6,0.0|MIRROR", "ZX|1.6,0.0|MIRROR", 1.6, 0.0, 70.0),
    ] {
        let f = c2.prepare(&s, k, wl, n1, n2).unwrap();
        let g = c2.prepare(&s, km, wl, n1, n2).unwrap();
        for i in 0..=40 {
            let th = (amax * i as f64 / 40.0f64).to_radians();
            let (a, b) = (f.eval(th).unwrap(), g.eval(th).unwrap());
            let d = (a[0] - b[0]).norm().max((a[1] - b[1]).norm());
            assert!(d < 1e-5, "{km} {th}: {a:?} {b:?}");
            worst = worst.max(d);
        }
    }
    eprintln!(
        "fitted model vs stack: max {worst:.1e}, BMAX {}",
        rep.model.bmax
    );
    // バイナリ（LBCM 3）で書いて読み直すと、値は刻み（透過 1e-9、鏡 1e-12）の半分しか変わらず、係数もほぼ同じ
    use lensbench_core::coat_fit::format_models_bin;
    use lensbench_core::coating::{parse_models_bin, MODEL_BIN_HEADER};
    let bin = format_models_bin(&[("ZX", &rep.model)]).unwrap();
    assert!(bin.starts_with(MODEL_BIN_HEADER));
    assert!(bin.len() * 5 < txt.len(), "{} vs {}", bin.len(), txt.len());
    let back = parse_models_bin(&bin).unwrap();
    assert_eq!(back.len(), 1);
    let (bname, fm) = (&back[0].0, &back[0].1);
    assert_eq!(bname, "ZX");
    assert_eq!(fm.kinks, rep.model.kinks);
    assert_eq!(
        (fm.wavelengths.len(), fm.betas.len()),
        (rep.model.wavelengths.len(), rep.model.betas.len())
    );
    let mut dq = 0f64;
    for (a, b) in rep.model.m.iter().flatten().zip(fm.m.iter().flatten()) {
        for (x, y) in a.iter().flatten().zip(b.iter().flatten()) {
            dq = dq.max((x.re - y.re).abs()).max((x.im - y.im).abs());
        }
    }
    for (a, b) in rep.model.psi.iter().flatten().zip(fm.psi.iter().flatten()) {
        dq = dq.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs());
    }
    assert!(dq <= 5.01e-10, "transmit quantization {dq:e}");
    let (ma, mb) = (
        rep.model.mirror.as_ref().unwrap(),
        fm.mirror.as_ref().unwrap(),
    );
    let mut dm = 0f64;
    for (a, b) in ma.r.iter().flatten().zip(mb.r.iter().flatten()) {
        for (x, y) in a.iter().zip(b) {
            dm = dm.max((x.re - y.re).abs()).max((x.im - y.im).abs());
        }
    }
    assert!(dm <= 5.01e-13, "mirror quantization {dm:e}");
    let mut c3 = Coatings {
        file: c.file.clone(),
        ..Default::default()
    };
    c3.models.extend(back);
    let mut worst = 0.0f64;
    for (km, n1, n2, amax) in [
        ("ZX|AIR|1.5,0.0", 1.0, 1.5, 80.0),
        ("ZX|1.5,0.0|AIR", 1.5, 1.0, 40.0),
        ("ZX|1.7,0.0|1.5,0.0", 1.7, 1.5, 40.0),
        ("ZX|AIR|MIRROR", 1.0, 0.0, 80.0),
        ("ZX|1.6,0.0|MIRROR", 1.6, 0.0, 70.0),
    ] {
        let f = c2.prepare(&s, km, wl, n1, n2).unwrap();
        let g = c3.prepare(&s, km, wl, n1, n2).unwrap();
        for i in 0..=40 {
            let th = (amax * i as f64 / 40.0f64).to_radians();
            let (a, b) = (f.eval(th).unwrap(), g.eval(th).unwrap());
            worst = worst.max((a[0] - b[0]).norm().max((a[1] - b[1]).norm()));
        }
    }
    assert!(worst < 1e-8, "binary vs text {worst:.1e}");
    // 途中で切れたファイルは読めない
    assert!(parse_models_bin(&bin[..bin.len() - 10]).is_err());
    // 膜の数が壊れていても（巨大でも）確保で落ちずにエラーになる
    let mut bad = bin.clone();
    bad[MODEL_BIN_HEADER.len() + 3] = 0x7f;
    assert!(parse_models_bin(&bad).is_err());
    // 1 バイトずつ壊しても panic しない
    for i in MODEL_BIN_HEADER.len()..bin.len().min(MODEL_BIN_HEADER.len() + 200) {
        for x in [0x01u8, 0x80, 0xff] {
            let mut b = bin.clone();
            b[i] ^= x;
            let _ = parse_models_bin(&b);
        }
    }
    eprintln!(
        "binary {} bytes (text {}), max {worst:.1e}",
        bin.len(),
        txt.len()
    );
}
