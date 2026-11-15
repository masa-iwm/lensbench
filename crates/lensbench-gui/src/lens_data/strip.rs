//! レンズデータの表の左端の縦の帯（lens_data.rs の ui から分けた、GPN-03）

use super::*;

/// 左端の縦の帯の中で決めたこと（[`ui`] が後でまとめて当てる）
#[derive(Default)]
pub(super) struct StripOut {
    /// 階層の一覧で選んだ行き先（scope の長さ）
    pub(super) go: Option<usize>,
    /// 入るブロック
    pub(super) enter: Option<String>,
    pub(super) export_btn: bool,
    pub(super) toggle: Option<String>,
    pub(super) fx: RowMenu,
}

/// 帯を描くのに要る、表の側で求めた値
pub(super) struct StripCtx<'a> {
    pub(super) inside: &'a [String],
    pub(super) wide: bool,
    pub(super) strip_w: f32,
    pub(super) weak: egui::Color32,
    pub(super) children: &'a [usize],
    pub(super) tree: bool,
}

/// 左端の縦の帯: 階層・ブロック・面の挿入削除・詳細設定・玉群・保存のアイコン（[`ui`] から分けた、GPN-03）
pub(super) fn strip_ui(
    ui: &mut Ui,
    st: &mut LensData,
    c: &Ctx,
    sc: &StripCtx,
    out: &mut StripOut,
    acts: &mut Vec<Act>,
) {
    let StripCtx {
        inside,
        wide,
        strip_w,
        weak,
        children,
        tree,
    } = *sc;
    ui.spacing_mut().item_spacing.y = 4.0;
    let icon = |ui: &mut Ui, t: &str, on: bool| -> egui::Response {
        // 記号がフォントに無ければ（豆腐 □ になる）字だけにする
        let has = ui.fonts(|f| f.has_glyphs(&egui::FontId::proportional(15.0), t));
        let (text, w) = match icon_label(t).filter(|_| wide || !has) {
            Some(l) if !has => (RichText::new(tr(l)).size(11.0), strip_w.max(40.0) - 4.0),
            Some(l) => (
                RichText::new(format!("{t} {}", tr(l))).size(12.0),
                strip_w - 4.0,
            ),
            None => (RichText::new(t).size(15.0), 26.0),
        };
        ui.add(
            egui::Button::new(text)
                .selected(on)
                .min_size(egui::vec2(w, 26.0)),
        )
    };
    // 階層: 系と全部のブロックの入れ子の一覧（クリックでそのブロックに入る）と、表示の切り替え
    let here = st.scope.last().cloned();
    let hint = match &here {
        Some(n) => trf(
            "Level: {} (click for the list and view mode; Backspace goes up)",
            &[n],
        ),
        None => tr("Level: all surfaces (click for the list and view mode)").to_string(),
    };
    let r = icon(ui, "🗀", here.is_some()).on_hover_text(hint);
    popup(ui, &r, "ld_scope", |ui| {
        ui.label(RichText::new(tr("Drill-down")).strong());
        if ui
            .selectable_label(st.scope.is_empty(), tr("System (all surfaces)"))
            .clicked()
        {
            out.go = Some(0);
            ui.memory_mut(|m| m.close_popup());
        }
        for (i, b) in c.blocks.iter().enumerate() {
            let mut depth = 0;
            let mut p = b.parent;
            while let Some(q) = p {
                depth += 1;
                p = c.blocks[q].parent;
            }
            let lk = block_locked(c.blocks, c.locked, i);
            ui.horizontal(|ui| {
                ui.add_space(12.0 * (depth + 1) as f32);
                let text = if lk {
                    format!("■ {}", b.name)
                } else {
                    b.name.clone()
                };
                let in_path = st.scope.contains(&b.name);
                let mut rt = RichText::new(text);
                if in_path {
                    rt = rt.strong();
                }
                if ui
                    .selectable_label(here.as_deref() == Some(b.name.as_str()), rt)
                    .on_hover_text(trf(
                        "Surfaces {}–{} (click to enter)",
                        &[&b.surfs[0], &b.surfs[b.surfs.len() - 1]],
                    ))
                    .clicked()
                {
                    out.enter = Some(b.name.clone());
                    ui.memory_mut(|m| m.close_popup());
                }
            });
        }
        ui.separator();
        ui.label(RichText::new(tr("View")).strong());
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!st.tree, tr("Drill-down"))
                .on_hover_text(tr(
                    "Fold each block to one row; double-click to go inside and edit its surfaces",
                ))
                .clicked()
            {
                st.tree = false;
            }
            if ui
                .selectable_label(st.tree, tr("Collapsible"))
                .on_hover_text(tr(
                    "Expand blocks in place (▶/▼) and edit their surfaces without going inside",
                ))
                .clicked()
            {
                st.tree = true;
            }
        });
        if st.tree && !inside.is_empty() {
            ui.horizontal(|ui| {
                if ui.small_button(tr("Expand all")).clicked() {
                    st.open.extend(inside.iter().cloned());
                }
                if ui.small_button(tr("Collapse all")).clicked() {
                    for n in inside {
                        st.open.remove(n);
                    }
                }
            });
        }
    });
    // 全部開く・全部たたむを帯に常に出す（折りたたみ表示のとき）
    if st.tree && !inside.is_empty() {
        let all_open = inside.iter().all(|n| st.open.contains(n));
        let (t, tip) = if all_open {
            ("▶", tr("Collapse all (blocks on one row)"))
        } else {
            ("▼", tr("Expand all (show every surface)"))
        };
        if icon(ui, t, false).on_hover_text(tip).clicked() {
            if all_open {
                for n in inside {
                    st.open.remove(n);
                }
            } else {
                st.open.extend(inside.iter().cloned());
            }
        }
    }
    // ブロック: 今の階層の直下のブロックのドロップダウンと、選んだブロックのロック・開く・中へ
    if !children.is_empty() || st.sel.is_some() {
        let hint = match &st.sel {
            Some(n) => trf("Block: {} (click to select, lock, enter)", &[n]),
            None => tr("Blocks (click to select, lock, enter)").to_string(),
        };
        let r = icon(ui, "▦", st.sel.is_some()).on_hover_text(hint);
        popup(ui, &r, "ld_block", |ui| {
            ui.label(RichText::new(tr("Blocks")).strong());
            for &i in children {
                let b = &c.blocks[i];
                let lk = block_locked(c.blocks, c.locked, i);
                let text = if lk {
                    format!("■ {}", b.name)
                } else {
                    b.name.clone()
                };
                let r = ui
                    .selectable_label(st.sel.as_deref() == Some(b.name.as_str()), text)
                    .on_hover_text(trf(
                        "Surfaces {}–{}",
                        &[&b.surfs[0], &b.surfs[b.surfs.len() - 1]],
                    ));
                if r.clicked() {
                    st.sel = Some(b.name.clone());
                    acts.push(Act::Select(st.sel.clone()));
                }
            }
            if let Some(name) = st.sel.clone() {
                ui.separator();
                let mut lk = c.locked.contains(&name);
                if ui
                    .checkbox(&mut lk, trf("Lock {}", &[&name]))
                    .on_hover_text(tr(
                        "Surfaces of a locked block (and its inner blocks) cannot be changed (Ctrl+L also toggles)",
                    ))
                    .changed()
                {
                    acts.push(Act::Lock(name.clone(), lk));
                }
                ui.horizontal(|ui| {
                    if tree {
                        let label = if st.open.contains(&name) {
                            tr("Collapse")
                        } else {
                            tr("Expand")
                        };
                        if ui
                            .button(label)
                            .on_hover_text(tr("Also by double-click or Enter"))
                            .clicked()
                        {
                            out.toggle = Some(name.clone());
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                    if ui
                        .button(tr("Enter"))
                        .on_hover_text(if tree {
                            tr("Show only the inside of the block")
                        } else {
                            tr("Also by double-click or Ctrl+Enter")
                        })
                        .clicked()
                    {
                        out.enter = Some(name.clone());
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
            }
        });
    }
    // 選び方: ブロック単位（面を選ぶとそのブロックも選んでハイライト）・面単位（選んだ面だけ）
    let (t, hint) = if st.by_surface {
        (
            "▭",
            tr("Selection: by surface (only the chosen surfaces are highlighted). Click for by block"),
        )
    } else {
        (
            "▣",
            tr("Selection: by block (choosing a surface selects its whole block). Click for by surface"),
        )
    };
    if icon(ui, t, st.by_surface).on_hover_text(hint).clicked() {
        st.by_surface = !st.by_surface;
        if st.by_surface {
            st.sel = None;
            acts.push(Act::Select(None));
        }
    }
    ui.separator();
    // 面の挿入・削除（開いた系だけ。Insert・Delete キー・右クリックでも）
    if !c.combo {
        if icon(ui, "＋", false)
            .on_hover_text(tr(
                "Insert a surface before the selected row (Insert; Alt+Insert inserts after)",
            ))
            .clicked()
        {
            out.fx.insert = true;
            out.fx.insert_before = true;
        }
        out.fx.delete |= icon(ui, "－", false)
            .on_hover_text(tr("Delete the selected surface (Delete)"))
            .clicked();
    }
    // 表の下の詳細設定（標準的な設定は表で。開口・傾き偏心・形のデータ・ブロックの設定は詳細で）
    if icon(ui, "⚙", st.detail)
        .on_hover_text(tr(
            "Show surface details (aperture, tilt/decenter before and after, shape data, all items as JSON) and block settings below the table. Also by double-clicking a surface number",
        ))
        .clicked()
    {
        st.detail = !st.detail;
    }
    if let Some(on) = c.unit_mech {
        if icon(ui, "⊞", on)
            .on_hover_text(tr(
                "Create lenses/groups (zoom, focus groups, …) from surface ranges and place mechanical parts. The range starts at the selected surface",
            ))
            .clicked()
        {
            acts.push(Act::UnitMech);
        }
    }
    // 選んだ面の範囲（玉・群）だけを系ファイルに書き出す（系全体の保存はファイルのメニュー・Ctrl+S）
    out.export_btn |= icon(ui, "📤", false)
        .on_hover_text(tr(
            "Export only the selected surface range (lens/group) to a system file… (Shift selects a range; save the whole system with File → Save)",
        ))
        .clicked();
    if let Some(note) = c.note {
        ui.label(RichText::new("ℹ").size(15.0).color(weak))
            .on_hover_text(note);
    }
}
