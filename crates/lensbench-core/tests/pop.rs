//! 簡易 POPを解析解と照合する: 円形開口の軸上（Fresnel・Rayleigh-Sommerfeld の閉じた式）、
//! 正方形開口の断面（Fresnel 積分）、集光の Airy 像と軸上のデフォーカス、光線から作った場（理想レンズ）。

use lensbench_core::coating::C64;
use lensbench_core::pop::{self, Beam, OutGrid, Pilot, PopMethod, RayBeamOpts};
use lensbench_core::surface::Aperture;
use lensbench_core::system::FieldSel;
use std::f64::consts::PI;

const WL: f64 = 0.5e-3;

fn circ(a: f64) -> Aperture {
    Aperture::Circular {
        min: 0.0,
        max: a,
        dx: 0.0,
        dy: 0.0,
    }
}

/// 一様な振幅 1 の平面波・球面波（曲率 curv）に開口を掛けた場
fn uniform(n: usize, half: f64, curv: f64, ap: Aperture) -> Beam {
    let d = 2.0 * half / n as f64;
    let mut b = Beam::from_fn(
        (n, n),
        (d, d),
        (0.0, 0.0),
        WL,
        1.0,
        Pilot {
            curv,
            ..Default::default()
        },
        |_, _| C64::new(1.0, 0.0),
    );
    b.apply_aperture(ap, 16);
    b.w = b.rms_width();
    b
}

fn on_axis(b: &Beam, z: f64, m: PopMethod) -> f64 {
    let o = OutGrid {
        n: Some((1, 1)),
        half: Some((1e-6, 1e-6)),
        center: Some((0.0, 0.0)),
    };
    let p = pop::propagate(b, z, m, o).unwrap();
    assert!(p.kernel_step < PI, "核の標本化 {}", p.kernel_step);
    p.beam.u[0].norm_sqr()
}

#[test]
fn circular_aperture_on_axis_matches_closed_forms() {
    // Fresnel 数 a²/(λz) = 10〜0.25。軸上の値は縁の位置に敏感（N_F = 10 で ∂I/∂a ≈ 250 /mm）なので細かい格子で
    let a = 0.5;
    let b = uniform(512, 0.55, 0.0, circ(a));
    let k = 2.0 * PI / WL;
    for z in [50.0, 100.0, 170.0, 400.0, 2000.0] {
        // Fresnel: 4 sin²(π a²/(2 λ z))
        let f = 4.0 * (PI * a * a / (2.0 * WL * z)).sin().powi(2);
        let got = on_axis(&b, z, PopMethod::Fresnel);
        assert!((got - f).abs() < 4e-3, "Fresnel z {z}: {got} vs {f}");
        // Rayleigh-Sommerfeld: |1 − (z/R) e^{ik(R − z)}|²
        let r = (z * z + a * a).sqrt();
        let rs = (C64::new(1.0, 0.0) - C64::from_polar(z / r, k * (r - z))).norm_sqr();
        let got = on_axis(&b, z, PopMethod::Direct);
        assert!((got - rs).abs() < 4e-3, "RS z {z}: {got} vs {rs}");
    }
}

/// ∫_{t1}^{t2} exp(iπ t²/2) dt（Simpson）
fn fresnel_int(t1: f64, t2: f64) -> C64 {
    let n = 4000;
    let h = (t2 - t1) / n as f64;
    let f = |t: f64| C64::from_polar(1.0, PI * t * t / 2.0);
    let mut s = f(t1) + f(t2);
    for i in 1..n {
        s += f(t1 + i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0
}

#[test]
fn square_aperture_profile_matches_fresnel_integrals() {
    // 半幅 a の正方形、平面波。U = (1/2i) F(x) F(y)、F(x) = ∫ exp(iπt²/2)、t = √(2/λz)(±a − x)
    let a = 0.5;
    // 格子の縁が開口の縁に一致するように（256 標本で半幅 0.64、間隔 0.005）
    let b = uniform(
        256,
        0.64,
        0.0,
        Aperture::Rect {
            hx: a,
            hy: a,
            dx: 0.0,
            dy: 0.0,
        },
    );
    let z = 120.0;
    let s = (2.0 / (WL * z)).sqrt();
    let fx = |x: f64| fresnel_int(s * (-a - x), s * (a - x));
    let o = OutGrid {
        n: Some((41, 41)),
        half: Some((0.8 * 41.0 / 40.0, 0.8 * 41.0 / 40.0)),
        center: Some((0.0, 0.0)),
    };
    let p = pop::propagate(&b, z, PopMethod::Fresnel, o).unwrap();
    assert!(p.kernel_step < PI);
    let mut worst: f64 = 0.0;
    for j in 0..41 {
        for i in 0..41 {
            let (x, y) = (p.beam.x(i), p.beam.y(j));
            let want = (fx(x) * fx(y) / 2.0).norm_sqr();
            let got = p.beam.u[j * 41 + i].norm_sqr();
            worst = worst.max((got - want).abs());
        }
    }
    assert!(worst < 2e-3, "Fresnel 積分との差 {worst}");
    // パワーの保存（窓は影の範囲を含む）
    let inside = 4.0 * a * a;
    assert!((b.power() - inside).abs() < 1e-9);
}

/// J1(x)（積分表示）
fn j1(x: f64) -> f64 {
    let n = 2000;
    let h = PI / n as f64;
    let f = |t: f64| (t - x * t.sin()).cos();
    let mut s = f(0.0) + f(PI);
    for i in 1..n {
        s += f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0 / PI
}

/// 半径 a の一様な開口から距離 r_focus に集光する波の焦点の照度（近軸）
fn airy(a: f64, r_focus: f64, r: f64) -> f64 {
    airy_na(a, r_focus, a / r_focus, r)
}

/// 開口数 na の Airy（中心は近軸の値）
fn airy_na(a: f64, r_focus: f64, na: f64, r: f64) -> f64 {
    let k = 2.0 * PI / WL;
    let i0 = (PI * a * a / (WL * r_focus)).powi(2);
    let v = k * na * r;
    if v.abs() < 1e-12 {
        i0
    } else {
        i0 * (2.0 * j1(v) / v).powi(2)
    }
}

#[test]
fn focused_wave_gives_airy_pattern_and_axial_defocus() {
    let (a, rf) = (5.0, 100.0);
    let b = uniform(200, 5.2, -1.0 / rf, circ(a));
    // 焦点の面: Airy
    let o = OutGrid {
        n: Some((61, 1)),
        half: Some((0.03 * 61.0 / 60.0, 1e-6)),
        center: Some((0.0, 0.0)),
    };
    let p = pop::propagate(&b, rf, PopMethod::Fresnel, o).unwrap();
    assert!(p.kernel_step < PI);
    let i0 = airy(a, rf, 0.0);
    for i in 0..61 {
        let r = p.beam.x(i);
        let want = airy(a, rf, r) / i0;
        let got = p.beam.u[i].norm_sqr() / i0;
        assert!((got - want).abs() < 2e-3, "r {r}: {got} vs {want}");
    }
    // 軸上のデフォーカス: |U|² = (π/(λzβ))² 4 sin²(βa²/2)、β = (k/2)(1/z − 1/R)
    let k = 2.0 * PI / WL;
    for dz in [-0.6, -0.2, 0.1, 0.35, 1.0] {
        let z = rf + dz;
        let be = k / 2.0 * (1.0 / z - 1.0 / rf);
        let want = (PI / (WL * z * be)).powi(2) * 4.0 * (be * a * a / 2.0).sin().powi(2);
        let got = on_axis(&b, z, PopMethod::Fresnel);
        assert!((got - want).abs() / i0 < 2e-3, "dz {dz}: {got} vs {want}");
    }
}

#[test]
fn direct_and_fresnel_agree_far_from_aperture() {
    // Fresnel 近似が効く距離（a⁴/(8λz³) ≪ 1）では二つの積分が一致する。2 次元の小さな窓
    let b = uniform(96, 0.55, 0.0, circ(0.5));
    let o = OutGrid {
        n: Some((15, 15)),
        half: Some((0.6, 0.6)),
        center: Some((0.1, -0.05)),
    };
    let f = pop::propagate(&b, 80.0, PopMethod::Fresnel, o).unwrap();
    let d = pop::propagate(&b, 80.0, PopMethod::Direct, o).unwrap();
    let mut worst: f64 = 0.0;
    for (x, y) in f.beam.u.iter().zip(&d.beam.u) {
        worst = worst.max((x.norm_sqr() - y.norm_sqr()).abs());
    }
    assert!(worst < 2e-3, "{worst}");
}

#[test]
fn tilted_wave_moves_with_its_direction() {
    // 傾き a の平面波は、距離 d で d·a/√(1 − a²) だけずれる（場の形は傾きなしと同じ）
    let mut b = uniform(384, 0.6, 0.0, circ(0.5));
    let z = 100.0;
    let o = OutGrid {
        n: Some((33, 33)),
        half: Some((0.8, 0.8)),
        center: None,
    };
    let p0 = pop::propagate(&b, z, PopMethod::Fresnel, o).unwrap();
    b.pilot.ax = 0.02;
    // 既定の中心は実際の向き（d·tan θ）
    let p1 = pop::propagate(&b, z, PopMethod::Fresnel, o).unwrap();
    let sh = z * 0.02 / (1.0f64 - 0.0004).sqrt();
    assert!((p1.beam.x0 - sh).abs() < 1e-12);
    // Fresnel の核（近軸）では a·d だけずれる: その点を中心にすると傾きなしと同じ
    let o1 = OutGrid {
        center: Some((z * 0.02, 0.0)),
        ..o
    };
    let p1 = pop::propagate(&b, z, PopMethod::Fresnel, o1).unwrap();
    assert!(
        p0.kernel_step < 0.6 && p1.kernel_step < 0.6,
        "{} {}",
        p0.kernel_step,
        p1.kernel_step
    );
    let mut worst: f64 = 0.0;
    for (x, y) in p0.beam.u.iter().zip(&p1.beam.u) {
        worst = worst.max((x.norm_sqr() - y.norm_sqr()).abs());
    }
    assert!(worst < 1e-9, "{worst}");
}

fn paraxial_lens(f: f64, epd: f64) -> lensbench_core::System {
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD {epd}\nFTYP 0 0 1 1 0 0 0\n\
         XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5 1\nPWAV 1\n\
         SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
         SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 {f}\n  PARM 2 1\n  DISZ {f}\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let raw = lensbench_core::zmx::parse_text(&t).unwrap();
    lensbench_core::zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

/// 理想レンズの直後の場を光線から作り、焦点まで伝える。f/10 の正確な球面波なので
/// Rayleigh-Sommerfeld で開口数 sin U の Airy に、遅い f/25 なら Fresnel でも近軸の Airy になる
#[test]
fn beam_from_rays_through_ideal_lens_focuses_to_airy() {
    for (epd, method) in [(10.0, PopMethod::Direct), (4.0, PopMethod::Fresnel)] {
        let f = 100.0;
        let a = epd / 2.0;
        let s = paraxial_lens(f, epd);
        let rb = pop::beam_from_rays(
            &s,
            FieldSel::Index(0),
            None,
            1,
            &RayBeamOpts {
                rays: 33,
                n: 160,
                ..Default::default()
            },
        )
        .unwrap();
        let b = &rb.beam;
        assert!(rb.aperture.is_some());
        assert!((b.pilot.curv + 1.0 / f).abs() < 1e-3 / f, "{:?}", b.pilot);
        assert!((rb.transmitted - 1.0).abs() < 5e-3, "{}", rb.transmitted);
        // 照度は瞳の中で一様（パワー 1 / 面積）
        let i_in = 1.0 / (PI * a * a);
        let c = b.u[(b.ny / 2) * b.nx + b.nx / 2].norm_sqr();
        assert!((c / i_in - 1.0).abs() < 1e-3, "{c} vs {i_in}");
        let d = pop::distance_to(&s, 1, 2, b.dir).unwrap();
        assert_eq!(d, f);
        let rmax = 2.0 * WL * f / a;
        let o = OutGrid {
            n: Some((41, 1)),
            half: Some((rmax * 41.0 / 40.0, 1e-6)),
            center: Some((0.0, 0.0)),
        };
        let p = pop::propagate(b, d, method, o).unwrap();
        assert!(p.kernel_step < 0.5, "{}", p.kernel_step);
        assert!(p.fresnel_error < 0.05, "{}", p.fresnel_error);
        let na = if method == PopMethod::Direct {
            a / (a * a + f * f).sqrt()
        } else {
            a / f
        };
        // パワー 1 の Airy の中心は π a²/(λ f)² × 照度
        let i0 = airy(a, f, 0.0) * i_in;
        for i in 0..41 {
            let r = p.beam.x(i);
            let want = airy_na(a, f, na, r) * i_in / i0;
            let got = p.beam.u[i].norm_sqr() / i0;
            assert!(
                (got - want).abs() < 3e-3,
                "{method:?} r {r}: {got} vs {want}"
            );
        }
    }
}

/// 絞り面の開口が遮蔽・スパイダー（外の縁が無い）でも、光は絞りの円（瞳の縁）の外を通らない（公開 #62）。
/// 遮蔽・スパイダーの分だけパワーが減る
#[test]
fn obscuration_and_spider_on_the_stop_keep_the_stop_circle() {
    let (f, epd) = (100.0, 10.0);
    let a = epd / 2.0;
    for ap in [
        Aperture::Spider {
            width: 0.5,
            arms: 4,
            dx: 0.0,
            dy: 0.0,
        },
        Aperture::Obsc {
            min: 0.0,
            max: 1.5,
            dx: 0.0,
            dy: 0.0,
        },
    ] {
        let mut s = paraxial_lens(f, epd);
        s.surfaces[1].ap = ap;
        let rb = pop::beam_from_rays(
            &s,
            FieldSel::Index(0),
            None,
            1,
            &RayBeamOpts {
                rays: 65,
                n: 160,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rb.aperture, Some(ap));
        let b = &rb.beam;
        let (mut out, mut all) = (0.0, 0.0);
        for j in 0..b.ny {
            for i in 0..b.nx {
                let p = b.u[j * b.nx + i].norm_sqr();
                all += p;
                if b.x(i).hypot(b.y(j)) > a + 1.5 * b.dx.max(b.dy) {
                    out += p;
                }
            }
        }
        assert!(out <= 1e-12 * all, "{ap:?}: {out} / {all}");
        // 遮る割合（スパイダー: 4 本 × 幅 0.5 × 半径 5 − 中心の重なり、遮蔽: 半径 1.5 の円）
        let blocked = match ap {
            Aperture::Spider { .. } => (4.0 * 0.5 * a - 0.25) / (PI * a * a),
            _ => (1.5f64 / a).powi(2),
        };
        assert!(
            (rb.transmitted - (1.0 - blocked)).abs() < 2e-2,
            "{ap:?}: {} vs {}",
            rb.transmitted,
            1.0 - blocked
        );
    }
}

/// Gauss ビーム（入力の面で q = q0、パワー 1）
fn gaussian(n: usize, half: f64, q0: C64) -> Beam {
    let k = 2.0 * PI / WL;
    let iq = C64::new(1.0, 0.0) / q0;
    let d = 2.0 * half / n as f64;
    let mut b = Beam::from_fn(
        (n, n),
        (d, d),
        (0.0, 0.0),
        WL,
        1.0,
        Pilot {
            curv: iq.re,
            ..Default::default()
        },
        // exp(ik r²/(2q)) のうち虚部（振幅）だけ。実部はパイロット
        |x, y| C64::new((-(k * iq.im.abs()) * (x * x + y * y) / 2.0).exp(), 0.0),
    );
    b.normalize(1.0);
    b
}

/// Gauss ビームの照度 2/(π w²) exp(−2r²/w²)、1/q = κ − iλ/(πw²)
fn gauss_irr(q: C64, r: f64) -> f64 {
    let iq = C64::new(1.0, 0.0) / q;
    let w2 = WL / (PI * -iq.im);
    2.0 / (PI * w2) * (-2.0 * r * r / w2).exp()
}

#[test]
fn gaussian_beam_matches_q_parameter_with_every_method() {
    // 半径 0.5 の Gauss ビームを、前方 100 の点に集光させる（腰の半径 ≈ λ·100/(π·0.5) = 0.032）
    let (w, l) = (0.5, 100.0);
    let zr_in = PI * w * w / WL;
    // 入力の面での q: 1/q = −1/L − iλ/(πw²)
    let q0 = C64::new(1.0, 0.0) / C64::new(-1.0 / l, -1.0 / zr_in);
    let b = gaussian(256, 1.5, q0);
    for (z, methods) in [
        (2.0, vec![PopMethod::Angular, PopMethod::Auto]),
        (20.0, vec![PopMethod::Angular, PopMethod::Auto]),
        (60.0, vec![PopMethod::Fresnel, PopMethod::Auto]),
        (99.0, vec![PopMethod::Fresnel, PopMethod::Auto]),
        (101.0, vec![PopMethod::Fresnel, PopMethod::Auto]),
        (180.0, vec![PopMethod::Fresnel, PopMethod::Auto]),
    ] {
        let q = q0 + z;
        let iq = C64::new(1.0, 0.0) / q;
        let wz = (WL / (PI * -iq.im)).sqrt();
        for m in methods {
            let line = OutGrid {
                n: Some((21, 1)),
                half: Some((2.0 * wz * 21.0 / 20.0, 1e-9)),
                center: Some((0.0, 0.0)),
            };
            let p = pop::propagate(&b, z, m, line).unwrap();
            let i0 = gauss_irr(q, 0.0);
            for i in 0..21 {
                let r = p.beam.x(i);
                let got = p.beam.u[i].norm_sqr() / i0;
                let want = gauss_irr(q, r) / i0;
                assert!(
                    (got - want).abs() < 1e-3,
                    "z {z} {m:?}→{:?} r {r}: {got} vs {want}",
                    p.method
                );
            }
            // 位相: 隣の標本との差が球面の曲率 Re(1/q) と合う（パイロットを含めた全体の場で）
            let (j, jj) = (10, 14);
            let ph = |i: usize| p.beam.field(i, 0).arg();
            let (x1, x2) = (p.beam.x(j), p.beam.x(jj));
            let want = 2.0 * PI / WL * iq.re * (x2 * x2 - x1 * x1) / 2.0;
            let dph = (C64::from_polar(1.0, ph(jj) - ph(j)) * C64::from_polar(1.0, -want)).arg();
            assert!(dph.abs() < 2e-2, "z {z} {m:?} 位相 {dph}");
        }
    }
}

#[test]
fn angular_spectrum_composes_and_keeps_power() {
    // 近い距離の開口の影: 近似なしの角スペクトルを 5 + 5 と 10 で比べる（Fresnel 数 50）
    let b = uniform(256, 0.55, 0.0, circ(0.5));
    let line = OutGrid {
        n: Some((41, 1)),
        half: Some((0.6 * 41.0 / 40.0, 1e-9)),
        center: Some((0.0, 0.0)),
    };
    let one = pop::propagate(&b, 10.0, PopMethod::Angular, line).unwrap();
    let h = pop::propagate(&b, 5.0, PopMethod::Angular, OutGrid::default()).unwrap();
    assert!((h.beam.power() - b.power()).abs() < 1e-9 * b.power());
    let two = pop::propagate(&h.beam, 5.0, PopMethod::Angular, line).unwrap();
    for (x, y) in one.beam.u.iter().zip(&two.beam.u) {
        // 振幅は 0.1〜1 程度
        assert!((x - y).norm() < 1e-3, "{x} vs {y}");
    }
    // Auto はこの距離では角スペクトルを選ぶ
    let q = pop::propagate(&b, 10.0, PopMethod::Auto, OutGrid::default()).unwrap();
    assert_eq!(q.method, PopMethod::Angular);
}

fn a0() -> lensbench_core::System {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_core::zmx::LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    lensbench_core::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

/// 実レンズ（f/5 のトリプレット）: 最後の面の後ろの場を光線から作り、像面へ伝えた照度の最大は
/// OPD から求めた Strehl 比 × 無収差の Airy の中心（π (NA/λ)²、パワー 1）。速い集光なので
/// Fresnel 近似は効かず（誤差の目安が大きい）、Auto は Rayleigh-Sommerfeld を選ぶ
#[test]
fn real_lens_peak_matches_strehl_from_opd() {
    let s = a0();
    let w = 1;
    let (px, py) = lensbench_core::analysis::pupil_grid(201);
    let o =
        lensbench_core::analysis::opd(&s, FieldSel::Index(0), &px, &py, Some(w), false).unwrap();
    let (mut z, mut n) = (C64::new(0.0, 0.0), 0.0);
    for (i, v) in o.opd.iter().enumerate() {
        if o.ok(i) {
            z += C64::from_polar(1.0, 2.0 * PI * v);
            n += 1.0;
        }
    }
    let strehl = (z / n).norm_sqr();
    let rb = pop::beam_from_rays(
        &s,
        FieldSel::Index(0),
        Some(w),
        7,
        &RayBeamOpts {
            n: 192,
            ..Default::default()
        },
    )
    .unwrap();
    let d = pop::distance_to(&s, 7, 8, rb.beam.dir).unwrap();
    let out = OutGrid {
        n: Some((21, 21)),
        half: Some((0.004, 0.004)),
        center: None,
    };
    let p = pop::propagate(&rb.beam, d, PopMethod::Auto, out).unwrap();
    assert_eq!(p.method, PopMethod::Direct);
    let f = pop::propagate(&rb.beam, d, PopMethod::Fresnel, out).unwrap();
    assert!(f.fresnel_error > 1.0, "{}", f.fresnel_error);
    // 像空間の NA（実光線、周辺光線の sin U）
    let tr = s
        .try_trace(FieldSel::Index(0), &[0.0], &[1.0], Some(w), None, false)
        .unwrap();
    let na = tr.rays[0].lmn[1].abs() * s.image_index(w);
    let lam = s.wavelengths[w] * 1e-3;
    let i0 = PI * (na / lam).powi(2);
    let got = p.beam.peak() / i0;
    assert!(
        (got / strehl - 1.0).abs() < 0.03,
        "POP {got} vs Strehl {strehl}"
    );
}
