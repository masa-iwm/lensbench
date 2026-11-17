//! clap の引数の誤りを日本語の文にする。
//!
//! 文言の方針（11/15 決定）: 誤りの文は英語が元で、日本語の表示（[`lensbench_core::i18n`]）では訳す。clap の誤りも
//! 日本語の表示では訳して、1 つの画面に 2 言語が混じらないようにする。help・version の表示はそのまま clap に任せる。

use clap::error::{ContextKind, ContextValue, ErrorKind};

/// 文脈の値を文字列に（並びは「、」でつなぐ）
fn ctx(e: &clap::Error, k: ContextKind) -> Option<String> {
    match e.get(k)? {
        ContextValue::String(s) => Some(s.clone()),
        ContextValue::Strings(v) => Some(v.join("、")),
        ContextValue::StyledStr(s) => Some(s.to_string()),
        ContextValue::StyledStrs(v) => Some(
            v.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("、"),
        ),
        ContextValue::Number(n) => Some(n.to_string()),
        ContextValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// 値を読めなかった理由（Rust の標準の解析の誤りは日本語に、ほかはそのまま）
fn source_ja(e: &clap::Error) -> Option<String> {
    let s = std::error::Error::source(e)?;
    if s.downcast_ref::<std::num::ParseIntError>().is_some() {
        return Some("整数でない（または範囲の外）".into());
    }
    if s.downcast_ref::<std::num::ParseFloatError>().is_some() {
        return Some("数でない".into());
    }
    if s.downcast_ref::<std::str::ParseBoolError>().is_some() {
        return Some("true・false でない".into());
    }
    Some(lensbench_core::i18n::translate(&s.to_string()))
}

/// 「（X のこと？）」の添え書き
fn suggest(e: &clap::Error) -> String {
    ctx(e, ContextKind::SuggestedArg)
        .or_else(|| ctx(e, ContextKind::SuggestedSubcommand))
        .or_else(|| ctx(e, ContextKind::SuggestedValue))
        .map(|s| format!("（{s} のこと？）"))
        .unwrap_or_default()
}

/// clap の誤り e の日本語の文。help・version の表示（誤りでないもの）は None
pub fn message(e: &clap::Error) -> Option<String> {
    let arg = || ctx(e, ContextKind::InvalidArg).unwrap_or_default();
    let val = || ctx(e, ContextKind::InvalidValue).unwrap_or_default();
    Some(match e.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => return None,
        ErrorKind::InvalidValue => {
            let valid = ctx(e, ContextKind::ValidValue)
                .map(|v| format!("（{v} のどれか）"))
                .unwrap_or_default();
            if val().is_empty() {
                format!("{} に値がない{valid}", arg())
            } else {
                format!("{} に「{}」は使えない{valid}{}", arg(), val(), suggest(e))
            }
        }
        ErrorKind::UnknownArgument => format!("知らない引数 {}{}", arg(), suggest(e)),
        ErrorKind::InvalidSubcommand => {
            let sub = ctx(e, ContextKind::InvalidSubcommand).unwrap_or_else(arg);
            format!("知らないコマンド {sub}{}", suggest(e))
        }
        ErrorKind::NoEquals => format!("{} は「=」で値をつなぐ", arg()),
        ErrorKind::ValueValidation if e.get(ContextKind::InvalidArg).is_some() => {
            let why = source_ja(e).map(|s| format!(": {s}")).unwrap_or_default();
            format!("{} の値「{}」が不正{why}", arg(), val())
        }
        ErrorKind::TooManyValues => format!("{} の値が多すぎる（「{}」）", arg(), val()),
        ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
            let want = ctx(e, ContextKind::ExpectedNumValues)
                .or_else(|| ctx(e, ContextKind::MinValues))
                .map(|n| format!("（{n} 個要る）"))
                .unwrap_or_default();
            format!("{} の値の数が合わない{want}", arg())
        }
        ErrorKind::ArgumentConflict => match ctx(e, ContextKind::PriorArg) {
            Some(p) => format!("{} と {p} は一緒に使えない", arg()),
            None => format!("{} は一緒に使えない引数がある", arg()),
        },
        ErrorKind::MissingRequiredArgument => format!("要る引数がない: {}", arg()),
        ErrorKind::MissingSubcommand => "コマンドがない".into(),
        ErrorKind::InvalidUtf8 => "引数が UTF-8 でない".into(),
        // 自前の文（解析の設定の誤りなど、Command::error で作ったもの）は 1 行目をそのまま
        _ => {
            let s = e.to_string();
            let first = s.lines().next().unwrap_or("");
            first.strip_prefix("error: ").unwrap_or(first).to_string()
        }
    })
}

/// 引数の誤りで終わる: 英語の表示は clap の文のまま、日本語の表示は訳して終了コード 2（help・version は clap が出す）
pub fn fail(e: clap::Error) -> ! {
    match message(&e).filter(|_| lensbench_core::i18n::is_ja()) {
        Some(m) => {
            crate::error::report("lensbench", &crate::error::AppError::usage(m));
            std::process::exit(2)
        }
        None => e.exit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Arg, Command};

    fn cmd() -> Command {
        Command::new("t")
            .arg(
                Arg::new("rings")
                    .long("rings")
                    .value_parser(clap::value_parser!(usize)),
            )
            .arg(Arg::new("dir").long("dir").value_parser(["tan", "sag"]))
    }

    fn err(args: &[&str]) -> String {
        let e = cmd().try_get_matches_from(args).unwrap_err();
        message(&e).unwrap()
    }

    #[test]
    fn japanese_messages() {
        assert_eq!(
            err(&["t", "--rings", "x"]),
            "--rings <rings> の値「x」が不正: 整数でない（または範囲の外）"
        );
        let m = err(&["t", "--dir", "both"]);
        assert!(
            m.starts_with("--dir <dir> に「both」は使えない（tan、sag のどれか）"),
            "{m}"
        );
        let m = err(&["t", "--ring", "3"]);
        assert!(m.starts_with("知らない引数 --ring"), "{m}");
        assert!(m.contains("--rings のこと？"), "{m}");
        let e = Command::new("t")
            .subcommand(Command::new("spot"))
            .try_get_matches_from(["t", "spto"])
            .unwrap_err();
        assert!(message(&e)
            .unwrap()
            .starts_with("知らないコマンド spto（spot のこと？）"));
        let e = cmd().try_get_matches_from(["t", "--help"]).unwrap_err();
        assert!(message(&e).is_none());
        let e = cmd().error(ErrorKind::ValueValidation, "設定 x は数に");
        assert_eq!(message(&e).unwrap(), "設定 x は数に");
    }
}
