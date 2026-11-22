//! 熱の構成（lensbench_core::thermal）: OpticStudio 26.1 の Make Thermal の表の値と、各構成の屈折率・焦点距離が合う

use lensbench_core::param::{Kind, Param};
use lensbench_core::thermal::{make_thermal, ThermalSpec};
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(name: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

fn kind(op: &str, parm: u8) -> Option<Kind> {
    Some(match op {
        "TEMP" => Kind::Temp,
        "PRES" => Kind::Pres,
        "CRVT" => Kind::Curv,
        "THIC" => Kind::Thick,
        "PRAM" => Kind::Asph(parm - 1),
        "SDIA" => Kind::Semi,
        "MCSD" => Kind::MechSemi,
        "CHZN" => Kind::ChipZone,
        _ => return None,
    })
}

#[test]
fn matches_opticstudio_make_thermal() {
    for name in ["A0", "E0"] {
        let txt =
            std::fs::read_to_string(root().join(format!("tests/refs/{name}/thermal_os.json")))
                .unwrap();
        let doc: serde_json::Value = serde_json::from_str(&txt).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let mut s = load(name);
            // 測ったときと同じく、面 1 から像面の前までの空気の面に TCE を与える
            if let Some(a) = case["air_tce"].as_f64() {
                for j in 1..s.image() {
                    if s.surfaces[j].mat.is_air() {
                        s.set_param(Param::new(j, Kind::Tce), a).unwrap();
                    }
                }
            }
            let nums = |k: &str| -> Vec<f64> {
                case[k]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect()
            };
            let spec = ThermalSpec {
                temps: nums("temps"),
                pressures: nums("pressures"),
            };
            let out = make_thermal(&s, 1, &spec).unwrap();
            let t = &out.system;
            let cf = t.configs().unwrap();
            let what = format!("{name} air_tce {}", case["air_tce"]);
            assert_eq!(cf.n, 4, "{what}");
            // OpticStudio の行はどれも lensbench の表にあり、値が合う
            for r in case["rows"].as_array().unwrap() {
                let op = r["op"].as_str().unwrap();
                let parm = r["parm"].as_u64().unwrap() as u8;
                let k = kind(op, parm).unwrap();
                let p = if k.is_global() {
                    Param::system(k)
                } else {
                    Param::new(r["surface"].as_u64().unwrap() as usize, k)
                };
                let expect: Vec<f64> = r["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect();
                let row = cf.rows.iter().find(|x| x.param == p);
                // 平らな面の曲率 0・値 0 の行は無くてもよい
                let Some(row) = row else {
                    assert!(
                        expect.iter().all(|v| *v == 0.0),
                        "{what}: no row {op} {}",
                        r["surface"]
                    );
                    continue;
                };
                for (k, (&a, &e)) in row.values.iter().zip(&expect).enumerate() {
                    assert!(
                        (a - e).abs() <= 1e-12 * e.abs().max(1.0),
                        "{what}: {op} surface {} parm {parm} config {}: {a} vs OpticStudio {e}",
                        r["surface"],
                        k + 1
                    );
                }
            }
            // 各構成の屈折率（面の後、全波長）と焦点距離（測った EFFL は波長 1）
            for (k, c) in case["configs"].as_array().unwrap().iter().enumerate() {
                let sk = t.config(k + 1).unwrap();
                let (_, u) = sk.ptrace(1.0, 0.0, 0, None);
                let efl = -1.0 / u[sk.image() - 1];
                for (j, ns) in c["index"].as_object().unwrap() {
                    let j: usize = j.parse().unwrap();
                    for (w, n) in ns.as_array().unwrap().iter().enumerate() {
                        let n = n.as_f64().unwrap();
                        let ours = sk.n(j)[w];
                        assert!(
                            (ours - n).abs() <= 1e-9,
                            "{what} config {} surface {j} wave {}: n {ours} vs {n}",
                            k + 1,
                            w + 1
                        );
                    }
                }
                let e = c["effl"].as_f64().unwrap();
                assert!(
                    (efl - e).abs() <= 1e-9 * e.abs(),
                    "{what} config {}: EFFL {efl} vs {e}",
                    k + 1
                );
            }
        }
    }
}

/// 面タイプのパラメータ（OpticStudio 2026 R1.01 の Make Thermal の規則）: Odd Asphere の r^k の係数は s^(1−k)、Irregular の偏心と
/// Zs・Za・Zc は s 倍で傾き・角度はそのまま、Tilted の tan はそのまま（s = 1 + α ΔT。形だけを見るので面は鏡にして TCE を与える）
#[test]
fn surface_type_parameters_follow_opticstudio() {
    let head = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\nGCAT SCHOTT\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";
    let sys = |surf1: &str| {
        let txt = format!(
            "{head}SURF 1\n  STOP\n{surf1}  DISZ -40\n  GLAS MIRROR 0 0 1.5 4.0E+1 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        let raw = zmx::parse_text(&txt).unwrap();
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        };
        let mut s = zmx::build(&raw, &opt, &[], None).unwrap();
        // 鏡はレンズデータの TCE（N-BK7 の 7.1e-6 と同じ）
        s.set_param(Param::new(1, Kind::Tce), 7.1).unwrap();
        s
    };
    let spec = ThermalSpec {
        temps: vec![-30.0, 70.0],
        pressures: vec![],
    };
    let check = |s: &System, want: &[(u8, Option<f64>)]| {
        let out = make_thermal(s, 1, &spec).unwrap();
        let cf = out.system.configs().unwrap();
        let a = s.temperature;
        let tce = 7.1e-6;
        for &(i, e) in want {
            let p = Param::new(1, Kind::Asph(i));
            let row = cf.rows.iter().find(|r| r.param == p);
            match e {
                None => assert!(row.is_none(), "parm {}", i + 1),
                Some(e) => {
                    let row = row.unwrap_or_else(|| panic!("no row of parm {}", i + 1));
                    let v = row.values[0];
                    for (k, t) in [-30.0, 70.0].iter().enumerate() {
                        let g: f64 = 1.0 + tce * (t - a);
                        let want = v * g.powf(e);
                        assert!(
                            (row.values[k + 1] - want).abs() <= 1e-12 * want.abs(),
                            "parm {} at {t}: {} vs {want}",
                            i + 1,
                            row.values[k + 1]
                        );
                    }
                }
            }
        }
        out.warnings
    };
    let odd = sys("  TYPE ODDASPHE\n  CURV 0.02\n  CONI -0.5\n  PARM 1 1e-3\n  PARM 2 1e-4\n  PARM 3 1e-5\n  PARM 4 1e-6\n  PARM 8 1e-10\n");
    let w = check(
        &odd,
        &[
            (0, Some(0.0)),
            (1, Some(-1.0)),
            (2, Some(-2.0)),
            (7, Some(-7.0)),
        ],
    );
    assert!(w.iter().all(|x| !x.contains("not scaled")), "{w:?}");
    let irr = sys("  TYPE IRREGULA\n  CURV 0.02\n  PARM 1 0.1\n  PARM 2 -0.2\n  PARM 3 1\n  PARM 4 2\n  PARM 5 0.01\n  PARM 6 0.02\n  PARM 7 0.03\n  PARM 8 30\n");
    check(
        &irr,
        &[
            (0, Some(1.0)),
            (1, Some(1.0)),
            (2, None),
            (3, None),
            (4, Some(1.0)),
            (5, Some(1.0)),
            (6, Some(1.0)),
            (7, None),
        ],
    );
    let tilt = sys("  TYPE TILTSURF\n  PARM 1 0.01\n  PARM 2 0.02\n");
    check(&tilt, &[(0, None), (1, None)]);
}
