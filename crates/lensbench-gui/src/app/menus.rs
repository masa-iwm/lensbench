//! 上の帯のメニュー・コマンドパレット・操作の実行（GAR-18 で app.rs から分けた）

use super::*;

impl App {
    pub(super) fn exec(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::New => self.c.new_system(),
            Cmd::Undo(back, n) => self.c.undo(back, n),
            Cmd::Open => self.c.pick_and_open(),
            Cmd::Recent(f) => {
                self.c.prepare_sample(&f);
                self.c.open_doc(Path::new(&f));
            }
            Cmd::Reload => self.c.reload(),
            Cmd::RunAll => self.run_all(false),
            Cmd::WriteZmx => self.c.write_zmx(),
            Cmd::Save(as_new) => self.c.save(as_new),
            // 1 枚だけのパネルは、あれば表に出す（GUX-16）
            Cmd::Add(k) if k.single() => self.exec(ctx, Cmd::FocusKind(k)),
            Cmd::Add(k) => {
                let id = self.add_pane(k);
                self.focus(id);
            }
            Cmd::Focus(id) => self.focus(id),
            Cmd::FocusKind(k) => {
                let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
                let id = match self.pane_of(same) {
                    Some(id) => id,
                    None => self.add_pane(k),
                };
                self.focus(id);
            }
            Cmd::ToWindow(id) => self.detach(id),
            Cmd::Preset(p) => self.set_layout(p.build()),
            Cmd::Layout(i) => {
                if let Some((_, l)) = self.c.s.layouts.get(i) {
                    let l = l.clone();
                    self.set_layout(l);
                }
            }
            Cmd::SaveLayout => {
                self.c.save_layout_open = true;
            }
            Cmd::Theme(m) => {
                self.c.s.look.mode = m;
                theme::apply(ctx, &self.c.s.look);
            }
            Cmd::Accent(i) => {
                self.c.s.look.accent = theme::ACCENTS[i % theme::ACCENTS.len()];
                theme::apply(ctx, &self.c.s.look);
            }
            Cmd::Lang(l) => {
                self.c.s.lang = l;
                i18n::set(l);
                self.apply_dock_translations();
            }
            Cmd::Folders => self.c.folders_open = true,
            Cmd::Help => self.c.help_open = true,
        }
    }

    /// パレットの項目
    pub(super) fn palette_items(&self) -> Vec<Item<Cmd>> {
        let mut v = Vec::new();
        let mut push = |label: String, hint: &str, cmd: Cmd| {
            v.push(Item {
                label,
                hint: hint.into(),
                danger: matches!(cmd, Cmd::New | Cmd::Reload),
                cmd,
            })
        };
        push(
            tr("新規（組合せを空にする）").into(),
            "new ctrl+n",
            Cmd::New,
        );
        push(
            tr("系のファイルを開く…").into(),
            "open file ctrl+o",
            Cmd::Open,
        );
        push(
            tr("元に戻す（Ctrl+Z）").into(),
            "undo ctrl+z",
            Cmd::Undo(true, 1),
        );
        push(
            tr("やり直す（Ctrl+Y）").into(),
            "redo ctrl+y ctrl+shift+z",
            Cmd::Undo(false, 1),
        );
        push(tr("系を読み直す").into(), "reload", Cmd::Reload);
        push(
            tr("開いている解析をすべて計算し直す").into(),
            "run all f5",
            Cmd::RunAll,
        );
        push(
            tr(".zmx に書き出す…").into(),
            "write zmx export",
            Cmd::WriteZmx,
        );
        push(tr("保存（Ctrl+S）").into(), "save ctrl+s", Cmd::Save(false));
        push(
            tr("名前を付けて保存…（Ctrl+Shift+S）").into(),
            "save as ctrl+shift+s",
            Cmd::Save(true),
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
                trf("パネルを出す: {}", &[&t])
            } else {
                trf("パネルを足す: {}", &[&t])
            };
            push(label, &hint, Cmd::Add(k));
        }
        for (_, p) in self.dock.iter_all_tabs() {
            let name = match (&p.kind, p.kind.tool()) {
                (Kind::Analysis(a), _) => a.cmd.clone(),
                (_, Some(t)) => t.name.to_string(),
                _ => String::new(),
            };
            push(
                trf("パネルへ移る: {}", &[&p.title()]),
                &format!("go focus panel {name}"),
                Cmd::Focus(p.id),
            );
            push(
                format!(
                    "{}: {}",
                    detach_label(cfg!(target_arch = "wasm32")),
                    p.title()
                ),
                &format!("window monitor detach {name}"),
                Cmd::ToWindow(p.id),
            );
        }
        for p in Preset::ALL {
            push(
                trf("並び: {}", &[&p.label()]),
                "layout preset",
                Cmd::Preset(p),
            );
        }
        for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
            push(
                trf("保存した並び: {}", &[n]),
                "layout saved",
                Cmd::Layout(i),
            );
        }
        push(tr("今の並びを保存…").into(), "save layout", Cmd::SaveLayout);
        push(
            tr("配色: 暗い").into(),
            "theme dark",
            Cmd::Theme(Mode::Dark),
        );
        push(
            tr("配色: 明るい").into(),
            "theme light",
            Cmd::Theme(Mode::Light),
        );
        push(
            tr("配色: OS に合わせる").into(),
            "theme system",
            Cmd::Theme(Mode::System),
        );
        for i in 0..theme::ACCENTS.len() {
            push(
                trf("アクセント色 {}", &[&(i + 1)]),
                "accent color",
                Cmd::Accent(i),
            );
        }
        for l in Lang::ALL {
            push(trf("言語: {}", &[&l.label()]), "language", Cmd::Lang(l));
        }
        push(
            tr("カタログのフォルダ…").into(),
            "folders settings",
            Cmd::Folders,
        );
        push(tr("キーボード操作の一覧").into(), "help keys f1", Cmd::Help);
        for f in &self.c.s.recent {
            push(
                trf("最近のファイル: {}", &[f]),
                "recent",
                Cmd::Recent(f.clone()),
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
                ui.menu_button(tr("ファイル"), |ui| {
                    if ui.button(tr("新規（Ctrl+N）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::New);
                    }
                    if ui.button(tr("開く…（Ctrl+O）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Open);
                    }
                    if !self.c.samples.is_empty() {
                        ui.menu_button(tr("見本"), |ui| {
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
                                    ui.weak(tr("組合せ（開くと組合せに戻る）"));
                                }
                                last = Some(g);
                                let name = sample_name(f);
                                if ui.button(name).on_hover_text(f).clicked() {
                                    ui.close_menu();
                                    cmd = Some(Cmd::Recent(f.clone()));
                                }
                            }
                        });
                    }
                    ui.menu_button(tr("最近のファイル"), |ui| {
                        if self.c.s.recent.is_empty() {
                            ui.label(tr("（なし）"));
                        }
                        for f in &self.c.s.recent {
                            if ui.button(f).clicked() {
                                ui.close_menu();
                                cmd = Some(Cmd::Recent(f.clone()));
                            }
                        }
                    });
                    if ui
                        .add_enabled(
                            self.c.file.is_some(),
                            egui::Button::new(tr("読み直す（Ctrl+R）")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Reload);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.c.summary.is_some(),
                            egui::Button::new(tr(".zmx に書き出す…")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::WriteZmx);
                    }
                    let can_save = self.c.file.is_some() || !self.c.browser.combo.picks.is_empty();
                    // 開いた .zmx は系ファイル（.json）に保存する（.zmx は上書きしない）ことを名前で示す（GUX-26）
                    let zmx = !self.c.is_combo()
                        && self.c.doc_path.is_none()
                        && self.c.file.as_ref().is_some_and(|f| {
                            f.extension().is_some_and(|x| x.eq_ignore_ascii_case("zmx"))
                        });
                    let save_label = if zmx {
                        tr("系ファイル（.json）として保存…（Ctrl+S）")
                    } else {
                        tr("保存（Ctrl+S）")
                    };
                    if ui
                        .add_enabled(can_save, egui::Button::new(save_label))
                        .on_hover_text(tr(
                            "組合せの系は組合せごとプロジェクトの .json に保存（開くと組合せに戻る）。開いた系は系ファイルに",
                        ))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Save(false));
                    }
                    if ui
                        .add_enabled(
                            can_save,
                            egui::Button::new(tr("名前を付けて保存…（Ctrl+Shift+S）")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Save(true));
                    }
                    ui.separator();
                    if ui.button(tr("カタログのフォルダ…")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Folders);
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let (n, b) = crate::web_store::usage();
                        if ui
                            .add_enabled(
                                n > 0,
                                egui::Button::new(trf(
                                    "ブラウザに残したファイルを消す（{} 個、{} MB）…",
                                    &[&n, &format!("{:.1}", b as f64 / 1e6)],
                                )),
                            )
                            .on_hover_text(tr(
                                "読み込んだファイルとユーザーカタログは、ページを閉じてもブラウザに残る",
                            ))
                            .clicked()
                        {
                            ui.close_menu();
                            self.c.ask(
                                "lensbench",
                                tr("ブラウザに残した、読み込んだファイルとユーザーカタログを全部消します。よろしいですか？"),
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
                        if ui.button(tr("終了")).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
                ui.menu_button(tr("編集"), |ui| {
                    let (u, r) = (self.c.can_undo(true), self.c.can_undo(false));
                    if ui
                        .add_enabled(u, egui::Button::new(tr("元に戻す（Ctrl+Z）")))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Undo(true, 1));
                    }
                    if ui
                        .add_enabled(r, egui::Button::new(tr("やり直す（Ctrl+Y）")))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Undo(false, 1));
                    }
                    // 段の一覧: 選んだ段まで（その段を含めて）まとめて戻す・やり直す
                    for (back, title) in [(true, tr("ここまで元に戻す")), (false, tr("ここまでやり直す"))] {
                        let list = self.c.undo_list(back);
                        ui.add_enabled_ui(!list.is_empty(), |ui| {
                            ui.menu_button(title, |ui| {
                                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                                    for (i, l) in list.iter().enumerate() {
                                        if ui.button(format!("{}. {l}", i + 1)).clicked() {
                                            ui.close_menu();
                                            cmd = Some(Cmd::Undo(back, i + 1));
                                        }
                                    }
                                });
                            });
                        });
                    }
                    ui.label(
                        RichText::new(tr(
                            "系の変更（面の表・多重構成・最適化の結果・組合せ）と、系を開く・読み直す・新規を 100 段まで",
                        ))
                        .small()
                        .weak(),
                    );
                });
                // 1 枚だけのパネルは「出す」（あれば表に）、解析は「足す」を下の段に分ける（GUX-16）
                ui.menu_button(tr("パネル"), |ui| {
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
                            .on_hover_text(tr("1 枚だけのパネル。あれば表に出す"))
                            .clicked()
                        {
                            ui.close_menu();
                            cmd = Some(Cmd::Add(k));
                        }
                    }
                    ui.separator();
                    ui.menu_button(tr("解析を足す"), |ui| {
                        for k in many {
                            let t = Pane {
                                id: 0,
                                kind: k.clone(),
                            }
                            .title();
                            if ui.button(trf("＋ {}", &[&t])).clicked() {
                                ui.close_menu();
                                cmd = Some(Cmd::Add(k));
                            }
                        }
                    });
                });
                ui.menu_button(tr("画面の配置"), |ui| {
                    ui.label(RichText::new(tr("初期の並び")).small().color(weak));
                    for p in Preset::ALL {
                        if ui.button(p.label()).clicked() {
                            ui.close_menu();
                            cmd = Some(Cmd::Preset(p));
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new(tr("保存した並び")).small().color(weak));
                    if self.c.s.layouts.is_empty() {
                        ui.label(tr("（なし）"));
                    }
                    let mut del = None;
                    for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button(n).clicked() {
                                ui.close_menu();
                                cmd = Some(Cmd::Layout(i));
                            }
                            if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                                del = Some(i);
                            }
                        });
                    }
                    if let Some(i) = del {
                        self.c.s.layouts.remove(i);
                    }
                    ui.separator();
                    if ui.button(tr("今の並びを保存…")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::SaveLayout);
                    }
                });
                ui.menu_button(tr("表示"), |ui| {
                    ui.label(RichText::new(tr("配色")).small().color(weak));
                    for (m, l) in [
                        (Mode::Dark, tr("暗い")),
                        (Mode::Light, tr("明るい")),
                        (Mode::System, tr("OS に合わせる")),
                    ] {
                        if ui.radio(self.c.s.look.mode == m, l).clicked() {
                            cmd = Some(Cmd::Theme(m));
                        }
                    }
                    ui.label(RichText::new(tr("アクセント色")).small().color(weak));
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
                                cmd = Some(Cmd::Accent(i));
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
                        ui.checkbox(&mut self.c.s.os_titlebar, tr("OS のタイトルバーを使う"))
                            .on_hover_text(tr(
                                "外すと上の帯をタイトルバーにする（ドラッグで移動、ダブルクリックで最大化、縁で大きさを変える）",
                            ));
                    }
                    ui.separator();
                    ui.label(RichText::new(tr("言語")).small().color(weak));
                    for l in Lang::ALL {
                        if ui.radio(self.c.s.lang == l, l.label()).clicked() {
                            cmd = Some(Cmd::Lang(l));
                        }
                    }
                    ui.separator();
                    if ui.button(tr("キーボード操作の一覧（F1）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Help);
                    }
                });
                ui.menu_button(tr("ヘルプ"), |ui| {
                    for (label, page, tip) in HELP_PAGES {
                        if ui
                            .button(tr(label))
                            .on_hover_text(tr(tip))
                            .clicked()
                        {
                            ui.close_menu();
                            ctx.open_url(egui::OpenUrl::new_tab(docs_url(page)));
                        }
                    }
                    ui.separator();
                    if ui.button(tr("キーボード操作の一覧（F1）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Help);
                    }
                    ui.separator();
                    ui.label(RichText::new(version()).small().color(weak));
                });
            }
            let shown = if self.c.is_combo() {
                self.c.doc_path.clone()
            } else {
                self.c.file.clone()
            };
            let dirty = self.c.dirty();
            if shown.is_some() || dirty {
                let name = shown
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_else(|| tr("（未保存）").to_string());
                let mark = if dirty { " ●" } else { "" };
                let r = ui.label(RichText::new(format!("{name}{mark}")).strong());
                let tip = shown.map(|f| f.display().to_string()).unwrap_or_default();
                if dirty {
                    r.on_hover_text(format!("{tip}\n{}", tr("● 保存していない変更がある（Ctrl+S で保存）")));
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
                    RichText::new(tr("▶ 計算（F5）"))
                        .strong()
                        .color(Color32::from_rgb(6, 18, 28)),
                )
                .fill(accent)
                .min_size(egui::vec2(0.0, 28.0));
                if ui.add_enabled(self.c.summary.is_some(), run).clicked() {
                    cmd = Some(Cmd::RunAll);
                }
                // 1 段の元に戻す・やり直す（読み直すはファイルのメニュー、GAR-15）。押すと何を戻すかをホバーで
                for (back, icon, key) in [(false, ["↷", ">"], "Ctrl+Y"), (true, ["↶", "<"], "Ctrl+Z")] {
                    let next = self.c.undo_list(back).into_iter().next();
                    let tip = match (&next, back) {
                        (Some(n), true) => trf("元に戻す（{}）: {}", &[&key, n]),
                        (Some(n), false) => trf("やり直す（{}）: {}", &[&key, n]),
                        (None, true) => tr("元に戻す段は無い").to_string(),
                        (None, false) => tr("やり直す段は無い").to_string(),
                    };
                    if ui
                        .add_enabled(
                            self.c.can_undo(back),
                            egui::Button::new(RichText::new(crate::fonts::glyph_or(ui, icon[0], icon[1])).size(16.0))
                                .min_size(egui::vec2(28.0, 28.0)),
                        )
                        .on_hover_text(tip.clone())
                        .on_disabled_hover_text(tip)
                        .clicked()
                    {
                        cmd = Some(Cmd::Undo(back, 1));
                    }
                }
                // 探す欄は残りの幅に合わせて縮める（狭ければ 🔍 だけ。左のメニューに重ねない、GUX-03）
                let room = ui.available_width() - 8.0;
                let label = if room >= 300.0 {
                    format!("🔍  {}    Ctrl K", tr("解析・パネル・操作を探す"))
                } else if room >= 150.0 {
                    format!("🔍  {}", tr("探す"))
                } else {
                    "🔍".to_string()
                };
                let search = egui::Button::new(RichText::new(label).color(weak))
                    .min_size(egui::vec2(room.clamp(28.0, 300.0), 28.0));
                let r = ui.add(search).on_hover_text(tr("解析・パネル・操作を探す（Ctrl+K）"));
                self.palette.anchor = Some(r.rect);
                if r.clicked() {
                    self.palette.show();
                }
                if custom {
                    crate::chrome::drag_area(ui);
                }
            });
        });
        if let Some(c) = cmd {
            self.exec(ctx, c);
        }
    }
}

/// ファイルの「見本」の一覧: Web 版は置き場に入れたもの（引数）、アプリ版は exe に同梱したもの（GUX-26）
pub(super) fn samples(args: &Args) -> Vec<String> {
    #[cfg(not(target_arch = "wasm32"))]
    if args.samples.is_empty() {
        return crate::samples::list();
    }
    args.samples.clone()
}

impl Core {
    /// アプリ版で同梱の見本を開く前に: 書き出し、見本のガラス・レンズのフォルダを設定に足す（GUX-26）
    fn prepare_sample(&mut self, _f: &str) {
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
                    self.fail(trf("見本を書き出せない: {}", &[&e]));
                }
            }
        }
    }
}

/// ヘルプのメニューのページ（名前、マニュアルの中のページ、説明）
const HELP_PAGES: &[(&str, &str, &str)] = &[
    (n_("マニュアル"), "", n_("使い方の目次（ブラウザで開く）")),
    (
        n_("GUI の使い方"),
        "gui.html",
        n_("画面・パネルの使い方（図つき）"),
    ),
    (
        n_("解析と CLI"),
        "cli.html",
        n_("解析の種類と引数、出力の例"),
    ),
    (
        n_("プロジェクトファイル"),
        "project-file.html",
        n_("ブロックで系を組む JSON（状態・群・BS）"),
    ),
    (n_("最適化"), "optimize.html", n_("評価関数・変数・DLS")),
    (n_("公差"), "tolerance.html", n_("モンテカルロ公差の設定")),
];

/// パネルを外に出す操作の名前。Web 版はブラウザの別の窓（ポップアップ）に出し、元のタブで描いた画面を送って
/// 別の窓で操作する（#137・GAR-10）
pub(super) fn detach_label(web: bool) -> &'static str {
    if web {
        tr("別の窓に出す")
    } else {
        tr("別の窓に出す（別モニターへ）")
    }
}

#[test]
fn detach_label_web() {
    assert_eq!(detach_label(true), "別の窓に出す");
    assert_eq!(detach_label(false), "別の窓に出す（別モニターへ）");
}
