//! Web 版: 解析のパネルを出した別の窓が自分で計算する（[`crate::web_tabs::Calc`]）。元のタブは系の写し（[`PopupDoc`]）を
//! 送り、別の窓はそれを読んで自分の計算の Web Worker で解析を回す。元のタブが裏に回ってブラウザに止められても、
//! 別の窓の解析は動く。
//!
//! 写しは、元のタブが読んだ文書（ファイルの中身と load の引数）と、読んだ後の変更（set・undo・redo の引数）。
//! 別の窓は同じに読んで同じ変更を当てるので、文書の読み方が要る解析（2 つの状態・構成・経路を重ねる interference
//! など）も元のタブと同じに動く。読んだ文書が分からない（読む前の系へ戻した）ときと、別の窓が写しを読めなかった
//! ときは、serve の save で書き出した平たい系ファイルを送る

use super::*;

/// 元のタブ: 別の窓に写す系
#[derive(Default)]
pub(super) struct PopupDoc {
    /// 読んだ文書と、その後の変更（None は文書が分からない: 平たい系ファイルで送る）
    base: Option<Base>,
    /// 平たい系ファイルの書き出しを頼んだときの系の世代（[`Core::sys_gen`]）
    asked: Option<u64>,
    /// 書き出した平たい系ファイルの世代と中身
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    flat: Option<(u64, std::sync::Arc<Vec<u8>>)>,
    /// 別の窓ごとに、送った系の世代
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    sent: HashMap<u64, u64>,
    /// 写した文書を読めなかった別の窓（平たい系ファイルで送る）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    flat_only: std::collections::HashSet<u64>,
    /// 別の窓: 系を変える要求が通らなかった（写しを読めなかったと元のタブへ知らせる）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    failed: bool,
}

/// 読んだ文書と、その後の変更
struct Base {
    /// load の引数（file はファイルのパス）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    load: Value,
    /// 読んだときのファイルの中身
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    bytes: std::sync::Arc<Vec<u8>>,
    /// 読んだ後の変更（`{"set": 引数}`・`{"history": 引数}`）
    ops: Vec<Value>,
}

/// 平たい系ファイルを置くパス（元のタブ・別の窓とも置き場の中）
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn flat_path() -> PathBuf {
    lensbench_core::vfs::temp_dir().join("popup-doc.json")
}

impl PopupDoc {
    /// 平たい系ファイルの書き出しの返事（[`Tag::PopupDoc`]）。読めなければ持たない（系が変わったら頼み直す）
    pub(super) fn take(&mut self, d: Done) {
        let Some(gen) = self.asked else {
            return;
        };
        let (_, params, r) = d.first();
        let b = r
            .ok()
            .and_then(|_| params["path"].as_str())
            .and_then(|p| lensbench_core::vfs::read(p).ok());
        if let Some(b) = b {
            self.flat = Some((gen, std::sync::Arc::new(b)));
        }
    }

    /// 系を変える要求の返事（[`Core::take`]、誤りでないもの）: 読んだ文書とその後の変更を覚える
    pub(super) fn track(&mut self, d: &Done) {
        let (_, params, r) = d.first();
        match d.tag {
            Tag::Load => {
                // 読んだときの中身（組合せの作り直しは同じパスに書き直すので、読んだときに持つ）
                let bytes = params["file"]
                    .as_str()
                    .and_then(|f| lensbench_core::vfs::read(f).ok());
                self.base = bytes.map(|b| Base {
                    load: params.clone(),
                    bytes: std::sync::Arc::new(b),
                    ops: Vec::new(),
                });
            }
            // 読む前の系へ戻した（開く・読み直すを戻した）: その文書は持っていない
            Tag::History { .. } if r.is_ok_and(|v| v.get("loaded").is_some()) => self.base = None,
            Tag::History { .. } => {
                if let Some(b) = &mut self.base {
                    b.ops.push(json!({ "history": params }));
                }
            }
            Tag::Set => {
                if let Some(b) = &mut self.base {
                    b.ops.push(json!({ "set": params }));
                }
            }
            _ => {}
        }
    }

    /// 別の窓: 系を変える要求が通らなかった
    pub(super) fn failed(&mut self) {
        self.failed = true;
    }
}

impl App {
    /// パネル id は自分で計算する別の窓に出した解析のパネルか（元のタブはそのパネルの解析を回さない）
    pub(super) fn calc_window(&self, id: u64) -> bool {
        #[cfg(target_arch = "wasm32")]
        return self.tabs.as_ref().is_some_and(|t| t.is_calc(id));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = id;
            false
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Core {
    /// 元のタブ: 解析のパネルの別の窓 ids に今の系を送る（まだその世代を受けていない窓に）。文書が分からないか、
    /// 文書を読めなかった窓には、計算が空いてから書き出した平たい系ファイルを送る
    pub(super) fn send_popup_doc(&mut self, t: &mut crate::web_tabs::Tabs, ids: &[u64]) {
        let pd = &mut self.popup_doc;
        pd.sent.retain(|id, _| ids.contains(id));
        pd.flat_only.retain(|id| ids.contains(id));
        if ids.is_empty() || self.loc.file.is_none() {
            return;
        }
        let gen = self.sys_gen;
        let todo: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|id| pd.sent.get(id) != Some(&gen))
            .collect();
        let (flat, doc): (Vec<u64>, Vec<u64>) = todo
            .into_iter()
            .partition(|id| pd.base.is_none() || pd.flat_only.contains(id));
        if let Some(b) = &pd.base {
            let how = json!({ "load": b.load, "ops": b.ops });
            for id in doc {
                t.send_doc(id, &how, &b.bytes);
                pd.sent.insert(id, gen);
            }
        }
        if flat.is_empty() {
            return;
        }
        let fresh = pd.flat.clone().filter(|(g, _)| *g == gen);
        let Some((_, bytes)) = fresh else {
            if self.popup_doc.asked != Some(gen) && !self.busy_core() {
                self.popup_doc.asked = Some(gen);
                let path = flat_path().display().to_string();
                self.calc(Tag::PopupDoc, json!({ "path": path }));
            }
            return;
        };
        let how = json!({ "load": self.load_params(&flat_path()), "ops": [], "flat": true });
        for id in flat {
            t.send_doc(id, &how, &bytes);
            self.popup_doc.sent.insert(id, gen);
        }
    }

    /// 元のタブ: 別の窓 id が始まった（受けた系を持っていない）。次に系を送り直す
    pub(super) fn popup_restarted(&mut self, id: u64) {
        self.popup_doc.sent.remove(&id);
    }

    /// 元のタブ: 別の窓 id が写した文書を読めなかった。平たい系ファイルで送り直す
    pub(super) fn popup_wants_flat(&mut self, id: u64) {
        self.popup_doc.sent.remove(&id);
        self.popup_doc.flat_only.insert(id);
    }

    /// 別の窓: 元のタブの系の写しを読む（how は読み方と変更、bytes は読むファイルの中身）。前の系の待っている解析は
    /// 要らない。段は積まない（元に戻すのは元のタブ）
    fn open_popup_doc(&mut self, how: &Value, bytes: Vec<u8>) {
        let mut load = how["load"].clone();
        let Some(file) = load["file"].as_str().map(PathBuf::from) else {
            return;
        };
        // 同じ中身なら書かない（ブラウザに残すファイルを書き直さない）
        if lensbench_core::vfs::read(&file).ok().as_deref() != Some(&bytes[..]) {
            if let Err(e) = lensbench_core::vfs::write(&file, bytes) {
                return self.fail(format!("{}: {e}", file.display()));
            }
        }
        if let Some(m) = load.as_object_mut() {
            m.remove("history");
        }
        self.popup_doc.failed = false;
        self.drop_queued();
        self.calc(Tag::Load, load);
        for op in how["ops"].as_array().into_iter().flatten() {
            if let Some(p) = op.get("set") {
                self.calc_refreshing(Tag::Set, p.clone());
            } else if let Some(p) = op.get("history") {
                let back = p["back"].as_bool() == Some(true);
                self.calc_refreshing(Tag::History { back }, p.clone());
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl App {
    /// 別の窓（解析のパネル）として始める。パネルと系は元のタブから届く
    pub(super) fn start_calc(&mut self, ctx: &egui::Context, id: u64, sid: String) {
        self.calc = Some(crate::web_tabs::Calc::new(ctx, id, sid));
        self.dock = egui_dock::DockState::new(Vec::new());
        self.windows.clear();
        self.apply_dock_translations();
    }

    /// 別の窓（解析のパネル）の 1 画面: 元のタブから届いたものを入れ、計算の結果を受け、パネルを描く
    pub(super) fn calc_frame(&mut self, ctx: &egui::Context) {
        use crate::web_tabs::CalcEv;
        let Some(k) = self.calc.as_mut() else {
            return;
        };
        let mut flat = None;
        for e in k.events(ctx) {
            match e {
                CalcEv::Pane(v) => {
                    if let Ok(p) = serde_json::from_value::<Pane>(v) {
                        self.dock = egui_dock::DockState::new(vec![p]);
                        self.apply_dock_translations();
                        self.c.rerun = Some(false);
                    }
                }
                CalcEv::Doc(how, b) => {
                    flat = Some(how["flat"].as_bool() == Some(true));
                    self.c.open_popup_doc(&how, b);
                }
            }
        }
        if let Some(f) = flat {
            self.calc_flat = f;
        }
        self.logic(ctx);
        // 写した文書を読めなかった（元のタブにしか無いファイルを指すなど）: 平たい系ファイルで送り直してもらう
        if std::mem::take(&mut self.c.popup_doc.failed) && !self.calc_flat {
            if let Some(k) = &self.calc {
                k.ask_flat();
            }
        }
        // 解析以外の頼み（パレットなど）はこの窓では受けない
        self.c.msgs.clear();
        let (closed, waiting) = match &self.calc {
            Some(k) => (k.closed, self.c.sys.summary.is_none()),
            None => return,
        };
        let title = self
            .dock
            .iter_all_tabs()
            .next()
            .map(|(_, p)| p.title())
            .unwrap_or_default();
        egui::TopBottomPanel::top("calc-bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&title).strong());
                if ui.small_button(tr("Back to the main tab")).clicked() {
                    crate::web_tabs::close_this();
                }
                if closed {
                    ui.label(
                        RichText::new(tr(
                            "The main tab was closed, so this panel no longer follows the system",
                        ))
                        .small()
                        .color(theme::warn(ui.visuals())),
                    );
                } else if self.calc_flat {
                    ui.label(
                        RichText::new(tr(
                            "Showing a copy without the original file, so states, configurations and beam paths are not available",
                        ))
                        .small()
                        .color(theme::warn(ui.visuals())),
                    );
                } else if self.c.status.error {
                    ui.label(
                        RichText::new(&self.c.status.text)
                            .small()
                            .color(theme::warn(ui.visuals())),
                    );
                }
            });
        });
        let App { c, dock, .. } = self;
        egui::CentralPanel::default().show(ctx, |ui| {
            let pane = dock.iter_all_tabs_mut().next().map(|(_, p)| p);
            match pane {
                Some(p) if !waiting => c.pane_ui(ui, p),
                _ => {
                    ui.centered_and_justified(|ui| ui.label(tr("Waiting for the main tab…")));
                }
            }
        });
        if c.ask.is_some() {
            c.ask_ui(ctx);
        }
        let pane = self
            .dock
            .iter_all_tabs()
            .next()
            .and_then(|(_, p)| serde_json::to_value(p).ok());
        if let (Some(k), Some(v)) = (self.calc.as_mut(), pane) {
            k.set_title(&title);
            k.send_pane(v);
        }
    }
}
