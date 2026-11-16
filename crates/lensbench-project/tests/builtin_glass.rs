//! 同梱のガラスカタログ（SCHOTT・OHARA、CLI-31）で開いた系の、系ファイル（v2）の往復。ガラスのフォルダを渡さないで
//! .zmx を読み、保存では同梱のデータの hash を書き、読むときはフォルダに無いので同梱のものを使って注意を返す。
//! このプロセスで tests/glass の AGF を読むと、それが候補になるので、ほかの結合テストとは別のプロセスにしてある。

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_project::sysfile;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn builtin_catalog_roundtrip() {
    let s = zmx::load_with(
        &root().join("tests/refs/A0/system.zmx"),
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        s.import_notes
            .iter()
            .any(|n| n.contains("built-in catalog")),
        "{:?}",
        s.import_notes
    );
    // 同梱のデータと値が同じなので、保存の注意は無く hash を書く
    assert!(sysfile::save_notes(&s).is_empty());
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let cats = v["system"]["catalogs"].as_array().unwrap();
    let schott = cats.iter().find(|c| c["name"] == "SCHOTT").unwrap();
    let body = std::fs::read(root().join("crates/lensbench-core/assets/glass/SCHOTT.AGF")).unwrap();
    let hex: String = Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(schott["hash"], format!("sha256:{hex}"));
    // 読み直し: フォルダに無いので同梱のものを使い、注意を返す。ガラスは同じ
    let (t, _, warn) = sysfile::from_str(&txt).unwrap();
    assert!(
        warn.iter()
            .any(|w| w.contains("SCHOTT") && w.contains("built-in catalog is used")),
        "{warn:?}"
    );
    assert!(!warn.iter().any(|w| w.contains("differs")), "{warn:?}");
    for d in &s.surfaces {
        if let Some(g) = s.glasses.get(d.mat.trim()) {
            assert_eq!(t.glasses.get(d.mat.trim()), Some(g), "{}", d.mat);
        }
    }
    // hash が違えば（書いた後に同梱のデータが変わった）注意
    let mut v2 = v.clone();
    for c in v2["system"]["catalogs"].as_array_mut().unwrap() {
        c["hash"] = serde_json::json!("sha256:00");
    }
    let (_, _, warn) = sysfile::from_str(&v2.to_string()).unwrap();
    assert!(warn.iter().any(|w| w.contains("differs")), "{warn:?}");
}
