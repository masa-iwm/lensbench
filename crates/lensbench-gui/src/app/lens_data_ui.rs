//! レンズデータの表・状態の表のパネルと、組合せの系の面の書き換え（app.rs から分けた）

use super::*;

impl Core {
    /// 面の表: 組合せ（要素の後の空気・面の上書き）を変えて作り直す
    pub(super) fn lens_data_ui(&mut self, ui: &mut Ui) {
        use lens_data::Col;
        if self.sys.summary.is_none() || self.sys.ld_rows.is_empty() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from the combination to see its surfaces here.",
            ));
            // 空の文書: 平面 1 枚から面を足していける（11/16 masa 決定: 表から自由に面を足す）
            if self.doc.combo.picks.is_empty() {
                ui.add_space(4.0);
                if ui
                    .button(tr("+ Add a surface"))
                    .on_hover_text(tr(
                        "Starts a surface list with one plane surface. Insert more surfaces from the table, and set a glass after a surface to make a lens",
                    ))
                    .clicked()
                {
                    edit::stage(&mut self.doc.combo, &mut self.msgs, |c| c.insert_plane(0));
                }
            }
            return;
        }
        let ids = self.doc.combo.ids();
        // 図で選んだ要素を表でも選ぶ（最上位のとき）
        if self.p.ld.scope.is_empty() {
            if let Some(k) = self.p.combo_view.sel() {
                self.p.ld.sel = ids.get(k).cloned();
            }
        }
        // 今の経路に沿った要素（分岐の無い組合せは並びそのもの）
        let picks = &self.doc.combo.chain_view().picks;
        let locked: BTreeSet<String> = ids
            .iter()
            .zip(picks)
            .filter(|(_, p)| p.locked)
            .map(|(i, _)| i.clone())
            .collect();
        let lead = self.doc.combo.lead_stop();
        let live = self.doc.combo.span_ids();
        let spans = lens_data::elem_spans(&live, &self.sys.elem_blocks, lead, &self.sys.ld_rows);
        // 図で別の要素を選んだら、その要素の外の今の面は外す（図の目立たせと表を合わせる）
        if self.p.combo_view.take_moved() {
            if let (Some(k), Some((j, _))) = (self.p.combo_view.sel(), self.p.ld.cur) {
                let inside = spans
                    .get(k)
                    .copied()
                    .flatten()
                    .is_some_and(|s| s[0] <= j && j <= s[1]);
                if !inside {
                    self.p.ld.cur = None;
                }
            }
            // 先頭に自動で置く絞りを選んだ: 絞りの面の行に印（今の面は外す）
            if self.p.combo_view.lead() {
                self.p.ld.cur = None;
            }
        }
        // 表のブロックで選べない要素（BS の面・先頭に自動で置く絞りの面は表のブロックの名前を持たない）は、
        // 要素の面の行を選んだ印にする。表で範囲の外の行を選んだら外す
        self.p.ld.hl = if !self.p.ld.scope.is_empty() {
            Vec::new()
        } else if self.p.combo_view.lead() && lead {
            self.sys
                .ld_rows
                .iter()
                .position(|r| r.stop)
                .map(|j| vec![[j, j]])
                .unwrap_or_default()
        } else {
            match self.p.combo_view.sel().and_then(|k| ids.get(k)) {
                Some(id) if !self.sys.ld_blocks.iter().any(|b| &b.name == id) => self
                    .sys
                    .elem_blocks
                    .iter()
                    .find(|(b, _)| b == id)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        };
        if let Some((j, _)) = self.p.ld.cur {
            if !self.p.ld.hl.iter().any(|r| r[0] <= j && j <= r[1]) {
                self.p.ld.hl.clear();
            }
        }
        let rows = &self.sys.ld_rows;
        let read_only = |j: usize, col: Col| -> Option<String> {
            ld_combo::read_only(rows, &spans, picks, lead, j, col)
        };
        let overridden = |j: usize, col: Col| ld_combo::overridden(rows, &spans, picks, j, col);
        let ctx = lens_data::Ctx {
            glasses: &self.ld_glasses,
            glasses_loaded: self.browser.loaded && !self.browser.glasses_pending,
            rows,
            blocks: &self.sys.ld_blocks,
            locked: &locked,
            read_only: &read_only,
            overridden: &overridden,
            note: Some(tr(
                "Combination system: changes go into the combination elements (the trailing air is the element spacing). Catalog lenses and imported elements keep them as surface overrides, marked in the cell (right-click to remove). Set decenter, tilt and BS in the element settings.",
            )),
        };
        let (acts, mut bottom) = lens_data::ui(ui, &mut self.p.ld, &ctx);
        for a in acts {
            self.msgs.push(Msg::LensData(a));
        }
        // ブロックの設定（組合せの要素の設定）
        let view = self.p.ld.block_view.clone();
        if view.is_some() {
            egui::ScrollArea::vertical()
                .id_salt("block_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    if let Some(name) = &view {
                        match ids.iter().position(|i| i == name) {
                            Some(k) => edit::stage(&mut self.doc.combo, &mut self.msgs, |c| {
                                combo::block_settings(ui, c, k)
                            }),
                            None => {
                                ui.weak(trf(
                                    "{} is not a combination element (select the element for its settings)",
                                    &[name],
                                ));
                            }
                        }
                    }
                });
        }
    }

    /// 組合せの次（back なら前）の状態にする（Ctrl+A・Ctrl+Shift+A、最後の次は最初）
    pub(super) fn cycle_config(&mut self, back: bool) {
        let c = &mut self.doc.combo;
        let n = c.states.len();
        if n < 2 {
            return;
        }
        let now = c
            .active_state()
            .and_then(|a| c.states.iter().position(|s| s == a))
            .unwrap_or(0);
        let k = if back {
            (now + n - 1) % n
        } else {
            (now + 1) % n
        };
        let name = c.states[k].clone();
        c.state = Some(name.clone());
        self.info(trf("State {} ({} / {})", &[&name, &(k + 1), &n]));
    }

    /// 状態の表（変えた値は組合せの変更として組み直す）
    pub(super) fn config_ui(&mut self, ui: &mut Ui) {
        let (faces, cbs) = (self.face_index(), self.face_index_of(true));
        let rows = &self.sys.ld_rows;
        let now =
            |f: &crate::state_table::Face| -> Option<Value> { face_now(rows, &faces, &cbs, f) };
        // 絞りにできる面: 今の系の要素の面（経路の順、面の順）
        let mut stops: Vec<(crate::combo_tree::Loc, usize)> = faces.keys().cloned().collect();
        stops.sort_by_key(|k| faces[k]);
        edit::stage(&mut self.doc.combo, &mut self.msgs, |c| {
            crate::state_table::ui(ui, c, &mut self.p.states, &now, &stops)
        });
    }

    /// 多重構成のピックアップを当てる（組合せを変えたあと・面の表を受け取ったあと）。変えたか
    pub(super) fn apply_mc_picks(&mut self) -> bool {
        if self.doc.combo.states.is_empty() {
            return false;
        }
        let (faces, cbs) = (self.face_index(), self.face_index_of(true));
        let rows = &self.sys.ld_rows;
        let now =
            |f: &crate::state_table::Face| -> Option<Value> { face_now(rows, &faces, &cbs, f) };
        crate::state_table::apply_picks(&mut self.doc.combo, &self.p.states.base_cache, &now)
    }

    /// 組合せの要素の面（場所・要素の面の番号）→ 今の系の面の番号（今の経路の要素だけ）
    pub(super) fn face_index(&self) -> HashMap<(crate::combo_tree::Loc, usize), usize> {
        self.face_index_of(false)
    }

    /// 今の系の要素の面（cb なら座標ブレークだけを数えた番号、でなければ座標ブレークを除いた番号）→ 系の面
    fn face_index_of(&self, cb: bool) -> HashMap<(crate::combo_tree::Loc, usize), usize> {
        let mut out = HashMap::new();
        let rows = &self.sys.ld_rows;
        if rows.is_empty() {
            return out;
        }
        let lead = self.doc.combo.lead_stop();
        let live = self.doc.combo.span_ids();
        let spans = lens_data::elem_spans(&live, &self.sys.elem_blocks, lead, rows);
        let c = &self.doc.combo;
        for (step, span) in c.chain().iter().zip(spans) {
            let Some([a, b]) = span else { continue };
            let mut js: Vec<usize> = (a..=b)
                .filter(|&j| rows.get(j).is_some_and(|r| r.is_cb() == cb))
                .collect();
            // 偏心・傾きを付けた要素の入口・戻りの座標ブレークは要素の面ではない（要素の座標ブレークの番号に数えない）。
            // 鏡の座標ブレークは傾きのものだけ
            let p = c.get(&step.loc);
            if cb && p.is_some_and(|p| p.kind == crate::combo::ElemKind::Mirror) {
                continue;
            }
            let moved = p.is_some_and(|p| {
                let pl = p.place_in(c.active_state());
                p.has_state_place() && pl.decenter.iter().chain(&pl.tilt).any(|v| *v != 0.0)
            });
            if cb && moved {
                if js.len() < 2 {
                    continue;
                }
                js.pop();
                js.remove(0);
            }
            for (i, j) in js.into_iter().enumerate() {
                out.insert((step.loc.clone(), i), j);
            }
        }
        out
    }

    /// 組合せの系の面 j の値を、組合せの要素に当てて作り直す
    pub(super) fn surface_fields(
        &mut self,
        spans: &[Option<[usize; 2]>],
        j: usize,
        fields: Map<String, Value>,
    ) {
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

/// 状態の表の面の行の今の系の値（座標ブレークの項目は座標ブレークの番号で引く）
fn face_now(
    rows: &[crate::lens_data::Row],
    faces: &HashMap<(crate::combo_tree::Loc, usize), usize>,
    cbs: &HashMap<(crate::combo_tree::Loc, usize), usize>,
    f: &crate::state_table::Face,
) -> Option<Value> {
    let (map, field) = match f.field.strip_prefix(crate::combo_tree::CB_FIELD) {
        Some(k) => (cbs, k),
        None => (faces, f.field.as_str()),
    };
    let j = *map.get(&(f.loc.clone(), f.surf))?;
    ld_combo::face_value(rows.get(j)?, field)
}
