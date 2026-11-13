//! 壊れた .zmx で panic・メモリ確保の暴走をしない（数の欄を 1 つずつ壊して読む。誤りを返すのはよい）

use lensbench_core::zmx::{self, LoadOptions};
use std::path::PathBuf;

#[test]
fn broken_numbers_do_not_panic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let txt = lensbench_core::glass::read_text(&root.join("tests/refs/A0/system.zmx")).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let lines: Vec<&str> = txt.lines().collect();
    let mut cases = 0;
    let mut panics = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for k in 1..toks.len() {
            if toks[k].parse::<f64>().is_err() {
                continue;
            }
            for bad in ["-1", "1e9", "1e300", "abc", "NaN", "2.5"] {
                let mut t2 = toks.clone();
                t2[k] = bad;
                let indent = if line.starts_with("  ") { "  " } else { "" };
                let mut l2 = lines.clone();
                let new = format!("{indent}{}", t2.join(" "));
                l2[i] = &new;
                let src = l2.join("\n");
                cases += 1;
                let r = std::panic::catch_unwind(|| {
                    if let Ok(raw) = zmx::parse_text(&src) {
                        let _ = zmx::build(&raw, &opt, &opt.catalog_dirs, None);
                    }
                });
                if r.is_err() {
                    panics.push(new.clone());
                }
            }
        }
    }
    assert!(cases > 200, "{cases}");
    assert!(
        panics.is_empty(),
        "panic した行（{} / {cases}）:\n{}",
        panics.len(),
        panics.join("\n")
    );
}
