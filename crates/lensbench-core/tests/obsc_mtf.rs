//! 中心遮蔽で主光線がけられる系の回折 MTF。基準の光線（主光線・瞳の大きさを決める光線）は
//! OpticStudio と同じく開口・遮蔽を無視し、瞳の光線だけがけられる。
use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx;
use lensbench_core::Waves;

/// 理想レンズ（f 100、入射瞳径 10、λ 0.55 µm）の前の平行光の中に遮蔽（半径 obsc）を置く
fn ideal(obsc: f64) -> System {
    let t = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 2\n  STOP\n  TYPE PARAXIAL\n  PARM 1 100\n  DISZ 100\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let mut s = zmx::build(&zmx::parse_text(t).unwrap(), &Default::default(), &[], None).unwrap();
    s.surfaces[1].ap = Aperture::Obsc {
        min: 0.0,
        max: obsc,
        dx: 0.0,
        dy: 0.0,
    };
    s.changed().unwrap();
    s
}

/// 半径 a・b の円の中心が d 離れたときの重なりの面積
fn lens_area(a: f64, b: f64, d: f64) -> f64 {
    if d >= a + b {
        return 0.0;
    }
    if d <= (a - b).abs() {
        return std::f64::consts::PI * a.min(b).powi(2);
    }
    let ca = ((d * d + a * a - b * b) / (2.0 * d * a))
        .clamp(-1.0, 1.0)
        .acos();
    let cb = ((d * d + b * b - a * a) / (2.0 * d * b))
        .clamp(-1.0, 1.0)
        .acos();
    a * a * (ca - ca.sin() * ca.cos()) + b * b * (cb - cb.sin() * cb.cos())
}

/// 無収差の輪帯瞳（外径 1、内径 e）の MTF（瞳を d = 2ν/νc ずらした輪帯どうしの重なり / 輪帯の面積）
fn annular(e: f64, v: f64) -> f64 {
    let d = 2.0 * v;
    let ov = lens_area(1.0, 1.0, d) - 2.0 * lens_area(1.0, e, d) + lens_area(e, e, d);
    ov / (std::f64::consts::PI * (1.0 - e * e))
}

#[test]
fn central_obscuration_matches_annular_pupil() {
    let e = 0.4;
    let s = ideal(5.0 * e);
    let cut = 10.0 / (0.55e-3 * 100.0);
    let fr: Vec<f64> = (0..10).map(|i| cut * i as f64 / 10.0).collect();
    for k in 0..2 {
        for m in [MtfMethod::Interp, MtfMethod::Fft] {
            let r =
                analysis::mtf_with(&s, FieldSel::Index(k), Waves::All, Some(128), &fr, m).unwrap();
            for (i, &f) in fr.iter().enumerate() {
                let a = annular(e, f / cut);
                for (what, v) in [("tan", r.tan[i]), ("sag", r.sag[i])] {
                    assert!(
                        (v - a).abs() < 5e-3,
                        "field {k} {m:?} {what} {f}: {v} vs {a}"
                    );
                }
            }
        }
    }
}

/// 正規化瞳の行 y で光線が通る x の区間: 単位円 ∩ 矩形の開口（y が [−0.3, 0.9]）から、中心 (0, cy)・半径 ob の
/// 遮蔽を除く
fn clipped_row(y: f64, ob: f64, cy: f64) -> Vec<(f64, f64)> {
    if !(-0.3..=0.9).contains(&y) || y.abs() >= 1.0 {
        return vec![];
    }
    let c = (1.0 - y * y).sqrt();
    let dy = y - cy;
    if dy.abs() < ob {
        let o = (ob * ob - dy * dy).sqrt();
        vec![(-c, -o), (o, c)]
    } else {
        vec![(-c, c)]
    }
}

fn overlap(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let mut s = 0.0;
    for &(p, q) in a {
        for &(r, t) in b {
            s += (q.min(t) - p.max(r)).max(0.0);
        }
    }
    s
}

/// 無収差で瞳が [`clipped_row`] の形の MTF（重なりの面積 / 面積）。ずれ d は正規化瞳で、(球欠, 子午)
fn clipped_mtf(ob: f64, cy: f64, d: (f64, f64)) -> (f64, f64) {
    let m = 200_000;
    let h = 2.0 / m as f64;
    let (mut area, mut sag, mut tan) = (0.0, 0.0, 0.0);
    for i in 0..m {
        let y = -1.0 + h * (i as f64 + 0.5);
        let r = clipped_row(y, ob, cy);
        if r.is_empty() {
            continue;
        }
        area += r.iter().map(|(a, b)| b - a).sum::<f64>();
        let sh: Vec<(f64, f64)> = r.iter().map(|&(a, b)| (a + d.0, b + d.0)).collect();
        sag += overlap(&r, &sh);
        tan += overlap(&r, &clipped_row(y - d.1, ob, cy));
    }
    (sag / area, tan / area)
}

/// 遮蔽に加えて偏心した矩形の開口で縁もけられる（内側の点も対称にはとれない）系。瞳の縁に合わせた求積で
/// 周波数 0 は 1、他の周波数も瞳の形から求めた値と合う（以前は周波数 0 でも 1 から 2e-2 ずれた）
#[test]
fn obscured_and_clipped() {
    let fr = [0.0, 1.0, 5.0, 20.0, 60.0, 120.0];
    for obsc in [0.0, 2.5] {
        let mut s = ideal(obsc);
        s.surfaces[2].ap = Aperture::Rect {
            hx: 6.0,
            hy: 3.0,
            dx: 0.0,
            dy: 1.5,
        };
        s.changed().unwrap();
        for k in 0..2 {
            // 遮蔽（絞りの 10 前）は視野 1° で瞳の y に 10 tan 1° ずれる
            let cy = if k == 0 {
                0.0
            } else {
                10.0 * 1f64.to_radians().tan() / 5.0
            };
            for m in [MtfMethod::Interp, MtfMethod::Direct] {
                for n in [None, Some(256)] {
                    let r =
                        analysis::mtf_with(&s, FieldSel::Index(k), Waves::All, n, &fr, m).unwrap();
                    for (i, &f) in fr.iter().enumerate() {
                        let (es, et) = clipped_mtf(
                            obsc / 5.0,
                            cy,
                            (2.0 * f / r.cutoff.sag, 2.0 * f / r.cutoff.tan),
                        );
                        let tol = if f == 0.0 { 1e-12 } else { 3e-4 };
                        assert!(
                            (r.sag[i] - es).abs() < tol && (r.tan[i] - et).abs() < tol,
                            "ob {obsc} field {k} {m:?} {n:?} {f}: sag {} vs {es}, tan {} vs {et}",
                            r.sag[i],
                            r.tan[i]
                        );
                    }
                }
            }
        }
    }
    let mut s = ideal(2.5);
    s.surfaces[2].ap = Aperture::Rect {
        hx: 6.0,
        hy: 3.0,
        dx: 0.0,
        dy: 1.5,
    };
    s.changed().unwrap();
    let sp = analysis::spot(&s, FieldSel::Index(0), Waves::All, 6).unwrap();
    assert!(sp.chief.0.is_finite() && sp.chief.1.is_finite());
}

/// 中心遮蔽で主光線がけられる系でも Huygens MTF が求まり、輪帯瞳の MTF と合う（以前は主光線を開口つきで見ていて
/// 「主光線がけられる」で止まった）
#[test]
fn huygens_central_obscuration() {
    use lensbench_core::huygens::{huygens_mtf, HuygensOpts};
    let e = 0.4;
    let s = ideal(5.0 * e);
    let cut = 10.0 / (0.55e-3 * 100.0);
    let fr: Vec<f64> = (0..8).map(|i| cut * i as f64 / 10.0).collect();
    let o = HuygensOpts {
        pupil: Some(64),
        image: 128,
        delta: None,
    };
    let r = huygens_mtf(&s, FieldSel::Index(0), Waves::All, o, &fr).unwrap();
    for (i, &f) in fr.iter().enumerate() {
        let a = annular(e, f / cut);
        for (what, v) in [("tan", r.tan[i]), ("sag", r.sag[i])] {
            assert!((v - a).abs() < 1e-2, "{what} {f}: {v} vs {a}");
        }
    }
}
