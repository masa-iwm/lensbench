//! 系のファイル（.zmx・プロジェクトファイル・系ファイル）を拡張子で見分けて読む。CLI・serve・Python
//! の共通の入口（XCT-06・CLI-16）。誤りは core の [`Error`] で、種類（読込・不正・未対応）が付く

use std::path::{Path, PathBuf};

use lensbench_core::catalog::LensCatalog;
use lensbench_core::error::code;
use lensbench_core::{zmx, Error, System};

/// 読み方の引数の組み合わせ・値の誤り（符号 [`code::BAD_ARGUMENT`]）
fn bad_arg(m: String) -> Error {
    Error::Invalid(m).with_code(code::BAD_ARGUMENT)
}

/// 系のファイルの種類（拡張子と中身で決める）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// OpticStudio の .zmx（.json 以外はすべてこれとして読む）
    Zmx,
    /// プロジェクトファイル（ブロックで組む .json）
    Project,
    /// 系ファイル（lensbench 独自の .json、`"format": "lensbench-system"`）
    SystemFile,
}

impl FileKind {
    /// パスの種類。.json は中身を読んで系ファイルかを見る（読めなければプロジェクトとする）
    pub fn of(path: &Path) -> FileKind {
        if !is_json(path) {
            FileKind::Zmx
        } else if crate::sysfile::is_system_file(path) {
            FileKind::SystemFile
        } else {
            FileKind::Project
        }
    }
}

/// 拡張子が .json か（大文字小文字を区別しない）
pub fn is_json(path: &Path) -> bool {
    path.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
}

/// 読み方
#[derive(Clone, Default)]
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
    /// プロジェクトの機構部品の詳しい形の STEP を引く口（[`crate::Env::step_files`]。None は簡易外形）
    pub step_files: Option<crate::StepLookup<'a>>,
}

impl std::fmt::Debug for OpenSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSpec")
            .field("glass_dirs", &self.glass_dirs)
            .field("lens_catalogs", &self.lens_catalogs)
            .field("cfg", &self.cfg)
            .field("state", &self.state)
            .field("split_path", &self.split_path)
            .field("wavelengths", &self.wavelengths)
            .field("names", &self.names)
            .field("step_files", &self.step_files.is_some())
            .finish()
    }
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
        split_path: "--beam-path",
    };
    /// serve の load の引数
    pub const SERVE: ArgNames = ArgNames {
        cfg: "cfg",
        state: "state",
        split_path: "beam_path",
    };
    /// Python の System.load の引数
    pub const PYTHON: ArgNames = ArgNames {
        cfg: "cfg",
        state: "state",
        split_path: "beam_path",
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

/// 系ファイル（lensbench 独自の JSON）・プロジェクトファイル（.json）・.zmx を読む（[`Source::read`] と [`Source::open`]）。
/// ファイルが無い・読めないときは [`Error::Io`]、引数の組み合わせや中身の誤りは [`Error::Invalid`]
pub fn open(path: &Path, o: &OpenSpec) -> Result<Opened, Error> {
    if o.cfg == 0 {
        return Err(bad_arg(format!("{} must be 1 or more", o.names.cfg)));
    }
    Source::read(path)?.open(o)
}

/// [`open`] に、プロジェクトの機構部品の詳しい形の STEP を引く口（[`crate::Env::step_files`]）を足したもの
pub fn open_with(
    path: &Path,
    o: &OpenSpec,
    step_files: Option<crate::StepLookup>,
) -> Result<Opened, Error> {
    open(
        path,
        &OpenSpec {
            step_files,
            ..o.clone()
        },
    )
}

/// 1 回だけ読んだ系のファイル（CLI-13）。プロジェクトは JSON を 1 回読んで解釈し、状態の名前・ビームスプリッターの経路・
/// 状態ごとの系をそこから引く（CLI・serve・Python の共通の入口）。系ファイル・.zmx は種類とパスだけ持ち、[`Source::open`] で読む
#[derive(Clone, Debug)]
pub enum Source {
    Zmx(PathBuf),
    SystemFile(PathBuf),
    Project {
        path: PathBuf,
        /// ファイルの中身（状態は当てない）
        text: String,
        project: Box<crate::Project>,
    },
}

impl Source {
    /// パスの種類を決め、プロジェクトなら読んで解釈する。無い・フォルダ・読めないは [`Error::Io`]、
    /// プロジェクトの中身の誤りは [`Error::Invalid`]
    pub fn read(path: &Path) -> Result<Source, Error> {
        if lensbench_core::vfs::is_dir(path) {
            return Err(
                Error::Io(format!("{}: a folder (give a system file)", path.display()))
                    .with_code(code::IS_FOLDER),
            );
        }
        if !lensbench_core::vfs::is_file(path) {
            return Err(Error::Io(format!("{}: no such file", path.display()))
                .with_code(code::FILE_NOT_FOUND));
        }
        let path_buf = path.to_path_buf();
        if !is_json(path) {
            return Ok(Source::Zmx(path_buf));
        }
        let txt = lensbench_core::vfs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        if crate::sysfile::is_system_json(&txt) {
            return Ok(Source::SystemFile(path_buf));
        }
        let project = crate::Project::from_json(&txt).map_err(|e| e.at(path.display()))?;
        Ok(Source::Project {
            path: path_buf,
            text: txt,
            project: Box::new(project),
        })
    }

    /// ファイルの種類
    pub fn kind(&self) -> FileKind {
        match self {
            Source::Zmx(_) => FileKind::Zmx,
            Source::SystemFile(_) => FileKind::SystemFile,
            Source::Project { .. } => FileKind::Project,
        }
    }

    /// ファイルのパス
    pub fn path(&self) -> &Path {
        match self {
            Source::Zmx(p) | Source::SystemFile(p) | Source::Project { path: p, .. } => p,
        }
    }

    /// 読んだプロジェクト（プロジェクトファイルでなければ None）
    pub fn project(&self) -> Option<&crate::Project> {
        match self {
            Source::Project { project, .. } => Some(project),
            _ => None,
        }
    }

    /// プロジェクトファイルの中身（状態は当てない。プロジェクトファイルでなければ None）
    pub fn project_text(&self) -> Option<&str> {
        match self {
            Source::Project { text, .. } => Some(text),
            _ => None,
        }
    }

    /// プロジェクトの状態の名前（.zmx・系ファイルなら空）
    pub fn states(&self) -> Vec<String> {
        self.project()
            .map(|p| p.state_names().iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// プロジェクトの状態 state（None は既定の状態）
    fn project_in(&self, state: Option<&str>) -> Result<Option<crate::Project>, Error> {
        let Some(p) = self.project() else {
            return Ok(None);
        };
        Ok(Some(match state {
            Some(n) => p.with_state(n)?,
            None => p.clone(),
        }))
    }

    /// プロジェクトを展開する場（相対パスはファイルのフォルダから）
    fn env<'c>(&self, o: &OpenSpec<'c>, cat: Option<crate::CatalogLookup<'c>>) -> crate::Env<'c> {
        crate::Env {
            base: self
                .path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
            glass_dirs: o.glass_dirs.to_vec(),
            catalog: cat,
            step_files: o.step_files,
        }
    }

    /// 状態 o.state のビームスプリッターの経路の一覧（.zmx・系ファイル・BS なしなら空）
    pub fn paths(&self, o: &OpenSpec) -> Result<Vec<String>, Error> {
        let Some(p) = self.project_in(o.state)? else {
            return Ok(Vec::new());
        };
        let cat = LensCatalog::load_registered(o.lens_catalogs)?;
        let look = |v: &str, q: &str| cat.zmx(v, q);
        crate::paths(&p, &self.env(o, Some(&look)))
    }

    /// 系を読む（[`open`] と同じ誤り）
    pub fn open(&self, o: &OpenSpec) -> Result<Opened, Error> {
        let ArgNames {
            cfg,
            state,
            split_path: split,
        } = o.names;
        if o.cfg == 0 {
            return Err(bad_arg(format!("{cfg} must be 1 or more")));
        }
        match self {
            Source::SystemFile(path) => {
                if o.cfg != 1 || o.state.is_some() || o.split_path.is_some() {
                    return Err(bad_arg(format!(
                        "{cfg}, {state} and {split} can't be used with a system file"
                    )));
                }
                if o.wavelengths.is_some() {
                    return Err(bad_arg("wavelengths is only for .zmx".into()));
                }
                let (system, m, warnings) = crate::sysfile::load(path)?;
                Ok(Opened {
                    system,
                    warnings,
                    coatings: m.coatings,
                })
            }
            Source::Project { path, project, .. } => {
                if o.cfg != 1 {
                    return Err(bad_arg(format!(
                        "choose the state with {state}, not {cfg}, in a project file"
                    )));
                }
                if o.wavelengths.is_some() {
                    return Err(bad_arg("wavelengths is only for .zmx".into()));
                }
                let at = |e: Error| e.at(path.display());
                let p = match o.state {
                    Some(n) => project.with_state(n).map_err(at)?,
                    None => (**project).clone(),
                };
                let cat = LensCatalog::load_registered(o.lens_catalogs)?;
                let look = |v: &str, q: &str| cat.zmx(v, q);
                let (system, warnings) =
                    crate::expand_path(&p, &self.env(o, Some(&look)), o.split_path).map_err(at)?;
                Ok(Opened {
                    system,
                    warnings,
                    coatings: Vec::new(),
                })
            }
            Source::Zmx(path) => {
                if o.state.is_some() {
                    return Err(bad_arg(format!(
                        "{state} is only for project files (.json) (.zmx uses {cfg})"
                    )));
                }
                if o.split_path.is_some() {
                    return Err(bad_arg(format!(
                        "{split} is only for project files (.json)"
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
        }
    }
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
            assert_eq!(e.code(), code::FILE_NOT_FOUND, "{f}: {e}");
        }
    }

    #[test]
    fn bad_arguments_are_invalid() {
        let o = OpenSpec {
            cfg: 0,
            ..Default::default()
        };
        let e = open(Path::new("x.zmx"), &o).unwrap_err();
        assert_eq!((e.kind(), e.code()), ("invalid", code::BAD_ARGUMENT));
    }

    #[test]
    fn file_kinds() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let prj = root.join("examples/projects/P10_catalog_four.json");
        assert_eq!(FileKind::of(&prj), FileKind::Project);
        assert_eq!(
            FileKind::of(&root.join("tests/refs/A0/system.zmx")),
            FileKind::Zmx
        );
        let a0 = lensbench_core::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_core::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let s = crate::sysfile::to_string(&a0, &[]).unwrap();
        let dir = std::env::temp_dir().join(format!("lb_open_kind_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sys.JSON");
        std::fs::write(&f, s).unwrap();
        assert_eq!(FileKind::of(&f), FileKind::SystemFile);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CLI-13: プロジェクトは 1 回読んで、状態・経路・状態ごとの系をそこから引く（ファイルから直に読んだものと同じ）
    #[test]
    fn source_reads_the_project_once() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = [root.join("tests/glass")];
        for f in ["P16_twyman_green.json", "P6_states_focus_group.json"] {
            let path = root.join("examples/projects").join(f);
            let src = Source::read(&path).unwrap();
            assert_eq!(src.kind(), FileKind::Project);
            assert!(src.project_text().is_some());
            let names = src.states();
            assert_eq!(names, src.project().unwrap().state_names());
            let mut states: Vec<Option<&str>> = vec![None];
            states.extend(names.iter().map(|s| Some(s.as_str())));
            for st in states {
                let o = OpenSpec {
                    glass_dirs: &glass,
                    cfg: 1,
                    state: st,
                    ..Default::default()
                };
                let direct = open(&path, &o).unwrap().system;
                let s = src.open(&o).unwrap().system;
                assert_eq!(format!("{s:?}"), format!("{direct:?}"), "{f} {st:?}");
                src.paths(&o).unwrap();
            }
        }
    }

    /// XCT-06: 誤りの符号は読む所を通っても残る（状態の名前・ガラス）
    #[test]
    fn codes_survive_open() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = [root.join("tests/glass")];
        let p = root.join("examples/projects/P6_states_focus_group.json");
        let o = OpenSpec {
            glass_dirs: &glass,
            cfg: 1,
            state: Some("nosuch"),
            ..Default::default()
        };
        let e = open(&p, &o).unwrap_err();
        assert_eq!(
            (e.kind(), e.code()),
            ("invalid", code::STATE_NOT_FOUND),
            "{e}"
        );
        // ガラスのフォルダが空: .zmx から取った玉のガラスが無い
        let empty = std::env::temp_dir().join(format!("lb_open_code_{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let dirs = [empty.clone()];
        let o = OpenSpec {
            glass_dirs: &dirs,
            cfg: 1,
            ..Default::default()
        };
        let e = open(&root.join("examples/projects/P5_triplet_blocks.json"), &o).unwrap_err();
        let _ = std::fs::remove_dir_all(&empty);
        assert_eq!(e.code(), code::GLASS_NOT_FOUND, "{e}");
    }

    #[test]
    fn folder_is_io_with_its_own_message() {
        let o = OpenSpec {
            cfg: 1,
            ..Default::default()
        };
        let e = open(Path::new("."), &o).unwrap_err();
        assert_eq!((e.kind(), e.code()), ("io", code::IS_FOLDER));
        assert!(e.message().unwrap_or_default().contains("a folder"), "{e}");
    }
}
