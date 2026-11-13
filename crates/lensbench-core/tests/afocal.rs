//! アフォーカル像空間（.zmx の FTYP の 7 番目）: 波面・Zernike を射出瞳の位置で主光線に垂直な平面を
//! 基準に測る。OpticStudio との照合は非公開側（8 コンフィグのリレー系、Zernike 37 項が 1.2e-7 波で一致）

use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::psf::{self, PsfOpts};
use lensbench_core::system::{AfocalUnit, FieldSel, System};
use lensbench_core::{zmx, zmx_write};

/// ケプラー型の望遠鏡（物体は無限遠、視野 0・0.5°）。afocal で FTYP の 7 番目、img で像面までの距離
fn telescope(lens: &str, afocal: bool, img: f64) -> System {
    telescope_epd(lens, afocal, img, 10.0)
}

fn telescope_epd(lens: &str, afocal: bool, img: f64, epd: f64) -> System {
    let txt = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD {epd}\nFTYP 0 0 2 1 0 0 {}\n\
XFLN 0 0\nYFLN 0 0.5\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
{lens}SURF 9\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {img}\nSURF 10\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        afocal as i32
    );
    zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap()
}

/// 近軸の薄肉レンズ（f 50 と 25 を 75 離す）
const PARAXIAL: &str = "SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  DISZ 75\n\
SURF 2\n  TYPE PARAXIAL\n  PARM 1 25\n  DISZ 0\n";

/// 球面の単レンズ 2 枚（球面収差あり。n 1.5、平凸）
const REAL: &str =
    "SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.04\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 73\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.08\n  DISZ 3\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

fn zern(s: &System, f: usize) -> Vec<f64> {
    analysis::zernike(
        s,
        FieldSel::Index(f),
        None,
        ZernKind::Standard,
        37,
        64,
        false,
    )
    .unwrap()
    .coef
}

#[test]
fn reads_and_writes_the_flag() {
    let s = telescope(PARAXIAL, true, 20.0);
    assert!(s.afocal);
    assert!(!telescope(PARAXIAL, false, 20.0).afocal);
    let txt = zmx_write::write(&s).unwrap();
    let line = txt.lines().find(|l| l.starts_with("FTYP")).unwrap();
    assert_eq!(line.split_whitespace().nth(7), Some("1"), "{line}");
    let back = zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap();
    assert!(back.afocal);
}

/// 近軸レンズの望遠鏡は平面波を出す: 平面基準なら全項 0、像点への参照球だと大きな Z4 が出る
#[test]
fn perfect_telescope_is_a_plane_wave() {
    for f in 0..2 {
        let z = zern(&telescope(PARAXIAL, true, 20.0), f);
        let m = z[1..].iter().fold(0.0f64, |a, b| a.max(b.abs()));
        assert!(m < 1e-9, "視野 {f}: 最大 {m}");
        let zs = zern(&telescope(PARAXIAL, false, 20.0), f);
        assert!(zs[3].abs() > 10.0, "視野 {f}: 参照球の Z4 {}", zs[3]);
    }
}

/// 平面波の基準は像面の位置によらない（参照球は像面までの距離で変わる）
#[test]
fn independent_of_the_image_distance() {
    for f in 0..2 {
        let a = zern(&telescope(REAL, true, 10.0), f);
        let b = zern(&telescope(REAL, true, 300.0), f);
        assert!(a[10].abs() > 1e-2, "球面収差が出る: Z11 {}", a[10]);
        for k in 1..37 {
            assert!(
                (a[k] - b[k]).abs() < 1e-9,
                "視野 {f} Z{}: {} {}",
                k + 1,
                a[k],
                b[k]
            );
        }
    }
}

/// 像面の前に焦点距離 F の理想レンズを置いた焦点のある系（像面はその焦点）。アフォーカルの角度 θ は x = F tan θ に写る
fn focused(lens: &str, f: f64, epd: f64) -> System {
    let txt = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD {epd}\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 0.5\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
{lens}SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 20\nSURF 9\n  TYPE PARAXIAL\n  PARM 1 {f}\n  DISZ {f}\n\
SURF 10\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap()
}

const F: f64 = 200.0;

/// スポットは光線の向きの角度 atan(l/n)・atan(m/n) [mrad]（OpticStudio のスポットと同じ定義）。
/// 理想レンズの像面の位置は F tan θ なので、atan(x / F) と一致する
#[test]
fn spot_is_the_ray_angle() {
    let a = telescope(REAL, true, 20.0);
    let b = focused(REAL, F, 10.0);
    assert_eq!(a.image_unit(), "mrad");
    for f in 0..2 {
        let sa = analysis::spot(&a, FieldSel::Index(f), None, 6).unwrap();
        let sb = analysis::spot(&b, FieldSel::Index(f), None, 6).unwrap();
        assert_eq!(sa.x.len(), sb.x.len());
        for i in 0..sa.x.len() {
            let (x, y) = ((sb.x[i] / F).atan() * 1e3, (sb.y[i] / F).atan() * 1e3);
            assert!(
                (sa.x[i] - x).abs() < 1e-9 && (sa.y[i] - y).abs() < 1e-9,
                "視野 {f} 光線 {i}"
            );
        }
        // 0.5° の視野の主光線（望遠鏡の倍率 2 で 1°）
        if f == 1 {
            assert!(
                (sa.chief.1.abs() - 1e3 * 1f64.to_radians()).abs() < 0.5,
                "{:?}",
                sa.chief
            );
        }
        assert!(sa.rms_chief > 1e-3, "球面収差で広がる: {}", sa.rms_chief);
    }
    // 単位を度にすると値が 1 mrad / 1° 倍になる
    let mut d = a.clone();
    d.afocal_unit = AfocalUnit::Degrees;
    let s1 = analysis::spot(&a, FieldSel::Index(1), None, 6).unwrap();
    let s2 = analysis::spot(&d, FieldSel::Index(1), None, 6).unwrap();
    let k = 1e-3f64.to_degrees();
    assert!((s2.rms_chief - s1.rms_chief * k).abs() < 1e-12 * s1.rms_chief.max(1.0));
    assert_eq!(d.image_unit(), "deg");
}

/// MTF の周波数は cycles/mrad。理想レンズを置いた系の cycles/mm と f [cycles/mm] = ν / (F · 1e-3) で対応する。
/// 近軸の望遠鏡の遮断周波数は D′/λ（D′ = 5 mm で 9.0909 cycles/mrad、OpticStudio と同じ）
#[test]
fn mtf_is_per_angle() {
    let p = telescope(PARAXIAL, true, 20.0);
    let m = analysis::mtf(&p, FieldSel::Index(0), None, None, &[0.0, 4.0]).unwrap();
    assert!(
        (m.cutoff.0 - 5.0 / 0.55e-3 * 1e-3).abs() < 1e-6,
        "{:?}",
        m.cutoff
    );
    assert!(
        (m.tan[1] - psf::diffraction_limit(4.0 / m.cutoff.1)).abs() < 2e-3,
        "{:?}",
        m.tan
    );
    let a = telescope(PARAXIAL, true, 20.0);
    let b = focused(PARAXIAL, F, 10.0);
    for f in 0..2 {
        let nu = [0.0, 1.0, 2.5, 5.0];
        let fm: Vec<f64> = nu.iter().map(|v| v / (F * 1e-3)).collect();
        let ma = analysis::mtf(&a, FieldSel::Index(f), None, None, &nu).unwrap();
        let mb = analysis::mtf(&b, FieldSel::Index(f), None, None, &fm).unwrap();
        for (k, v) in nu.iter().enumerate() {
            assert!(
                (ma.tan[k] - mb.tan[k]).abs() < 2e-3 && (ma.sag[k] - mb.sag[k]).abs() < 2e-3,
                "視野 {f} ν {v}: {} {} / {} {}",
                ma.tan[k],
                mb.tan[k],
                ma.sag[k],
                mb.sag[k]
            );
        }
        let ga = psf::geometric_mtf(&a, FieldSel::Index(f), None, 32, &nu, false).unwrap();
        let gb = psf::geometric_mtf(&b, FieldSel::Index(f), None, 32, &fm, false).unwrap();
        for k in 0..nu.len() {
            assert!(
                (ga.tan[k] - gb.tan[k]).abs() < 2e-3,
                "幾何 視野 {f}: {:?} {:?}",
                ga.tan,
                gb.tan
            );
        }
    }
}

/// PSF の格子は角度 [mrad]。理想レンズを置いた系の格子 [mm] を F · 1e-3 で割ったものと同じ
#[test]
fn psf_is_per_angle() {
    let a = telescope(PARAXIAL, true, 20.0);
    let b = focused(PARAXIAL, F, 10.0);
    let o = PsfOpts::default();
    let pa = psf::psf(&a, FieldSel::Index(1), None, o).unwrap();
    let pb = psf::psf(&b, FieldSel::Index(1), None, o).unwrap();
    let r = (pa.delta.0 * F * 1e-3 - pb.delta.0) / pb.delta.0;
    assert!(r.abs() < 2e-3, "間隔 {:?} {:?}", pa.delta, pb.delta);
    assert!(
        ((pa.center.1 * 1e-3).tan() * F - pb.center.1).abs() < 1e-9,
        "{:?} {:?}",
        pa.center,
        pb.center
    );
    assert!(
        (pa.peak - pb.peak).abs() < 5e-3 * pb.peak,
        "ピーク {} {}",
        pa.peak,
        pb.peak
    );
}

/// OpticStudio 26.1 で測った値（tel_real_epd3: REAL の望遠鏡を入射瞳 3 mm にしたもの、視野 (0, 0.5°)・(0.3°, 0.4°)、
/// 単位 mrad）。FFT MTF・PSF は sampling 128、幾何 MTF は 64×64、PSF の間隔は OpticStudio の既定 0.0914374977 mrad。
/// スポットの RMS は OpticStudio のスポット図（六角の光線の並び）と求積（RSCE と同じ）で 3% 違うので比べない
/// （光線ごとの角度は 3e-11 µrad で一致を確認済み）
struct Os {
    chief: [f64; 2],
    tan: [f64; 6],
    sag: [f64; 6],
    geo_tan: [f64; 3],
    geo_sag: [f64; 3],
    psf_peak: f64,
}

const OS_FREQS: [f64; 6] = [0.090301, 0.180602, 0.270903, 0.5, 1.0, 2.0];

const OS_EPD3: [Os; 2] = [
    Os {
        chief: [0.0, -17.7272191],
        tan: [
            0.914415,
            0.771203,
            0.607745,
            0.262235389,
            0.0369655232,
            0.0187487745,
        ],
        sag: [
            0.922862,
            0.798612,
            0.655572,
            0.334993725,
            0.0346152386,
            0.034177156,
        ],
        geo_tan: [0.9503, 0.811963, 0.614901],
        geo_sag: [0.96064, 0.849148, 0.684025],
        psf_peak: 0.154414798,
    },
    Os {
        chief: [-10.6368712, -14.1821797],
        tan: [
            0.917444,
            0.780954,
            0.624566,
            0.286660061,
            0.0312240937,
            0.0227112078,
        ],
        sag: [
            0.919809,
            0.788628,
            0.637955,
            0.307007953,
            0.0291871381,
            0.0269592105,
        ],
        geo_tan: [0.954012, 0.825208, 0.639148],
        geo_sag: [0.956907, 0.835619, 0.6585],
        psf_peak: 0.154415514,
    },
];

fn epd3() -> System {
    let txt = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 3\nFTYP 0 0 3 1 0 0 1\n\
XFLN 0 0 0.3\nYFLN 0 0.5 0.4\nFWGN 1 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
{REAL}SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 20\nSURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap()
}

/// OpticStudio との照合（斜めの視野 2 つ）。FFT MTF の差は低い周波数で 3e-3 以下、0.5 cycles/mrad で 9e-3。
/// OpticStudio を sampling 512 にしても 0.5 cycles/mrad の差は 7e-3 ほど残る。OpticStudio は周波数の縮尺に
/// 全視野で 1 つのビーム幅（1.47768 mm）を使い、lensbench は視野ごとの実光線の幅を使うため（既知の差、合わせない）。
/// lensbench は求積の点を増やしても 1e-10 で変わらない。
/// 幾何 MTF は 7e-4 以下、PSF のピークは 1.3% 以下
#[test]
fn matches_opticstudio() {
    let s = epd3();
    for (k, o) in OS_EPD3.iter().enumerate() {
        let f = FieldSel::Index(k + 1);
        let sp = analysis::spot(&s, f, None, 6).unwrap();
        assert!(
            (sp.chief.0 - o.chief[0]).abs() < 1e-6 && (sp.chief.1 - o.chief[1]).abs() < 1e-6,
            "視野 {}: {:?}",
            k + 2,
            sp.chief
        );
        let m = analysis::mtf(&s, f, None, None, &OS_FREQS).unwrap();
        for (i, &nu) in OS_FREQS.iter().enumerate() {
            let tol = if nu == 0.5 { 1e-2 } else { 4e-3 };
            assert!(
                (m.tan[i] - o.tan[i]).abs() < tol && (m.sag[i] - o.sag[i]).abs() < tol,
                "視野 {} ν {}: {} {} / {} {}",
                k + 2,
                nu,
                m.tan[i],
                o.tan[i],
                m.sag[i],
                o.sag[i]
            );
        }
        let g = psf::geometric_mtf(&s, f, None, 64, &OS_FREQS[..3], false).unwrap();
        for i in 0..3 {
            assert!(
                (g.tan[i] - o.geo_tan[i]).abs() < 1.5e-3
                    && (g.sag[i] - o.geo_sag[i]).abs() < 1.5e-3,
                "幾何 視野 {}: {:?} {:?}",
                k + 2,
                g.tan,
                g.sag
            );
        }
        let po = PsfOpts {
            pupil: Some(128),
            image: 256,
            delta: Some(0.0914374977),
        };
        let p = psf::psf(&s, f, None, po).unwrap();
        assert!(
            (p.peak - o.psf_peak).abs() < 2e-2 * o.psf_peak,
            "PSF 視野 {}: {} {}",
            k + 2,
            p.peak,
            o.psf_peak
        );
    }
}
