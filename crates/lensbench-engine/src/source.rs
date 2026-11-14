//! 解析の登録簿の [`lensbench_analysis::Source`]: コマンドラインの引数（serve では load で選んだもの）からの系の読み方

use lensbench_core::{Error, Result, System};

use crate::load::SysSpec;

/// 読み込んだ後の変更（serve の set・edit）を別に読んだ系に当てる口（系, 説明に付ける名前, 当てなかった変更）
pub(crate) type Replay<'a> =
    &'a dyn Fn(System, &str, &mut Vec<String>) -> std::result::Result<System, String>;

/// 系のファイルの読み方
pub struct FileSource<'a> {
    pub c: &'a SysSpec,
    /// serve で読み込んだ後の変更を当てる（CLI は None）
    pub replay: Option<Replay<'a>>,
}

/// 読む所の誤りの文（`load_file` の「不正: …」は頭を重ねない）
fn invalid(e: String) -> Error {
    Error::Invalid(e.strip_prefix("不正: ").map(str::to_string).unwrap_or(e))
}

impl lensbench_analysis::Source for FileSource<'_> {
    fn is_project(&self) -> bool {
        lensbench_project::open::FileKind::of(&self.c.file)
            == lensbench_project::open::FileKind::Project
    }
    fn states(&self) -> Result<Vec<String>> {
        Ok(crate::load::state_names(self.c))
    }
    fn state(&self) -> Option<String> {
        self.c.state.clone()
    }
    fn paths(&self) -> Result<Vec<String>> {
        crate::load::split_paths(self.c).map_err(invalid)
    }
    fn path(&self) -> Option<String> {
        self.c.path.clone()
    }
    fn load(&self, v: &lensbench_analysis::Variant, notes: &mut Vec<String>) -> Result<System> {
        let project = self.is_project();
        let (cfg, state) = match (&v.beam, project) {
            (None, _) => (self.c.cfg, self.c.state.clone()),
            (Some(b), true) => (1, Some(b.clone())),
            (Some(b), false) => {
                let k: usize = b.trim().parse().map_err(|_| {
                    Error::Invalid(format!("光束 {b}（.zmx では構成の番号、1 始まり）"))
                })?;
                (k, None)
            }
        };
        let c = SysSpec {
            cfg,
            state,
            ..self.c.clone()
        };
        let s = match &v.wavelengths {
            None => crate::load::load_file(&c)?,
            Some(wl) => {
                let zmx = c
                    .file
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("zmx"));
                if !zmx {
                    return Err(Error::Invalid(
                        "追跡する波長を置き換えて読めるのは .zmx だけ".into(),
                    ));
                }
                let opt = lensbench_core::zmx::LoadOptions {
                    cfg,
                    catalog_dirs: c.glass_dir.clone(),
                    wavelengths: Some(wl.clone()),
                    ..Default::default()
                };
                lensbench_core::zmx::load_with(&c.file, &opt)?
            }
        };
        notes.extend(
            s.dispersion_warnings()
                .into_iter()
                .chain(s.macro_warnings())
                .chain(s.import_notes.iter().cloned()),
        );
        match self.replay {
            None => Ok(s),
            Some(r) => {
                let label = match &v.beam {
                    Some(b) if project => format!("状態 {b}"),
                    Some(b) => format!("構成 {b}"),
                    None if project => format!("状態 {}", c.state.as_deref().unwrap_or("既定")),
                    None => format!("構成 {cfg}"),
                };
                r(s, &label, notes).map_err(invalid)
            }
        }
    }
    fn project_text(&self) -> Result<Option<String>> {
        if !self.is_project() {
            return Ok(None);
        }
        lensbench_core::vfs::read_to_string(&self.c.file)
            .map(Some)
            .map_err(|x| Error::Io(format!("{}: {x}", self.c.file.display())))
    }
    fn system_hash(&self, s: &System) -> Result<String> {
        lensbench_project::sysfile::system_hash(s).map_err(invalid)
    }
}
