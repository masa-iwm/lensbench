//! ファイルのメニュー: 開く・名前を付けて保存（ファイルの窓は出さずに答えを置く: files::answer_next）

use super::*;

/// 開く → 系が出る → 状態を足す → 名前を付けて保存 → 書いたファイルに状態が入っている
#[test]
fn flow_open_and_save_as() {
    let mut d = Drive::new(
        "open and save",
        Args {
            glass_dirs: vec![root().join("tests/glass").to_string_lossy().into_owned()],
            ..Args::default()
        },
    );
    crate::files::answer_next(Some(
        root().join("examples/projects/P5_triplet_blocks.json"),
    ));
    d.menu(&["File", "Open… (Ctrl+O)"]);
    let st = d.app().test_state();
    assert_eq!(st["system"], true, "[open] {st}");
    assert!(!d.app().c.doc.combo.picks.is_empty(), "[open] {st}");
    d.type_into("State name", 0, "flows");
    let dir = std::env::temp_dir().join(format!("lensbench-flows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("saved.json");
    crate::files::answer_next(Some(out.clone()));
    d.menu(&["File", "Save As… (Ctrl+Shift+S)"]);
    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("[save as] {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        text.contains("\"flows\""),
        "[save as] the state is not in the file"
    );
    assert_eq!(
        d.app().test_state()["doc"],
        "saved.json",
        "[save as] {}",
        d.app().test_state()
    );
}
