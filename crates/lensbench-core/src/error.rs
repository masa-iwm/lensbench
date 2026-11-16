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
    /// 機械で見分ける符号（[`code`] の定数）を付けた誤り（[`Error::with_code`]。種類・文は中の誤りのもの）
    Coded(&'static str, Box<Error>),
}

/// 誤りの符号（[`Error::code`]）。serve の `error.data.code`・Python の例外の `code`・試験の判定に使う。
/// 符号の付いていない誤りの符号は種類の名前（[`Error::kind`]）
pub mod code {
    /// ファイルが無い（種類 io）
    pub const FILE_NOT_FOUND: &str = "file.not_found";
    /// ファイルでなくフォルダ（種類 io）
    pub const IS_FOLDER: &str = "file.is_folder";
    /// ガラスがカタログに無い・ガラスカタログが見つからない（種類 invalid）
    pub const GLASS_NOT_FOUND: &str = "glass.not_found";
    /// プロジェクトに無い状態の名前（種類 invalid）
    pub const STATE_NOT_FOUND: &str = "project.state_not_found";
    /// プロジェクトに無いブロックの id（種類 invalid）
    pub const BLOCK_NOT_FOUND: &str = "project.block_not_found";
    /// レンズカタログに無い品（種類 invalid）
    pub const LENS_NOT_FOUND: &str = "catalog.lens_not_found";
    /// 引数の組み合わせ・値の誤り（読み方の構成・状態・経路の指定など。種類 invalid）
    pub const BAD_ARGUMENT: &str = "argument.invalid";

    /// 符号の一覧（docs・試験で使う）
    pub const ALL: [&str; 7] = [
        FILE_NOT_FOUND,
        IS_FOLDER,
        GLASS_NOT_FOUND,
        STATE_NOT_FOUND,
        BLOCK_NOT_FOUND,
        LENS_NOT_FOUND,
        BAD_ARGUMENT,
    ];
}

/// 種類の前置き（英語と、日本語の表示のとき）
const PREFIXES: [(&str, &str); 3] = [
    ("not supported: ", "未対応: "),
    ("invalid: ", "不正: "),
    ("I/O: ", "入出力: "),
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
            Error::Coded(_, e) => e.fmt(f),
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
            Error::Coded(_, e) => e.kind(),
        }
    }

    /// 機械で見分ける符号（[`code`] の定数。符号を付けていなければ種類の名前 [`Error::kind`]）
    pub fn code(&self) -> &'static str {
        match self {
            Error::Coded(c, _) => c,
            e => e.kind(),
        }
    }

    /// 符号 c を付ける（種類・文は変えない。付いていれば付け替える）
    pub fn with_code(self, c: &'static str) -> Error {
        Error::Coded(c, Box::new(self.base_owned()))
    }

    /// 符号を外した誤り（種類で `match` するとき）
    pub fn base(&self) -> &Error {
        match self {
            Error::Coded(_, e) => e.base(),
            e => e,
        }
    }

    fn base_owned(self) -> Error {
        match self {
            Error::Coded(_, e) => e.base_owned(),
            e => e,
        }
    }

    /// 符号のない文の誤り（[`Error::Invalid`]・[`Error::Io`]・[`Error::NotImplemented`]）の文（中断は None）
    pub fn message(&self) -> Option<&str> {
        match self.base() {
            Error::NotImplemented(s) | Error::Invalid(s) | Error::Io(s) => Some(s),
            _ => None,
        }
    }

    /// 文を f で書き換える（種類・符号は変えない。中断はそのまま）。訳の表の文をソースに残すとき
    /// （`e.map_message(|e| format!("operation {k}: {e}"))`）
    pub fn map_message(self, f: impl FnOnce(&str) -> String) -> Error {
        match self {
            Error::NotImplemented(s) => Error::NotImplemented(f(&s)),
            Error::Invalid(s) => Error::Invalid(f(&s)),
            Error::Io(s) => Error::Io(f(&s)),
            Error::Cancelled => Error::Cancelled,
            Error::Coded(c, e) => Error::Coded(c, Box::new(e.map_message(f))),
        }
    }

    /// 文を前に足す（種類は変えない）。例: `e.at(sys.surf_label(j))` で「面 87(STANDARD, 膜 THORB): …」
    pub fn at(self, ctx: impl std::fmt::Display) -> Error {
        match self {
            Error::NotImplemented(s) => Error::NotImplemented(format!("{ctx}: {s}")),
            Error::Invalid(s) => Error::Invalid(format!("{ctx}: {s}")),
            Error::Io(s) => Error::Io(format!("{ctx}: {s}")),
            Error::Cancelled => Error::Cancelled,
            Error::Coded(c, e) => Error::Coded(c, Box::new(e.at(ctx))),
        }
    }
}

/// 入出力の誤り（種類 io。ファイルが無い・フォルダのときは符号 [`code::FILE_NOT_FOUND`]・[`code::IS_FOLDER`] も）
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        let c = match e.kind() {
            std::io::ErrorKind::NotFound => Some(code::FILE_NOT_FOUND),
            std::io::ErrorKind::IsADirectory => Some(code::IS_FOLDER),
            _ => None,
        };
        let x = Error::Io(e.to_string());
        match c {
            Some(c) => x.with_code(c),
            None => x,
        }
    }
}

impl Error {
    /// 入出力の誤りに文脈（ファイルの名前など）を付けて種類 io のまま上げる: `map_err(Error::io_at(p.display()))`
    pub fn io_at<C: std::fmt::Display>(ctx: C) -> impl FnOnce(std::io::Error) -> Error {
        move |e| Error::from(e).at(ctx)
    }
}

/// 文字列の誤り（種類の前置きがあればその種類、無ければ [`Error::Invalid`]）。`Result<_, String>` を返す関数を `?` で呼べる
impl From<String> for Error {
    fn from(s: String) -> Self {
        parse_display(&s).unwrap_or(Error::Invalid(s))
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::from(s.to_string())
    }
}

/// 誤りを文字列にする（`Result<_, String>` を返す関数の中で `?` で呼べる。種類・符号は消えるので、新しいコードは
/// [`Error`] のまま返す）。文字列の誤りは表示する所で訳す約束なので、ここでは訳さない（英語の元の文）。
/// 不正（[`Error::Invalid`]）は前置きなしの文、ほかは英語の種類の前置きつき（[`From<String>`] で種類に戻る）
impl From<Error> for String {
    fn from(e: Error) -> Self {
        match e {
            Error::Invalid(s) => s,
            Error::NotImplemented(s) => format!("{}{s}", PREFIXES[0].0),
            Error::Io(s) => format!("{}{s}", PREFIXES[2].0),
            Error::Cancelled => "cancelled".into(),
            Error::Coded(_, e) => String::from(*e),
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
    (code = $c:expr; $($a:tt)*) => {
        return Err($crate::error::Error::Invalid(format!($($a)*)).with_code($c))
    };
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

    #[test]
    fn io_errors_keep_the_kind() {
        let nf = std::io::Error::from(std::io::ErrorKind::NotFound);
        let e = Error::io_at("a.zmx")(nf);
        assert_eq!((e.kind(), e.code()), ("io", code::FILE_NOT_FOUND));
        assert!(
            e.message().unwrap_or_default().starts_with("a.zmx: "),
            "{e}"
        );
        let e = Error::from(std::io::Error::other("x"));
        assert_eq!((e.kind(), e.code()), ("io", "io"));
    }

    #[test]
    fn codes_keep_kind_and_text() {
        let e = Error::Invalid("x".into()).with_code(code::GLASS_NOT_FOUND);
        assert_eq!((e.kind(), e.code()), ("invalid", "glass.not_found"));
        assert_eq!(e.to_string(), Error::Invalid("x".into()).to_string());
        let e = e.at("surface 3");
        assert_eq!(e.code(), "glass.not_found");
        assert_eq!(e.message(), Some("surface 3: x"));
        assert_eq!(Error::Io("y".into()).code(), "io");
        assert_eq!(
            Error::Cancelled.with_code(code::BAD_ARGUMENT).base(),
            &Error::Cancelled
        );
        assert_eq!(
            Error::from("invalid: z".to_string()),
            Error::Invalid("z".into())
        );
    }
}
