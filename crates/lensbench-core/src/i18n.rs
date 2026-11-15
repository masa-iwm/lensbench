//! Message language (review XCT-20). Messages are written in English; a Japanese translation is shown
//! when the language is Japanese.
//!
//! - Language: [`set_lang`] (the GUI setting), otherwise `LENSBENCH_LANG` (`ja`・`en`), otherwise the OS language
//!   (Japanese if it starts with `ja`, English otherwise).
//! - Translation: `lang/ja.tsv` holds `English template<TAB>Japanese template`. A `{...}` in the English
//!   template matches any text, and the same `{...}` in the Japanese template is replaced by that text
//!   (identical tokens such as `{}` are taken in order). A message that is not in the table is split at
//!   `": "` and each part is translated (the context added by [`crate::Error::at`]); text with no match stays as is.
//! - New messages: write them in English and add the Japanese line to `lang/ja.tsv`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// Message language
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// English (the language the messages are written in)
    En,
    /// Japanese (from `lang/ja.tsv`)
    Ja,
}

/// 0: not decided yet, 1: English, 2: Japanese
static LANG: AtomicU8 = AtomicU8::new(0);

/// Choose the message language (the GUI calls this when its language changes)
pub fn set_lang(l: Lang) {
    LANG.store(if l == Lang::Ja { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(not(target_arch = "wasm32"))]
fn os_locale() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

/// On wasm the GUI chooses the language with [`set_lang`]
#[cfg(target_arch = "wasm32")]
fn os_locale() -> String {
    String::new()
}

/// Language from `LENSBENCH_LANG`, otherwise the OS language
fn detect() -> Lang {
    let env = std::env::var("LENSBENCH_LANG").unwrap_or_default();
    let s = if env.trim().is_empty() {
        os_locale()
    } else {
        env
    };
    if s.trim().to_lowercase().starts_with("ja") {
        Lang::Ja
    } else {
        Lang::En
    }
}

/// The current message language
pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::Ja,
        _ => {
            let l = detect();
            set_lang(l);
            l
        }
    }
}

/// Whether messages are shown in Japanese
pub fn is_ja() -> bool {
    lang() == Lang::Ja
}

/// One piece of a template: literal text or a `{...}` slot
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Slot(String),
}

fn pieces(t: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a..].find('}').map(|b| a + b) else {
            break;
        };
        if a > 0 {
            out.push(Piece::Text(rest[..a].to_string()));
        }
        out.push(Piece::Slot(rest[a..=b].to_string()));
        rest = &rest[b + 1..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest.to_string()));
    }
    out
}

struct Entry {
    en: Vec<Piece>,
    ja: Vec<Piece>,
}

const JA: &str = include_str!("../lang/ja.tsv");

fn table() -> &'static [Entry] {
    static T: OnceLock<Vec<Entry>> = OnceLock::new();
    T.get_or_init(|| {
        JA.lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .map(|(en, ja)| Entry {
                en: pieces(en),
                ja: pieces(ja),
            })
            .collect()
    })
}

/// Match `s` against the pieces `p` (anchored at both ends); the slots' text in order
fn matches(p: &[Piece], s: &str) -> Option<Vec<String>> {
    match p.split_first() {
        None => s.is_empty().then(Vec::new),
        Some((Piece::Text(t), rest)) => matches(rest, s.strip_prefix(t.as_str())?),
        Some((Piece::Slot(_), rest)) => {
            // shortest slot text first; a slot never takes an empty string
            let next = match rest.first() {
                Some(Piece::Text(t)) => Some(t.as_str()),
                _ => None,
            };
            let ends: Vec<usize> = match next {
                Some(t) => s
                    .match_indices(t)
                    .map(|(i, _)| i)
                    .filter(|&i| i > 0)
                    .collect(),
                None if rest.is_empty() => vec![s.len()],
                None => s.char_indices().map(|(i, _)| i).skip(1).collect(),
            };
            ends.into_iter().filter(|&i| i > 0).find_map(|i| {
                let mut v = matches(rest, &s[i..])?;
                v.insert(0, s[..i].to_string());
                Some(v)
            })
        }
    }
}

/// Fill the Japanese template with the slot texts (by slot token, in order for repeated tokens)
fn fill(e: &Entry, vals: Vec<String>) -> String {
    let slots: Vec<&String> =
        e.en.iter()
            .filter_map(|p| match p {
                Piece::Slot(s) => Some(s),
                Piece::Text(_) => None,
            })
            .collect();
    let mut used = vec![false; vals.len()];
    let mut out = String::new();
    for p in &e.ja {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Slot(s) => {
                let i = slots
                    .iter()
                    .enumerate()
                    .position(|(i, x)| *x == s && !used[i])
                    .or_else(|| slots.iter().position(|x| *x == s));
                match i {
                    Some(i) => {
                        used[i] = true;
                        out.push_str(&translate(&vals[i]));
                    }
                    None => out.push_str(s),
                }
            }
        }
    }
    out
}

/// Translate one English message into Japanese with the table (None if no entry matches)
fn lookup(s: &str) -> Option<String> {
    table()
        .iter()
        .find_map(|e| matches(&e.en, s).map(|v| fill(e, v)))
}

/// The Japanese of an English message (the whole message, otherwise each `": "`-separated part)
pub fn translate(s: &str) -> String {
    if let Some(t) = lookup(s) {
        return t;
    }
    match s.split_once(": ") {
        Some((a, b)) => format!("{}: {}", translate(a), translate(b)),
        None => s.to_string(),
    }
}

/// The message in the current language (English as written, Japanese from the table)
pub fn tr(s: &str) -> String {
    if is_ja() {
        translate(s)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(en: &str, ja: &str) -> Entry {
        Entry {
            en: pieces(en),
            ja: pieces(ja),
        }
    }

    #[test]
    fn templates_match_and_fill() {
        let e = entry(
            "glass {name} is not in the catalogs (surface {s})",
            "ガラス {name} がカタログにない（面 {s}）",
        );
        let v = matches(&e.en, "glass N-SK16 is not in the catalogs (surface 1)").unwrap();
        assert_eq!(fill(&e, v), "ガラス N-SK16 がカタログにない（面 1）");
        // repeated {} in order, reordered in Japanese by name is not possible, but order is kept
        let e = entry("{} must be at most {}", "{} は {} 以下に");
        let v = matches(&e.en, "rings must be at most 50").unwrap();
        assert_eq!(fill(&e, v), "rings は 50 以下に");
        assert!(matches(&e.en, "nothing").is_none());
    }

    #[test]
    fn every_line_has_the_same_slots() {
        for (n, l) in JA.lines().enumerate() {
            if l.trim().is_empty() || l.starts_with('#') {
                continue;
            }
            let (en, ja) = l
                .split_once('\t')
                .unwrap_or_else(|| panic!("lang/ja.tsv line {}: no tab", n + 1));
            let slots = |t: &str| {
                let mut v: Vec<String> = pieces(t)
                    .into_iter()
                    .filter_map(|p| match p {
                        Piece::Slot(s) => Some(s),
                        Piece::Text(_) => None,
                    })
                    .collect();
                v.sort();
                v
            };
            assert_eq!(slots(en), slots(ja), "lang/ja.tsv line {}: {l}", n + 1);
            assert!(
                !pieces(en)
                    .windows(2)
                    .any(|w| matches!(w, [Piece::Slot(_), Piece::Slot(_)])),
                "lang/ja.tsv line {}: two slots side by side can't be matched",
                n + 1
            );
        }
    }

    #[test]
    fn kinds_are_translated() {
        assert_eq!(translate("invalid"), "不正");
        assert_eq!(
            translate("surface 3: invalid"),
            "面 3: 不正",
            "context parts are translated one by one"
        );
        // 表の行（埋めた値も訳す）
        assert_eq!(
            translate("set 3: no wavelength 2 (1 to 1)"),
            "組 3: 波長 2 はありません（1〜1）"
        );
        assert_eq!(
            translate("the ray stopped (wavelength 1, pupil (0.000, 1.000), surface 4)"),
            "光線が止まった（波長 1、瞳 (0.000, 1.000)、面 4）"
        );
    }
}
