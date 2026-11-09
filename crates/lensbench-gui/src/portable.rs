//! 設定の保存先: OS の場所（eframe の既定、Windows は `%APPDATA%\lensbench\data\app.ron`）か、exe の隣の
//! ポータブル（`settings.ron`）。ポータブルでは、設定の中のフォルダ・ファイルのパスを、設定のフォルダの中なら
//! そこからの相対パス（区切りは `/`）で書き、読むときに戻す。
//!
//! 保存先の順: `--settings FILE` → exe の隣に `lensbench.portable` か `settings.ron` があればポータブル →
//! OS の場所。ポータブルのフォルダに書けなければ、注意を出して OS の場所にする。
// Web 版では設定の保存先を選ばない（ブラウザの保存領域）ので、選ぶ側は使わない
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// ポータブルの印のファイル
pub const MARKER: &str = "lensbench.portable";
/// ポータブルの設定のファイル
pub const FILE: &str = "settings.ron";

/// 決めた保存先
#[derive(Debug, Clone, PartialEq)]
pub struct Store {
    /// 設定のファイル（None なら eframe の既定の OS の場所）
    pub file: Option<PathBuf>,
    /// 相対パスの基準（ポータブル・`--settings` のときの設定のフォルダ）
    pub base: Option<PathBuf>,
    /// 起動時に出す知らせ
    pub notice: Option<Notice>,
}

/// 起動時の知らせ（画面の言語で出すので、文は [`Notice::text`] で作る）
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// OS の場所の設定を取り込んだ
    Imported(PathBuf),
    /// ポータブルのフォルダに書けないので OS の場所にした
    ReadOnly(PathBuf),
}

impl Notice {
    pub fn text(&self) -> String {
        match self {
            Notice::Imported(p) => crate::i18n::trf("{} の設定を取り込みました", &[&p.display()]),
            Notice::ReadOnly(p) => crate::i18n::trf(
                "{} に書けないので、設定は OS の場所に保存します",
                &[&p.display()],
            ),
        }
    }
}

static STORE: OnceLock<(Option<PathBuf>, Option<PathBuf>)> = OnceLock::new();

/// 相対パスの基準（[`set`] の後）
pub fn base() -> Option<&'static Path> {
    STORE.get().and_then(|b| b.1.as_deref())
}

/// 設定のファイル（OS の場所なら eframe の既定の場所）
pub fn location() -> Option<PathBuf> {
    STORE.get().and_then(|b| b.0.clone()).or_else(os_file)
}

/// 決めた保存先を覚える（1 回だけ）
pub fn set(s: &Store) {
    STORE.set((s.file.clone(), s.base.clone())).ok();
}

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// フォルダに書けるか（一時ファイルを作って消す）
fn writable(d: &Path) -> bool {
    let p = d.join(format!(".lensbench-write-test-{}", std::process::id()));
    let ok = std::fs::write(&p, b"").is_ok();
    std::fs::remove_file(&p).ok();
    ok
}

/// 保存先を決める。`os_file` は OS の場所の設定のファイル（取り込みの元、無ければ None）
pub fn choose(settings: Option<PathBuf>, exe: Option<&Path>, os_file: Option<&Path>) -> Store {
    if let Some(f) = settings {
        let base = f.parent().map(Path::to_path_buf);
        return Store {
            file: Some(f),
            base,
            notice: None,
        };
    }
    let Some(d) = exe else {
        return Store {
            file: None,
            base: None,
            notice: None,
        };
    };
    let file = d.join(FILE);
    if !d.join(MARKER).is_file() && !file.is_file() {
        return Store {
            file: None,
            base: None,
            notice: None,
        };
    }
    if !writable(d) {
        return Store {
            file: None,
            base: None,
            notice: Some(Notice::ReadOnly(d.to_path_buf())),
        };
    }
    // 初めてのポータブル: OS の場所の設定があれば写す
    let mut notice = None;
    if !file.is_file() {
        if let Some(src) = os_file.filter(|p| p.is_file()) {
            if std::fs::copy(src, &file).is_ok() {
                notice = Some(Notice::Imported(src.to_path_buf()));
            }
        }
    }
    Store {
        file: Some(file),
        base: Some(d.to_path_buf()),
        notice,
    }
}

/// OS の場所の設定のファイル（eframe と同じ決め方）
pub fn os_file() -> Option<PathBuf> {
    // Web 版はファイルでなくブラウザの保存領域（eframe が決める）
    #[cfg(target_arch = "wasm32")]
    return None;
    #[cfg(not(target_arch = "wasm32"))]
    eframe::storage_dir("lensbench").map(|d| d.join("app.ron"))
}

/// `..` と `.` を畳んだパス（ファイルは見ない）
fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// 保存する形: `base` の中なら相対（区切り `/`）、外は元のまま
pub fn to_stored(p: &str, base: &Path) -> String {
    let path = Path::new(p);
    if p.is_empty() || !path.is_absolute() {
        return p.to_string();
    }
    match clean(path).strip_prefix(clean(base)) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => p.to_string(),
    }
}

/// 読んだ形を使う形へ: 相対なら `base` からの絶対パス
pub fn from_stored(p: &str, base: &Path) -> String {
    let path = Path::new(p);
    if p.is_empty() || path.is_absolute() {
        return p.to_string();
    }
    let mut out = base.to_path_buf();
    for part in p.split(['/', '\\']).filter(|s| !s.is_empty()) {
        out.push(part);
    }
    clean(&out).to_string_lossy().into_owned()
}

/// 設定が空のときの既定のカタログ: `dir` の `catalogs/glass`（硝材）と `catalogs/lens`・`catalogs/items`（レンズ）
pub fn default_catalogs(dir: &Path) -> (Vec<String>, Vec<String>) {
    let c = dir.join("catalogs");
    let has = |n: &str| {
        c.join(n)
            .is_dir()
            .then(|| c.join(n).to_string_lossy().into_owned())
    };
    (
        has("glass").into_iter().collect(),
        ["lens", "items"].iter().filter_map(|n| has(n)).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_round_trip() {
        let tmp = std::env::temp_dir().join(format!("lbport-{}", std::process::id()));
        let base = tmp.join("app");
        for (p, want) in [
            (
                base.join("catalogs").join("items"),
                "catalogs/items".to_string(),
            ),
            (base.clone(), ".".to_string()),
            (
                tmp.join("other").join("x.zmx"),
                tmp.join("other")
                    .join("x.zmx")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ] {
            let s = p.to_string_lossy().into_owned();
            let st = to_stored(&s, &base);
            assert_eq!(st, want);
            assert_eq!(PathBuf::from(from_stored(&st, &base)), clean(&p));
        }
        // 相対のまま書かれた値・空は変えない
        assert_eq!(to_stored("rel/x", &base), "rel/x");
        assert_eq!(to_stored("", &base), "");
        // フォルダごと移しても、相対の値は新しい場所で解ける
        let moved = tmp.join("moved");
        assert_eq!(
            PathBuf::from(from_stored("catalogs/items", &moved)),
            moved.join("catalogs").join("items")
        );
    }

    #[test]
    fn choose_store() {
        let tmp = std::env::temp_dir().join(format!("lbchoose-{}", std::process::id()));
        let exe = tmp.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        // 印が無ければ OS の場所
        assert_eq!(choose(None, Some(&exe), None).file, None);
        // 印があればポータブル、OS の場所の設定を写す
        std::fs::write(exe.join(MARKER), b"").unwrap();
        let os = tmp.join("app.ron");
        std::fs::write(&os, b"(\n)").unwrap();
        let s = choose(None, Some(&exe), Some(&os));
        assert_eq!(s.file, Some(exe.join(FILE)));
        assert_eq!(s.base, Some(exe.clone()));
        assert!(s.notice.is_some());
        assert!(exe.join(FILE).is_file());
        // 2 回目は写さない
        assert!(choose(None, Some(&exe), Some(&os)).notice.is_none());
        // --settings が先
        let f = tmp.join("my").join("s.ron");
        let s = choose(Some(f.clone()), Some(&exe), None);
        assert_eq!((s.file, s.base), (Some(f), Some(tmp.join("my"))));
        // 既定のカタログ
        std::fs::create_dir_all(exe.join("catalogs/glass")).unwrap();
        std::fs::create_dir_all(exe.join("catalogs/items")).unwrap();
        let (g, l) = default_catalogs(&exe);
        assert_eq!(g.len(), 1);
        assert_eq!(
            l,
            vec![exe
                .join("catalogs")
                .join("items")
                .to_string_lossy()
                .into_owned()]
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}
