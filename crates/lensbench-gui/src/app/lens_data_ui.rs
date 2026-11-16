//! レンズデータの表・多重構成・機構の設定のパネルと、組合せの系の面の書き換え（app.rs から分けた、GAR-07）

use super::*;

impl Core {
    /// 面の表: 開いた系は serve の edit で変え、組合せの系は組合せ（要素の後の空気・面の上書き）を変えて作り直す
    pub(super) fn lens_data_ui(&mut self, ui: &mut Ui) {
        use lens_data::{Act, Col};
        if self.sys.summary.is_none() || self.sys.ld_rows.is_empty() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from the combination to see its surfaces here.",
            ));
            // 空の文書: 平面 1 枚から面を足していける（11/16 masa 決定: 表から自由に面を足す）
            if self.doc.combo.picks.is_empty() && (self.loc.file.is_none() || self.is_combo()) {
                ui.add_space(4.0);
                if ui
                    .button(tr("+ Add a surface"))
                    .on_hover_text(tr(
                        "Starts a surface list with one plane surface. Insert more surfaces from the table, and set a glass after a surface to make a lens",
                    ))
                    .clicked()
                {
                    self.doc.combo.insert_plane(0);
                }
            }
            return;
        }
        let is_combo = self.is_combo();
        let ids = if is_combo {
            self.doc.combo.ids()
        } else {
            Vec::new()
        };
        // 組合せ: 図で選んだ要素を表でも選ぶ（最上位のとき）
        if is_combo && self.p.ld.scope.is_empty() {
            if let Some(k) = self.p.combo_view.sel {
                self.p.ld.sel = ids.get(k).cloned();
            }
        }
        // 今の経路に沿った要素（分岐の無い組合せは並びそのもの）
        let picks = &self.doc.combo.chain_view().picks;
        let locked: BTreeSet<String> = if is_combo {
            ids.iter()
                .zip(picks)
                .filter(|(_, p)| p.locked)
                .map(|(i, _)| i.clone())
                .collect()
        } else {
            self.p.ld.locked.clone()
        };
        let lead = self.doc.combo.lead_stop();
        let spans = if is_combo {
            let live = self.doc.combo.span_ids();
            lens_data::elem_spans(&live, &self.sys.ld_units, lead, &self.sys.ld_rows)
        } else {
            Vec::new()
        };
        // 図で別の要素を選んだら、その要素の外の今の面は外す（図の目立たせと表を合わせる）
        if is_combo && self.p.combo_view.sel != self.combo_sel_prev {
            if let (Some(k), Some((j, _))) = (self.p.combo_view.sel, self.p.ld.cur) {
                let inside = spans
                    .get(k)
                    .copied()
                    .flatten()
                    .is_some_and(|s| s[0] <= j && j <= s[1]);
                if !inside {
                    self.p.ld.cur = None;
                }
            }
        }
        self.combo_sel_prev = self.p.combo_view.sel;
        let rows = &self.sys.ld_rows;
        let read_only = |j: usize, col: Col| -> Option<String> {
            ld_combo::read_only(is_combo, rows, &spans, picks, lead, j, col)
        };
        let overridden =
            |j: usize, col: Col| is_combo && ld_combo::overridden(rows, &spans, picks, j, col);
        let cfg_rows: Vec<String> = self
            .p
            .mc
            .table
            .as_ref()
            .map(|t| t.rows.iter().map(|r| r.0.clone()).collect())
            .unwrap_or_default();
        let ctx = lens_data::Ctx {
            config_n: self.p.mc.table.as_ref().map_or(1, |t| t.n),
            config_rows: &cfg_rows,
            glasses: &self.ld_glasses,
            glasses_loaded: self.browser.loaded && !self.browser.glasses_pending,
            rows,
            blocks: &self.sys.ld_blocks,
            locked: &locked,
            read_only: &read_only,
            overridden: &overridden,
            combo: is_combo,
            unit_mech: (!is_combo).then_some(self.um_open),
            note: is_combo.then(|| tr(
                "Combination system: changes go into the combination elements (the trailing air is the element spacing). Catalog lenses and imported elements keep them as surface overrides, marked in the cell (right-click to remove). Set decenter, tilt and BS in the element settings.",
            )),
        };
        let (acts, mut bottom) = lens_data::ui(ui, &mut self.p.ld, &ctx);
        for a in acts {
            match a {
                Act::Select(name) => {
                    // 選ばないとき（面単位の選択など）は図の要素も外す。残すと次のフレームで表に戻る
                    if is_combo && self.p.ld.scope.is_empty() {
                        match name.and_then(|n| ids.iter().position(|i| *i == n)) {
                            Some(k) => self.p.combo_view.select(k),
                            None => self.p.combo_view.sel = None,
                        }
                    }
                }
                Act::Lock(name, on) => {
                    if is_combo {
                        match ids.iter().position(|i| *i == name) {
                            Some(k) => {
                                if let Some(p) = self.doc.combo.el_mut(k) {
                                    p.locked = on;
                                }
                            }
                            None => self.fail(tr("In a combination, lock whole elements")),
                        }
                    } else {
                        if on {
                            self.p.ld.locked.insert(name);
                        } else {
                            self.p.ld.locked.remove(&name);
                        }
                        // ファイルごとに設定へ残す（空になったら消す、GPN-14）
                        if let Some(f) = self.loc.file.as_ref().map(|f| f.display().to_string()) {
                            if self.p.ld.locked.is_empty() {
                                self.s.locked.remove(&f);
                            } else {
                                self.s.locked.insert(f, self.p.ld.locked.clone());
                            }
                        }
                    }
                }
                Act::ExportRange([a, b]) => {
                    let stem = self
                        .loc
                        .file
                        .as_ref()
                        .and_then(|f| f.file_stem())
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "system".into());
                    if let Some(p) = crate::files::FileDialog::new()
                        .add_filter(tr("System file"), &["json"])
                        .set_file_name(format!("{stem}_{a}-{b}.json"))
                        .save_file()
                    {
                        self.worker.send(
                            Tag::SaveRange,
                            "save_range",
                            json!({ "path": p.display().to_string(), "from": a, "to": b }),
                        );
                    }
                }
                Act::UnitMech => {
                    self.um_open = !self.um_open;
                    // 範囲の欄が空なら、選んでいる面から
                    if self.um_open && self.p.um.new_spans.trim().is_empty() {
                        if let Some((j, _)) = self.p.ld.cur {
                            self.p.um.new_spans = j.to_string();
                        }
                    }
                }
                Act::Refused(why) => self.fail(why),
                Act::Edit { j, col, value } => {
                    let mut fields = Map::new();
                    fields.insert(col.field().into(), value);
                    self.surface_fields(is_combo, &spans, j, fields, false);
                }
                Act::Fields { j, fields } => {
                    self.surface_fields(is_combo, &spans, j, fields, false)
                }
                Act::Data { j, data } => self.surface_fields(is_combo, &spans, j, data, true),
                Act::Ops(ops) => {
                    if is_combo {
                        match self.combo_ops(&spans, &ids, &ops) {
                            Ok(()) => {
                                if !self.s.auto_build {
                                    self.build_combo(self.doc.combo.project());
                                }
                            }
                            Err(e) => self.fail(e),
                        }
                    } else {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                    }
                }
                Act::DropOverride { j, col } => {
                    if let Some((k, i)) = self.combo_face(&spans, j) {
                        let names = ld_combo::override_fields(col);
                        if self.doc.combo.el(k).is_some_and(|p| p.locked) {
                            self.fail(tr("A locked element cannot be changed"));
                            continue;
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
                    if is_combo {
                        // 組合せの系: 面 after を持つ要素の後（要素の外の面なら、その面より前の要素の後）
                        let at = spans
                            .iter()
                            .rposition(|s| s.is_some_and(|s| s[0] <= after))
                            .map_or(0, |k| k + 1);
                        let k = combo::insert_pick(&mut self.doc.combo, at, *pick);
                        self.p.combo_view.select(k);
                    } else if self.loc.file.is_some() {
                        if open_gap_ok(&self.sys.ld_rows, after) {
                            let n = self.doc.combo.picks.len();
                            self.insert_into_open(Some(after), *pick, n, |_, _| {});
                        } else {
                            self.fail(trf(
                                "Cannot insert after surface {}: not an air gap",
                                &[&after],
                            ));
                        }
                    }
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
                    self.cmds.push(Cmd::FocusKind(Kind::Optimize));
                    self.info(trf("Added {} to the optimization variables", &[&name]));
                }
                Act::NeedData(j) => {
                    self.worker
                        .send(Tag::SurfData(j), "surface_data", json!({ "surface": j }));
                }
            }
        }
        // ブロックの設定（組合せは要素の設定、開いた系は玉・群の偏心・傾き・移動・dn・disp）と、
        // 開いた系の玉・群を足す・機構部品の欄（同じスクロールの中）
        let view = self.p.ld.block_view.clone();
        if view.is_some() {
            egui::ScrollArea::vertical()
                .id_salt("block_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    if let Some(name) = &view {
                        if is_combo {
                            match ids.iter().position(|i| i == name) {
                                Some(k) => combo::block_settings(ui, &mut self.doc.combo, k),
                                None => {
                                    ui.weak(trf(
                                        "{} is not a combination element (select the element for its settings)",
                                        &[name],
                                    ));
                                }
                            }
                        } else {
                            self.unit_settings(ui, name);
                        }
                    }
                });
        }
        // 開いた系の玉・群を足す・機構部品は別の小窓（面の表の下では、面の設定の欄に押し出されて見えないことがあった、#136）
        if !is_combo && self.um_open {
            let mut open = true;
            let n = self.sys.ld_rows.len();
            let mut ops = Vec::new();
            egui::Window::new(tr("Lenses, groups and mechanics"))
                .id(egui::Id::new("unit_mech_win"))
                .open(&mut open)
                .default_width(380.0)
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    ops = crate::unit_mech::ui(ui, &mut self.p.um, n);
                });
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
            self.um_open = open;
        }
    }

    /// 多重構成の次（back なら前）の構成を系に入れる（Ctrl+A・Ctrl+Shift+A、最後の次は最初）
    pub(super) fn cycle_config(&mut self, back: bool) {
        let Some(t) = self.p.mc.table.as_ref().filter(|t| t.n > 1) else {
            return;
        };
        let k = if back {
            if t.active <= 1 {
                t.n
            } else {
                t.active - 1
            }
        } else if t.active >= t.n {
            1
        } else {
            t.active + 1
        };
        self.info(trf("Configuration {} / {}", &[&k, &t.n]));
        self.worker.send(
            Tag::Edit,
            "edit",
            json!({ "ops": [{ "op": "set_config", "k": k }] }),
        );
    }

    /// 多重構成の表
    pub(super) fn config_ui(&mut self, ui: &mut Ui) {
        let combo = self.is_combo();
        let cands = self.param_sys();
        for a in config_panel::ui(ui, &mut self.p.mc, combo, &cands) {
            match a {
                config_panel::Act::Ops(ops) => {
                    self.p.mc.notes.clear();
                    self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                }
                config_panel::Act::Import => {
                    self.worker.send(Tag::ConfigDiff, "config_diff", json!({}));
                }
                config_panel::Act::Refused(why) => self.fail(why),
            }
        }
    }

    /// 開いた系の玉・群の設定（serve の set の `名前.dx` など）
    pub(super) fn unit_settings(&mut self, ui: &mut Ui, name: &str) {
        // 名前・範囲を変える・外す（serve の edit）
        if let Some(u) = self.sys.ld_units.iter().find(|u| u.name == name) {
            let moved = u.motion.iter().any(|&x| x != 0.0) || u.shift != 0.0;
            let spans = u.spans.clone();
            let ops = crate::unit_mech::unit_ui(ui, &mut self.p.um, name, &spans, moved);
            for op in &ops {
                match op["op"].as_str() {
                    Some("remove_unit") => self.p.ld.block_view = None,
                    _ => {
                        if let Some(n) = op["name"].as_str() {
                            self.p.ld.block_view = Some(n.to_string());
                            self.p.ld.sel = Some(n.to_string());
                        }
                    }
                }
            }
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
        }
        let Some(u) = self.sys.ld_units.iter_mut().find(|u| u.name == name) else {
            return;
        };
        let weak = theme::weak_text(ui.visuals());
        let locked = self
            .sys
            .ld_blocks
            .iter()
            .position(|b| b.name == name)
            .is_some_and(|i| lens_data::block_locked(&self.sys.ld_blocks, &self.p.ld.locked, i));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            let spans = u
                .spans
                .iter()
                .map(|s| format!("{}–{}", s[0], s[1]))
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(trf("Surface {}", &[&spans])).color(weak));
            if locked {
                ui.label(RichText::new(trf("{} is locked", &[&name])).color(weak));
            }
        });
        let mut set = Vec::new();
        // ロックは中身（dn・disp）だけを止める。偏心・傾き・移動は変えられる
        {
            ui.horizontal_wrapped(|ui| {
                let labels = [
                    (tr("Decenter x"), " mm"),
                    (tr("Decenter y"), " mm"),
                    (tr("Tilt x"), "°"),
                    (tr("Tilt y"), "°"),
                    (tr("Tilt z"), "°"),
                ];
                for (i, (k, (label, unit))) in ["dx", "dy", "tx", "ty", "tz"]
                    .iter()
                    .zip(labels)
                    .enumerate()
                {
                    ui.label(RichText::new(label).color(weak));
                    let r = ui.add(
                        egui::DragValue::new(&mut u.motion[i])
                            .speed(0.01)
                            .max_decimals(6)
                            .suffix(unit),
                    );
                    if r.changed() {
                        set.push((*k, u.motion[i]));
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Shift")).color(weak))
                    .on_hover_text(tr(
                        "Group shift (positive along the light). Added to the gap before and taken from the gap after",
                    ));
                if ui
                    .add(
                        egui::DragValue::new(&mut u.shift)
                            .speed(0.01)
                            .max_decimals(6)
                            .suffix(" mm"),
                    )
                    .changed()
                {
                    set.push(("shift", u.shift));
                }
                for (k, v) in [("dn", &mut u.dn), ("disp", &mut u.disp)] {
                    ui.label(RichText::new(k).color(weak));
                    match v {
                        Some(x) => {
                            if ui
                                .add_enabled(
                                    !locked,
                                    egui::DragValue::new(x).speed(1e-5).max_decimals(8),
                                )
                                .changed()
                            {
                                set.push((k, *x));
                            }
                        }
                        None => {
                            ui.weak("—").on_hover_text(tr(
                                "No medium, or the value differs per surface (change it in the surface settings)",
                            ));
                        }
                    }
                }
            });
        }
        for (k, v) in set {
            self.pending_set.insert(format!("{name}.{k}"), v);
        }
    }

    /// 組合せの系の面 j の値を、組合せの要素に当てて作り直す
    /// 面 j の値を変える。開いた系は serve の edit（data は set_surface_data、ほかは set_surface）、組合せの系は
    /// 組合せの要素に当てて作り直す
    pub(super) fn surface_fields(
        &mut self,
        is_combo: bool,
        spans: &[Option<[usize; 2]>],
        j: usize,
        fields: Map<String, Value>,
        data: bool,
    ) {
        if !is_combo {
            let op = if data {
                json!({ "op": "set_surface_data", "surface": j, "data": fields })
            } else {
                json!({ "op": "set_surface", "surface": j, "fields": fields })
            };
            self.worker.send(Tag::Edit, "edit", json!({ "ops": [op] }));
            return;
        }
        let why = (self.sys.ld_rows.get(j))
            .and_then(|r| fields.keys().find_map(|k| ld_combo::solved_reason(r, k)));
        if let Some(why) = why {
            self.fail(why);
            return;
        }
        for (key, value) in fields {
            if !self.combo_surface_edit(spans, j, &key, value) {
                return;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
    }

    /// 組合せの系の面 j（系の面の番号）の要素と、要素の面の番号（座標ブレークを除く）
    pub(super) fn combo_face(
        &self,
        spans: &[Option<[usize; 2]>],
        j: usize,
    ) -> Option<(usize, usize)> {
        ld_combo::face_of(&self.sys.ld_rows, spans, j)
    }

    /// 組合せの系の面 j の値 key を、組合せの要素に当てる（要素の最後の面の厚さは後の空気、ほかは要素の中身か面の上書き）
    pub(super) fn combo_surface_edit(
        &mut self,
        spans: &[Option<[usize; 2]>],
        j: usize,
        key: &str,
        value: Value,
    ) -> bool {
        let rows = &self.sys.ld_rows;
        let k = spans
            .iter()
            .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]));
        let x = value.as_f64();
        match (k, key, x) {
            // 前の絞りの厚さ = 絞りから最初の要素まで
            (None, "t", Some(x)) if x >= 0.0 && rows[j].stop => self.doc.combo.stop_gap = x,
            (Some(k), "t", Some(x)) if spans[k].is_some_and(|s| s[1] == j) => {
                // 要素の最後の面の厚さ = 後の空気（鏡の後は符号が逆）
                let air = if rows[j].t < 0.0 { -x } else { x };
                if air < 0.0 {
                    self.fail(tr("The spacing must be 0 or more"));
                    return false;
                }
                match self.doc.combo.gap_after(k) {
                    Some(v) => *v = air,
                    None => {
                        self.fail(tr("This value cannot be changed here"));
                        return false;
                    }
                }
            }
            (Some(k), "material", _) if self.material_reshapes(spans, k, j, &value) => {
                // 面の並びにガラスを入れる・自作の玉の中を空気にする: 要素を玉と面の並びに分け直す（11/16 masa 決定）
                let a = spans[k].map(|s| s[0]).unwrap_or(j);
                let rel = (a..j).filter(|&i| !rows[i].is_cb()).count();
                let glass = value
                    .as_str()
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let r = if glass.is_empty() {
                    self.doc.combo.glass_out_of_lens(k, rel)
                } else {
                    self.doc.combo.glass_into_surfaces(k, rel, &glass)
                };
                if let Err(e) = r {
                    self.fail(e);
                    return false;
                }
            }
            (Some(k), _, _) => {
                let a = spans[k].map(|s| s[0]).unwrap_or(j);
                let rel = (a..j).filter(|&i| !rows[i].is_cb()).count();
                if let Some(p) = self.doc.combo.el_mut(k) {
                    p.write_face(rel, key, value);
                }
            }
            _ => {
                self.fail(tr("This value cannot be changed here"));
                return false;
            }
        }
        true
    }

    /// 硝材の変更が要素の分け直しになるか: 面の並びにガラスを入れる、または自作の玉の最後でない面の後を空気にする
    pub(super) fn material_reshapes(
        &self,
        spans: &[Option<[usize; 2]>],
        k: usize,
        j: usize,
        value: &Value,
    ) -> bool {
        let air = value.as_str().is_none_or(|g| g.trim().is_empty());
        let last = spans[k].is_some_and(|s| s[1] == j);
        match self.doc.combo.el(k).map(|p| p.kind) {
            Some(combo::ElemKind::Surfaces) => !air,
            Some(combo::ElemKind::Lens) => air && !last,
            _ => false,
        }
    }
}
