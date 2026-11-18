//! 最適化・ズーム・公差・差し替え・系の設定のパネル（app.rs から分けた）

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
        let cands = op::Cands {
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            combo: true,
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
                Action::Load => load = true,
                a => self.msgs.push(Msg::Opt(a)),
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
            self.msgs.push(Msg::Zoom(p));
        }
    }

    /// 文字列をファイルに保存する（保存先はダイアログで選ぶ）
    pub(super) fn save_text(&mut self, filter: &str, ext: &str, name: &str, txt: String) {
        let d = crate::files::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(name);
        self.save_as(d, move |c, p| c.write_reported(&p, txt));
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
                Action::Load => load = true,
                a => self.msgs.push(Msg::Tol(a)),
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
                "Build a system from a combination to run a swap search.",
            ));
            return;
        }
        let (_, mut cands) = sp::combo_project(&self.doc.combo);
        // 同時に評価できる状態
        cands.states = self.doc.combo.states.clone();
        let progress = if self.p.swap.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::swap()
        } else {
            None
        };
        cands.sys = self.param_sys();
        let acts = sp::ui(ui, &mut self.p.swap, &cands, progress);
        let mut load = false;
        for a in acts {
            match a {
                Action::Load => load = true,
                a => self.msgs.push(Msg::Swap(a)),
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

    /// 変数の値を組合せの要素に当てて作り直す。
    /// 当てられない値があれば何もせず false
    pub(super) fn apply_values(&mut self, vals: &[(String, f64)]) -> bool {
        if vals.is_empty() {
            return false;
        }
        // 厚さ・半径・コーニックは要素の欄に。ほかの値（非球面係数・偏心など）があれば、serve の write_back で
        // 要素の面の上書きにする（返事で要素に入れて作り直す）
        let Some(edits) = vals
            .iter()
            .map(|(n, x)| crate::optim_panel::combo_edit(n, *x).ok())
            .collect::<Option<Vec<_>>>()
        else {
            let ops: Vec<Value> = vals
                .iter()
                .map(|(k, x)| json!({ "op": "set_param", "param": k, "value": x }))
                .collect();
            self.calc(Tag::WriteBack, json!({ "ops": ops }));
            return true;
        };
        let ids = self.doc.combo.span_ids();
        let lead = self.doc.combo.lead_stop();
        let spans = lens_data::elem_spans(&ids, &self.sys.elem_blocks, lead, &self.sys.ld_rows);
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
        let cats = &self.folders.agf;
        egui::ScrollArea::vertical()
            .id_salt("sysset_scroll")
            .show(ui, |ui| crate::sysset::ui(ui, &mut self.doc.combo, cats));
    }
}
