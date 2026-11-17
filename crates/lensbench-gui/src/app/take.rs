//! 計算の結果を受け取って状態に入れる（app.rs から分けた）

use super::*;

impl Core {
    pub(super) fn take(&mut self, mut d: Done) {
        if d.tag == Tag::Refresh {
            let n = d.replies.len();
            if !d.skipped {
                for (m, p) in Tag::refresh_calls().into_iter().skip(n) {
                    self.worker.send(Tag::of_refresh(&m), p);
                }
            }
            return d.split().into_iter().for_each(|d| self.take(d));
        }
        // 束ねたパラメータの get: 1 つずつ Get として受ける。途中の誤り（消えたパラメータなど）で止まったら、返事の無い
        // ものは 1 つずつ頼み直す（先頭で止まって返事が 1 つでも同じ。束ねた要求は Gets の印で見分ける）
        if d.tag == Tag::Gets {
            let asked: Vec<String> = d
                .replies
                .iter()
                .filter_map(|r| r.params["param"].as_str().map(String::from))
                .collect();
            let stopped = d.error().is_some() && !d.skipped;
            let mut d = d;
            d.tag = Tag::Get;
            d.split_same().into_iter().for_each(|d| self.take(d));
            if stopped {
                for p in self.ses.params.clone() {
                    if !asked.contains(&p) {
                        self.worker.send(Tag::Get, json!({ "param": p }));
                    }
                }
            }
            return;
        }
        // 系を変える要求に付けた読み直し（send_change）: 変更の返事を先に受け、通ったら残りを読み直しとして受ける。
        // 断られたら読み直しは回っていない（途中の誤りで止まる）
        if matches!(d.tag, Tag::Set | Tag::History { .. }) && d.replies.len() > 1 {
            let (d, rest) = d.split_change();
            let ok = d.error().is_none() && !d.skipped;
            self.bundled_refresh = ok;
            self.take(d);
            self.bundled_refresh = false;
            if ok {
                self.take(rest);
            }
            return;
        }
        self.rev += 1;
        if matches!(d.tag, Tag::Load | Tag::Set | Tag::History { .. }) {
            self.sys_gen += 1;
        }
        let ms = d.ms;
        if d.error().is_some() {
            return self.take_error(&d);
        }
        let text = trf(
            "{} done ({} ms)",
            &[
                &method_label(&d.replies[0].method),
                &fmt_num((ms * 10.0).round() / 10.0),
            ],
        );
        let (params, v) = d.take_first();
        if let Some(text) = self.on_reply(&d, params, v, ms, text) {
            self.set_done_status(&d.tag, text);
        }
    }

    /// 誤りの返事: 解析の結果・欄・長い計算のパネルに誤りを入れ、下の行に出す
    fn take_error(&mut self, d: &Done) {
        let Some((method, e)) = d.error() else {
            return;
        };
        let text = format!(
            "{}: {}",
            method_label(method),
            super::status::plain_error(e)
        );
        if let Some((running, error)) = self.job(&d.tag) {
            *running = false;
            *error = Some(e.to_string());
        }
        match &d.tag {
            Tag::Analyze(id) | Tag::Rays(id) => {
                self.results.entry(*id).or_default().error = Some(e.to_string());
            }
            Tag::Get => {
                if let Some(p) = d.replies[0].params["param"].as_str() {
                    self.sys.param_vals.remove(p);
                }
            }
            // 開けなかったファイルは最近のファイルから外す。ガラスが無いなら案内を出す。ほかの理由で
            // 読めなければ開く前の文書に戻す
            Tag::Load => {
                if missing_glass_of(e).is_none() {
                    self.revert_open();
                }
                if let Some(f) = d.replies[0].params["file"].as_str() {
                    let f = f.to_string();
                    self.s.recent.retain(|x| x != &f);
                    if let Some(name) = missing_glass_of(e) {
                        let m = self.missing_glass.get_or_insert_with(Default::default);
                        let same = m
                            .model
                            .as_ref()
                            .is_some_and(|p| p.display().to_string() == f);
                        if !same {
                            *m = MissingGlass {
                                file: PathBuf::from(&f),
                                ..Default::default()
                            };
                        }
                        if !m.names.contains(&name) {
                            m.names.push(name);
                        }
                        if same && m.tries < 64 {
                            // 代用した写しにまだ無いガラスがある: それも代用して読み直す
                            self.open_model_glass();
                            return;
                        }
                    }
                }
            }
            Tag::UserCatalog(_) => self.p.cat_save.note = Some((e.to_string(), true)),
            Tag::CatalogFill => self.p.fill.failed(e),
            _ => {}
        }
        self.status_err = Some((d.tag.clone(), text.clone()));
        let hint = super::status::error_hint(e);
        self.fail(text);
        // 案内と符号は、訳した後の文に結びつける（文が変われば出さない）
        let shown = self.status.text.clone();
        self.status_hint = hint.filter(|h| !shown.contains(h.as_str())).map(|h| {
            let c = e.code.as_deref().unwrap_or_default();
            (shown.clone(), format!("{h}\n[{c}]"))
        });
    }

    /// うまくいった返事を状態に入れる。下の行に出す文を返す（None は出さない。画面が自分で頼んだ途中の要求など）
    fn on_reply(
        &mut self,
        d: &Done,
        params: Value,
        v: Value,
        ms: f64,
        mut text: String,
    ) -> Option<String> {
        match d.tag {
            Tag::Optimize | Tag::Swap | Tag::Tolerance | Tag::Zoom => {
                return Some(self.job_done(&d.tag, v, ms, text));
            }
            Tag::Glasses | Tag::Lenses | Tag::UserCatalog(_) | Tag::CatalogFill => {
                return Some(self.catalog_done(d, v, ms, text));
            }
            Tag::Load | Tag::Set | Tag::History { .. } | Tag::Surfaces => {
                return self.system_done(d, params, v, text);
            }
            Tag::Info => self.sys.info = Some(v),
            Tag::Analyze(id) => {
                let cmd = params["cmd"].as_str().unwrap_or_default().to_string();
                let gen = self.rev;
                let r = self.results.entry(id).or_default();
                // 前の結果は同じ解析・同じ設定のとき（系を変えたときの差）だけ。設定を変えたら出さない
                r.prev = if r.cmd == cmd && r.params == params {
                    r.value.take()
                } else {
                    None
                };
                r.params = params;
                r.cmd = cmd;
                r.value = Some(v);
                r.ms = ms;
                r.error = None;
                r.gen = gen;
            }
            Tag::Rays(id) => {
                let rays = d
                    .replies
                    .iter()
                    .filter_map(|r| {
                        let (_, a) = r.result.as_ref().ok()?;
                        let f = r.params["field"].as_u64()? as usize;
                        let w = r.params["wave"].as_u64()? as usize;
                        Some(plots::rays_from(f, w, a))
                    })
                    .collect();
                let r = self.results.entry(id).or_default();
                r.rays = rays;
                // 要求ごとに進む rev（パネルの結果を消した後も前の世代と重ならない）
                r.rays_gen = self.rev;
                return None;
            }
            Tag::Get => {
                if let (Some(p), Some(x)) = (params["param"].as_str(), v["value"].as_f64()) {
                    self.sys.param_vals.insert(p.to_string(), x);
                    if !self.ses.params.iter().any(|q| q == p) {
                        self.ses.params.push(p.to_string());
                    }
                }
                return None;
            }
            Tag::HistoryDepth => {
                self.take_history(&v);
                return None;
            }
            // take の頭で分けて回すので来ない
            Tag::Write | Tag::Refresh | Tag::Gets => {}
            Tag::LayoutExport => {
                text = trf(
                    "Exported the layout to {}",
                    &[&v["written"].as_str().unwrap_or_default()],
                );
            }
            Tag::SurfData(j) => {
                self.p.ld.insp.set(j, v);
                return None;
            }
            Tag::WriteBack => {
                match self.take_write_back(&v) {
                    Ok(true) if !self.s.auto_build => self.build_combo(self.doc.combo.project()),
                    Ok(_) => {}
                    Err(e) => self.fail(e),
                }
                let notes: Vec<String> = v["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(lensbench_core::i18n::tr))
                    .collect();
                if notes.is_empty() {
                    return Some(String::new());
                }
                text = notes.join("\n");
            }
            Tag::SaveRange => {
                text = trf(
                    "Exported to {}",
                    &[&params["path"].as_str().unwrap_or_default()],
                );
            }
            Tag::Save => {
                self.loc.ld_dirty = false;
                text = trf(
                    "Saved to {}",
                    &[&params["path"].as_str().unwrap_or_default()],
                );
            }
            Tag::Layout => {
                self.layout_data = Some(combo::LayoutData::parse(&v));
                // 前の図の誤り（別の系で残った重ねる経路など）は、図が描けたら消す
                self.set_done_status(&d.tag, String::new());
                return None;
            }
            // 下書きの Worker の分（tick で受け取る）
            Tag::Preview | Tag::LensPreview(_) | Tag::Folders | Tag::CadMeasure | Tag::OpenDoc => {
                return None
            }
            Tag::Focus => {
                self.focus_from(d);
                return None;
            }
            Tag::Configs => {
                // 別の系を開いて、重ねていた構成・状態・BS の経路が無くなった: 開いたときのレイアウトは
                // 前の選択で頼んで誤りになっているので、選択を外して頼み直す（誤りは同じ要求の成功で消える）
                if self.overlay.update(&v) {
                    self.request_layout();
                }
                return None;
            }
        }
        Some(text)
    }

    /// 系を開いた・変えた・読み直した返事（面の表・要約・元に戻す段）。下の行に出す文を返す
    fn system_done(
        &mut self,
        d: &Done,
        params: Value,
        mut v: Value,
        mut text: String,
    ) -> Option<String> {
        match d.tag {
            Tag::Load => {
                self.docs.opening = false;
                // 開く・読み直すで段を積んだときは段の数と一覧が付く（無ければ段は空）
                self.sys.hist = (0, 0);
                self.sys.hist_list = Default::default();
                self.take_history(&v);
                if d.replies[0].method == "reload" {
                    text = tr("Reloaded (Ctrl+Z to undo)").into();
                }
                // 読んだときの注意（プロジェクトの展開・分散式・.zmx の読めなかった行など）は履歴へ、下の行には数
                let notes: Vec<String> = v["notes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .map(lensbench_core::i18n::warning)
                            .collect()
                    })
                    .unwrap_or_default();
                if !notes.is_empty() {
                    text = trf(
                        "{} ({} notes, click for the history)",
                        &[&text, &notes.len()],
                    );
                    self.note_history(notes);
                }
                if let Some(m) = self.missing_glass.take() {
                    if m.model.is_some() {
                        text = trf(
                            "Opened with model glasses (nd, νd) in place of {} (the original file is unchanged; use Save As)",
                            &[&m.names.join(crate::i18n::tr(", "))],
                        );
                    }
                }
                self.on_loaded(v);
            }
            Tag::Set | Tag::History { .. } => {
                self.take_history(&v);
                if !matches!(d.tag, Tag::History { .. }) {
                    self.docs.edited();
                }
                let back = params["back"].as_bool() == Some(true);
                // 開く・読み直すを戻した: 戻した系を開いたときと同じく表・解析を出し直す。読んだままの系でなければ
                // 面の表は変更あり
                if let Some(l) = v.get_mut("loaded") {
                    self.on_loaded(l.take());
                    let top = &self.sys.hist_list.0.first();
                    self.loc.ld_dirty =
                        top.is_some_and(|t| t.get("open").is_none() && t.get("reload").is_none());
                    // 前の文書へ戻した: 写しの欄（組合せ・保存先・変更あり）を当てる
                    self.take_pending_doc();
                    self.info(tr(if back { "Undone" } else { "Redone" }));
                    return None;
                }
                if matches!(d.tag, Tag::History { .. }) {
                    text = tr(if back { "Undone" } else { "Redone" }).into();
                }
                // 値の変更（Set）も保存していない変更
                self.loc.ld_dirty = true;
                // 値を変えたら面の表・要約と、開いている解析を計算し直す
                self.refresh();
                self.worker.send(Tag::Layout, self.layout_params());
                self.rerun = Some(true);
            }
            Tag::Surfaces => {
                // 要約の面の数も今の系に（読み込み時のままにしない）
                if let (Some(sum), Some(a)) = (self.sys.summary.as_mut(), v["surfaces"].as_array())
                {
                    sum["surfaces"] = json!(a.len());
                }
                let (rows, units) = match lens_data::parse(&v) {
                    Ok(x) => x,
                    Err(text) => {
                        self.fail(text);
                        return None;
                    }
                };
                self.sys.ld_blocks = lens_data::tree(&units);
                self.p.ld.insp.stale();
                self.sys.ld_rows = rows;
                self.sys.ld_units = units;
                // 同梱のカタログ（屈折率だけ）から取ったガラスがあれば知らせる
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mats: Vec<&str> = self
                        .sys
                        .ld_rows
                        .iter()
                        .map(|r| r.material.as_str())
                        .collect();
                    let used = crate::builtin_glass::used(
                        &mats,
                        &crate::worker::agf_names(&self.s.glass_dirs),
                    );
                    if !used.is_empty() && !self.status.error {
                        self.info(trf(
                                "Glass {} came from the built-in catalog (index only; no internal transmittance or thermal expansion). Add the maker catalogs in File > Catalog folders…",
                                &[&used.join(crate::i18n::tr(", "))],
                            ));
                    }
                }
                return None;
            }
            _ => {}
        }
        Some(text)
    }

    /// ガラス・カタログの返事。下の行に出す文を返す
    fn catalog_done(&mut self, d: &Done, v: Value, ms: f64, mut text: String) -> String {
        match d.tag {
            Tag::Glasses => {
                self.browser.set_glasses(catalog::parse_glasses(&v));
                self.ld_glasses = self
                    .browser
                    .glasses
                    .iter()
                    .map(|g| lens_data::GlassChoice {
                        name: g.name.clone(),
                        desc: format!("{}  nd {:.5}  νd {:.2}", g.catalog, g.nd, g.vd),
                    })
                    .collect();
                text = trf("{} glasses", &[&self.browser.glasses.len()]);
            }
            Tag::Lenses => {
                // 選んでいた品は一覧の中で探し直す（set_lenses）。断面は捨てて頼み直す
                // （同じ番号の行でも別の玉かもしれない。作りかけの結果も受け取らない）
                self.browser.set_lenses(catalog::parse_lenses(&v));
                self.browser.preview = None;
                self.preview_req = None;
                let bad = self
                    .browser
                    .lenses
                    .iter()
                    .filter(|l| l.error.is_some())
                    .count();
                text = if bad > 0 {
                    trf(
                        "{} catalog lenses, {} unreadable ({} ms)",
                        &[&self.browser.lenses.len(), &bad, &fmt_num(ms.round())],
                    )
                } else {
                    trf(
                        "{} catalog lenses ({} ms)",
                        &[&self.browser.lenses.len(), &fmt_num(ms.round())],
                    )
                };
            }
            Tag::UserCatalog(op) => {
                self.p.cat_save.take(&v);
                text = match op {
                    CatOp::Add => trf(
                        "Saved to {}: {}",
                        &[
                            &v["catalog"].as_str().unwrap_or_default(),
                            &v["added"].as_str().unwrap_or_default(),
                        ],
                    ),
                    CatOp::Edit => trf("Updated {}", &[&v["catalog"].as_str().unwrap_or_default()]),
                    CatOp::Items => text,
                };
                // Web 版: ブラウザに残すユーザーカタログのフォルダは、最初に保存したときにレンズのフォルダに足す
                #[cfg(target_arch = "wasm32")]
                if op == CatOp::Add {
                    let d = crate::web_store::USER_CATALOGS.to_string();
                    if v["catalog"].as_str().is_some_and(|c| c.starts_with(&d))
                        && !self.s.lens_dirs.contains(&d)
                    {
                        self.s.lens_dirs.push(d);
                    }
                }
                if op != CatOp::Items {
                    self.p.cat_save.note = Some((text.clone(), false));
                    // カタログブラウザに並ぶカタログなら一覧を読み直す
                    self.refresh_catalog();
                }
            }
            Tag::CatalogFill => {
                self.p.fill.done(&v);
                text = trf(
                    "Completed {} {} and saved to {}",
                    &[
                        &v["vendor"].as_str().unwrap_or_default(),
                        &v["item"].as_str().unwrap_or_default(),
                        &v["catalog"].as_str().unwrap_or_default(),
                    ],
                );
                // 補足のカタログのフォルダをレンズのフォルダに足し、一覧を読み直す
                if let Some(d) = crate::fill::fill_dir() {
                    let d = d.display().to_string();
                    if !self.s.lens_dirs.contains(&d) {
                        self.s.lens_dirs.push(d);
                    }
                }
                self.refresh_catalog();
            }
            _ => {}
        }
        text
    }

    /// 長い計算のパネル（最適化・差し替え・公差・ズーム）の動いている印と誤りの欄
    fn job(&mut self, tag: &Tag) -> Option<(&mut bool, &mut Option<String>)> {
        let p = &mut self.p;
        Some(match tag {
            Tag::Optimize => (&mut p.opt.running, &mut p.opt.error),
            Tag::Swap => (&mut p.swap.running, &mut p.swap.error),
            Tag::Tolerance => (&mut p.tol.running, &mut p.tol.error),
            Tag::Zoom => (&mut p.zoom.running, &mut p.zoom.error),
            _ => return None,
        })
    }

    /// 長い計算が終わった: 動いている印と誤りを下ろして結果を入れる。下の行に出す文を返す
    fn job_done(&mut self, tag: &Tag, v: Value, ms: f64, mut text: String) -> String {
        if let Some((running, error)) = self.job(tag) {
            *running = false;
            *error = None;
        }
        match tag {
            Tag::Optimize => {
                self.p.opt.undo = None;
                // 結果を系に当てたら元に戻す段が増える
                self.worker.send(Tag::HistoryDepth, json!({}));
                text = trf(
                    "Optimization done ({} ms)",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                (self.p.opt.result, self.p.opt.error) = views::read::<views::OptimizeView>(&v)
                    .map_or_else(
                        |e| (None, Some(String::from(e))),
                        |r| (Some(std::sync::Arc::new(r)), None),
                    );
            }
            Tag::Swap => {
                self.p.swap.inserted = None;
                let r = views::read::<lensbench_design::swap::SwapReply>(&v);
                if r.as_ref().is_ok_and(|r| r.results.is_some()) {
                    text = trf(
                        "Swap search done ({} ms)",
                        &[&fmt_num((ms * 10.0).round() / 10.0)],
                    );
                }
                (self.p.swap.result, self.p.swap.error) = r.map_or_else(
                    |e| (None, Some(String::from(e))),
                    |r| (Some(std::sync::Arc::new(r)), None),
                );
            }
            Tag::Tolerance => {
                text = trf(
                    "Tolerance done ({} ms)",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                (self.p.tol.result, self.p.tol.error) =
                    match views::read::<lensbench_design::tolerance::ToleranceReport>(&v) {
                        Ok(r) => (Some(std::sync::Arc::new(r)), None),
                        Err(e) => (None, Some(format!("tolerance: {e}"))),
                    };
            }
            Tag::Zoom => {
                text = trf(
                    "Zoom track done ({} ms)",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                (self.p.zoom.result, self.p.zoom.error) = views::read::<views::ZoomView>(&v)
                    .map_or_else(|e| (None, Some(String::from(e))), |r| (Some(r), None));
            }
            _ => {}
        }
        text
    }

    /// 要求がうまくいったときの下の行。画面が自分で頼んだ要求（[`Tag::quiet`]）は、同じ要求が出した誤りを
    /// 消すだけで、利用者の操作の結果（元に戻した・保存したなど）を上書きしない
    pub(super) fn set_done_status(&mut self, tag: &Tag, text: String) {
        if !tag.quiet() {
            self.status_err = None;
            self.info(text);
            return;
        }
        if self.status_err.as_ref().is_some_and(|(t, _)| t == tag) {
            if let Some((_, e)) = self.status_err.take() {
                if self.status.error && self.status.text == e {
                    self.status = Status::default();
                }
            }
        }
    }
}
