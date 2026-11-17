//! 文書の所在（文書モデルの 2 段目）: 開いた系のファイル・保存先・最後に作った組合せ。
//! 新規にする前の状態（[`NewSnap`]）にも同じ欄を写す

use super::*;

/// 文書の所在
#[derive(Default)]
pub(super) struct DocLoc {
    /// 読み込んだ系のファイル
    pub(super) file: Option<PathBuf>,
    /// 今の系（組合せ）を保存するファイル（開いた・保存したプロジェクトの .json。Ctrl+S の書き先）
    pub(super) doc_path: Option<PathBuf>,
    /// 保存先の無い組合せを作った元のファイル（.zmx から作った組合せ）。題名・保存の既定の名前に使う
    pub(super) doc_from: Option<PathBuf>,
    /// 組合せの系のファイル（開いている系が組合せか）
    pub(super) combo_path: Option<PathBuf>,
    /// 最後に組合せから作ったプロジェクト
    pub(super) built: Option<Value>,
    /// 最後に系を作った組合せ（今の組合せと違えば作り直す）
    pub(super) built_combo: crate::combo_doc::Snap,
    /// 最後に保存した・開いた組合せ（今の組合せと違えば未保存の変更あり）
    pub(super) saved_combo: crate::combo_doc::Snap,
    /// 開いた系を面の表で変えて、まだ保存していない
    pub(super) ld_dirty: bool,
}

/// 組合せの系・カタログの下書きを置くフォルダ。中身はプロセスの中のメモリ（[`crate::work_fs`]）。
/// ネイティブはプロセスごとに分ける。Web 版の置き場はページごとなので分けない
pub(super) fn work_dir() -> PathBuf {
    crate::work_fs::work_dir()
}

/// 終わるときに [`work_dir`] を消す。作るときに、異常終了で残った前のプロセスのフォルダ（2 日より古いもの）を消す
pub(super) struct WorkDir;

impl WorkDir {
    pub(super) fn new() -> WorkDir {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(root) = work_dir().parent() {
            sweep(
                root,
                std::process::id(),
                std::time::Duration::from_secs(2 * 86400),
            );
        }
        WorkDir
    }
}

/// root の下の、名前がプロセス id（数字だけ）で keep でなく、最後に書いてから max_age より古いフォルダを消す
#[cfg(not(target_arch = "wasm32"))]
fn sweep(root: &Path, keep: u32, max_age: std::time::Duration) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if pid != keep && old && e.path().is_dir() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[test]
fn sweep_removes_old_pid_dirs_only() {
    let root = std::env::temp_dir().join(format!("lensbench-sweep-{}", std::process::id()));
    for d in ["11", "22", "notes"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("33"), "file").unwrap();
    // 古さ 0 なら自分（22）と数字でない名前・ファイル以外を消す
    sweep(&root, 22, std::time::Duration::ZERO);
    let left: std::collections::BTreeSet<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["22", "33", "notes"].map(String::from).into());
    // 新しいものは消さない
    std::fs::create_dir_all(root.join("44")).unwrap();
    sweep(&root, 22, std::time::Duration::from_secs(3600));
    assert!(root.join("44").is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = std::fs::remove_dir_all(work_dir());
    }
}

impl DocLoc {
    /// 題名に出すファイル（組合せは保存先か作った元、開いた系はそのファイル）
    pub(super) fn shown(&self, combo: bool) -> Option<&PathBuf> {
        if combo {
            self.doc_path.as_ref().or(self.doc_from.as_ref())
        } else {
            self.file.as_ref()
        }
    }
}

/// .zmx から作った組合せは、保存するまで元のファイルの名前を出す
#[test]
fn shown_falls_back_to_the_source() {
    let mut l = DocLoc {
        file: Some("/tmp/w/combo.json".into()),
        doc_from: Some("/d/a.zmx".into()),
        ..Default::default()
    };
    assert_eq!(l.shown(true), Some(&PathBuf::from("/d/a.zmx")));
    assert_eq!(l.shown(false), Some(&PathBuf::from("/tmp/w/combo.json")));
    l.doc_path = Some("/d/a.json".into());
    assert_eq!(l.shown(true), Some(&PathBuf::from("/d/a.json")));
}
