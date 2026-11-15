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
            let text = format!("{}: {}", method_label(method), plain_error(e));
            match &d.tag {
                Tag::Analyze(id) | Tag::Rays(id) => {
                    self.results.entry(*id).or_default().error = Some(e.to_string());
                }
                Tag::Get => {
                    if let Some(p) = d.replies[0].params["param"].as_str() {
                        self.param_vals.remove(p);
                    }
                }
                // 断られた変更は欄から消す（系の値を読み直す）
                Tag::Edit => {
                    self.oss = Default::default();
                    self.ld.insp.stale();
                }
                // 開けなかったファイルは最近のファイルから外す。ガラスが無いなら案内を出す（GUX-01）
                Tag::Load => {
                    if let Some(f) = d.replies[0].params["file"].as_str() {
                        let f = f.to_string();
                        self.s.recent.retain(|x| x != &f);
                        if let Some(name) = missing_glass_name(e) {
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
                Tag::UserCatalog => self.cat_save.note = Some((e.to_string(), true)),
                Tag::CatalogFill => self.fill.failed(e),
                Tag::Optimize => {
                    self.opt.running = false;
                    self.opt.error = Some(e.to_string());
                }
                Tag::Swap => {
                    self.swap.running = false;
                    self.swap.error = Some(e.to_string());
                }
                Tag::Tolerance => {
                    self.tol.running = false;
                    self.tol.error = Some(e.to_string());
                }
                Tag::Zoom => {
                    self.zoom.running = false;
                    self.zoom.error = Some(e.to_string());
                }
                _ => {}
            }
            self.status_err = Some((d.tag.clone(), text.clone()));
            self.fail(text);
            return;
        }
        let mut text = trf(
            "{} 完了（{} ms）",
            &[
                &method_label(&d.replies[0].method),
                &fmt_num((ms * 10.0).round() / 10.0),
            ],
        );
        let (params, mut v) = d.take_first();
        match d.tag {
            Tag::Load => {
                // 開く・読み直すで段を積んだときは段の数と一覧が付く（無ければ段は空）
                self.hist = (0, 0);
                self.hist_list = Default::default();
                self.take_history(&v);
                self.new_undo = None;
                self.new_redo = None;
                if d.replies[0].method == "reload" {
                    text = tr("読み直した（Ctrl+Z で戻す）").into();
                }
                if let Some(m) = self.missing_glass.take() {
                    if m.model.is_some() {
                        text = trf(
                            "ガラス {} をモデルガラス（nd・νd）で代用して開いた（元のファイルは変えていない。保存は名前を付けて）",
                            &[&m.names.join("・")],
                        );
                    }
                }
                self.on_loaded(v);
            }
            Tag::Info => self.info = Some(v),
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
                self.results.entry(id).or_default().rays = rays;
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
                text = trf("ガラス {} 種", &[&self.browser.glasses.len()]);
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
                        "カタログレンズ {} 枚、うち {} 枚は読めない（{} ms）",
                        &[&self.browser.lenses.len(), &bad, &fmt_num(ms.round())],
                    )
                } else {
                    trf(
                        "カタログレンズ {} 枚（{} ms）",
                        &[&self.browser.lenses.len(), &fmt_num(ms.round())],
                    )
                };
            }
            Tag::Get => {
                if let (Some(p), Some(x)) = (params["param"].as_str(), v["value"].as_f64()) {
                    self.param_vals.insert(p.to_string(), x);
                    if !self.s.params.iter().any(|q| q == p) {
                        self.s.params.push(p.to_string());
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
                    self.new_redo = None;
                }
                if d.tag == Tag::History && std::mem::take(&mut self.redo_new_pending) {
                    self.new_system();
                    return;
                }
                let back = params["back"].as_bool() == Some(true);
                // 開く・読み直すを戻した: 戻した系を開いたときと同じく表・解析を出し直す。読んだままの系でなければ
                // 面の表は変更あり
                if let Some(l) = v.get_mut("loaded") {
                    self.on_loaded(l.take());
                    let top = &self.hist_list.0.first();
                    self.ld_dirty =
                        top.is_some_and(|t| t.get("open").is_none() && t.get("reload").is_none());
                    self.info(tr(if back {
                        "元に戻した"
                    } else {
                        "やり直した"
                    }));
                    return;
                }
                if d.tag == Tag::History {
                    text = tr(if back {
                        "元に戻した"
                    } else {
                        "やり直した"
                    })
                    .into();
                }
                // 値の変更（Set）も保存していない変更
                self.ld_dirty = true;
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
                    "レイアウトを {} に書き出した",
                    &[&v["written"].as_str().unwrap_or_default()],
                );
            }
            Tag::Surfaces => {
                self.oss.stale();
                // 要約の面の数も今の系に（読み込み時のままにしない）
                if let (Some(sum), Some(a)) = (self.summary.as_mut(), v["surfaces"].as_array()) {
                    sum["surfaces"] = json!(a.len());
                }
                let (rows, units) = match lens_data::parse(&v) {
                    Ok(x) => x,
                    Err(text) => return self.fail(text),
                };
                self.um.set_mechs(&v["mechanics"]);
                self.ld_blocks = lens_data::tree(&units);
                self.ld.insp.stale();
                self.ld_rows = rows;
                self.ld_units = units;
                // 同梱のカタログ（屈折率だけ）から取ったガラスがあれば知らせる（GUX-01）
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mats: Vec<&str> =
                        self.ld_rows.iter().map(|r| r.material.as_str()).collect();
                    let used = crate::builtin_glass::used(&mats, &agf_names(&self.s.glass_dirs));
                    if !used.is_empty() && !self.status.error {
                        self.info(trf(
                                "ガラス {} は同梱のカタログ（屈折率だけ。内部透過率・熱膨張なし）から取りました。メーカーのカタログはファイル > カタログのフォルダ… で足せます",
                                &[&used.join("・")],
                            ));
                    }
                }
                return;
            }
            Tag::SurfData(j) => {
                self.ld.insp.set(j, v);
                return;
            }
            Tag::SysData => {
                // 要約の視野・波長も今の系に
                if let Some(sum) = self.summary.as_mut() {
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
                self.oss.set(&v);
                return;
            }
            Tag::ConfigTable => match config_panel::parse(&v) {
                Ok(t) => return self.mc.table = Some(t),
                Err(text) => return self.fail(text),
            },
            Tag::ConfigDiff => {
                self.mc.notes = v["notes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let ops = v["ops"].clone();
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                text = trf("多重構成の表に {} 行を取り込んだ", &[&v["rows"]]);
            }
            Tag::SaveRange => {
                text = trf(
                    "{} に書き出し",
                    &[&params["path"].as_str().unwrap_or_default()],
                );
            }
            Tag::Save => {
                self.ld_dirty = false;
                text = trf("{} に保存", &[&params["path"].as_str().unwrap_or_default()]);
            }
            Tag::Layout => {
                self.layout_data = Some(combo::LayoutData::parse(&v));
                // 前の図の誤り（別の系で残った重ねる経路など）は、図が描けたら消す
                self.set_done_status(&d.tag, String::new());
                return;
            }
            Tag::LensPreview(_) => return,
            Tag::Focus => {
                self.focus_from(&d);
                return;
            }
            Tag::UserCatalog => {
                let (method, _, _) = d.first();
                self.cat_save.take(&v);
                text = match method {
                    "catalog_add" => trf(
                        "{} に {} を保存",
                        &[
                            &v["catalog"].as_str().unwrap_or_default(),
                            &v["added"].as_str().unwrap_or_default(),
                        ],
                    ),
                    "catalog_edit" => trf(
                        "{} を書き換え",
                        &[&v["catalog"].as_str().unwrap_or_default()],
                    ),
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
                    self.cat_save.note = Some((text.clone(), false));
                    // カタログブラウザに並ぶカタログなら一覧を読み直す
                    self.refresh_catalog();
                }
            }
            Tag::CatalogFill => {
                self.fill.done(&v);
                text = trf(
                    "{} {} を補って {} に保存",
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
                self.opt.running = false;
                self.opt.error = None;
                self.opt.undo = None;
                // 結果を系に当てたら元に戻す段が増える
                self.worker.send(Tag::HistoryDepth, "history", json!({}));
                text = trf(
                    "最適化 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.opt.result = Some(std::sync::Arc::new(v));
            }
            Tag::Swap => {
                self.swap.running = false;
                self.swap.error = None;
                self.swap.inserted = None;
                let r = views::read::<lensbench_design::swap::SwapReply>(&v);
                if r.as_ref().is_ok_and(|r| r.results.is_some()) {
                    text = trf(
                        "差し替え探索 完了（{} ms）",
                        &[&fmt_num((ms * 10.0).round() / 10.0)],
                    );
                }
                (self.swap.result, self.swap.error) = r.map_or_else(
                    |e| (None, Some(e)),
                    |r| (Some(std::sync::Arc::new(r)), None),
                );
            }
            Tag::Tolerance => {
                self.tol.running = false;
                self.tol.error = None;
                text = trf(
                    "公差 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.tol.result = Some(std::sync::Arc::new(v));
            }
            Tag::Zoom => {
                self.zoom.running = false;
                self.zoom.error = None;
                text = trf(
                    "ズーム軌跡 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.zoom.result = Some(v);
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
