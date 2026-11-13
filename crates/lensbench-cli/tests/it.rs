//! 結合テストを 1 つの exe にまとめる（ファイルごとに exe を作るとリンクが 100 回になり、Windows のビルドが 6 分かかっていた。
//! 公開 #216 BLD-03）。tests/ の .rs は各ファイルが 1 つのモジュール。テストを足すときは下の一覧にも足す
//! （足し忘れは `every_test_file_is_listed` が落とす）。1 ファイルだけ回すときは `cargo test --test it <モジュール名>::`。

mod cad_step;
mod catalog_fill_step;
mod catalog_search;
mod cli;
mod config_table;
mod extend;
mod run;
mod search;
mod serve;
mod serve_ipc;
mod serve_optimize;
mod serve_swap;
mod serve_tolerance;
mod user_catalog;

/// tests/ の .rs が全部上の一覧にある（autotests = false なので、足し忘れると黙って回らなくなる）
#[test]
fn every_test_file_is_listed() {
    let me = include_str!("it.rs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let separate: &[&str] = &[];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if p.extension().is_some_and(|x| x == "rs") && stem != "it" && !separate.contains(&stem) {
            assert!(
                me.contains(&format!("\nmod {stem};\n")),
                "tests/{stem}.rs が tests/it.rs の一覧に無い"
            );
        }
    }
}
