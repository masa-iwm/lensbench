//! レンズカタログの差し込み口。
//!
//! CLI・GUI・Python が読む形式は [`register_source`] で足せる（既定は .zmx だけ）。別の exe に組み込むときは、
//! 起動の最初に登録してから CLI・GUI の入口を呼ぶ（ZMF など公開側に置けない形式を非公開側で足す用）。

use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

/// カタログ中の 1 レンズ（面データは .zmx 断片のまま持つ）。
#[derive(Debug, Clone)]
pub struct CatalogLens {
    pub vendor: String,
    pub part: String,
    pub efl: f64,
    pub zmx: String,
}

/// 外部のレンズカタログを読む実装が満たすトレイト。
pub trait LensCatalogSource {
    /// この実装が読める拡張子（小文字、ドットなし）。
    fn extensions(&self) -> &[&str];
    /// ファイルを読み、レンズの一覧を返す。
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String>;
}

/// .zmx 1 つを 1 レンズとするカタログ（自作のレンズや、カタログから書き出したレンズ）。
/// メーカーは親フォルダの名前、型番はファイル名（拡張子なし）。焦点距離はカタログの値がないので NaN。
pub struct ZmxFileSource;

impl LensCatalogSource for ZmxFileSource {
    fn extensions(&self) -> &[&str] {
        &["zmx"]
    }
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String> {
        let zmx = crate::glass::read_text(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = |p: Option<&Path>| {
            p.and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let part = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 相対パスでも親フォルダの名前が取れるように、絶対パスにしてから見る
        let abs = crate::vfs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Ok(vec![CatalogLens {
            vendor: name(abs.parent()),
            part,
            efl: f64::NAN,
            zmx,
        }])
    }
}

fn registered() -> &'static RwLock<Vec<Arc<dyn LensCatalogSource + Send + Sync>>> {
    static G: OnceLock<RwLock<Vec<Arc<dyn LensCatalogSource + Send + Sync>>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// プロセスで共通に使う形式を足す（同じ拡張子なら後から足したものを使う）。
/// [`LensCatalog::load_registered`] と、それを使う CLI・GUI・Python のすべてに効く
pub fn register_source(s: Arc<dyn LensCatalogSource + Send + Sync>) {
    registered()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(s);
}

/// プロセスで共通に使う形式の一覧（足したものが先、最後に [`ZmxFileSource`]）
pub fn sources() -> Vec<Arc<dyn LensCatalogSource + Send + Sync>> {
    let mut v: Vec<Arc<dyn LensCatalogSource + Send + Sync>> = registered()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .cloned()
        .collect();
    v.push(Arc::new(ZmxFileSource));
    v
}

/// 読み込んだレンズカタログの集まり。メーカー・型番は大文字・小文字を区別しない。
#[derive(Debug, Clone, Default)]
pub struct LensCatalog {
    lenses: Vec<CatalogLens>,
    index: std::collections::HashMap<(String, String), usize>,
}

impl LensCatalog {
    /// ファイルまたはフォルダ（中のファイル、1 階層）を、拡張子が合う source で読む。
    /// 同じメーカー・型番は後に読んだものを使う。
    pub fn load(
        paths: &[std::path::PathBuf],
        sources: &[&dyn LensCatalogSource],
    ) -> Result<LensCatalog, String> {
        let mut c = LensCatalog::default();
        for p in paths {
            let mut files = Vec::new();
            if crate::vfs::is_dir(p) {
                let rd = crate::vfs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
                files.extend(rd.into_iter().filter(|f| crate::vfs::is_file(f)));
                files.sort();
            } else {
                files.push(p.clone());
            }
            for f in files {
                let ext = f
                    .extension()
                    .map(|x| x.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                let src = sources
                    .iter()
                    .find(|s| s.extensions().contains(&ext.as_str()));
                match src {
                    Some(s) => {
                        for l in s.load(&f)? {
                            c.add(l);
                        }
                    }
                    None if crate::vfs::is_dir(p) => {} // フォルダの中の関係のないファイルは飛ばす
                    None => return Err(format!("{}: 読めない形式のレンズカタログ", f.display())),
                }
            }
        }
        Ok(c)
    }

    /// [`load`](Self::load) を、プロセスで共通の形式（[`sources`]）で
    pub fn load_registered(paths: &[std::path::PathBuf]) -> Result<LensCatalog, String> {
        let src = sources();
        let refs: Vec<&dyn LensCatalogSource> = src
            .iter()
            .map(|s| s.as_ref() as &dyn LensCatalogSource)
            .collect();
        Self::load(paths, &refs)
    }

    fn key(vendor: &str, part: &str) -> (String, String) {
        (vendor.trim().to_uppercase(), part.trim().to_uppercase())
    }

    pub fn add(&mut self, l: CatalogLens) {
        let k = Self::key(&l.vendor, &l.part);
        match self.index.get(&k) {
            Some(&i) => self.lenses[i] = l,
            None => {
                self.index.insert(k, self.lenses.len());
                self.lenses.push(l);
            }
        }
    }

    pub fn get(&self, vendor: &str, part: &str) -> Option<&CatalogLens> {
        self.index
            .get(&Self::key(vendor, part))
            .map(|&i| &self.lenses[i])
    }

    /// (メーカー, 型番) → .zmx の本文（プロジェクトの展開に渡す形）
    pub fn zmx(&self, vendor: &str, part: &str) -> Result<String, String> {
        self.get(vendor, part)
            .map(|l| l.zmx.clone())
            .ok_or_else(|| format!("レンズカタログに {vendor} {part} がない"))
    }

    pub fn lenses(&self) -> &[CatalogLens] {
        &self.lenses
    }

    pub fn len(&self) -> usize {
        self.lenses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lenses.is_empty()
    }
}
