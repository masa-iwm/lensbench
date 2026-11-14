//! 公開側の例題（tests/verify）を同梱のプラグインで照合する

use std::path::Path;

#[test]
fn public_samples_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verify");
    lensbench_verify::assert_dir(&dir, &lensbench_design::plugins::builtin());
}

/// 許容差を締めると落ち、表に差が出る
#[test]
fn tight_tolerance_fails_with_report() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verify");
    let p = dir.join("a0_zernike.verify.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    v["checks"]["coef"] = serde_json::json!({"abs": 1e-15});
    let m = lensbench_verify::Manifest::from_json(&v, &p).unwrap();
    let r = m.run(&lensbench_design::plugins::builtin());
    assert!(!r.passed());
    let t = r.to_text();
    assert!(t.starts_with("FAIL") && t.contains("coef"), "{t}");
    // スレッド数ではビット同じ
    assert!(r.runs.iter().all(|x| x.threads_identical == Some(true)));
}
