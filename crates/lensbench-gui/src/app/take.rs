//! 計算の結果を受け取って状態に入れる（GAR-18 で app.rs から分けた）

use super::*;

impl Core {
    pub(super) fn take(&mut self, mut d: Done) {
        if d.tag == Tag::Refresh {
            let n = d.replies.len();
            if !d.skipped {
                for (m, p) in refresh_calls().into_iter().skip(n) {
                    self.worker.send(Tag::of_refresh(&m), &m, p);
                }
            }
            return d.split().into_iter().for_each(|d| self.take(d));
        }
        self.rev += 1;
        if matches!(d.tag, Tag::Load | Tag::Set | Tag::Edit | Tag::History) {
            self.sys_gen += 1;
        }
        let ms = d.ms;
        if let Some((method, e)) = d.error() {
            let text = format!(
                "{}: {}",
                method_label(method),
                super::status::plain_error(e)
            );
            match &d.tag {
                Tag::Analyze(id) | Tag::Rays(id) => {
                    self.results.entry(*id).or_default().error = Some(e.to_string());
                }
                Tag::Get => {
                    if let Some(p) = d.replies[0].params["param"].as_str() {
                        self.sys.param_vals.remove(p);
                    }
                }
                // 断られた変更は欄から消す（系の値を読み直す）
                Tag::Edit => {
                    self.p.oss = Default::default();
                    self.p.ld.insp.stale();
                }
                // 開けなかったファイルは最近のファイルから外す。ガラスが無いなら案内を出す（GUX-01）。ほかの理由で
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
                Tag::UserCatalog => self.p.cat_save.note = Some((e.to_string(), true)),
                Tag::CatalogFill => self.p.fill.failed(e),
                Tag::Optimize => {
                    self.p.opt.running = false;
                    self.p.opt.error = Some(e.to_string());
                }
                Tag::Swap => {
                    self.p.swap.running = false;
                    self.p.swap.error = Some(e.to_string());
                }
                Tag::Tolerance => {
                    self.p.tol.running = false;
                    self.p.tol.error = Some(e.to_string());
                }
                Tag::Zoom => {
                    self.p.zoom.running = false;
                    self.p.zoom.error = Some(e.to_string());
                }
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
            return;
        }
        let mut text = trf(
            "{} done ({} ms)",
            &[
                &method_label(&d.replies[0].method),
                &fmt_num((ms * 10.0).round() / 10.0),
            ],
        );
        let (params, mut v) = d.take_first();
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
                // 読んだときの注意（プロジェクトの展開・分散式・.zmx の読めなかった行など）は履歴へ、下の行には数（CLI-10）
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
            Tag::Info => self.sys.info = Some(v),
            Tag::Analyze(id) => {
                let cmd = params["cmd"].as_str().unwrap_or_default().to_string();
                let gen = self.rev;
                let r = self.results.entry(id).or_default();
                // 前の結果は同じ解析・同じ設定のとき（系を変えたときの差）だけ。設定を変えたら出さない（GUX-19）
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
                return;
            }
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
            Tag::Get => {
                if let (Some(p), Some(x)) = (params["param"].as_str(), v["value"].as_f64()) {
                    self.sys.param_vals.insert(p.to_string(), x);
                    if !self.ses.params.iter().any(|q| q == p) {
                        self.ses.params.push(p.to_string());
                    }
                }
                return;
            }
            Tag::HistoryDepth => {
                self.take_history(&v);
                return;
            }
            Tag::Set | Tag::Edit | Tag::History => {
                self.take_history(&v);
                if d.tag != Tag::History {
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
                    return;
                }
                if d.tag == Tag::History {
                    text = tr(if back { "Undone" } else { "Redone" }).into();
                }
                // 値の変更（Set）も保存していない変更
                self.loc.ld_dirty = true;
                // 値を変えたら面の表・要約と、開いている解析を計算し直す
                self.refresh();
                self.worker
                    .send(Tag::Layout, "layout", self.layout_params());
                self.rerun = Some(true);
            }
            // take の頭で分けて回すので来ない
            Tag::Write | Tag::Refresh => {}
            Tag::LayoutExport => {
                text = trf(
                    "Exported the layout to {}",
                    &[&v["written"].as_str().unwrap_or_default()],
                );
            }
            Tag::Surfaces => {
                self.p.oss.stale();
                // 要約の面の数も今の系に（読み込み時のままにしない）
                if let (Some(sum), Some(a)) = (self.sys.summary.as_mut(), v["surfaces"].as_array())
                {
                    sum["surfaces"] = json!(a.len());
                }
                let (rows, units) = match lens_data::parse(&v) {
                    Ok(x) => x,
                    Err(text) => return self.fail(text),
                };
                self.p.um.set_mechs(&v["mechanics"]);
                self.sys.ld_blocks = lens_data::tree(&units);
                self.p.ld.insp.stale();
                self.sys.ld_rows = rows;
                self.sys.ld_units = units;
                // 同梱のカタログ（屈折率だけ）から取ったガラスがあれば知らせる（GUX-01）
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
                return;
            }
            Tag::SurfData(j) => {
                self.p.ld.insp.set(j, v);
                return;
            }
            Tag::SysData => {
                // 要約の視野・波長も今の系に
                if let Some(sum) = self.sys.summary.as_mut() {
                    let d = &v["data"];
                    if let Some(f) = d["fields"].as_array() {
                        sum["fields"] = json!(f
                            .iter()
                            .map(|f| [f["x"].as_f64(), f["y"].as_f64()])
                            .collect::<Vec<_>>());
                    }
                    if d["wavelengths"].is_array() {
                        sum["wavelengths_um"] = d["wavelengths"].clone();
                    }
                    if let Some(p) = d["primary"].as_u64() {
                        sum["primary"] = json!(p + 1);
                    }
                }
                self.p.oss.set(&v);
                return;
            }
            Tag::ConfigTable => match config_panel::parse(&v) {
                Ok(t) => return self.p.mc.table = Some(t),
                Err(text) => return self.fail(text),
            },
            Tag::ConfigDiff => {
                self.p.mc.notes = v["notes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(lensbench_core::i18n::tr))
                            .collect()
                    })
                    .unwrap_or_default();
                let ops = v["ops"].clone();
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                text = trf(
                    "Imported {} rows into the multi-configuration table",
                    &[&v["rows"]],
                );
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
                return;
            }
            // 下書きの Worker の分（tick で受け取る）
            Tag::LensPreview(_) | Tag::Folders | Tag::CadMeasure | Tag::OpenDoc => return,
            Tag::Focus => {
                self.focus_from(&d);
                return;
            }
            Tag::UserCatalog => {
                let (method, _, _) = d.first();
                self.p.cat_save.take(&v);
                text = match method {
                    "catalog_add" => trf(
                        "Saved to {}: {}",
                        &[
                            &v["catalog"].as_str().unwrap_or_default(),
                            &v["added"].as_str().unwrap_or_default(),
                        ],
                    ),
                    "catalog_edit" => {
                        trf("Updated {}", &[&v["catalog"].as_str().unwrap_or_default()])
                    }
                    _ => text,
                };
                // Web 版: ブラウザに残すユーザーカタログのフォルダは、最初に保存したときにレンズのフォルダに足す
                #[cfg(target_arch = "wasm32")]
                if method == "catalog_add" {
                    let d = crate::web_store::USER_CATALOGS.to_string();
                    if v["catalog"].as_str().is_some_and(|c| c.starts_with(&d))
                        && !self.s.lens_dirs.contains(&d)
                    {
                        self.s.lens_dirs.push(d);
                    }
                }
                if method != "catalog_items" {
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
            Tag::Configs => {
                // 別の系を開いて、重ねていた構成・状態・BS の経路が無くなった: 開いたときのレイアウトは
                // 前の選択で頼んで誤りになっているので、選択を外して頼み直す（誤りは同じ要求の成功で消える）
                if self.overlay.update(&v) {
                    self.request_layout();
                }
                return;
            }
            Tag::Optimize => {
                self.p.opt.running = false;
                self.p.opt.error = None;
                self.p.opt.undo = None;
                // 結果を系に当てたら元に戻す段が増える
                self.worker.send(Tag::HistoryDepth, "history", json!({}));
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
                self.p.swap.running = false;
                self.p.swap.error = None;
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
                self.p.tol.running = false;
                self.p.tol.error = None;
                text = trf(
                    "Tolerance done ({} ms)",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.p.tol.result = Some(std::sync::Arc::new(v));
            }
            Tag::Zoom => {
                self.p.zoom.running = false;
                self.p.zoom.error = None;
                text = trf(
                    "Zoom track done ({} ms)",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                (self.p.zoom.result, self.p.zoom.error) = views::read::<views::ZoomView>(&v)
                    .map_or_else(|e| (None, Some(String::from(e))), |r| (Some(r), None));
            }
        }
        self.set_done_status(&d.tag, text);
    }

    /// 要求がうまくいったときの下の行。画面が自分で頼んだ要求（[`Tag::quiet`]）は、同じ要求が出した誤りを
    /// 消すだけで、利用者の操作の結果（元に戻した・保存したなど）を上書きしない（GUX-27）
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

/// 系を変えた後に読み直す表（[`Tag::Refresh`]。途中の誤りで止まれば残りは 1 つずつ頼み直す）
pub(super) fn refresh_calls() -> Vec<(String, Value)> {
    let calls = [
        ("surface", json!({})),
        ("config_table", json!({})),
        ("analyze", json!({ "cmd": "info" })),
    ];
    calls.into_iter().map(|(m, p)| (m.to_string(), p)).collect()
}
