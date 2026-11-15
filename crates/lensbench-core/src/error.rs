//! エラー型。対応していない機能は黙って無視せず `NotImplemented` で止める。

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// 対応していない機能を含む
    NotImplemented(String),
    /// 値や構成が不正
    Invalid(String),
    /// ファイルの読込
    Io(String),
    /// 計算を途中でやめた（[`crate::par::CancelToken::cancel`]、GUI で系が変わったとき）
    Cancelled,
}

/// 種類の前置き（英語と、日本語の表示のとき）
const PREFIXES: [(&str, &str); 3] = [
    ("not supported: ", "未対応: "),
    ("invalid: ", "不正: "),
    ("read: ", "読込: "),
];

/// 表示の文（[`crate::i18n`]: 英語の元の文、日本語の表示なら表で訳す）
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ja = crate::i18n::is_ja();
        let pre = |i: usize| if ja { PREFIXES[i].1 } else { PREFIXES[i].0 };
        match self {
            Error::NotImplemented(s) => write!(f, "{}{}", pre(0), crate::i18n::tr(s)),
            Error::Invalid(s) => write!(f, "{}{}", pre(1), crate::i18n::tr(s)),
            Error::Io(s) => write!(f, "{}{}", pre(2), crate::i18n::tr(s)),
            Error::Cancelled => f.write_str(&crate::i18n::tr("cancelled")),
        }
    }
}

/// 誤りを文字列にしたもの s から種類の前置き（英語・日本語）を外し、種類の誤りに戻す（前置きが無ければ None）
pub fn parse_display(s: &str) -> Option<Error> {
    let kinds: [fn(String) -> Error; 3] = [Error::NotImplemented, Error::Invalid, Error::Io];
    PREFIXES.iter().zip(kinds).find_map(|((en, ja), k)| {
        s.strip_prefix(en)
            .or_else(|| s.strip_prefix(ja))
            .map(|m| k(m.to_string()))
    })
}

/// 誤りを文字列にしたもの s から「不正: 」（英語は「invalid: 」）の前置きを外す（無ければそのまま）
pub fn strip_invalid(s: &str) -> &str {
    s.strip_prefix(PREFIXES[1].0)
        .or_else(|| s.strip_prefix(PREFIXES[1].1))
        .unwrap_or(s)
}

impl std::error::Error for Error {}

impl Error {
    /// 種類の名前（serve の誤りの `data.kind`・Python の例外のクラスに使う）: `not_implemented`・`invalid`・`io`・`cancelled`
    pub fn kind(&self) -> &'static str {
        match self {
            Error::NotImplemented(_) => "not_implemented",
            Error::Invalid(_) => "invalid",
            Error::Io(_) => "io",
            Error::Cancelled => "cancelled",
        }
    }

    /// 文を前に足す（種類は変えない）。例: `e.at(sys.surf_label(j))` で「面 87(STANDARD, 膜 THORB): …」
    pub fn at(self, ctx: impl std::fmt::Display) -> Error {
        match self {
            Error::NotImplemented(s) => Error::NotImplemented(format!("{ctx}: {s}")),
            Error::Invalid(s) => Error::Invalid(format!("{ctx}: {s}")),
            Error::Io(s) => Error::Io(format!("{ctx}: {s}")),
            Error::Cancelled => Error::Cancelled,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[macro_export]
macro_rules! notimpl {
    ($($a:tt)*) => { return Err($crate::error::Error::NotImplemented(format!($($a)*))) };
}
#[macro_export]
macro_rules! invalid {
    ($($a:tt)*) => { return Err($crate::error::Error::Invalid(format!($($a)*))) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_round_trip() {
        for e in [
            Error::Invalid("x".into()),
            Error::Io("y".into()),
            Error::NotImplemented("z".into()),
        ] {
            assert_eq!(parse_display(&e.to_string()), Some(e));
        }
        assert_eq!(strip_invalid("不正: a"), "a");
        assert_eq!(strip_invalid("invalid: a"), "a");
        assert_eq!(parse_display("a"), None);
    }
}
