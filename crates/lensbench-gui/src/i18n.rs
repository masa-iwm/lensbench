//! 画面の文字列の言語（国際化）。
//!
//! コードには英語の文字列を書き、[`tr`]・[`trf`] で今の言語の文字列にする。英語の文字列が鍵で、
//! ほかの言語は `lang/` の表（1 行 1 つ、`English<TAB>訳`）。言語を足すときは表を 1 つ足し、[`Lang`] に並べる。
//! 表に無い文字列は英語のまま出す（テスト `all_keys_translated` が、コードの tr・trf の文字列がすべて
//! 表にあるかを確かめる）。数の書式は言語によらず同じ（小数点は「.」）。
//! 日本語の訳語は OpticStudio の日本語版で広く使われる言い回し（カタカナ語を含む）に合わせる。

use lensbench_core::RwLockExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum Lang {
    /// OS の言語に合わせる
    #[default]
    Auto,
    Ja,
    En,
}

impl Lang {
    /// メニューに出す名前（その言語で書く）
    pub(crate) fn label(self) -> &'static str {
        match self {
            Lang::Auto => "Auto / 自動",
            Lang::Ja => "日本語",
            Lang::En => "English",
        }
    }
    pub(crate) const ALL: [Lang; 3] = [Lang::Auto, Lang::Ja, Lang::En];
}

/// 表の言語（Auto を解いた後）。0 日本語、1 英語（既定。テストも英語で動く）
static CURRENT: AtomicU8 = AtomicU8::new(1);

/// 言語を選ぶ（Auto は OS の言語。日本語でなければ英語）
pub(crate) fn set(l: Lang) {
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
    // 計算側の誤りの文も同じ言語に（訳の表は lensbench-lang）
    lensbench_lang::register();
    lensbench_core::i18n::set_lang(if l == Lang::En {
        lensbench_core::i18n::Lang::En
    } else {
        lensbench_core::i18n::Lang::Ja
    });
}

/// 今の言語は日本語か
pub(crate) fn is_ja() -> bool {
    CURRENT.load(Ordering::Relaxed) == 0
}

const JA: &str = include_str!("../lang/ja.tsv");

fn table(src: &'static str) -> HashMap<&'static str, &'static str> {
    src.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect()
}

fn ja() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| table(JA))
}

/// 拡張が足した日本語の訳（[`crate::ext::GuiExtensions::gui_translations`]）
static EXTRA: std::sync::RwLock<Option<HashMap<&'static str, &'static str>>> =
    std::sync::RwLock::new(None);

/// 日本語の訳を足す（`(English, 日本語)`。同梱の表にあるものは同梱の方を使う）
pub(crate) fn add(pairs: &'static [(&'static str, &'static str)]) {
    let mut g = EXTRA.write_ok();
    let m = g.get_or_insert_with(HashMap::new);
    for (en, ja) in pairs {
        m.insert(en, ja);
    }
}

/// 訳す文字列の印（表の鍵の英語。表示するときに `tr` で引く）。何もしないで返す。
/// 網羅の試験は `tr("…")`・`trf("…")` と同じくこの中の文字列も拾う
pub(crate) const fn n_(en: &'static str) -> &'static str {
    en
}

/// 番地を混ぜるだけのハッシュ（鍵は文字列の番地と長さ）
#[derive(Default)]
struct AddrHasher(u64);

impl std::hash::Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.write_u64(u64::from(x));
        }
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

thread_local! {
    /// 同梱の表で引けた訳を英語の番地で（拡張の訳は後から足せるので入れない）
    static BY_ADDR: std::cell::RefCell<
        HashMap<(usize, usize), &'static str, std::hash::BuildHasherDefault<AddrHasher>>,
    > = Default::default();
}

/// 今の言語の文字列
pub fn tr(en: &'static str) -> &'static str {
    if !is_ja() {
        return en;
    }
    // 同じ文字列の定数は同じ番地なので、番地で先に引く（毎フレーム何百回も呼ぶ。文字列をハッシュしない）
    let key = (en.as_ptr() as usize, en.len());
    if let Some(j) = BY_ADDR.with(|m| m.borrow().get(&key).copied()) {
        return j;
    }
    if let Some(j) = ja().get(en) {
        BY_ADDR.with(|m| m.borrow_mut().insert(key, *j));
        return j;
    }
    EXTRA
        .read()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(en).copied()))
        .unwrap_or(en)
}

/// 今の言語の文字列の穴を引数で埋める。英語（鍵）は `{}` を順に、訳は `{}`（順に）か `{0}`・`{1}`…（引数の位置。
/// 語順の違う言語で入れ替える）で書く
pub fn trf(en: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    fill(tr(en), args)
}

/// t の `{}`（順に）と `{N}`（N 番目の引数）を args で埋める。引数の無い穴は空、穴でない `{` はそのまま
fn fill(t: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(t.len() + 16);
    let mut next = 0;
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = &after[..close];
        let k = if inner.is_empty() {
            next += 1;
            Some(next - 1)
        } else {
            inner.parse::<usize>().ok()
        };
        match k {
            Some(k) => {
                if let Some(a) = args.get(k) {
                    out.push_str(&a.to_string());
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 訳の穴: `{}` の数と、`{N}` の番号の並び
#[cfg(test)]
fn slots(t: &str) -> (usize, Vec<usize>) {
    let mut seq = 0;
    let mut pos = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let Some(close) = after.find('}') else { break };
        let inner = &after[..close];
        if inner.is_empty() {
            seq += 1;
        } else if let Ok(k) = inner.parse::<usize>() {
            pos.push(k);
        }
        rest = &after[close + 1..];
    }
    (seq, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ソースの tr("…")・trf("…") の文字列（1 行に書いたもの）
    fn keys_in_source() -> Vec<String> {
        // src の下を全部（app/ などの下のフォルダも）
        let mut files = Vec::new();
        let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    files.push(p);
                }
            }
        }
        let mut keys = Vec::new();
        for path in files {
            // この表の説明（コメント）は除く
            if path.ends_with("i18n.rs") {
                continue;
            }
            let src = lensbench_core::vfs::read_to_string(path).unwrap();
            for pat in ["tr(", "trf(", "n_("] {
                let mut s = src.as_str();
                while let Some(i) = s.find(pat) {
                    // 識別子の途中（str( など）は除く
                    let before = s[..i].chars().last();
                    s = &s[i + pat.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    // 引数（括弧の対まで）の中の文字列を全部拾う。rustfmt で折り返した trf(\n "…") と
                    // tr(if c { "…" } else { "…" }) も。trf の 2 つ目からの引数は表示する値なので、
                    // 文字列で始まる呼び出しは初めの文字列だけ
                    let first_only = s.trim_start().starts_with('"');
                    let mut depth = 1;
                    let mut chars = s.chars().peekable();
                    while let Some(c) = chars.next() {
                        match c {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            '\'' => {
                                // 文字（'"' など）。寿命の印 'a は 1 字で閉じないので読み飛ばさない
                                let mut look = chars.clone();
                                let c1 = look.next();
                                let c2 = look.next();
                                if c1 == Some('\\') {
                                    while chars.next().is_some_and(|x| x != '\'') {}
                                } else if c2 == Some('\'') {
                                    chars.next();
                                    chars.next();
                                }
                            }
                            '"' => {
                                let mut k = String::new();
                                while let Some(c) = chars.next() {
                                    match c {
                                        '"' => break,
                                        '\\' => match chars.next() {
                                            Some('n') => k.push('\n'),
                                            Some('t') => k.push('\t'),
                                            // 行の終わりの \ は次の行の頭の空白ごと読み飛ばす
                                            Some('\n') => {
                                                while chars
                                                    .peek()
                                                    .is_some_and(|c| c.is_whitespace())
                                                {
                                                    chars.next();
                                                }
                                            }
                                            Some(x) => k.push(x),
                                            None => {}
                                        },
                                        _ => k.push(c),
                                    }
                                }
                                keys.push(k);
                                if first_only {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        // 解析の名前の表（tr で引く）
        keys.extend(crate::layout::ANALYSES.iter().map(|a| a.1.to_string()));
        keys.extend(crate::layout::TOOLS.iter().map(|t| t.title.to_string()));
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
        let t = table(JA);
        // 衝突の印が残っていない（表の読み方は TAB の無い行を読み飛ばすので、ここで見る）
        for l in JA.lines() {
            assert!(
                !(l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>") || l == "======="),
                "lang/ja.tsv に衝突の印: {l}"
            );
        }
        // 同じ英語の行が 2 つ無い（取り込みの merge=union で重なりやすい）
        let mut seen = std::collections::HashSet::new();
        for l in JA.lines().filter(|l| !l.starts_with('#')) {
            if let Some((en, _)) = l.split_once('\t') {
                assert!(seen.insert(en), "lang/ja.tsv に同じ英語の行が 2 つ: {en}");
            }
        }
        let keys = keys_in_source();
        assert!(keys.len() > 50, "{}", keys.len());
        let missing: Vec<&String> = keys
            .iter()
            .filter(|k| !t.contains_key(k.as_str()))
            .collect();
        assert!(missing.is_empty(), "lang/ja.tsv に無い: {missing:#?}");
        // 鍵の英語に日本語の文字が無い（訳し忘れの日本語を鍵にしない）
        for k in &keys {
            assert!(
                !k.chars()
                    .any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}')),
                "鍵が日本語: {k}"
            );
        }
        // 表の訳の穴は英語と同じ数。`{N}` で書くなら `{}` と混ぜず、0…n−1 を 1 つずつ
        for (k, v) in &t {
            let n = k.matches("{}").count();
            assert!(slots(k).1.is_empty(), "鍵は {{}} だけ: {k}");
            match slots(v) {
                (m, p) if p.is_empty() => assert_eq!(n, m, "{k}"),
                (0, mut p) => {
                    p.sort();
                    assert_eq!(p, (0..n).collect::<Vec<_>>(), "{k}");
                }
                _ => panic!("訳で {{}} と {{N}} を混ぜている: {k}"),
            }
        }
    }

    #[test]
    fn format() {
        CURRENT.store(0, Ordering::Relaxed);
        let s = trf("{} / {} lenses", &[&3, &5]);
        CURRENT.store(1, Ordering::Relaxed);
        assert_eq!(s, "3 / 5 枚");
        assert_eq!(trf("{} / {} lenses", &[&3, &5]), "3 / 5 lenses");
    }

    /// 訳は `{N}` で引数の順を入れ替えられる。穴でない波括弧はそのまま
    #[test]
    fn positional_slots() {
        assert_eq!(fill("{1} の {0}", &[&"a", &"b"]), "b の a");
        assert_eq!(fill("{} と {}", &[&1, &2]), "1 と 2");
        assert_eq!(fill("{x} {", &[&1]), "{x} {");
        assert_eq!(fill("{2}", &[&1]), "");
        assert_eq!(slots("{1} の {0}"), (0, vec![1, 0]));
    }
}
