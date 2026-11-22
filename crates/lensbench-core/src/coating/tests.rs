use super::*;

#[test]
fn eval_cos_matches_eval_for_bare_interfaces() {
    // 実数の屈折率の境界（透過・鏡、n1 > ns の全反射の手前まで）で、cos からの直接計算が角度経由と一致する
    for &(n1, ns, mirror) in &[
        (1.0, 1.5, false),
        (1.5, 1.0, false),
        (1.0, 1.5, true),
        (1.7, 1.2, true),
    ] {
        let f = Iface {
            kind: Kind::Fresnel {
                ns: C64::new(ns, 0.0),
            },
            mirror,
            n1,
            wl: 0.55,
        };
        for i in 0..=200 {
            let th = i as f64 / 200.0 * (std::f64::consts::FRAC_PI_2 - 1e-4);
            if n1 * th.sin() >= ns * (1.0 - 1e-9) {
                continue;
            }
            let (a, b) = (f.eval(th).unwrap(), f.eval_cos(th.cos()).unwrap());
            for k in 0..2 {
                assert!(
                    (a[k] - b[k]).norm() < 1e-13,
                    "{n1} {ns} {mirror} {th}: {:?} {:?}",
                    a,
                    b
                );
            }
        }
    }
}

#[test]
fn eval_cos_matches_eval_for_coated_interfaces() {
    // 膜（層・実測表）の境界でも、cos からの計算（acos を省く）が角度経由と丸めの差で一致する
    let layers = vec![(C64::new(1.38, 0.0), 0.1), (C64::new(2.1, -0.01), 0.07)];
    let mut cases = Vec::new();
    for mirror in [false, true] {
        cases.push(Iface {
            kind: Kind::Stack {
                layers: layers.clone(),
                ns: C64::new(1.52, 0.0),
                n_out: 1.0,
                layer_phase: true,
                front: true,
            },
            mirror,
            n1: 1.0,
            wl: 0.55,
        });
    }
    let xs: Vec<f64> = (0..=12)
        .map(|i| xcos(1.0 / 1.5, (i as f64 * 7.0).to_radians()))
        .collect();
    let sp = [0.0, 1.0, 2.0, 3.0].map(|c| {
        let ys: Vec<f64> = xs.iter().map(|x| (c + 1.0) * x * x - c * x).collect();
        Spline::not_a_knot(&xs, &ys).unwrap()
    });
    cases.push(Iface {
        kind: Kind::Measured {
            key: "m".into(),
            r: 1.0 / 1.5,
            max_deg: 84.0,
            cmax: 84f64.to_radians().cos(),
            sp: Box::new(sp),
        },
        mirror: false,
        n1: 1.0,
        wl: 0.55,
    });
    for f in &cases {
        for i in 0..=200 {
            let th = i as f64 / 200.0 * 84f64.to_radians();
            let (a, b) = (f.eval(th).unwrap(), f.eval_cos(th.cos()).unwrap());
            for k in 0..2 {
                assert!((a[k] - b[k]).norm() < 1e-12, "{th}: {a:?} {b:?}");
            }
        }
        // 範囲の外は eval と同じく止める
        if matches!(f.kind, Kind::Measured { .. }) {
            assert!(f.eval_cos(85f64.to_radians().cos()).is_err());
            assert!(f.eval(85f64.to_radians()).is_err());
        }
    }
}

#[test]
fn rational_follows_a_sharp_resonance() {
    // 実軸のすぐ近くに極がある関数（導波の共鳴の形）。多項式では追えないが有理関数なら追える
    let f = |x: f64| C64::new(1.0, 0.0) / C64::new(x - 1.2345, 2e-3) + C64::new(0.3 * x, -0.1);
    let xs: Vec<f64> = (0..60).map(|i| 1.0 + 0.01 * i as f64).collect();
    let mut worst = 0.0f64;
    for k in 0..500 {
        let q = 1.02 + 0.55 * k as f64 / 499.0;
        let v = rational_at(&xs, 0, xs.len(), q, &|j| f(xs[j]));
        worst = worst.max((v - f(q)).norm());
    }
    assert!(worst < 1e-8, "{worst:e}");
}

#[test]
fn mirror_branch_is_not_found_without_one() {
    let b2: Vec<f64> = (0..=800).map(|i| (0.0025 * i as f64).powi(2)).collect();
    let tab = |f: &dyn Fn(f64) -> f64| -> Vec<[C64; 2]> {
        b2.iter()
            .map(|&x| {
                let a = f(x);
                [
                    C64::from_polar(a, 0.3 + x),
                    C64::from_polar(a, -0.2 + 0.5 * x),
                ]
            })
            .collect()
    };
    // 全反射しにくい高反射膜: |r| が β とともに少しずつ（指数的に）1 に近づくだけ
    let slow = tab(&|x| 1.0 - 1e-6 * (-8.0 * x).exp());
    assert!(slow.iter().any(|v| (v[0].norm() - 1.0).abs() < 1e-10));
    assert_eq!(mirror_branch(&b2, &slow), None);
    // 吸収のある膜: |r| は 1 に届かない
    assert_eq!(mirror_branch(&b2, &tab(&|x| 0.9 + 0.05 * x / 4.0)), None);
    // 分岐点（β = 1.4）の上で |r| = 1 に張り付く表では見つかる
    let xb = 1.96;
    let br = tab(&|x| {
        if x < xb {
            1.0 - 0.3 * (xb - x).sqrt()
        } else {
            1.0
        }
    });
    let got = mirror_branch(&b2, &br).unwrap().xb;
    assert!((got - xb).abs() < 1e-3, "{got}");
}

#[test]
fn mirror_branch_is_stable_against_tiny_noise() {
    // 分岐点が測った β の点の上にある鏡（外の媒質 1.38、分散なし。β の格子 0.00125 刻みに 1.38 がある）。
    // r に 1e-9 の雑音を足しても、全反射の判定と分岐点の位置が変わらず、係数がほとんど変わらないこと
    let layers = [
        (C64::new(2.10, 0.0), 0.12),
        (C64::new(2.30, 0.0), 0.08),
        (C64::new(1.60, 0.0), 0.15),
    ];
    let exact = |wl: f64, b: f64| {
        let (rs, rp, _, _) = stack(2.05, &layers, C64::new(1.38, 0.0), wl, (b / 2.05).asin());
        [rs, rp]
    };
    let wavelengths: Vec<f64> = (0..41)
        .rev()
        .map(|i| 1.0 / (1.0 / 0.7 + i as f64 * (2.0 - 1.0 / 0.7) / 40.0))
        .collect();
    let betas: Vec<f64> = (0..=1600).map(|i| 0.00125 * i as f64).collect();
    let clean: Vec<Vec<[C64; 2]>> = wavelengths
        .iter()
        .map(|&w| betas.iter().map(|&b| exact(w, b)).collect())
        .collect();
    // 決まった擬似乱数で ±5e-10（1e-9 に丸めたのと同じ大きさ）
    let mut seed = 12345u64;
    let mut noise = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 1e-9
    };
    let noisy: Vec<Vec<[C64; 2]>> = clean
        .iter()
        .map(|w| {
            w.iter()
                .map(|v| v.map(|z| z + C64::new(noise(), noise())))
                .collect()
        })
        .collect();
    let model = |r: Vec<Vec<[C64; 2]>>| CoatModel {
        mirror: Some(MirrorModel {
            n_ref: 2.05,
            d: 0.0,
            wavelengths: wavelengths.clone(),
            betas: betas.clone(),
            r,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (m0, m1) = (model(clean), model(noisy));
    let b2: Vec<f64> = betas.iter().map(|b| b * b).collect();
    let xb = b2[1104];
    for md in [&m0, &m1] {
        let brs = md.mirror.as_ref().unwrap().branches(&b2);
        assert!(
            brs.iter()
                .all(|v| v.is_some_and(|v| v.xb == xb && v.on_grid)),
            "{brs:?}"
        );
    }
    // 測った波長と中点、β の点と中点で、硝材側の鏡の係数を比べる
    let mut wls = wavelengths.clone();
    wls.extend(wavelengths.windows(2).map(|w| 0.5 * (w[0] + w[1])));
    let mut worst = 0.0f64;
    for &wl in &wls {
        let prep = |md: &CoatModel| {
            prepare_model(
                "T",
                md,
                true,
                false,
                false,
                wl,
                0.0,
                PhaseConvention::OpticStudio,
            )
            .unwrap()
        };
        let (k0, k1) = (prep(&m0), prep(&m1));
        for n1 in [1.45, 1.65, 2.05] {
            let f = |k: &Kind| Iface {
                kind: k.clone(),
                mirror: true,
                n1,
                wl,
            };
            let (f0, f1) = (f(&k0), f(&k1));
            for i in 800..1400 {
                let b = 0.000625 * i as f64;
                if b >= n1 {
                    break;
                }
                let th = (b / n1).asin();
                let (a, c) = (f0.eval(th).unwrap(), f1.eval(th).unwrap());
                worst = worst.max((a[0] - c[0]).norm().max((a[1] - c[1]).norm()));
            }
        }
    }
    assert!(worst < 1e-6, "{worst:e}");
}

#[test]
fn mirror_model_does_not_cross_the_branch() {
    // 2.05 の媒質から 3 層を経て、分散のある基板（n ≈ 1.41）へ光が抜ける鏡。β > n_s で |r| = 1 になり、
    // β = n_s で r が sqrt の形で変わる。分岐点は波長で動く
    let ns = |wl: f64| 1.4003 + 0.004 / (wl * wl);
    let layers = [
        (C64::new(2.10, 0.0), 0.12),
        (C64::new(2.30, 0.0), 0.08),
        (C64::new(2.15, 0.0), 0.15),
    ];
    let exact = |wl: f64, b: f64| {
        let (rs, rp, _, _) = stack(2.05, &layers, C64::new(ns(wl), 0.0), wl, (b / 2.05).asin());
        [rs, rp]
    };
    let wavelengths: Vec<f64> = (0..41)
        .rev()
        .map(|i| 1.0 / (1.0 / 0.7 + i as f64 * (2.0 - 1.0 / 0.7) / 40.0))
        .collect();
    let betas: Vec<f64> = (0..=1600).map(|i| 0.00125 * i as f64).collect();
    let r = wavelengths
        .iter()
        .map(|&w| betas.iter().map(|&b| exact(w, b)).collect())
        .collect();
    let md = CoatModel {
        mirror: Some(MirrorModel {
            n_ref: 2.05,
            d: 0.0,
            wavelengths,
            betas,
            r,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (mut worst, mut over) = (0.0f64, 0.0f64);
    for wl in [0.5123, 0.5871, 0.6402, 0.6957] {
        let f = Iface {
            kind: prepare_model(
                "T",
                &md,
                true,
                false,
                false,
                wl,
                0.0,
                PhaseConvention::OpticStudio,
            )
            .unwrap(),
            mirror: true,
            n1: 2.05,
            wl,
        };
        let nb = ns(wl);
        let mut skipped = 0;
        for k in 0..2000 {
            let b = nb - 0.02 + 0.04 * k as f64 / 1999.0;
            // 分岐点のそば（β で 2〜3 刻み）は notimpl
            let Ok(v) = f.eval((b / 2.05).asin()) else {
                assert!((b - nb).abs() < 0.004, "{wl} {b}");
                skipped += 1;
                continue;
            };
            let e = exact(wl, b);
            let d = (v[0] - e[0]).norm().max((v[1] - e[1]).norm());
            worst = worst.max(d);
            over = over.max(v[0].norm().max(v[1].norm()) - 1.0);
        }
        assert!(skipped < 300, "{skipped}");
    }
    assert!(worst < 1e-6, "{worst:e}");
    assert!(over < 1e-7, "{over:e}");
}

#[test]
fn mirror_model_stops_where_the_branch_leaves_the_range() {
    // mirror_model_does_not_cross_the_branch と同じ鏡で、基板の屈折率が波長で大きく変わり、分岐点が測った β の上端（2.0）を
    // 横切る（短い波長では範囲の上、長い波長では中）。補間に使う波長の一部にだけ分岐点があるところでは、上端の近くを
    // notimpl にし、ほかの点は正しい値を返すこと。分岐点がどの波長でも上端の 4 点以内にある（mirror_branch が見ない）ときも同じ
    // 分岐点が上端のすぐ上（1 刻み未満）にある波長も同じ
    for (a, c) in [(1.95, 0.018), (1.9960, 0.0012), (1.9990, 0.0003)] {
        branch_leaves_the_range(&|wl: f64| a + c / (wl * wl));
    }
}

fn branch_leaves_the_range(ns: &dyn Fn(f64) -> f64) {
    let layers = [
        (C64::new(2.10, 0.0), 0.12),
        (C64::new(2.30, 0.0), 0.08),
        (C64::new(2.15, 0.0), 0.15),
    ];
    let exact = |wl: f64, b: f64| {
        let (rs, rp, _, _) = stack(2.05, &layers, C64::new(ns(wl), 0.0), wl, (b / 2.05).asin());
        [rs, rp]
    };
    let wavelengths: Vec<f64> = (0..41)
        .rev()
        .map(|i| 1.0 / (1.0 / 0.7 + i as f64 * (2.0 - 1.0 / 0.7) / 40.0))
        .collect();
    let betas: Vec<f64> = (0..=1600).map(|i| 0.00125 * i as f64).collect();
    let r = wavelengths
        .iter()
        .map(|&w| betas.iter().map(|&b| exact(w, b)).collect())
        .collect();
    let md = CoatModel {
        mirror: Some(MirrorModel {
            n_ref: 2.05,
            d: 0.0,
            wavelengths,
            betas,
            r,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (mut worst, mut skipped) = (0.0f64, 0);
    for wl in [
        0.5553, 0.5671, 0.5782, 0.5853, 0.5904, 0.5971, 0.6082, 0.61, 0.6204, 0.65,
    ] {
        let f = Iface {
            kind: prepare_model(
                "T",
                &md,
                true,
                false,
                false,
                wl,
                0.0,
                PhaseConvention::OpticStudio,
            )
            .unwrap(),
            mirror: true,
            n1: 2.05,
            wl,
        };
        for k in 0..400 {
            let b = 1.96 + 0.04 * k as f64 / 399.0;
            let Ok(v) = f.eval((b / 2.05).asin()) else {
                skipped += 1;
                continue;
            };
            let e = exact(wl, b);
            let d = (v[0] - e[0]).norm().max((v[1] - e[1]).norm());
            worst = worst.max(d);
        }
    }
    assert!(skipped > 0);
    assert!(worst < 1e-5, "{worst:e}");
}

#[test]
fn wave_weights_do_not_cross_kinks() {
    // 波数 x = 1/λ について、折れ目 0.5 µm の両側で別の 3 次式（値は連続、傾きが跳ぶ）
    let xk = 2.0;
    let f = |wl: f64| {
        let x = 1.0 / wl;
        let t = x - xk;
        1.0 + 0.3 * t - 0.2 * t * t + 0.05 * t * t * t + if t > 0.0 { 0.7 * t } else { 0.0 }
    };
    let wls: Vec<f64> = (0..15).map(|i| 0.4 + 0.025 * i as f64).collect();
    assert!(wls.iter().any(|&w| (w - 0.5).abs() < 1e-12));
    let ev = |wl: f64, k: &[f64]| {
        let (i, w) = wave_weights(&wls, wl, k).unwrap();
        w.iter()
            .enumerate()
            .map(|(a, wa)| wa * f(wls[i + a]))
            .sum::<f64>()
    };
    for wl in [0.41, 0.46, 0.49, 0.499, 0.5, 0.501, 0.51, 0.55, 0.74] {
        assert!((ev(wl, &[0.5]) - f(wl)).abs() < 1e-12, "{wl}");
    }
    // 折れ目を知らないと、またぐ区間でずれる
    assert!((ev(0.49, &[]) - f(0.49)).abs() > 1e-5);
}

#[test]
fn fresnel_normal_incidence() {
    let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.5, 0.0), 0.55, 0.0);
    assert!((rs.re + 0.2).abs() < 1e-15 && (rp.re + 0.2).abs() < 1e-15);
    assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-14);
    assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-14);
}

#[test]
fn energy_is_conserved_off_axis() {
    for deg in [10.0f64, 30.0, 55.0] {
        let th = deg.to_radians();
        let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
        assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-13);
        assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-13);
    }
    // ブルースター角で p の反射が消える
    let th = 1.7f64.atan();
    let (_, rp, _, _) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
    assert!(rp.norm() < 1e-14);
}

#[test]
fn quarter_wave_ar() {
    // n = √1.5 の 1/4 波長膜は垂直入射で反射ゼロ
    let n = 1.5f64.sqrt();
    let d = 0.55 / 4.0 / n;
    let (rs, _, ts, _) = stack(1.0, &[(C64::new(n, 0.0), d)], C64::new(1.5, 0.0), 0.55, 0.0);
    assert!(rs.norm() < 1e-14);
    assert!((ts.norm_sqr() - 1.0).abs() < 1e-14);
}

#[test]
fn spline_matches_cubic_exactly() {
    // not-a-knot は 3 次多項式を正確に再現する
    let f = |x: f64| 0.3 * x * x * x - x * x + 2.0 * x - 0.5;
    let x: Vec<f64> = (0..9)
        .map(|i| -1.0 + 0.13 * i as f64 + 0.01 * (i * i) as f64)
        .collect();
    let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
    let s = Spline::not_a_knot(&x, &y).unwrap();
    for v in [-1.2, -0.95, -0.3, 0.1, 0.7, 1.3] {
        assert!((s.eval(v) - f(v)).abs() < 1e-12, "{v}");
    }
}

#[test]
fn measured_file_round_trip() {
    let m = Measured {
        wavelengths: vec![0.84, 0.85],
        angles_deg: vec![0.0, 10.0, 20.0],
        data: vec![
            vec![[C64::new(0.9, -0.1), C64::new(0.8, 0.05)]; 3],
            vec![
                [C64::new(0.1, 0.2), C64::new(0.3, 0.4)],
                [C64::new(1.0 / 3.0, 0.0), C64::new(0.0, -1e-17)],
                [C64::new(f64::NAN, f64::NAN), C64::new(f64::NAN, f64::NAN)],
            ],
        ],
    };
    let txt = format_measured(&[("X|AIR|N-BK7", &m), ("Y|AIR|MIRROR", &m)]);
    let v = parse_measured(&txt).unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[1].0, "Y|AIR|MIRROR");
    let r = &v[0].1;
    assert_eq!(r.wavelengths, m.wavelengths);
    assert_eq!(r.angles_deg, m.angles_deg);
    assert_eq!(r.data[1][1], m.data[1][1]);
    assert!(r.data[1][2][0].re.is_nan());
    // 誤り
    for bad in [
        "",
        "LBCT 2\n",
        "LBCT 1\nWAVE 0.5\n",
        "LBCT 1\nKEY A|AIR|B\n0 1 0 1 0\n",
        "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1\n",
        "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n1 1 0 1 0\n0 1 0 1 0\n",
        "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\nWAVE 0.6\n1 1 0 1 0\n",
        "LBCT 1\nKEY A|AIR|B\n",
        "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\n",
    ] {
        assert!(parse_measured(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn kinds() {
    let mut c = Coatings {
        file: CoatingFile::parse(
            "COAT AR\nN15 0.25\nTABLE T\nANGL 0\nWAVE 0.5 0 0 1 1\nMATE N15\n0.5 1.5 0\n",
        )
        .unwrap(),
        measured: HashMap::new(),
        models: HashMap::new(),
        phase: PhaseConvention::default(),
        table_normal: TableNormal::default(),
        auto: Vec::new(),
        auto_measured: Vec::new(),
        registered: Vec::new(),
        auto_stacks: Vec::new(),
        notes: Vec::new(),
    };
    assert_eq!(c.kind("-|AIR|N-BK7"), CoatKind::Bare);
    assert_eq!(c.kind("I.98|AIR|N-BK7"), CoatKind::Ideal);
    assert_eq!(c.kind("AR|AIR|MIRROR"), CoatKind::Stack);
    assert_eq!(c.kind("AR|AIR|N-BK7"), CoatKind::Stack);
    assert_eq!(c.kind("T|AIR|N-BK7"), CoatKind::Table);
    assert_eq!(c.kind("ZEC|AIR|N-BK7"), CoatKind::Missing);
    let m = Measured {
        wavelengths: vec![0.5],
        angles_deg: vec![0.0, 1.0],
        data: vec![vec![[C64::new(1.0, 0.0); 2]; 2]],
    };
    c.add_measured("AR|AIR|N-BK7", m.clone()).unwrap();
    c.add_measured("ZEC|AIR|N-BK7", m).unwrap();
    assert_eq!(c.kind("AR|AIR|N-BK7"), CoatKind::Stack);
    assert_eq!(c.kind("ZEC|AIR|N-BK7"), CoatKind::Measured);
    assert!(CoatKind::Missing.needs_table() && !CoatKind::Measured.needs_table());
}

#[test]
fn parse_file() {
    let txt = "! c\nMATE N15\n0.55 1.5 0\n\nCOAT AR\nN15 0.25\nN15 0.1 1\n\nTABLE T1\nANGL 0\n\
               WAVE 0.5 0.1 0.1 0.9 0.9 0 0 0 0\nANGL 45\nWAVE 0.5 0.2 0.3 0.8 0.7 0 0 10 20\n\
               ENCRYPTED X\n";
    let f = CoatingFile::parse(txt).unwrap();
    assert_eq!(f.mate["N15"], vec![[0.55, 1.5, 0.0]]);
    assert_eq!(f.coat["AR"].len(), 2);
    assert!(f.coat["AR"][1].absolute && !f.coat["AR"][0].absolute);
    assert_eq!(f.table["T1"].len(), 2);
    assert!(CoatingFile::parse("TABLE T\nWAVE 1 2 3 4 5 6 7 8 9\n").is_err());
    assert!(CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 2 3 4\n").is_err());
    let f = CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 0.1 0.1 0.9 0.9\n").unwrap();
    assert_eq!(
        f.table["T"][0].1[0],
        [1.0, 0.1, 0.1, 0.9, 0.9, 0.0, 0.0, 0.0, 0.0]
    );
    // k > 0（増幅）の材料も読むだけなら止めない（使うときに誤り。polarization の試験）
    assert!(CoatingFile::parse("MATE M\n0.5 0.5 3\n").is_ok());
}

/// 表の位相は ±180° の折り返しをほどいて補間する
#[test]
fn table_phase_unwrap() {
    let mut v = vec![170.0, 179.0, -179.0, -170.0, 175.0];
    unwrap_deg(&mut v);
    assert_eq!(v, vec![170.0, 179.0, 181.0, 190.0, 175.0]);
    let x = interp(0.55, &[0.5, 0.6], &[179.0, 181.0]);
    assert!((x - 180.0).abs() < 1e-12);
}
