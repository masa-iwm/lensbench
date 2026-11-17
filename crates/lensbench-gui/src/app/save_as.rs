//! 保存先を選んでから書く。窓はネイティブでは別のスレッドで出すので、選んだ後の仕事を預けて、
//! 選び終わった後のフレームで [`Core::poll_saves`] が呼ぶ（メニューの中など、毎フレーム同じ所で呼べない所から使う）

use super::*;

/// 選んだパスで呼ぶ仕事
pub(super) type SaveThen = Box<dyn FnOnce(&mut Core, PathBuf)>;

/// 選んでいる途中の保存（窓の id と、選んだ後の仕事）
#[derive(Default)]
pub(super) struct Saves {
    next: u64,
    wait: Vec<(String, SaveThen)>,
}

impl Saves {
    /// 保存先を選び始め、選んだら `then` を呼ぶ（やめたら呼ばない）。Core のほかの欄を借りている所から
    pub(super) fn push(
        &mut self,
        d: crate::files::FileDialog,
        then: impl FnOnce(&mut Core, PathBuf) + 'static,
    ) {
        let id = format!("save-as-{}", self.next);
        self.next += 1;
        crate::files::start_save(&id, d);
        self.wait.push((id, Box::new(then)));
    }
}

impl Core {
    /// 保存先を選び、選んだら `then` を呼ぶ（やめたら呼ばない）
    pub(super) fn save_as(
        &mut self,
        d: crate::files::FileDialog,
        then: impl FnOnce(&mut Core, PathBuf) + 'static,
    ) {
        self.saves.push(d, then);
    }

    /// 保存先を選んでいる途中か
    pub(super) fn saving(&self) -> bool {
        !self.saves.wait.is_empty()
    }

    /// 選び終わった保存の仕事を呼ぶ（毎フレーム）
    pub(super) fn poll_saves(&mut self) {
        let mut keep = Vec::new();
        for (id, then) in std::mem::take(&mut self.saves.wait) {
            match crate::files::finished(&id) {
                None => keep.push((id, then)),
                Some(Some(p)) => then(self, p),
                Some(None) => {}
            }
        }
        // 仕事の中で足した保存は後ろに
        keep.append(&mut self.saves.wait);
        self.saves.wait = keep;
    }

    /// 文書を path に書く（[`Core::save`] が書き先を決めてから）。組合せはプロジェクトの .json、開いた系は serve で
    pub(super) fn save_to(&mut self, combo: bool, path: PathBuf) {
        if combo {
            let mut v = doc_file::project_file(&self.doc.combo);
            let panels = self.panels_value();
            if let Some(o) = panels.as_object() {
                for (k, x) in o {
                    v["gui"][k] = x.clone();
                }
            }
            let r = lensbench_core::vfs::write(
                &path,
                serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
            self.status = match r {
                Ok(()) => {
                    self.loc.doc_path = Some(path.clone());
                    self.loc.saved_combo = Snap::new(Some(self.doc.combo.clone()));
                    self.push_recent(&path.display().to_string());
                    Status {
                        text: trf("Saved to {}", &[&path.display()]),
                        error: false,
                    }
                }
                Err(e) => Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                },
            };
        } else {
            self.loc.doc_path = Some(path.clone());
            self.loc.doc_from = None;
            self.worker.send(
                Tag::Save,
                "save",
                json!({ "path": path.display().to_string() }),
            );
        }
    }

    /// 文字列を書いて、状態の行に知らせる
    pub(super) fn write_reported(&mut self, path: &Path, txt: String) {
        self.status = match lensbench_core::vfs::write(path, txt) {
            Ok(()) => Status {
                text: trf("Saved to {}", &[&path.display()]),
                error: false,
            },
            Err(e) => Status {
                text: format!("{}: {e}", path.display()),
                error: true,
            },
        };
    }
}
