//! 同梱する見本（公開用ガラス tests/glass、例題のレンズ・プロジェクト、tests/refs の系）の一覧を作る。
//! ファイルを足せば自動で同梱される

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out, keep);
        } else if keep(&p) {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let root = root.canonicalize().unwrap();
    let mut files = Vec::new();
    walk(&root.join("tests/glass"), &mut files, &|p| {
        p.extension().is_some_and(|x| x.eq_ignore_ascii_case("agf"))
    });
    walk(&root.join("examples/lenses"), &mut files, &|_| true);
    walk(&root.join("examples/projects"), &mut files, &|p| {
        p.extension().is_some_and(|x| x == "json")
    });
    walk(&root.join("tests/refs"), &mut files, &|p| {
        p.file_name().is_some_and(|x| x == "system.zmx")
    });
    let mut src =
        String::from("/// (置き場のパス, 中身)\npub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        writeln!(
            src,
            "    (\"/lensbench/{rel}\", include_bytes!({:?})),",
            f.display().to_string()
        )
        .unwrap();
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("samples.rs");
    std::fs::write(out, src).unwrap();
    for d in [
        "tests/glass",
        "examples/lenses",
        "examples/projects",
        "tests/refs",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(d).display());
    }
}
