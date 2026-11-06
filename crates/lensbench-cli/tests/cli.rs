use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(root)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const ZMX: &str = "tests/refs/A0/system.zmx";

#[test]
fn info_json() {
    let (ok, out, _) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--json"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let effl = v["effl"].as_f64().unwrap();
    // OpticStudio の EFFL（基準の ops.json）と一致する
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let ops: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{root}/tests/refs/A0/ops.json")).unwrap(),
    )
    .unwrap();
    let os = ops["ops"]["EFFL"].as_f64().unwrap();
    assert!((effl - os).abs() < 1e-9, "{effl} {os}");
}

#[test]
fn spot_and_zernike() {
    let (ok, out, _) = run(&[
        "spot",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--field",
        "2",
        "--json",
    ]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["spot"].as_array().unwrap().len(), 1);
    let (ok, out, _) = run(&[
        "zernike",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--terms",
        "16",
        "--json",
    ]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["zernike"].as_array().unwrap().len(), 4);
    assert_eq!(v["zernike"][0]["coef"].as_array().unwrap().len(), 16);
}

#[test]
fn errors() {
    let (ok, _, err) = run(&["spot", ZMX]);
    assert!(!ok && err.contains("N-SK16"), "{err}");
    let (ok, _, err) = run(&["spot", ZMX, "--glass-dir", "tests/glass", "--field", "9"]);
    assert!(!ok && err.contains("視野 9"), "{err}");
}

#[test]
fn cfg_out_of_range() {
    let (ok, _, err) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--cfg", "2"]);
    assert!(!ok && err.contains("構成 2"), "{err}");
}

#[test]
fn small_grid_is_an_error() {
    for cmd in ["zernike", "wavefront"] {
        let (ok, _, err) = run(&[cmd, ZMX, "--glass-dir", "tests/glass", "--n", "3"]);
        assert!(!ok && err.contains("4 以上の偶数"), "{cmd}: {err}");
    }
}

/// 非対称系（座標ブレーク・傾き）の視野にケラレ係数があっても、射出瞳の算出で落ちない
#[test]
fn vignetting_factors_in_a_tilted_system() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let raw = std::fs::read(format!("{root}/tests/refs/C0/system.zmx")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    assert!(txt.contains("VDYN 0 0 0 0"));
    let txt = txt.replacen("VDYN 0 0 0 0", "VDYN 0 0.1 0 0", 1);
    let dir = std::env::temp_dir().join(format!("lensbench-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("c0_vig.zmx");
    std::fs::write(&p, txt).unwrap();
    let glass = format!("{root}/tests/glass");
    let (ok, out, err) = run(&[
        "wavefront",
        p.to_str().unwrap(),
        "--glass-dir",
        &glass,
        "--json",
    ]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["wavefront"][0]["rms"].as_f64().unwrap().is_finite());
}

/// 無限遠物体で面 1 が無視（IGNR）でも追跡できる（光路長の基準は次の有効な面）
#[test]
fn first_surface_ignored() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let raw = std::fs::read(format!("{root}/tests/refs/A0/system.zmx")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    assert!(txt.contains("SURF 1\r\n"));
    let txt = txt.replacen("SURF 1\r\n", "SURF 1\r\n  IGNR 1\r\n", 1);
    let dir = std::env::temp_dir().join(format!("lensbench-cli-ignr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a0_ignr1.zmx");
    std::fs::write(&p, txt).unwrap();
    let glass = format!("{root}/tests/glass");
    let (ok, out, err) = run(&["spot", p.to_str().unwrap(), "--glass-dir", &glass, "--json"]);
    let (ok2, _, err2) = run(&["wavefront", p.to_str().unwrap(), "--glass-dir", &glass]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    assert!(ok2, "{err2}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["spot"][0]["rms_centroid"].as_f64().unwrap().is_finite());
}
