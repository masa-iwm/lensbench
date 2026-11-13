//! 版の欄に出す git のハッシュ（LENSBENCH_GIT_HASH で上書き。git が無ければ空）と、Windows の exe のアイコン

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=LENSBENCH_GIT_HASH");
    let hash = std::env::var("LENSBENCH_GIT_HASH").ok().or_else(|| {
        let h = git(&["rev-parse", "--short=9", "HEAD"])?;
        // 手元で変えた所があれば印を付ける
        let dirty =
            git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        Some(if dirty { format!("{h}+") } else { h })
    });
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        let dir = Path::new(&dir);
        // index は見ない（git add のたびに GUI のクレートが作り直しになる。手元の変更の印「+」はコミット・
        // ブランチの切り替えのときに付け直す。公開 #216 GAR-22）
        for f in ["HEAD", "packed-refs"] {
            println!("cargo:rerun-if-changed={}", dir.join(f).display());
        }
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={}", dir.join(r).display());
        }
    }
    println!(
        "cargo:rustc-env=LENSBENCH_GIT_HASH={}",
        hash.unwrap_or_default()
    );
    icon();
}

/// exe のアイコン（assets/icon.ico、元は icon.svg）
#[cfg(windows)]
fn icon() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        if let Err(e) = r.compile() {
            println!("cargo:warning=exe のアイコンを入れられない: {e}");
        }
    }
}

#[cfg(not(windows))]
fn icon() {}
