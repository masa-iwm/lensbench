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
