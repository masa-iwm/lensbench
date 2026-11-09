//! serve の tolerance: CLI の tolerance と同じ結果、読み込んだ系（set の後）を使うこと、構成の並び。
//! 系はプロセスに 1 つなので、別のテストの実行ファイルにしている。

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn call(m: &str, p: Value) -> Value {
    lensbench_cli::serve::call(m, &p, None).unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

#[test]
fn tolerance_serve() {
    let zmx = Path::new(ROOT).join("tests/refs/C0/system.zmx");
    let glass = Path::new(ROOT).join("tests/glass");
    call(
        "load",
        json!({ "file": zmx.display().to_string(), "glass_dir": [glass.display().to_string()] }),
    );
    let spec = json!({"trials": 4, "seed": 2, "preset": "precision", "compensators": ["6.t"]});
    let r = call("tolerance", spec.clone());
    assert!(lensbench_cli::serve::tolerance_progress().is_none());
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
    call("set", json!({ "values": { "2.t": 6.0 } }));
    let r2 = call("tolerance", spec.clone());
    assert_ne!(r2["nominal"], r["nominal"]);
    // 構成の並び（.zmx の構成番号。C0 は 1 つだけなので 1 を 2 回 = 同じ系の 2 構成）
    let mut sp = spec.clone();
    sp["configs"] = json!([1, 1]);
    sp["sensitivity"] = json!(false);
    let r3 = call("tolerance", sp);
    assert_eq!(
        r3["terms"].as_array().unwrap().len(),
        2 * r2["terms"].as_array().unwrap().len()
    );
    assert!(lensbench_cli::serve::call("tolerance", &json!({"dist": "foo"}), None).is_err());
}
