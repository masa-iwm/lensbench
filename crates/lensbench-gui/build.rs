//! 版の欄に出す git のハッシュ（LENSBENCH_GIT_HASH で上書き。git が無ければ空）と、Windows の exe のアイコン
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる

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
        // ブランチの切り替えのときに付け直す）
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
    samples();
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out, keep);
        } else if keep(&p) {
            out.push(p);
        }
    }
}

/// アプリ版に同梱する見本（例題の組合せ・プロジェクトとそのレンズ、tests/refs の系、公開用ガラス）の一覧。
/// Web 版は lensbench-web が同じものを置き場に入れるので、ここでは空にする
fn samples() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("samples.rs");
    let mut src =
        String::from("/// (リポジトリの中のパス, 中身)\npub static FILES: &[(&str, &[u8])] = &[\n");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../..")
            .canonicalize()
            .unwrap();
        let ext = |e: &'static str| {
            move |p: &Path| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(e))
        };
        let mut files = Vec::new();
        walk(&root.join("examples/combos"), &mut files, &ext("json"));
        walk(&root.join("examples/projects"), &mut files, &ext("json"));
        walk(&root.join("examples/lenses"), &mut files, &|_| true);
        walk(&root.join("tests/refs"), &mut files, &|p| {
            p.file_name().is_some_and(|x| x == "system.zmx")
        });
        walk(&root.join("tests/glass"), &mut files, &ext("agf"));
        for f in &files {
            let rel = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            src.push_str(&format!(
                "    ({rel:?}, include_bytes!({:?})),\n",
                f.display().to_string()
            ));
        }
        for d in [
            "examples/combos",
            "examples/projects",
            "examples/lenses",
            "tests/refs",
            "tests/glass",
        ] {
            println!("cargo:rerun-if-changed={}", root.join(d).display());
        }
    }
    src.push_str("];\n");
    std::fs::write(out, src).unwrap();
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
