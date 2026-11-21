//! 近軸ガウスビーム（gaussian-beam）: 薄いレンズ・平板・鏡での解析解との一致

use lensbench_analysis::gauss::{gauss, Beam};
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use std::f64::consts::PI;
use std::path::PathBuf;

fn sys(text: &str) -> System {
    let raw = zmx::parse_text(text).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 1 1 0 0 0\n\
    XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.6328 1\nPWAV 1\n\
    SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

/// 焦点距離 f の近軸レンズ（面 1）、その後に距離 d の空気と像面
fn thin(f: f64, d: f64) -> System {
    sys(&format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 {f}\n  PARM 2 0\n  DISZ {d}\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    ))
}

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!(
        (a - b).abs() <= tol * b.abs().max(1e-12),
        "{what}: {a} vs {b}"
    );
}

/// 薄いレンズの後のウエスト: 入射のウエストが距離 s の前にあるとき、
/// 1/q′ = 1/q − 1/f（q = s + i z_R）
#[test]
fn thin_lens_matches_the_q_law() {
    let lam = 0.6328e-3;
    for &(w0, s, m2) in &[(0.5, 0.0, 1.0), (0.2, 100.0, 1.0), (0.3, -40.0, 1.5)] {
        let f = 50.0;
        let g = gauss(
            &thin(f, 80.0),
            &Beam {
                wave: 0,
                waist: w0,
                surf1_to_waist: -s,
                m2,
            },
        )
        .unwrap();
        // 与えるウエストは基本モードのもの（M² によらない）
        let zr = PI * w0 * w0 / lam;
        let q = rustfft::num_complex::Complex64::new(s, zr);
        let qp = 1.0 / (1.0 / q - 1.0 / f);
        let r1 = &g.rows[0];
        close(r1.position, qp.re, 1e-10, "waist position after the lens");
        close(r1.rayleigh, qp.im, 1e-10, "Rayleigh range after the lens");
        let w0p = (qp.im * lam / PI).sqrt();
        close(r1.waist, w0p, 1e-10, "waist after the lens");
        close(
            r1.divergence,
            (lam / (PI * w0p)).atan(),
            1e-10,
            "divergence",
        );
        let _ = m2;
        // 像面（80 先）での半径
        let q2 = qp + 80.0;
        let w2 = w0p * (1.0 + (q2.re / q2.im).powi(2)).sqrt();
        close(g.rows[1].size, w2, 1e-10, "radius at the image");
        // 輪郭の終わりは像面の半径
        let last = g.profile.last().unwrap().last().unwrap();
        close(last[0], 80.0, 1e-12, "profile end");
        close(last[1], w2, 1e-10, "profile end radius");
    }
}

/// 屈折率 n の平板の中: ウエストの半径は同じ、レイリー長は n 倍、広がり角は 1/n
#[test]
fn plate_scales_rayleigh_with_index() {
    let n = 1.5;
    let s = sys(&format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n  GLAS ___BLANK 1 0 {n} 0 0 0 0 0 0\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
         SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    ));
    let g = gauss(
        &s,
        &Beam {
            wave: 0,
            waist: 0.1,
            surf1_to_waist: 5.0,
            m2: 1.0,
        },
    )
    .unwrap();
    let (air, glass) = (&g.rows[1], &g.rows[0]);
    close(glass.waist, air.waist, 1e-12, "waist");
    close(glass.rayleigh, n * air.rayleigh, 1e-12, "Rayleigh range");
    close(
        glass.divergence.tan(),
        air.divergence.tan() / n,
        1e-12,
        "divergence",
    );
    // 面 1 の後: ウエストは 5 × n 先（ガラスの中の実の距離。位置はウエストから面へ測るので負）
    close(
        glass.position,
        -5.0 * n,
        1e-12,
        "waist position in the glass",
    );
    // 面 2 の後: ガラスの中を 10 進んだので、空気の換算で 10 / n − 5
    close(
        air.position,
        (10.0 - 5.0 * n) / n,
        1e-12,
        "waist position after the plate",
    );
}

/// 凹面鏡（曲率半径 R、f = R/2）の後も、展開した向きで薄いレンズと同じ（位置の符号は局所 z で逆）
#[test]
fn mirror_is_a_thin_lens_unfolded() {
    let s = sys(&format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -60\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    ));
    let b = Beam {
        wave: 0,
        waist: 0.4,
        surf1_to_waist: -30.0,
        m2: 1.0,
    };
    let gm = gauss(&s, &b).unwrap();
    let gl = gauss(&thin(50.0, 60.0), &b).unwrap();
    for (a, l) in gm.rows.iter().zip(&gl.rows) {
        // 反射の後は局所 z が逆なので、位置の符号が逆
        close(a.position, -l.position, 1e-10, "position");
        close(a.size, l.size, 1e-10, "size");
        close(a.z, l.z, 1e-12, "z");
    }
}

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

/// OpticStudio 26.1 の Paraxial Gaussian Beam（tests/refs/<系>/gauss_os.json、表示の 6 桁）と合う。
/// 基本モードと M² の混合モード、ウエストが面 1 の前・上・後、2 つの波長、鏡（C0）・近軸レンズ（D0）・非球面（E0）
#[test]
fn matches_opticstudio_paraxial_gaussian_beam() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["A0", "C0", "D0", "E0"] {
        let s = load(name);
        let txt =
            std::fs::read_to_string(root.join(format!("tests/refs/{name}/gauss_os.json"))).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&txt).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let f = |k: &str| case[k].as_f64().unwrap();
            let m2 = f("m2");
            let g = gauss(
                &s,
                &Beam {
                    wave: case["wave"].as_u64().unwrap() as usize - 1,
                    waist: f("waist"),
                    surf1_to_waist: f("surf1_to_waist"),
                    m2,
                },
            )
            .unwrap();
            for (key, scale) in [("fundamental", 1.0), ("mixed", m2.sqrt())] {
                let Some(rows) = case[key].as_array() else {
                    continue;
                };
                assert_eq!(rows.len(), g.rows.len(), "{name} {key}: number of surfaces");
                for (r, b) in rows.iter().zip(&g.rows) {
                    // 曲率半径の null はウエストの上（無限大）
                    let o: Vec<f64> = r
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_f64().unwrap_or(f64::INFINITY))
                        .collect();
                    let ours = [
                        b.size * scale,
                        b.waist * scale,
                        b.position,
                        b.radius,
                        (scale * b.waist / b.rayleigh).atan(),
                        b.rayleigh,
                    ];
                    for (k, (&a, &e)) in ours.iter().zip(&o).enumerate() {
                        // 表示は 6 桁（相対 5e-6）。0 に近い位置は桁落ちするので長さの絶対 1e-6 も許す
                        let tol = 6e-6 * e.abs() + 1e-6;
                        assert!(
                            a == e || (a - e).abs() <= tol,
                            "{name} {key} surface {} column {}: {a} vs OpticStudio {e} (case {case})",
                            b.surface,
                            doc["columns"][k]
                        );
                    }
                }
            }
        }
    }
}
