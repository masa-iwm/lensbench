//! serve の tolerance: CLI の tolerance と同じ結果、読み込んだ系（set の後）を使うこと、構成の並び。
//! 系はプロセスに 1 つなので、別のテストの実行ファイルにしている。

use lensbench_engine::Session;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn call(sv: &Session, m: &str, p: Value) -> Value {
    sv.call(m, &p, None)
        .unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

#[test]
fn tolerance_serve() {
    let sv = Session::new();
    let zmx = Path::new(ROOT).join("tests/refs/C0/system.zmx");
    let glass = Path::new(ROOT).join("tests/glass");
    call(
        &sv,
        "load",
        json!({ "file": zmx.display().to_string(), "glass_dirs": [glass.display().to_string()] }),
    );
    let spec = json!({"trials": 4, "seed": 2, "preset": "precision", "compensators": ["6.t"]});
    let r = call(&sv, "tolerance", spec.clone());
    assert!(sv.tolerance_progress().is_none());
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .args([
            "tolerance",
            zmx.to_str().unwrap(),
            "--glass-dir",
            glass.to_str().unwrap(),
            "--json",
        ])
        .args(["--set", &spec.to_string()])
        .output()
        .unwrap();
    assert!(o.status.success());
    let c: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(r["cdf"], c["cdf"]);
    assert_eq!(r["sensitivity"], c["sensitivity"]);
    // set の後の系を使う
    call(&sv, "set", json!({ "values": { "2.t": 6.0 } }));
    let r2 = call(&sv, "tolerance", spec.clone());
    assert_ne!(r2["nominal"], r["nominal"]);
    // 構成の並び（.zmx の構成番号。C0 は 1 つだけなので 1 を 2 回 = 同じ系の 2 構成）
    let mut sp = spec.clone();
    sp["configs"] = json!([1, 1]);
    sp["sensitivity"] = json!(false);
    let r3 = call(&sv, "tolerance", sp);
    assert_eq!(
        r3["terms"].as_array().unwrap().len(),
        2 * r2["terms"].as_array().unwrap().len()
    );
    assert!(lensbench_engine::call("tolerance", &json!({"dist": "foo"}), None).is_err());
}

/// 進み具合・停止・中断は、処理している要求を待たずに別のスレッド（TCP の別の接続）から答える
#[test]
fn progress_and_stop_while_running() {
    let sv = Session::new();
    let zmx = Path::new(ROOT).join("tests/refs/C0/system.zmx");
    let glass = Path::new(ROOT).join("tests/glass");
    call(
        &sv,
        "load",
        json!({ "file": zmx.display().to_string(), "glass_dirs": [glass.display().to_string()] }),
    );
    // 何もしていない
    let p = call(&sv, "progress", json!({}));
    assert_eq!(p["running"], false);
    assert_eq!(p["tolerance"], Value::Null);
    assert_eq!(call(&sv, "cancel", json!({})), false);
    assert!(sv.call("stop", &json!({"what": "foo"}), None).is_err());
    let spec = json!({"trials": 100000, "seed": 2, "preset": "precision", "sensitivity": false});
    std::thread::scope(|s| {
        let h = s.spawn(|| call(&sv, "tolerance", spec.clone()));
        let t0 = std::time::Instant::now();
        loop {
            let p = call(&sv, "progress", json!({}));
            if p["tolerance"].is_array() {
                assert_eq!(p["running"], true);
                break;
            }
            assert!(t0.elapsed().as_secs() < 60, "進み具合が出ない");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(call(&sv, "stop", json!({"what": "tolerance"})), true);
        let r = h.join().unwrap();
        assert_eq!(r["cancelled"], true);
    });
    assert_eq!(call(&sv, "progress", json!({}))["running"], false);
}
