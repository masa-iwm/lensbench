//! Message language (review XCT-20). Messages are written in English; a Japanese translation is shown
//! when the language is Japanese.
//!
//! - Language: [`set_lang`] (the GUI setting), otherwise `LENSBENCH_LANG` (`ja`・`en`), otherwise the OS language
//!   (Japanese if it starts with `ja`, English otherwise).
//! - Translation: `lang/ja.tsv` holds `English template<TAB>Japanese template`. A `{...}` in the English
//!   template matches any text, and the same `{...}` in the Japanese template is replaced by that text
//!   (identical tokens such as `{}` are taken in order). Only the text slots `{e}` (a nested message),
//!   `{why}`・`{hint}` (an added note) and `{what}` (a label) are translated again; other slot values (names,
//!   numbers, file names) are kept as they are. Only `{e}` and `{why}` may hold `": "`. When several lines match, the one with the most
//!   literal text wins. A message that is not in the table is split at
//!   `": "` and each part is translated (the context added by [`crate::Error::at`]); text with no match stays as is.
//! - New messages: write them in English and add the Japanese line to `lang/ja.tsv`. Crates outside this
//!   repository's public part keep their own table and add it with [`add_table`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{OnceLock, RwLock};

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

/// The table lines with a tab: `(in an "# items" section, line)`. Lines of an `# items` section are names
/// (tolerance items such as `{name} decenter x`) that only [`translate_item`] uses: their templates
/// match any head, so they must not take part of a message
fn table_lines(text: &str) -> impl Iterator<Item = (bool, &str)> + '_ {
    let mut items = false;
    text.lines().filter_map(move |l| {
        if l.starts_with('#') {
            items = l.starts_with("# items");
            return None;
        }
        (!l.trim().is_empty() && l.contains('\t')).then_some((items, l))
    })
}

fn entries(text: &str, items: bool) -> impl Iterator<Item = Entry> + '_ {
    table_lines(text)
        .filter(move |(i, _)| *i == items)
        .filter_map(|(_, l)| l.split_once('\t'))
        .map(|(en, ja)| Entry {
            en: pieces(en),
            ja: pieces(ja),
        })
}

/// The table: `lang/ja.tsv` and the tables added with [`add_table`]
fn table() -> &'static RwLock<Vec<Entry>> {
    static T: OnceLock<RwLock<Vec<Entry>>> = OnceLock::new();
    T.get_or_init(|| RwLock::new(entries(JA, false).collect()))
}

/// The names of the `# items` sections ([`translate_item`])
fn item_table() -> &'static RwLock<Vec<Entry>> {
    static T: OnceLock<RwLock<Vec<Entry>>> = OnceLock::new();
    T.get_or_init(|| RwLock::new(entries(JA, true).collect()))
}

/// Add a table of the same format as `lang/ja.tsv` (an exe that adds plugins adds the Japanese of their
/// messages at start-up). Check it with [`check_table`] in a test
pub fn add_table(text: &'static str) {
    table()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .extend(entries(text, false));
    item_table()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .extend(entries(text, true));
    whole_lines()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .extend(lines(text));
}

fn lines(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    table_lines(text)
        .filter(|(i, _)| !i)
        .filter_map(|(_, l)| l.split_once('\t'))
        .map(|(en, ja)| (en.to_string(), ja.to_string()))
}

/// The table by whole English line for [`translate_doc`] (later lines win). The help of every command is
/// translated at each start, so this is a hash lookup rather than a pass over the templates
fn whole_lines() -> &'static RwLock<HashMap<String, String>> {
    static T: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    T.get_or_init(|| RwLock::new(lines(JA).collect()))
}

/// Check a table: each line has a tab, the same `{…}` in English and Japanese, unnamed `{}` in the same
/// order, and no two slots side by side. The errors name the line
pub fn check_table(text: &str) -> Result<(), String> {
    for (n, l) in text.lines().enumerate() {
        if l.trim().is_empty() || l.starts_with('#') {
            continue;
        }
        let at = |m: &str| format!("line {}: {m}: {l}", n + 1);
        let (en, ja) = l.split_once('\t').ok_or_else(|| at("no tab"))?;
        let slots = |t: &str, unnamed: bool| {
            let mut v: Vec<String> = pieces(t)
                .into_iter()
                .filter_map(|p| match p {
                    Piece::Slot(s) if !unnamed || s == "{}" || s.starts_with("{:") => Some(s),
                    _ => None,
                })
                .collect();
            if !unnamed {
                v.sort();
            }
            v
        };
        if slots(en, false) != slots(ja, false) {
            return Err(at("different slots"));
        }
        // 名前の無い {}・{:..} は引数の順に埋まるので、英語と日本語で順を変えない
        if slots(en, true) != slots(ja, true) {
            return Err(at("unnamed slots in a different order"));
        }
        if pieces(en)
            .windows(2)
            .any(|w| matches!(w, [Piece::Slot(_), Piece::Slot(_)]))
        {
            return Err(at("two slots side by side can't be matched"));
        }
    }
    Ok(())
}

/// Match `s` against the pieces `p` (anchored at both ends); the slots' text in order
fn matches(p: &[Piece], s: &str) -> Option<Vec<String>> {
    matches_at(p, s, false)
}

/// [`matches`]; `open` is whether the text just before ends with "("
fn matches_at(p: &[Piece], s: &str, open: bool) -> Option<Vec<String>> {
    match p.split_first() {
        None => s.is_empty().then(Vec::new),
        Some((Piece::Text(t), rest)) => {
            matches_at(rest, s.strip_prefix(t.as_str())?, t.ends_with('('))
        }
        Some((Piece::Slot(name), rest)) => {
            // shortest slot text first; a slot never takes an empty string, except a whole
            // parenthesis "({})" (an empty list: "no state x ()")
            let next = match rest.first() {
                Some(Piece::Text(t)) => Some(t.as_str()),
                _ => None,
            };
            let empty = open && next.is_some_and(|t| t.starts_with(')'));
            let ends: Vec<usize> = match next {
                Some(t) => s
                    .match_indices(t)
                    .map(|(i, _)| i)
                    .filter(|&i| i > 0 || empty)
                    .collect(),
                None if rest.is_empty() => vec![s.len()],
                None => s.char_indices().map(|(i, _)| i).skip(1).collect(),
            };
            // 値の欄は「: 」を跨がない（跨ぐのは入れ子の誤り {e} と添え書き {why} だけ。「surface 3: cancelled」は「surface {}」に
            // 当てず、「: 」で分けて訳す）
            let nested = name == "{e}" || name == "{why}";
            // ラベルの欄（{what}・{hint}）は句（「incidence angle」）が入るので長い方から。ほかは短い方から
            let mut ends = ends;
            if matches!(name.to_ascii_lowercase().as_str(), "{what}" | "{hint}") {
                ends.reverse();
            }
            ends.into_iter()
                .filter(|&i| {
                    let colon =
                        s[..i].contains(": ") || (s[..i].ends_with(':') && s[i..].starts_with(' '));
                    (i > 0 || empty) && (nested || !colon)
                })
                .find_map(|i| {
                    let mut v = matches_at(rest, &s[i..], false)?;
                    v.insert(0, s[..i].to_string());
                    Some(v)
                })
        }
    }
}

/// A slot whose text is itself a message to translate: `{e}`, `{why}`, `{hint}`, `{what}` (any case)
fn is_text_slot(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "{e}" | "{why}" | "{hint}" | "{what}"
    )
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
                        // 文の欄（`{e}`・`{why}`・`{hint}`・`{what}`）だけ訳す。名前・数などの値はそのまま
                        // （ファイル名「surface 3.zmx」を「面 3.zmx」にしない）
                        if is_text_slot(s) {
                            out.push_str(&translate(&vals[i]));
                        } else {
                            out.push_str(&vals[i]);
                        }
                    }
                    None => out.push_str(s),
                }
            }
        }
    }
    out
}

/// Literal text length of a template (a longer one is more specific)
fn literal_len(p: &[Piece]) -> usize {
    p.iter()
        .map(|x| match x {
            Piece::Text(t) => t.len(),
            Piece::Slot(_) => 0,
        })
        .sum()
}

/// Translate one English message into Japanese with the table (None if no entry matches). When several
/// entries match, the one with the most literal text wins (`input {}` loses to `input {key} must be …`)
fn lookup(s: &str) -> Option<String> {
    lookup_in(table(), s)
}

fn lookup_in(t: &RwLock<Vec<Entry>>, s: &str) -> Option<String> {
    let t = t.read().unwrap_or_else(|e| e.into_inner());
    t.iter()
        .filter_map(|e| matches(&e.en, s).map(|v| (literal_len(&e.en), e, v)))
        .max_by_key(|(n, _, _)| *n)
        .map(|(_, e, v)| fill(e, v))
}

/// The Japanese of an English message (the whole message, otherwise each `": "`-separated part,
/// otherwise, when every sentence has a line, each sentence: a note of several sentences joined by `". "`
/// becomes sentences joined by "。")
pub fn translate(s: &str) -> String {
    if let Some(t) = lookup(s) {
        return t;
    }
    match s.split_once(": ") {
        Some((a, b)) => format!("{}: {}", translate(a), translate(b)),
        // 文ごとに表の行があるときだけ（英語の文が混ざったまま「。」でつながない）
        None if s.contains(". ") => s
            .split(". ")
            .map(lookup)
            .collect::<Option<Vec<_>>>()
            .map_or_else(|| s.to_string(), |v| v.join("。")),
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

/// The Japanese of a line only when a table line is exactly that text (descriptions: an
/// English text with no line of its own must not be taken apart by message templates such as `surface {}`)
fn exact(s: &str) -> Option<String> {
    let t = whole_lines().read().unwrap_or_else(|e| e.into_inner());
    t.get(s).cloned()
}

/// Japanese of a description of several lines (help, analysis settings): each line from its own table line
/// (a trailing ` [unit]` is kept as it is); lines with no line of their own stay English
pub fn translate_doc(s: &str) -> String {
    let line = |l: &str| {
        exact(l)
            .or_else(|| {
                let (a, u) = l.rsplit_once(" [").filter(|(_, u)| u.ends_with(']'))?;
                exact(a).map(|j| format!("{j} [{u}"))
            })
            .unwrap_or_else(|| l.to_string())
    };
    s.split('\n').map(line).collect::<Vec<_>>().join("\n")
}

/// The Japanese of a name from an `# items` section of the table (a tolerance item such as
/// `L1 group decenter x`); other text stays as it is
pub fn translate_item(s: &str) -> String {
    lookup_in(item_table(), s).unwrap_or_else(|| s.to_string())
}

/// [`translate_item`] in the current language
pub fn tr_item(s: &str) -> String {
    if is_ja() {
        translate_item(s)
    } else {
        s.to_string()
    }
}

/// [`translate_doc`] in the current language
pub fn tr_doc(s: &str) -> String {
    if is_ja() {
        translate_doc(s)
    } else {
        s.to_string()
    }
}

/// A warning line for the terminal: "warning: …" in English, "注意: …" (translated) in Japanese
pub fn warning(w: impl AsRef<str>) -> String {
    tr(&format!("warning: {e}", e = w.as_ref()))
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
            "硝材 {name} がカタログにありません（面 {s}）",
        );
        let v = matches(&e.en, "glass N-SK16 is not in the catalogs (surface 1)").unwrap();
        assert_eq!(fill(&e, v), "硝材 N-SK16 がカタログにありません（面 1）");
        // repeated {} in order, reordered in Japanese by name is not possible, but order is kept
        let e = entry("{} must be at most {}", "{} は {} 以下に");
        let v = matches(&e.en, "rings must be at most 50").unwrap();
        assert_eq!(fill(&e, v), "rings は 50 以下に");
        assert!(matches(&e.en, "nothing").is_none());
    }

    #[test]
    fn every_line_has_the_same_slots() {
        check_table(JA).unwrap_or_else(|e| panic!("lang/ja.tsv {e}"));
        assert!(check_table("a {} b {:.3}\tA {:.3} B {}").is_err());
        assert!(check_table("a {}{}\tA {}{}").is_err());
    }

    #[test]
    fn docs_translate_only_whole_lines() {
        // 表に行の無い英語の説明は誤りの文の型（`surface {}` など）で崩さない
        assert_eq!(translate_doc("surface tilt"), "surface tilt");
        assert_eq!(
            translate_doc("Number of quadrature points\nno such line [mm]"),
            "求積の点の数\nno such line [mm]"
        );
        assert_eq!(
            translate_doc("Propagation distance [mm]"),
            "伝搬の距離 [mm]"
        );
    }

    #[test]
    fn tolerance_item_names() {
        assert_eq!(translate_item("L1 group decenter x"), "L1 群の偏心x");
        assert_eq!(translate_item("L1 decenter x"), "L1 偏心x");
        assert_eq!(translate_item("lens 2 thickness 3"), "lens 2 厚み3");
        assert_eq!(translate_item("L1 R2"), "L1 R2");
        // 項目名の型は誤りの文の経路では使わない（先頭が何でも当たるので）
        assert_eq!(translate("no such glass index 3"), "no such glass index 3");
    }

    #[test]
    fn notes_of_several_sentences() {
        assert_eq!(
            translate("the seat is the mount end (not known from the STEP). inner thread SM1 (for elements of diameter 25.4 mm)"),
            "座の位置はマウントの端（STEP から分かりません）。内側のねじ SM1（入る光学素子の径 25.4 mm）"
        );
        // 訳の無い文が混ざれば英語のまま
        let s = "the seat is the mount end (not known from the STEP). no such sentence";
        assert_eq!(translate(s), s);
        assert_eq!(
            translate("outer diameter: catalog 30 mm, STEP 40.64 mm (difference 10.64 mm)"),
            "外径: カタログ 30 mm、STEP 40.64 mm（差 10.64 mm）"
        );
    }

    #[test]
    fn swap_progress_eta_in_seconds() {
        assert!(translate("screening 3/10 (5 s left)").ends_with("3/10（残り 5 秒）"));
    }

    #[test]
    fn surface_label_coating_is_translated() {
        assert_eq!(
            translate("surface 3(STANDARD, N-BK7, coating AR, 「L1」)"),
            "面 3(STANDARD, N-BK7, コーティング AR, 「L1」)"
        );
        assert_eq!(translate("surface 2(STANDARD)"), "面 2(STANDARD)");
    }

    #[test]
    fn kinds_are_translated() {
        assert_eq!(translate("cancelled"), "中断しました");
        assert_eq!(
            translate("surface 3: cancelled"),
            "面 3: 中断しました",
            "context parts are translated one by one"
        );
        // 埋めた値は訳さない（入れ子の誤り {e} だけ訳す）
        assert_eq!(
            translate("surface 3.zmx: no such file"),
            "surface 3.zmx: ファイルがありません"
        );
        assert_eq!(translate("set 2: surface 3"), "組 2: 面 3");
        // 表の行。短い型の「surface {}」より長い型が勝つ
        assert_eq!(
            translate("surface 3 can't be written (1 to 9)"),
            "面 3 は書けません（1〜9）"
        );
        // 注意の行: 「注意: 」を付け、中の文も訳す
        assert_eq!(
            translate("warning: removed multi-configuration row 3.t"),
            "注意: 多重構成の行 3.t を外しました"
        );
        // 札（{what}）も訳す: 「state 2」→「状態 2」
        assert!(translate(
            "state 2: 7.t differs from the loaded system (1, loaded system 2), so set is not applied"
        )
        .starts_with("状態 2: 7.t は"));
        assert_eq!(
            translate("lensbench run: a.py (Python: python3, threads 4)"),
            "lensbench run: a.py（Python: python3、スレッド 4）"
        );
        // 空の一覧の括弧「()」は空の欄に当てる
        assert_eq!(
            translate("no state nosuch ()"),
            "状態 nosuch はありません（）"
        );
        assert_eq!(
            translate("set 3: no wavelength 2 (1 to 1)"),
            "組 3: 波長 2 はありません（1〜1）"
        );
        assert_eq!(
            translate("the ray stopped (wavelength 1, pupil (0.000, 1.000), surface 4)"),
            "光線が止まりました（波長 1、瞳 (0.000, 1.000)、面 4）"
        );
        // ラベルの欄 {what} に 2 語以上の句（#381 のレビュー）
        assert_eq!(
            translate("surface 3: Grid Sag grid columns 1 (an integer 2 to 16000000)"),
            "面 3: Grid Sag のグリッドの列の数 1（2〜16000000 の整数）"
        );
        assert_eq!(
            translate("number of zmx configurations MNUM 1.5 (an integer 1 to 1000)"),
            "zmx の構成の数 MNUM 1.5（1〜1000 の整数）"
        );
        assert_eq!(
            translate(
                "coating table AR: incidence angle 95.0000 deg is outside the table range 0 to 89"
            ),
            "コーティングの表 AR: 入射角 95.0000 deg が表の範囲 0〜89 の外"
        );
    }
}
