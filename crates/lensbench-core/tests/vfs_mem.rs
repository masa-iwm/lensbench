//! ファイルの口を計算ごとに渡す: メモリの上の置き場（`vfs::MemFs`）に系とガラスを入れると、ディスクに
//! 触らずに系を読めて、結果はディスクから読んだものと同じ。口の外（既定のファイルシステム）には漏れない

use lensbench_core::system::FieldSel;
use lensbench_core::vfs::{self, Fs, MemFs};
use lensbench_formats::zmx::{self, LoadOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(sys: &Path, glass: PathBuf) -> lensbench_core::Result<lensbench_core::System> {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![glass],
        ..Default::default()
    };
    zmx::load_with(sys, &opt)
}

#[test]
fn system_loads_from_memory() {
    let disk = load(
        &root().join("tests/refs/A0/system.zmx"),
        root().join("tests/glass"),
    )
    .unwrap();

    let m = MemFs::new();
    m.write(
        Path::new("/mem/a0.zmx"),
        &std::fs::read(root().join("tests/refs/A0/system.zmx")).unwrap(),
    )
    .unwrap();
    for e in std::fs::read_dir(root().join("tests/glass")).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_owned();
        m.write(
            &Path::new("/mem/glass").join(name),
            &std::fs::read(&p).unwrap(),
        )
        .unwrap();
    }
    let m: Arc<dyn Fs> = Arc::new(m);
    let mem = vfs::with(m.clone(), || {
        assert!(vfs::is_file("/mem/a0.zmx") && vfs::is_dir("/mem/glass"));
        load(Path::new("/mem/a0.zmx"), PathBuf::from("/mem/glass"))
    })
    .unwrap();
    let (px, py) = ([0.0, 0.3, -0.7], [0.0, 0.5, 0.2]);
    for fi in 0..disk.fields.len() {
        let a = disk
            .try_trace(FieldSel::Index(fi), &px, &py, None, None, false)
            .unwrap();
        let b = mem
            .try_trace(FieldSel::Index(fi), &px, &py, None, None, false)
            .unwrap();
        for (ra, rb) in a.rays.iter().zip(&b.rays) {
            assert_eq!(ra.xyz, rb.xyz);
        }
    }
    // 口の外では置き場のパスは見えない
    assert!(!vfs::is_file("/mem/a0.zmx"));
    assert!(load(Path::new("/mem/a0.zmx"), PathBuf::from("/mem/glass")).is_err());
    // 並列の作業のスレッドにも引き継ぐ
    let seen = vfs::with(m, || {
        lensbench_core::with_threads(Some(3), || {
            lensbench_core::par::map(&[0, 1, 2, 3], |_| vfs::is_file("/mem/a0.zmx"))
        })
    });
    assert!(seen.iter().all(|&x| x), "{seen:?}");
}

#[test]
fn shared_loads_do_not_mix_stores() {
    // 別の置き場の同じパスに同じ大きさで中身の違う AGF を置く（ガラス名を 1 文字だけ変える）。読込の使い回しの
    // 印（stamp）が置き場ごとに違うので、2 つ目の置き場で 1 つ目の中身が返らない
    let txt = std::fs::read_to_string(root().join("tests/glass/OHARA.AGF")).unwrap();
    assert!(txt.contains("NM S-FSL5 ") && !txt.contains("S-FSL6"));
    let other = txt.replacen("NM S-FSL5 ", "NM S-FSL6 ", 1);
    assert_eq!(other.len(), txt.len());
    let names = ["T".to_string()];
    let dirs = [PathBuf::from("/g")];
    let glasses = |body: &str| {
        let m = MemFs::new();
        m.write(Path::new("/g/T.AGF"), body.as_bytes()).unwrap();
        vfs::with(Arc::new(m), || {
            let (g, missing) = lensbench_core::glass::load_catalogs_shared(&names, &dirs);
            assert!(missing.is_empty(), "{missing:?}");
            (g.contains_key("S-FSL5"), g.contains_key("S-FSL6"))
        })
    };
    assert_eq!(glasses(&txt), (true, false));
    assert_eq!(glasses(&other), (false, true));
}
