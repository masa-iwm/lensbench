//! 膜のモデルの当てはめ（coat_fit）: モデルガラスの表から当てたモデルが膜の重なりと合う（core の tests/polarization.rs から移した）

use lensbench_core::coating::Coatings;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
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

#[test]
fn coating_model_fitted_from_model_glass_tables_matches_the_stack() {
    use lensbench_core::coating::{parse_models, CoatingFile, Measured};
    use lensbench_design::coat_fit::{fit_model, format_models, FitOptions};
    let s = a0();
    let txt = "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\nMATE M\n0.4 0.5 -3.0\n1.2 0.5 -3.0\n\
               COAT AR2\nL 0.11 1\nH 0.06 1\nCOAT MIR\nL 0.1 1\nM 1.0 1\n\
               MATE G\n0.4 1.5 2.0\n1.2 1.5 2.0\nCOAT GAIN\nG 0.1 1\n\
               MATE E\n0.4 1.46 -1e-4\n0.5 1.46 4.8e-7\n1.2 1.46 -1e-4\nCOAT EPS\nE 0.1 1\n";
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
    use lensbench_core::coating::{parse_models_bin, MODEL_BIN_HEADER};
    use lensbench_design::coat_fit::format_models_bin;
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
    // 注記（NOTE）はテキストでも LBCM 3 でも残る
    let mut noted = rep.model.clone();
    noted.note = "近似: 短い測定".into();
    let t2 = format_models(&[("ZX", &noted)]);
    assert!(t2.contains("\nNOTE 近似: 短い測定\n"));
    assert_eq!(parse_models(&t2).unwrap()[0].1.note, "近似: 短い測定");
    let b2 = format_models_bin(&[("ZX", &noted)]).unwrap();
    let back2 = parse_models_bin(&b2).unwrap();
    assert_eq!(back2[0].1.note, "近似: 短い測定");
    assert_eq!(back2[0].1.wavelengths.len(), rep.model.wavelengths.len());
    assert!(parse_models_bin(&b2[..b2.len() - 3]).is_err());
    eprintln!(
        "binary {} bytes (text {}), max {worst:.1e}",
        bin.len(),
        txt.len()
    );
}
