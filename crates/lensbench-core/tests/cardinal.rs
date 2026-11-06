//! 基点（report::cardinal）を厚肉レンズ・像空間がガラス・鏡の式と比べる

use lensbench_core::report::cardinal;
use lensbench_core::system::System;
use lensbench_core::zmx;

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
YFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

fn sys(body: &str) -> System {
    let raw = zmx::parse_text(&format!("{HEAD}{body}")).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn close(a: f64, b: f64, what: &str) {
    assert!(
        (a - b).abs() < 1e-10 * (1.0 + b.abs()),
        "{what}: {a} != {b}"
    );
}

/// 空気中の厚肉レンズ（R1・R2・厚さ d・屈折率 n）の式
#[test]
fn thick_lens() {
    let (r1, r2, d, n, t_img): (f64, f64, f64, f64, f64) = (40.0, -60.0, 6.0, 1.6, 30.0);
    let s = sys(&format!(
        "SURF 1\n  STOP\n  TYPE STANDARD\n  CURV {}\n  DISZ {d}\n  GLAS ___BLANK 1 0 {n} 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV {}\n  DISZ {t_img}\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        1.0 / r1,
        1.0 / r2
    ));
    let c = cardinal(&s, 0);
    let phi = (n - 1.0) * (1.0 / r1 - 1.0 / r2) + (n - 1.0).powi(2) * d / (n * r1 * r2);
    let f = 1.0 / phi;
    close(c.efl, f, "efl");
    close(c.efl, s.paraxial().efl, "EFFL と同じ");
    close(c.f_image, f, "f′");
    close(c.f_object, -f, "f");
    // 主点: 後側は最後の頂点から −f(n−1)d/(nR1)、前側は最初の頂点から −f(n−1)d/(nR2)
    let p2 = -f * (n - 1.0) * d / (n * r1);
    let p1 = -f * (n - 1.0) * d / (n * r2);
    close(c.bfl, f + p2, "BFL");
    close(c.ffl, p1 - f, "FFL");
    close(c.principal_image, p2 - t_img, "P′（像面から）");
    close(c.principal_object, p1, "P（面 1 から）");
    close(c.focal_image, f + p2 - t_img, "F′");
    close(c.focal_object, p1 - f, "F");
    // 空気中は節点 = 主点、反主点は焦点について主点の反対側（主点から 2f）
    close(c.nodal_image, c.principal_image, "N′");
    close(c.nodal_object, c.principal_object, "N");
    close(c.anti_principal_image, p2 + 2.0 * f - t_img, "反主点′");
    close(c.anti_principal_object, p1 - 2.0 * f, "反主点");
}

/// 像空間がガラス（単一の屈折面 R、n → n′）: f′ = n′R/(n′−n)、f = −nR/(n′−n)、主点は頂点、節点は曲率中心
#[test]
fn single_surface_into_glass() {
    let (r, n1, t_img) = (25.0, 1.5, 80.0);
    let s = sys(&format!(
        "SURF 1\n  STOP\n  TYPE STANDARD\n  CURV {}\n  DISZ {t_img}\n  GLAS ___BLANK 1 0 {n1} 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        1.0 / r
    ));
    let c = cardinal(&s, 0);
    close(c.n_image, n1, "n′");
    close(c.f_image, n1 * r / (n1 - 1.0), "f′");
    close(c.f_object, -r / (n1 - 1.0), "f");
    close(c.efl, r / (n1 - 1.0), "efl（空気換算）");
    close(c.principal_object, 0.0, "P");
    close(c.principal_image, -t_img, "P′");
    close(c.nodal_object, r, "N = 曲率中心");
    close(c.nodal_image, r - t_img, "N′ = 曲率中心");
    close(c.focal_image, n1 * r / (n1 - 1.0) - t_img, "F′");
}

/// 凹面鏡（R = −100、像面は鏡の後 −50）: 焦点距離 50、像空間の屈折率は −1 で f′ = −50、位置は局所 z（OpticStudio と同じ）
#[test]
fn concave_mirror() {
    let s = sys(
        "SURF 1\n  STOP\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -50\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
    );
    let c = cardinal(&s, 0);
    close(c.efl, 50.0, "efl");
    close(c.n_image, -1.0, "n′");
    close(c.f_image, -50.0, "f′");
    close(c.bfl, -50.0, "BFL");
    close(c.focal_image, 0.0, "F′ は像面");
    close(c.principal_image, 50.0, "P′ は鏡");
    // 節点は主点から (n′ − n)/φ = −100: 曲率中心
    close(c.nodal_object, -100.0, "N は曲率中心");
}

/// 無焦点（ケプラー型、f 50 と 25 の薄肉近軸レンズを 75 離す）
#[test]
fn afocal() {
    let s = sys("SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  DISZ 75\n\
SURF 2\n  TYPE PARAXIAL\n  PARM 1 25\n  DISZ 10\nSURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n");
    let c = cardinal(&s, 0);
    assert!(c.efl.is_infinite() && c.focal_image.is_infinite());
}

/// OpticStudio の Cardinal Points（tests/refs/*/cardinal.json、表は 6 桁）と主波長で比べる。主波長以外の値は
/// 測るときに主波長を替えて求めたもので、ソルブがその波長で解き直されているので使わない
#[test]
fn opticstudio() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let mut n = 0;
    for e in std::fs::read_dir(root.join("refs")).unwrap() {
        let d = e.unwrap().path();
        let Ok(t) = std::fs::read_to_string(d.join("cardinal.json")) else {
            continue;
        };
        let j: serde_json::Value = serde_json::from_str(&t).unwrap();
        let opt = zmx::LoadOptions {
            catalog_dirs: vec![root.join("glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&d.join("system.zmx"), &opt).unwrap();
        let r = &j["waves"][s.primary];
        let c = cardinal(&s, s.primary);
        let pair = |k: &str| [r[k][0].as_f64().unwrap(), r[k][1].as_f64().unwrap()];
        for (k, a, b) in [
            ("focal_length", c.f_object, c.f_image),
            ("focal", c.focal_object, c.focal_image),
            ("principal", c.principal_object, c.principal_image),
            (
                "anti_principal",
                c.anti_principal_object,
                c.anti_principal_image,
            ),
            ("nodal", c.nodal_object, c.nodal_image),
            ("anti_nodal", c.anti_nodal_object, c.anti_nodal_image),
        ] {
            let [ra, rb] = pair(k);
            for (x, v) in [(a, ra), (b, rb)] {
                assert!(
                    (x - v).abs() <= 1e-6 * v.abs().max(1.0),
                    "{}: {k} {x} != {v}",
                    d.display()
                );
            }
        }
        n += 1;
    }
    assert!(n >= 28, "{n}");
}
