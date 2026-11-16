//! 組合せの系を serve に渡す置き場（GAR-03）: [`crate::app`] の作業フォルダの下だけをプロセスの中のメモリに置き、
//! ほかのパスはふつうのファイルの口へ回す。組合せを変えるたびにディスクへ書かない（ドラッグ中も）、Windows で読み中の
//! ファイルを置き換えられない逃げ道も要らない。計算のスレッド（[`crate::worker`]）と、系を書く GUI のスレッドの
//! 両方がこの口の下で動く。wasm は既定の口がもともとメモリなので、そのまま
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use lensbench_core::vfs::{Fs, MemFs, Stamp};

/// 作業フォルダ dir の下だけメモリ、ほかは base
#[cfg(not(target_arch = "wasm32"))]
pub struct Overlay {
    dir: PathBuf,
    mem: MemFs,
    base: Arc<dyn Fs>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Overlay {
    pub fn new(dir: PathBuf, base: Arc<dyn Fs>) -> Overlay {
        Overlay {
            dir,
            mem: MemFs::new(),
            base,
        }
    }

    fn pick(&self, p: &Path) -> &dyn Fs {
        if p.starts_with(&self.dir) {
            &self.mem
        } else {
            &*self.base
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Fs for Overlay {
    fn read(&self, p: &Path) -> std::io::Result<Vec<u8>> {
        self.pick(p).read(p)
    }
    fn write(&self, p: &Path, b: &[u8]) -> std::io::Result<()> {
        self.pick(p).write(p, b)
    }
    fn write_atomic(&self, p: &Path, b: &[u8]) -> std::io::Result<()> {
        self.pick(p).write_atomic(p, b)
    }
    fn append(&self, p: &Path, b: &[u8]) -> std::io::Result<()> {
        self.pick(p).append(p, b)
    }
    fn is_file(&self, p: &Path) -> bool {
        self.pick(p).is_file(p)
    }
    fn is_dir(&self, p: &Path) -> bool {
        self.pick(p).is_dir(p)
    }
    fn read_dir(&self, p: &Path) -> std::io::Result<Vec<PathBuf>> {
        self.pick(p).read_dir(p)
    }
    fn create_dir_all(&self, p: &Path) -> std::io::Result<()> {
        self.pick(p).create_dir_all(p)
    }
    fn remove_file(&self, p: &Path) -> std::io::Result<()> {
        self.pick(p).remove_file(p)
    }
    fn rename(&self, a: &Path, b: &Path) -> std::io::Result<()> {
        match (a.starts_with(&self.dir), b.starts_with(&self.dir)) {
            (true, true) => self.mem.rename(a, b),
            (false, false) => self.base.rename(a, b),
            // 置き場をまたぐときは写して消す
            _ => {
                let x = self.read(a)?;
                self.write(b, &x)?;
                self.remove_file(a)
            }
        }
    }
    fn stamp(&self, p: &Path) -> std::io::Result<Stamp> {
        self.pick(p).stamp(p)
    }
    fn canonicalize(&self, p: &Path) -> std::io::Result<PathBuf> {
        self.pick(p).canonicalize(p)
    }
}

/// プロセスの置き場の口（作業フォルダの下はメモリ）
#[cfg(not(target_arch = "wasm32"))]
pub fn fs() -> Arc<Overlay> {
    static S: std::sync::OnceLock<Arc<Overlay>> = std::sync::OnceLock::new();
    S.get_or_init(|| Arc::new(Overlay::new(work_dir(), lensbench_core::vfs::default_fs())))
        .clone()
}

/// f を置き場の口の下で回す
pub fn with<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    return lensbench_core::vfs::with(fs(), f);
    #[cfg(target_arch = "wasm32")]
    f()
}

/// 作業フォルダ（組合せの系・カタログの玉の断面を置く。ネイティブはプロセスごと）
pub fn work_dir() -> PathBuf {
    let d = lensbench_core::vfs::temp_dir().join("lensbench-gui");
    #[cfg(not(target_arch = "wasm32"))]
    let d = d.join(std::process::id().to_string());
    d
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// 作業フォルダの下はディスクに書かず、ほかはふつうに書く
    #[test]
    fn work_dir_stays_in_memory() {
        let tmp =
            lensbench_core::vfs::temp_dir().join(format!("lb-overlay-{}", std::process::id()));
        let dir = tmp.join("work");
        let o = Arc::new(Overlay::new(dir.clone(), lensbench_core::vfs::default_fs()));
        lensbench_core::vfs::with(o.clone(), || {
            lensbench_core::vfs::create_dir_all(&dir).unwrap();
            lensbench_core::vfs::write(dir.join("a.json.tmp"), "1").unwrap();
            lensbench_core::vfs::rename(dir.join("a.json.tmp"), dir.join("a.json")).unwrap();
            assert_eq!(
                lensbench_core::vfs::read_to_string(dir.join("a.json")).unwrap(),
                "1"
            );
            assert!(lensbench_core::vfs::stamp(dir.join("a.json")).is_ok());
            lensbench_core::vfs::create_dir_all(&tmp).unwrap();
            lensbench_core::vfs::write(tmp.join("b.json"), "2").unwrap();
        });
        assert!(!dir.join("a.json").exists());
        assert_eq!(std::fs::read_to_string(tmp.join("b.json")).unwrap(), "2");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
