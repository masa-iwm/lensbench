//! 像の模擬（[`image_sim`]）: 明るさの単位・PSF の和・歪曲の向き・RGB のチャンネル

use lensbench_analysis::imagesim::{
    flip, image_sim, rotate, Aberrations, Flip, ImageSimOpts, Rotate, SimImage,
};
use lensbench_core::system::System;
use lensbench_formats::zmx::{self, LoadOptions};
use std::path::PathBuf;

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join("tests/refs").join(name).join("system.zmx"), &opt).unwrap()
}

fn gray(w: usize, h: usize, f: impl Fn(usize, usize) -> f64) -> SimImage {
    SimImage {
        width: w,
        height: h,
        channels: vec![(0..w * h).map(|k| f(k % w, k / w)).collect()],
    }
}

/// 一様な元の画像（値 1）は、収差なし・周辺光量なしで軸上の検出器の画素が 1。回折 PSF をかけても（PSF の和が 1
/// なので）中央は 1 のまま
#[test]
fn uniform_image_is_one_on_axis() {
    let s = load("A0");
    for ab in [
        Aberrations::None,
        Aberrations::Diffraction,
        Aberrations::Geometric,
    ] {
        let o = ImageSimOpts {
            field_height: 10.0,
            psf_grid: (3, 3),
            map_grid: 9,
            aberrations: ab,
            relative_illumination: false,
            ..Default::default()
        };
        let r = image_sim(&s, &gray(32, 32, |_, _| 1.0), &o).unwrap();
        let im = &r.image;
        let c = im.channels[0][16 * 32 + 16];
        assert!((c - 1.0).abs() < 2e-2, "{ab:?}: center {c}");
    }
}

/// 点（1 画素）の像: 収差なしは明るさの和が倍率どおりに保たれ、回折 PSF の像も和は同じ（像の外に出ない限り）
#[test]
fn point_source_keeps_energy() {
    let s = load("A0");
    let src = gray(33, 33, |i, j| if i == 16 && j == 16 { 1.0 } else { 0.0 });
    let base = ImageSimOpts {
        field_height: 4.0,
        psf_grid: (1, 1),
        map_grid: 5,
        relative_illumination: false,
        ..Default::default()
    };
    let none = image_sim(
        &s,
        &src,
        &ImageSimOpts {
            aberrations: Aberrations::None,
            ..base.clone()
        },
    )
    .unwrap();
    let diff = image_sim(&s, &src, &base).unwrap();
    let sum =
        |r: &lensbench_analysis::imagesim::ImageSimOut| r.image.channels[0].iter().sum::<f64>();
    assert!((sum(&none) - sum(&diff)).abs() < 1e-6 * sum(&none));
    // 収差なしの点は中央の画素に来る
    let k = none.image.channels[0]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    assert_eq!(k, 16 * 33 + 16);
}

/// 元の画像の上（行 0）は視野の +y。A0 の +y の視野の主光線は像面の +y に来るので、上の明るい点は検出器の上半分に来る
#[test]
fn orientation_follows_the_chief_ray() {
    let s = load("A0");
    let src = gray(32, 32, |i, j| {
        if j < 4 && (14..18).contains(&i) {
            1.0
        } else {
            0.0
        }
    });
    let o = ImageSimOpts {
        field_height: 20.0,
        psf_grid: (1, 1),
        map_grid: 9,
        aberrations: Aberrations::None,
        relative_illumination: false,
        ..Default::default()
    };
    let r = image_sim(&s, &src, &o).unwrap();
    let ch = &r.image.channels[0];
    let (top, bottom): (f64, f64) = (ch[..16 * 32].iter().sum(), ch[16 * 32..].iter().sum());
    assert!(top > 0.0 && bottom == 0.0, "top {top} bottom {bottom}");
}

/// RGB は 3 チャンネル、周辺光量で端が暗くなる
#[test]
fn rgb_and_relative_illumination() {
    let s = load("D0");
    let src = SimImage {
        width: 24,
        height: 24,
        channels: vec![vec![1.0; 24 * 24]; 3],
    };
    let o = ImageSimOpts {
        field_height: 2.0 * s.fields.iter().map(|f| f.y.abs()).fold(0.0, f64::max) / 2f64.sqrt(),
        psf_grid: (3, 3),
        map_grid: 9,
        ..Default::default()
    };
    let r = image_sim(&s, &src, &o).unwrap();
    assert_eq!(r.image.channels.len(), 3);
    for ch in &r.image.channels {
        let c = ch[12 * 24 + 12];
        assert!(c > 0.5 && c < 1.2, "center {c}");
    }
}

/// 一様な元の画像は端の画素まで 1（面積の差分が格子の外で切り詰められない）
#[test]
fn uniform_image_reaches_the_edges() {
    let s = load("A0");
    let o = ImageSimOpts {
        field_height: 10.0,
        psf_grid: (1, 1),
        map_grid: 9,
        aberrations: Aberrations::None,
        relative_illumination: false,
        ..Default::default()
    };
    let r = image_sim(&s, &gray(32, 32, |_, _| 1.0), &o).unwrap();
    let ch = &r.image.channels[0];
    for (i, j) in [(0, 0), (0, 16), (16, 0), (31, 31), (31, 16)] {
        let v = ch[j * 32 + i];
        assert!((v - 1.0).abs() < 3e-2, "pixel ({i}, {j}) {v}");
    }
}

/// 検出器の画素が PSF の標本の間隔より細かくても、回折の像に櫛の凸凹が出ない（一様な元の画像の中央が平ら）
#[test]
fn fine_detector_pixels_give_a_smooth_image() {
    let s = load("D0");
    let o = ImageSimOpts {
        field_height: 1.0,
        psf_grid: (1, 1),
        map_grid: 5,
        pixel: Some(0.0005),
        size: Some((96, 96)),
        relative_illumination: false,
        ..Default::default()
    };
    let r = image_sim(&s, &gray(64, 64, |_, _| 1.0), &o).unwrap();
    let ch = &r.image.channels[0];
    let mid: Vec<f64> = (32..64)
        .flat_map(|j| (32..64).map(move |i| (i, j)))
        .map(|(i, j)| ch[j * 96 + i])
        .collect();
    let (lo, hi) = mid
        .iter()
        .fold((f64::INFINITY, 0.0f64), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi - lo < 1e-2 && (hi - 1.0).abs() < 2e-2, "{lo}..{hi}");
}

/// 1 点の回折の像は、検出器の画素が PSF の標本の間隔より細かくても、中央の行が中心から単調に下がる（核が櫛にならない）
#[test]
fn point_image_has_no_comb() {
    let s = load("D0");
    let o = ImageSimOpts {
        field_height: 0.0107, // 元の 1 画素の像が 0.2 µm ほど（PSF より十分小さい）
        psf_grid: (1, 1),
        map_grid: 3,
        pixel: Some(0.0005),
        size: Some((41, 41)),
        relative_illumination: false,
        ..Default::default()
    };
    let r = image_sim(
        &s,
        &gray(33, 33, |i, j| if i == 16 && j == 16 { 1.0 } else { 0.0 }),
        &o,
    )
    .unwrap();
    let ch = &r.image.channels[0];
    let row: Vec<f64> = (0..41).map(|i| ch[20 * 41 + i]).collect();
    let peak = (0..41).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
    for k in 0..3 {
        assert!(
            row[peak + k + 1] < row[peak + k] && row[peak - k - 1] < row[peak - k],
            "{row:?}"
        );
    }
}

/// 幾何 PSF: 点（1 画素）の像の明るさの和は収差なしと同じで、重心のまわりの広がり（RMS）はスポットの RMS
/// （[`lensbench_analysis::analysis::spot`]、重心から）に、検出器の画素と元の 1 画素の像の分を足したものに近い
#[test]
fn geometric_psf_spreads_like_the_spot() {
    let s = load("A0");
    let w = s.primary;
    // 軸上の点だけの元の画像（視野の高さ 2 度の 33 × 33）
    let src = gray(33, 33, |i, j| if i == 16 && j == 16 { 1.0 } else { 0.0 });
    let sp = lensbench_analysis::analysis::spot(
        &s,
        lensbench_core::system::FieldSel::Norm(0.0, 0.0),
        lensbench_core::Waves::One(w),
        20,
    )
    .unwrap();
    let pixel = sp.rms_centroid / 8.0;
    let base = ImageSimOpts {
        field_height: 2.0,
        psf_grid: (1, 1),
        map_grid: 5,
        relative_illumination: false,
        pixel: Some(pixel),
        size: Some((161, 161)),
        wave: Some(w),
        pupil: Some(256),
        ..Default::default()
    };
    let run = |ab| {
        image_sim(
            &s,
            &src,
            &ImageSimOpts {
                aberrations: ab,
                ..base.clone()
            },
        )
        .unwrap()
    };
    let (none, geo) = (run(Aberrations::None), run(Aberrations::Geometric));
    let moments = |r: &lensbench_analysis::imagesim::ImageSimOut| {
        let (w, im) = (r.image.width, &r.image.channels[0]);
        let (mut t, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for (k, &v) in im.iter().enumerate() {
            t += v;
            sx += v * (k % w) as f64;
            sy += v * (k / w) as f64;
        }
        let (cx, cy) = (sx / t, sy / t);
        let mut m2 = 0.0;
        for (k, &v) in im.iter().enumerate() {
            m2 += v * (((k % w) as f64 - cx).powi(2) + ((k / w) as f64 - cy).powi(2));
        }
        (t, (m2 / t).sqrt() * r.pixel)
    };
    let ((t0, r0), (t1, r1)) = (moments(&none), moments(&geo));
    assert!((t1 / t0 - 1.0).abs() < 1e-9, "sum {t1} vs {t0}");
    // 収差なしの広がり（画素・元の画素の像）を 2 乗で引く
    let rms = (r1 * r1 - r0 * r0).sqrt();
    assert!(
        (rms / sp.rms_centroid - 1.0).abs() < 0.05,
        "geometric PSF rms {rms} vs spot {}",
        sp.rms_centroid
    );
}

/// 反転と回転: 90 度は反時計回り（上の行が左の列へ）、4 回で元に戻り、上下＋左右の反転は 180 度と同じ
#[test]
fn flip_and_rotate_the_source() {
    // 3 × 2（幅 3）: 値は 10·行 + 列
    let im = gray(3, 2, |i, j| (10 * j + i) as f64);
    let r = rotate(&im, Rotate::R90);
    assert_eq!((r.width, r.height), (2, 3));
    // 反時計回り: 元の右上 (2, 0) が左上、元の左上 (0, 0) が左下
    assert_eq!(r.channels[0], vec![2.0, 12.0, 1.0, 11.0, 0.0, 10.0]);
    assert_eq!(
        rotate(&im, Rotate::R270),
        rotate(&rotate(&r, Rotate::R90), Rotate::R90)
    );
    let mut t = im.clone();
    for _ in 0..4 {
        t = rotate(&t, Rotate::R90);
    }
    assert_eq!(t, im);
    assert_eq!(flip(&im, Flip::Both), rotate(&im, Rotate::R180));
    assert_eq!(
        flip(&im, Flip::TopBottom).channels[0],
        vec![10.0, 11.0, 12.0, 0.0, 1.0, 2.0]
    );
    assert_eq!(
        flip(&im, Flip::LeftRight).channels[0],
        vec![2.0, 1.0, 0.0, 12.0, 11.0, 10.0]
    );
}

/// 元の画像を 90 度回すと、上の明るい点は検出器の左半分に来る。検出器の像の左右の反転で右半分に戻る
#[test]
fn rotated_source_turns_the_image() {
    let s = load("A0");
    let src = gray(32, 32, |i, j| {
        if j < 4 && (14..18).contains(&i) {
            1.0
        } else {
            0.0
        }
    });
    let o = ImageSimOpts {
        field_height: 20.0,
        psf_grid: (1, 1),
        map_grid: 9,
        aberrations: Aberrations::None,
        relative_illumination: false,
        rotate: Rotate::R90,
        ..Default::default()
    };
    let halves = |o: &ImageSimOpts| {
        let r = image_sim(&s, &src, o).unwrap();
        let ch = &r.image.channels[0];
        let (mut l, mut rr) = (0.0, 0.0);
        for (k, v) in ch.iter().enumerate() {
            if k % 32 < 16 {
                l += v;
            } else {
                rr += v;
            }
        }
        (l, rr)
    };
    let (l, r) = halves(&o);
    assert!(l > 0.0 && r == 0.0, "left {l} right {r}");
    let (l, r) = halves(&ImageSimOpts {
        flip_image: Flip::LeftRight,
        ..o.clone()
    });
    assert!(r > 0.0 && l == 0.0, "left {l} right {r}");
}

/// 中心の視野が軸の外: 元の画像の中央の点は検出器の中央に来て、検出器の中心はその視野の主光線の像点
#[test]
fn center_field_moves_the_detector() {
    let s = load("A0");
    let fy = s.fields.iter().map(|f| f.y.abs()).fold(0.0, f64::max) / 2.0;
    let src = gray(33, 33, |i, j| if i == 16 && j == 16 { 1.0 } else { 0.0 });
    let o = ImageSimOpts {
        field_height: 4.0,
        psf_grid: (1, 1),
        map_grid: 5,
        aberrations: Aberrations::None,
        relative_illumination: false,
        center_field: (0.0, fy),
        ..Default::default()
    };
    let r = image_sim(&s, &src, &o).unwrap();
    let k = r.image.channels[0]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    assert_eq!(k, 16 * 33 + 16);
    let (_, ny) = s.field_norm_values();
    let c = s
        .try_trace(
            lensbench_core::system::FieldSel::Norm(0.0, fy / ny),
            &[0.0],
            &[0.0],
            Some(s.primary),
            None,
            false,
        )
        .unwrap();
    assert!((r.center.1 - c.rays[0].xyz[1]).abs() < 1e-12 && r.center.1.abs() > 1.0);
}

/// Huygens PSF: 無収差の D0 の点の像は回折（FFT PSF）とほぼ同じ（明るさの和は 1 %、最大は 2 % 以内）
#[test]
fn huygens_psf_matches_fft_on_a_perfect_lens() {
    let s = load("D0");
    let base = ImageSimOpts {
        field_height: 0.0107,
        psf_grid: (1, 1),
        map_grid: 3,
        pixel: Some(0.0005),
        size: Some((41, 41)),
        relative_illumination: false,
        wave: Some(s.primary),
        ..Default::default()
    };
    let src = gray(33, 33, |i, j| if i == 16 && j == 16 { 1.0 } else { 0.0 });
    let run = |ab| {
        image_sim(
            &s,
            &src,
            &ImageSimOpts {
                aberrations: ab,
                ..base.clone()
            },
        )
        .unwrap()
    };
    let (f, h) = (run(Aberrations::Diffraction), run(Aberrations::Huygens));
    let sum =
        |r: &lensbench_analysis::imagesim::ImageSimOut| r.image.channels[0].iter().sum::<f64>();
    let max = |r: &lensbench_analysis::imagesim::ImageSimOut| {
        r.image.channels[0].iter().copied().fold(0.0, f64::max)
    };
    // 和は検出器（20 µm 角）の外に出る裾の分だけ違う
    assert!(
        (sum(&h) / sum(&f) - 1.0).abs() < 0.01,
        "sum {} vs {}",
        sum(&h),
        sum(&f)
    );
    assert!(
        (max(&h) / max(&f) - 1.0).abs() < 0.02,
        "max {} vs {}",
        max(&h),
        max(&f)
    );
}
