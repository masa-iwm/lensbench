//! 展開でファイル・カタログから読んだものの使い回し（[`SourceCache`]）

use super::*;
use lensbench_core::sync::LockExt;

/// 展開でファイル・カタログから読んだものの使い回し（[`ExpandOpts::cache`]）。
/// - カタログの玉: (メーカー, 型番) → (玉, 玉の .zmx のガラス)
/// - `.zmx`・系ファイルのブロック: (正規化したパス, 構成番号) → 系。ファイルの更新時刻と大きさが変われば読み直す
///
/// 同じ環境（ガラスのフォルダ・カタログ）の展開の間だけで使う。状態・経路・差し替えの組・GUI の編集ごとの展開が、
/// 同じファイルを読んで解析し直さないため（入れ子のプロジェクトは中の葉だけ覚える）
#[derive(Default)]
pub struct SourceCache {
    pub(crate) lenses: std::sync::Mutex<HashMap<(String, String), CachedLens>>,
    pub(crate) files:
        std::sync::Mutex<HashMap<(PathBuf, usize), (lensbench_core::vfs::Stamp, System)>>,
    scope: std::sync::Mutex<String>,
    /// 合流の先の BS の置き方（[`crate::splitter::splitter_places_cached`]）: 鍵 → (集める間に読んだファイルと
    /// その更新の印, 置き方)
    places: std::sync::Mutex<HashMap<String, PlacesEntry>>,
}

type PlacesArc = std::sync::Arc<HashMap<String, crate::splitter::SplitInfo>>;
type PlacesEntry = (
    Vec<(PathBuf, Option<lensbench_core::vfs::Stamp>)>,
    PlacesArc,
);

type CachedLens = std::sync::Arc<(Element, std::sync::Arc<HashMap<String, Glass>>)>;

impl std::fmt::Debug for SourceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceCache")
            .field("lenses", &self.lenses.lock_ok().len())
            .field("files", &self.files.lock_ok().len())
            .finish()
    }
}

impl SourceCache {
    /// 覚えているファイルの数（試験用）
    pub fn file_count(&self) -> usize {
        self.files.lock_ok().len()
    }

    /// 環境（ガラスのフォルダ・レンズカタログなど）を表す文字列 key が前と違えば、覚えたものを捨てる。
    /// 環境を変えうる呼び手（[`crate::open::Source`]）が展開の前に呼ぶ
    pub fn scope(&self, key: &str) {
        let mut s = self.scope.lock_ok();
        if *s != key {
            self.lenses.lock_ok().clear();
            self.files.lock_ok().clear();
            self.places.lock_ok().clear();
            *s = key.to_string();
        }
    }

    /// 覚えた BS の置き方（読んだファイルが変わっていなければ）
    pub(crate) fn places_get(&self, key: &str) -> Option<PlacesArc> {
        let m = self.places.lock_ok();
        let (read, got) = m.get(key)?;
        let same = read
            .iter()
            .all(|(f, st)| lensbench_core::vfs::stamp(f).ok() == *st);
        // 使い回すときも、読んだファイルを外側の記録（読み直しの判定）に残す
        let files: Vec<PathBuf> = read.iter().map(|(f, _)| f.clone()).collect();
        let got = got.clone();
        drop(m);
        lensbench_core::vfs::record(&files);
        same.then_some(got)
    }

    /// BS の置き方を覚える（read は集める間に読んだファイル）。覚える数は少しだけ（編集で鍵が変わり続けるので）
    pub(crate) fn places_put(&self, key: String, read: &[PathBuf], got: PlacesArc) {
        let read = read
            .iter()
            .map(|f| (f.clone(), lensbench_core::vfs::stamp(f).ok()))
            .collect();
        let mut m = self.places.lock_ok();
        if m.len() >= 8 {
            m.clear();
        }
        m.insert(key, (read, got));
    }

    pub(crate) fn file(
        &self,
        k: &(PathBuf, usize),
        st: &lensbench_core::vfs::Stamp,
    ) -> Option<System> {
        match self.files.lock_ok().get(k) {
            Some((s, sys)) if s == st => Some(sys.clone()),
            _ => None,
        }
    }
}
