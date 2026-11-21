//! 操作（[`Msg`]）と外への作用（[`Effect`]）: 操作は [`App::update`] が当てて状態を変え、計算の要求・見た目を
//! 画面に当てることは作用として返す（当てるのは [`App::apply`]）。キー・メニュー・パレット・パネルは操作を出す
//! だけで、フレームの決まった所（入力の後・パネルを描いた後）でまとめて当てる。画面なしでも操作を当てて、状態と
//! 作用を確かめられる。パネルの中で直接変える状態はまだ残っており、その計算の要求も作用として溜め、フレームの
//! 終わりに当てる（[`App::flush_fx`]）

use super::*;

/// パレット・メニュー・キー・パネルから出す操作
#[derive(Clone, Debug)]
pub(super) enum Msg {
    New,
    /// 元に戻す（true）・やり直す、段の数
    Undo(bool, usize),
    Open,
    /// ファイルを開く（最近のファイル・見本・窓に落としたファイル）
    OpenFile(PathBuf),
    Reload,
    RunAll,
    WriteZmx,
    /// 保存（true は名前を付けて保存）
    Save(bool),
    Add(Kind),
    Focus(u64),
    /// その種類のパネルを表に出す（無ければ足す）
    FocusKind(Kind),
    /// ドックのパネルを順に表に出す（true は逆順）
    CyclePanes(bool),
    ToWindow(u64),
    Preset(Preset),
    Layout(usize),
    SaveLayout,
    Theme(Mode),
    Accent(usize),
    Lang(Lang),
    Folders,
    Help,
    /// 使い方の窓を開く・閉じる
    ToggleHelp,
    /// コマンドパレットを開く・閉じる
    TogglePalette,
    /// 多重構成の次・前（true）の構成
    CycleConfig(bool),
    /// 最適化を今の設定で回す・公差を回す・像面を近軸の焦点へ（パレットの実行系）
    RunOpt,
    RunTol,
    FocusImage,
    /// パネルの操作: 最適化・公差・差し替え探索・ズーム（軌跡を解く）・組合せの図・カタログ
    Opt(crate::optim_panel::Action),
    Tol(crate::tol_panel::Action),
    Swap(crate::swap_panel::Action),
    Zoom(Value),
    Combo(combo::Act),
    Catalog(catalog::Action),
    /// レンズデータの表の操作
    LensData(lens_data::Act),
    /// 解析のパネルを今の設定で計算する
    Analyze(u64),
    /// 見るパラメータを足す・外す、値を変える（システムのパネル）
    AddParam(String),
    RemoveParam(usize),
    SetParam(String, f64),
    /// 欄の値の書き換え（組合せの図・要素の設定・状態の表・系の設定・カタログから足す。[`super::edit`]）。
    /// 元に戻す段は区切らない（入力の途中・ドラッグの途中も 1 段のまま。区切りは欄を離れたとき）
    Edit(super::edit::Edit),
    /// 操作の記録を始める・やめて保存する（[`super::record`]）
    Record,
    /// 記録した操作のファイルを選んで流し直す
    Replay,
    /// 流し直す操作（1 始まりの番号、操作）。当たらなかったら残りは当てない（[`super::record`]）
    Replayed(usize, Box<Msg>),
}

/// 計算の順番待ち: 本体（serve の系）と下書き（レイアウト図・カタログ）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pool {
    Main,
    Preview,
}

/// 1 つの要求の呼び出し
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Calls {
    /// 1 回（[`Worker::send`]）
    One(Call),
    /// 順に（[`Worker::send_batch`]）
    Each(Vec<Call>),
    /// 面の表・要約の読み直しを付ける（[`Worker::send_refreshing`]。None は読み直しだけ）
    Refreshing(Option<Call>),
}

impl Calls {
    /// 順番待ちに入る要求か（呼び出しが空なら何も出さない）
    fn sends(&self, _tag: &Tag) -> bool {
        match self {
            Calls::One(_) | Calls::Refreshing(_) => true,
            Calls::Each(v) => !v.is_empty(),
        }
    }
}

/// 状態の外への作用
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Effect {
    /// 計算の要求
    Calc { pool: Pool, tag: Tag, calls: Calls },
    /// 待っている解析を捨て、回している解析をやめる（[`Worker::drop_queued`]）
    DropQueued,
    /// 色を画面に当てる
    Look,
    /// 言語を画面（字形）に当てる
    Lang,
    /// 画面を描き直す（パネルを描いた後に当てた書き換えを、先に描いたパネルにも出す）
    Repaint,
    /// ファイルの窓を出す（答えは同じ id で受ける）
    Dialog {
        id: String,
        kind: crate::files::DialogKind,
        dialog: crate::files::FileDialog,
    },
}

/// まだ当てていない作用（[`Core`] の操作が足し、[`App::apply`] が当てる）
#[derive(Default)]
pub(super) struct Fx(Vec<Effect>);

impl Fx {
    pub(super) fn push(&mut self, e: Effect) {
        self.0.push(e);
    }

    pub(super) fn take(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.0)
    }

    /// 本体の順番待ちに入れる要求の数: (すべて、解析でない要求、長い要求)
    fn queued(&self) -> (usize, usize, usize) {
        let mut n = (0, 0, 0);
        for e in &self.0 {
            if let Effect::Calc {
                pool: Pool::Main,
                tag,
                calls,
            } = e
            {
                if !calls.sends(tag) {
                    continue;
                }
                n.0 += 1;
                if tag.long() {
                    n.2 += 1;
                } else if !tag.skippable() {
                    n.1 += 1;
                }
            }
        }
        n
    }

    /// 下書きの順番待ちに入れる要求がある
    fn preview_queued(&self) -> bool {
        self.0.iter().any(
            |e| matches!(e, Effect::Calc { pool: Pool::Preview, tag, calls } if calls.sends(tag)),
        )
    }
}

impl Core {
    /// 本体に型を付けていない要求を出す（method は印から、params は serve の JSON）
    pub(super) fn calc(&mut self, tag: Tag, params: Value) {
        // method の無い印（読み直しを束ねた印）は出さない
        if tag.method().is_none() {
            return;
        }
        let c = tag.call(params);
        self.call(tag, c);
    }

    /// 本体に型の要求を出す（JSON を通さない）
    pub(super) fn call(&mut self, tag: Tag, c: Call) {
        self.fx.push(Effect::Calc {
            pool: Pool::Main,
            tag,
            calls: Calls::One(c),
        });
    }

    /// 本体に型の呼び出しを 1 つの要求として出す
    pub(super) fn call_each(&mut self, tag: Tag, calls: Vec<Call>) {
        self.fx.push(Effect::Calc {
            pool: Pool::Main,
            tag,
            calls: Calls::Each(calls),
        });
    }

    /// 本体に、面の表・要約の読み直しを付けた要求を出す（印が [`Tag::Refresh`] なら読み直しだけ）
    pub(super) fn calc_refreshing(&mut self, tag: Tag, params: Value) {
        let c = tag.method().map(|_| tag.call(params));
        self.call_refreshing(tag, c);
    }

    /// 型の要求に、面の表・要約の読み直しを付けて出す
    pub(super) fn call_refreshing(&mut self, tag: Tag, c: Option<Call>) {
        self.fx.push(Effect::Calc {
            pool: Pool::Main,
            tag,
            calls: Calls::Refreshing(c),
        });
    }

    /// 下書きの順番待ちに要求を出す
    pub(super) fn draft(&mut self, tag: Tag, params: Value) {
        let c = tag.call(params);
        self.draft_call(tag, c);
    }

    /// 下書きの worker に型の要求を出す
    pub(super) fn draft_call(&mut self, tag: Tag, c: Call) {
        self.fx.push(Effect::Calc {
            pool: Pool::Preview,
            tag,
            calls: Calls::One(c),
        });
    }

    /// ファイルを開く窓を出す（答えは同じ id の [`crate::files::open`] が返す）
    pub(super) fn dialog(&mut self, id: &str, d: crate::files::FileDialog) {
        self.fx.push(Effect::Dialog {
            id: id.to_string(),
            kind: crate::files::DialogKind::Open,
            dialog: d.or_directory(self.loc.dir()),
        });
    }

    /// 待っている解析を捨て、回している解析をやめる
    pub(super) fn drop_queued(&mut self) {
        self.fx.push(Effect::DropQueued);
    }

    /// 何か待っている（まだ出していない要求も）
    pub(super) fn busy(&self) -> bool {
        self.worker.busy() || self.fx.queued().0 > 0
    }

    /// 待っている要求の数
    pub(super) fn pending(&self) -> usize {
        self.worker.pending() + self.fx.queued().0
    }

    /// 長い要求（最適化など）のほかに待っている
    pub(super) fn busy_short(&self) -> bool {
        let q = self.fx.queued();
        self.worker.busy_short() || q.0 > q.2
    }

    /// 解析でない要求（系の読み込み・レイアウトなど）を待っている
    pub(super) fn busy_core(&self) -> bool {
        self.worker.busy_core() || self.fx.queued().1 > 0
    }

    /// 下書きの順番待ちに何かある
    pub(super) fn preview_busy(&self) -> bool {
        self.preview.busy() || self.fx.preview_queued()
    }

    /// 計算の作用を順番待ちに渡す（画面に当てる作用は返す）
    pub(super) fn send_calcs(&mut self, fx: Vec<Effect>) -> Vec<Effect> {
        let mut rest = Vec::new();
        for e in fx {
            match e {
                Effect::Calc { pool, tag, calls } => {
                    let w = match pool {
                        Pool::Main => &mut self.worker,
                        Pool::Preview => &mut self.preview,
                    };
                    match calls {
                        Calls::One(c) => w.send(tag, c),
                        Calls::Each(v) => w.send_batch(tag, v),
                        Calls::Refreshing(c) => w.send_refreshing(tag, c),
                    }
                }
                Effect::DropQueued => self.worker.drop_queued(),
                e => rest.push(e),
            }
        }
        rest
    }
}

impl App {
    /// 操作を当てる: 状態を変え、外への作用を返す（当てるのは [`App::apply`]）
    pub(super) fn update(&mut self, msg: Msg) -> Vec<Effect> {
        // 操作は元に戻す段の区切り: 前の入力・ドラッグと、この操作の変更をそれぞれ 1 段に。欄の値の書き換えは
        // 区切らない（欄を離れたときに [`ComboHist::track`] が区切る）
        let step = !matches!(&msg, Msg::Edit(_))
            && !matches!(&msg, Msg::Replayed(_, m) if matches!(**m, Msg::Edit(_)));
        if step {
            self.c.combo_hist.commit(&self.c.doc.combo);
            // この操作の段の名前の頭（最適化の反映・差し替えなど）
            self.c.combo_hist.kind = super::history::hist_names::msg_kind(&msg);
        }
        self.record_before();
        let rec = self
            .c
            .rec
            .as_ref()
            .and_then(|_| super::record::Rec::from_msg(&msg));
        self.update_state(msg);
        if step {
            // 面の上書きの印も、この操作の段に含める（後の画面で付けると別の段になる）
            self.c.tidy_combo();
            self.c.combo_hist.commit(&self.c.doc.combo);
        }
        self.record(rec);
        self.c.fx.take()
    }

    /// 作用を当てる
    pub(super) fn apply(&mut self, ctx: &egui::Context, fx: Vec<Effect>) {
        for e in self.c.send_calcs(fx) {
            match e {
                Effect::Look => theme::apply(ctx, &self.c.s.look),
                Effect::Lang => crate::fonts::follow_lang(ctx),
                Effect::Repaint => ctx.request_repaint(),
                Effect::Dialog { id, kind, dialog } => crate::files::start(&id, kind, dialog),
                Effect::Calc { .. } | Effect::DropQueued => {}
            }
        }
    }

    /// 操作を当て、作用もすぐ当てる
    pub(super) fn dispatch(&mut self, ctx: &egui::Context, msg: Msg) {
        let fx = self.update(msg);
        self.apply(ctx, fx);
    }

    /// パネルが出した操作を当てる（操作が出した操作も続けて）
    pub(super) fn dispatch_queued(&mut self, ctx: &egui::Context) {
        for _ in 0..8 {
            let ms = std::mem::take(&mut self.c.msgs);
            if ms.is_empty() {
                return;
            }
            for m in ms {
                self.dispatch(ctx, m);
            }
        }
    }

    /// パネルなどがまだ直接足した作用を当てる（フレームの終わり）
    pub(super) fn flush_fx(&mut self, ctx: &egui::Context) {
        let fx = self.c.fx.take();
        self.apply(ctx, fx);
    }

    fn update_state(&mut self, msg: Msg) {
        match msg {
            Msg::New => self.c.new_system(),
            Msg::Undo(back, n) => self.c.undo(back, n),
            Msg::Open => self.c.pick_and_open(),
            Msg::OpenFile(f) => {
                self.c.prepare_sample(&f.display().to_string());
                self.c.open_doc(&f);
            }
            Msg::Reload => self.c.reload(),
            Msg::RunAll => self.run_all(false),
            Msg::WriteZmx => self.c.write_zmx(),
            Msg::Save(as_new) => self.c.save(as_new),
            // 1 枚だけのパネルは、あれば表に出す
            Msg::Add(k) if k.single() => self.update_state(Msg::FocusKind(k)),
            Msg::Add(k) => {
                let id = self.add_pane(k);
                self.focus(id);
            }
            Msg::Focus(id) => self.focus(id),
            Msg::FocusKind(k) => {
                let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
                let id = match self.pane_of(same) {
                    Some(id) => id,
                    None => self.add_pane(k),
                };
                self.focus(id);
            }
            Msg::CyclePanes(back) => self.cycle(back),
            Msg::ToWindow(id) => self.detach(id),
            Msg::Preset(p) => self.set_layout(p.build()),
            Msg::Layout(i) => {
                if let Some((_, l)) = self.c.s.layouts.get(i) {
                    let l = l.clone();
                    self.set_layout(l);
                }
            }
            Msg::SaveLayout => self.c.save_layout_open = true,
            Msg::Theme(m) => {
                self.c.s.look.mode = m;
                self.c.fx.push(Effect::Look);
            }
            Msg::Accent(i) => {
                self.c.s.look.accent = theme::ACCENTS[i % theme::ACCENTS.len()];
                self.c.fx.push(Effect::Look);
            }
            Msg::Lang(l) => {
                self.c.s.lang = l;
                i18n::set(l);
                self.apply_dock_translations();
                self.c.fx.push(Effect::Lang);
            }
            Msg::Folders => self.c.folders_open = true,
            Msg::Help => self.c.help_open = true,
            Msg::ToggleHelp => self.c.help_open = !self.c.help_open,
            Msg::TogglePalette => {
                if self.palette.open {
                    self.palette.open = false;
                } else {
                    self.palette.show();
                }
            }
            Msg::CycleConfig(back) => self.c.cycle_config(back),
            Msg::RunOpt => {
                self.update_state(Msg::FocusKind(Kind::Optimize));
                self.c.start_optimize();
            }
            Msg::RunTol => {
                self.update_state(Msg::FocusKind(Kind::Tolerance));
                self.c.start_tolerance();
            }
            Msg::FocusImage => self.c.request_focus(),
            Msg::Opt(a) => self.c.on_opt(a),
            Msg::Tol(a) => self.c.on_tol(a),
            Msg::Swap(a) => self.c.on_swap(a),
            Msg::Zoom(p) => self.c.on_zoom(p),
            Msg::Combo(a) => self.c.on_combo(a),
            Msg::Catalog(a) => self.c.on_catalog(a),
            Msg::LensData(a) => self.c.on_ld(a),
            Msg::Analyze(id) => {
                if let Some((_, cfg)) = self.analyses().into_iter().find(|x| x.0 == id) {
                    self.c.run(id, &cfg);
                }
            }
            Msg::AddParam(p) => self.c.call(Tag::Get, Call::Get(GetReq::One(p))),
            Msg::RemoveParam(i) => {
                if i < self.c.ses.params.len() {
                    let p = self.c.ses.params.remove(i);
                    self.c.sys.param_vals.remove(&p);
                }
            }
            Msg::SetParam(p, x) => {
                self.c.sys.param_vals.insert(p.clone(), x);
                self.c.pending_set.insert(p, x);
            }
            Msg::Edit(e) => match e.apply(&mut self.c.doc.combo) {
                Ok(()) => {
                    // 変えた値を多重構成のピックアップの先にも
                    // 組合せを変えたので、面の表からの当て直しもまたできる（組み直しが失敗したままにしない）
                    self.c.p.states.picks_rebuilt = false;
                    self.c.apply_mc_picks();
                    self.c.fx.push(Effect::Repaint)
                }
                Err(why) => self
                    .c
                    .fail(format!("{}: {why}", tr("The change could not be applied"))),
            },
            Msg::Record => self.c.toggle_record(),
            Msg::Replay => self.c.pick_replay(),
            Msg::Replayed(i, m) => {
                if !self.c.replay_ok(i, &m) {
                    return;
                }
                let fails = self.c.fails;
                // 記録した書き換えは記録を始めた組合せから順に当てるので、必ず何かを変える（変えなければ当たっていない）
                let before = matches!(*m, Msg::Edit(_)).then(|| self.c.doc.combo.clone());
                self.update_state(*m);
                if self.c.fails == fails && before.is_some_and(|b| b == self.c.doc.combo) {
                    self.c
                        .fail(tr("The recorded change did not change the document"));
                }
                self.c.replayed(i, fails);
            }
        }
    }
}

impl Core {
    /// 最適化を今の設定で回す（回していれば何もしない）
    pub(super) fn start_optimize(&mut self) {
        if self.p.opt.running {
            return;
        }
        match self.p.opt.params() {
            Ok(p) => {
                self.p.opt.error = None;
                self.p.opt.running = true;
                self.p.opt.result = None;
                self.p.opt.gen = self.sys_gen;
                self.calc(Tag::Optimize, p);
            }
            Err(e) => self.p.opt.error = Some(e),
        }
    }

    /// 公差を今の設定で回す（回していれば何もしない）
    pub(super) fn start_tolerance(&mut self) {
        if self.p.tol.running {
            return;
        }
        match self.p.tol.spec() {
            Ok(p) => {
                self.p.tol.error = None;
                self.p.tol.running = true;
                self.p.tol.result = None;
                self.calc(Tag::Tolerance, p);
            }
            Err(e) => self.p.tol.error = Some(e),
        }
    }
}
