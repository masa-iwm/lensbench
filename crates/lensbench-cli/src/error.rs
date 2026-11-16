//! コマンドラインの誤り（見直し CLI-25）: 種類ごとに終了コードと次の手の案内を持つ。
//!
//! serve・Python は core の誤りの種類（[`lensbench_core::Error::kind`]）を `data.kind`・例外のクラスに写す。
//! コマンドラインはこの型で終了コードに写す（表は [`EXIT_CODES`]、`lensbench --help` の末尾と docs/cli.md にも同じ表）。

use lensbench_core::Error as CoreError;

/// 終了コードと意味（`lensbench --help`・`lensbench help --json`・docs に出す）
pub const EXIT_CODES: &[(u8, &str)] = &[
    (0, "Success"),
    (
        1,
        "Failure (invalid system or value, calculation failed, verification failed, or results written but partly failed)",
    ),
    (2, "Usage error (arguments or settings file; also run settings errors)"),
    (3, "File missing, unreadable or unwritable"),
    (4, "Unsupported feature (unsupported .zmx surfaces, items, etc.)"),
    (127, "run: Python not found"),
    (130, "Cancelled"),
];

/// コマンドラインの誤り
#[derive(Debug, Clone, PartialEq)]
pub enum AppError {
    /// 引数・設定の誤り（終了コード 2）
    Usage(String),
    /// ファイルの読み書き（終了コード 3）
    Io(String),
    /// core の誤り（種類で終了コードを決める）
    Core(CoreError),
    /// そのほかの失敗（終了コード 1）
    Failed(String),
}

impl AppError {
    /// 使い方の誤り
    pub fn usage(m: impl Into<String>) -> AppError {
        AppError::Usage(m.into())
    }

    /// 引数の読めない値（core の Invalid）は使い方の誤り、ほかはそのまま
    pub fn usage_of(x: lensbench_core::Error) -> AppError {
        match x {
            lensbench_core::Error::Invalid(m) => AppError::Usage(m),
            x => AppError::Core(x),
        }
    }

    /// ファイルの読み書きの誤り
    pub fn io(m: impl Into<String>) -> AppError {
        AppError::Io(m.into())
    }

    /// ファイル p の読み書きの誤りにする（`.map_err(AppError::io_at(p))`）
    pub fn io_at(p: &std::path::Path) -> impl Fn(std::io::Error) -> AppError + '_ {
        move |e| AppError::Io(format!("{}: {e}", p.display()))
    }

    /// 終了コード（[`EXIT_CODES`]）
    pub fn exit_code(&self) -> u8 {
        match self {
            AppError::Usage(_) => 2,
            AppError::Io(_) => 3,
            AppError::Failed(_) => 1,
            AppError::Core(e) => match e.base() {
                CoreError::Io(_) => 3,
                CoreError::NotImplemented(_) => 4,
                CoreError::Cancelled => 130,
                _ => 1,
            },
        }
    }

    /// 次の手の案内（無ければ None）。ガラスが無いときの案内は core の文に入っている
    pub fn hint(&self) -> Option<String> {
        if let AppError::Core(e @ CoreError::Coded(..)) = self {
            return AppError::Core(e.base().clone()).hint();
        }
        let h = match self {
            AppError::Usage(_) => "See the arguments with lensbench <command> --help",
            AppError::Io(_) | AppError::Core(CoreError::Io(_)) => {
                "Check the file name and location (relative paths are from the current folder)"
            }
            AppError::Core(CoreError::NotImplemented(_)) => {
                "The supported surfaces and items are in the manual (Reading .zmx files, zmx.md)"
            }
            _ => return None,
        };
        Some(lensbench_core::i18n::tr(h))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Usage(s) | AppError::Io(s) | AppError::Failed(s) => {
                f.write_str(&lensbench_core::i18n::tr(s))
            }
            AppError::Core(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        AppError::Core(e)
    }
}

/// 組み込み用（[`crate::ext::run_cli`]・watch）: 使い方の誤りは符号 argument.invalid の不正、ファイルは io
impl From<AppError> for CoreError {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Usage(m) => {
                CoreError::Invalid(m).with_code(lensbench_core::error::code::BAD_ARGUMENT)
            }
            AppError::Io(m) => CoreError::Io(m),
            AppError::Core(e) => e,
            AppError::Failed(m) => CoreError::from(m),
        }
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Failed(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Failed(s.to_string())
    }
}

/// 誤りを標準エラーに出す（`lensbench: <文>` と、案内があれば `  hint: <案内>`、日本語の表示は `  案内: `）
pub fn report(prefix: &str, e: &AppError) {
    eprintln!("{prefix}: {e}");
    if let Some(h) = e.hint() {
        let label = if lensbench_core::i18n::is_ja() {
            "案内"
        } else {
            "hint"
        };
        eprintln!("  {label}: {h}");
    }
}

/// `lensbench --help` の末尾に付ける終了コードの表
pub fn exit_code_help() -> String {
    let f = lensbench_core::i18n::tr_doc;
    let mut s = format!("{}\n", f("Exit codes:"));
    for (c, m) in EXIT_CODES {
        s.push_str(&format!("  {c:>3}  {}\n", f(m)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_kind() {
        assert_eq!(AppError::usage("x").exit_code(), 2);
        assert_eq!(AppError::io("x").exit_code(), 3);
        assert_eq!(AppError::from(CoreError::Io("x".into())).exit_code(), 3);
        assert_eq!(
            AppError::from(CoreError::NotImplemented("x".into())).exit_code(),
            4
        );
        assert_eq!(
            AppError::from(CoreError::Invalid("x".into())).exit_code(),
            1
        );
        assert_eq!(AppError::from(CoreError::Cancelled).exit_code(), 130);
        assert_eq!(AppError::from("x").exit_code(), 1);
        // 表にない終了コードを返さない
        for e in [
            AppError::usage("x"),
            AppError::io("x"),
            AppError::from("x"),
            AppError::from(CoreError::Invalid("x".into())),
            AppError::from(CoreError::NotImplemented("x".into())),
            AppError::from(CoreError::Cancelled),
        ] {
            assert!(EXIT_CODES.iter().any(|(c, _)| *c == e.exit_code()), "{e:?}");
        }
    }

    #[test]
    fn core_message_and_code_are_kept() {
        let e = AppError::from(CoreError::Invalid("surface 3".into()));
        assert_eq!(e.to_string(), "invalid: surface 3");
        assert_eq!(e.exit_code(), 1);
        assert!(AppError::usage("x").hint().unwrap().contains("--help"));
    }
}
