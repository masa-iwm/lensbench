//! 結合テストを 1 つの exe にまとめる（ファイルごとに exe を作るとリンクが 100 回になり、Windows のビルドが 6 分かかっていた）。
//! tests/ の .rs は各ファイルが 1 つのモジュール。テストを足すときは下の一覧にも足す
//! （足し忘れは `every_test_file_is_listed` が落とす）。1 ファイルだけ回すときは `cargo test --test it <モジュール名>::`。
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる

mod beams;
mod catalog;
mod configs_zmx;
mod detector;
mod edit_ops;
mod expand;
mod field_solve;
mod filter;
mod mech;
mod os_a3;
mod os_p0;
mod pickups;
mod retarder;
mod schema;
mod solve;
mod splitter;
mod states;
mod surface_edits;
mod surface_ops;
mod sysfile;
mod units;
mod usermaterial;
mod usershape;
mod usersurf;

/// tests/ の .rs が全部上の一覧にある（autotests = false なので、足し忘れると黙って回らなくなる）
#[test]
fn every_test_file_is_listed() {
    let me = include_str!("it.rs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    // 別の exe（Cargo.toml の [[test]]）: 同梱のガラスカタログの往復は、このプロセスで読んだ AGF に左右されないよう分ける
    let separate: &[&str] = &["builtin_glass"];
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
mod zmx_beams;
mod zmx_wrap;

/// プロジェクトの JSON の id のブロック（群・BS の腕・.zmx の取り込みの群の中も）に、項目 key の値（JSON）を入れる。
/// ブロックの面の上書き・操作（surface_edits・surface_ops）の試験で使う。id が無ければ panic
pub fn put(txt: &str, key: &str, by_id: &[(&str, &str)]) -> String {
    fn walk(v: &mut serde_json::Value, id: &str, key: &str, x: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Object(m) => {
                if m.get("id").and_then(|i| i.as_str()) == Some(id)
                    && (m.contains_key("type") || m.contains_key("from"))
                {
                    m.insert(key.into(), x.clone());
                    return true;
                }
                m.iter_mut()
                    .filter(|(k, _)| !matches!(k.as_str(), "states" | "surfaces"))
                    .any(|(_, w)| walk(w, id, key, x))
            }
            serde_json::Value::Array(a) => a.iter_mut().any(|w| walk(w, id, key, x)),
            _ => false,
        }
    }
    let mut v: serde_json::Value = serde_json::from_str(txt).unwrap();
    for (id, x) in by_id {
        let x: serde_json::Value = serde_json::from_str(x).unwrap();
        assert!(walk(&mut v, id, key, &x), "no block {id}");
    }
    v.to_string()
}
