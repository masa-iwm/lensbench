//! 瞳の振幅: アポダイゼーション（.zmx の GFAC 係数 種類）と膜・偏光の振幅を、解析的に分かる場合と
//! OpticStudio 26.1 の実測値で確かめる。
//! - ガウスの瞳（係数 G が大きく縁で切れない）の PSF はガウス I(x)/I(0) = exp(−2π² a² x² / G)（a は瞳の座標から
//!   像の位相への係数）、MTF は自己相関 exp(−G d² / 2)（d は瞳のずれ = 2ν / 遮断周波数）
//! - Cosine Cubed の振幅は物点から入射瞳の点への向きの (cos θ / cos θ₀)^{3/2}、無限遠物体では一様
//! - 一様なら振幅を入れる前とビット単位で同じ、理想レンズだけの系の膜・偏光の振幅は 1

use lensbench_analysis::huygens::{huygens_psf, huygens_psf_with, HuygensOpts};
use lensbench_analysis::pupil_amp::PolAmp;
use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::coating::Coatings;
use lensbench_core::psf::{self, PsfOpts};
use lensbench_core::pupil::{ApodMap, Apodization};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx;
use lensbench_core::Waves;
use std::f64::consts::PI;

/// 理想レンズ（f 100、入射瞳径 10、λ 0.55 µm）。obj が有限なら物体距離 [mm]（像面は結像の位置）
fn ideal(extra: &str, obj: Option<f64>) -> System {
    let (d0, d2) = match obj {
        None => ("INFINITY".to_string(), 100.0),
        Some(u) => (u.to_string(), 1.0 / (1.0 / 100.0 - 1.0 / (u + 10.0))),
    };
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n{extra}\
XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {d0}\n\
SURF 1\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 2\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ {d2}\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap()
}

#[test]
fn zmx_apodization_is_read_and_written() {
    assert_eq!(ideal("", None).apodization, Apodization::Uniform);
    // OpticStudio 26.1 が書く形 GFAC 係数 種類（Gaussian 1.5 → GFAC 1.5 1、Cosine Cubed → GFAC 0 2）
    let s = ideal("GFAC 2.5 1\n", None);
    assert_eq!(s.apodization, Apodization::Gaussian(2.5));
    assert_eq!(
        ideal("GFAC 1.5 1\n", None).apodization,
        Apodization::Gaussian(1.5)
    );
    assert_eq!(
        ideal("GFAC 0 2\n", None).apodization,
        Apodization::CosineCubed
    );
    assert_eq!(ideal("GFAC 0 0\n", None).apodization, Apodization::Uniform);
    // 種類を変えても係数は残る（一様のまま）
    assert_eq!(
        ideal("GFAC 2.25 0\n", None).apodization,
        Apodization::Uniform
    );
    // 書き出して読み直す
    for a in [
        Apodization::Uniform,
        Apodization::Gaussian(0.75),
        Apodization::CosineCubed,
    ] {
        let mut t = s.clone();
        t.edit_raw(|rx| {
            rx.apodization = a;
            Ok(())
        })
        .unwrap();
        let text = lensbench_formats::zmx_write::write(&t).unwrap();
        let r = zmx::build(
            &zmx::parse_text(&text).unwrap(),
            &Default::default(),
            &[],
            None,
        )
        .unwrap();
        assert_eq!(r.apodization, a, "{text}");
    }
    // 知らない種類・負の係数は誤り
    let bad = |e: &str| {
        let t = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\n{e}WAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  DISZ INFINITY\nSURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ 100\nSURF 2\n  TYPE STANDARD\n"
        );
        zmx::build(
            &zmx::parse_text(&t).unwrap(),
            &Default::default(),
            &[],
            None,
        )
        .is_err()
    };
    assert!(bad("GFAC 0 7\n"));
    assert!(bad("GFAC -2 1\n"));
}

#[test]
fn gaussian_pupil_psf_is_gaussian() {
    // G = 16: 縁の振幅は e^−16 で切れの影響は 1e-6 より小さい（G = 6 では 2e-3 ずれる）。像の格子の点 m
    // （既定の間隔 Δ = 1/(4a)）で I/I(0) = exp(−2π² a² (mΔ)² / G) = exp(−π² m² / (8 G))
    let g = 16.0;
    let s = ideal(&format!("GFAC {g} 1\n"), None);
    let f0 = FieldSel::Index(0);
    let o = PsfOpts {
        pupil: Some(128),
        image: 64,
        ..Default::default()
    };
    let p = psf::psf(&s, f0, Waves::All, o).unwrap();
    let ho = HuygensOpts {
        pupil: Some(128),
        image: 64,
        pixel: None,
    };
    let h = huygens_psf(&s, f0, Waves::All, ho).unwrap();
    let c = 32;
    let (i0, h0) = (p.i[c][c], h.i[c][c]);
    for m in 1..12 {
        let want = (-PI * PI * (m * m) as f64 / (8.0 * g)).exp();
        let (ex, ey) = (p.i[c][c + m] / i0, p.i[c + m][c] / i0);
        assert!(
            (ex - want).abs() < 1e-6 && (ey - want).abs() < 1e-6,
            "FFT m {m}: {ex} {ey} {want}"
        );
        // Huygens は理想レンズの F/10 の非近軸の分だけ違う
        let hx = h.i[c][c + m] / h0;
        assert!((hx - want).abs() < 3e-3, "Huygens m {m}: {hx} {want}");
    }
    // 中心の値は収差のない同じ瞳（振幅込み）で 1
    assert!((p.peak - 1.0).abs() < 1e-9 && (h.peak - 1.0).abs() < 1e-3);
    // 振幅つきの全光量: 格子に入った光量の割合は 1 に近い（ガウスの裾は格子の中）
    assert!(p.enclosed > 0.999, "{}", p.enclosed);
}

#[test]
fn gaussian_pupil_mtf_is_the_gaussian_autocorrelation() {
    // G = 10: MTF(ν) = exp(−G d² / 2)、d = 2ν / 遮断周波数（縁の切れは e^−9.8 以下）
    let g = 10.0;
    let s = ideal(&format!("GFAC {g} 1\n"), None);
    let f0 = FieldSel::Index(0);
    let cut = analysis::mtf_cutoff(&s, f0, 0).unwrap().sag;
    let fr: Vec<f64> = (1..6).map(|k| cut * k as f64 / 20.0).collect();
    for method in [MtfMethod::Auto, MtfMethod::Direct, MtfMethod::Fft] {
        let m = analysis::mtf_with(&s, f0, Waves::All, Some(64), &fr, method).unwrap();
        for (k, &nu) in fr.iter().enumerate() {
            let d = 2.0 * nu / cut;
            let want = (-g * d * d / 2.0).exp();
            assert!(
                (m.sag[k] - want).abs() < 1e-3 && (m.tan[k] - want).abs() < 1e-3,
                "{method:?} {nu:.1}: {} {} {want}",
                m.sag[k],
                m.tan[k]
            );
        }
    }
    // 一様の瞳より中域で低い
    let u = analysis::mtf(&ideal("", None), f0, Waves::All, Some(64), &fr).unwrap();
    let m = analysis::mtf(&s, f0, Waves::All, Some(64), &fr).unwrap();
    assert!(m.sag[2] < u.sag[2] - 0.1);
}

#[test]
fn gaussian_psf_transform_is_its_mtf() {
    // 振幅つきでも PSF のフーリエ変換が回折 MTF（G = 1: 縁で振幅 1/e、切れも効く）
    let s = ideal("GFAC 1 1\n", None);
    let f0 = FieldSel::Index(0);
    let p = psf::psf(
        &s,
        f0,
        Waves::All,
        PsfOpts {
            image: 256,
            ..Default::default()
        },
    )
    .unwrap();
    let cut = analysis::mtf_cutoff(&s, f0, 0).unwrap().sag;
    let fr: Vec<f64> = (1..10).map(|k| cut * k as f64 / 10.0).collect();
    let m = analysis::mtf(&s, f0, Waves::All, None, &fr).unwrap();
    let tot: f64 = p.i.data().iter().sum::<f64>() / p.enclosed;
    for (k, &nu) in fr.iter().enumerate() {
        let (mut re, mut im) = (0.0, 0.0);
        for (ix, &x) in p.i.x().iter().enumerate() {
            let col: f64 = p.i.rows().map(|r| r[ix]).sum();
            re += col * (2.0 * PI * nu * x).cos();
            im += col * (2.0 * PI * nu * x).sin();
        }
        let t = re.hypot(im) / tot;
        assert!((t - m.sag[k]).abs() < 1e-3, "{nu:.1}: {t} {}", m.sag[k]);
    }
}

#[test]
fn cosine_cubed_follows_the_object_point_geometry() {
    // 有限物体（物体距離 200、入射瞳は面 1 から 10 = 絞り）: 軸上の物点から入射瞳の縁への cos θ = L / √(L² + r²)
    let s = ideal("GFAC 0 2\n", Some(200.0));
    let a = ApodMap::new(&s, FieldSel::Index(0)).unwrap();
    let p = s.paraxial();
    let (l, r) = (p.enpp + 200.0, 0.5 * p.epd);
    assert!((a.amp(0.0, 0.0) - 1.0).abs() < 1e-12);
    for &(px, py) in &[(1.0, 0.0), (0.0, -1.0), (0.6, 0.6)] {
        let q: f64 = px * px + py * py;
        let c = l / (l * l + q * r * r).sqrt();
        let want = c.powf(1.5);
        assert!(
            (a.amp(px, py) - want).abs() < 1e-9,
            "{px} {py}: {}",
            a.amp(px, py)
        );
    }
    // 軸外の物点では、物点の側の縁の方が明るい（物点に近い）
    let a1 = ApodMap::new(&s, FieldSel::Index(1)).unwrap();
    assert!((a1.amp(0.0, 0.0) - 1.0).abs() < 1e-9);
    let (up, down) = (a1.amp(0.0, 1.0), a1.amp(0.0, -1.0));
    assert!(up != down, "{up} {down}");
    // 無限遠物体では一様
    let si = ideal("GFAC 0 2\n", None);
    assert!(ApodMap::new(&si, FieldSel::Index(1)).unwrap().is_uniform());
    // PSF・MTF も一様の瞳と同じ
    let f = FieldSel::Index(0);
    let o = PsfOpts {
        pupil: Some(32),
        image: 32,
        ..Default::default()
    };
    let p0 = psf::psf(&ideal("", None), f, Waves::All, o).unwrap();
    let p1 = psf::psf(&si, f, Waves::All, o).unwrap();
    assert_eq!(p0.i.data(), p1.i.data());
}

#[test]
fn coating_amplitude_of_an_ideal_lens_is_one() {
    // 理想レンズと空気だけの系は膜・境界がないので、膜・偏光の振幅は 1（PSF は振幅なしとほぼ同じ）
    let s = ideal("", None);
    let f0 = FieldSel::Index(0);
    let co = Coatings::for_system(&s).unwrap();
    let pa = PolAmp::new(&s, f0, &[0], &co, 9, None).unwrap();
    use lensbench_core::pupil::PupilAmp;
    for &(x, y) in &[(0.0, 0.0), (0.7, 0.0), (0.3, -0.5)] {
        // 格子の補間の丸め（1e-9 程度）だけ
        assert!(
            (pa.amp(0, x, y) - 1.0).abs() < 1e-8,
            "{x} {y} {}",
            pa.amp(0, x, y)
        );
    }
    let o = PsfOpts {
        pupil: Some(32),
        image: 32,
        ..Default::default()
    };
    let p0 = psf::psf(&s, f0, Waves::All, o).unwrap();
    let p1 = psf::psf_with(&s, f0, Waves::All, o, Some(&pa)).unwrap();
    let d =
        p0.i.data()
            .iter()
            .zip(p1.i.data())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
    assert!(d < 1e-8, "{d}");
}

#[test]
fn coating_amplitude_of_an_uncoated_singlet_drops_at_the_rim() {
    // 膜のないガラスの単レンズ: 境界の Fresnel の透過で振幅は 1 より小さく、縁（入射角が大きい）でさらに下がる。
    // 偏光なしの振幅は x・y の偏光の透過率の平均の平方根
    let t = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 20\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.02\n  DISZ 48\n\
SURF 3\n  TYPE STANDARD\n";
    let s = zmx::build(&zmx::parse_text(t).unwrap(), &Default::default(), &[], None).unwrap();
    let f0 = FieldSel::Index(0);
    let co = Coatings::for_system(&s).unwrap();
    let pa = PolAmp::new(&s, f0, &[0], &co, 9, None).unwrap();
    use lensbench_core::pupil::PupilAmp;
    let c = pa.amp(0, 0.0, 0.0);
    // 垂直入射の 2 面の透過 (1 − R)²、R = ((n − 1)/(n + 1))² = 0.04
    assert!((c - 0.96).abs() < 2e-3, "{c}");
    let e = pa.amp(0, 1.0, 0.0);
    assert!(e < c && e > 0.8, "{e} {c}");
    // MTF・PSF に入る（振幅が下がる縁の分だけ MTF が変わる）
    let fr = [20.0, 40.0];
    let m0 = analysis::mtf(&s, f0, Waves::All, Some(32), &fr).unwrap();
    let m1 = analysis::mtf_amp(
        &s,
        f0,
        Waves::All,
        Some(32),
        &fr,
        MtfMethod::Auto,
        None,
        Some(&pa),
        Default::default(),
    )
    .unwrap();
    assert!(m0
        .sag
        .iter()
        .zip(&m1.sag)
        .any(|(a, b)| (a - b).abs() > 1e-6));
}

/// 波長ごとに一定の振幅 c_w を返す膜・偏光の振幅
struct PerWave(Vec<f64>);

impl lensbench_core::pupil::PupilAmp for PerWave {
    fn amp(&self, w: usize, _px: f64, _py: f64) -> f64 {
        self.0[w]
    }
}

/// 多色の Huygens PSF も、膜・偏光の振幅の透過した光量の割合（ここでは c_w²）を波長の重みに掛ける: 振幅 c_w の多色は
/// 振幅 1 で重みを c_w² 倍した系と同じ
#[test]
fn huygens_polychromatic_weights_include_the_transmitted_power() {
    // 理想レンズ（f 100、入射瞳径 10）、3 波長
    let s = zmx::build(
        &zmx::parse_text(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 3 0 0 0\n\
XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nWAVM 2 0.75 1\nWAVM 3 0.45 2\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\nSURF 1\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 2\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ 100\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(s.wavelengths.len(), 3);
    let c = vec![0.9, 0.5, 0.7];
    let mut t = s.clone();
    t.edit_raw(|p| {
        for (w, ci) in p.weights.iter_mut().zip(&c) {
            *w *= ci * ci;
        }
        Ok(())
    })
    .unwrap();
    let f = FieldSel::Index(1);
    let ho = HuygensOpts {
        pupil: Some(32),
        image: 32,
        pixel: None,
    };
    let h = huygens_psf_with(&s, f, Waves::All, ho, Some(&PerWave(c.clone()))).unwrap();
    let h1 = huygens_psf_with(&t, f, Waves::All, ho, Some(&PerWave(vec![1.0; 3]))).unwrap();
    let h0 = huygens_psf_with(&s, f, Waves::All, ho, Some(&PerWave(vec![1.0; 3]))).unwrap();
    let (mut d1, mut d0): (f64, f64) = (0.0, 0.0);
    for ((a, b), z) in h.i.data().iter().zip(h1.i.data()).zip(h0.i.data()) {
        d1 = d1.max((a - b).abs());
        d0 = d0.max((a - z).abs());
    }
    assert!(d1 < 1e-12 * h1.peak, "{d1}");
    assert!(d0 > 1e-4 * h0.peak, "the weights must change the PSF: {d0}");
}

/// 射出瞳が求まらない系（偏心した球の面に軸上の光線が届かない）: Huygens PSF も OPD と同じ誤りを返し、平面の基準に
/// 黙って切り替えない
#[test]
fn huygens_without_an_exit_pupil_is_the_opd_error() {
    let s = zmx::build(
        &zmx::parse_text(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 2\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n\
SURF 2\n  TYPE COORDBRK\n  PARM 2 50\n  DISZ 0\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.1\n  DISZ 50\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap();
    let f = FieldSel::Index(0);
    let e1 = analysis::opd(&s, f, &[0.0], &[0.0], None, true)
        .err()
        .map(|e| e.to_string());
    let ho = HuygensOpts {
        pupil: Some(16),
        image: 16,
        pixel: None,
    };
    let e2 = huygens_psf(&s, f, Waves::All, ho)
        .err()
        .map(|e| e.to_string());
    assert!(s.exit_pupil().is_nan());
    assert!(e1.is_some() && e1 == e2, "{e1:?} {e2:?}");
}

/// アポダイゼーションは幾何の解析の光線の重みにも入る: スポット・幾何 MTF は強度 A²、波面の RMS は振幅 A
/// （OpticStudio のガウス求積の RSCE・GMTT は A²、RWCE は A）。
/// 焦点から 1 mm ずらした理想レンズ（F/50）の光線の着地点は瞳の座標に比例するので、ガウスの瞳（係数 G = 8、縁で
/// 強度 e^{−16}）のスポットは標準偏差 σ = c / (2√G) のガウス（c は瞳の縁の高さ）: RMS 半径 c / √(2G)、幾何 MTF は
/// exp(−2π² σ² ν²)。波面 W = W_d ρ² の重みは u = ρ² について e^{−G u}（[0, 1] で切った指数分布）なので、RMS は
/// W_d √(1/G² − 1/(4 sinh²(G/2)))（一様なら W_d / √12）。c と W_d は一様の値から求める。
/// 一様なら重みを入れる前とビット単位で同じ（Uniform と Gaussian(0) が同じ）
#[test]
fn apodization_weights_the_geometric_analyses() {
    let base = |extra: &str| {
        let t = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 2\nFTYP 0 0 1 1 0 0 0\n{extra}\
YFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ 101\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        zmx::build(
            &zmx::parse_text(&t).unwrap(),
            &Default::default(),
            &[],
            None,
        )
        .unwrap()
    };
    let (u, g) = (base(""), base("GFAC 8 1\n"));
    assert_eq!(g.apodization, Apodization::Gaussian(8.0));
    let f = FieldSel::Index(0);
    let o = lensbench_core::sweep::RmsOpts {
        rings: 20,
        ..Default::default()
    };
    let (ru, rg) = (
        lensbench_core::sweep::rms(&u, f, Waves::One(0), o).unwrap(),
        lensbench_core::sweep::rms(&g, f, Waves::One(0), o).unwrap(),
    );
    let c = ru.spot * 2f64.sqrt();
    let gg = 8.0f64;
    let want = c / (2.0 * gg).sqrt();
    println!(
        "RMS spot: uniform {} gauss {} want {want}",
        ru.spot, rg.spot
    );
    assert!((rg.spot / want - 1.0).abs() < 1e-3, "{} {want}", rg.spot);
    let wd = ru.wfe * 12f64.sqrt();
    let want_w = wd * (1.0 / (gg * gg) - 1.0 / (4.0 * (gg / 2.0).sinh().powi(2))).sqrt();
    println!(
        "RMS wavefront: uniform {} gauss {} want {want_w}",
        ru.wfe, rg.wfe
    );
    assert!((rg.wfe / want_w - 1.0).abs() < 2e-3, "{} {want_w}", rg.wfe);
    // スポットの解析も同じ重み
    let sp = analysis::spot(&g, f, Waves::One(0), 20).unwrap();
    assert!((sp.rms_centroid / rg.spot - 1.0).abs() < 1e-12);
    // 幾何 MTF
    let sigma = c / (2.0 * gg.sqrt());
    let freqs = [50.0, 100.0, 200.0];
    let m = psf::geometric_mtf(&g, f, Waves::One(0), 64, &freqs, false).unwrap();
    for (i, &nu) in freqs.iter().enumerate() {
        let want = (-2.0 * PI * PI * sigma * sigma * nu * nu).exp();
        println!("geometric MTF {nu}: {} {} want {want}", m.tan[i], m.sag[i]);
        assert!((m.tan[i] - want).abs() < 2e-3 && (m.sag[i] - want).abs() < 2e-3);
    }
    // 一様（Gaussian(0)）はビット単位で同じ
    let z = base("GFAC 0 1\n");
    let rz = lensbench_core::sweep::rms(&z, f, Waves::One(0), o).unwrap();
    assert_eq!(rz.spot.to_bits(), ru.spot.to_bits());
    assert_eq!(rz.wfe.to_bits(), ru.wfe.to_bits());
    let mu = psf::geometric_mtf(&u, f, Waves::One(0), 64, &freqs, false).unwrap();
    let mz = psf::geometric_mtf(&z, f, Waves::One(0), 64, &freqs, false).unwrap();
    assert_eq!(mu.tan, mz.tan);
}
