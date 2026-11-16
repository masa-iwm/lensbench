//! 毎フレームの描かない処理（結果の受け取り・解析の回し直し・系の作り直しなど）と、Web 版の別の窓のパネル（web_tabs）・元のタブが
//! 裏に回っている間の処理（#237）

use super::*;

impl App {
    /// 毎フレームの描かない処理。画面を描く前に呼ぶ（[`eframe::App::update`]）。Web 版で元のタブが裏に
    /// 回っている間は [`App::background`] からも呼ぶ
    pub(super) fn logic(&mut self, ctx: &egui::Context) {
        // Web 版: 選び終わったファイルを開く・書かれたファイルをダウンロードにする
        self.c.pick_and_open_when(false);
        crate::files::flush_downloads();
        if crate::files::take_failed() {
            self.c.fail(tr("Could not open the file dialog (Linux needs zenity). You can also drop files onto the window"));
        }
        // Web 版: 読み込んだファイル・ユーザーカタログをブラウザに残す
        #[cfg(target_arch = "wasm32")]
        crate::web_store::flush();
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
        while let Some(d) = self.c.preview.poll() {
            // カタログの一覧は系の要求と同じに扱う（状態の行・失敗の表示も）
            if matches!(
                d.tag,
                Tag::Glasses | Tag::Lenses | Tag::UserCatalog | Tag::CatalogFill
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
            // 失敗は系の読み込みの方で出す
            let (_, params, r) = d.first();
            match (&d.tag, r) {
                (Tag::Layout, Ok(v)) => self.c.layout_data = Some(combo::LayoutData::parse(v)),
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
        // 取った品の STEP と簡易外形の照合（Web 版。品の載ったカタログは下書きの Worker が持つ、GAR-11）
        for p in self.c.browser.cad.take_measure() {
            self.c.preview.send(Tag::CadMeasure, "cad_measure", p);
        }
        // 組合せを動かしている間は解析を待たせ、系の作り直しとレイアウト図を先に回す
        let settle = std::time::Duration::from_millis(350);
        let moving = self.c.is_combo() && self.c.combo_edit.is_some_and(|t| t.elapsed() < settle);
        if moving && self.c.rerun.is_some() {
            ctx.request_repaint_after(settle);
        } else if let Some(live) = self.c.rerun.take() {
            self.run_all(live);
        }
        self.c.flush_set();
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        let dropped = self.c.doc.combo.drop_stale_edits();
        if dropped > 0 {
            self.c.info(trf(
                "The element's surfaces changed, so the values changed in Lens Data ({}) were dropped (Ctrl+Z to undo)",
                &[&dropped],
            ));
        }
        // 消す操作（要素・状態・面の上書き）の後は、戻し方を状態の行に出す（確認の窓は出さない、GPN-20）
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
        // 組合せの版をここで進める（以下と次の画面の「写しと同じか」は版で見る、GPN-13）
        self.c.doc.sync();
        self.c
            .combo_hist
            .track(&self.c.doc.combo, self.c.doc.rev, busy);
        // 最後に玉を足した組合せの系ができたら、像面を近軸の焦点へ（GUX-10）
        if self.c.doc.refocus
            && self.c.is_combo()
            && self.c.loc.built_combo.same(&self.c.doc)
            && self.c.layout_data.is_some()
            && !self.c.worker.busy_short()
        {
            self.c.doc.refocus = false;
            self.c.request_focus();
        }
        self.c.auto_build();
    }

    /// Web 版: 元のタブが裏に回っている間（ブラウザが画面を止める）に、別の窓の入力・計算の結果を合図に
    /// 回す（[`crate::web_bg`]）。描かない処理を回し、別の窓に出したパネルを描いて送る（#237）。
    /// 別の窓が無ければ何もしない（タブが前に戻れば、いつもの画面で受け取る）
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn background(&mut self, ctx: &egui::Context) {
        if self.viewer.is_some() || !self.tabs.as_ref().is_some_and(|t| t.any_open()) {
            return;
        }
        self.logic(ctx);
        self.web_tabs(ctx);
    }

    /// Web 版（元のタブ）: 別の窓に出したパネルを別の窓で開き、元のタブで描いた画面を送る（GAR-10）。
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
            if !t.is_open(id) && !t.blocked.contains(&id) && !t.open(id, w.size) {
                self.c.fail(tr("The browser blocked opening another tab or window (allow pop-ups to move panels to another tab or window). Showing it as a window inside the page"));
            }
        }
        for e in t.events() {
            match e {
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
                    self.hosts.remove(&id);
                    if let Some(k) = self.windows.iter().position(|w| w.pane.id == id) {
                        let w = self.windows.remove(k);
                        self.win_init.remove(&id);
                        crate::layout::redock(&mut self.dock, w.pane, w.home.as_ref());
                    }
                }
            }
        }
        // 別の窓のパネル: 元のタブで描いて、変わった画面を送る（解析のパネルも同じ。GAR-10）
        let App {
            c, windows, hosts, ..
        } = self;
        for w in windows.iter_mut() {
            let id = w.pane.id;
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
            // 元のタブが裏に回っている間は、パネルの描き直しの頼みを別の窓からの合図に回す（#237）
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
