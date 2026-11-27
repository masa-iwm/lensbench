//! 1 フレームの画面の段（[`App::update`] から分けた）: 入力、窓の題と枠、パネル、小窓

use super::*;

impl App {
    /// 窓に落としたファイルとキーを操作にして当てる
    pub(super) fn handle_input(&mut self, ctx: &egui::Context) {
        for m in input_msgs(ctx) {
            self.dispatch(ctx, m);
        }
    }

    /// パネルを描いた後のキー: パネルが使わなかったものだけ。Ctrl+A・Ctrl+Shift+A は多重構成の次・前の構成
    /// （入力欄にフォーカスがあるときは全選択、面の表を使っているときは表の全部の行の選択に、パネルが先に取る）
    pub(super) fn handle_late_input(&mut self, ctx: &egui::Context) {
        if let Some(m) = late_input_msg(ctx) {
            self.dispatch(ctx, m);
        }
    }

    /// 窓の題（開いている文書と変更の印）と、OS の枠を使うか
    pub(super) fn sync_window_chrome(&mut self, ctx: &egui::Context) {
        // 組合せの系は保存したプロジェクトの名前（一時ファイルの名前は出さない）
        let shown = self.c.loc.shown();
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
            let k = egui::Id::unique("lensbench-title");
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
                // 枠を付け外しすると、Windows は窓の外形を保って中身の大きさを変える。起動のたびに枠を外すと、終わりに
                // 保存する中身の大きさが枠の分（幅 16 px など）ずつ大きくなっていた。中身の大きさを保つ（最大化・全画面は
                // 大きさを送ると元に戻るので、元に戻した最初のフレームで保存の大きさを送る。下の restore_normal_size）
                if let Some(r) = ctx
                    .input(|i| i.viewport().inner_rect)
                    .filter(|_| !maximized(ctx))
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
                    ctx.data_mut(|d| d.remove_temp::<egui::Vec2>(normal_size_id()));
                }
            } else {
                restore_normal_size(ctx);
            }
            if !os {
                crate::chrome::resize_edges(ctx);
            }
        }
    }

    /// 上の帯・下の行・ドック・別の窓のパネル
    pub(super) fn show_panels(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        egui::Panel::top("header")
            .frame(
                egui::Frame::side_top_panel(&ctx.global_style())
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| self.header(ctx, ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        let mut viewer = Viewer {
            c: &mut self.c,
            to_window: Vec::new(),
            added: Vec::new(),
            dragging: None,
        };
        let mut style = egui_dock::Style::from_egui(ctx.global_style().as_ref());
        style.tab_bar.corner_radius = egui::CornerRadius::same(6);
        style.tab.tab_body.inner_margin = egui::Margin::same(10);
        // 残りの広さにドック（egui_dock 0.16 の DockArea::show と同じ、余白なし・透明の枠）
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.global_style())
                    .inner_margin(egui::Margin::ZERO)
                    .fill(egui::Color32::TRANSPARENT),
            )
            .show(ui, |ui| {
                DockArea::new(&mut self.dock)
                    .style(style)
                    .show_add_buttons(true)
                    .show_add_popup(true)
                    .show_leaf_collapse_buttons(false)
                    // フロートの窓を本体の外まで動かせるように（外に置くと別の OS の窓にする）
                    .window_bounds(egui::Rect::EVERYTHING)
                    .show_inside(ui, &mut viewer);
            });
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
            egui::Area::new(egui::Id::unique("lensbench-drop-pick"))
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
            self.dock.set_focused_node_and_surface(egui_dock::NodePath {
                surface: s,
                node: n,
            });
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
            if let Some(m) = self.palette.ui(ctx, &items) {
                self.dispatch(ctx, m);
            }
        }
        self.dispatch_queued(ctx);
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
        self.c.thermal_window(ctx);
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

/// 窓に落としたファイルとキーの操作（パネルを描く前）
pub(super) fn input_msgs(ctx: &egui::Context) -> Vec<Msg> {
    let mut out = Vec::new();
    // 窓に落としたファイル
    // 落としたのが何個でも開くのは最初の 1 つ
    let dropped = ctx
        .input(|i| i.raw.dropped_files.first().and_then(crate::files::dropped))
        .or_else(crate::files::take_dropped);
    if let Some(p) = dropped {
        out.push(Msg::OpenFile(p));
    }
    let keys = [
        (
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::S,
            Msg::Save(true),
        ),
        (Modifiers::COMMAND, Key::S, Msg::Save(false)),
        (Modifiers::COMMAND, Key::N, Msg::New),
        (Modifiers::COMMAND, Key::O, Msg::Open),
        (Modifiers::COMMAND, Key::K, Msg::TogglePalette),
        (Modifiers::NONE, Key::F5, Msg::RunAll),
        (Modifiers::COMMAND, Key::R, Msg::Reload),
        (Modifiers::NONE, Key::F1, Msg::ToggleHelp),
        (
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::Tab,
            Msg::CyclePanes(true),
        ),
        (Modifiers::COMMAND, Key::Tab, Msg::CyclePanes(false)),
    ];
    ctx.input_mut(|i| {
        for (m, k, msg) in keys {
            if i.consume_key(m, k) {
                out.push(msg);
            }
        }
    });
    // Ctrl+Z・Ctrl+Y（Ctrl+Shift+Z）: 系の変更を元に戻す・やり直す（入力欄の中では欄の文字の取り消し）
    if !ctx.egui_wants_keyboard_input() {
        let (redo, undo) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Y)
                    || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Z),
            )
        });
        if undo || redo {
            out.push(Msg::Undo(undo, 1));
        }
    }
    out
}

/// パネルを描いた後のキーの操作
fn late_input_msg(ctx: &egui::Context) -> Option<Msg> {
    if ctx.egui_wants_keyboard_input() {
        return None;
    }
    let (back, fwd) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A),
            i.consume_key(Modifiers::COMMAND, Key::A),
        )
    });
    (back || fwd).then_some(Msg::CycleConfig(back))
}

/// 最大化か全画面か
#[cfg(not(target_arch = "wasm32"))]
fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true))
}

/// 前の終わりに保存した、最大化していないときの中身の大きさ（egui のメモリ。最大化で始まったときだけ使う）
#[cfg(not(target_arch = "wasm32"))]
fn normal_size_id() -> egui::Id {
    egui::Id::unique("lensbench-normal-size")
}

/// 保存の大きさ（eframe の "window"）を覚える。起動で App を作る前に呼ぶ
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remember_normal_size(cc: &eframe::CreationContext<'_>) {
    let v = cc.storage.and_then(|st| st.get_string("window"));
    if let Some(v) = v.as_deref().and_then(normal_size) {
        cc.egui_ctx.data_mut(|d| d.insert_temp(normal_size_id(), v));
    }
}

/// eframe の "window"（egui-winit の WindowSettings の RON）の、最大化していないときの中身の大きさ
#[cfg(not(target_arch = "wasm32"))]
fn normal_size(ron_text: &str) -> Option<egui::Vec2> {
    #[derive(serde::Deserialize)]
    struct Geometry {
        inner_size_points: egui::Vec2,
    }
    #[derive(serde::Deserialize)]
    struct Window {
        normal_geometry: Geometry,
    }
    let w: Window = ron::from_str(ron_text).ok()?;
    Some(w.normal_geometry.inner_size_points)
}

/// 最大化（全画面）で始まり枠を外したとき、Windows が持つ「元に戻したときの大きさ」は枠の分ずれている。
/// 元に戻した最初のフレームで保存の大きさを送る（最大化の往復のたびに広がっていた）
#[cfg(not(target_arch = "wasm32"))]
fn restore_normal_size(ctx: &egui::Context) {
    if maximized(ctx) {
        return;
    }
    if let Some(v) = ctx.data_mut(|d| d.remove_temp::<egui::Vec2>(normal_size_id())) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(v));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// eframe 0.37 が保存する形から、最大化していないときの大きさを読む（読めない形は None）
    #[test]
    fn reads_normal_size_from_saved_window() {
        let s = "(normal_geometry:(outer_position_pixels:Some((x:0.0,y:0.0)),scale_factor:1.0,inner_size_points:(x:1280.0,y:820.0)),fullscreen:false,maximized:true)";
        assert_eq!(super::normal_size(s), Some(egui::vec2(1280.0, 820.0)));
        assert_eq!(
            super::normal_size("(inner_size_points:(x:1.0,y:2.0))"),
            None
        );
    }
}
