//! アプリ版に同梱する見本（例題の組合せ・プロジェクトとそのレンズ、tests/refs の系、公開用ガラス）。
//! 一覧は build.rs が作る。開くときに設定の隣（無ければ一時フォルダ）の `samples` に書き出して開く。
//! Web 版は lensbench-web が同じものを置き場に入れて `Args::samples` で渡すので、ここは空

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/samples.rs"));

static BASE: OnceLock<PathBuf> = OnceLock::new();

/// 起動時に 1 回: 書き出す先 `base/samples`（base は設定のフォルダ。無ければ一時フォルダ）
pub(crate) fn install(base: Option<PathBuf>) {
    let d = base
        .unwrap_or_else(|| std::env::temp_dir().join("lensbench"))
        .join("samples");
    let _ = BASE.set(d);
}

fn base() -> PathBuf {
    BASE.get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("lensbench").join("samples"))
}

/// 見本の一覧（書き出した先のパス）。並びは Web 版と同じ: 組合せ・プロジェクト・tests/refs の系
pub(crate) fn list() -> Vec<String> {
    list_in(&base())
}

fn list_in(base: &Path) -> Vec<String> {
    FILES
        .iter()
        .map(|f| f.0)
        .filter(|p| {
            ((p.starts_with("examples/combos/") || p.starts_with("examples/projects/"))
                && p.ends_with(".json"))
                || p.ends_with("/system.zmx")
        })
        .map(|p| base.join(p).display().to_string())
        .collect()
}

/// 見本の全ファイルを書き出す（中身が同じなら書かない）。書けたら (ガラスのフォルダ, レンズのフォルダ)
pub(crate) fn extract() -> Result<(String, String), String> {
    extract_to(&base())
}

fn extract_to(base: &Path) -> Result<(String, String), String> {
    for (rel, b) in FILES {
        let p = base.join(rel);
        if std::fs::read(&p).is_ok_and(|x| x == *b) {
            continue;
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, b).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok((
        base.join("tests/glass").display().to_string(),
        base.join("examples/lenses").display().to_string(),
    ))
}

/// このパスが見本か（開く前に書き出す）
pub(crate) fn is_sample(f: &str) -> bool {
    Path::new(f).starts_with(base())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_bundled_and_extract() {
        let d = std::env::temp_dir().join(format!("lb-samples-{}", std::process::id()));
        let l = list_in(&d);
        // 組合せ・プロジェクト・Cooke トリプレット（A0）が入っている
        assert!(l.iter().any(|f| f.contains("examples/combos/")), "{l:?}");
        assert!(
            l.iter().any(|f| f.ends_with("P8_catalog_two.json")),
            "{l:?}"
        );
        assert!(l.iter().any(|f| f.ends_with("A0/system.zmx")), "{l:?}");
        let (g, lens) = extract_to(&d).unwrap();
        assert!(Path::new(&g).join("SCHOTT.AGF").is_file());
        assert!(Path::new(&lens).is_dir());
        assert!(l.iter().all(|f| Path::new(f).is_file()));
        // 2 回目は書かずに通る
        extract_to(&d).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }
}
