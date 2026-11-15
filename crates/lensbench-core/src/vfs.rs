//! ファイルの読み書きの入口。読み書きは今の計算のファイルの口（[`Fs`]）を通す（XCT-29）。既定はネイティブなら
//! `std::fs` そのまま（[`StdFs`]）、wasm（ブラウザ）にはファイルシステムが無いのでプロセスの中のファイル置き場
//! （[`MemFs`]、見本の系・ガラス・利用者が読み込んだファイルは [`insert`] で入れてから、ネイティブと同じパスで読む）。
//! [`with`] で計算ごとに別の口（メモリの上の [`MemFs`] など）を渡せる: 文字列から系を組む・試験で偽のファイルを
//! 渡す、がディスクに触らずにできる。口は計算の文脈（[`crate::par::Ctx`]）に載り、並列の作業のスレッドにも効く。
//!
//! 系・カタログ・膜・マクロなど、利用者のファイルを読む・書く所は `std::fs` ではなくこれを通す
//! （テストと、ネイティブでしか動かない機能（serve・run など）はそのままでよい）。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// ファイルの読み書きの口（[`with`] で計算ごとに渡す）。パスはネイティブと同じ形で受ける
pub trait Fs: Send + Sync {
    fn read(&self, p: &Path) -> io::Result<Vec<u8>>;
    /// 書く（あれば置き換える）
    fn write(&self, p: &Path, b: &[u8]) -> io::Result<()>;
    /// 書く（途中で落ちても前か新しい中身のどちらかが残る）。既定は [`Fs::write`]
    fn write_atomic(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        self.write(p, b)
    }
    fn append(&self, p: &Path, b: &[u8]) -> io::Result<()>;
    fn is_file(&self, p: &Path) -> bool;
    fn is_dir(&self, p: &Path) -> bool;
    /// フォルダの中のファイル・フォルダのパス（並びは決まっていない）
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>>;
    fn create_dir_all(&self, p: &Path) -> io::Result<()>;
    fn remove_file(&self, p: &Path) -> io::Result<()>;
    fn rename(&self, a: &Path, b: &Path) -> io::Result<()>;
    /// 更新の印（読み直しの判定用）
    fn stamp(&self, p: &Path) -> io::Result<Stamp>;
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf>;
}

/// ファイルの更新の印（読み直しの判定用）: 更新時刻（分かれば）と大きさ
pub type Stamp = (Option<std::time::SystemTime>, u64);

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn read(p: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(p)
    }
    /// 同じフォルダの一時ファイルに書いてから名前を変える（書く途中で落ちても元のファイルは残る）。
    /// 書き先がシンボリックリンクならリンクの先を置き換え、普通のファイルでないもの（/dev/stdout・名前付きパイプなど）
    /// には直接書く（置き換えると壊す）
    pub fn write(p: &Path, b: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let resolved;
        let p = match std::fs::metadata(p) {
            Ok(m) if !m.is_file() => return std::fs::write(p, b),
            Ok(_) if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) => {
                resolved = std::fs::canonicalize(p)?;
                resolved.as_path()
            }
            _ => p,
        };
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let name = p
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(name);
        tmp_name.push(format!(
            ".{}-{}.tmp",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = p.with_file_name(tmp_name);
        let res = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(b)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, p)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }
    pub fn write_atomic(p: &Path, b: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let dir = match p.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let name = p
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(name);
        tmp_name.push(format!(".{}.tmp", std::process::id()));
        let tmp = dir.join(tmp_name);
        let r = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(b)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, p)
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r
    }
    pub fn append(p: &Path, b: &[u8]) -> io::Result<()> {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(p)?
            .write_all(b)
    }
    pub fn is_file(p: &Path) -> bool {
        p.is_file()
    }
    pub fn is_dir(p: &Path) -> bool {
        p.is_dir()
    }
    pub fn read_dir(p: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect()
    }
    pub fn create_dir_all(p: &Path) -> io::Result<()> {
        std::fs::create_dir_all(p)
    }
    pub fn remove_file(p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }
    pub fn rename(a: &Path, b: &Path) -> io::Result<()> {
        std::fs::rename(a, b)
    }
    pub fn stamp(p: &Path) -> io::Result<Stamp> {
        let m = std::fs::metadata(p)?;
        Ok((m.modified().ok(), m.len()))
    }
    pub fn canonicalize(p: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(p)
    }
}

/// ネイティブのファイルシステム（`std::fs`）
#[cfg(not(target_arch = "wasm32"))]
pub struct StdFs;

#[cfg(not(target_arch = "wasm32"))]
impl Fs for StdFs {
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        imp::read(p)
    }
    fn write(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        imp::write(p, b)
    }
    fn write_atomic(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        imp::write_atomic(p, b)
    }
    fn append(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        imp::append(p, b)
    }
    fn is_file(&self, p: &Path) -> bool {
        imp::is_file(p)
    }
    fn is_dir(&self, p: &Path) -> bool {
        imp::is_dir(p)
    }
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        imp::read_dir(p)
    }
    fn create_dir_all(&self, p: &Path) -> io::Result<()> {
        imp::create_dir_all(p)
    }
    fn remove_file(&self, p: &Path) -> io::Result<()> {
        imp::remove_file(p)
    }
    fn rename(&self, a: &Path, b: &Path) -> io::Result<()> {
        imp::rename(a, b)
    }
    fn stamp(&self, p: &Path) -> io::Result<Stamp> {
        imp::stamp(p)
    }
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        imp::canonicalize(p)
    }
}

/// メモリの上のファイル置き場（パス → 中身）。wasm の既定の口で、ネイティブでも [`with`] で渡せる
/// （試験の偽のファイル・文字列から組む系）。パスは `/` 区切りの絶対パスに揃える（相対パスは `/` から）
#[derive(Default)]
pub struct MemFs {
    inner: std::sync::Mutex<MemInner>,
}

/// パス → 中身と版（書くたびに増やす。読み直しの判定に使う）、消したファイル（パス → 消したときの版）
#[derive(Default)]
struct MemInner {
    store: std::collections::BTreeMap<String, (Vec<u8>, u64)>,
    removed: std::collections::BTreeMap<String, u64>,
    ver: u64,
}

impl MemInner {
    /// 次の版。版の数はプロセスで 1 つ（どの置き場でも増えるだけ）: 別の置き場の同じパス・同じ大きさのファイルの
    /// stamp が重ならず、stamp で読込を使い回す所（glass::load_agf_shared など）が前の置き場の中身を返さない
    fn next(&mut self) -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static VER: AtomicU64 = AtomicU64::new(0);
        self.ver = VER.fetch_add(1, Ordering::Relaxed) + 1;
        self.ver
    }
    /// 書いた版（消した印を外す）
    fn put(&mut self, k: String, b: Vec<u8>) -> u64 {
        let v = self.next();
        self.removed.remove(&k);
        self.store.insert(k, (b, v));
        v
    }
    /// 消す（消した印を付ける）
    fn drop_key(&mut self, k: &str) -> Option<u64> {
        self.store.remove(k)?;
        let v = self.next();
        self.removed.insert(k.to_string(), v);
        Some(v)
    }
    fn is_dir(&self, k: &str) -> bool {
        let pre = dir_prefix(k);
        self.store
            .range(pre.clone()..)
            .next()
            .is_some_and(|(f, _)| f.starts_with(&pre))
    }
}

/// `.`・`..`・重なった `/` を畳んだ絶対パス（相対パスは `/` から）
fn mem_key(p: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in p.to_string_lossy().replace('\\', "/").split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c.to_string()),
        }
    }
    format!("/{}", parts.join("/"))
}

fn dir_prefix(k: &str) -> String {
    if k == "/" {
        k.to_string()
    } else {
        format!("{k}/")
    }
}

fn not_found(p: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: no such file", p.display()),
    )
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// いまの版（これより後の変更を [`MemFs::changes_since`] が返す）
    pub fn seq(&self) -> u64 {
        self.lock().ver
    }

    /// 版 seq より後に書いた・消したファイル（パス、中身（消したら None）、版）
    pub fn changes_since(&self, seq: u64) -> Vec<(String, Option<Vec<u8>>, u64)> {
        let g = self.lock();
        let mut out: Vec<(String, Option<Vec<u8>>, u64)> = g
            .store
            .iter()
            .filter(|(_, (_, v))| *v > seq)
            .map(|(k, (b, v))| (k.clone(), Some(b.clone()), *v))
            .collect();
        out.extend(
            g.removed
                .iter()
                .filter(|(_, v)| **v > seq)
                .map(|(k, v)| (k.clone(), None, *v)),
        );
        out
    }

    /// 別の置き場の変更を当てる（中身が None なら消す）。当てた後の版
    pub fn apply(&self, k: &str, b: Option<Vec<u8>>) -> u64 {
        let k = mem_key(Path::new(k));
        let mut g = self.lock();
        match b {
            Some(b) => g.put(k, b),
            None => g.drop_key(&k).unwrap_or(g.ver),
        }
    }

    /// 置き場のファイルの一覧（パスと大きさ）
    pub fn files(&self) -> Vec<(PathBuf, u64)> {
        self.lock()
            .store
            .iter()
            .map(|(k, (b, _))| (PathBuf::from(k), b.len() as u64))
            .collect()
    }
}

impl Fs for MemFs {
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        self.lock()
            .store
            .get(&mem_key(p))
            .map(|x| x.0.clone())
            .ok_or_else(|| not_found(p))
    }
    fn write(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        self.lock().put(mem_key(p), b.to_vec());
        Ok(())
    }
    fn append(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        let k = mem_key(p);
        let mut g = self.lock();
        let mut x = g.store.get(&k).map(|e| e.0.clone()).unwrap_or_default();
        x.extend_from_slice(b);
        g.put(k, x);
        Ok(())
    }
    fn is_file(&self, p: &Path) -> bool {
        self.lock().store.contains_key(&mem_key(p))
    }
    fn is_dir(&self, p: &Path) -> bool {
        self.lock().is_dir(&mem_key(p))
    }
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        let k = mem_key(p);
        let g = self.lock();
        if !g.is_dir(&k) {
            return Err(not_found(p));
        }
        let pre = dir_prefix(&k);
        let mut out: Vec<PathBuf> = Vec::new();
        for f in g.store.keys().filter(|f| f.starts_with(&pre)) {
            let first = f[pre.len()..].split('/').next().unwrap_or("");
            let q = PathBuf::from(format!("{pre}{first}"));
            if out.last() != Some(&q) {
                out.push(q);
            }
        }
        Ok(out)
    }
    fn create_dir_all(&self, _p: &Path) -> io::Result<()> {
        // フォルダはファイルのパスから決まる（空のフォルダは持たない）
        Ok(())
    }
    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.lock()
            .drop_key(&mem_key(p))
            .map(|_| ())
            .ok_or_else(|| not_found(p))
    }
    fn rename(&self, a: &Path, b: &Path) -> io::Result<()> {
        let mut g = self.lock();
        let ka = mem_key(a);
        let x = g
            .store
            .get(&ka)
            .map(|e| e.0.clone())
            .ok_or_else(|| not_found(a))?;
        g.drop_key(&ka);
        g.put(mem_key(b), x);
        Ok(())
    }
    fn stamp(&self, p: &Path) -> io::Result<Stamp> {
        let g = self.lock();
        let (b, v) = g.store.get(&mem_key(p)).ok_or_else(|| not_found(p))?;
        // 時刻の代わりに版を使う（同じ大きさで書き直しても読み直す。秒にするのは Windows の SystemTime が 100 ns 刻みのため）
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(*v);
        Ok((Some(t), b.len() as u64))
    }
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        if self.is_file(p) || self.is_dir(p) {
            Ok(PathBuf::from(mem_key(p)))
        } else {
            Err(not_found(p))
        }
    }
}

/// wasm のプロセスの置き場（ブラウザにはこれしかファイルの置き場がない）
#[cfg(target_arch = "wasm32")]
fn wasm_store() -> &'static Arc<MemFs> {
    static S: std::sync::OnceLock<Arc<MemFs>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// プロセスの既定の口（ネイティブは [`StdFs`]、wasm はプロセスの置き場の [`MemFs`]）
pub fn default_fs() -> Arc<dyn Fs> {
    #[cfg(target_arch = "wasm32")]
    return wasm_store().clone();
    #[cfg(not(target_arch = "wasm32"))]
    {
        static S: std::sync::OnceLock<Arc<dyn Fs>> = std::sync::OnceLock::new();
        S.get_or_init(|| Arc::new(StdFs)).clone()
    }
}

/// 今の計算の口（[`with`] で渡したもの、無ければ [`default_fs`]）
pub fn current() -> Arc<dyn Fs> {
    crate::par::current_fs().unwrap_or_else(default_fs)
}

/// f をファイルの口 fs の下で回す（文脈のほかの項目は引き継ぐ。並列の作業のスレッドにも効く）
pub fn with<R>(fs: Arc<dyn Fs>, f: impl FnOnce() -> R) -> R {
    crate::par::with_fs(fs, f)
}

thread_local! {
    /// [`recording`] の間にこのスレッドで読んだファイル・フォルダ
    static RECORD: std::cell::RefCell<Option<std::collections::BTreeSet<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

fn note(p: &Path) {
    RECORD.with(|r| {
        if let Some(s) = r.borrow_mut().as_mut() {
            s.insert(p.to_path_buf());
        }
    });
}

/// f を呼び、その間にこのスレッドで [`read`]・[`read_dir`] で開いた（開こうとした）パスを並べて返す（読み直しの
/// 判定に使う。公開 #67: カタログの一覧が参照する、見ているフォルダの外のファイル）。ほかのスレッドで読んだものは
/// 入らない（f の中で並列に読むなら使えない）。入れ子にすると内側の分は外側にも入る
pub fn recording<R>(f: impl FnOnce() -> R) -> (R, Vec<PathBuf>) {
    let outer = RECORD.with(|r| r.borrow_mut().replace(Default::default()));
    let res = f();
    let got = RECORD.with(|r| {
        let mut g = r.borrow_mut();
        let got = g.take().unwrap_or_default();
        if let Some(mut o) = outer {
            o.extend(got.iter().cloned());
            *g = Some(o);
        }
        got
    });
    (res, got.into_iter().collect())
}

/// ファイルの中身
pub fn read(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    note(p.as_ref());
    current().read(p.as_ref())
}

/// UTF-8 のテキストとして読む
pub fn read_to_string(p: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// 書く（あれば置き換える）
pub fn write(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    current().write(p.as_ref(), b.as_ref())
}

/// 書く（あれば置き換える）。途中で落ちても前の中身か新しい中身のどちらかが残るように、同じフォルダの
/// 一時ファイルに書いて `sync_all` してから名前を替える。カタログ・系ファイルなど作り直しのきかないものの保存に
pub fn write_atomic(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    current().write_atomic(p.as_ref(), b.as_ref())
}

/// 末尾に足す（無ければ作る）
pub fn append(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    current().append(p.as_ref(), b.as_ref())
}

pub fn exists(p: impl AsRef<Path>) -> bool {
    is_file(&p) || is_dir(&p)
}

pub fn is_file(p: impl AsRef<Path>) -> bool {
    current().is_file(p.as_ref())
}

pub fn is_dir(p: impl AsRef<Path>) -> bool {
    current().is_dir(p.as_ref())
}

/// フォルダの中のファイル・フォルダのパス（並びは決まっていない）
pub fn read_dir(p: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    note(p.as_ref());
    current().read_dir(p.as_ref())
}

pub fn create_dir_all(p: impl AsRef<Path>) -> io::Result<()> {
    current().create_dir_all(p.as_ref())
}

pub fn remove_file(p: impl AsRef<Path>) -> io::Result<()> {
    current().remove_file(p.as_ref())
}

/// 名前を変える（置き換え先があれば置き換える）
pub fn rename(a: impl AsRef<Path>, b: impl AsRef<Path>) -> io::Result<()> {
    current().rename(a.as_ref(), b.as_ref())
}

/// 更新の印（更新時刻と大きさ）
pub fn stamp(p: impl AsRef<Path>) -> io::Result<Stamp> {
    current().stamp(p.as_ref())
}

pub fn canonicalize(p: impl AsRef<Path>) -> io::Result<PathBuf> {
    current().canonicalize(p.as_ref())
}

/// 一時ファイルのフォルダ（ネイティブは `std::env::temp_dir`、wasm は置き場の `/tmp`。wasm の
/// `std::env::temp_dir` は panic する）
pub fn temp_dir() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    return PathBuf::from("/tmp");
    #[cfg(not(target_arch = "wasm32"))]
    std::env::temp_dir()
}

/// 置き場にファイルを入れる（wasm 用。ネイティブでは [`write`] と同じ）
pub fn insert(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    write(p, b)
}

/// プロセスの置き場のファイルの一覧（パスと大きさ）。wasm だけ
#[cfg(target_arch = "wasm32")]
pub fn files() -> Vec<(PathBuf, u64)> {
    wasm_store().files()
}

/// プロセスの置き場を別の置き場（Web Worker の中の wasm）へ写す。wasm だけ
#[cfg(target_arch = "wasm32")]
pub mod sync {
    /// いまの版
    pub fn seq() -> u64 {
        super::wasm_store().seq()
    }
    /// 版 seq より後に書いた・消したファイル
    pub fn changes_since(seq: u64) -> Vec<(String, Option<Vec<u8>>, u64)> {
        super::wasm_store().changes_since(seq)
    }
    /// 別の置き場の変更を当てる
    pub fn apply(k: &str, b: Option<Vec<u8>>) -> u64 {
        super::wasm_store().apply(k, b)
    }
}

#[cfg(test)]
mod tests {
    use super::{Fs, MemFs};
    use std::path::{Path, PathBuf};

    /// 保存は一時ファイルから名前を変える: 上書きでき、一時ファイルは残らない
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_write_is_atomic_replace() {
        let d = std::env::temp_dir().join(format!("lb-vfs-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("a.zmx");
        super::imp::write(&p, b"old").unwrap();
        super::imp::write(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(super::imp::write(&d.join("no/such/dir.zmx"), b"x").is_err());
        // シンボリックリンクはリンクの先を書き換え、リンクは残す
        #[cfg(unix)]
        {
            let link = d.join("link.zmx");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            super::imp::write(&link, b"via link").unwrap();
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read(&p).unwrap(), b"via link");
            // 普通のファイルでない書き先（/dev/null）は置き換えずに直接書く
            super::imp::write(Path::new("/dev/null"), b"x").unwrap();
            assert!(!std::fs::metadata("/dev/null").unwrap().is_file());
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn memory_store_reads_lists_and_stamps() {
        let mem = MemFs::new();
        let d = Path::new("/vfs-test");
        mem.write(&d.join("a/x.agf"), b"1").unwrap();
        mem.write(&d.join("a/./b/../y.agf"), b"22").unwrap();
        mem.write(Path::new("\\vfs-test\\a\\sub\\z.zmx"), b"3")
            .unwrap();
        assert_eq!(mem.read(&d.join("a/y.agf")).unwrap(), b"22");
        assert!(mem.is_dir(&d.join("a")) && mem.is_dir(d) && !mem.is_dir(&d.join("a/x.agf")));
        assert!(mem.is_file(&d.join("a/x.agf")) && !mem.is_file(&d.join("a")));
        let mut l = mem.read_dir(&d.join("a")).unwrap();
        l.sort();
        let want: Vec<PathBuf> = ["/vfs-test/a/sub", "/vfs-test/a/x.agf", "/vfs-test/a/y.agf"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(l, want);
        assert!(mem.read_dir(&d.join("none")).is_err());
        // 同じ大きさで書き直しても印が変わる
        let s0 = mem.stamp(&d.join("a/x.agf")).unwrap();
        mem.write(&d.join("a/x.agf"), b"9").unwrap();
        assert_ne!(mem.stamp(&d.join("a/x.agf")).unwrap(), s0);
        mem.append(&d.join("a/x.agf"), b"8").unwrap();
        assert_eq!(mem.read(&d.join("a/x.agf")).unwrap(), b"98");
        mem.rename(&d.join("a/x.agf"), &d.join("a/w.agf")).unwrap();
        assert!(mem.read(&d.join("a/x.agf")).is_err());
        assert_eq!(mem.read(&d.join("a/w.agf")).unwrap(), b"98");
        mem.rename(&d.join("a/w.agf"), &d.join("a/x.agf")).unwrap();
        // 別の置き場へ写す変更: 書いたものと消したもの
        let s0 = mem.seq();
        mem.remove_file(&d.join("a/x.agf")).unwrap();
        mem.write(&d.join("a/new.agf"), b"n").unwrap();
        let mut ch: Vec<(String, Option<Vec<u8>>)> = mem
            .changes_since(s0)
            .into_iter()
            .filter(|c| c.0.starts_with("/vfs-test/"))
            .map(|c| (c.0, c.1))
            .collect();
        ch.sort();
        assert_eq!(
            ch,
            vec![
                ("/vfs-test/a/new.agf".to_string(), Some(b"n".to_vec())),
                ("/vfs-test/a/x.agf".to_string(), None),
            ]
        );
        assert!(mem.changes_since(mem.seq()).is_empty());
        mem.apply("/vfs-test/a/x.agf", Some(b"r".to_vec()));
        assert_eq!(mem.read(&d.join("a/x.agf")).unwrap(), b"r");
        mem.apply("/vfs-test/a/x.agf", None);
        assert!(mem.read(&d.join("a/x.agf")).is_err());
    }

    /// 書き替えは一時ファイルを残さず、前の中身を置き換える
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn write_atomic_replaces() {
        let d = std::env::temp_dir().join(format!("lb-vfs-write-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("cat.json");
        super::write_atomic(&p, "a").unwrap();
        super::write_atomic(&p, "bb").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "bb");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
