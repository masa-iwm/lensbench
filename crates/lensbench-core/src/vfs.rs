//! ファイルの読み書きの入口。ネイティブは `std::fs` そのまま。wasm（ブラウザ）にはファイルシステムが
//! 無いので、プロセスの中のファイル置き場（パス → 中身）を使う。見本の系・ガラス・利用者が読み込んだファイルは
//! [`insert`] で置き場に入れてから、ネイティブと同じパスで読む。
//!
//! 系・カタログ・膜・マクロなど、利用者のファイルを読む・書く所は `std::fs` ではなくこれを通す
//! （テストと、ネイティブでしか動かない機能（serve・run など）はそのままでよい）。

use std::io;
use std::path::{Path, PathBuf};

/// ファイルの更新の印（読み直しの判定用）: 更新時刻（分かれば）と大きさ
pub type Stamp = (Option<std::time::SystemTime>, u64);

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn read(p: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(p)
    }
    pub fn write(p: &Path, b: &[u8]) -> io::Result<()> {
        std::fs::write(p, b)
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

/// プロセスの中のファイル置き場（wasm の実装。ネイティブではテストだけ）
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod mem {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    /// パス（`/` 区切りに揃えた文字列）→ 中身と版（書くたびに増やす。読み直しの判定に使う）
    type Store = BTreeMap<String, (Vec<u8>, u64)>;

    fn store() -> std::sync::MutexGuard<'static, Store> {
        static S: OnceLock<Mutex<Store>> = OnceLock::new();
        S.get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// `.`・`..`・重なった `/` を畳んだ絶対パス（相対パスは `/` から）
    pub fn key(p: &Path) -> String {
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

    fn not_found(p: &Path) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: ファイルが無い", p.display()),
        )
    }

    /// 版の数（書く・消すたびに増やす）
    static V: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn next_version() -> u64 {
        V.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// 消したファイル（パス → 消したときの版）。別の置き場へ写すとき（[`changes_since`]）に使う
    fn removed() -> std::sync::MutexGuard<'static, BTreeMap<String, u64>> {
        static R: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();
        R.get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 書いた版（消した印を外す）
    fn put(k: String, b: Vec<u8>) -> u64 {
        let v = next_version();
        removed().remove(&k);
        store().insert(k, (b, v));
        v
    }

    /// 消す（消した印を付ける）
    fn drop_key(k: &str) -> Option<u64> {
        store().remove(k)?;
        let v = next_version();
        removed().insert(k.to_string(), v);
        Some(v)
    }

    /// いまの版（これより後の変更を [`changes_since`] が返す）
    pub fn seq() -> u64 {
        V.load(std::sync::atomic::Ordering::Relaxed) - 1
    }

    /// 版 seq より後に書いた・消したファイル（パス、中身（消したら None）、版）
    pub fn changes_since(seq: u64) -> Vec<(String, Option<Vec<u8>>, u64)> {
        let mut out: Vec<(String, Option<Vec<u8>>, u64)> = store()
            .iter()
            .filter(|(_, (_, v))| *v > seq)
            .map(|(k, (b, v))| (k.clone(), Some(b.clone()), *v))
            .collect();
        out.extend(
            removed()
                .iter()
                .filter(|(_, v)| **v > seq)
                .map(|(k, v)| (k.clone(), None, *v)),
        );
        out
    }

    /// 別の置き場の変更を当てる（中身が None なら消す）。当てた後の版
    pub fn apply(k: &str, b: Option<Vec<u8>>) -> u64 {
        let k = key(Path::new(k));
        match b {
            Some(b) => put(k, b),
            None => drop_key(&k).unwrap_or_else(seq),
        }
    }

    pub fn read(p: &Path) -> io::Result<Vec<u8>> {
        store()
            .get(&key(p))
            .map(|x| x.0.clone())
            .ok_or_else(|| not_found(p))
    }
    pub fn write(p: &Path, b: &[u8]) -> io::Result<()> {
        put(key(p), b.to_vec());
        Ok(())
    }
    pub fn append(p: &Path, b: &[u8]) -> io::Result<()> {
        let k = key(p);
        let mut x = store().get(&k).map(|e| e.0.clone()).unwrap_or_default();
        x.extend_from_slice(b);
        put(k, x);
        Ok(())
    }
    pub fn is_file(p: &Path) -> bool {
        store().contains_key(&key(p))
    }
    pub fn is_dir(p: &Path) -> bool {
        let k = key(p);
        let pre = if k == "/" { k } else { k + "/" };
        store()
            .range(pre.clone()..)
            .next()
            .is_some_and(|(f, _)| f.starts_with(&pre))
    }
    pub fn read_dir(p: &Path) -> io::Result<Vec<PathBuf>> {
        if !is_dir(p) {
            return Err(not_found(p));
        }
        let k = key(p);
        let pre = if k == "/" { k } else { k + "/" };
        let mut out: Vec<PathBuf> = Vec::new();
        for f in store().keys().filter(|f| f.starts_with(&pre)) {
            let first = f[pre.len()..].split('/').next().unwrap_or("");
            let q = PathBuf::from(format!("{pre}{first}"));
            if out.last() != Some(&q) {
                out.push(q);
            }
        }
        Ok(out)
    }
    pub fn create_dir_all(_p: &Path) -> io::Result<()> {
        // フォルダはファイルのパスから決まる（空のフォルダは持たない）
        Ok(())
    }
    pub fn remove_file(p: &Path) -> io::Result<()> {
        drop_key(&key(p)).map(|_| ()).ok_or_else(|| not_found(p))
    }
    pub fn rename(a: &Path, b: &Path) -> io::Result<()> {
        let x = read(a)?;
        drop_key(&key(a));
        put(key(b), x);
        Ok(())
    }
    pub fn stamp(p: &Path) -> io::Result<Stamp> {
        let s = store();
        let (b, v) = s.get(&key(p)).ok_or_else(|| not_found(p))?;
        // 時刻の代わりに版を使う（同じ大きさで書き直しても読み直す。秒にするのは Windows の SystemTime が 100 ns 刻みのため）
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(*v);
        Ok((Some(t), b.len() as u64))
    }
    pub fn canonicalize(p: &Path) -> io::Result<PathBuf> {
        if is_file(p) || is_dir(p) {
            Ok(PathBuf::from(key(p)))
        } else {
            Err(not_found(p))
        }
    }
    pub fn files() -> Vec<(PathBuf, u64)> {
        store()
            .iter()
            .map(|(k, (b, _))| (PathBuf::from(k), b.len() as u64))
            .collect()
    }
}

#[cfg(target_arch = "wasm32")]
use mem as imp;

/// ファイルの中身
pub fn read(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    imp::read(p.as_ref())
}

/// UTF-8 のテキストとして読む
pub fn read_to_string(p: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// 書く（あれば置き換える）
pub fn write(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    imp::write(p.as_ref(), b.as_ref())
}

/// 末尾に足す（無ければ作る）
pub fn append(p: impl AsRef<Path>, b: impl AsRef<[u8]>) -> io::Result<()> {
    imp::append(p.as_ref(), b.as_ref())
}

pub fn exists(p: impl AsRef<Path>) -> bool {
    is_file(&p) || is_dir(&p)
}

pub fn is_file(p: impl AsRef<Path>) -> bool {
    imp::is_file(p.as_ref())
}

pub fn is_dir(p: impl AsRef<Path>) -> bool {
    imp::is_dir(p.as_ref())
}

/// フォルダの中のファイル・フォルダのパス（並びは決まっていない）
pub fn read_dir(p: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    imp::read_dir(p.as_ref())
}

pub fn create_dir_all(p: impl AsRef<Path>) -> io::Result<()> {
    imp::create_dir_all(p.as_ref())
}

pub fn remove_file(p: impl AsRef<Path>) -> io::Result<()> {
    imp::remove_file(p.as_ref())
}

/// 名前を変える（置き換え先があれば置き換える）
pub fn rename(a: impl AsRef<Path>, b: impl AsRef<Path>) -> io::Result<()> {
    imp::rename(a.as_ref(), b.as_ref())
}

/// 更新の印（更新時刻と大きさ）
pub fn stamp(p: impl AsRef<Path>) -> io::Result<Stamp> {
    imp::stamp(p.as_ref())
}

pub fn canonicalize(p: impl AsRef<Path>) -> io::Result<PathBuf> {
    imp::canonicalize(p.as_ref())
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

/// 置き場のファイルの一覧（パスと大きさ）。wasm だけ
#[cfg(target_arch = "wasm32")]
pub fn files() -> Vec<(PathBuf, u64)> {
    imp::files()
}

/// 置き場を別の置き場（Web Worker の中の wasm）へ写す。wasm だけ
#[cfg(target_arch = "wasm32")]
pub mod sync {
    pub use super::mem::{apply, changes_since, seq};
}

#[cfg(test)]
mod tests {
    use super::mem;
    use std::path::{Path, PathBuf};

    #[test]
    fn memory_store_reads_lists_and_stamps() {
        let d = Path::new("/vfs-test");
        mem::write(&d.join("a/x.agf"), b"1").unwrap();
        mem::write(&d.join("a/./b/../y.agf"), b"22").unwrap();
        mem::write(Path::new("\\vfs-test\\a\\sub\\z.zmx"), b"3").unwrap();
        assert_eq!(mem::read(&d.join("a/y.agf")).unwrap(), b"22");
        assert!(mem::is_dir(&d.join("a")) && mem::is_dir(d) && !mem::is_dir(&d.join("a/x.agf")));
        assert!(mem::is_file(&d.join("a/x.agf")) && !mem::is_file(&d.join("a")));
        let mut l = mem::read_dir(&d.join("a")).unwrap();
        l.sort();
        let want: Vec<PathBuf> = ["/vfs-test/a/sub", "/vfs-test/a/x.agf", "/vfs-test/a/y.agf"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(l, want);
        assert!(mem::read_dir(&d.join("none")).is_err());
        // 同じ大きさで書き直しても印が変わる
        let s0 = mem::stamp(&d.join("a/x.agf")).unwrap();
        mem::write(&d.join("a/x.agf"), b"9").unwrap();
        assert_ne!(mem::stamp(&d.join("a/x.agf")).unwrap(), s0);
        mem::append(&d.join("a/x.agf"), b"8").unwrap();
        assert_eq!(mem::read(&d.join("a/x.agf")).unwrap(), b"98");
        mem::rename(&d.join("a/x.agf"), &d.join("a/w.agf")).unwrap();
        assert!(mem::read(&d.join("a/x.agf")).is_err());
        assert_eq!(mem::read(&d.join("a/w.agf")).unwrap(), b"98");
        mem::rename(&d.join("a/w.agf"), &d.join("a/x.agf")).unwrap();
        // 別の置き場へ写す変更: 書いたものと消したもの
        let s0 = mem::seq();
        mem::remove_file(&d.join("a/x.agf")).unwrap();
        mem::write(&d.join("a/new.agf"), b"n").unwrap();
        let mut ch: Vec<(String, Option<Vec<u8>>)> = mem::changes_since(s0)
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
        assert!(mem::changes_since(mem::seq()).is_empty());
        mem::apply("/vfs-test/a/x.agf", Some(b"r".to_vec()));
        assert_eq!(mem::read(&d.join("a/x.agf")).unwrap(), b"r");
        mem::apply("/vfs-test/a/x.agf", None);
        assert!(mem::read(&d.join("a/x.agf")).is_err());
    }
}
