//! 文書の所在（GAR-01 の 2 段目）: 開いた系のファイル・保存先・最後に作った組合せ。
//! 新規にする前の状態（[`NewSnap`]）にも同じ欄を写す

use super::*;

/// 文書の所在
#[derive(Default)]
pub(super) struct DocLoc {
    /// 読み込んだ系のファイル
    pub(super) file: Option<PathBuf>,
    /// 今の系（組合せ）を保存するファイル（開いた・保存したプロジェクトの .json。Ctrl+S の書き先）
    pub(super) doc_path: Option<PathBuf>,
    /// 組合せの系のファイル（開いている系が組合せか）
    pub(super) combo_path: Option<PathBuf>,
    /// 最後に組合せから作ったプロジェクト
    pub(super) built: Option<Value>,
    /// 最後に系を作った組合せ（今の組合せと違えば作り直す）
    pub(super) built_combo: Option<Combo>,
    /// 最後に保存した・開いた組合せ（今の組合せと違えば未保存の変更あり、GUX-02）
    pub(super) saved_combo: Option<Combo>,
    /// 開いた系を面の表で変えて、まだ保存していない
    pub(super) ld_dirty: bool,
}

/// 組合せの系・カタログの下書きを書く一時フォルダ。ネイティブはプロセスごとに分ける（同時に 2 つ起動しても
/// 互いの組合せを読み違えない、GAR-03）。Web 版の置き場はページごとなので分けない
pub(super) fn work_dir() -> PathBuf {
    let d = lensbench_core::vfs::temp_dir().join("lensbench-gui");
    #[cfg(not(target_arch = "wasm32"))]
    let d = d.join(std::process::id().to_string());
    d
}

/// 終わるときに [`work_dir`] を消す
#[derive(Default)]
pub(super) struct WorkDir;

impl Drop for WorkDir {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = std::fs::remove_dir_all(work_dir());
    }
}
