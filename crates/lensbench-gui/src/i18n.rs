//! 画面の文字列の言語（国際化）。
//!
//! コードには日本語の文字列を書き、[`tr`]・[`trf`] で今の言語の文字列にする。日本語の文字列が鍵で、
//! ほかの言語は `lang/` の表（1 行 1 つ、`日本語<TAB>訳`）。言語を足すときは表を 1 つ足し、[`Lang`] に並べる。
//! 表に無い文字列は日本語のまま出す（テスト `all_keys_translated` が、コードの tr・trf の文字列がすべて
//! 表にあるかを確かめる）。数の書式は言語によらず同じ（小数点は「.」）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    /// OS の言語に合わせる
    #[default]
    Auto,
    Ja,
    En,
}

impl Lang {
    /// メニューに出す名前（その言語で書く）
    pub fn label(self) -> &'static str {
        match self {
            Lang::Auto => "Auto / 自動",
            Lang::Ja => "日本語",
            Lang::En => "English",
        }
    }
    pub const ALL: [Lang; 3] = [Lang::Auto, Lang::Ja, Lang::En];
}

/// 表の言語（Auto を解いた後）。0 日本語、1 英語
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// 言語を選ぶ（Auto は OS の言語。日本語でなければ英語）
pub fn set(l: Lang) {
    let l = match l {
        Lang::Auto => {
            let os = sys_locale::get_locale().unwrap_or_default();
            if os.to_lowercase().starts_with("ja") {
                Lang::Ja
            } else {
                Lang::En
            }
        }
        x => x,
    };
    CURRENT.store(if l == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

/// 今の言語は日本語か
pub fn is_ja() -> bool {
    CURRENT.load(Ordering::Relaxed) == 0
}

const EN: &str = include_str!("../lang/en.tsv");

fn table(src: &'static str) -> HashMap<&'static str, &'static str> {
    src.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect()
}

fn en() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| table(EN))
}

/// 拡張が足した英語の訳（[`crate::ext::translations`]）
static EXTRA: std::sync::RwLock<Option<HashMap<&'static str, &'static str>>> =
    std::sync::RwLock::new(None);

/// 英語の訳を足す（同梱の表にあるものは同梱の方を使う）
pub fn add(pairs: &'static [(&'static str, &'static str)]) {
    let mut g = EXTRA.write().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    for (ja, en) in pairs {
        m.insert(ja, en);
    }
}

/// 今の言語の文字列
pub fn tr(ja: &'static str) -> &'static str {
    if is_ja() {
        return ja;
    }
    if let Some(e) = en().get(ja) {
        return e;
    }
    EXTRA
        .read()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(ja).copied()))
        .unwrap_or(ja)
}

/// 今の言語の文字列の `{}` を順に引数で埋める
pub fn trf(ja: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let t = tr(ja);
    let mut out = String::with_capacity(t.len() + 16);
    let mut it = args.iter();
    let mut rest = t;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = it.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ソースの tr("…")・trf("…") の文字列（1 行に書いたもの）
    fn keys_in_source() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut keys = Vec::new();
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            // この表の説明（コメント）は除く
            if path.ends_with("i18n.rs") {
                continue;
            }
            let src = lensbench_core::vfs::read_to_string(path).unwrap();
            for pat in ["tr(", "trf("] {
                let mut s = src.as_str();
                while let Some(i) = s.find(pat) {
                    // 識別子の途中（str( など）は除く
                    let before = s[..i].chars().last();
                    s = &s[i + pat.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    // rustfmt で折り返した trf(\n "…") も拾う。文字列で始まらない呼び出しは除く
                    let t = s.trim_start();
                    let Some(rest) = t.strip_prefix('"') else {
                        continue;
                    };
                    let s2 = rest;
                    let mut k = String::new();
                    let mut chars = s2.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => k.push('\n'),
                                Some(x) => k.push(x),
                                None => {}
                            },
                            _ => k.push(c),
                        }
                    }
                    keys.push(k);
                }
            }
        }
        // 解析の名前の表（tr で引く）
        keys.extend(crate::layout::ANALYSES.iter().map(|a| a.1.to_string()));
        keys.extend(crate::layout_rays::LABELS.iter().map(|a| a.to_string()));
        keys.extend(crate::coat_defs::LABELS.iter().map(|a| a.to_string()));
        keys.extend(crate::tol_catalog::FIELDS.iter().map(|a| a.1.to_string()));
        keys.extend(crate::lens_data::labels().iter().map(|a| a.to_string()));
        keys.sort();
        keys.dedup();
        keys
    }

    #[test]
    fn all_keys_translated() {
        let t = table(EN);
        // 衝突の印が残っていない（表の読み方は TAB の無い行を読み飛ばすので、ここで見る）
        for l in EN.lines() {
            assert!(
                !(l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>") || l == "======="),
                "lang/en.tsv に衝突の印: {l}"
            );
        }
        let keys = keys_in_source();
        assert!(keys.len() > 50, "{}", keys.len());
        let missing: Vec<&String> = keys
            .iter()
            .filter(|k| !t.contains_key(k.as_str()))
            .collect();
        assert!(missing.is_empty(), "lang/en.tsv に無い: {missing:#?}");
        // 表の訳の {} の数は日本語と同じ
        for (k, v) in &t {
            assert_eq!(k.matches("{}").count(), v.matches("{}").count(), "{k}");
        }
    }

    #[test]
    fn format() {
        CURRENT.store(0, Ordering::Relaxed);
        assert_eq!(trf("{} / {} 枚", &[&3, &5]), "3 / 5 枚");
    }
}
