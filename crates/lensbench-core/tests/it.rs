//! 結合テストを 1 つの exe にまとめる（ファイルごとに exe を作るとリンクが 100 回になり、Windows のビルドが 6 分かかっていた。
//! 公開 #216 BLD-03）。tests/ の .rs は各ファイルが 1 つのモジュール。テストを足すときは下の一覧にも足す
//! （足し忘れは `every_test_file_is_listed` が落とす）。1 ファイルだけ回すときは `cargo test --test it <モジュール名>::`。
//! batch は別の exe（wasm32-wasip1 でもその 1 本だけを回すため。Cargo.toml の [[test]]）。

mod afocal;
mod aim;
mod aim_wide;
mod birefringent_os;
mod cardinal;
mod common;
mod determinism;
mod dispersion_range;
mod edit;
mod elements_refs;
mod fermat;
mod fiber;
mod freeform;
mod ghost;
mod grin;
mod huygens;
mod huygens_refs;
mod invariants;
mod landing;
mod launch;
mod material_pickup;
mod mtf_family;
mod mtf_family_refs;
mod mtf_refs;
mod obsc_mtf;
mod opd_solve_os;
mod optim;
mod optim_terms;
mod params;
mod pickups;
mod polarization;
mod pop;
mod pop_os;
mod record_sel;
mod runs;
mod semi_auto;
mod spider;
mod surftypes;
mod surftypes_os;
mod tolerance;
mod units;
mod usersurf;
mod vig_interp;
mod zmx_broken;
mod zmx_lines;
mod zmx_roundtrip;
mod zmx_solves;
mod zmx_solves2;
mod zpl_solve;
mod zplm;

/// tests/ の .rs が全部上の一覧にある（autotests = false なので、足し忘れると黙って回らなくなる）
#[test]
fn every_test_file_is_listed() {
    let me = include_str!("it.rs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let separate: &[&str] = &["batch"];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if p.extension().is_some_and(|x| x == "rs") && stem != "it" && !separate.contains(&stem) {
            assert!(
                // 行ごとに見る（Windows の checkout は改行が CRLF になる）
                me.lines().any(|l| l.trim() == format!("mod {stem};")),
                "tests/{stem}.rs が tests/it.rs の一覧に無い"
            );
        }
    }
}
