//! 取った品の STEP のキャッシュ（フォルダ・ファイルの名前）。以前は lensbench-project の `catalog_fill` にあった（XCT-11）

/// 取った品の STEP のキャッシュのフォルダ（[`set_cad_cache_dir`] で決めたもの。無ければ環境変数
/// `LENSBENCH_CAD_CACHE`、なければ OS の設定の場所: Windows は `%APPDATA%\lensbench\cad`、ほかは
/// `$XDG_CACHE_HOME/lensbench/cad` か `~/.cache/lensbench/cad`）
pub fn cad_cache_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(d) = CAD_CACHE_DIR.get() {
        return d.clone();
    }
    // Web 版（ファイルシステムが無い）: 置き場（vfs）の /cad
    if cfg!(target_arch = "wasm32") {
        return PathBuf::from("/cad");
    }
    if let Some(d) = std::env::var_os("LENSBENCH_CAD_CACHE").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("lensbench")
        .join("cad")
}

static CAD_CACHE_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// キャッシュのフォルダを決める（GUI の起動時に設定のフォルダの `cad`。1 回だけ）
pub fn set_cad_cache_dir(d: std::path::PathBuf) {
    let _ = CAD_CACHE_DIR.set(d);
}

/// キャッシュにある品の STEP（gzip の [`cad_cache_path`]、無ければ以前の形の `.step`）。どちらも無ければ None
pub fn cad_cache_find(dir: &std::path::Path, item: &str, url: &str) -> Option<std::path::PathBuf> {
    let p = cad_cache_path(dir, item, url);
    if lensbench_core::vfs::is_file(&p) {
        return Some(p);
    }
    let old = p.with_extension("");
    lensbench_core::vfs::is_file(&old).then_some(old)
}

/// キャッシュのファイル: `<品の名前（ファイル名に使えない文字は _）>-<URL の FNV-1a 64 の 16 進>.step.gz`（gzip の STEP）。
/// 共有の置き場の鍵は `.gz` を除いた名前
pub fn cad_cache_path(dir: &std::path::Path, item: &str, url: &str) -> std::path::PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let name: String = item
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    dir.join(format!("{name}-{h:016x}.step.gz"))
}

/// キャッシュにある品 `item`（STEP の公式の URL `url`）の STEP。プロジェクトの展開に機構部品の詳しい形の
/// ファイルを渡す口（`lensbench_project::Env::step_files`）に、そのまま渡せる（PRJ-21）
pub fn cached_step(item: &str, url: &str) -> Option<std::path::PathBuf> {
    cad_cache_find(&cad_cache_dir(), item, url)
}
