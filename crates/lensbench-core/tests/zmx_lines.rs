//! 在庫レンズカタログの .zmx に出てくる行・面タイプ: トロイダル面（XDAT）、楕円の開口（ELAP）、
//! 円錐係数のピックアップ（PKUP）、読み飛ばす行（EDGE）。
use lensbench_core::surface::{Aperture, SurfType};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::trace::{sag, sag_toroidal};
use lensbench_core::{zmx, zmx_write};

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 4 -3\nYFLN 0 2 5\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

fn build(txt: &str) -> System {
    let raw = zmx::parse_text(txt).unwrap();
    zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap()
}

/// 面 1（与えた行）・平面・像面の単玉
fn lens(surf1: &str) -> String {
    format!(
        "{HEAD}SURF 1\n  STOP\n{surf1}  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

fn toroid(c: f64, k: f64, r_rot: f64, asph: &[f64]) -> String {
    let mut s = format!("  TYPE TOROIDAL\n  CURV {c}\n  CONI {k}\n  PARM 1 {r_rot}\n");
    for (i, a) in asph.iter().enumerate() {
        s += &format!("  PARM {} {a}\n", i + 2);
    }
    s += "  XDAT 1 0.000000000000E+00 0 0 1.000000000000E+00 0.000000000000E+00 0 \"\"\n";
    s += "  XDAT 2 1.000000000000E+02 0 0 1.000000000000E+00 0.000000000000E+00 0 \"\"\n";
    s
}

const PX: [f64; 6] = [0.0, 0.7, -0.5, 0.3, 0.0, -0.8];
const PY: [f64; 6] = [0.0, 0.2, 0.6, -0.9, 0.95, 0.0];

/// 全視野の光線の像面の位置・方向・光路長が同じ
fn same_rays(a: &System, b: &System, px: &[f64], py: &[f64], tol: f64) {
    for fi in 0..a.fields.len() {
        let ta = a.trace(FieldSel::Index(fi), px, py, None, None, false);
        let tb = b.trace(FieldSel::Index(fi), px, py, None, None, false);
        for (i, (ra, rb)) in ta.rays.iter().zip(&tb.rays).enumerate() {
            assert_eq!((ra.err, ra.vig), (rb.err, rb.vig), "視野 {fi} 光線 {i}");
            assert_eq!(ra.err, 0);
            for j in 0..3 {
                assert!(
                    (ra.xyz[j] - rb.xyz[j]).abs() < tol,
                    "視野 {fi} 光線 {i}: {:?} {:?}",
                    ra.xyz,
                    rb.xyz
                );
                assert!((ra.lmn[j] - rb.lmn[j]).abs() < tol, "視野 {fi} 光線 {i}");
            }
            assert!((ra.opl - rb.opl).abs() < tol, "視野 {fi} 光線 {i}");
        }
    }
}

#[test]
fn toroidal_sag_derivatives() {
    let (c, k, asph) = (1.0 / 15.0, -0.5, [0.0, 2e-5, -3e-8]);
    for cx in [0.0, 1.0 / 30.0, -1.0 / 25.0] {
        for (x, y) in [(0.0, 0.0), (1.3, -2.1), (-3.0, 4.0), (4.5, 0.3)] {
            let (z, zx, zy) = sag_toroidal(c, k, &asph, cx, x, y);
            let h = 1e-6;
            let fx = (sag_toroidal(c, k, &asph, cx, x + h, y).0
                - sag_toroidal(c, k, &asph, cx, x - h, y).0)
                / (2.0 * h);
            let fy = (sag_toroidal(c, k, &asph, cx, x, y + h).0
                - sag_toroidal(c, k, &asph, cx, x, y - h).0)
                / (2.0 * h);
            assert!(
                (zx - fx).abs() < 1e-8 && (zy - fy).abs() < 1e-8,
                "{cx} {x} {y}"
            );
            // x = 0 の断面は YZ 面の曲線
            assert_eq!(
                sag_toroidal(c, k, &asph, cx, 0.0, y).0,
                sag(c, k, &asph, y * y).0
            );
            // y を固めた断面は、軸から 1/cx − z(y) の円
            if cx != 0.0 {
                let z0 = sag(c, k, &asph, y * y).0;
                let rr = 1.0 / cx - z0;
                assert!(((1.0 / cx - z).powi(2) + x * x - rr * rr).abs() < 1e-9);
            }
        }
    }
}

/// 回転半径が y の曲率半径と同じトロイダル面は球面
#[test]
fn toroidal_with_equal_radii_is_a_sphere() {
    let a = build(&lens(&toroid(0.05, 0.0, 20.0, &[])));
    assert_eq!(a.surfaces[1].typ, SurfType::Toroidal);
    let b = build(&lens("  TYPE STANDARD\n  CURV 0.05\n"));
    same_rays(&a, &b, &PX, &PY, 1e-11);
    assert!((a.paraxial().efl - b.paraxial().efl).abs() < 1e-12);
}

/// 回転半径 0（無限）の円柱面: YZ 面の光線は偶数次非球面と、XZ 面の光線は平面と同じ
#[test]
fn cylinder_matches_evenasph_in_yz_and_flat_in_xz() {
    let asph = [0.0, 1e-4, -2e-6];
    let cyl = |xfln: &str, yfln: &str| {
        lens(&toroid(0.08, -0.7, 0.0, &asph))
            .replace("XFLN 0 4 -3", xfln)
            .replace("YFLN 0 2 5", yfln)
    };
    // y の視野と瞳の y だけ
    let a = build(&cyl("XFLN 0 0 0", "YFLN 0 2 -5"));
    // 偶数次非球面のパラメータ 1〜 は r^2〜（トロイダル面は 2〜 が y^2〜）
    let mut ev = "  TYPE EVENASPH\n  CURV 0.08\n  CONI -0.7\n".to_string();
    for (i, v) in asph.iter().enumerate() {
        ev += &format!("  PARM {} {v}\n", i + 1);
    }
    let b = build(
        &lens(&ev)
            .replace("XFLN 0 4 -3", "XFLN 0 0 0")
            .replace("YFLN 0 2 5", "YFLN 0 2 -5"),
    );
    let py = [0.0, 0.3, -0.6, 0.95];
    same_rays(&a, &b, &[0.0; 4], &py, 1e-11);
    assert!((a.paraxial().efl - b.paraxial().efl).abs() < 1e-12);
    // x の視野と瞳の x だけ: 平面
    let a = build(&cyl("XFLN 0 3 -4", "YFLN 0 0 0"));
    let b = build(
        &lens("  TYPE STANDARD\n  CURV 0.0\n")
            .replace("XFLN 0 4 -3", "XFLN 0 3 -4")
            .replace("YFLN 0 2 5", "YFLN 0 0 0"),
    );
    same_rays(&a, &b, &py, &[0.0; 4], 1e-11);
}

/// 一般のトロイダル面: 交点が面の上にあり、屈折が法線とスネルの法則を満たす
#[test]
fn toroidal_rays_land_on_the_surface() {
    let (c, k, r, asph) = (1.0 / 15.0, -0.5, -30.0, [0.0, 2e-5]);
    let s = build(&lens(&toroid(c, k, r, &asph)));
    assert!((s.surfaces[1].cx + 1.0 / 30.0).abs() < 1e-15);
    for fi in 0..s.fields.len() {
        let t = s.trace(FieldSel::Index(fi), &PX, &PY, None, Some(1), true);
        let rec = t.rec_at(1).unwrap();
        for (i, e) in (0..rec.xyz.len()).map(|i| (i, rec.entry(i))) {
            assert_eq!(t.rays[i].err, 0);
            let q = e.xyz;
            let (z, zx, zy) = sag_toroidal(c, k, &asph, 1.0 / r, q[0], q[1]);
            assert!((q[2] - z).abs() < 1e-12, "視野 {fi} 光線 {i}");
            // 屈折の後の方向と法線の外積の大きさ ×n' = 入射側の ×n（入射方向は面 0 の後の空気）
            let nn = {
                let v = [-zx, -zy, 1.0];
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                [v[0] / l, v[1] / l, v[2] / l]
            };
            let cr = |a: [f64; 3]| {
                [
                    a[1] * nn[2] - a[2] * nn[1],
                    a[2] * nn[0] - a[0] * nn[2],
                    a[0] * nn[1] - a[1] * nn[0],
                ]
            };
            let k_in = t.k0[i];
            let (a, b) = (cr(k_in), cr(e.lmn));
            for j in 0..3 {
                assert!((a[j] - 1.5168 * b[j]).abs() < 1e-12, "視野 {fi} 光線 {i}");
            }
        }
    }
}

#[test]
fn toroidal_zernike_terms_are_not_implemented() {
    let t = lens(&toroid(0.05, 0.0, 0.0, &[]))
        .replace("XDAT 1 0.000000000000E+00", "XDAT 1 3.000000000000E+00");
    let raw = zmx::parse_text(&t).unwrap();
    let e = zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    );
    assert!(e.unwrap_err().to_string().contains("Zernike"));
}

/// 楕円の開口: 半軸の外の光線はけられる
#[test]
fn elliptical_aperture() {
    let t = lens("  TYPE STANDARD\n  CURV 0.02\n").replace(
        "  DIAM 6 1 0 0 1 \"\"\n",
        "  DIAM 6 1 0 0 1 \"\"\n  ELAP 3 4.6 0\n  EDGE 2 10.125\n",
    );
    let s = build(&t);
    assert_eq!(
        s.surfaces[1].ap,
        Aperture::Ellipse {
            hx: 3.0,
            hy: 4.6,
            dx: 0.0,
            dy: 0.0
        }
    );
    // 瞳の半径 5: (0, 0.9) は y 4.5 で中、(0.9, 0) は x 4.5 で外、(0.5, 0.75) は 0.69 + 0.66 > 1 で外
    let tr = s.trace(
        FieldSel::Index(0),
        &[0.0, 0.9, 0.5, 0.4],
        &[0.9, 0.0, 0.75, 0.5],
        None,
        None,
        false,
    );
    let vig: Vec<usize> = tr.rays.iter().map(|r| r.vig).collect();
    assert_eq!(vig, vec![0, 1, 1, 0]);
}

/// 円錐係数のピックアップ（PKUP 元の面 倍率 列。オフセットはない。OpticStudio 2026 R1.01 で実測）
#[test]
fn conic_pickup() {
    let t = format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.05\n  CONI -0.4\n  DISZ 3\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.05\n  CONI 9\n  PKUP 1 -2 0\n  DISZ 40\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let s = build(&t);
    assert!((s.surfaces[2].k - 0.8).abs() < 1e-15);
    // 別の列（2 = 厚さ）からのピックアップ: 倍率 × 面 1 の厚さ
    let s = build(&t.replace("PKUP 1 -2 0\n", "PKUP 1 -2 2\n"));
    assert_eq!(s.surfaces[2].k, -6.0);
    // 知らない列は未対応
    let raw = zmx::parse_text(&t.replace("PKUP 1 -2 0\n", "PKUP 1 -2 999\n"));
    assert!(raw.is_err());
}

/// 書き出して読み戻しても同じ光線
#[test]
fn toroid_and_ellipse_round_trip() {
    let t = lens(&toroid(1.0 / 15.0, -0.5, -30.0, &[0.0, 2e-5])).replace(
        "  DIAM 6 1 0 0 1 \"\"\n",
        "  DIAM 6 1 0 0 1 \"\"\n  ELAP 6 7 0\n  OBDC 0.2 -0.1\n",
    );
    let a = build(&t);
    let b = build(&zmx_write::write(&a).unwrap());
    assert_eq!(a.surfaces[1].ap, b.surfaces[1].ap);
    assert_eq!(b.surfaces[1].typ, SurfType::Toroidal);
    same_rays(&a, &b, &PX, &PY, 1e-12);
}

/// 分散ありのモデルガラス: OpticStudio 2026 R1.01 の INDX（非公開側 tools/zos-cs/logs/modelglass.json の一部）
#[test]
fn model_glass_matches_opticstudio() {
    #[rustfmt::skip]
    let os = [
        (1.517, 52.0, 0.0, 0.35, 1.5462767712605419),
        (1.517, 52.0, 0.0, 0.5875618, 1.517001964311855),
        (1.517, 52.0, 0.0, 0.6562725, 1.5140069283794462),
        (1.517, 52.0, 0.0, 2.3, 1.4998772011159875),
        (1.55974358588, 42.4352086008, 0.0, 0.35, 1.6000712259033136),
        (1.55974358588, 42.4352086008, 0.0, 0.5875618, 1.5597461696289514),
        (1.55974358588, 42.4352086008, 0.0, 0.6562725, 1.5558351181081045),
        (1.55974358588, 42.4352086008, 0.0, 2.3, 1.5382715755426737),
        (1.45, 18.0, 0.01, 0.35, 1.5353779237154606),
        (1.45, 18.0, 0.01, 0.5875618, 1.4500047626002155),
        (1.45, 18.0, 0.01, 0.6562725, 1.4429683419731012),
        (1.45, 18.0, 0.01, 2.3, 1.4167988104648341),
        (2.05, 90.0, -0.008, 0.35, 2.0784796013633504),
        (2.05, 90.0, -0.008, 0.5875618, 2.050002393245533),
        (2.05, 90.0, -0.008, 0.6562725, 2.0462408444436826),
        (2.05, 90.0, -0.008, 2.3, 2.024985250161405),
        (1.75, 35.0, 0.0, 0.35, 1.817390311240109),
        (1.75, 35.0, 0.0, 0.5875618, 1.7500041692162833),
        (1.75, 35.0, 0.0, 0.6562725, 1.7437294224895663),
        (1.75, 35.0, 0.0, 2.3, 1.7166900380361565),
    ];
    for (nd, vd, dp, wl, n) in os {
        let v = lensbench_core::glass::model_index(nd, vd, dp, wl);
        assert!((v - n).abs() < 1e-14, "{nd} {vd} {dp} {wl}: {v} {n}");
    }
    // .zmx の GLAS ___BLANK（nd vd ΔPgF）から波長ごとに、書き出して読み戻しても同じ
    let t = lens("  TYPE STANDARD\n  CURV 0.05\n")
        .replace(
            "___BLANK 1 0 1.5168 0 0 0 0 0 0",
            "___BLANK 1 0 1.45 18 0.01 0 0 0 0",
        )
        .replace("WAVM 1 0.5876 1\n", "WAVM 1 0.35 1\nWAVM 2 2.3 1\n")
        .replace("FTYP 0 0 3 1 0 0 0", "FTYP 0 0 3 2 0 0 0");
    let s = build(&t);
    let n = &s.surfaces[1].n;
    assert_eq!(n.len(), 2);
    assert!((n[0] - 1.5353779237154606).abs() < 1e-14 && (n[1] - 1.4167988104648341).abs() < 1e-14);
    let b = build(&zmx_write::write(&s).unwrap());
    assert_eq!(b.surfaces[1].n, s.surfaces[1].n);
    assert_eq!(b.surfaces[1].mat, "1.45,18,0.01");
}

/// 像面がガラスの中にある系の EFL: OpticStudio 2026 R1.01 の System Data
/// （面 1 R=±20、N-BK7 n=1.5167984 の中に像面、t=60、視野 5°）
#[test]
fn efl_with_image_in_glass() {
    let sys = |r: f64, ap: &str| {
        build(&format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\n{ap}\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 5\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV {}\n  DISZ 60\n  GLAS ___BLANK 1 0 1.5167984 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
            1.0 / r
        ))
    };
    let close = |a: f64, b: f64| (a - b).abs() < 2e-6 * b.abs();
    for r in [20.0, -20.0] {
        let d = lensbench_core::report::system_data(&sys(r, "ENPD 4"));
        assert!(close(d.effl, 38.69981 * r.signum()), "{}", d.effl);
        assert!(close(d.isfn, 9.674952), "{}", d.isfn);
        assert!(close(d.pwfn, 9.674952), "{}", d.pwfn);
        assert!(close(d.pimh, 3.385794), "{}", d.pimh);
        // 像空間の NA はヘルプの定義 n′·sin(atan u′)（OpticStudio の表示 0.05161097 とは違う）
        let u: f64 = 2.0 / (1.5167984 * 38.69981);
        assert!(close(d.isna, 1.5167984 * u.atan().sin()), "{}", d.isna);
        assert!((d.isna - 0.0516499).abs() < 1e-7, "{}", d.isna);
        for ap in ["FNUM 5 0", "FNUM 5 1"] {
            let d = lensbench_core::report::system_data(&sys(r, ap));
            assert!(close(d.epdi, 7.739961), "{ap} {}", d.epdi);
        }
    }
}

/// 多重構成の PRAM（面の PARM の値を構成ごとに変える）と、光線追跡に効かない行（SCOL・TCED、MCOM・CWGT）
#[test]
fn pram_and_harmless_lines() {
    // 面 1 の後に座標ブレーク（面 2、PARM 2 = y 偏心）を置き、構成 2 で偏心を変える
    let base = |dec: f64, extra_mce: &str| {
        format!(
            "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  SCOL 3\n  TCED 8.25 0 0\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE COORDBRK\n  PARM 1 0\n  PARM 2 {dec}\n  PARM 3 0\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ 0\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n{extra_mce}"
        )
    };
    let extra = "MNUM 2 1\n\
PRAM   2   1 2.0E-01 0 2 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\n\
PRAM   2   2 -3.0E-01 0 2 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\n\
MCOM   0   1 a 0 0 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\nMCOM   0   2 b 0 0 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\n\
CWGT   0   1 1.0E+00 0 0 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\nCWGT   0   2 2.0E+00 0 0 0 1 1 1.0E+00 0.0E+00 0 \"\" 0\n";
    let txt = base(5.0, extra);
    let raw = zmx::parse_text(&txt).unwrap();
    for (cfg, dec) in [(1, 0.2), (2, -0.3)] {
        let s = zmx::build(
            &raw,
            &zmx::LoadOptions {
                cfg,
                ..Default::default()
            },
            &[],
            None,
        )
        .unwrap();
        // 同じ偏心を PARM に直接書いた系と同じ光線
        let r = build(&base(dec, ""));
        same_rays(&s, &r, &PX, &PY, 1e-12);
        assert_eq!(s.surfaces[2].typ, SurfType::CoordBrk);
    }
}

/// mm 以外のレンズ単位（in・cm・m）は読込で mm に直す: 同じ系を mm で書いたものと面の値・光線が一致
#[test]
fn lens_units_convert_to_mm() {
    let sys = |unit: &str, f: f64| {
        // mm の値を f（mm / 単位）で割って書く
        let l = |v: f64| v / f;
        format!(
            "VERS 260327 42\nMODE SEQ\nUNIT {unit} X W X CM MR CPMM\nENPD {}\nFTYP 1 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 {}\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {}\n\
SURF 1\n  STOP\n  TYPE EVENASPH\n  CURV {}\n  PARM 1 {}\n  PARM 2 {}\n  CONI -0.5\n  DISZ {}\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM {} 1 0 0 1 \"\"\n  CLAP 0 {} 0\n\
SURF 2\n  TYPE COORDBRK\n  PARM 1 {}\n  PARM 2 0\n  PARM 3 0.5\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ 0\n\
SURF 3\n  TYPE STANDARD\n  CURV {}\n  DISZ {}\n  MAZH 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
            l(10.0),
            l(2.0),
            l(500.0),
            0.04 * f,
            1e-5 * f.powi(1),
            -2e-8 * f.powi(3),
            l(4.0),
            l(6.0),
            l(5.5),
            l(0.1),
            -0.01 * f,
            l(40.0),
        )
    };
    let mm = build(&sys("MM", 1.0));
    for (unit, f) in [("IN", 25.4), ("CM", 10.0), ("METER", 1000.0)] {
        let s = build(&sys(unit, f));
        for (a, b) in mm.surfaces.iter().zip(&s.surfaces) {
            let close = |x: f64, y: f64| (x - y).abs() <= 1e-12 * x.abs().max(1e-30) || x == y;
            assert!(
                close(a.c, b.c) && close(a.semi, b.semi),
                "{unit}: {a:?} {b:?}"
            );
            assert!(
                a.t.is_infinite() || close(a.t, b.t),
                "{unit}: t {} {}",
                a.t,
                b.t
            );
            for (x, y) in a.asph.iter().zip(&b.asph) {
                assert!(close(*x, *y), "{unit}: asph {x} {y}");
            }
        }
        same_rays(&mm, &s, &PX, &PY, 1e-9);
    }
    // 換算の決まっていない面タイプは止める
    let t = sys("IN", 25.4).replace("TYPE COORDBRK", "TYPE TOROIDAL");
    assert!(zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &zmx::LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None
    )
    .is_err());
}

/// 固定の半直径の浮動開口（OpticStudio 2026 R1 の読込と同じ）: MEMA の行の有無によらず玉の面に付ける。空気どうしの面はそのまま
#[test]
fn fixed_semi_diameter_becomes_floating_aperture() {
    use lensbench_core::surface::Aperture;
    let txt = |mema: bool| {
        let m = if mema { "  MEMA 6 0 0 0 1 \"\"\n" } else { "" };
        format!(
            "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 6 1 0 0 1 \"\"\n{m}\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n{m}\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n  DIAM 5.5 1 0 0 1 \"\"\n{m}\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        )
    };
    for mema in [false, true] {
        let s = build(&txt(mema));
        assert_eq!(
            s.surfaces[1].ap,
            Aperture::None,
            "空気どうしの絞り（MEMA {mema}）"
        );
        assert!(
            matches!(s.surfaces[2].ap, Aperture::Floating { .. }),
            "MEMA {mema}"
        );
        assert!(
            matches!(s.surfaces[3].ap, Aperture::Floating { .. }),
            "MEMA {mema}"
        );
    }
}

/// Odd Asphere の面のコバ厚のソルブ（アキシコン: 平面 → β1 の円錐、EDGE で中心厚を決める）
#[test]
fn edge_solve_on_odd_asphere() {
    let txt = format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 1\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  EDGE 5 0\n  DIAM 12.7 1 0 0 1 \"\"\n\
SURF 2\n  TYPE ODDASPHE\n  CURV 0.0\n  PARM 1 -0.0087268677907590\n  DISZ 100\n  DIAM 12.7 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let s = build(&txt);
    // 半直径 12.7 での矢高は β1·r なので、中心厚 = 5 − β1·12.7
    let want = 5.0 + 0.0087268677907590 * 12.7;
    assert!(
        (s.surfaces[1].t - want).abs() < 1e-12,
        "{}",
        s.surfaces[1].t
    );
}

/// OpticStudio が書いた .zmx（構成 1 つでも MNUM・MOFF の行がある）の単位を cm にしても読め、長さが 10 倍の系になる
/// （以前は多重構成のオペランド MOFF で止まった）
#[test]
fn real_zmx_in_cm() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let txt = lensbench_core::glass::read_text(&root.join("tests/refs/A0/system.zmx")).unwrap();
    assert!(txt.contains("MOFF"));
    let opt = zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let load =
        |t: &str| zmx::build(&zmx::parse_text(t).unwrap(), &opt, &opt.catalog_dirs, None).unwrap();
    let mm = load(&txt);
    let cm = load(&txt.replacen("UNIT MM", "UNIT CM", 1));
    let (a, b) = (mm.paraxial().efl, cm.paraxial().efl);
    assert!((b / a - 10.0).abs() < 1e-9, "{a} {b}");
}

fn a0_text() -> (String, zmx::LoadOptions) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let txt = lensbench_core::glass::read_text(&root.join("tests/refs/A0/system.zmx"))
        .unwrap()
        .replace("\r\n", "\n");
    let opt = zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    (txt, opt)
}

/// UTF-8・Shift_JIS の .zmx の日本語の名前・面のコメントが化けずに読め、書いて読み直しても同じ
#[test]
fn japanese_names_roundtrip() {
    let (txt, opt) = a0_text();
    let txt = txt.replacen("NAME ", "NAME 試験レンズ", 1).replacen(
        "SURF 1\n",
        "SURF 1\n  COMM 前玉\n",
        1,
    );
    let (sj, _, _) = encoding_rs::SHIFT_JIS.encode(&txt);
    for bytes in [txt.as_bytes().to_vec(), sj.into_owned()] {
        let t = lensbench_core::glass::decode_text(&bytes);
        let s = zmx::build(&zmx::parse_text(&t).unwrap(), &opt, &opt.catalog_dirs, None).unwrap();
        assert_eq!(s.name, "試験レンズ");
        assert_eq!(s.surfaces[1].comment, "前玉");
        let w = zmx_write::write(&s).unwrap();
        let s2 = zmx::build(&zmx::parse_text(&w).unwrap(), &opt, &opt.catalog_dirs, None).unwrap();
        assert_eq!(
            (s2.name.as_str(), s2.surfaces[1].comment.as_str()),
            ("試験レンズ", "前玉")
        );
    }
}

/// 読まない系の行は注意に残し、瞳のアポダイゼーションは止める（カタログの玉としてなら注意）。多重構成の読めない行は誤り
#[test]
fn unread_lines_are_reported() {
    let (txt, opt) = a0_text();
    let load =
        |t: &str| zmx::parse_text(t).and_then(|r| zmx::build(&r, &opt, &opt.catalog_dirs, None));
    let s = load(&txt).unwrap();
    assert!(s.import_notes.is_empty(), "{:?}", s.import_notes);
    let s = load(&txt.replacen("SDMA 0 1 0", "SDMA 1 1 0", 1).replacen(
        "HYPR 0",
        "HYPR 0\nZZZZ 3 4",
        1,
    ))
    .unwrap();
    assert_eq!(s.import_notes.len(), 2, "{:?}", s.import_notes);
    assert!(load(&txt.replacen("GFAC 0 0", "GFAC 1 3", 1)).is_err());
    // カタログの玉として読むとき（fixed_semi）は注意だけ
    let cat = zmx::LoadOptions {
        fixed_semi: true,
        ..opt.clone()
    };
    let s = zmx::parse_text(&txt.replacen("GFAC 0 0", "GFAC 1 1", 1))
        .and_then(|r| zmx::build(&r, &cat, &cat.catalog_dirs, None))
        .unwrap();
    assert_eq!(s.import_notes.len(), 1, "{:?}", s.import_notes);
    assert!(s.import_notes[0].contains("GFAC"));
    assert!(load(&format!("{txt}\nTHIC 2 x broken\n")).is_err());
    // 数として読めない系の値は既定値にせず誤り
    assert!(load(&txt.replacen("ENPD ", "ENPD abc ", 1)).is_err());
}

/// 多重構成の .zmx は多重構成の表として読み、どの構成も構成を選んで読んだ系と同じ。書いて読み直しても表は同じ
#[test]
fn multi_config_table_roundtrip() {
    let (txt, opt) = a0_text();
    let mce = "MNUM 2 1\nMOFF   0   1 \"\" 0 0 0 1 1 0 0.0 \"\" 0\n\
               THIC   4   1 2.3 0 0 0 1 1 1 0 0 \"\" 0\n\
               THIC   4   2 2.8 0 0 0 1 1 1 0 0 \"\" 0\n\
               THIC   2   1 5.8 0 0 0 1 1 1 0 0 \"\" 0\n\
               THIC   2   2 6.3 0 0 0 1 1 1 0 0 \"\" 0\n";
    let i = txt.find("MNUM 1 1").unwrap();
    let txt = format!("{}{mce}", &txt[..i]);
    let load = |t: &str, cfg: usize| {
        let o = zmx::LoadOptions { cfg, ..opt.clone() };
        zmx::build(&zmx::parse_text(t).unwrap(), &o, &o.catalog_dirs, None).unwrap()
    };
    let s = load(&txt, 1);
    let c = s.configs.as_ref().expect("多重構成の表");
    assert_eq!((c.n, c.active, c.rows.len()), (2, 1, 2), "{c:?}");
    assert!(s.import_notes.is_empty(), "{:?}", s.import_notes);
    for k in 1..=2 {
        let (a, b) = (s.config(k).unwrap(), load(&txt, k));
        for j in 0..a.surfaces.len() {
            assert_eq!(a.surfaces[j].t, b.surfaces[j].t, "構成 {k} 面 {j}");
        }
        assert!((a.paraxial().efl - b.paraxial().efl).abs() < 1e-12);
    }
    let out = zmx_write::write(&s).unwrap();
    assert!(out.contains("MNUM 2 1"), "{out}");
    let back = load(&out, 1);
    assert_eq!(back.configs.as_ref().unwrap().rows, c.rows);
}

/// .zmx に書けないもの（曲率半径のピックアップ等）は注意として返し、書けるものだけの系では空
#[test]
fn write_notes_list_dropped_items() {
    use lensbench_core::param::{PickKind, Pickup};
    let (txt, opt) = a0_text();
    let mut s = zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &opt,
        &opt.catalog_dirs,
        None,
    )
    .unwrap();
    assert!(zmx_write::write_notes(&s).is_empty());
    s.pickups
        .push(Pickup::new(3, 1, PickKind::Radius, 1.0, 0.0));
    let notes = zmx_write::write_notes(&s);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("曲率半径") && n.contains("面 3")),
        "{notes:?}"
    );
}
