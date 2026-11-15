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
            "階層: {}（クリックで一覧・表示の切り替え。Backspace で 1 つ上へ）",
            &[n],
        ),
        None => tr("階層: 系の全部の面（クリックで一覧・表示の切り替え）").to_string(),
    };
    let r = icon(ui, "🗀", here.is_some()).on_hover_text(hint);
    popup(ui, &r, "ld_scope", |ui| {
        ui.label(RichText::new(tr("階層")).strong());
        if ui
            .selectable_label(st.scope.is_empty(), tr("系（全部の面）"))
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
                        "面 {}〜{}（クリックで中へ）",
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
        ui.label(RichText::new(tr("表示")).strong());
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!st.tree, tr("階層"))
                .on_hover_text(tr(
                    "ブロックを 1 行にたたみ、ダブルクリックで中に入って面を変える",
                ))
                .clicked()
            {
                st.tree = false;
            }
            if ui
                .selectable_label(st.tree, tr("折りたたみ"))
                .on_hover_text(tr(
                    "ブロックをその場で開いて（▶/▼）、中に入らずに面を変える",
                ))
                .clicked()
            {
                st.tree = true;
            }
        });
        if st.tree && !inside.is_empty() {
            ui.horizontal(|ui| {
                if ui.small_button(tr("全部開く")).clicked() {
                    st.open.extend(inside.iter().cloned());
                }
                if ui.small_button(tr("全部たたむ")).clicked() {
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
            ("▶", tr("全部たたむ（ブロックを 1 行に）"))
        } else {
            ("▼", tr("全部開く（面を全部見せる）"))
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
            Some(n) => trf("ブロック: {}（クリックで選ぶ・ロック・中へ）", &[n]),
            None => tr("ブロック（クリックで選ぶ・ロック・中へ）").to_string(),
        };
        let r = icon(ui, "▦", st.sel.is_some()).on_hover_text(hint);
        popup(ui, &r, "ld_block", |ui| {
            ui.label(RichText::new(tr("ブロック")).strong());
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
                        "面 {}〜{}",
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
                    .checkbox(&mut lk, trf("{} をロック", &[&name]))
                    .on_hover_text(tr(
                        "ロックしたブロック（と中のブロック）の面は変えられない（Ctrl+L でも切り替え）",
                    ))
                    .changed()
                {
                    acts.push(Act::Lock(name.clone(), lk));
                }
                ui.horizontal(|ui| {
                    if tree {
                        let label = if st.open.contains(&name) {
                            tr("たたむ")
                        } else {
                            tr("開く")
                        };
                        if ui
                            .button(label)
                            .on_hover_text(tr("ダブルクリック・Enter でも"))
                            .clicked()
                        {
                            out.toggle = Some(name.clone());
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                    if ui
                        .button(tr("中へ"))
                        .on_hover_text(if tree {
                            tr("ブロックの中だけを見る")
                        } else {
                            tr("ダブルクリック・Ctrl+Enter でも")
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
            tr("選び方: 面単位（選んだ面だけをハイライト）。クリックでブロック単位に"),
        )
    } else {
        (
            "▣",
            tr("選び方: ブロック単位（面を選ぶとそのブロック全体を選ぶ）。クリックで面単位に"),
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
            .on_hover_text(tr("選んだ行の前に面を挿入（Insert。後は Alt+Insert）"))
            .clicked()
        {
            out.fx.insert = true;
            out.fx.insert_before = true;
        }
        out.fx.delete |= icon(ui, "－", false)
            .on_hover_text(tr("選んだ面を消す（Delete）"))
            .clicked();
    }
    // 表の下の詳細設定（標準的な設定は表で。開口・傾き偏心・形のデータ・ブロックの設定は詳細で）
    if icon(ui, "⚙", st.detail)
        .on_hover_text(tr(
            "表の下に面の詳細設定（開口・前後の傾き偏心・形のデータ・全項目 JSON）とブロックの設定を出す。面の番号のダブルクリックでも",
        ))
        .clicked()
    {
        st.detail = !st.detail;
    }
    if let Some(on) = c.unit_mech {
        if icon(ui, "⊞", on)
            .on_hover_text(tr(
                "面の範囲で玉・群（ズーム群・フォーカス群など）を作る・機構部品を置く。範囲は選んでいる面から始まる",
            ))
            .clicked()
        {
            acts.push(Act::UnitMech);
        }
    }
    // 選んだ面の範囲（玉・群）だけを系ファイルに書き出す（系全体の保存はファイルのメニュー・Ctrl+S）
    out.export_btn |= icon(ui, "📤", false)
        .on_hover_text(tr(
            "選んだ面の範囲（玉・群）だけを系ファイルに書き出す…（Shift で範囲を選ぶ。系全体の保存はファイル → 保存）",
        ))
        .clicked();
    if let Some(note) = c.note {
        ui.label(RichText::new("ℹ").size(15.0).color(weak))
            .on_hover_text(note);
    }
}
