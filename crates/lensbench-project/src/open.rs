//! 系のファイル（.zmx・プロジェクトファイル・系ファイル）を拡張子で見分けて読む。CLI・serve・Python
//! の共通の入口（XCT-06・CLI-16）。誤りは core の [`Error`] で、種類（読込・不正・未対応）が付く

use std::path::{Path, PathBuf};

use lensbench_core::catalog::LensCatalog;
use lensbench_core::{zmx, Error, System};

/// 読み方
#[derive(Clone, Debug, Default)]
pub struct OpenSpec<'a> {
    /// ガラスカタログを探すフォルダ
    pub glass_dirs: &'a [PathBuf],
    /// プロジェクトファイルのカタログレンズを探すレンズカタログ
    pub lens_catalogs: &'a [PathBuf],
    /// .zmx の多重構成の番号（1 始まり。0 は誤り）
    pub cfg: usize,
    /// プロジェクトファイルの状態の名前
    pub state: Option<&'a str>,
    /// プロジェクトファイルのビームスプリッターの経路
    pub split_path: Option<&'a str>,
    /// .zmx の波長を置き換える [µm]
    pub wavelengths: Option<Vec<f64>>,
    /// 誤りの文に書く引数の名前
    pub names: ArgNames,
}

/// 誤りの文に書く引数（構成・状態・経路）の名前。呼ぶ側の書き方に合わせる
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgNames {
    pub cfg: &'static str,
    pub state: &'static str,
    pub split_path: &'static str,
}

impl ArgNames {
    /// CLI の旗
    pub const CLI: ArgNames = ArgNames {
        cfg: "--cfg",
        state: "--state",
        split_path: "--path",
    };
    /// serve の load の引数
    pub const SERVE: ArgNames = ArgNames {
        cfg: "cfg",
        state: "state",
        split_path: "path",
    };
    /// Python の System.load の引数
    pub const PYTHON: ArgNames = ArgNames {
        cfg: "cfg",
        state: "state",
        split_path: "split_path",
    };
}

/// 既定は CLI の旗
impl Default for ArgNames {
    fn default() -> Self {
        ArgNames::CLI
    }
}

/// 読んだ系
#[derive(Clone, Debug)]
pub struct Opened {
    pub system: System,
    /// 注意（呼ぶ側が表示する）
    pub warnings: Vec<String>,
    /// 系ファイルに書いてあった膜の実測表・モデルのパス
    pub coatings: Vec<PathBuf>,
}

/// 系ファイル（lensbench 独自の JSON）・プロジェクトファイル（.json）・.zmx を読む。
/// ファイルが無い・読めないときは [`Error::Io`]、引数の組み合わせや中身の誤りは [`Error::Invalid`]
pub fn open(path: &Path, o: &OpenSpec) -> Result<Opened, Error> {
    let ArgNames {
        cfg,
        state,
        split_path: split,
    } = o.names;
    if o.cfg == 0 {
        return Err(Error::Invalid(format!("{cfg} は 1 以上")));
    }
    if lensbench_core::vfs::is_dir(path) {
        return Err(Error::Io(format!(
            "{}: フォルダ（系のファイルを渡す）",
            path.display()
        )));
    }
    if !lensbench_core::vfs::is_file(path) {
        return Err(Error::Io(format!("{}: ファイルが無い", path.display())));
    }
    if crate::sysfile::is_system_file(path) {
        if o.cfg != 1 || o.state.is_some() || o.split_path.is_some() {
            return Err(Error::Invalid(format!(
                "系ファイルでは {cfg}・{state}・{split} は使えない"
            )));
        }
        if o.wavelengths.is_some() {
            return Err(Error::Invalid("wavelengths は .zmx だけ".into()));
        }
        let (system, m, warnings) = crate::sysfile::load(path).map_err(invalid)?;
        return Ok(Opened {
            system,
            warnings,
            coatings: m.coatings,
        });
    }
    if path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        if o.cfg != 1 {
            return Err(Error::Invalid(format!(
                "プロジェクトファイルでは {cfg} ではなく {state} で状態を選ぶ"
            )));
        }
        if o.wavelengths.is_some() {
            return Err(Error::Invalid("wavelengths は .zmx だけ".into()));
        }
        let cat = LensCatalog::load_registered(o.lens_catalogs).map_err(invalid)?;
        let look = |v: &str, p: &str| cat.zmx(v, p);
        let (system, warnings) =
            crate::load_path_report(path, o.glass_dirs, Some(&look), o.state, o.split_path)
                .map_err(invalid)?;
        return Ok(Opened {
            system,
            warnings,
            coatings: Vec::new(),
        });
    }
    if o.state.is_some() {
        return Err(Error::Invalid(format!(
            "{state} はプロジェクトファイル（.json）だけ（.zmx は {cfg}）"
        )));
    }
    if o.split_path.is_some() {
        return Err(Error::Invalid(format!(
            "{split} はプロジェクトファイル（.json）だけ"
        )));
    }
    let opt = zmx::LoadOptions {
        cfg: o.cfg,
        catalog_dirs: o.glass_dirs.to_vec(),
        wavelengths: o.wavelengths.clone(),
        ..Default::default()
    };
    Ok(Opened {
        system: zmx::load_with(path, &opt)?,
        warnings: Vec::new(),
        coatings: Vec::new(),
    })
}

/// 文字列の誤りを [`Error::Invalid`] に（core の誤りを文字列にしたものは前置きを外す）
fn invalid(e: String) -> Error {
    for (p, k) in [
        ("不正: ", Error::Invalid as fn(String) -> Error),
        ("読込: ", Error::Io),
        ("未対応: ", Error::NotImplemented),
    ] {
        if let Some(s) = e.strip_prefix(p) {
            return k(s.to_string());
        }
    }
    Error::Invalid(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_io() {
        let o = OpenSpec {
            cfg: 1,
            ..Default::default()
        };
        for f in ["nosuch.zmx", "nosuch.json", "nosuch.lbsys.json"] {
            let e = open(Path::new(f), &o).unwrap_err();
            assert_eq!(e.kind(), "io", "{f}: {e}");
        }
    }

    #[test]
    fn bad_arguments_are_invalid() {
        let o = OpenSpec {
            cfg: 0,
            ..Default::default()
        };
        assert_eq!(open(Path::new("x.zmx"), &o).unwrap_err().kind(), "invalid");
    }

    #[test]
    fn folder_is_io_with_its_own_message() {
        let o = OpenSpec {
            cfg: 1,
            ..Default::default()
        };
        let e = open(Path::new("."), &o).unwrap_err();
        assert_eq!(e.kind(), "io");
        assert!(e.to_string().contains("フォルダ"), "{e}");
    }
}
