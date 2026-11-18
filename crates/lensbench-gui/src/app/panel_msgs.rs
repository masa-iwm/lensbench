//! パネルの操作を当てる（パネルは描くときに操作を出すだけで、ここで状態を変えて計算を頼む）

use super::*;
use crate::optim_panel as op;
use crate::swap_panel as sp;
use crate::tol_panel as tp;

impl Core {
    /// 最適化のパネルの操作
    pub(super) fn on_opt(&mut self, a: op::Action) {
        use op::Action;
        match a {
            Action::Run(p) => {
                self.p.opt.running = true;
                self.p.opt.result = None;
                self.p.opt.gen = self.sys_gen;
                self.calc(Tag::Optimize, p);
            }
            Action::Stop => crate::worker::progress::stop("optimize"),
            Action::Apply => {
                let Some(r) = self.p.opt.result.clone() else {
                    return;
                };
                // 最適化の間・後に系が変わっていれば、古い系の値なので当てない
                if self.p.opt.gen != self.sys_gen || self.busy_core() {
                    self.p.opt.error = Some(
                        tr("The system changed after the optimization started, so the result was not applied (optimize again)")
                            .to_string(),
                    );
                    return;
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
                let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                let d = crate::files::FileDialog::new()
                    .add_filter(tr("Optimization settings"), &["json"])
                    .set_file_name("optimize.json");
                self.save_as(d, move |c, p| c.write_reported(&p, txt + "\n"));
            }
            // 読み込みのファイルの窓はパネルが毎フレーム見る
            Action::Load => {}
        }
    }

    /// 公差のパネルの操作
    pub(super) fn on_tol(&mut self, a: tp::Action) {
        use tp::Action;
        match a {
            Action::Run(p) => {
                self.p.tol.running = true;
                self.p.tol.result = None;
                self.calc(Tag::Tolerance, p);
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
            Action::Load => {}
        }
    }

    /// 差し替え探索のパネルの操作
    pub(super) fn on_swap(&mut self, a: sp::Action) {
        use sp::Action;
        let count = matches!(a, Action::Count(_));
        match a {
            Action::Run(_) | Action::Count(_) if self.p.swap.running => {}
            Action::Run(mut v) | Action::Count(mut v) => {
                if count {
                    v["count_only"] = json!(true);
                }
                v["project"] = sp::combo_project(&self.doc.combo).0;
                v["top"] = json!(self.p.swap.top);
                self.swap_ids = self.doc.combo.ids();
                self.p.swap.running = true;
                self.p.swap.result = None;
                self.calc(Tag::Swap, v);
            }
            Action::Stop => crate::worker::progress::stop("swap"),
            Action::Insert(i) => {
                let res = self.p.swap.result.as_ref().and_then(|r| r.results.as_ref());
                let Some(row) = res.and_then(|r| r.get(i)).cloned() else {
                    return;
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
                let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                let d = crate::files::FileDialog::new()
                    .add_filter(tr("Swap search settings"), &["json"])
                    .set_file_name("swap.json");
                self.save_as(d, move |c, p| c.write_reported(&p, txt + "\n"));
            }
            Action::Load => {}
        }
    }

    /// ズームのパネル: 軌跡を解く
    pub(super) fn on_zoom(&mut self, p: Value) {
        self.p.zoom.running = true;
        self.p.zoom.result = None;
        self.calc(Tag::Zoom, p);
    }

    /// 組合せのレイアウト図の操作
    pub(super) fn on_combo(&mut self, act: combo::Act) {
        match act {
            combo::Act::Catalog => self.msgs.push(Msg::FocusKind(Kind::Catalog)),
            combo::Act::Build => self.build_combo(self.doc.combo.project()),
            combo::Act::Focus => self.request_focus(),
            combo::Act::SaveToCatalog(k) => {
                self.p.cat_save.open_for(k, &self.doc.combo);
                let c = self.p.cat_save.catalog.trim().to_string();
                if !c.is_empty() && self.p.cat_save.listed != c {
                    self.draft(Tag::UserCatalog(CatOp::Items), json!({ "catalog": c }));
                }
            }
            combo::Act::Open(k) => {
                if let Some(id) = self.doc.combo.ids().get(k) {
                    // 面の表がまだ前の系なら、木に無い名前は入れない（最上位で選ぶ）
                    self.p.ld.scope.clear();
                    self.p.ld.enter(&self.sys.ld_blocks, id);
                    if self.p.ld.scope.is_empty() {
                        self.p.ld.sel = Some(id.clone());
                    }
                }
                self.msgs.push(Msg::FocusKind(Kind::LensData));
            }
        }
    }

    /// カタログのパネルの操作
    pub(super) fn on_catalog(&mut self, a: catalog::Action) {
        match a {
            catalog::Action::Refresh => self.refresh_catalog(),
            catalog::Action::Settings => self.folders_open = true,
            catalog::Action::SysSettings => self.msgs.push(Msg::FocusKind(Kind::SysSettings)),
            catalog::Action::New => self.msgs.push(Msg::New),
            catalog::Action::Build(p) => self.build_combo(p),
        }
    }

    /// レンズデータの表の操作
    pub(super) fn on_ld(&mut self, a: lens_data::Act) {
        use lens_data::Act;
        let ids = self.doc.combo.ids();
        let lead = self.doc.combo.lead_stop();
        let live = self.doc.combo.span_ids();
        let spans = lens_data::elem_spans(&live, &self.sys.elem_blocks, lead, &self.sys.ld_rows);
        match a {
            Act::Select(name) => {
                // 選ばないとき（面単位の選択など）は図の要素も外す。残すと次のフレームで表に戻る
                if self.p.ld.scope.is_empty() {
                    match name.and_then(|n| ids.iter().position(|i| *i == n)) {
                        Some(k) => self.p.combo_view.select(k),
                        None => self.p.combo_view.set_sel(None),
                    }
                }
            }
            Act::Lock(name, on) => match ids.iter().position(|i| *i == name) {
                Some(k) => {
                    if let Some(p) = self.doc.combo.el_mut(k) {
                        p.locked = on;
                    }
                }
                None => self.fail(tr("In a combination, lock whole elements")),
            },
            Act::ExportRange([a, b]) => {
                let stem = self
                    .loc
                    .file
                    .as_ref()
                    .and_then(|f| f.file_stem())
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "system".into());
                let d = crate::files::FileDialog::new()
                    .add_filter(tr("System file"), &["json"])
                    .set_file_name(format!("{stem}_{a}-{b}.json"));
                self.save_as(d, move |c, p| {
                    c.calc(
                        Tag::SaveRange,
                        json!({ "path": p.display().to_string(), "from": a, "to": b }),
                    );
                });
            }
            Act::Refused(why) => self.fail(why),
            Act::Edit { j, col, value } => {
                let mut fields = Map::new();
                fields.insert(col.field().into(), value);
                self.surface_fields(&spans, j, fields);
            }
            Act::Fields { j, fields } => self.surface_fields(&spans, j, fields),
            Act::Data { j, data } => self.surface_fields(&spans, j, data),
            Act::Ops(ops) => match self.combo_ops(&spans, &ids, &ops) {
                // ソルブ・ピックアップは serve の返事で要素に当ててから組み直す
                Ok(true) => {}
                Ok(false) => {
                    if !self.s.auto_build {
                        self.build_combo(self.doc.combo.project());
                    }
                }
                Err(e) => self.fail(e),
            },
            Act::DropOverride { j, col } => {
                if let Some((k, i)) = self.combo_face(&spans, j) {
                    let names = ld_combo::override_fields(col);
                    if self.doc.combo.el(k).is_some_and(|p| p.locked) {
                        self.fail(tr("A locked element cannot be changed"));
                        return;
                    }
                    if let Some(p) = self.doc.combo.el_mut(k) {
                        p.edits
                            .retain(|e| !(e.surface == i && names.contains(&e.field.as_str())));
                    }
                    if !self.s.auto_build {
                        self.build_combo(self.doc.combo.project());
                    }
                }
            }
            Act::DropPick { after, pick } => {
                // 面 after を持つ要素の後（要素の外の面なら、その面より前の要素の後）
                let at = spans
                    .iter()
                    .rposition(|s| s.is_some_and(|s| s[0] <= after))
                    .map_or(0, |k| k + 1);
                let k = combo::insert_pick(&mut self.doc.combo, at, *pick);
                self.p.combo_view.select(k);
            }
            Act::NeedGlasses => {
                if !self.browser.glasses_pending {
                    self.refresh_catalog();
                }
            }
            Act::OptVar(name) => {
                if !self.p.opt.vars.iter().any(|v| v.name.trim() == name) {
                    // 空の行があればそこに入れる
                    match self
                        .p
                        .opt
                        .vars
                        .iter_mut()
                        .find(|v| v.name.trim().is_empty())
                    {
                        Some(v) => v.name = name.clone(),
                        None => self.p.opt.vars.push(crate::optim_panel::VarRow {
                            name: name.clone(),
                            ..Default::default()
                        }),
                    }
                }
                self.msgs.push(Msg::FocusKind(Kind::Optimize));
                self.info(trf("Added {} to the optimization variables", &[&name]));
            }
            Act::NeedData(j) => {
                self.calc(Tag::SurfData(j), json!({ "surface": j }));
            }
        }
    }
}
