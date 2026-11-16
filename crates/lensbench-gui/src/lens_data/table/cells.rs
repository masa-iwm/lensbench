//! 表のセル（面の番号・ブロック・値・書き換え中）と行の残り（table.rs から分けた、GPN-03）

use super::*;

/// 面の番号の列（たたんだブロックの行は面の範囲）
pub(super) fn num_cell(ui: &mut Ui, st: &mut LensData, c: &Ctx, v: &View, out: &mut Out, j: usize) {
    let r = &c.rows[j];
    let fold = v.folded(j);
    if let Some((i, a)) = fold {
        let resp = ui.interact(ui.max_rect(), ui.id().with("ld_num"), Sense::click());
        ui.label(RichText::new(format!("{a}–{j}")).monospace().color(v.weak));
        if resp.clicked() || resp.secondary_clicked() {
            out.clicked_any = true;
            let col = st.cur.map_or(Col::R, |x| x.1);
            click_to(st, j, col, v.shift_down, false);
            out.select(st, c, i);
        }
        if resp.double_clicked() {
            out.open_or_enter(c, v.tree, i);
        }
        return;
    }
    let t = if j == 0 {
        "OBJ".to_string()
    } else if j == v.last {
        "IMA".to_string()
    } else if r.stop {
        format!("STO {j}")
    } else {
        j.to_string()
    };
    let mut rt = RichText::new(t).monospace();
    if r.ignore {
        rt = rt.strikethrough().color(v.weak);
    }
    // セル全体で受ける（文字の外の空いたところでも右クリック・クリックが効く）
    let resp = ui.interact(ui.max_rect(), ui.id().with("ld_num"), Sense::click());
    ui.label(rt);
    let resp = if r.ignore {
        resp.on_hover_text(tr("This surface is ignored (rays skip it)"))
    } else {
        resp.on_hover_text(tr(
            "Right-click or Shift+F10 for stop, ignore, insert, delete; double-click for details",
        ))
    };
    if resp.clicked() || resp.secondary_clicked() {
        out.clicked_any = true;
        let col = st.cur.map_or(Col::R, |x| x.1);
        let keep = resp.secondary_clicked() && v.selected.contains(&j);
        click_to(st, j, col, v.shift_down, keep);
    }
    if resp.double_clicked() {
        out.fx.detail = true;
    }
    resp.context_menu(|ui| {
        let m = v.row_menu(ui, &mut st.menu, c, j);
        out.fx.merge(m);
    });
}

/// ブロックの列（字下げと ▶・▼ のブロック名）
pub(super) fn block_cell(
    ui: &mut Ui,
    st: &mut LensData,
    c: &Ctx,
    v: &View,
    out: &mut Out,
    j: usize,
) {
    let fold = v.folded(j);
    let blk = v.child_of(j).filter(|&i| Some(i) != v.scope);
    // セル全体（ブロック名の外）: 行を選ぶ・右クリックで行の欄（面の番号の列と同じ）。
    // ブロック名は後に足すので、名前の上はブロックの操作が勝つ
    let cell = ui.interact(ui.max_rect(), ui.id().with("ld_blk_cell"), Sense::click());
    if cell.clicked() || cell.secondary_clicked() {
        out.clicked_any = true;
        let col = st.cur.map_or(Col::R, |x| x.1);
        let keep = cell.secondary_clicked() && v.selected.contains(&j);
        click_to(st, j, col, v.shift_down, keep);
    }
    if fold.is_none() {
        cell.context_menu(|ui| {
            let m = v.row_menu(ui, &mut st.menu, c, j);
            out.fx.merge(m);
        });
    }
    let (depth, heads) = v
        .line(j)
        .map_or((0, Vec::new()), |l| (l.depth, l.heads.clone()));
    ui.spacing_mut().item_spacing.x = 2.0;
    // 字下げ（開いたブロックの中）
    for _ in 0..depth.saturating_sub(heads.len()) {
        ui.label(RichText::new("│").color(v.weak));
    }
    // この行から始まる開いたブロック（▼）と、たたんだブロック（▶）
    let mut marks: Vec<(usize, bool)> = heads.iter().map(|&i| (i, true)).collect();
    if let (Some(_), Some(i)) = (fold, blk) {
        marks.push((i, false));
    }
    for (i, opened) in marks {
        let b = &c.blocks[i];
        let lk = block_locked(c.blocks, c.locked, i);
        let text = match (v.tree, opened, lk) {
            (false, _, true) => format!("■ {}", b.name),
            (false, _, false) => format!("▶ {}", b.name),
            (true, o, l) => format!(
                "{} {}{}",
                if o { "▼" } else { "▶" },
                b.name,
                if l { " ■" } else { "" }
            ),
        };
        let hint = match (v.tree, opened) {
            (false, _) => tr("Double-click to enter"),
            (true, true) => tr("Click to collapse"),
            (true, false) => tr("Click to expand"),
        };
        let r = ui
            .add(
                egui::Label::new(RichText::new(text).color(v.accent).strong())
                    .sense(Sense::click()),
            )
            .on_hover_text(hint);
        if v.tree && r.clicked() {
            out.toggle = Some(b.name.clone());
        }
        if !v.tree && r.double_clicked() {
            out.enter = Some(b.name.clone());
        } else if r.clicked() {
            st.sel = Some(b.name.clone());
            out.acts.push(Act::Select(st.sel.clone()));
        }
    }
}

/// 値の列のセル: たたんだブロックの行・書き換え中・ふつうのセル
pub(super) fn cell(
    ui: &mut Ui,
    st: &mut LensData,
    c: &Ctx,
    v: &View,
    out: &mut Out,
    j: usize,
    col: Col,
) {
    if let (Some((i, a)), false) = (v.folded(j), col == Col::T) {
        fold_cell(ui, st, c, v, out, j, col, i, a);
        return;
    }
    if let Some(e) = st.edit.as_mut().filter(|e| e.j == j && e.col == col) {
        edit_cell(ui, e, c, v, out, j, col);
        return;
    }
    value_cell(ui, st, c, v, out, j, col);
}

/// たたんだブロックの行のセル: 厚さ（ブロックの後の間隔）だけ見せて変えられる。ほかはダブルクリックで中へ
#[allow(clippy::too_many_arguments)]
fn fold_cell(
    ui: &mut Ui,
    st: &mut LensData,
    c: &Ctx,
    v: &View,
    out: &mut Out,
    j: usize,
    col: Col,
    i: usize,
    a: usize,
) {
    let text = match col {
        Col::Type => tr("Blocks").to_string(),
        Col::Comment => trf(
            "{} surfaces",
            &[&c.blocks[i]
                .surfs
                .iter()
                .filter(|&&x| a <= x && x <= j)
                .count()],
        ),
        _ => String::new(),
    };
    let resp = ui.interact(ui.max_rect(), ui.id().with("ld_fold_cell"), Sense::click());
    ui.add(egui::Label::new(RichText::new(text).color(v.weak)).truncate());
    if st.cur == Some((j, col)) {
        ui.painter().rect_stroke(
            ui.max_rect(),
            2.0,
            egui::Stroke::new(1.5_f32, v.accent),
            egui::StrokeKind::Inside,
        );
    }
    if resp.clicked() || resp.double_clicked() {
        out.clicked_any = true;
        click_to(st, j, col, v.shift_down, false);
        out.select(st, c, i);
    }
    if resp.double_clicked() {
        out.open_or_enter(c, v.tree, i);
    }
}

/// 書き換え中のセル（入力欄。種類・硝材は入力できるドロップダウン）
fn edit_cell(ui: &mut Ui, e: &mut Editing, c: &Ctx, v: &View, out: &mut Out, j: usize, col: Col) {
    let r = &c.rows[j];
    let (tab, shift, up, down, enter_k, esc) = ui.input(|i| {
        (
            i.key_pressed(Key::Tab),
            i.modifiers.shift,
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::Escape),
        )
    });
    // Tab で欄の外へフォーカスを移さない（Tab・↑↓ は決めて隣のセルへ）
    let te = egui::TextEdit::singleline(&mut e.text)
        .desired_width(f32::INFINITY)
        .lock_focus(true);
    let te = if col.numeric() {
        te.horizontal_align(egui::Align::RIGHT)
    } else {
        te
    };
    let resp = ui.add(te);
    if let Some(why) = &e.error {
        ui.painter().rect_stroke(
            resp.rect.expand(1.0),
            2.0,
            egui::Stroke::new(1.5_f32, ui.visuals().error_fg_color),
            egui::StrokeKind::Outside,
        );
        let why = why.clone();
        egui::Area::new(resp.id.with("why"))
            .order(egui::Order::Tooltip)
            .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 2.0))
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        trf("⚠ {} (Esc to cancel)", &[&why]),
                    );
                });
            });
        if resp.changed() {
            e.error = None;
        }
    }
    if !e.focused {
        resp.request_focus();
        if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
            let n = e.text.chars().count();
            let from = if e.select_all { 0 } else { n };
            s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(from),
                egui::text::CCursor::new(n),
            )));
            s.store(ui.ctx(), resp.id);
        }
        e.focused = true;
    }
    let mv = if tab {
        Some((0, if shift { -1 } else { 1 }))
    } else if up && !col.has_list() {
        Some((-1, 0))
    } else if down && !col.has_list() {
        Some((1, 0))
    } else {
        None
    };
    if col.has_list() {
        // 種類・硝材: 入力できるドロップダウン。開いたときは全部の候補から今の値を選び、
        // 打ち始めたら絞る（↑↓ で選び Enter、クリックでも）
        if resp.changed() {
            e.filtered = true;
            e.moved = false;
            e.pick = 0;
            e.scroll = true;
        }
        let fresh = e.choices.as_ref().is_some_and(|x| {
            x.0 == e.text && x.1 == e.filtered && x.2 == c.glasses.len() && x.3 == c.rows.len()
        });
        if !fresh {
            let mut used: Vec<String> = Vec::new();
            if col == Col::Material {
                for x in c.rows {
                    let n = x.material.trim();
                    if !n.is_empty() && !used.iter().any(|u| u.eq_ignore_ascii_case(n)) {
                        used.push(n.to_string());
                    }
                }
            }
            let m = list_choices(col, c.glasses, &used, &e.text, e.filtered);
            e.choices = Some((
                e.text.clone(),
                e.filtered,
                c.glasses.len(),
                c.rows.len(),
                std::sync::Arc::new(m),
            ));
        }
        let m = e.choices.as_ref().map(|x| x.4.clone()).unwrap_or_default();
        if !e.filtered && !e.moved && e.scroll {
            let now = col.text(r);
            e.pick = m
                .iter()
                .position(|x| x.0.eq_ignore_ascii_case(&now))
                .unwrap_or(0);
        }
        if e.pick >= m.len() {
            e.pick = 0;
        }
        if up && e.pick > 0 {
            e.pick -= 1;
            e.moved = true;
            e.scroll = true;
        }
        if down && e.pick + 1 < m.len() {
            e.pick += 1;
            e.moved = true;
            e.scroll = true;
        }
        let mut chosen: Option<String> = None;
        let row_hh = 18.0;
        let scroll_to = std::mem::take(&mut e.scroll).then_some(e.pick);
        let area = egui::Area::new(resp.id.with("choices"))
                                            .order(egui::Order::Foreground)
                                            .fixed_pos(resp.rect.left_bottom())
                                            .show(ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                    ui.set_min_width(if col == Col::Material { 300.0 } else { 220.0 });
                                                    if m.is_empty() {
                                                        ui.weak(if col == Col::Type {
                                                            tr("No matching surface type")
                                                        } else if c.glasses.is_empty() && !c.glasses_loaded {
                                                            tr("Loading glass catalogs… (type a name or a model glass such as 1.5,60 and press Enter)")
                                                        } else {
                                                            tr("No matching glass (names not in the catalogs are refused; write a model glass as nd,νd like 1.5,60)")
                                                        });
                                                        return;
                                                    }
                                                    let mut sa = egui::ScrollArea::vertical()
                                                        .id_salt("choices_scroll")
                                                        .max_height(280.0)
                                                        .auto_shrink([false, true]);
                                                    if let Some(k) = scroll_to {
                                                        sa = sa.vertical_scroll_offset(
                                                            (k as f32 - 5.0).max(0.0) * row_hh,
                                                        );
                                                    }
                                                    ui.spacing_mut().item_spacing.y = 0.0;
                                                    sa.show_rows(ui, row_hh, m.len(), |ui, range| {
                                                        for k in range {
                                                            let (t, d) = &m[k];
                                                            let txt = if d.is_empty() {
                                                                t.clone()
                                                            } else {
                                                                format!("{t}  {d}")
                                                            };
                                                            if ui.selectable_label(k == e.pick, txt).clicked()
                                                            {
                                                                chosen = Some(t.clone());
                                                            }
                                                        }
                                                    });
                                                    if col == Col::Material {
                                                        ui.label(
                                                            RichText::new(trf("{} items. Type to filter (name, catalog, nd)", &[&m.len()]))
                                                                .small()
                                                                .color(v.weak),
                                                        );
                                                    }
                                                });
                                            });
        let hover = area.response.contains_pointer();
        // 種類は選んでいる候補、硝材は ↑↓ で選んだか打った文字で始まる候補（ほかは打った文字のまま）
        let pick = m.get(e.pick).map(|x| x.0.clone()).filter(|x| {
            e.moved
                || (!e.filtered && x.eq_ignore_ascii_case(&e.text))
                || (col == Col::Type && e.filtered)
                || (!e.text.trim().is_empty()
                    && x.to_uppercase().starts_with(&e.text.trim().to_uppercase()))
        });
        if esc {
            out.cancel = true;
        } else if let Some(t) = chosen {
            out.commit = Some((j, col, t));
        } else if enter_k || mv.is_some() {
            out.commit = Some((j, col, pick.unwrap_or(e.text.clone())));
            out.after = mv;
        } else if resp.lost_focus() && !hover && !e.list_hover {
            out.commit = Some((
                j,
                col,
                if e.filtered || e.moved {
                    pick.unwrap_or(e.text.clone())
                } else {
                    e.text.clone()
                },
            ));
        }
        e.list_hover = hover;
    } else if esc {
        out.cancel = true;
    } else if mv.is_some() || resp.lost_focus() {
        out.commit = Some((j, col, e.text.clone()));
        out.after = mv;
    }
    if out.commit.is_some() && out.after.is_some() {
        resp.surrender_focus();
    }
}

/// ふつうのセル（値・▾・[…]・今のセルの枠・右クリックの欄）
fn value_cell(
    ui: &mut Ui,
    st: &mut LensData,
    c: &Ctx,
    v: &View,
    out: &mut Out,
    j: usize,
    col: Col,
) {
    let r = &c.rows[j];
    let fold = v.folded(j);
    let blk = v.child_of(j).filter(|&i| Some(i) != v.scope);
    let text = col.text(r);
    let is_cur = st.cur == Some((j, col));
    let ro = v.cell_ro(c, j, col);
    let mut rt = RichText::new(&text);
    if col.numeric() {
        rt = rt.monospace();
    }
    if col == Col::Type {
        rt = rt.small();
    }
    if ro.is_some() || r.ignore {
        rt = rt.color(v.weak);
    }
    let layout = if col.numeric() {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    let solved = col.solved(r);
    // セル全体でクリックを受ける（文字の外の空いたところでも、このセルを選ぶ）。文字のラベルは
    // クリックを取らない（取ると、文字の上と外で選ばれるセルが変わる）
    let cell_rect = ui.max_rect();
    let resp = ui.interact(cell_rect, ui.id().with("ld_cell"), Sense::click());
    let hover = ui.rect_contains_pointer(cell_rect);
    let mut extra_at: Option<egui::Pos2> = None;
    let mut drop_open = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // 種類・硝材のドロップダウンの ▾（全部の候補から今の値を選んで開く）
        if col.has_list() && ro.is_none() && (is_cur || hover) {
            let (rect, b) = ui.allocate_exact_size(egui::vec2(12.0, ROW_H - 4.0), Sense::click());
            if b.hovered() {
                ui.painter()
                    .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "▼",
                egui::FontId::proportional(8.0),
                v.weak,
            );
            if b.on_hover_text(tr("Choose from the list (type to filter)"))
                .clicked()
            {
                drop_open = true;
            }
        }
        // セルの右端の [...]: ソルブ・ピックアップ（と硝材の追加設定）の欄を開く。
        // ソルブ・ピックアップがあれば S・P（OpticStudio のセルの右の小さな欄のように）
        if col.has_extra() {
            let show = solved.is_some() || is_cur || hover;
            let can = ro.is_none() && show;
            let (rect, b) = ui.allocate_exact_size(
                egui::vec2(14.0, ROW_H - 4.0),
                if can { Sense::click() } else { Sense::hover() },
            );
            if show {
                let (glyph, color) = match &solved {
                    Some((ch, _)) => (ch.to_string(), v.accent),
                    None => ("…".to_string(), v.weak),
                };
                if can && b.hovered() {
                    ui.painter()
                        .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(11.0),
                    color,
                );
            }
            let b = match (&solved, can) {
                (Some((_, why)), _) => b.on_hover_text(why.as_str()),
                (None, true) if col == Col::Material => {
                    b.on_hover_text(tr("Pickup, index offset, dispersion, birefringence"))
                }
                (None, true) => b.on_hover_text(tr("Solve / pickup")),
                _ => b,
            };
            if b.clicked() {
                extra_at = Some(rect.left_bottom());
            }
        }
        ui.with_layout(layout, |ui| {
            ui.add(egui::Label::new(rt).truncate());
        });
    });
    if drop_open {
        out.clicked_any = true;
        click_to(st, j, col, false, false);
        out.start = Some((j, col, None));
    }
    if let Some(pos) = extra_at {
        out.clicked_any = true;
        click_to(st, j, col, false, false);
        st.kbd_menu = Some((j, col, pos, true));
        st.kbd_fresh = true;
    }
    if is_cur {
        out.cur_pos = Some(ui.max_rect().left_bottom());
        if st.hscroll {
            out.hscroll_to = Some(ui.max_rect());
            st.hscroll = false;
        }
        ui.painter().rect_stroke(
            ui.max_rect(),
            2.0,
            egui::Stroke::new(1.5_f32, v.accent),
            egui::StrokeKind::Inside,
        );
    }
    // 組合せの要素の面の上書き: 左上の小さな三角（表計算のメモの印のように）
    let over = (c.overridden)(j, col);
    if over {
        let rc = ui.max_rect();
        let w = 6.0;
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                rc.left_top(),
                rc.left_top() + egui::vec2(w, 0.0),
                rc.left_top() + egui::vec2(0.0, w),
            ],
            crate::theme::warn(ui.visuals()),
            egui::Stroke::NONE,
        ));
    }
    let over_why = || tr("Overridden in this combination (right-click to remove)");
    let resp = match (&ro, &solved) {
        (Some(why), _) => resp.on_hover_text(why),
        (None, Some((_, why))) if over => resp.on_hover_text(format!("{why}\n{}", over_why())),
        (None, Some((_, why))) => resp.on_hover_text(why),
        (None, None) if over => resp.on_hover_text(over_why()),
        _ => resp,
    };
    if resp.clicked() || resp.double_clicked() || resp.secondary_clicked() {
        out.clicked_any = true;
        // 範囲を選んでいる行の右クリックは範囲を残す（今のセルも動かさない）
        let keep = resp.secondary_clicked() && v.selected.len() > 1 && v.selected.contains(&j);
        click_to(st, j, col, v.shift_down, keep);
        if let Some(i) = blk.filter(|_| fold.is_some() || !st.by_surface) {
            out.select(st, c, i);
        }
    }
    if resp.double_clicked() {
        match &ro {
            Some(why) => out.acts.push(Act::Refused(why.clone())),
            None => out.start = Some((j, col, None)),
        }
    }
    resp.context_menu(|ui| {
        out.fx.acts.extend(override_item(ui, c, j, col));
        if ro.is_none() {
            out.fx.acts.extend(cell_menu(
                ui,
                &mut st.menu,
                j,
                col,
                r,
                v.bire_of(j),
                c.combo,
                (c.config_n, c.config_rows),
            ));
        }
        let m = v.row_menu(ui, &mut st.menu, c, j);
        out.fx.merge(m);
    });
}

/// 行の残り（セルの文字の外のクリック・カタログの行を落とす）
pub(super) fn row_end(
    rr: &egui::Response,
    st: &mut LensData,
    c: &Ctx,
    v: &View,
    out: &mut Out,
    j: usize,
) {
    let fold = v.folded(j);
    let blk = v.child_of(j).filter(|&i| Some(i) != v.scope);
    // カタログの行を落とす: 行の下の線に差し込む（たたんだブロックの行はブロックの後、像面には落とせない）
    if j + 1 < c.rows.len()
        && egui::DragAndDrop::has_payload_of_type::<crate::combo::DropPick>(&rr.ctx)
        && rr
            .ctx
            .pointer_hover_pos()
            .is_some_and(|q| rr.rect.contains(q))
    {
        let drop_color = rr.ctx.style().visuals.selection.stroke.color;
        rr.ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("ld_drop"),
            ))
            .line_segment(
                [rr.rect.left_bottom(), rr.rect.right_bottom()],
                egui::Stroke::new(2.5_f32, drop_color),
            );
        if rr.ctx.input(|i| i.pointer.any_released()) {
            if let Some(p) = egui::DragAndDrop::take_payload::<crate::combo::DropPick>(&rr.ctx) {
                out.acts.push(Act::DropPick {
                    after: j,
                    pick: Box::new(p.0.clone()),
                });
            }
        }
    }
    if rr.clicked() || rr.double_clicked() {
        out.clicked_any = true;
        let col = st.cur.map_or(Col::R, |x| x.1);
        click_to(st, j, col, v.shift_down, false);
        st.edit = None;
        if let Some(i) = blk.filter(|_| fold.is_some() || !st.by_surface) {
            out.select(st, c, i);
        }
        if let (true, Some((i, _))) = (rr.double_clicked(), fold) {
            out.open_or_enter(c, v.tree, i);
        }
    }
}
