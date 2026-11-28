//! 非順次の追跡がゴーストの順次の系（.zmx）を書く一時フォルダ（[`crate::nsc_view::ghost_dir`]、
//! `%TEMP%\lensbench\nsc-ghosts-<PID>`）の後始末。終わるときに自分のフォルダを消し、起動するときに、落ちて残った
//! ほかのプロセスのフォルダ（そのプロセスがもう居ないもの）を消す。
//!
//! プロセスが居るかは、Linux は `/proc/<PID>`、Windows はフォルダの中の `.lock`（起動の間ずっと、ほかから開けない
//! 形で開いておく）で見る。どちらでもない OS は、ほかのプロセスのフォルダには触らない

use std::path::Path;

/// フォルダの名前の頭
const PREFIX: &str = "nsc-ghosts-";

/// Windows: 起動の間ずっと開いておく `.lock`（ほかのプロセスはこのフォルダが使われていると分かる）
#[cfg(windows)]
static LOCK: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// 起動するときに呼ぶ: 前の起動が残したフォルダを消し、Windows では自分のフォルダに `.lock` を開いておく
pub(crate) fn start() {
    let Some(own) = crate::nsc_view::ghost_dir() else {
        return;
    };
    if let Some(root) = own.parent() {
        sweep(root, &own);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _ = std::fs::create_dir_all(&own);
        // 共有なし: ほかのプロセスは開けない・消せない
        if let Ok(f) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(own.join(".lock"))
        {
            if let Ok(mut l) = LOCK.lock() {
                *l = Some(f);
            }
        }
    }
}

/// 終わるときに呼ぶ: 自分のフォルダを消す（空になった `lensbench` のフォルダも）
pub(crate) fn finish() {
    let Some(own) = crate::nsc_view::ghost_dir() else {
        return;
    };
    // 開いている .lock を閉じてから消す
    #[cfg(windows)]
    if let Ok(mut l) = LOCK.lock() {
        l.take();
    }
    remove_own(&own);
}

/// 自分のフォルダを消し、親（`lensbench`）が空になれば親も消す
fn remove_own(own: &Path) {
    let _ = std::fs::remove_dir_all(own);
    if let Some(p) = own.parent() {
        // 空でなければ失敗する（ほかの GUI のフォルダ・tour のフォルダがある）
        let _ = std::fs::remove_dir(p);
    }
}

/// root の中の、own 以外の `nsc-ghosts-<PID>` で、そのプロセスがもう居ないものを消す
fn sweep(root: &Path, own: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p == own || !p.is_dir() {
            continue;
        }
        let Some(pid) = e
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix(PREFIX))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if stale(&p, pid) {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

/// フォルダ p（プロセス pid のもの）を使うプロセスがもう居ないか（分からなければ false）
fn stale(p: &Path, pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let _ = p;
        !Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(windows)]
    {
        let _ = pid;
        let lock = p.join(".lock");
        // 居るプロセスは共有なしで開いているので、開けない（.lock の無い古い版のフォルダは居ないとみなす）
        !lock.exists() || std::fs::OpenOptions::new().write(true).open(&lock).is_ok()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (p, pid);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 居ないプロセスのフォルダは消え、自分のフォルダと名前の違うフォルダは残る。終わると自分のフォルダも消える
    #[cfg(target_os = "linux")]
    #[test]
    fn sweep_removes_folders_of_gone_processes() {
        let root =
            std::env::temp_dir().join(format!("lensbench-ghost-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let own = root.join(format!("{PREFIX}{}", std::process::id()));
        // PID の上限（4194304）より大きいので居ない
        let gone = root.join(format!("{PREFIX}4294967295"));
        let other = root.join("cad");
        for d in [&own, &gone, &other] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("ghost_1.zmx"), "x").unwrap();
        }
        // 居るプロセス（PID 1）のフォルダも残る
        let alive = root.join(format!("{PREFIX}1"));
        std::fs::create_dir_all(&alive).unwrap();
        sweep(&root, &own);
        assert!(own.exists() && other.exists() && alive.exists());
        assert!(!gone.exists());
        remove_own(&own);
        assert!(!own.exists());
        // ほかのフォルダがあるので親は残る
        assert!(root.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
