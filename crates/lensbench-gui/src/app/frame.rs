//! 1 フレームの画面の段（[`App::update`] から分けた）: 入力、窓の題と枠、パネル、小窓

use super::*;

impl App {
    /// 窓に落としたファイルとキー
    pub(super) fn handle_input(&mut self, ctx: &egui::Context) {
        // 窓に落としたファイル
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(crate::files::dropped)
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            self.c.open_doc(&p);
        }
        // キー
        let (save_as, save) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        if save_as || save {
            self.c.save(save_as);
        }
        let (new, open, palette, f5, reload, help, next, prev) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::K),
                i.consume_key(Modifiers::NONE, Key::F5),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::NONE, Key::F1),
                i.consume_key(Modifiers::COMMAND, Key::Tab),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab),
            )
        });
        if new {
            self.c.new_system();
        }
        if open {
            self.c.pick_and_open();
        }
        if palette {
            if self.palette.open {
                self.palette.open = false;
            } else {
                self.palette.show();
            }
        }
        if f5 {
            self.run_all(false);
        }
        if reload {
            self.c.reload();
        }
        if help {
            self.c.help_open = !self.c.help_open;
        }
        if next || prev {
            self.cycle(prev);
        }
        // Ctrl+Z・Ctrl+Y（Ctrl+Shift+Z）: 系の変更を元に戻す・やり直す（入力欄の中では欄の文字の取り消し）
        if !ctx.wants_keyboard_input() {
            let (redo, undo) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::Y)
                        || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                    i.consume_key(Modifiers::COMMAND, Key::Z),
                )
            });
            if undo || redo {
                self.c.undo(undo, 1);
            }
        }
    }

    /// パネルを描いた後のキー: パネルが使わなかったものだけ。Ctrl+A・Ctrl+Shift+A は多重構成の次・前の構成
    /// （入力欄にフォーカスがあるときは全選択、面の表を使っているときは表の全部の行の選択に、パネルが先に取る）
    pub(super) fn handle_late_input(&mut self, ctx: &egui::Context) {
        if !ctx.wants_keyboard_input() {
            let (back, fwd) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A),
                    i.consume_key(Modifiers::COMMAND, Key::A),
                )
            });
            if back || fwd {
                self.c.cycle_config(back);
            }
        }
    }

    /// 窓の題（開いている文書と変更の印）と、OS の枠を使うか
    pub(super) fn sync_window_chrome(&mut self, ctx: &egui::Context) {
        // 組合せの系は保存したプロジェクトの名前（一時ファイルの名前は出さない）
        let shown = self.c.loc.shown(self.c.is_combo());
        let mark = if self.c.dirty() { "● " } else { "" };
        let title = match shown {
            Some(f) => format!(
                "lensbench — {mark}{}",
                f.file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None if !mark.is_empty() => format!("lensbench — {mark}{}", tr("(unsaved)")),
            None => "lensbench".into(),
        };
        // 変わったときだけ送る（送ると次のフレームを描くので、毎フレーム送ると何もしていなくても描き続ける）
        let changed = ctx.data_mut(|d| {
            let k = egui::Id::new("lensbench-title");
            let same = d.get_temp::<String>(k).is_some_and(|t| t == title);
            if !same {
                d.insert_temp(k, title.clone());
            }
            !same
        });
        if changed {
            // Web 版はブラウザのタブの題にする（起動確認 scripts/web_smoke.mjs が開いたファイルの名前を見る）
            #[cfg(target_arch = "wasm32")]
            crate::web_tabs::set_title(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let os = self.c.s.os_titlebar || self.tour.is_some();
            if self.decorations != Some(os) {
                self.decorations = Some(os);
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(os));
            }
            if !os {
                crate::chrome::resize_edges(ctx);
            }
        }
    }

    /// 上の帯・下の行・ドック・別の窓のパネル
    pub(super) fn show_panels(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| self.header(ctx, ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        let mut viewer = Viewer {
            c: &mut self.c,
            to_window: Vec::new(),
            added: Vec::new(),
            dragging: None,
        };
        let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());
        style.tab_bar.corner_radius = egui::CornerRadius::same(6);
        style.tab.tab_body.inner_margin = egui::Margin::same(10);
        DockArea::new(&mut self.dock)
            .style(style)
            .show_add_buttons(true)
            .show_add_popup(true)
            .show_leaf_collapse_buttons(false)
            // フロートの窓を本体の外まで動かせるように（外に置くと別の OS の窓にする）
            .window_bounds(egui::Rect::EVERYTHING)
            .show(ctx, &mut viewer);
        let Viewer {
            to_window,
            added,
            dragging,
            ..
        } = viewer;
        self.drag_out(ctx, dragging);
        // カタログの行を引きずっている間は、ポインタの横に名前を出す
        if let (Some(p), Some(q)) = (
            egui::DragAndDrop::payload::<combo::DropPick>(ctx),
            ctx.pointer_latest_pos(),
        ) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            egui::Area::new(egui::Id::new("lensbench-drop-pick"))
                .order(egui::Order::Tooltip)
                .fixed_pos(q + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(trf("{} → layout", &[&p.0.label()]));
                    });
                });
        }
        for (s, n, k) in added {
            // 1 枚だけのパネルがもうあれば、2 枚目を作らず表に出す
            if k.single() {
                let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
                if let Some(id) = self.pane_of(same) {
                    self.focus(id);
                    continue;
                }
            }
            let id = self.c.new_id();
            self.dock.set_focused_node_and_surface((s, n));
            self.dock.push_to_focused_leaf(Pane { id, kind: k });
        }
        for id in to_window {
            self.detach(id);
        }
        self.float_out(ctx);
        #[cfg(target_arch = "wasm32")]
        self.web_tabs(ctx);
        self.os_windows(ctx);
    }

    /// パレット・頼まれた操作・小窓・確かめ
    pub(super) fn show_dialogs(&mut self, ctx: &egui::Context) {
        // 項目はパレットを開いているときだけ作る
        if self.palette.open {
            let items = self.palette_items();
            if let Some(cmd) = self.palette.ui(ctx, &items) {
                self.exec(ctx, cmd);
            }
        }
        for cmd in std::mem::take(&mut self.c.cmds) {
            self.exec(ctx, cmd);
        }
        if self.c.folders_open {
            self.c.folders_window(ctx);
        }
        self.close_guard(ctx);
        self.take_shot(ctx);
        self.c.missing_glass_window(ctx);
        if self.c.help_open {
            self.c.help_window(ctx);
        }
        if self.c.p.cat_save.open {
            self.c.cat_save_window(ctx);
        }
        self.c.fill_window(ctx);
        self.save_layout_window(ctx);
        // 別の窓で出た確かめはその窓に出す（窓が閉じていれば本体に）
        #[cfg(target_arch = "wasm32")]
        let elsewhere = self.c.ask_from().is_some_and(|id| {
            self.windows.iter().any(|w| w.pane.id == id)
                && self.tabs.as_ref().is_some_and(|t| t.is_open(id))
        });
        #[cfg(not(target_arch = "wasm32"))]
        let elsewhere = false;
        if !elsewhere {
            self.c.ask_ui(ctx);
        }
    }
}
