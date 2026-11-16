//! 最適化・ズーム・公差・差し替え・系の設定のパネル（app.rs から分けた、GAR-07）

use super::*;

impl Core {
    /// 最適化
    pub(super) fn optimize_ui(&mut self, ui: &mut Ui) {
        use crate::optim_panel::{self as op, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from a combination to optimize.",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let cands = op::Cands {
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            combo: is_combo,
            sys: self.param_sys(),
            rows: &self.sys.ld_rows,
        };
        let progress = if self.p.opt.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::optimize()
        } else {
            None
        };
        let acts = op::ui(ui, &mut self.p.opt, &cands, progress.as_deref());
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.p.opt.running = true;
                    self.p.opt.result = None;
                    self.p.opt.gen = self.sys_gen;
                    self.worker.send(Tag::Optimize, "optimize", p);
                }
                Action::Stop => crate::worker::progress::stop("optimize"),
                Action::Apply => {
                    let Some(r) = self.p.opt.result.clone() else {
                        continue;
                    };
                    // 最適化の間・後に系が変わっていれば、古い系の値なので当てない（GAR-20）
                    if self.p.opt.gen != self.sys_gen || self.worker.busy_core() {
                        self.p.opt.error = Some(
                            tr("The system changed after the optimization started, so the result was not applied (optimize again)")
                                .to_string(),
                        );
                        continue;
                    }
                    if self.apply_values(&op::result_values(&r, false)) {
                        self.p.opt.undo = Some(op::result_values(&r, true));
                    }
                }
                Action::Undo => {
                    if let Some(u) = self.p.opt.undo.take() {
                        self.apply_values(&u);
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("Optimization settings"), &["json"])
                        .set_file_name("optimize.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
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
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("opt-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Optimization settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.opt.set_params(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// モンテカルロ公差
    /// ズーム軌跡を解ける状態（.zmx の構成か、プロジェクトの状態）
    pub(super) fn zoom_cands(&self) -> Vec<crate::zoom_panel::Cand> {
        self.overlay
            .items
            .iter()
            .filter(|x| x.0 == "cfgs" || x.0 == "states")
            .map(|x| (x.0 == "cfgs", x.1.clone()))
            .collect()
    }

    pub(super) fn zoom_ui(&mut self, ui: &mut Ui) {
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr("Open a system to solve a zoom track."));
            return;
        }
        let cands = self.zoom_cands();
        let sys = self.param_sys();
        if let Some(p) = crate::zoom_panel::ui(ui, &mut self.p.zoom, &cands, &sys) {
            self.p.zoom.running = true;
            self.p.zoom.result = None;
            self.worker.send(Tag::Zoom, "zoom", p);
        }
    }

    /// 文字列をファイルに保存する（保存先はダイアログで選ぶ）
    pub(super) fn save_text(&mut self, filter: &str, ext: &str, name: &str, txt: String) {
        if let Some(path) = crate::files::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(name)
            .save_file()
        {
            self.status = match lensbench_core::vfs::write(&path, txt) {
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

    pub(super) fn tolerance_ui(&mut self, ui: &mut Ui) {
        use crate::tol_panel::{self as tp, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from a combination to run tolerancing.",
            ));
            return;
        }
        let cands = crate::optim_panel::Cands {
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            // 補償子は系の中で動かすだけ（組合せに当てない）ので、面の値の種類を絞らない
            combo: false,
            sys: self.param_sys(),
            rows: &self.sys.ld_rows,
        };
        let progress = if self.p.tol.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
            Some(crate::worker::progress::tolerance().unwrap_or((0, 0)))
        } else {
            None
        };
        let acts = tp::ui(ui, &mut self.p.tol, &cands, progress);
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.p.tol.running = true;
                    self.p.tol.result = None;
                    self.worker.send(Tag::Tolerance, "tolerance", p);
                }
                Action::Stop => crate::worker::progress::stop("tolerance"),
                Action::Save(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(tr("Tolerance settings"), "json", "tolerance.json", txt);
                }
                Action::SaveResult(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(
                        tr("Tolerance results"),
                        "json",
                        "tolerance_result.json",
                        txt,
                    );
                }
                Action::SaveCsv(txt) => {
                    self.save_text("CSV", "csv", "tolerance_compensators.csv", txt);
                }
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("tol-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Tolerance settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.tol.set_spec(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// 差し替え探索
    pub(super) fn swap_ui(&mut self, ui: &mut Ui) {
        use crate::swap_panel::{self as sp, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Build a system from a combination, or open a project with lens blocks, to run a swap search.",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let (project, mut cands) = if is_combo {
            let (p, c) = sp::combo_project(&self.doc.combo);
            (Some(p), c)
        } else {
            let f = self.loc.file.clone().unwrap_or_default();
            if self.swap_file.as_ref().map(|x| &x.0) != Some(&f) {
                let c = lensbench_core::vfs::read_to_string(&f)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .map(|v| sp::project_cands(&v))
                    .unwrap_or_default();
                self.swap_file = Some((f, c));
            }
            (
                None,
                std::mem::take(&mut self.swap_file.as_mut().unwrap().1),
            )
        };
        // 同時に評価できる状態（組合せは組合せの状態、開いたプロジェクトは読み込んだ系の状態）
        cands.states = if is_combo {
            self.doc.combo.states.clone()
        } else {
            self.overlay
                .items
                .iter()
                .filter(|x| x.0 == "states")
                .map(|x| x.1.clone())
                .collect()
        };
        let progress = if self.p.swap.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::swap()
        } else {
            None
        };
        cands.sys = self.param_sys();
        let acts = sp::ui(ui, &mut self.p.swap, &cands, progress);
        if !is_combo {
            if let Some(x) = self.swap_file.as_mut() {
                x.1 = cands;
            }
        }
        let mut load = false;
        for a in acts {
            let count = matches!(a, Action::Count(_));
            match a {
                Action::Run(_) | Action::Count(_) if self.p.swap.running => {}
                Action::Run(mut v) | Action::Count(mut v) => {
                    if count {
                        v["count_only"] = json!(true);
                    }
                    if let Some(p) = &project {
                        v["project"] = p.clone();
                    }
                    v["top"] = json!(self.p.swap.top);
                    self.swap_ids = self.doc.combo.ids();
                    self.p.swap.running = true;
                    self.p.swap.result = None;
                    self.worker.send(Tag::Swap, "swap", v);
                }
                Action::Stop => crate::worker::progress::stop("swap"),
                Action::Insert(i) => {
                    let res = self.p.swap.result.as_ref().and_then(|r| r.results.as_ref());
                    let Some(row) = res.and_then(|r| r.get(i)).cloned() else {
                        continue;
                    };
                    let before = self.doc.combo.clone();
                    match sp::insert_row(&mut self.doc.combo, &self.swap_ids, &row) {
                        Ok(()) => {
                            if self.p.swap.undo.is_none() {
                                self.p.swap.undo = Some(before);
                            }
                            self.p.swap.inserted = Some(i);
                            if !self.s.auto_build {
                                self.build_combo(self.doc.combo.project());
                            }
                        }
                        Err(e) => self.fail(e),
                    }
                }
                Action::Undo => {
                    if let Some(c) = self.p.swap.undo.take() {
                        self.doc.combo = c;
                        self.p.swap.inserted = None;
                        if !self.s.auto_build {
                            self.build_combo(self.doc.combo.project());
                        }
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("Swap search settings"), &["json"])
                        .set_file_name("swap.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
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
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("swap-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Swap search settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.swap.set_spec(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// 変数の値を開いている系に当てる（開いた系は serve の set、組合せの系は組合せの要素に当てて作り直す）。
    /// 当てられない値があれば何もせず false
    pub(super) fn apply_values(&mut self, vals: &[(String, f64)]) -> bool {
        if vals.is_empty() {
            return false;
        }
        if !self.is_combo() {
            let m: Map<String, Value> = vals.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            self.worker.send(Tag::Set, "set", json!({ "values": m }));
            return true;
        }
        let mut edits = Vec::new();
        for (n, x) in vals {
            match crate::optim_panel::combo_edit(n, *x) {
                Ok(e) => edits.push(e),
                Err(n) => {
                    self.fail(trf(
                        "Variable {} cannot be applied to the combination (only surface thickness, radius and conic)",
                        &[&n],
                    ));
                    return false;
                }
            }
        }
        let ids = self.doc.combo.span_ids();
        let lead = self.doc.combo.lead_stop();
        let spans = lens_data::elem_spans(&ids, &self.sys.ld_units, lead, &self.sys.ld_rows);
        for (j, key, value) in edits {
            if !self.combo_surface_edit(&spans, j, key, value) {
                return false;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
        true
    }

    /// 組合せの系の設定
    pub(super) fn sysset_ui(&mut self, ui: &mut Ui) {
        self.ask_folders();
        if self.loc.file.is_some() && !self.is_combo() {
            // 開いた系の設定。組合せの設定はその下にたたむ
            if self.p.oss.want() {
                self.worker.send(Tag::SysData, "system_data", json!({}));
            }
            let ns = self.sys.ld_rows.len();
            egui::ScrollArea::vertical()
                .id_salt("sysset_scroll")
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr(
                            "Settings of the open system. Changes go into the system; keep them with \"Save system file…\".",
                        ))
                        .small()
                        .weak(),
                    );
                    if let Some(op) = open_sysset::ui(ui, &mut self.p.oss, ns) {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": [op] }));
                    }
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(tr("Combination settings (used when a combination is built)"))
                        .id_salt("combo_sysset")
                        .default_open(false)
                        .show(ui, |ui| {
                            crate::sysset::ui(ui, &mut self.doc.combo, &self.folders.agf)
                        });
                });
            return;
        }
        let cats = &self.folders.agf;
        egui::ScrollArea::vertical()
            .id_salt("sysset_scroll")
            .show(ui, |ui| crate::sysset::ui(ui, &mut self.doc.combo, cats));
    }
}
