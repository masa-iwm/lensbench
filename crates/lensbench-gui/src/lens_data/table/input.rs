//! 表のキー・キーボードの欄・面の挿入削除と、決めたことを当てる（table.rs から分けた）

use super::*;

/// キー（表をクリックした後。書き換え中は入力欄が使う）: 移動・範囲・書き換えの開始・コピー・貼り付け・挿入・削除
pub(super) fn key_input(ui: &mut Ui, st: &mut LensData, c: &Ctx, v: &View, out: &mut Out) {
    // Shift+F10: 今のセルの右クリックの欄をキーボードで
    if crate::widgets::menu::menu_key(ui) {
        if let Some((j, col)) = st.cur {
            let pos = out.cur_pos.unwrap_or_else(|| ui.min_rect().left_top());
            st.kbd_menu = Some((j, col, pos, false));
        }
    }
    // Insert は前に、Alt+Insert は後に。修飾キーは押したときので見る（同じフレームで離しても）。
    // Shift+Insert は Windows の貼り付け（egui も貼り付けにする）なので挿入に使わない
    let (ins_k, ins_shift, del_k) = ui.input(|i| {
        let (mut k, mut sh) = (false, false);
        for e in &i.events {
            match e {
                egui::Event::Key {
                    key: Key::Insert,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.shift => {
                    k = true;
                    sh |= !modifiers.alt;
                }
                _ => {}
            }
        }
        (k, sh, i.key_pressed(Key::Delete))
    });
    out.fx.insert |= ins_k;
    out.fx.insert_before |= ins_shift;
    out.fx.delete |= del_k;
    let (up, down, left, right, enter_k, f2, back, esc, ctrl, lkey) = ui.input(|i| {
        (
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::ArrowLeft) || (i.key_pressed(Key::Tab) && i.modifiers.shift),
            i.key_pressed(Key::ArrowRight) || (i.key_pressed(Key::Tab) && !i.modifiers.shift),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::F2),
            i.key_pressed(Key::Backspace),
            i.key_pressed(Key::Escape),
            i.modifiers.command,
            // 文字を打つと書き換えを始めるので、ロックは Ctrl+L
            i.modifiers.command && i.key_pressed(Key::L),
        )
    });
    // コピー（Ctrl+C）・切り取りはコピーだけ・貼り付け（Ctrl+V）。タブ区切りで Excel と行き来できる
    let (copy, paste) = ui.input(|i| {
        let copy = i
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut));
        let paste = i.events.iter().find_map(|e| match e {
            egui::Event::Paste(t) => Some(t.clone()),
            _ => None,
        });
        (copy, paste)
    });
    if copy {
        let sel: Vec<usize> = v
            .selected
            .iter()
            .copied()
            .filter(|&j| v.folded(j).is_none())
            .collect();
        if !sel.is_empty() {
            ui.ctx().copy_text(copy_text(c.rows, &sel, &v.cols));
        }
    }
    // Ctrl+Shift+V・右クリックの「挿入して貼り付け」は面を挿入して（右クリックは次のフレームの Paste まで覚える）
    if st.paste_insert.is_some() && ui.ctx().cumulative_pass_nr() > st.paste_asked + 30 {
        st.paste_insert = None;
    }
    let insert_paste = ui.input(|i| i.modifiers.shift) || st.paste_insert == Some(true);
    if paste.is_some() {
        st.paste_insert = None;
    }
    if let (Some(_), Some(_), true) = (&paste, st.cur, insert_paste) {
        // 面を足して貼り付ける操作は組合せの系ではまだできない
        out.acts.push(Act::Refused(
            tr("Inserting surfaces and pasting is not available in a combination").into(),
        ));
    } else if let (Some(t), Some((j, col))) = (paste, st.cur) {
        // 貼り付け先: 範囲を選んでいれば範囲の行、なければ今の行から下へ（たたんだ行は飛ばす）
        let from = if v.selected.len() > 1 {
            v.selected[0]
        } else {
            j
        };
        let p0 = v.keys.iter().position(|&x| x == from).unwrap_or(0);
        let targets: Vec<usize> = if v.selected.len() > 1 {
            v.selected.clone()
        } else {
            v.keys[p0..].to_vec()
        }
        .into_iter()
        .filter(|&k| v.folded(k).is_none())
        .collect();
        let ci = v.cols.iter().position(|&x| x == col).unwrap_or(0);
        out.acts
            .extend(paste_acts(c.rows, &targets, &v.cols, ci, &t, &|j, col| {
                v.cell_ro(c, j, col)
            }));
    }
    // 打った文字（Ctrl・Alt なし）。セルの書き換えをその文字から始める
    let typed: Option<String> = ui.input(|i| {
        if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
            return None;
        }
        i.events.iter().find_map(|e| match e {
            egui::Event::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
    });
    // Ctrl+A: 表の全部の行を選ぶ（表を使っているとき。多重構成の次の構成より先に取る）
    let all = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::A));
    if all && !v.keys.is_empty() {
        let col = st.cur.map_or(Col::R, |x| x.1);
        st.anchor = Some(v.keys[0]);
        st.cur = Some((v.keys[v.keys.len() - 1], col));
    }
    if !v.keys.is_empty() {
        let (j, col) = st.cur.unwrap_or((v.keys[0], Col::R));
        let pos = v.keys.iter().position(|&x| x == j).unwrap_or(0);
        let ci = v.cols.iter().position(|&x| x == col).unwrap_or(2);
        let mut moved = None;
        if up && pos > 0 {
            moved = Some((v.keys[pos - 1], col));
        } else if down && pos + 1 < v.keys.len() {
            moved = Some((v.keys[pos + 1], col));
        } else if left && ci > 0 {
            moved = Some((j, v.cols[ci - 1]));
        } else if right && ci + 1 < v.cols.len() {
            moved = Some((j, v.cols[ci + 1]));
        } else if (up || down || left || right) && st.cur.is_none() {
            moved = Some((j, col));
        }
        if let Some(m) = moved {
            // Shift+↑↓ は範囲を広げる
            let shift = ui.input(|i| i.modifiers.shift);
            if shift && (up || down) {
                if st.anchor.is_none() {
                    st.anchor = Some(j);
                }
            } else if up || down {
                st.anchor = None;
            }
            st.cur = Some(m);
            st.hscroll = left || right;
            match v
                .child_of(m.0)
                .filter(|&i| Some(i) != v.scope && (v.folded(m.0).is_some() || !st.by_surface))
            {
                Some(i) => out.select(st, c, i),
                None if st.by_surface => out.unselect(st),
                None => {}
            }
        }
        if let Some((j, col)) = st.cur {
            let on_block = v.folded(j).filter(|_| col != Col::T);
            if v.tree && ((ctrl && enter_k) || (enter_k && on_block.is_some())) {
                // 折りたたみ表示: たたんだ行は Enter で開く、開いたブロックの中の面は Ctrl+Enter でそのブロックをたたむ
                if let Some(i) = v.child_of(j) {
                    out.toggle = Some(c.blocks[i].name.clone());
                }
            } else if (ctrl && enter_k) || (enter_k && on_block.is_some()) {
                if let Some(i) = v.child_of(j) {
                    out.enter = Some(c.blocks[i].name.clone());
                }
            } else if on_block.is_some() {
                // たたんだブロックの行のほかの列は書き換えない
            } else if enter_k || f2 || typed.is_some() {
                match v.cell_ro(c, j, col) {
                    Some(why) => out.acts.push(Act::Refused(why)),
                    None => out.start = Some((j, col, typed.clone())),
                }
            }
        }
    }
    if lkey {
        if let Some(s) = st.sel.clone() {
            let now = c.locked.contains(&s);
            out.acts.push(Act::Lock(s, !now));
        }
    }
    if back && !st.scope.is_empty() {
        let out = st.scope.pop();
        st.sel = out;
        st.cur = None;
    }
    if esc {
        st.cur = None;
        st.anchor = None;
    }
}

/// キーボード（Shift+F10）・セルの [...] で開いたセルの欄（右クリックと同じ中身。Esc・外のクリックで閉じる）
pub(super) fn kbd_menu(ui: &mut Ui, st: &mut LensData, c: &Ctx, v: &View, out: &mut Out) {
    if let Some((j, col, pos, cell_only)) = st.kbd_menu {
        if j >= c.rows.len() || v.folded(j).is_some() {
            st.kbd_menu = None;
        } else {
            let r = &c.rows[j];
            let ro = v.cell_ro(c, j, col);
            let menu = &mut st.menu;
            let fresh = std::mem::take(&mut st.kbd_fresh);
            let close = crate::widgets::menu::float_menu(
                ui,
                ui.id().with("ld_kbd_menu"),
                pos,
                340.0,
                fresh,
                |ui| {
                    let mut done = false;
                    if ro.is_none() {
                        if let Some(a) = override_item(ui, c, j, col) {
                            done = true;
                            out.acts.push(a);
                        }
                        let a = cell_menu(ui, menu, j, col, r, v.bire_of(j));
                        done |= !a.is_empty();
                        out.acts.extend(a);
                    }
                    if !cell_only {
                        let m = v.row_menu(ui, menu, c, j);
                        done |= out.fx.merge(m);
                    }
                    done
                },
            );
            if close {
                st.kbd_menu = None;
                st.active = true;
            }
            if out.fx.detail {
                st.detail = true;
            }
            // キーボードの欄で選んだ操作（表の右クリックの分は上で渡した）
            out.acts.append(&mut out.fx.acts);
        }
    }
}

/// 選んだ範囲の書き出しと、面の挿入・削除（serve の edit の insert_surface・delete_surface）
pub(super) fn surfaces(st: &mut LensData, c: &Ctx, v: &View, out: &mut Out) {
    if out.export_btn {
        match v
            .sel_span
            .map(|[a, b]| [a.max(1), b.min(v.last.saturating_sub(1))])
        {
            Some([a, b]) if a <= b => out.acts.push(Act::ExportRange([a, b])),
            _ => out.acts.push(Act::Refused(
                tr("Select the surfaces to export (not the object or image surface)").into(),
            )),
        }
    }
    // 面の挿入・削除（serve の edit の insert_surface・delete_surface）
    if out.fx.insert || out.fx.delete {
        let cur = st.cur.map(|x| x.0).or(v.keys.first().copied());
        match cur {
            None => {}
            Some(j) => {
                // たたんだブロックの行は、ブロックの最初の面
                let first = v.folded(j).map_or(j, |(_, a)| a);
                let lock = locked_at(c.blocks, c.locked, first);
                if let Some(n) = lock {
                    out.acts.push(Act::Refused(trf("{} is locked", &[&n])));
                } else if out.fx.insert {
                    // 後に: たたんだブロックの行はブロックの最後の面の後。像面の後には足せないので像面の前
                    let at = if out.fx.insert_before {
                        first.max(1)
                    } else {
                        (j + 1).min(v.last).max(1)
                    };
                    let after = !out.fx.insert_before && at == j + 1;
                    let mut ops = vec![
                        serde_json::json!({ "op": "insert_surface", "at": at, "surface": {} }),
                    ];
                    // 要素の中身に足す。後に足すときは前の面の要素（要素の境目でどちらに足すか）
                    ops[0]["after"] = serde_json::json!(after);
                    out.acts.push(Act::Ops(ops));
                    st.ins_at = Some((at, c.rows.len()));
                } else if v.selected.len() > 1 {
                    // 選んだ範囲の面をまとめて消す（後ろから。OBJ・IMA・ブロックの行・絞り・ロックした面は残す）
                    let mut del: Vec<usize> = v
                        .selected
                        .iter()
                        .copied()
                        .filter(|&k| {
                            v.folded(k).is_none()
                                && k != 0
                                && k + 1 != c.rows.len()
                                && !c.rows[k].stop
                                && locked_at(c.blocks, c.locked, k).is_none()
                        })
                        .collect();
                    del.sort_unstable_by(|a, b| b.cmp(a));
                    if del.is_empty() {
                        out.acts.push(Act::Refused(
                            tr("No deletable surface in the selection").into(),
                        ));
                    } else {
                        out.acts.push(Act::Ops(
                            del.iter()
                                .map(|&k| serde_json::json!({ "op": "delete_surface", "surface": k }))
                                .collect(),
                        ));
                        st.anchor = None;
                        st.cur = Some((
                            del[del.len() - 1].min(c.rows.len() - 2),
                            st.cur.map_or(Col::R, |x| x.1),
                        ));
                    }
                } else if v.folded(j).is_some() {
                    out.acts.push(Act::Refused(
                        tr("A block row cannot be deleted (go into it and delete surfaces)").into(),
                    ));
                } else if j == 0 || j + 1 == c.rows.len() {
                    out.acts.push(Act::Refused(
                        tr("The object and image surfaces cannot be deleted").into(),
                    ));
                } else if c.rows[j].stop {
                    // 範囲で消すときと同じく絞りの面は残す
                    out.acts.push(Act::Refused(
                        tr("The stop surface cannot be deleted (move the stop to another surface first)").into(),
                    ));
                } else {
                    out.acts.push(Act::Ops(vec![
                        serde_json::json!({ "op": "delete_surface", "surface": j }),
                    ]));
                }
            }
        }
    }
}

/// 決めたことを当てる: 書き換えの開始・確定、開く・たたむ・入る、右クリックの貼り付け
pub(super) fn apply(ui: &mut Ui, st: &mut LensData, c: &Ctx, v: &View, out: &mut Out) {
    if let Some((j, col, typed)) = out.start.take() {
        st.cur = Some((j, col));
        let r = &c.rows[j];
        // 打った文字から（その文字だけ）、Enter・F2・ダブルクリックは今の値（丸めない値）を全部選んで
        let (text, select_all) = match typed {
            Some(t) => (t, false),
            None => (col.edit_text(r), true),
        };
        st.edit = Some(Editing {
            j,
            col,
            text,
            focused: false,
            select_all,
            pick: 0,
            list_hover: false,
            filtered: !select_all,
            moved: false,
            scroll: true,
            error: None,
            choices: None,
        });
        if col == Col::Material && c.glasses.is_empty() {
            out.acts.push(Act::NeedGlasses);
        }
    }
    if let Some((j, col, text)) = out.commit.take() {
        st.active = true;
        let mut act = commit_act(&c.rows[j], j, col, &text);
        // カタログに無いガラスは serve に送る前に止める（Enter で欄を閉じて黙って戻らない）
        if col == Col::Material && matches!(act, Some(Act::Edit { .. })) {
            if let Err(e) = material_ok(&text, c.glasses, c.rows) {
                act = Some(Act::Refused(e));
            }
        }
        let refused = match &act {
            Some(Act::Refused(e)) => Some(e.clone()),
            _ => None,
        };
        if let Some(e) = refused {
            // 誤りなら欄を閉じず、打った文字を残して赤枠と理由を出す（Esc で取り消し）
            st.edit = Some(Editing {
                j,
                col,
                text,
                focused: false,
                select_all: false,
                pick: 0,
                list_hover: false,
                filtered: true,
                moved: false,
                scroll: false,
                error: Some(e),
                choices: None,
            });
            st.cur = Some((j, col));
        } else {
            st.edit = None;
            if let Some(a) = act {
                out.acts.push(a);
            }
        }
        // Tab・↑↓ で決めたら隣のセルへ
        if let Some((dr, dc)) = out.after.filter(|_| st.edit.is_none()) {
            let pos = v.keys.iter().position(|&x| x == j).unwrap_or(0) as i32;
            let ci = v.cols.iter().position(|&x| x == col).unwrap_or(0) as i32;
            let p = (pos + dr).clamp(0, v.keys.len() as i32 - 1) as usize;
            let k = (ci + dc).clamp(0, v.cols.len() as i32 - 1) as usize;
            st.cur = Some((v.keys[p], v.cols[k]));
            st.hscroll = dc != 0;
        }
    }
    if let Some(name) = out.toggle.take() {
        if st.open.remove(&name) {
            // たたんだブロックの中の面を選んでいたら、たたんだ行（続く面の最後）へ
            if let (Some((j, col)), Some(b)) = (st.cur, c.blocks.iter().find(|b| b.name == name)) {
                if b.surfs.contains(&j) {
                    st.cur = Some((run_end(b, j), col));
                    st.edit = None;
                }
            }
        } else if let Some(path) = path_to(c.blocks, &name) {
            // 外のブロックも開く（たたんだ外のブロックの中では見えないので）
            st.open.extend(path);
        }
        st.active = true;
    }
    if let Some(name) = out.enter.take() {
        st.enter(c.blocks, &name);
        st.active = true;
    }
    // 右クリックの貼り付け: クリップボードを頼み、次のフレームの Paste で貼る
    if let Some(p) = out.fx.paste {
        st.paste_insert = Some(p);
        st.paste_asked = ui.ctx().cumulative_pass_nr();
        st.active = true;
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
}
