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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotImplemented(s) => write!(f, "未対応: {s}"),
            Error::Invalid(s) => write!(f, "不正: {s}"),
            Error::Io(s) => write!(f, "読込: {s}"),
            Error::Cancelled => write!(f, "中断した"),
        }
    }
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
