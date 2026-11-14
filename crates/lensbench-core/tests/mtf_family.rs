//! スルーフォーカス・フィールド MTF、RMS の各種、FFT PSF、幾何 MTFの性質を
//! 既存の解析と突き合わせて確かめる（OpticStudio との照合は別、tests/refs の基準が揃ってから）。

use crate::common::root;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::Waves;
use lensbench_core::{analysis, psf, sweep};

fn load(name: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &opt,
    )
    .unwrap()
}

fn names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("system.zmx").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

const FREQS: [f64; 4] = [0.0, 20.0, 50.0, 100.0];

#[test]
fn through_focus_at_zero_is_the_mtf() {
    for name in ["A0", "B0", "D4"] {
        let s = load(name);
        let f = FieldSel::Index(s.fields.len() - 1);
        let m = analysis::mtf(&s, f, Waves::All, None, &FREQS).unwrap();
        let t = sweep::through_focus_mtf(&s, f, Waves::All, None, &FREQS, &[-0.01, 0.0, 0.01]);
        assert!(t.warnings.is_empty(), "{name}: {:?}", t.warnings);
        for k in 0..FREQS.len() {
            assert_eq!(t.tan[k][1], m.tan[k], "{name}");
            assert_eq!(t.sag[k][1], m.sag[k], "{name}");
        }
        // ずらしは像面の手前の厚さを変えたのと同じ
        let mut d = s.clone();
        let j = d.image() - 1;
        d.surfaces[j].t += 0.01;
        d.changed().unwrap();
        let md = analysis::mtf(&d, f, Waves::All, None, &FREQS).unwrap();
        for k in 0..FREQS.len() {
            assert_eq!(t.tan[k][2], md.tan[k], "{name}");
        }
    }
}

#[test]
fn field_mtf_at_defined_fields_is_the_mtf() {
    let mut checked = 0;
    for name in names() {
        let s = load(&name);
        let y_only = s.fields.iter().all(|f| f.x == 0.0)
            && s.fields.iter().all(|f| {
                [f.vdx, f.vdy, f.vcx, f.vcy, f.van]
                    .iter()
                    .all(|&v| v == 0.0)
            });
        if !y_only || s.fields.len() < 2 {
            continue;
        }
        let h: Vec<f64> = (0..s.fields.len()).map(|i| s.hxhy(i).1).collect();
        let fm = sweep::field_mtf(&s, sweep::FieldAxis::Y, &h, Waves::All, None, &FREQS);
        for (i, &hi) in h.iter().enumerate() {
            match analysis::mtf(&s, FieldSel::Index(i), Waves::All, None, &FREQS) {
                Ok(m) => {
                    for k in 0..FREQS.len() {
                        assert_eq!(fm.tan[k][i], m.tan[k], "{name} 視野 {}", i + 1);
                        assert_eq!(fm.sag[k][i], m.sag[k], "{name} 視野 {}", i + 1);
                    }
                    assert_eq!(sweep::FieldAxis::Y.value(&s, hi), s.fields[i].y);
                }
                Err(_) => assert!(fm.tan[0][i].is_nan() && !fm.warnings.is_empty()),
            }
        }
        checked += 1;
    }
    assert!(checked >= 2, "y 軸上の視野だけの系が {checked} 個");
}

#[test]
fn rms_agrees_with_spot_and_wavefront() {
    for name in ["A0", "B0", "D4", "F0"] {
        let s = load(name);
        for i in 0..s.fields.len() {
            let f = FieldSel::Index(i);
            let sp = analysis::spot(&s, f, Waves::All, 6).unwrap();
            let c = sweep::rms(&s, f, Waves::All, sweep::RmsOpts::default()).unwrap();
            let h = sweep::rms(
                &s,
                f,
                Waves::All,
                sweep::RmsOpts {
                    rings: 6,
                    reference: sweep::RmsRef::Chief,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!((c.spot - sp.rms_centroid).abs() <= 1e-15 * sp.rms_centroid.max(1.0));
            assert!((h.spot - sp.rms_chief).abs() <= 1e-15 * sp.rms_chief.max(1.0));
            assert!((c.spot_x.hypot(c.spot_y) - c.spot).abs() < 1e-15);
            // 単色の波面（主光線基準）は格子の波面マップの RMS とほぼ同じ
            let w = s.primary;
            let r = sweep::rms(
                &s,
                f,
                Waves::One(w),
                sweep::RmsOpts {
                    rings: 20,
                    reference: sweep::RmsRef::Chief,
                    ..Default::default()
                },
            )
            .unwrap();
            let wf = analysis::wavefront(&s, f, Some(w), 256, false).unwrap();
            assert!(
                (r.wfe - wf.rms).abs() < 5e-3 * wf.rms.max(0.01),
                "{name} 視野 {}: {} {}",
                i + 1,
                r.wfe,
                wf.rms
            );
            let e = (2.0 * std::f64::consts::PI * r.wfe).powi(2);
            assert!((r.strehl - (-e).exp()).abs() < 1e-15);
            // 重心基準は傾きも除くので主光線基準以下
            assert!(c.wfe <= h.wfe + 1e-12, "{name}");
        }
    }
}

#[test]
fn added_wavelength_reproduces_the_system_indices() {
    for name in names() {
        let s = load(&name);
        for (w, &lam) in s.wavelengths.iter().enumerate() {
            let (d, k) = sweep::with_wavelength(&s, lam).unwrap();
            assert_eq!(k, s.wavelengths.len());
            for (j, sf) in d.surfaces.iter().enumerate() {
                assert_eq!(sf.n[k], s.surfaces[j].n[w], "{name} 面 {j} 波長 {}", w + 1);
            }
        }
    }
    // 足した波長の RMS は元の波長の単色の RMS と同じ
    let s = load("B0");
    let f = FieldSel::Index(1);
    let r = sweep::rms_vs_wavelength(&s, f, &s.wavelengths, Default::default());
    for (w, q) in r.rms.iter().enumerate() {
        let a = sweep::rms(&s, f, Waves::One(w), Default::default()).unwrap();
        assert_eq!((q.spot, q.wfe), (a.spot, a.wfe));
    }
}

#[test]
fn rms_vs_focus_has_its_minimum_near_best_focus() {
    let s = load("B0");
    let f = FieldSel::Index(0);
    let dz = sweep::steps_sym(0.3, 13);
    let r = sweep::rms_vs_focus(&s, f, &dz, Waves::One(s.primary), Default::default());
    assert!(r.warnings.is_empty());
    let (imin, _) = r
        .rms
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.wfe.total_cmp(&b.1.wfe))
        .unwrap();
    assert!(imin > 0 && imin < dz.len() - 1, "最小が範囲の端 {imin}");
}

#[test]
fn psf_centroid_is_the_spot_centroid_and_its_otf_is_the_mtf() {
    for name in ["A0", "B0", "D4"] {
        let s = load(name);
        let f = FieldSel::Index(s.fields.len() - 1);
        let p = psf::psf(
            &s,
            f,
            Waves::All,
            psf::PsfOpts {
                pupil: Some(128),
                image: 256,
                delta: None,
            },
        )
        .unwrap();
        let sp = analysis::spot(&s, f, Waves::All, 10).unwrap();
        // 格子（主光線に垂直な面）の点を像面へ
        let img = |p: &psf::Psf, x: f64, y: f64| {
            let [e1, e2] = p.axes;
            let v: Vec<f64> = (0..3).map(|c| x * e1[c] + y * e2[c]).collect();
            let k = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            (v[0] - k[0] * v[2] / k[2], v[1] - k[1] * v[2] / k[2])
        };
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        for (iy, row) in p.i.iter().enumerate() {
            for (ix, v) in row.iter().enumerate() {
                let (x, y) = img(&p, p.x[ix], p.y[iy]);
                sx += v * x;
                sy += v * y;
                sw += v;
            }
        }
        let (cx, cy) = (sx / sw + p.center.0, sy / sw + p.center.1);
        let d = (cx - sp.centroid.0).hypot(cy - sp.centroid.1);
        assert!(
            d < 0.05 * sp.rms_centroid,
            "{name}: {d} / {}",
            sp.rms_centroid
        );
        // 単色の PSF の OTF は回折 MTF（瞳の標本の粗さの分だけ違う）
        let w = s.primary;
        let p = psf::psf(
            &s,
            f,
            Waves::One(w),
            psf::PsfOpts {
                pupil: Some(256),
                image: 512,
                delta: None,
            },
        )
        .unwrap();
        let fr = [10.0, 30.0, 60.0];
        let m = analysis::mtf(&s, f, Waves::One(w), None, &fr).unwrap();
        let tot: f64 = p.i.iter().flatten().sum();
        for (k, &nu) in fr.iter().enumerate() {
            // MTF の周波数は瞳の軸に沿う（J の対角）ので、像面の y に直した格子の y で
            let (mut re, mut im) = (0.0, 0.0);
            for (iy, row) in p.i.iter().enumerate() {
                let ry: f64 = row.iter().sum();
                let y = img(&p, 0.0, p.y[iy]).1;
                let ph = -2.0 * std::f64::consts::PI * nu * y;
                re += ry * ph.cos();
                im += ry * ph.sin();
            }
            let t = re.hypot(im) / tot;
            assert!((t - m.tan[k]).abs() < 3e-3, "{name} {nu}: {t} {}", m.tan[k]);
        }
    }
}

#[test]
fn psf_of_a_near_perfect_system_peaks_at_its_strehl() {
    // 小さな収差: ピーク ≈ |⟨exp(2πi W)⟩|² ≈ exp(−(2πσ)²)（傾きを除いた σ）
    let s = load("B0");
    let f = FieldSel::Index(0);
    let w = s.primary;
    let p = psf::psf(&s, f, Waves::One(w), Default::default()).unwrap();
    let r = sweep::rms(&s, f, Waves::One(w), Default::default()).unwrap();
    assert!(p.peak <= 1.0 + 1e-9);
    assert!(
        (p.peak - r.strehl).abs() < 0.1,
        "ピーク {} ストレール {}",
        p.peak,
        r.strehl
    );
    // 像の格子が瞳の標本の周期を超えるとエラー
    assert!(psf::psf(
        &s,
        f,
        Waves::One(w),
        psf::PsfOpts {
            pupil: Some(32),
            image: 128,
            delta: None
        }
    )
    .is_err());
}

#[test]
fn geometric_mtf_basics() {
    let s = load("D4");
    let f = FieldSel::Index(2);
    let g = psf::geometric_mtf(&s, f, Waves::All, 128, &FREQS, false).unwrap();
    assert!((g.tan[0] - 1.0).abs() < 1e-14 && (g.sag[0] - 1.0).abs() < 1e-14);
    let g2 = psf::geometric_mtf(&s, f, Waves::All, 256, &FREQS, false).unwrap();
    for k in 0..FREQS.len() {
        assert!((g.tan[k] - g2.tan[k]).abs() < 1e-6 && (g.sag[k] - g2.sag[k]).abs() < 1e-6);
    }
    let gd = psf::geometric_mtf(&s, f, Waves::All, 64, &[g.cutoff.0 * 1.5], true).unwrap();
    assert_eq!(gd.sag[0], 0.0);
    // 大きくぼかすと幾何 MTF は回折 MTF に近づく
    let d = sweep::defocus(&s, 0.3).unwrap();
    let fr = [2.0, 5.0, 10.0];
    let m = analysis::mtf(&d, f, Waves::One(s.primary), None, &fr).unwrap();
    let g = psf::geometric_mtf(&d, f, Waves::One(s.primary), 128, &fr, false).unwrap();
    for k in 0..fr.len() {
        assert!(
            (m.tan[k] - g.tan[k]).abs() < 0.05,
            "{} {}",
            m.tan[k],
            g.tan[k]
        );
    }
}

/// 多色と各波長をまとめて求めた値は、別々に求めた値とビットまで同じ
#[test]
fn mtf_waves_is_the_separate_mtfs() {
    use analysis::MtfMethod;
    for name in ["A0", "D4"] {
        let s = load(name);
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            for method in [MtfMethod::Interp, MtfMethod::Fft] {
                let mw = analysis::mtf_waves(&s, f, None, &FREQS, method).unwrap();
                let p = analysis::mtf_with(&s, f, Waves::All, None, &FREQS, method).unwrap();
                assert_eq!(
                    (&mw.poly.tan, &mw.poly.sag),
                    (&p.tan, &p.sag),
                    "{name} {fi}"
                );
                assert_eq!(mw.waves.len(), s.wavelengths.len());
                for (wi, m) in mw.waves.iter().enumerate() {
                    let o =
                        analysis::mtf_with(&s, f, Waves::One(wi), None, &FREQS, method).unwrap();
                    assert_eq!((&m.tan, &m.sag), (&o.tan, &o.sag), "{name} {fi} {wi}");
                }
            }
        }
    }
}

/// 軸上の回転対称な系では子午を球欠で済ませる: 直接追跡（省かない）と合う
#[test]
fn on_axis_tangential_is_the_sagittal() {
    use analysis::MtfMethod;
    let s = load("A0");
    let f = FieldSel::Index(0);
    let i = analysis::mtf_with(&s, f, Waves::All, Some(48), &FREQS, MtfMethod::Interp).unwrap();
    assert_eq!(i.tan, i.sag);
    let d = analysis::mtf_with(&s, f, Waves::All, Some(48), &FREQS, MtfMethod::Direct).unwrap();
    for k in 0..FREQS.len() {
        assert!(
            (i.tan[k] - d.tan[k]).abs() < 1e-6,
            "{k}: {} {}",
            i.tan[k],
            d.tan[k]
        );
    }
}

/// 外挿しない Zernike 面の正規化半径の縁で OPD に段差がある系（Z3）でも、補間は直接追跡と合う。
/// 段差を跨ぐ区間の近くの求積点は直接追跡に回す（直す前は 1e-3〜7e-3 違った）
#[test]
fn interp_mtf_matches_direct_across_an_opd_step() {
    use analysis::MtfMethod;
    let s = load("Z3");
    let freqs = [0.0, 40.0, 80.0, 120.0, 160.0];
    let f = FieldSel::Index(1);
    let i = analysis::mtf_with(&s, f, Waves::One(1), None, &freqs, MtfMethod::Interp).unwrap();
    let d = analysis::mtf_with(&s, f, Waves::One(1), None, &freqs, MtfMethod::Direct).unwrap();
    for (k, fr) in freqs.iter().enumerate() {
        for (a, b) in [(i.tan[k], d.tan[k]), (i.sag[k], d.sag[k])] {
            assert!((a - b).abs() < 1e-9, "{fr}: {a} {b}");
        }
    }
}

/// FFT の自己相関は既定（補間の求積）と 1.5e-4 以内で合う（瞳の標本 512 で 5e-5 程度）
#[test]
fn fft_mtf_is_the_quadrature_mtf() {
    use analysis::MtfMethod;
    for name in ["A0", "A6", "B0", "D4"] {
        let s = load(name);
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            let m0 = analysis::mtf(&s, f, Waves::One(s.primary), Some(4), &[]).unwrap();
            let c = m0.cutoff.0.min(m0.cutoff.1);
            let freqs: Vec<f64> = (0..=20).map(|k| 0.95 * c * k as f64 / 20.0).collect();
            let a = analysis::mtf_with(&s, f, Waves::All, None, &freqs, MtfMethod::Interp).unwrap();
            let b = analysis::mtf_with(&s, f, Waves::All, None, &freqs, MtfMethod::Fft).unwrap();
            for k in 0..freqs.len() {
                for (x, y) in [(a.tan[k], b.tan[k]), (a.sag[k], b.sag[k])] {
                    assert!((x - y).abs() < 1.5e-4, "{name} {fi} {k}: {x} {y}");
                }
            }
        }
    }
}

/// 既定（Auto）は求積の点数を自動で決めて 1024 を超える視野（E0 の視野 3、n が 2000 超）で周波数が 8 個以上の
/// ときだけ FFT にし、求積と 1e-4 以内で合う。点数の少ない視野・n を指定したときは補間の求積とビットまで同じ
#[test]
fn auto_mtf_switches_to_fft_for_many_points() {
    use analysis::MtfMethod;
    let s = load("E0");
    let w = Some(s.primary);
    let m0 = analysis::mtf(&s, FieldSel::Index(2), Waves::all_or(w), Some(4), &[]).unwrap();
    let c = m0.cutoff.0.min(m0.cutoff.1);
    let freqs: Vec<f64> = (0..=20).map(|k| 0.95 * c * k as f64 / 20.0).collect();
    let run = |fi: usize, n: Option<usize>, m: MtfMethod| {
        analysis::mtf_with(&s, FieldSel::Index(fi), Waves::all_or(w), n, &freqs, m).unwrap()
    };
    let (a, i, f) = (
        run(2, None, MtfMethod::Auto),
        run(2, None, MtfMethod::Interp),
        run(2, None, MtfMethod::Fft),
    );
    assert_eq!((&a.tan, &a.sag), (&f.tan, &f.sag));
    for k in 0..freqs.len() {
        for (x, y) in [(a.tan[k], i.tan[k]), (a.sag[k], i.sag[k])] {
            assert!((x - y).abs() < 1e-4, "{k}: {x} {y}");
        }
    }
    for (fi, n) in [(0, None), (2, Some(64))] {
        let (a, i) = (run(fi, n, MtfMethod::Auto), run(fi, n, MtfMethod::Interp));
        assert_eq!((&a.tan, &a.sag), (&i.tan, &i.sag), "{fi} {n:?}");
    }
    // 周波数が少ない（7 個以下）ときは求積の方が速いので切り替えない
    let few = &freqs[..analysis::AUTO_FFT_FREQS - 1];
    let a = analysis::mtf_with(
        &s,
        FieldSel::Index(2),
        Waves::all_or(w),
        None,
        few,
        MtfMethod::Auto,
    )
    .unwrap();
    let i = analysis::mtf_with(
        &s,
        FieldSel::Index(2),
        Waves::all_or(w),
        None,
        few,
        MtfMethod::Interp,
    )
    .unwrap();
    assert_eq!((&a.tan, &a.sag), (&i.tan, &i.sag));
}
