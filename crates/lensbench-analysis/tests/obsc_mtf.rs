//! 中心遮蔽で主光線がけられる系の Huygens MTF（FFT・補間の MTF の試験は core の tests/obsc_mtf.rs、系と輪帯瞳の式はそこと同じ）。基準の光線（主光線・瞳の大きさを決める光線）は
//! OpticStudio と同じく開口・遮蔽を無視し、瞳の光線だけがけられる。
use lensbench_analysis::huygens::{huygens_mtf, HuygensOpts};
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

/// 中心遮蔽で主光線がけられる系でも Huygens MTF が求まり、輪帯瞳の MTF と合う（以前は主光線を開口つきで見ていて
/// 「主光線がけられる」で止まった）
#[test]
fn huygens_central_obscuration() {
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
