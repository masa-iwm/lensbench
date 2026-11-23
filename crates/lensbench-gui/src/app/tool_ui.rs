//! 最適化・ズーム・公差・差し替え・系の設定のパネル（app.rs から分けた）

use super::*;

impl Core {
    /// 最適化
    pub(super) fn optimize_ui(&mut self, ui: &mut Ui) {
        use crate::optim_panel as op;
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
        for a in acts {
            self.msgs.push(Msg::Opt(a));
        }
        // 設定の読み込み（窓は操作が出す。選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("opt-load", false, crate::files::FileDialog::new) {
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
        use crate::tol_panel as tp;
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
        for a in acts {
            self.msgs.push(Msg::Tol(a));
        }
        // 設定の読み込み（窓は操作が出す。選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("tol-load", false, crate::files::FileDialog::new) {
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
        use crate::swap_panel as sp;
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
        for a in acts {
            self.msgs.push(Msg::Swap(a));
        }
        // 設定の読み込み（窓は操作が出す。選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("swap-load", false, crate::files::FileDialog::new) {
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
            use lensbench_project::edit_spec::{EditOp, Num};
            let ops = vals
                .iter()
                .map(|(k, x)| EditOp::SetParam {
                    param: k.clone(),
                    value: Num(*x),
                })
                .collect();
            self.call(Tag::WriteBack, Call::WriteBack { ops });
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

    /// 面の材料を組合せの要素に当てる（ガラスの置き換えの結果）。当てられなければ false
    pub(super) fn apply_materials(&mut self, mats: &[(usize, String)]) -> bool {
        if mats.is_empty() {
            return false;
        }
        let ids = self.doc.combo.span_ids();
        let lead = self.doc.combo.lead_stop();
        let spans = lens_data::elem_spans(&ids, &self.sys.elem_blocks, lead, &self.sys.ld_rows);
        for (j, m) in mats {
            if !self.combo_surface_edit(&spans, *j, "material", json!(m)) {
                return false;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
        true
    }

    /// ガラスの置き換えの結果 gs（問題の構成 i は状態 states[i]。空は開いている系）を組合せに当てる。当てられなければ
    /// None、当てたら元に戻す材料。状態があれば、置き換えた状態の材料を書く: 面の材料の上書きのある状態は上書きを、
    /// 上書きの無い状態（要素の面の材料に従う）がどれも同じ硝材に置き換わったら面の材料を、そうでなければその状態の
    /// 上書きを（serve の apply が多重構成の材料の行の置き換えた構成の欄を書くのと同じ）
    pub(super) fn apply_glass(
        &mut self,
        gs: &[crate::optim_panel::GlassApply],
        states: &[String],
    ) -> Option<Vec<crate::optim_panel::GlassUndo>> {
        let mut undo = Vec::new();
        if self.apply_glass_into(gs, states, &mut undo).is_some() {
            return Some(undo);
        }
        // 途中で当てられなくなったら、それまでに書いた状態の上書きを戻す（半端に替わった組合せを残さない）
        if !undo.is_empty() {
            self.undo_glass(&undo);
        }
        None
    }

    /// [`Self::apply_glass`] の本体。書いた分の元に戻す材料を undo に足していく
    fn apply_glass_into(
        &mut self,
        gs: &[crate::optim_panel::GlassApply],
        states: &[String],
        undo: &mut Vec<crate::optim_panel::GlassUndo>,
    ) -> Option<()> {
        use crate::optim_panel::GlassUndo;
        let c = &self.doc.combo;
        // 問題の構成の状態（状態の無い組合せ・基本を開いて構成を並べなかったときは面の材料だけ）
        let probs: Vec<String> = match (states, &c.state) {
            _ if c.states.is_empty() => Vec::new(),
            ([], None) => Vec::new(),
            ([], Some(n)) => vec![n.clone()],
            _ => states.to_vec(),
        };
        let ids = c.span_ids();
        let spans = lens_data::elem_spans(
            &ids,
            &self.sys.elem_blocks,
            c.lead_stop(),
            &self.sys.ld_rows,
        );
        let mut base: Vec<(usize, String)> = Vec::new();
        let mut base_old: Vec<(usize, String)> = Vec::new();
        for g in gs {
            let j = g.surface;
            if probs.is_empty() {
                if let Some(Some((from, to))) = g.per.first() {
                    base.push((j, to.clone()));
                    base_old.push((j, from.clone()));
                }
                continue;
            }
            // 置き換えた状態の (状態, 前, 後)
            let hits: Vec<(&String, &String, &String)> = probs
                .iter()
                .zip(&g.per)
                .filter_map(|(n, x)| x.as_ref().map(|(a, b)| (n, a, b)))
                .filter(|(n, _, _)| self.doc.combo.states.contains(n))
                .collect();
            if hits.is_empty() {
                continue;
            }
            let Some((k, face)) = self.combo_face(&spans, j) else {
                self.fail(tr("This value cannot be changed here"));
                return None;
            };
            let loc = self.doc.combo.chain().get(k)?.loc.clone();
            let p = self.doc.combo.get(&loc)?;
            let own = |n: &str| p.state_edit(n, face, "material").cloned();
            // 面の材料に従う状態がどれも同じ硝材に置き換わったら面の材料を書く
            let users: Vec<&String> = self
                .doc
                .combo
                .states
                .iter()
                .filter(|n| own(n).is_none())
                .collect();
            let to_base = users
                .first()
                .and_then(|u| hits.iter().find(|h| h.0 == *u))
                .map(|h| (h.1, h.2));
            let to_base = to_base.filter(|(_, m)| {
                users
                    .iter()
                    .all(|u| hits.iter().any(|h| h.0 == *u && h.2 == *m))
            });
            let mut writes = Vec::new();
            for (n, _, m) in &hits {
                let old = own(n);
                if old.is_some() || to_base.is_none() {
                    undo.push(GlassUndo::State {
                        loc: loc.clone(),
                        face,
                        state: (*n).clone(),
                        old,
                    });
                    writes.push(((*n).clone(), (*m).clone()));
                }
            }
            if let Some((from, m)) = to_base {
                base.push((j, m.clone()));
                base_old.push((j, from.clone()));
            }
            let p = self.doc.combo.get_mut(&loc)?;
            for (n, m) in writes {
                p.set_state_edit(&n, face, "material", json!(m));
            }
        }
        // 面の材料は状態の上書きの後に（材料で要素を分け直すことがある）
        if !base.is_empty() {
            if !self.apply_materials(&base) {
                return None;
            }
            undo.extend(base_old.into_iter().map(|(j, m)| GlassUndo::Base(j, m)));
        } else if !undo.is_empty() && !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
        Some(())
    }

    /// ガラスの置き換えを当てる前の材料に戻す（[`Self::apply_glass`] の逆）
    pub(super) fn undo_glass(&mut self, undo: &[crate::optim_panel::GlassUndo]) {
        use crate::optim_panel::GlassUndo;
        let mut base = Vec::new();
        for u in undo {
            match u {
                GlassUndo::Base(j, m) => base.push((*j, m.clone())),
                GlassUndo::State {
                    loc,
                    face,
                    state,
                    old,
                } => {
                    let Some(p) = self.doc.combo.get_mut(loc) else {
                        continue;
                    };
                    match old {
                        Some(v) => p.set_state_edit(state, *face, "material", v.clone()),
                        None => {
                            if let Some(es) = p.state_edits.get_mut(state) {
                                es.retain(|e| !(e.surface == *face && e.field == "material"));
                                if es.is_empty() {
                                    p.state_edits.remove(state);
                                }
                            }
                        }
                    }
                }
            }
        }
        if !base.is_empty() {
            self.apply_materials(&base);
        } else if !undo.is_empty() && !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
    }

    /// 組合せの系の設定
    pub(super) fn sysset_ui(&mut self, ui: &mut Ui) {
        self.ask_folders();
        let cats = &self.folders.agf;
        egui::ScrollArea::vertical()
            .id_salt("sysset_scroll")
            .show(ui, |ui| {
                edit::stage(&mut self.doc.combo, &mut self.msgs, |c| {
                    crate::sysset::ui(ui, c, cats)
                })
            });
    }
}
