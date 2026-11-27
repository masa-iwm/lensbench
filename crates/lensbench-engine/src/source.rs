//! 解析の登録簿の [`lensbench_analysis::Source`]: コマンドラインの引数（serve では load で選んだもの）からの系の読み方

use lensbench_core::{Error, Result, System};

use crate::load::SysSpec;
use lensbench_project::open::Source;
use std::sync::Arc;

/// 読み込んだ後の変更（serve の set・edit）を別に読んだ系に当てる口（系, 系の面の名前（プロジェクトでなければ空）,
/// 説明に付ける名前, 当てなかった変更）
pub(crate) type Replay<'a> = &'a dyn Fn(
    System,
    &[Option<lensbench_project::FaceId>],
    &str,
    &mut Vec<String>,
) -> lensbench_core::Result<System>;

/// 系のファイルの読み方。ファイルは初めに要るときに 1 回だけ読み（[`lensbench_project::open::Source`]）、状態・経路・
/// プロジェクトの文字列・別の状態の系はそこから引く
pub struct FileSource<'a> {
    pub c: &'a SysSpec,
    /// serve で読み込んだ後の変更を当てる（CLI は None）
    pub replay: Option<Replay<'a>>,
    /// serve で読み込んだ系（読み込んだ後の変更の入った系。CLI は None）
    cur: Option<&'a System>,
    src: std::sync::OnceLock<Result<Arc<Source>>>,
}

impl<'a> FileSource<'a> {
    /// CLI: ファイルは初めに要るときに読む
    pub fn new(c: &'a SysSpec, replay: Option<Replay<'a>>) -> Self {
        Self {
            c,
            replay,
            cur: None,
            src: Default::default(),
        }
    }

    /// serve: 読んだときのファイル src と、読み込んだ系 cur から引く。今の構成・状態・経路は cur そのもの、
    /// .zmx の別の構成は cur の多重構成の表から（変更が入ったまま）、ほかは src から読んで replay で変更を当てる
    pub fn loaded(c: &'a SysSpec, src: Arc<Source>, cur: &'a System, replay: Replay<'a>) -> Self {
        Self {
            c,
            replay: Some(replay),
            cur: Some(cur),
            src: std::sync::OnceLock::from(Ok(src)),
        }
    }

    /// 1 回だけ読んだファイル
    fn src(&self) -> Result<&Source> {
        self.src
            .get_or_init(|| Source::read(&self.c.file).map(Arc::new))
            .as_ref()
            .map(|s| &**s)
            .map_err(Clone::clone)
    }

    /// 読み込んだ系から引ける構成・状態の系（変更が入ったまま。引けなければ None）
    fn loaded_variant(&self, cfg: usize, state: &Option<String>) -> Result<Option<System>> {
        let Some(cur) = self.cur else {
            return Ok(None);
        };
        if lensbench_analysis::Source::is_project(self) {
            return Ok((*state == self.c.state).then(|| cur.clone()));
        }
        let active = cur.configs.as_ref().map_or(self.c.cfg, |t| t.active);
        if cfg == active {
            return Ok(Some(cur.clone()));
        }
        match &cur.configs {
            Some(_) => Ok(Some(cur.config(cfg)?)),
            None => Ok(None),
        }
    }
}

impl lensbench_analysis::Source for FileSource<'_> {
    fn is_project(&self) -> bool {
        match self.src() {
            Ok(s) => s.kind() == lensbench_project::open::FileKind::Project,
            Err(_) => {
                lensbench_project::open::FileKind::of(&self.c.file)
                    == lensbench_project::open::FileKind::Project
            }
        }
    }
    fn states(&self) -> Result<Vec<String>> {
        Ok(self.src().map(|s| s.states()).unwrap_or_default())
    }
    fn beams(&self) -> Result<Vec<String>> {
        Ok(self.src().map(|s| s.beams()).unwrap_or_default())
    }
    fn state(&self) -> Option<String> {
        self.c.state.clone()
    }
    fn paths(&self) -> Result<Vec<String>> {
        if !self.is_project() {
            return Ok(Vec::new());
        }
        self.src()?.paths(&self.c.open_spec())
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
                    Error::Invalid(format!("beam {b} (a configuration number in .zmx, from 1)"))
                })?;
                (k, None)
            }
        };
        if v.wavelengths.is_none() && v.path.is_none() {
            // 光束を選ばなければ読み込んだ系そのもの
            if let (None, Some(cur)) = (&v.beam, self.cur) {
                return Ok(cur.clone());
            }
            if let Some(s) = self.loaded_variant(cfg, &state)? {
                return Ok(s);
            }
        }
        // 経路を持つ光束は自分の経路で読む（読み込んだ系の経路は残さない）
        let own_path = project
            && state.as_deref().is_some_and(|n| {
                self.src()
                    .ok()
                    .and_then(|s| s.project())
                    .and_then(|p| p.beam(n))
                    .is_some_and(|b| b.path.is_some())
            });
        // 経路を選んだときは、読み込んだ系そのもの・同じ状態の系の写しは使わない
        let c = SysSpec {
            cfg,
            path: if own_path {
                None
            } else {
                v.path.clone().or_else(|| self.c.path.clone())
            },
            state,
            ..self.c.clone()
        };
        let mut faces = Vec::new();
        let (s, n) = match &v.wavelengths {
            None if c.cfg == 0 => crate::load::load_file_notes(&c)?,
            None => {
                let (s, n, f) = crate::load::load_source_faces(self.src()?, &c)?;
                faces = f;
                (s, n)
            }
            Some(wl) => {
                let zmx = c
                    .file
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("zmx"));
                if !zmx && !project {
                    return Err(Error::Invalid(
                        "only .zmx and project files can be read with replaced trace wavelengths"
                            .into(),
                    ));
                }
                // 読んだときの中身から（ファイルは読み直さない）。プロジェクトは光束（状態）と経路のまま
                let mut o = c.open_spec();
                o.wavelengths = Some(wl.clone());
                let op = self.src()?.open(&o)?;
                faces = op.faces;
                let s = op.system;
                let mut n = op.warnings;
                n.extend(crate::load::system_notes(&s));
                (s, n)
            }
        };
        notes.extend(n);
        match self.replay {
            None => Ok(s),
            Some(r) => {
                let label = match &v.beam {
                    Some(b) if project => format!("state {b}"),
                    Some(b) => format!("configuration {b}"),
                    None if project => format!("state {}", c.state.as_deref().unwrap_or("default")),
                    None => format!("configuration {cfg}"),
                };
                r(s, &faces, &label, notes)
            }
        }
    }
    fn project_text(&self) -> Result<Option<String>> {
        if !self.is_project() {
            return Ok(None);
        }
        Ok(self.src()?.project_text().map(str::to_string))
    }
    fn system_hash(&self, s: &System) -> Result<String> {
        lensbench_project::sysfile::system_hash(s)
    }
}
