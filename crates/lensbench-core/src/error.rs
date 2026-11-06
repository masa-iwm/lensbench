//! エラー型。対応していない機能は黙って無視せず `NotImplemented` で止める。

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// 対応していない機能を含む
    NotImplemented(String),
    /// 値や構成が不正
    Invalid(String),
    /// ファイルの読込
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotImplemented(s) => write!(f, "未対応: {s}"),
            Error::Invalid(s) => write!(f, "不正: {s}"),
            Error::Io(s) => write!(f, "読込: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[macro_export]
macro_rules! notimpl {
    ($($a:tt)*) => { return Err($crate::error::Error::NotImplemented(format!($($a)*))) };
}
#[macro_export]
macro_rules! invalid {
    ($($a:tt)*) => { return Err($crate::error::Error::Invalid(format!($($a)*))) };
}
