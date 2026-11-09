//! .zmx の書き出し → 読み戻しで、面の値と状態（半直径の固定・自動、開口、無視、ピックアップ・ソルブ）が保たれる。
//! 系は tests/refs の OpticStudio が保存した .zmx

use std::path::PathBuf;

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{glass, zmx_write, System};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn opt() -> LoadOptions {
    LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    }
}

fn reread(s: &System) -> System {
    let txt = zmx_write::write(s).unwrap();
    let raw = zmx::parse_text(&txt).unwrap();
    assert!(!raw.legacy_lensbench());
    zmx::build(
        &raw,
        &opt(),
        &glass::catalog_dirs(&[root().join("tests/glass")]),
        None,
    )
    .unwrap()
}

#[test]
fn refs_keep_values_and_states() {
    let mut n = 0;
    for e in std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .flatten()
    {
        let p = e.path().join("system.zmx");
        if !p.exists() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(s) = zmx::load_with(&p, &opt()) else {
            continue;
        };
        let Ok(txt) = zmx_write::write(&s) else {
            continue;
        };
        assert!(txt.contains("FTYP "), "{name}");
        let r = reread(&s);
        assert_eq!(s.surfaces.len(), r.surfaces.len(), "{name}");
        for (k, (a, b)) in s.surfaces.iter().zip(&r.surfaces).enumerate() {
            let at = format!("{name} 面 {k}");
            assert_eq!(a.typ, b.typ, "{at}");
            assert_eq!(a.semi_auto, b.semi_auto, "{at}: 半直径の自動・固定");
            if !a.semi_auto {
                assert_eq!(a.semi, b.semi, "{at}: 固定の半直径");
            } else {
                assert!((a.semi - b.semi).abs() <= 1e-9 * (1.0 + a.semi), "{at}");
            }
            assert_eq!(a.ap, b.ap, "{at}: 開口");
            assert_eq!(a.ignore, b.ignore, "{at}: 無視");
            assert_eq!(a.stop, b.stop, "{at}");
            assert_eq!(a.coat, b.coat, "{at}");
            assert_eq!(a.mirror, b.mirror, "{at}");
            assert_eq!(a.c, b.c, "{at}");
            assert_eq!(a.k, b.k, "{at}");
            assert!(
                (a.t - b.t).abs() <= 1e-12 * (1.0 + a.t.abs()) || a.t == b.t,
                "{at}"
            );
        }
        assert_eq!(s.pickups, r.pickups, "{name}: ピックアップ");
        assert_eq!(s.solves, r.solves, "{name}: ソルブ");
        assert_eq!(s.field_norm, r.field_norm, "{name}");
        assert_eq!(s.aiming, r.aiming, "{name}");
        assert_eq!(s.glrs, r.glrs, "{name}");
        assert_eq!(s.ropd, r.ropd, "{name}");
        n += 1;
    }
    assert!(n > 25, "比べた系が少ない（{n}）");
}
