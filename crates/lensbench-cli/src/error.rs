//! コマンドラインの誤り（見直し CLI-25）: 種類ごとに終了コードと次の手の案内を持つ。
//!
//! serve・Python は core の誤りの種類（[`lensbench_core::Error::kind`]）を `data.kind`・例外のクラスに写す。
//! コマンドラインはこの型で終了コードに写す（表は [`EXIT_CODES`]、`lensbench --help` の末尾と docs/cli.md にも同じ表）。

use lensbench_core::Error as CoreError;

/// 終了コードと意味（`lensbench --help`・`lensbench help --json`・docs に出す）
pub const EXIT_CODES: &[(u8, &str)] = &[
    (0, "成功"),
    (
        1,
        "失敗（系・値の不正、計算の失敗、照合の不合格、結果は出したが一部が失敗）",
    ),
    (2, "使い方の誤り（引数・設定ファイル。run の設定の誤りも）"),
    (3, "ファイルが無い・読めない・書けない"),
    (4, "未対応の機能（.zmx の未対応の面・項目など）"),
    (127, "run: Python が見つからない"),
    (130, "中断した"),
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
            AppError::Io(_) | AppError::Core(CoreError::Io(_)) => 3,
            AppError::Core(CoreError::NotImplemented(_)) => 4,
            AppError::Core(CoreError::Cancelled) => 130,
            AppError::Core(CoreError::Invalid(_)) | AppError::Failed(_) => 1,
        }
    }

    /// 種類の名前（core の誤りは [`CoreError::kind`] と同じ。ほかは `usage`・`io`・`failed`）
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Usage(_) => "usage",
            AppError::Io(_) => "io",
            AppError::Core(e) => e.kind(),
            AppError::Failed(_) => "failed",
        }
    }

    /// 次の手の案内（無ければ None）。ガラスが無いときの案内は core の文に入っている
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            AppError::Usage(_) => Some("引数の一覧は lensbench <コマンド> --help"),
            AppError::Io(_) | AppError::Core(CoreError::Io(_)) => {
                Some("ファイルの名前と場所を確かめる（相対パスは今のフォルダから）")
            }
            AppError::Core(CoreError::NotImplemented(_)) => {
                Some("対応している面・項目はマニュアルの「.zmx の読込」（zmx.md）にある")
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Usage(s) | AppError::Io(s) | AppError::Failed(s) => f.write_str(s),
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

/// 誤りを標準エラーに出す（`lensbench: <文>` と、案内があれば `  案内: <案内>`）
pub fn report(prefix: &str, e: &AppError) {
    eprintln!("{prefix}: {e}");
    if let Some(h) = e.hint() {
        eprintln!("  案内: {h}");
    }
}

/// `lensbench --help` の末尾に付ける終了コードの表
pub fn exit_code_help() -> String {
    let mut s = String::from("終了コード:\n");
    for (c, m) in EXIT_CODES {
        s.push_str(&format!("  {c:>3}  {m}\n"));
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
    fn core_message_and_kind_are_kept() {
        let e = AppError::from(CoreError::Invalid("面 3".into()));
        assert_eq!(e.to_string(), "不正: 面 3");
        assert_eq!(e.kind(), "invalid");
        assert!(AppError::usage("x").hint().unwrap().contains("--help"));
    }
}
