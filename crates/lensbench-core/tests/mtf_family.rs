//! スルーフォーカス・フィールド MTF、RMS の各種、FFT PSF、幾何 MTFの性質を
//! 既存の解析と突き合わせて確かめる（OpticStudio との照合は別、tests/refs の基準が揃ってから）。

use crate::common::root;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::Waves;
use lensbench_core::{analysis, psf, sweep};
use lensbench_formats::zmx::{self, LoadOptions};

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
        let t = sweep::through_focus_mtf(
            &s,
            f,
            Waves::All,
            None,
            &FREQS,
            &[-0.01, 0.0, 0.01],
            Default::default(),
        );
        assert!(t.warnings.is_empty(), "{name}: {:?}", t.warnings);
        for k in 0..FREQS.len() {
            assert_eq!(t.tan[k][1], m.tan[k], "{name}");
            assert_eq!(t.sag[k][1], m.sag[k], "{name}");
        }
        // ずらしは像面の手前の厚さを変えたのと同じ（ずらす前の光線を像面まで進めるので丸めの差だけ）
        let d = sweep::defocus(&s, 0.01).unwrap();
        let md = analysis::mtf(&d, f, Waves::All, None, &FREQS).unwrap();
        for k in 0..FREQS.len() {
            assert!((t.tan[k][2] - md.tan[k]).abs() < 1e-10, "{name}");
            assert!((t.sag[k][2] - md.sag[k]).abs() < 1e-10, "{name}");
        }
        let r = sweep::rms_vs_focus(&s, f, &[0.01], Waves::All, Default::default());
        let a = sweep::rms(&d, f, Waves::All, Default::default()).unwrap();
        assert!((r.rms[0].spot - a.spot).abs() < 1e-12 * a.spot, "{name}");
        assert!((r.rms[0].wfe - a.wfe).abs() < 1e-9, "{name}");
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
        let fm = sweep::field_mtf(
            &s,
            sweep::FieldAxis::Y,
            &h,
            Waves::All,
            None,
            &FREQS,
            Default::default(),
        );
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
            let (d, k) = s.with_wavelength(lam).unwrap();
            assert_eq!(k, s.wavelengths.len());
            for j in 0..d.surfaces.len() {
                assert_eq!(d.n(j)[k], s.n(j)[w], "{name} 面 {j} 波長 {}", w + 1);
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
                pixel: None,
                ..Default::default()
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
        for (iy, row) in p.i.rows().enumerate() {
            for (ix, v) in row.iter().enumerate() {
                let (x, y) = img(&p, p.i.x()[ix], p.i.y()[iy]);
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
                pixel: None,
                ..Default::default()
            },
        )
        .unwrap();
        let fr = [10.0, 30.0, 60.0];
        let (pupil, am) = (analysis::MtfAxes::Pupil, analysis::MtfMethod::Auto);
        let m = analysis::mtf_amp(&s, f, Waves::One(w), None, &fr, am, None, None, pupil).unwrap();
        let tot: f64 = p.i.data().iter().sum();
        // 子午の MTF は瞳を py の向きに δ = λν / J11 ずらす（OpticStudio の FFT MTF と同じ、J の非対角は使わない）。
        // 像面の上の周波数では J (0, δ) / λ = ν (J01 / J11, 1) なので、PSF をその向きにフーリエ変換して比べる
        // （x・y の両方に角度のある視野で J01 ≠ 0。A0 の視野 4 (10, 10)° で J01 / J11 = −0.033）
        let jm = analysis::pupil_jacobian(&s, f, w).unwrap();
        let fx_per = jm[0][1] / jm[1][1];
        for (k, &nu) in fr.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (iy, row) in p.i.rows().enumerate() {
                for (ix, v) in row.iter().enumerate() {
                    let (x, y) = img(&p, p.i.x()[ix], p.i.y()[iy]);
                    let ph = -2.0 * std::f64::consts::PI * nu * (fx_per * x + y);
                    re += v * ph.cos();
                    im += v * ph.sin();
                }
            }
            let t = re.hypot(im) / tot;
            assert!((t - m.tan[k]).abs() < 3e-3, "{name} {nu}: {t} {}", m.tan[k]);
        }
        // 既定（視野の向き）: 子午は瞳を視野の向き t に δ = λν / J'11 ずらす（J' = Rᵀ J R、R の列は
        // 球欠 s = (t.y, −t.x) と t）。像面の上の周波数は ν (J'01 / J'11 s + t)
        let mf = analysis::mtf(&s, f, Waves::One(w), None, &fr).unwrap();
        let tv = analysis::mtf_tangential(&s, f, Default::default())
            .unwrap()
            .unwrap_or([0.0, 1.0]);
        let sv = [tv[1], -tv[0]];
        let jv = |c: [f64; 2]| {
            [
                jm[0][0] * c[0] + jm[0][1] * c[1],
                jm[1][0] * c[0] + jm[1][1] * c[1],
            ]
        };
        let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
        let a = dot(sv, jv(tv)) / dot(tv, jv(tv));
        let dir = [a * sv[0] + tv[0], a * sv[1] + tv[1]];
        for (k, &nu) in fr.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (iy, row) in p.i.rows().enumerate() {
                for (ix, v) in row.iter().enumerate() {
                    let (x, y) = img(&p, p.i.x()[ix], p.i.y()[iy]);
                    let ph = -2.0 * std::f64::consts::PI * nu * (dir[0] * x + dir[1] * y);
                    re += v * ph.cos();
                    im += v * ph.sin();
                }
            }
            let t = re.hypot(im) / tot;
            assert!(
                (t - mf.tan[k]).abs() < 3e-3,
                "{name} {nu}: {t} {}",
                mf.tan[k]
            );
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
            pixel: None,
            ..Default::default()
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
    let gd = psf::geometric_mtf(&s, f, Waves::All, 64, &[g.cutoff.sag * 1.5], true).unwrap();
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
                assert_eq!(mw.poly.cutoff, p.cutoff, "{name} {fi}");
                assert_eq!(mw.waves.len(), s.wavelengths.len());
                for (wi, m) in mw.waves.iter().enumerate() {
                    let o =
                        analysis::mtf_with(&s, f, Waves::One(wi), None, &FREQS, method).unwrap();
                    assert_eq!((&m.tan, &m.sag), (&o.tan, &o.sag), "{name} {fi} {wi}");
                    // 遮断周波数はその波長の値（以前は常に主波長）
                    assert_eq!(m.cutoff, o.cutoff, "{name} {fi} {wi}");
                    let c = analysis::mtf_cutoff(&s, f, wi).unwrap();
                    assert_eq!(o.cutoff, c, "{name} {fi} {wi}");
                    let r = c.tan * s.wavelengths[wi] / (p.cutoff.tan * s.wavelengths[s.primary]);
                    assert!((r - 1.0).abs() < 0.05, "{name} {fi} {wi} {r}");
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

/// 外挿しない Zernike 面の縁で光線が落ちる細い帯（Z3、瞳で幅 1e-5 より細い）は、求積点が当たっても瞳が切れて
/// いるとはしない（線ごとの求積に切り替えない）。周波数 0 はどの点数でも 1 で、点の少ない求積も細かい求積と合う
/// （帯に点が当たると線ごとの求積に切り替えていたときは、視野 3・n = 120 の球欠で 8e-3 違った）
#[test]
fn thin_vignetted_band_does_not_switch_to_the_clipped_quadrature() {
    use analysis::MtfMethod;
    let s = load("Z3");
    let f = FieldSel::Index(2);
    let c = analysis::mtf_cutoff(&s, f, 0).unwrap().min();
    let freqs: Vec<f64> = (0..10).map(|k| c * 0.09 * k as f64).collect();
    let run =
        |n| analysis::mtf_with(&s, f, Waves::One(0), Some(n), &freqs, MtfMethod::Interp).unwrap();
    let fine = run(1500);
    for n in [120, 288, 373, 600] {
        let m = run(n);
        let one = (m.tan[0] - 1.0).abs().max((m.sag[0] - 1.0).abs());
        assert!(one < 1e-12, "{n}: {} {}", m.tan[0], m.sag[0]);
        for k in 0..freqs.len() {
            for (a, b) in [(m.tan[k], fine.tan[k]), (m.sag[k], fine.sag[k])] {
                assert!((a - b).abs() < 3e-3, "{n} {k}: {a} {b}");
            }
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
            let c = analysis::mtf_cutoff(&s, f, s.primary).unwrap().min();
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
    let c = analysis::mtf_cutoff(&s, FieldSel::Index(2), s.primary)
        .unwrap()
        .min();
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

/// 波長ごとに一定の振幅 c_w を返す膜・偏光の振幅
struct PerWave(Vec<f64>);

impl lensbench_core::pupil::PupilAmp for PerWave {
    fn amp(&self, w: usize, _px: f64, _py: f64) -> f64 {
        self.0[w]
    }
}

/// 膜・偏光の振幅を入れた多色の PSF・MTF は、波長の重みに透過した光量の割合（ここでは c_w²）を掛けて足す。
/// 振幅 c_w の多色は、振幅 1 で重みを c_w² 倍した系の多色と同じになる
#[test]
fn polychromatic_weights_include_the_transmitted_power() {
    let s = load("A0");
    let nw = s.wavelengths.len();
    assert!(nw > 1);
    let c: Vec<f64> = (0..nw)
        .map(|i| 0.4 + 0.5 * i as f64 / (nw - 1) as f64)
        .collect();
    let (amp, one) = (PerWave(c.clone()), PerWave(vec![1.0; nw]));
    let mut t = s.clone();
    t.edit_raw(|p| {
        for (w, ci) in p.weights.iter_mut().zip(&c) {
            *w *= ci * ci;
        }
        Ok(())
    })
    .unwrap();
    let f = FieldSel::Index(s.fields.len() - 1);
    for wi in 0..nw {
        let k = lensbench_core::pupil::wave_weight(&s, f, wi, true, Some(&amp)).unwrap();
        assert!((k - t.weights[wi]).abs() < 1e-12 * t.weights[wi].max(1.0));
        let k1 = lensbench_core::pupil::wave_weight(&s, f, wi, false, Some(&amp)).unwrap();
        assert_eq!(k1, 1.0);
    }
    let fr = [0.0, 20.0, 50.0];
    let m = analysis::mtf_amp(
        &s,
        f,
        Waves::All,
        None,
        &fr,
        analysis::MtfMethod::Auto,
        None,
        Some(&amp),
        Default::default(),
    )
    .unwrap();
    let m1 = analysis::mtf_amp(
        &t,
        f,
        Waves::All,
        None,
        &fr,
        analysis::MtfMethod::Auto,
        None,
        Some(&one),
        Default::default(),
    )
    .unwrap();
    let m0 = analysis::mtf_amp(
        &s,
        f,
        Waves::All,
        None,
        &fr,
        analysis::MtfMethod::Auto,
        None,
        Some(&one),
        Default::default(),
    )
    .unwrap();
    let mut differs = false;
    for k in 0..fr.len() {
        assert!((m.tan[k] - m1.tan[k]).abs() < 1e-12 && (m.sag[k] - m1.sag[k]).abs() < 1e-12);
        differs |= (m.tan[k] - m0.tan[k]).abs() > 1e-6;
    }
    assert!(differs, "the weights must change the polychromatic MTF");
    let mw = analysis::mtf_waves_amp(
        &s,
        f,
        None,
        &fr,
        analysis::MtfMethod::Auto,
        Some(&amp),
        Default::default(),
    )
    .unwrap();
    for k in 0..fr.len() {
        assert!((mw.poly.tan[k] - m1.tan[k]).abs() < 1e-12);
    }
    let o = psf::PsfOpts {
        pupil: Some(32),
        image: 32,
        ..Default::default()
    };
    let p = psf::psf_with(&s, f, Waves::All, o, Some(&amp)).unwrap();
    let p1 = psf::psf_with(&t, f, Waves::All, o, Some(&one)).unwrap();
    for (a, b) in p.i.data().iter().zip(p1.i.data()) {
        assert!((a - b).abs() < 1e-12 * p1.peak.max(1.0));
    }
}
