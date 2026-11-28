//! 毎フレームの描かない処理（結果の受け取り・解析の回し直し・系の作り直しなど）と、Web 版の別の窓のパネル（web_tabs）・元のタブが
//! 裏に回っている間の処理

use super::*;

impl App {
    /// 毎フレームの描かない処理。画面を描く前に呼ぶ（[`eframe::App::update`]）。Web 版で元のタブが裏に
    /// 回っている間は [`App::background`] からも呼ぶ
    pub(super) fn logic(&mut self, ctx: &egui::Context) {
        // ソルブ・ピックアップで厚みが決まる間隔は、図・要素の欄・システムの設定で変えない（レンズデータの表と同じ）
        let solved = self.c.solved_gaps();
        combo::SolvedGap::store(ctx, solved.clone());
        self.c.p.combo_view.solved = solved;
        self.c.mech_check_tick(ctx);
        // Web 版: 選び終わったファイルを開く・書かれたファイルをダウンロードにする
        self.c.poll_open();
        self.c.poll_replay();
        self.c.poll_saves();
        crate::files::flush_downloads();
        if crate::files::take_failed() {
            self.c.fail(tr("Could not open the file dialog (Linux needs zenity). You can also drop files onto the window"));
        }
        // Web 版: 読み込んだファイル・ユーザーカタログをブラウザに残す
        // 解析のパネルの別の窓は残さない（元のタブが同じ所に後から書いたものを、古い写しで戻さない）
        #[cfg(target_arch = "wasm32")]
        if self.calc.is_none() {
            crate::web_store::flush();
        }
        let mut redo = Vec::new();
        while let Some(d) = self.c.worker.poll() {
            if let Tag::Analyze(id) = d.tag {
                self.c.ana_back(id);
            }
            if !d.skipped {
                self.c.take(d);
            } else if let Tag::Analyze(id) | Tag::Rays(id) = d.tag {
                redo.push(id);
            }
        }
        // 系が変わって捨てた解析: 自動のパネルは系の変更のあとにまとめて回し直すので、自動でないパネル
        // （頼まれて回していたもの）だけ新しい系で頼み直す
        if !redo.is_empty() {
            redo.sort_unstable();
            redo.dedup();
            for (id, cfg) in self.analyses() {
                if !cfg.live && redo.contains(&id) {
                    self.c.run(id, &cfg);
                }
            }
        }
        while let Some(mut d) = self.c.preview.poll() {
            // カタログの一覧は系の要求と同じに扱う（状態の行・失敗の表示も）
            if matches!(
                d.tag,
                Tag::Glasses | Tag::Lenses | Tag::UserCatalog(_) | Tag::CatalogFill
            ) {
                // 失敗しても読み込み中は終わる
                match d.tag {
                    Tag::Glasses => self.c.browser.glasses_pending = false,
                    Tag::Lenses => self.c.browser.lenses_pending = false,
                    _ => {}
                }
                self.c.take(d);
                continue;
            }
            // 失敗は系の読み込みの方で出す。組合せの図は返事を複製せずに受け取る（コピー用に持つ）
            if d.tag == Tag::Preview {
                if d.error().is_none() && !d.skipped && !d.replies.is_empty() {
                    let (_, v) = d.take_first();
                    self.c.layout_data = Some(combo::LayoutData::from_value(v));
                }
                continue;
            }
            let (_, params, r) = d.first();
            match (&d.tag, r) {
                (Tag::Folders, Ok(v)) => self.c.take_folders(params, v),
                (Tag::CadMeasure, r) => self.c.browser.cad.measured(params, r.map_err(|e| &**e)),
                // 読めないファイルは系の読み込みの方で誤りを出す
                (Tag::OpenDoc, r) => {
                    if let Some(f) = params["file"].as_str() {
                        let f = PathBuf::from(f);
                        self.c.open_doc_peeked(&f, r.ok());
                    }
                }
                // カタログの玉の断面（読めなければ空の図）
                // 頼んだ行の結果だけ受け取る（カタログを読み直す前に頼んだ分は捨てる）
                (Tag::LensPreview(i), r) if self.c.preview_req == Some(*i) => {
                    self.c.preview_req = None;
                    let mut data = r.ok().map(combo::LayoutData::parse).unwrap_or_default();
                    // 負の単レンズの断面は虚物体（前側焦点へ集まる光）。入る光を玉の前から描く
                    data.virtual_object_from_front();
                    self.c.browser.preview = Some((*i, data));
                }
                _ => {}
            }
        }
        // 取った品の STEP と簡易外形の照合（Web 版。品の載ったカタログは下書きの Worker が持つ）
        for p in self.c.browser.cad.take_measure() {
            self.c.draft(Tag::CadMeasure, p);
        }
        // 組合せを動かしている間は解析を待たせ、系の作り直しとレイアウト図を先に回す
        let settle = std::time::Duration::from_millis(350);
        let moving =
            self.c.loc.file.is_some() && self.c.combo_edit.is_some_and(|t| t.elapsed() < settle);
        if moving && self.c.rerun.is_some() {
            ctx.request_repaint_after(settle);
        } else if let Some(live) = self.c.rerun.take() {
            self.run_all(live);
        }
        self.c.flush_set();
        // Web 版の別の窓: 組合せは元のタブの写し。手直し・作り直し・焦点合わせは元のタブがする
        if self.c.popup_doc.copies.follower {
            self.c.doc.sync();
            return;
        }
        let mut focus = ctx.memory(|m| m.focused());
        let mut busy = ctx.input(|i| i.pointer.any_down()) || focus.is_some();
        // Web 版: 写しを持つ別の窓で欄を書き換えている間も、元に戻す段を区切らない
        for (id, (b, f)) in &self.c.popup_doc.copies.busy {
            busy |= *b;
            if focus.is_none() {
                focus = f.map(|f| egui::Id::unique(("copy-window", *id, f)));
            }
        }
        let dropped = self.c.tidy_combo();
        // 消す操作（要素・状態・面の上書き）の後は、戻し方を状態の行に出す（確認の窓は出さない）
        let census = self.c.doc.combo.census();
        if let Some(prev) = self.c.combo_census.replace(census) {
            let lost: Vec<String> = [
                (prev[0].saturating_sub(census[0]), tr("{} elements")),
                (prev[1].saturating_sub(census[1]), tr("{} states")),
                (
                    prev[2].saturating_sub(census[2]),
                    tr("{} surface overrides"),
                ),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, f)| f.replace("{}", &n.to_string()))
            .collect();
            // 新規・開くで組合せごと入れ替えたときは出さない
            if !lost.is_empty() && census[0] > 0 && dropped == 0 {
                self.c.info(trf(
                    "Removed {} (Ctrl+Z to undo)",
                    &[&lost.join(crate::i18n::tr(", "))],
                ));
            }
        }
        // 組合せの版をここで進める（以下と次の画面の「写しと同じか」は版で見る）
        self.c.doc.sync();
        self.c
            .combo_hist
            .track(&self.c.doc.combo, self.c.doc.rev, busy, focus);
        // 最後に玉を足した組合せの系ができたら、像面を近軸の焦点へ
        if self.c.doc.refocus
            && self.c.loc.file.is_some()
            && self.c.loc.built_combo.same(&self.c.doc)
            && self.c.layout_data.is_some()
            && !self.c.busy_short()
        {
            self.c.doc.refocus = false;
            self.c.request_focus();
        }
        self.c.auto_build();
        self.sync_stray(ctx);
        self.nsc_request(ctx);
        self.bs_at_request(ctx);
    }

    /// BS の欄で「大域の座標に置く」を入れた: 組合せを展開して BS の今の置き方を求め、`at` に書く（元に戻せる書き換え）
    fn bs_at_request(&mut self, ctx: &egui::Context) {
        let Some(loc) = combo::join::take_at_request(ctx) else {
            return;
        };
        let paths = |v: &[String]| v.iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
        let glass = paths(&self.c.s.glass_dirs);
        let cat =
            lensbench_core::catalog::LensCatalog::load_registered(&paths(&self.c.s.lens_dirs));
        let base = (self.c.loc.combo_path.as_ref())
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(doc_loc::work_dir);
        let project = self.c.doc.combo.project().to_string();
        let frames = crate::work_fs::with(|| {
            let cat = cat.map_err(|e| e.to_string())?;
            let look = |v: &str, q: &str| cat.zmx(v, q);
            let env = lensbench_project::Env {
                catalog: Some(&look),
                ..lensbench_project::Env::in_dir(&base, &glass, None)
            };
            let p = lensbench_project::Project::from_json(&project).map_err(|e| e.to_string())?;
            lensbench_project::splitter_frames(&p, &env, None).map_err(|e| e.to_string())
        });
        let r = frames.and_then(|f| {
            edit::stage(&mut self.c.doc.combo, &mut self.c.msgs, |c| {
                combo::join::place_at(c, &loc, &f)
            })
        });
        if let Err(e) = r {
            self.c.fail(e);
        }
    }

    /// 非順次の経路の表から頼まれた操作: 組合せに迷光の光束を足す（元に戻せる書き換え）・ゴーストの系を開く
    fn nsc_request(&mut self, ctx: &egui::Context) {
        match crate::nsc_view::take_request(ctx) {
            Some(crate::nsc_view::NscAct::AddBeam { path, turn }) => {
                let r = edit::stage(&mut self.c.doc.combo, &mut self.c.msgs, |c| {
                    combo::beams::add_stray(c, path, turn)
                });
                match r {
                    Ok((name, true)) => self.c.info(trf("Added the beam {}", &[&name])),
                    Ok((name, false)) => self
                        .c
                        .info(trf("The beam {} already has this path", &[&name])),
                    Err(e) => self.c.fail(e),
                }
            }
            Some(crate::nsc_view::NscAct::Open(p)) => self.c.open_doc_unlisted(&p),
            None => {}
        }
    }

    /// レイアウト図に重ねる非順次の枝: 「レイアウト図に重ねる」を入れた最初の nsc の解析のパネルの、今の系の結果から
    fn sync_stray(&mut self, ctx: &egui::Context) {
        let found = self
            .dock
            .iter_all_tabs()
            .map(|(_, p)| p)
            .chain(self.windows.iter().map(|w| &w.pane))
            .find_map(|p| match &p.kind {
                Kind::Analysis(a) if a.cmd == "nsc" && a.fig.nsc.layout => {
                    let r = self.c.results.get(&p.id)?;
                    r.value.as_ref()?;
                    // 前の系の結果の枝は今のレイアウト図とずれるので重ねない
                    (r.sys_gen == self.c.sys_gen).then_some(())?;
                    Some((p.id, r.gen, a.fig.nsc.clone()))
                }
                _ => None,
            });
        crate::nsc_view::set_stray(ctx, found, |id| {
            self.c.results.get(&id).and_then(|r| r.value.as_ref())
        });
    }

    /// Web 版: 元のタブが裏に回っている間（ブラウザが画面を止める）に、別の窓の入力・計算の結果を合図に
    /// 回す（[`crate::web_bg`]）。描かない処理を回し、別の窓に出したパネルを描いて送る。
    /// 別の窓が無ければ何もしない（タブが前に戻れば、いつもの画面で受け取る）
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn background(&mut self, ctx: &egui::Context) {
        if self.viewer.is_some() || !self.tabs.as_ref().is_some_and(|t| t.any_open()) {
            return;
        }
        self.logic(ctx);
        self.web_tabs(ctx);
        // 写しを持つ別の窓から届いた操作も当てて、変わった写しを送る
        if !self.c.msgs.is_empty() {
            self.dispatch_queued(ctx);
            self.flush_fx(ctx);
            self.web_tabs(ctx);
        }
    }

    /// Web 版（元のタブ）: 別の窓に出したパネルを別の窓で開き、写しを持つ窓には系・組合せ・履歴の写しを、ほかの窓には
    /// 元のタブで描いた画面を送る。写しを持つ窓でした操作は、ここで操作の列に入れて元のタブで当てる。
    /// 窓を閉じたらドックに戻す
    #[cfg(target_arch = "wasm32")]
    pub(super) fn web_tabs(&mut self, ctx: &egui::Context) {
        use crate::web_tabs::Ev;
        let Some(t) = self.tabs.as_mut() else {
            return;
        };
        t.retain(&self.windows.iter().map(|w| w.pane.id).collect::<Vec<_>>());
        self.hosts
            .retain(|id, _| self.windows.iter().any(|w| w.pane.id == *id));
        for w in &self.windows {
            let id = w.pane.id;
            // 写しを持つパネル（解析・文書のパネル）は別の窓が自分で計算して描く
            let calc = copy_panel::copies(&w.pane.kind);
            if !t.is_open(id) && !t.blocked.contains(&id) && !t.open(id, w.size, calc) {
                self.c.fail(tr("The browser blocked opening another tab or window (allow pop-ups to move panels to another tab or window). Showing it as a window inside the page"));
            }
        }
        for e in t.events() {
            match e {
                // 解析のパネル: パネルの設定を送り、系を送り直す
                Ev::Hello(id) if t.is_calc(id) => {
                    if let Some(w) = self.windows.iter().find(|w| w.pane.id == id) {
                        if let Ok(v) = serde_json::to_value(&w.pane) {
                            t.send_pane(id, &v);
                        }
                    }
                    self.c.popup_restarted(id);
                    self.c.popup_doc.copies.sync.restart(id);
                }
                Ev::Flat(id) => self.c.popup_wants_flat(id),
                Ev::Resend(id) => self.c.popup_restarted(id),
                Ev::Recopy(id) => self.c.popup_doc.copies.sync.restart(id),
                Ev::Busy(id, busy, focus) => {
                    self.c.popup_doc.copies.busy.insert(id, (busy, focus));
                }
                Ev::Ops(id, ops) => {
                    for op in ops {
                        match serde_json::from_value::<copy_panel::Fwd>(op) {
                            Ok(f) => self.c.take_fwd(id, f),
                            Err(e) => self.c.fail(format!(
                                "{} {id}: {e}",
                                tr("Could not apply an operation from another window")
                            )),
                        }
                    }
                }
                Ev::Pane(id, v) => {
                    let p = serde_json::from_value::<Pane>(v)
                        .ok()
                        .filter(|p| p.id == id);
                    let w = self.windows.iter_mut().find(|w| w.pane.id == id);
                    if let (Some(w), Some(p)) = (w, p) {
                        w.pane = p;
                    }
                }
                Ev::Hello(id) => {
                    self.hosts.insert(id, crate::web_tabs::Host::new(ctx));
                }
                Ev::Input(id, v) => {
                    self.hosts
                        .entry(id)
                        .or_insert_with(|| crate::web_tabs::Host::new(ctx))
                        .input(&v);
                }
                Ev::Gone(id) => {
                    self.c.popup_doc.copies.busy.remove(&id);
                    self.hosts.remove(&id);
                    if let Some(k) = self.windows.iter().position(|w| w.pane.id == id) {
                        let w = self.windows.remove(k);
                        self.win_init.remove(&id);
                        // 別の窓で計算していた解析は、元のタブで回し直す
                        if let Kind::Analysis(cfg) = &w.pane.kind {
                            self.c.run(id, cfg);
                        }
                        crate::layout::redock(&mut self.dock, w.pane, w.home.as_ref());
                    }
                }
            }
        }
        // 解析のパネルの別の窓に今の系を送る
        let calc: Vec<u64> = self
            .windows
            .iter()
            .map(|w| w.pane.id)
            .filter(|id| t.is_calc(*id))
            .collect();
        self.c.send_popup_doc(t, &calc);
        // 文書のパネルの別の窓に、組合せ（と履歴のパネルには元に戻す段）の写しを送る
        let open: Vec<u64> = self.windows.iter().map(|w| w.pane.id).collect();
        self.c.popup_doc.copies.sync.retain(&open);
        self.c
            .popup_doc
            .copies
            .busy
            .retain(|id, _| open.contains(id));
        for w in &self.windows {
            let id = w.pane.id;
            if !t.is_calc(id) || !copy_panel::needs_combo(&w.pane.kind) {
                continue;
            }
            let c = &mut self.c;
            if let Some(v) = c
                .popup_doc
                .copies
                .sync
                .combo_for(id, c.doc.rev, &c.doc.combo)
            {
                t.send_copy(id, &json!({ "combo": v }));
            }
            if let Some(v) = c
                .popup_doc
                .copies
                .sync
                .view_for(id, copy_panel::ViewCopy::of(c))
            {
                t.send_copy(id, &json!({ "view": v }));
            }
            if w.pane.kind == Kind::History {
                let h = copy_panel::HistCopy::of(c);
                if let Some(v) = c.popup_doc.copies.sync.hist_for(id, h) {
                    t.send_copy(id, &json!({ "hist": v }));
                }
            }
        }
        // 別の窓のパネル: 元のタブで描いて、変わった画面を送る（解析のパネルも同じ）
        let App {
            c, windows, hosts, ..
        } = self;
        for w in windows.iter_mut() {
            let id = w.pane.id;
            if t.is_calc(id) {
                continue;
            }
            let Some(h) = hosts.get_mut(&id) else {
                continue;
            };
            if !t.is_open(id) {
                continue;
            }
            let pane = &mut w.pane;
            h.title = pane.title();
            let (frame, out) = h.frame(ctx, |ui| {
                c.drawing_in = Some(id);
                c.pane_ui(ui, pane);
                c.drawing_in = None;
                // この窓で出た確かめはこの窓に出す
                if c.ask_from() == Some(id) {
                    c.ask_ui(ui.ctx());
                }
            });
            if let Some(f) = frame {
                t.send_frame(id, &f);
            }
            if let Some(o) = out {
                t.send_out(id, o);
            }
            // 元のタブが裏に回っている間は、パネルの描き直しの頼みを別の窓からの合図に回す
            if let Some(ms) = h.take_again().filter(|_| crate::web_bg::hidden()) {
                t.send_again(id, ms);
            }
        }
        // 画面を送らなかった窓にも、入力を受けたと返す（別の窓は返事が無いと「元のタブが止まった」と出す）
        t.flush_acks();
        // 子のタブを閉じたのに気づく
        if t.any_open() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}
