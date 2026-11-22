//! 表のセル（面の番号・ブロック・値・書き換え中）と行の残り（table.rs から分けた）

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
    // 字下げと印は 1 行に並べる（セルは上から下の並びなので、そのまま足すと入れ子の行で 2 段に折れ、行が
    // ROW_H より高くなって表が最後の行まで送れなかった）
    // （horizontal は中身の高さに合わせて伸びるので、セルの高さの中で左から並べる）
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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
            // 群の名前の右クリック: 群を解除
            if c.groups.iter().any(|g| g.0 == b.name) {
                r.context_menu(|ui| {
                    if ui.button(trf("Ungroup {}", &[&b.name])).clicked() {
                        out.acts.push(Act::Ungroup(b.name.clone()));
                        ui.close_menu();
                    }
                });
            }
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
    });
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
        edit_cell(ui, e, c, out, j, col);
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
fn edit_cell(ui: &mut Ui, e: &mut Editing, c: &Ctx, out: &mut Out, j: usize, col: Col) {
    let r = &c.rows[j];
    // Tab で欄の外へフォーカスを移さない（Tab・↑↓ は決めて隣のセルへ。種類・硝材の ↑↓ は候補を選ぶ）
    let (resp, keys) = crate::widgets::cell::input(
        ui,
        &mut e.text,
        &mut e.focused,
        e.select_all,
        col.numeric(),
        e.error.as_deref(),
    );
    if e.error.is_some() && resp.changed() {
        e.error = None;
    }
    let (up, down, enter_k, esc) = (keys.up, keys.down, keys.enter, keys.esc);
    let mv = keys.nav(!col.has_list());
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
            // 候補が変わった（開いた後にガラスカタログが届いたなど）: 打っても選んでもいなければ、今の値を選び直す
            // （前の候補での位置のままだと、同じ位置の別の硝材が選ばれる）
            if !e.filtered && !e.moved {
                e.scroll = true;
            }
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
        let scroll_to = std::mem::take(&mut e.scroll).then_some(e.pick);
        let empty = if col == Col::Type {
            tr("No matching surface type")
        } else if c.glasses.is_empty() && !c.glasses_loaded {
            tr("Loading glass catalogs… (type a name or a model glass such as 1.5,60 and press Enter)")
        } else {
            tr("No matching glass (names not in the catalogs are refused; write a model glass as nd,νd like 1.5,60)")
        };
        let shown = crate::widgets::choices::Choices {
            scroll_to,
            min_width: if col == Col::Material { 300.0 } else { 220.0 },
            empty: Some(empty),
            footer: (col == Col::Material)
                .then(|| trf("{} items. Type to filter (name, catalog, nd)", &[&m.len()])),
            ..crate::widgets::choices::Choices::new(&m, e.pick)
        }
        .show(ui, resp.id.with("choices"), resp.rect);
        let chosen = shown.clicked.and_then(|k| m.get(k)).map(|x| x.0.clone());
        let hover = shown.hovered;
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
    let is_cur = st.cur == Some((j, col));
    let ro = v.cell_ro(c, j, col);
    // Alt+ドラッグで数を増減（共通の表と同じ量、widgets::num::drag_value）。離したときに 1 回で決める
    let dd = egui::Id::new(("ld_num_drag", j, format!("{col:?}")));
    let drag = ui.data(|d| d.get_temp::<(f64, f32)>(dd));
    let text = match drag {
        Some((x0, dx)) => crate::json_view::fmt_num(crate::widgets::num::drag_value(x0, dx, false)),
        None => col.text(r),
    };
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
    let draggable = col.numeric() && ro.is_none();
    let sense = if draggable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let resp = ui.interact(cell_rect, ui.id().with("ld_cell"), sense);
    num_drag(ui, &resp, dd, draggable, &text, drag);
    if let (true, Some((x0, dx))) = (resp.drag_stopped(), drag) {
        let x = crate::widgets::num::drag_value(x0, dx, false);
        out.acts
            .extend(commit_act(r, j, col, &crate::json_view::fmt_num(x)));
        ui.data_mut(|d| d.remove::<(f64, f32)>(dd));
    }
    // AccessKit の木に名前（面の番号と列）と値を出す（読み上げ・画面の試験がセルを文言で探せる）
    resp.widget_info(|| {
        let mut w = egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ro.is_none(),
            trf("Surface {} {}", &[&j, &col.header(Some(&r.typ))]),
        );
        w.current_text_value = Some(text.clone());
        w
    });
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
        match blk.filter(|_| fold.is_some() || !st.by_surface) {
            Some(i) => out.select(st, c, i),
            None => out.unselect(st),
        }
    }
    if resp.double_clicked() {
        match &ro {
            Some(why) => out.acts.push(Act::Refused(why.clone())),
            None => out.start = Some((j, col, None)),
        }
    }
    resp.context_menu(|ui| {
        if ro.is_none() {
            out.fx.acts.extend(override_item(ui, c, j, col));
            out.fx
                .acts
                .extend(cell_menu(ui, &mut st.menu, j, col, r, v.bire_of(j)));
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
        match blk.filter(|_| fold.is_some() || !st.by_surface) {
            Some(i) => out.select(st, c, i),
            None => out.unselect(st),
        }
        if let (true, Some((i, _))) = (rr.double_clicked(), fold) {
            out.open_or_enter(c, v.tree, i);
        }
    }
}

/// 数のセルの Alt+ドラッグ: 始めた値と動かした量（点、Shift で 1/10）を egui の一時の記憶に置く
fn num_drag(
    ui: &Ui,
    resp: &egui::Response,
    dd: egui::Id,
    on: bool,
    text: &str,
    drag: Option<(f64, f32)>,
) {
    let alt = ui.input(|i| i.modifiers.alt);
    if on && (drag.is_some() || (alt && resp.hovered())) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if on && alt && resp.drag_started() {
        if let Ok(x) = text.trim().parse::<f64>() {
            ui.data_mut(|d| d.insert_temp(dd, (x, 0.0_f32)));
        }
    } else if let (Some((x0, dx)), true) = (drag, resp.dragged()) {
        let k = if ui.input(|i| i.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        let dx = dx + resp.drag_delta().x * k;
        ui.data_mut(|d| d.insert_temp(dd, (x0, dx)));
    }
}
