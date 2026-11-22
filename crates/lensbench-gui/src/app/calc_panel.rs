//! Web 版: 解析のパネルを出した別の窓が自分で計算する（[`crate::web_tabs::Calc`]）。元のタブは系の写し（[`PopupDoc`]）を
//! 送り、別の窓はそれを読んで自分の計算の Web Worker で解析を回す。元のタブが裏に回ってブラウザに止められても、
//! 別の窓の解析は動く。
//!
//! 写しは、元のタブが読んだ文書（ファイルの中身と load の引数）と、読んだ後の変更（set・undo・redo の引数）。
//! 別の窓は同じに読んで同じ変更を当てるので、文書の読み方が要る解析（2 つの状態・構成・経路を重ねる interference
//! など）も元のタブと同じに動く。読んだ文書が分からない（読む前の系へ戻した）ときと、別の窓が写しを読めなかった
//! ときは、serve の save で書き出した平たい系ファイルを送る（膜のファイルは付かない）。
//!
//! 同じ文書を送り続けている窓には、前に送った後に増えた変更だけを送る（ドラッグで値を変え続けても、別の窓は
//! 読み直さずに増えた分だけ当てる）。別の窓は届いた写しが前のものと続かなければ、全部を送り直してもらう。

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
    /// 別の窓ごとに、送った系
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    sent: HashMap<u64, Sent>,
    /// 読んだ文書の通し番号（読むたびに増やす）
    seq: u64,
    /// 別の窓: 当てた写し（文書の通し番号と当てた変更の数。平たい系ファイルなら None）
    at: Option<(u64, usize)>,
    /// 別の窓: 今の写しを読み始めたときの計算の世代。これ以前に出した要求（前の写し）の誤りは数えない
    since: u64,
    /// 写した文書を読めなかった別の窓（平たい系ファイルで送る）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    flat_only: std::collections::HashSet<u64>,
    /// 別の窓: 系を変える要求が通らなかった（写しを読めなかったと元のタブへ知らせる）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    failed: bool,
    /// 文書のパネルの別の窓に写す組合せ・履歴・見え方（[`super::copy_panel`]）
    pub(super) copies: super::copy_panel::Copies,
}

/// 別の窓に送った系
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sent {
    /// 系の世代（[`Core::sys_gen`]）
    gen: u64,
    /// 文書の通し番号と送った変更の数（平たい系ファイルなら None）
    doc: Option<(u64, usize)>,
}

/// 別の窓が届いた写しをどう当てるか
#[derive(Debug, PartialEq)]
enum Accept {
    /// 読み直して全部の変更を当てる
    Reload,
    /// 読み直さず、届いた変更だけを当てる
    Append,
    /// 前の写しと続かない（全部を送り直してもらう）
    Resend,
}

/// 読んだ文書と、その後の変更
struct Base {
    /// 文書の通し番号
    seq: u64,
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

/// 写しの要求（serde の形の method と params）を型の要求に戻す
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn wire_call(method: &str, params: &Value) -> Option<Call> {
    serde_json::from_value(json!({ "method": method, "params": params })).ok()
}

impl PopupDoc {
    /// 平たい系ファイルの書き出しの返事（[`Tag::PopupDoc`]）。読めなければ持たない（系が変わったら頼み直す）
    pub(super) fn take(&mut self, d: Done) {
        let Some(gen) = self.asked else {
            return;
        };
        let path = match &d.replies[0].call {
            Call::Save { path } => Some(path),
            _ => None,
        };
        let (_, _, r) = d.first();
        let b = r
            .ok()
            .and(path)
            .and_then(|p| lensbench_core::vfs::read(p).ok());
        if let Some(b) = b {
            self.flat = Some((gen, std::sync::Arc::new(b)));
        }
    }

    /// 系を変える要求の返事（[`Core::take`]、誤りでないもの）: 読んだ文書とその後の変更を覚える
    pub(super) fn track(&mut self, d: &Done) {
        let c = &d.replies[0].call;
        // 写しは要求の serde の形の params（別の窓は [`wire_call`] で型の要求に戻す）
        let params = || {
            serde_json::to_value(c)
                .map(|v| v["params"].clone())
                .unwrap_or_default()
        };
        match c {
            Call::Load(l) => {
                // 読んだときの中身（組合せの作り直しは同じパスに書き直すので、読んだときに持つ）
                let bytes = lensbench_core::vfs::read(&l.file).ok();
                self.seq += 1;
                let seq = self.seq;
                self.base = bytes.map(|b| Base {
                    seq,
                    load: params(),
                    bytes: std::sync::Arc::new(b),
                    ops: Vec::new(),
                });
            }
            Call::Set { .. } => {
                if let Some(b) = &mut self.base {
                    b.ops.push(json!({ "set": params() }));
                }
            }
            _ => {}
        }
    }

    /// 別の窓: 系を変える要求が通らなかった（gen はその要求を出したときの計算の世代）。今の写しを読み始める前に
    /// 出した要求（前の写しの途中の呼び出し）の誤りは数えない
    pub(super) fn failed(&mut self, gen: u64) {
        if gen > self.since {
            self.failed = true;
        }
    }

    /// 元のタブ: 窓 id に送る写し（読み方と変更の JSON、ファイルの中身を付けるか）。前に同じ文書を送っていれば、
    /// その後に増えた変更だけ（`from` は前に送った変更の数）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn doc_for(&self, id: u64) -> Option<(Value, bool)> {
        let b = self.base.as_ref()?;
        Some(match self.sent.get(&id).and_then(|s| s.doc) {
            Some((seq, n)) if seq == b.seq && n <= b.ops.len() => (
                json!({ "seq": b.seq, "from": n, "ops": &b.ops[n..] }),
                false,
            ),
            _ => (json!({ "load": b.load, "ops": b.ops, "seq": b.seq }), true),
        })
    }

    /// 別の窓: 届いた写し how をどう当てるか決め、当てた写しを覚える
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn accept(&mut self, how: &Value) -> Accept {
        let seq = how["seq"].as_u64();
        let n = how["ops"].as_array().map_or(0, Vec::len);
        let Some(from) = how["from"].as_u64() else {
            self.at = seq.map(|s| (s, n));
            return Accept::Reload;
        };
        match self.at {
            Some((s, k)) if Some(s) == seq && k as u64 == from => {
                self.at = Some((s, k + n));
                Accept::Append
            }
            _ => {
                self.at = None;
                Accept::Resend
            }
        }
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
            .filter(|id| pd.sent.get(id).map(|s| s.gen) != Some(gen))
            .collect();
        let (flat, doc): (Vec<u64>, Vec<u64>) = todo
            .into_iter()
            .partition(|id| pd.base.is_none() || pd.flat_only.contains(id));
        for id in doc {
            let (Some((how, whole)), Some(b)) = (pd.doc_for(id), pd.base.as_ref()) else {
                continue;
            };
            t.send_doc(id, &how, if whole { &b.bytes[..] } else { &[] });
            let doc = Some((b.seq, b.ops.len()));
            pd.sent.insert(id, Sent { gen, doc });
        }
        if flat.is_empty() {
            return;
        }
        let fresh = pd.flat.clone().filter(|(g, _)| *g == gen);
        let Some((_, bytes)) = fresh else {
            if self.popup_doc.asked != Some(gen) && !self.busy_core() {
                self.popup_doc.asked = Some(gen);
                let path = flat_path().display().to_string();
                self.call(Tag::PopupDoc, Call::Save { path: path.into() });
            }
            return;
        };
        let load = serde_json::to_value(Call::Load(Box::new(self.load_req(&flat_path()))))
            .map(|v| v["params"].clone())
            .unwrap_or_default();
        let how = json!({ "load": load, "ops": [], "flat": true });
        for id in flat {
            t.send_doc(id, &how, &bytes);
            self.popup_doc.sent.insert(id, Sent { gen, doc: None });
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
    /// 要らない。段は積まない（元に戻すのは元のタブ）。前の写しと続かない増えた分だけの写しなら false
    /// （全部を送り直してもらう）
    fn open_popup_doc(&mut self, how: &Value, bytes: Vec<u8>) -> bool {
        match self.popup_doc.accept(how) {
            Accept::Resend => return false,
            Accept::Append => {
                self.apply_popup_ops(how);
                return true;
            }
            Accept::Reload => {}
        }
        let Some(Call::Load(l)) = wire_call("load", &how["load"]) else {
            return true;
        };
        let file = l.file.clone();
        // 同じ中身なら書かない（ブラウザに残すファイルを書き直さない）
        if lensbench_core::vfs::read(&file).ok().as_deref() != Some(&bytes[..]) {
            if let Err(e) = lensbench_core::vfs::write(&file, bytes) {
                self.fail(format!("{}: {e}", file.display()));
                return true;
            }
        }
        self.popup_doc.failed = false;
        self.popup_doc.since = self.worker.gen();
        self.drop_queued();
        self.call(Tag::Load, Call::Load(l));
        self.apply_popup_ops(how);
        true
    }

    /// 別の窓: 写しの変更を当てる
    fn apply_popup_ops(&mut self, how: &Value) {
        for op in how["ops"].as_array().into_iter().flatten() {
            if let Some(c) = op.get("set").and_then(|p| wire_call("set", p)) {
                self.call_refreshing(Tag::Set, Some(c));
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl App {
    /// 別の窓（解析のパネル）として始める。パネルと系は元のタブから届く
    pub(super) fn start_calc(&mut self, ctx: &egui::Context, id: u64, sid: String) {
        self.calc = Some(crate::web_tabs::Calc::new(ctx, id, sid));
        self.c.popup_doc.copies.follower = true;
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
        let mut resend = false;
        let mut recopy = false;
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
                    if how["from"].is_null() {
                        flat = Some(how["flat"].as_bool() == Some(true));
                    }
                    resend |= !self.c.open_popup_doc(&how, b);
                }
                CalcEv::Copy(v) => {
                    let c = &mut self.c;
                    if v["combo"].is_object()
                        && !c.popup_doc.copies.recv.combo(&mut c.doc.combo, &v["combo"])
                    {
                        recopy = true;
                    }
                    if let Ok(w) =
                        serde_json::from_value::<super::copy_panel::ViewCopy>(v["view"].clone())
                    {
                        w.apply(c);
                        c.popup_doc.copies.recv.view = Some(w);
                    }
                    if let Ok(h) =
                        serde_json::from_value::<super::copy_panel::HistCopy>(v["hist"].clone())
                    {
                        c.popup_doc.copies.recv.hist = Some((h, web_time::Instant::now()));
                    }
                }
            }
        }
        if let Some(f) = flat {
            self.calc_flat = f;
        }
        if let Some(k) = &self.calc {
            if resend {
                k.ask_resend();
            }
            if recopy {
                k.ask_recopy();
            }
        }
        self.logic(ctx);
        // 写した文書を読めなかった（元のタブにしか無いファイルを指すなど）: 平たい系ファイルで送り直してもらう
        if std::mem::take(&mut self.c.popup_doc.failed) && !self.calc_flat {
            if let Some(k) = &self.calc {
                k.ask_flat();
            }
        }
        let kind = self
            .dock
            .iter_all_tabs()
            .next()
            .map(|(_, p)| p.kind.clone());
        // 解析のパネルは系の写しを、文書のパネルは組合せの写し（履歴のパネルは段の一覧）を受けるまで待つ
        let waiting = match &kind {
            None => true,
            Some(Kind::History) => self.c.popup_doc.copies.recv.hist.is_none(),
            Some(k) if super::copy_panel::needs_combo(k) => !self.c.popup_doc.copies.recv.ready(),
            Some(_) => self.c.sys.summary.is_none(),
        };
        let closed = match &self.calc {
            Some(k) => k.closed,
            None => return,
        };
        // キーの操作（元に戻す・保存など）も元のタブへ送る（窓に落としたファイルは元のタブに無いので開かない）
        let keys: Vec<Msg> = super::frame::input_msgs(ctx)
            .into_iter()
            .filter(|m| !matches!(m, Msg::OpenFile(_)))
            .collect();
        self.c.msgs.extend(keys);
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
                            "Showing a copy without the original file, so states, configurations, beam paths and coating files are not available",
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
        // この窓で当てるのは文書を変えない操作だけ。文書を変える操作は元のタブへ送り、当てた結果が写しで戻る
        use super::copy_panel::{Fwd, ViewCopy};
        let mut fwd: Vec<Fwd> = self.c.msgs.iter().filter_map(Fwd::from_msg).collect();
        // カタログから足した玉の像面合わせの印・見え方（選んだ要素・今のセル・自動の作り直し）も元のタブへ
        if std::mem::take(&mut self.c.doc.refocus) {
            fwd.push(Fwd::Refocus);
        }
        let view = ViewCopy::of(&self.c);
        // 元のタブの見え方を受けてから（受ける前の窓の既定で元のタブの選択を消さない）
        if self
            .c
            .popup_doc
            .copies
            .recv
            .view
            .as_ref()
            .is_some_and(|v| *v != view)
        {
            self.c.popup_doc.copies.recv.view = Some(view.clone());
            fwd.push(Fwd::View { view });
        }
        let ops: Vec<Value> = fwd
            .into_iter()
            .filter_map(|f| serde_json::to_value(f).ok())
            .collect();
        let focus = ctx.memory(|m| m.focused());
        let busy = ctx.input(|i| i.pointer.any_down()) || focus.is_some();
        if let Some(k) = self.calc.as_mut() {
            k.send_ops(ops, busy, focus.map(|f| f.value()));
        }
        self.c.msgs.retain(super::copy_panel::local);
        self.dispatch_queued(ctx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_calls_come_back_typed() {
        let set = Call::Set {
            values: vec![("3.thic".into(), 2.5)],
        };
        let p = serde_json::to_value(&set).unwrap()["params"].clone();
        assert_eq!(wire_call("set", &p), Some(set));
        let load = Call::Load(Box::new(lensbench_engine::LoadReq {
            file: "a.json".into(),
            ..Default::default()
        }));
        let p = serde_json::to_value(&load).unwrap()["params"].clone();
        assert_eq!(wire_call("load", &p), Some(load));
    }

    fn base(seq: u64, ops: usize) -> Base {
        Base {
            seq,
            load: json!({ "file": "a.json" }),
            bytes: std::sync::Arc::new(b"x".to_vec()),
            ops: (0..ops).map(|i| json!({ "set": { "value": i } })).collect(),
        }
    }

    /// 元のタブは、同じ文書を送った窓には増えた変更だけ、違う文書・初めての窓には全部を送る
    #[test]
    fn sends_only_new_ops_of_the_same_doc() {
        let mut pd = PopupDoc {
            base: Some(base(3, 5)),
            ..Default::default()
        };
        let (how, whole) = pd.doc_for(1).unwrap();
        assert!(whole);
        assert_eq!(how["ops"].as_array().unwrap().len(), 5);
        assert!(how["from"].is_null());
        pd.sent.insert(
            1,
            Sent {
                gen: 7,
                doc: Some((3, 2)),
            },
        );
        let (how, whole) = pd.doc_for(1).unwrap();
        assert!(!whole);
        assert_eq!(
            (how["seq"].as_u64(), how["from"].as_u64()),
            (Some(3), Some(2))
        );
        assert_eq!(
            how["ops"],
            json!([{ "set": { "value": 2 } }, { "set": { "value": 3 } }, { "set": { "value": 4 } }])
        );
        // 読み直した文書（通し番号が違う）・平たい系ファイルを送った窓には全部
        pd.sent.insert(
            1,
            Sent {
                gen: 7,
                doc: Some((2, 2)),
            },
        );
        assert!(pd.doc_for(1).unwrap().1);
        pd.sent.insert(1, Sent { gen: 7, doc: None });
        assert!(pd.doc_for(1).unwrap().1);
        pd.base = None;
        assert!(pd.doc_for(1).is_none());
    }

    /// 別の窓は、前の写しに続く増えた分だけを当て、続かなければ送り直してもらう
    #[test]
    fn popup_accepts_only_continuing_ops() {
        let mut pd = PopupDoc::default();
        // 増えた分だけが先に届いた（前の写しが無い）
        let add = json!({ "seq": 3, "from": 0, "ops": [{}] });
        assert_eq!(pd.accept(&add), Accept::Resend);
        let full = json!({ "seq": 3, "load": {}, "ops": [{}, {}] });
        assert_eq!(pd.accept(&full), Accept::Reload);
        assert_eq!(pd.at, Some((3, 2)));
        assert_eq!(
            pd.accept(&json!({ "seq": 3, "from": 2, "ops": [{}] })),
            Accept::Append
        );
        assert_eq!(pd.at, Some((3, 3)));
        // 抜けがある・別の文書
        assert_eq!(
            pd.accept(&json!({ "seq": 3, "from": 5, "ops": [{}] })),
            Accept::Resend
        );
        assert_eq!(pd.at, None);
        pd.accept(&full);
        assert_eq!(
            pd.accept(&json!({ "seq": 4, "from": 2, "ops": [] })),
            Accept::Resend
        );
        // 平たい系ファイル（通し番号なし）の後に増えた分は続かない
        pd.accept(&json!({ "load": {}, "ops": [], "flat": true }));
        assert_eq!(pd.at, None);
        assert_eq!(
            pd.accept(&json!({ "seq": 3, "from": 0, "ops": [] })),
            Accept::Resend
        );
    }

    /// 今の写しを読み始める前に出した要求（前の写し）の誤りでは、平たい系ファイルに落ちない
    #[test]
    fn errors_of_an_old_copy_are_not_counted() {
        let mut pd = PopupDoc {
            since: 10,
            ..Default::default()
        };
        pd.failed(9);
        pd.failed(10);
        assert!(!pd.failed);
        pd.failed(11);
        assert!(pd.failed);
    }
}
