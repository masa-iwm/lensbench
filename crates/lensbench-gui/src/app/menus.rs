//! 上の帯のメニュー・コマンドパレット・操作の実行（app.rs から分けた）

use super::*;

impl App {
    /// パレットの項目
    pub(super) fn palette_items(&self) -> Vec<Item<Msg>> {
        let mut v = Vec::new();
        let mut push = |label: String, hint: &str, cmd: Msg| {
            v.push(Item {
                label,
                hint: hint.into(),
                danger: matches!(cmd, Msg::New | Msg::Reload),
                cmd,
            })
        };
        push(
            tr("New (clear the combination)").into(),
            "new ctrl+n",
            Msg::New,
        );
        push(
            tr("Open a system file…").into(),
            "open file ctrl+o",
            Msg::Open,
        );
        push(
            tr("Undo (Ctrl+Z)").into(),
            "undo ctrl+z",
            Msg::Undo(true, 1),
        );
        push(
            tr("Redo (Ctrl+Y)").into(),
            "redo ctrl+y ctrl+shift+z",
            Msg::Undo(false, 1),
        );
        push(tr("Reload the system").into(), "reload", Msg::Reload);
        // 実行系。設定はそれぞれのパネルのまま
        if self.c.sys.summary.is_some() {
            push(
                tr("Optimize (current settings)").into(),
                "run optimize opt",
                Msg::RunOpt,
            );
            push(
                tr("Run tolerancing (current settings)").into(),
                "run tolerance tol monte carlo",
                Msg::RunTol,
            );
        }
        if self.c.loc.file.is_some() && self.c.layout_data.is_some() {
            push(
                tr("Image at paraxial focus").into(),
                "focus image paraxial",
                Msg::FocusImage,
            );
        }
        push(
            tr("Recompute all open analyses").into(),
            "run all f5",
            Msg::RunAll,
        );
        push(
            tr("Export to .zmx…").into(),
            "write zmx export",
            Msg::WriteZmx,
        );
        push(tr("Save (Ctrl+S)").into(), "save ctrl+s", Msg::Save(false));
        push(
            tr("Save As… (Ctrl+Shift+S)").into(),
            "save as ctrl+shift+s",
            Msg::Save(true),
        );
        push(
            record_label(self.c.rec.is_some()).into(),
            "record operations macro",
            Msg::Record,
        );
        push(
            tr("Replay recorded operations…").into(),
            "replay operations macro",
            Msg::Replay,
        );
        for k in add_kinds() {
            let t = Pane {
                id: 0,
                kind: k.clone(),
            }
            .title();
            let hint = match k.tool() {
                Some(t) => format!("add panel {}", t.keywords),
                None => match &k {
                    Kind::Analysis(a) => format!("add panel {}", a.cmd),
                    _ => String::new(),
                },
            };
            let label = if k.single() {
                trf("Show panel: {}", &[&t])
            } else {
                trf("Add panel: {}", &[&t])
            };
            push(label, &hint, Msg::Add(k));
        }
        for (_, p) in self.dock.iter_all_tabs() {
            let name = match (&p.kind, p.kind.tool()) {
                (Kind::Analysis(a), _) => a.cmd.clone(),
                (_, Some(t)) => t.name.to_string(),
                _ => String::new(),
            };
            push(
                trf("Go to panel: {}", &[&p.title()]),
                &format!("go focus panel {name}"),
                Msg::Focus(p.id),
            );
            push(
                format!(
                    "{}: {}",
                    detach_label(cfg!(target_arch = "wasm32")),
                    p.title()
                ),
                &format!("window monitor detach {name}"),
                Msg::ToWindow(p.id),
            );
        }
        for p in Preset::ALL {
            push(
                trf("Workspace: {}", &[&p.label()]),
                "layout preset",
                Msg::Preset(p),
            );
        }
        for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
            push(
                trf("Saved workspace: {}", &[n]),
                "layout saved",
                Msg::Layout(i),
            );
        }
        push(
            tr("Save current workspace…").into(),
            "save layout",
            Msg::SaveLayout,
        );
        push(
            tr("Theme: dark").into(),
            "theme dark",
            Msg::Theme(Mode::Dark),
        );
        push(
            tr("Theme: light").into(),
            "theme light",
            Msg::Theme(Mode::Light),
        );
        push(
            tr("Theme: follow the OS").into(),
            "theme system",
            Msg::Theme(Mode::System),
        );
        for i in 0..theme::ACCENTS.len() {
            push(
                trf("Accent color {}", &[&(i + 1)]),
                "accent color",
                Msg::Accent(i),
            );
        }
        for l in Lang::ALL {
            push(trf("Language: {}", &[&l.label()]), "language", Msg::Lang(l));
        }
        push(
            tr("Catalog folders…").into(),
            "folders settings",
            Msg::Folders,
        );
        push(tr("Keyboard shortcuts").into(), "help keys f1", Msg::Help);
        for f in &self.c.s.recent {
            push(
                trf("Recent file: {}", &[f]),
                "recent",
                Msg::OpenFile(PathBuf::from(f)),
            );
        }
        v
    }

    /// 上の帯: ロゴ、ファイル、メニュー、探す、元に戻す、計算
    pub(super) fn header(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let accent = theme::accent(&self.c.s.look);
        let weak = theme::weak_text(ui.visuals());
        let mut cmd = None;
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            logo(ui, 26.0, accent);
            ui.label(RichText::new("lensbench").strong().size(15.0).color(accent));
            {
                self.file_menu(ctx, ui, &mut cmd);
                self.edit_menu(ui, &mut cmd);
                self.panels_menu(ui, &mut cmd);
                self.workspace_menu(ui, &mut cmd);
                self.view_menu(ctx, ui, &mut cmd);
                self.help_menu(ctx, ui, &mut cmd);
            }
            let shown = self.c.loc.shown().cloned();
            let dirty = self.c.dirty();
            if shown.is_some() || dirty {
                let name = shown
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_else(|| tr("(unsaved)").to_string());
                let mark = if dirty { " ●" } else { "" };
                let r = ui.label(RichText::new(format!("{name}{mark}")).strong());
                let tip = shown.map(|f| f.display().to_string()).unwrap_or_default();
                if dirty {
                    r.on_hover_text(format!(
                        "{tip}\n{}",
                        tr("● Unsaved changes (Ctrl+S to save)")
                    ));
                } else {
                    r.on_hover_text(tip);
                }
            }
            let custom = crate::chrome::custom(self.c.s.os_titlebar);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if custom {
                    crate::chrome::window_buttons(ui);
                }
                let run = egui::Button::new(
                    RichText::new(tr("▶ Compute (F5)"))
                        .strong()
                        .color(Color32::from_rgb(6, 18, 28)),
                )
                .fill(accent)
                .min_size(egui::vec2(0.0, 28.0));
                if ui.add_enabled(self.c.sys.summary.is_some(), run).clicked() {
                    cmd = Some(Msg::RunAll);
                }
                // 1 段の元に戻す・やり直す（読み直すはファイルのメニュー）。押すと何を戻すかをホバーで
                for (back, icon, key) in
                    [(false, ["↷", ">"], "Ctrl+Y"), (true, ["↶", "<"], "Ctrl+Z")]
                {
                    let next = self.c.undo_list(back).into_iter().next();
                    let tip = match (&next, back) {
                        (Some(n), true) => trf("Undo ({}): {}", &[&key, &n.tip()]),
                        (Some(n), false) => trf("Redo ({}): {}", &[&key, &n.tip()]),
                        (None, true) => tr("Nothing to undo").to_string(),
                        (None, false) => tr("Nothing to redo").to_string(),
                    };
                    if ui
                        .add_enabled(
                            self.c.can_undo(back),
                            egui::Button::new(
                                RichText::new(crate::fonts::glyph_or(ui, icon[0], icon[1]))
                                    .size(16.0),
                            )
                            .min_size(egui::vec2(28.0, 28.0)),
                        )
                        .on_hover_text(tip.clone())
                        .on_disabled_hover_text(tip)
                        .clicked()
                    {
                        cmd = Some(Msg::Undo(back, 1));
                    }
                }
                // 探す欄は残りの幅に合わせて縮める（狭ければ 🔍 だけ。左のメニューに重ねない）
                let room = ui.available_width() - 8.0;
                let label = if room >= 300.0 {
                    format!("🔍  {}    Ctrl K", tr("Search analyses, panels, actions"))
                } else if room >= 150.0 {
                    format!("🔍  {}", tr("Search"))
                } else {
                    "🔍".to_string()
                };
                // 🔍 も入らなければ出さない（ファイル名に重ねない。Ctrl+K で開ける）
                if room >= 32.0 {
                    let search = egui::Button::new(RichText::new(label).color(weak))
                        .min_size(egui::vec2(room.min(300.0), 28.0));
                    let r = ui
                        .add(search)
                        .on_hover_text(tr("Search analyses, panels and commands (Ctrl+K)"));
                    self.palette.anchor = Some(r.rect);
                    if r.clicked() {
                        self.palette.show();
                    }
                } else {
                    self.palette.anchor = None;
                }
                if custom {
                    crate::chrome::drag_area(ui);
                }
            });
        });
        if let Some(c) = cmd {
            self.dispatch(ctx, c);
        }
    }

    /// 上の帯の「File」メニュー
    fn file_menu(&mut self, ctx: &egui::Context, ui: &mut Ui, cmd: &mut Option<Msg>) {
        ui.menu_button(tr("File"), |ui| {
            if ui.button(tr("New (Ctrl+N)")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::New);
            }
            if ui.button(tr("Open… (Ctrl+O)")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::Open);
            }
            if !self.c.samples.is_empty() {
                ui.menu_button(tr("Samples"), |ui| {
                    // 組合せ・プロジェクト・.zmx の系の間に区切り
                    let group = |f: &str| {
                        if f.contains("/examples/combos/") {
                            0
                        } else if f.ends_with(".json") {
                            1
                        } else {
                            2
                        }
                    };
                    let mut last = None;
                    for f in &self.c.samples {
                        let g = group(f);
                        if last.is_some_and(|l| l != g) {
                            ui.separator();
                        }
                        if last != Some(g) && g == 0 {
                            ui.weak(tr("Combinations (open as a combination)"));
                        }
                        last = Some(g);
                        let name = sample_name(f);
                        if ui.button(name).on_hover_text(f).clicked() {
                            ui.close_menu();
                            *cmd = Some(Msg::OpenFile(PathBuf::from(f)));
                        }
                    }
                });
            }
            ui.menu_button(tr("Recent files"), |ui| {
                if self.c.s.recent.is_empty() {
                    ui.label(tr("(none)"));
                }
                // 親のフォルダとファイルの名前だけを 1 行で出し、フルパスはホバーで（フルパスはメニューの幅で
                // 区切りごとに折れて縦に長くなる）
                for f in &self.c.s.recent {
                    let b = egui::Button::new(recent_label(f)).wrap_mode(egui::TextWrapMode::Extend);
                    if ui.add(b).on_hover_text(f).clicked() {
                        ui.close_menu();
                        *cmd = Some(Msg::OpenFile(PathBuf::from(f)));
                    }
                }
            });
            if ui
                .add_enabled(
                    self.c.loc.file.is_some(),
                    egui::Button::new(tr("Reload (Ctrl+R)")),
                )
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Reload);
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.c.sys.summary.is_some(),
                    egui::Button::new(tr("Export to .zmx…")),
                )
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::WriteZmx);
            }
            let can_save = self.c.loc.file.is_some() || !self.c.doc.combo.picks.is_empty();
            let save_label = tr("Save (Ctrl+S)");
            if ui
                .add_enabled(can_save, egui::Button::new(save_label))
                .on_hover_text(tr(
                    "A combination is saved with the combination in a project .json (opening it restores the combination). An opened system is saved as a system file",
                ))
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Save(false));
            }
            if ui
                .add_enabled(
                    can_save,
                    egui::Button::new(tr("Save As… (Ctrl+Shift+S)")),
                )
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Save(true));
            }
            ui.separator();
            if ui
                .button(record_label(self.c.rec.is_some()))
                .on_hover_text(tr(
                    "Records the operations that change the document, to replay them later in the same order",
                ))
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Record);
            }
            if ui.button(tr("Replay recorded operations…")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::Replay);
            }
            ui.separator();
            if ui.button(tr("Catalog folders…")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::Folders);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let (n, b) = crate::web_store::usage();
                if ui
                    .add_enabled(
                        n > 0,
                        egui::Button::new(trf(
                            "Clear files kept in the browser ({} files, {} MB)…",
                            &[&n, &format!("{:.1}", b as f64 / 1e6)],
                        )),
                    )
                    .on_hover_text(tr(
                        "Loaded files and user catalogs stay in the browser after the page is closed",
                    ))
                    .clicked()
                {
                    ui.close_menu();
                    self.c.ask(
                        "lensbench",
                        tr("Delete all loaded files and user catalogs kept in the browser?"),
                        |c, yes| {
                            if yes {
                                crate::web_store::clear();
                                crate::web_store::prune_dirs(
                                    &mut c.s.glass_dirs,
                                    &mut c.s.lens_dirs,
                                );
                                c.refresh_catalog();
                            }
                        },
                    );
                }
            }
            // ブラウザ（Web 版）では閉じられないので出さない
            if cfg!(not(target_arch = "wasm32")) {
                ui.separator();
                if ui.button(tr("Quit")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        });
    }

    /// 上の帯の「Edit」メニュー
    fn edit_menu(&mut self, ui: &mut Ui, cmd: &mut Option<Msg>) {
        ui.menu_button(tr("Edit"), |ui| {
            let (u, r) = (self.c.can_undo(true), self.c.can_undo(false));
            if ui
                .add_enabled(u, egui::Button::new(tr("Undo (Ctrl+Z)")))
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Undo(true, 1));
            }
            if ui
                .add_enabled(r, egui::Button::new(tr("Redo (Ctrl+Y)")))
                .clicked()
            {
                ui.close_menu();
                *cmd = Some(Msg::Undo(false, 1));
            }
            // 段の一覧: 選んだ段まで（その段を含めて）まとめて戻す・やり直す
            for (back, title) in [(true, tr("Undo up to")), (false, tr("Redo up to"))] {
                let list = self.c.undo_list(back);
                ui.add_enabled_ui(!list.is_empty(), |ui| {
                    ui.menu_button(title, |ui| {
                        egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                            for (i, l) in list.iter().enumerate() {
                                // 文書を替えた段（新規・開く）の前で区切る（その先は前の文書）
                                if l.doc && i > 0 {
                                    ui.separator();
                                }
                                let label = format!("{}. {}   {}", i + 1, l.title, l.age());
                                let r = ui.button(label);
                                let r = if l.details.len() > 1 {
                                    r.on_hover_text(l.tip())
                                } else {
                                    r
                                };
                                if r.clicked() {
                                    ui.close_menu();
                                    *cmd = Some(Msg::Undo(back, i + 1));
                                }
                            }
                        });
                    });
                });
            }
            ui.label(
                RichText::new(tr(
                    "System changes (lens data, multi-config, optimization results, combination) and open, reload and new, up to 100 steps",
                ))
                .small()
                .weak(),
            );
        });
    }

    /// 上の帯の「Panels」メニュー
    fn panels_menu(&mut self, ui: &mut Ui, cmd: &mut Option<Msg>) {
        // 1 枚だけのパネルは「出す」（あれば表に）、解析は「足す」を下の段に分ける
        ui.menu_button(tr("Panels"), |ui| {
            let (single, many): (Vec<Kind>, Vec<Kind>) =
                add_kinds().into_iter().partition(Kind::single);
            for k in single {
                let t = Pane {
                    id: 0,
                    kind: k.clone(),
                }
                .title();
                if ui
                    .button(t)
                    .on_hover_text(tr("Single panel; brings it to the front if already open"))
                    .clicked()
                {
                    ui.close_menu();
                    *cmd = Some(Msg::Add(k));
                }
            }
            ui.separator();
            ui.menu_button(tr("Add analysis"), |ui| {
                for k in many {
                    let t = Pane {
                        id: 0,
                        kind: k.clone(),
                    }
                    .title();
                    if ui.button(trf("+ {}", &[&t])).clicked() {
                        ui.close_menu();
                        *cmd = Some(Msg::Add(k));
                    }
                }
            });
        });
    }

    /// 上の帯の「Workspace」メニュー
    fn workspace_menu(&mut self, ui: &mut Ui, cmd: &mut Option<Msg>) {
        let weak = theme::weak_text(ui.visuals());
        ui.menu_button(tr("Workspace"), |ui| {
            ui.label(RichText::new(tr("Preset workspaces")).small().color(weak));
            for p in Preset::ALL {
                if ui.button(p.label()).clicked() {
                    ui.close_menu();
                    *cmd = Some(Msg::Preset(p));
                }
            }
            ui.separator();
            ui.label(RichText::new(tr("Saved workspaces")).small().color(weak));
            if self.c.s.layouts.is_empty() {
                ui.label(tr("(none)"));
            }
            let mut del = None;
            for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(n).clicked() {
                        ui.close_menu();
                        *cmd = Some(Msg::Layout(i));
                    }
                    if ui.small_button("✖").on_hover_text(tr("Delete")).clicked() {
                        del = Some(i);
                    }
                });
            }
            if let Some(i) = del {
                self.c.s.layouts.remove(i);
            }
            ui.separator();
            if ui.button(tr("Save current workspace…")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::SaveLayout);
            }
        });
    }

    /// 上の帯の「View」メニュー
    fn view_menu(&mut self, ctx: &egui::Context, ui: &mut Ui, cmd: &mut Option<Msg>) {
        let weak = theme::weak_text(ui.visuals());
        ui.menu_button(tr("View"), |ui| {
            ui.label(RichText::new(tr("Theme")).small().color(weak));
            for (m, l) in [
                (Mode::Dark, tr("Dark")),
                (Mode::Light, tr("Light")),
                (Mode::System, tr("Follow the OS")),
            ] {
                if ui.radio(self.c.s.look.mode == m, l).clicked() {
                    *cmd = Some(Msg::Theme(m));
                }
            }
            ui.label(RichText::new(tr("Accent color")).small().color(weak));
            ui.horizontal(|ui| {
                for (i, a) in theme::ACCENTS.iter().enumerate() {
                    let c = Color32::from_rgb(a[0], a[1], a[2]);
                    let on = self.c.s.look.accent == *a;
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if on { "●" } else { "○" })
                                    .color(c)
                                    .size(18.0),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        *cmd = Some(Msg::Accent(i));
                    }
                }
                let mut c = Color32::from_rgb(
                    self.c.s.look.accent[0],
                    self.c.s.look.accent[1],
                    self.c.s.look.accent[2],
                );
                if ui.color_edit_button_srgba(&mut c).changed() {
                    self.c.s.look.accent = [c.r(), c.g(), c.b()];
                    theme::apply(ctx, &self.c.s.look);
                }
            });
            if cfg!(not(target_arch = "wasm32")) {
                ui.checkbox(&mut self.c.s.os_titlebar, tr("Use the OS title bar"))
                    .on_hover_text(tr(
                        "When off, the top bar acts as the title bar (drag to move, double-click to maximize, drag edges to resize)",
                    ));
            }
            ui.separator();
            ui.label(RichText::new(tr("Language")).small().color(weak));
            for l in Lang::ALL {
                if ui.radio(self.c.s.lang == l, l.label()).clicked() {
                    *cmd = Some(Msg::Lang(l));
                }
            }
            ui.separator();
            if ui.button(tr("Keyboard shortcuts (F1)")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::Help);
            }
        });
    }

    /// 上の帯の「Help」メニュー
    fn help_menu(&mut self, ctx: &egui::Context, ui: &mut Ui, cmd: &mut Option<Msg>) {
        let weak = theme::weak_text(ui.visuals());
        ui.menu_button(tr("Help"), |ui| {
            for (label, page, tip) in HELP_PAGES {
                if ui.button(tr(label)).on_hover_text(tr(tip)).clicked() {
                    ui.close_menu();
                    ctx.open_url(egui::OpenUrl::new_tab(docs_url(page)));
                }
            }
            ui.separator();
            if ui.button(tr("Keyboard shortcuts (F1)")).clicked() {
                ui.close_menu();
                *cmd = Some(Msg::Help);
            }
            ui.separator();
            ui.label(RichText::new(version()).small().color(weak));
        });
    }
}

/// ファイルの「見本」の一覧: Web 版は置き場に入れたもの（引数）、アプリ版は exe に同梱したもの
pub(super) fn samples(args: &Args) -> Vec<String> {
    #[cfg(not(target_arch = "wasm32"))]
    if args.samples.is_empty() {
        return crate::samples::list();
    }
    args.samples.clone()
}

impl Core {
    /// アプリ版で同梱の見本を開く前に: 書き出し、見本のガラス・レンズのフォルダを設定に足す
    pub(super) fn prepare_sample(&mut self, _f: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        if crate::samples::is_sample(_f) {
            match crate::samples::extract() {
                Ok((g, l)) => {
                    let mut added = false;
                    for (d, v) in [(g, &mut self.s.glass_dirs), (l, &mut self.s.lens_dirs)] {
                        if !v.contains(&d) {
                            v.push(d);
                            added = true;
                        }
                    }
                    if added {
                        self.refresh_catalog();
                    }
                }
                Err(e) => {
                    self.fail(trf("Cannot write the samples: {}", &[&e]));
                }
            }
        }
    }
}

/// ヘルプのメニューのページ（名前、マニュアルの中のページ、説明）
const HELP_PAGES: &[(&str, &str, &str)] = &[
    (
        n_("Manual"),
        "",
        n_("Table of contents (opens in the browser)"),
    ),
    (
        n_("Using the GUI"),
        "gui.html",
        n_("Screens and panels, with pictures"),
    ),
    (
        n_("Analyses and CLI"),
        "cli.html",
        n_("Analysis types, arguments and example outputs"),
    ),
    (
        n_("Project file"),
        "project-file.html",
        n_("Building systems from blocks in JSON (states, groups, beam splitters)"),
    ),
    (
        n_("Optimize"),
        "optimize.html",
        n_("Merit function, variables and DLS"),
    ),
    (
        n_("Tolerance"),
        "tolerance.html",
        n_("Monte Carlo tolerancing settings"),
    ),
];

/// パネルを外に出す操作の名前。Web 版はブラウザの別の窓（ポップアップ）に出し、元のタブで描いた画面を送って
/// 別の窓で操作する
pub(super) fn detach_label(web: bool) -> &'static str {
    if web {
        tr("Move to another window")
    } else {
        tr("Move to separate window (another monitor)")
    }
}

#[test]
fn detach_label_web() {
    assert_eq!(detach_label(true), "Move to another window");
    assert_eq!(
        detach_label(false),
        "Move to separate window (another monitor)"
    );
}

/// 操作の記録の項目の文言（記録中はやめて保存する）
fn record_label(on: bool) -> &'static str {
    if on {
        tr("Stop recording and save…")
    } else {
        tr("Record operations")
    }
}

/// 最近のファイルの欄の文字: 親のフォルダとファイルの名前（`A0\system.zmx`）。区切りは `/` と `\` のどちらでも
/// （Windows のパスを別の OS で読んでも）、元の区切りのまま出す。上にまだフォルダがあれば前に `…`
pub(crate) fn recent_label(f: &str) -> String {
    let parts: Vec<&str> = f.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
    if parts.len() <= 2 {
        return f.to_string();
    }
    let n = parts.len();
    let sep = if f.contains('\\') { '\\' } else { '/' };
    format!("…{sep}{}{sep}{}", parts[n - 2], parts[n - 1])
}
