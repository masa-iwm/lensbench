//! 元に戻す・やり直す。1 本の履歴で、今の文書の中の段（組合せの段）の先に、文書を替えた段（新規・開く）が続く。
//! 文書を替えた段を戻すと前の文書（とその中の段）に戻る。段はすべて GUI が持つ（serve の段は使わない）

use super::*;
use lensbench_core::history::History;

#[path = "hist_names.rs"]
pub(super) mod hist_names;
#[path = "history_ui.rs"]
mod history_ui;
pub(crate) use hist_names::Step;

/// 組合せの元に戻す・やり直す段（組合せの系の変更は GUI の組合せのデータなので、ここで積む）。段ごとに変更の名前
#[derive(Default)]
pub(super) struct ComboHist {
    hist: History<Combo, Step>,
    /// 次に積む段の操作の種類（[`App::update`] が操作の前に決める。積んだら・変わらなければ消す）
    pub(super) kind: Option<&'static str>,
    /// 最後に段に積んだ（落ち着いた）組合せ
    last: Option<Combo>,
    /// last が今の組合せと同じと確かめた版（版が同じ間は比べない）
    last_at: std::cell::Cell<Option<u64>>,
    /// 前の画面で入力を受けていた欄（欄を移ったら、前の欄の入力を 1 段にする）
    focus: Option<egui::Id>,
}

/// 文書の写し（新規・開くの前の状態。戻すときに当てる）
pub(super) struct NewSnap {
    combo: Combo,
    combo_hist: ComboHist,
    file: Option<PathBuf>,
    summary: Option<Value>,
    built: Option<Value>,
    built_combo: crate::combo_doc::Snap,
    combo_path: Option<PathBuf>,
    doc_path: Option<PathBuf>,
    doc_from: Option<PathBuf>,
    saved_combo: crate::combo_doc::Snap,
}

/// 組合せの変更の名前（要素の追加・削除・どの要素か・系の設定）
pub(super) fn combo_change(a: &Combo, b: &Combo) -> String {
    use std::cmp::Ordering;
    match b.picks.len().cmp(&a.picks.len()) {
        Ordering::Greater => {
            let k = (0..a.picks.len())
                .find(|&k| a.picks[k] != b.picks[k])
                .unwrap_or(a.picks.len());
            return trf("Add element ({})", &[&b.picks[k].label()]);
        }
        Ordering::Less => {
            let k = (0..b.picks.len())
                .find(|&k| a.picks[k] != b.picks[k])
                .unwrap_or(b.picks.len());
            return trf("Remove element ({})", &[&a.picks[k].label()]);
        }
        Ordering::Equal => {}
    }
    let diff: Vec<usize> = (0..a.picks.len())
        .filter(|&k| a.picks[k] != b.picks[k])
        .collect();
    match diff.as_slice() {
        [] => tr("Combination settings").into(),
        [k] => trf("Change element {} ({})", &[&(k + 1), &b.picks[*k].label()]),
        _ => trf("Change {} elements", &[&diff.len()]),
    }
}

impl ComboHist {
    /// 画面ごとに呼ぶ。組合せが変わって操作が終わったら（ポインタを離し入力欄を離れたか、別の欄へ移ったら）前を
    /// 積む。時間では区切らない。操作（[`Msg`]）の前後でも区切る（[`App::update`]）。busy はポインタを押しているか
    /// 欄が入力を受けている、focus はその欄
    pub(super) fn track(&mut self, cur: &Combo, rev: u64, busy: bool, focus: Option<egui::Id>) {
        let moved = std::mem::replace(&mut self.focus, focus) != focus;
        let Some(last) = &self.last else {
            self.last = Some(cur.clone());
            self.last_at.set(Some(rev));
            return;
        };
        if self.last_at.get() == Some(rev) {
            return;
        }
        if last == cur {
            self.last_at.set(Some(rev));
            return;
        }
        if !busy || moved {
            self.commit(cur);
            self.last_at.set(Some(rev));
        }
    }

    /// 前の落ち着いた組合せを積む（変わっていなければ何もしない）
    pub(super) fn commit(&mut self, cur: &Combo) {
        let kind = self.kind.take();
        if self.last.as_ref() == Some(cur) {
            return;
        }
        self.last_at.set(None);
        if let Some(last) = self.last.replace(cur.clone()) {
            let what = hist_names::describe(&last, cur, kind);
            self.hist.record(last, what);
        }
    }

    pub(super) fn can(&self, cur: &Combo, rev: u64, back: bool) -> bool {
        if back {
            if !self.hist.is_empty(true) {
                return true;
            }
            if self.last_at.get() == Some(rev) {
                return false;
            }
            let differs = self.last.as_ref().is_some_and(|l| l != cur);
            if !differs && self.last.is_some() {
                self.last_at.set(Some(rev));
            }
            differs
        } else {
            !self.hist.is_empty(false)
        }
    }

    /// 段の名前（元に戻す段は新しい順、やり直す段は次の順）
    pub(super) fn list(&self, back: bool) -> Vec<Step> {
        self.hist.labels(back).cloned().collect()
    }

    /// n 段戻す（back）・やり直す。戻した後の組合せ
    pub(super) fn step(&mut self, cur: &Combo, back: bool, n: usize) -> Option<Combo> {
        self.commit(cur);
        let mut c = cur.clone();
        let mut moved = false;
        for _ in 0..n.max(1) {
            let Some((x, what)) = self.hist.pop(back) else {
                break;
            };
            self.hist.pass(back, (std::mem::replace(&mut c, x), what));
            moved = true;
        }
        if !moved {
            return None;
        }
        self.last = Some(c.clone());
        self.last_at.set(None);
        Some(c)
    }
}

/// 文書を替えた段の履歴
#[derive(Default)]
pub(super) struct DocHist {
    /// 前の文書の写し。段の名前は新規・開くなど
    hist: History<NewSnap, Step>,
    /// 文書の段を戻した・やり直した直後の組合せ（組合せを変えたら、やり直す段は使わない）
    at: Option<Combo>,
    /// 開くの前に段を積み、まだ読めていない（読めなければ前の文書に戻す）
    pub(super) opening: bool,
}

impl DocHist {
    /// 文書の中の編集をした（やり直す段は捨てる）
    pub(super) fn edited(&mut self) {
        self.hist.clear_redo();
    }
}

impl Core {
    /// 組合せの面の上書きの印を今の面の並びに合わせ、並びが変わった要素の上書きを外す（外した数）。
    /// 操作の中（[`App::update`] の段を積む前）と画面ごと（[`ComboHist::track`] の前）に呼ぶ。操作の段を積んだ後に
    /// 印だけが変わると、1 回の操作で段が 2 つ積まれ、1 段戻しても印を付け直して戻らなかった
    pub(super) fn tidy_combo(&mut self) -> usize {
        let dropped = self.doc.combo.drop_stale_edits();
        if dropped > 0 {
            self.info(trf(
                "The element's surfaces changed, so the values changed in Lens Data ({}) were dropped (Ctrl+Z to undo)",
                &[&dropped],
            ));
            // 外した数は上の文で伝えた（画面ごとの「外した」の文で重ねて出さない）
            self.combo_census = Some(self.doc.combo.census());
        }
        dropped
    }

    /// 文書が空（組合せも開いた系も無い）
    fn doc_empty(&self) -> bool {
        self.sys.summary.is_none() && self.doc.combo.picks.is_empty()
    }

    /// 今の文書を写しに移す（文書の欄は空になる）
    fn take_snap(&mut self) -> NewSnap {
        NewSnap {
            combo: std::mem::take(&mut self.doc.combo),
            combo_hist: std::mem::take(&mut self.combo_hist),
            file: self.loc.file.take(),
            summary: self.sys.summary.take(),
            built: self.loc.built.take(),
            built_combo: std::mem::take(&mut self.loc.built_combo),
            combo_path: self.loc.combo_path.take(),
            doc_path: self.loc.doc_path.take(),
            doc_from: self.loc.doc_from.take(),
            saved_combo: std::mem::take(&mut self.loc.saved_combo),
        }
    }

    /// 画面の表・図・解析を空の文書に合わせる
    fn clear_doc_views(&mut self) {
        self.p.combo_view = combo::View::default();
        self.sys.info = None;
        self.rev += 1;
        self.layout_data = None;
        self.overlay = Overlay::default();
        self.load_stale = false;
        self.p.ld.reset();
        self.sys.ld_rows.clear();
        self.sys.ld_units.clear();
        self.sys.ld_blocks.clear();
        self.sys.param_vals.clear();
        self.pending_set.clear();
        for r in self.results.values_mut() {
            r.value = None;
            r.prev = None;
            r.typed = None;
            r.prev_typed = None;
            r.error = None;
        }
    }

    /// 文書を替える前に、今の文書を「元に戻す」の段に積む（空なら積まない）。label は段の名前。積んだか
    pub(super) fn push_doc(&mut self, label: String) -> bool {
        self.docs.opening = false;
        if self.doc_empty() {
            return false;
        }
        let snap = self.take_snap();
        // 段の時刻は積んだとき（一覧を開くたびに作ると、いつも「0 秒前」になる）
        self.docs.hist.record(snap, Step::doc(&label));
        self.docs.at = None;
        true
    }

    /// 開こうとしたファイルが読めなかった: 開く前に積んだ文書に戻す（serve の系は前のまま）
    pub(super) fn revert_open(&mut self) {
        if !std::mem::take(&mut self.docs.opening) {
            return;
        }
        // 画面の表・図・解析は開く前のまま（写しを取っただけ）なので、欄を戻すだけ（読み直さず、誤りの表示も残す）
        if let Some((snap, _)) = self.docs.hist.pop(true) {
            let (sum, file) = self.apply_snap(snap);
            (self.sys.summary, self.loc.file) = (sum, file);
        }
    }

    /// 戻した組合せを組み直す（組合せの段は写しのまま）
    fn rebuild_kept(&mut self) {
        self.loc.file = self.loc.combo_path.clone();
        self.build_combo(self.doc.combo.project());
    }

    /// 新規: 組合せと開いている系を空にする（前の文書は元に戻す段に。確かめずに空にし、元に戻せる）
    pub(super) fn new_system(&mut self) {
        if self.doc_empty() {
            return;
        }
        self.push_doc(tr("New").into());
        self.clear_doc_views();
        // 新規は serve の系を変えない（新規を戻すと、その系の表・解析を読み直さずに出す）
        self.info(tr("New (Ctrl+Z to undo)"));
    }

    /// 写しの文書の欄（系の要約・ファイルを除く）を戻す
    fn apply_snap(&mut self, n: NewSnap) -> (Option<Value>, Option<PathBuf>) {
        self.doc.combo = n.combo;
        self.combo_hist = n.combo_hist;
        self.loc.built = n.built;
        self.loc.built_combo = n.built_combo;
        self.loc.combo_path = n.combo_path;
        self.loc.doc_path = n.doc_path;
        self.loc.doc_from = n.doc_from;
        self.loc.saved_combo = n.saved_combo;
        (n.summary, n.file)
    }

    /// 写しの文書に戻る。was_empty は替える前の文書が空だったか（新規を戻す: serve に前の系が残っている）
    fn restore_doc(&mut self, n: NewSnap, was_empty: bool) {
        if n.combo_path.is_some() && n.combo_path == n.file {
            // 組合せは段を変えずに組み直す
            self.apply_snap(n);
            self.rebuild_kept();
            return;
        }
        let (sum, file) = self.apply_snap(n);
        match (sum, file) {
            // 新規を戻した: 読み直さずに表・解析を出し直す
            (Some(sum), file) if was_empty => {
                self.on_loaded(sum);
                self.loc.file = file;
            }
            // 開いていた系はファイルを読み直す
            (Some(_), Some(f)) => self.open(&f),
            // 何も開いていない文書
            _ => self.clear_doc_views(),
        }
    }

    /// 文書を替えた段を戻す（back）・やり直す
    fn step_doc(&mut self, back: bool) -> bool {
        let top = if back || self.doc_redo_ok() {
            self.docs.hist.pop(back)
        } else {
            None
        };
        let Some((snap, label)) = top else {
            return false;
        };
        self.flush_set();
        let was_empty = self.doc_empty();
        // 今の文書を反対の段に（空の文書も: 新規をやり直せる）
        let cur = self.take_snap();
        self.docs.hist.pass(back, (cur, label));
        self.restore_doc(snap, was_empty);
        self.docs.at = Some(self.doc.combo.clone());
        self.info(tr(if back { "Undone" } else { "Redone" }));
        true
    }

    /// 文書を替えた段をやり直せるか（戻した後に組合せを変えていない）
    fn doc_redo_ok(&self) -> bool {
        !self.docs.hist.is_empty(false)
            && self.docs.at.as_ref().is_none_or(|c| *c == self.doc.combo)
    }

    /// 元に戻す（back）・やり直すことができるか
    pub(super) fn can_undo(&self, back: bool) -> bool {
        // Web 版の別の窓: 元のタブの段の写し
        if let Some((h, _)) = &self.popup_doc.copies.recv.hist {
            return h.can[usize::from(!back)];
        }
        self.combo_hist.can(&self.doc.combo, self.doc.rev, back)
            || if back {
                !self.docs.hist.is_empty(true)
            } else {
                self.doc_redo_ok()
            }
    }

    /// 元に戻す・やり直す段の名前（元に戻す段は新しい順、やり直す段は次の順）。文書の中の段の後に、文書を替えた段
    pub(super) fn undo_list(&self, back: bool) -> Vec<Step> {
        if let Some((h, got)) = &self.popup_doc.copies.recv.hist {
            return h.steps(back, *got);
        }
        let mut out = self.combo_hist.list(back);
        let doc = if back || self.doc_redo_ok() {
            self.docs.hist.top(back)
        } else {
            None
        };
        out.extend(doc.map(|d| d.1.clone()));
        out
    }

    /// n 段元に戻す（back）・やり直す（Ctrl+Z・Ctrl+Y、編集メニューの一覧）。文書の中の段より先を選ぶと、文書を替えた段
    pub(super) fn undo(&mut self, back: bool, n: usize) {
        // 戻して要素が減っても「外しました」は出さない
        self.combo_census = None;
        let k = self.combo_hist.list(back).len();
        if n.max(1) > k {
            if !self.step_doc(back) {
                self.info(tr(if back {
                    "Nothing to undo"
                } else {
                    "Nothing to redo"
                }));
            }
            return;
        }
        if let Some(c) = self.combo_hist.step(&self.doc.combo, back, n) {
            self.doc.combo = c;
            if !self.s.auto_build {
                self.build_combo(self.doc.combo.project());
            }
            self.info(tr(if back { "Undone" } else { "Redone" }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 組合せの段: 入力・ドラッグの途中は積まず、落ち着いたら 1 段。戻す・やり直す、戻した後に変えるとやり直す段は消える
    #[test]
    fn combo_history_steps() {
        let mut h = ComboHist::default();
        // 版は文書が画面ごとに進める（ここでは変えるたびに sync）
        let mut d = crate::combo_doc::ComboDoc::default();
        let mut at = |c: &Combo| {
            d.combo = c.clone();
            d.sync();
            d.rev
        };
        let mut c = Combo::default();
        let name0 = c.name.clone();
        let r = at(&c);
        h.track(&c, r, false, None);
        assert!(!h.can(&c, r, true));
        c.name = "a".into();
        let r = at(&c);
        h.track(&c, r, true, None);
        c.name = "ab".into();
        let r = at(&c);
        h.track(&c, r, true, None);
        assert!(h.hist.is_empty(true) && h.can(&c, r, true));
        h.track(&c, r, false, None);
        assert_eq!(h.hist.len(true), 1);
        // 同じ版のままなら比べ直さず、元に戻す段も増えない
        h.track(&c, r, false, None);
        assert_eq!(h.hist.len(true), 1);
        c.name = "x".into();
        let r = at(&c);
        h.track(&c, r, false, None);
        assert_eq!(h.list(true).len(), 2);
        assert!(
            h.list(true)[0].title.contains("name"),
            "{:?}",
            h.list(true)[0]
        );
        let b = h.step(&c, true, 1).unwrap();
        assert_eq!(b.name, "ab");
        let b = h.step(&b, true, 1).unwrap();
        assert_eq!(b.name, name0);
        assert!(h.step(&b, true, 1).is_none());
        let f = h.step(&b, false, 1).unwrap();
        assert_eq!(f.name, "ab");
        let r = at(&f);
        h.track(&f, r, false, None);
        assert!(h.can(&f, r, true));
        let mut g = f.clone();
        g.name = "y".into();
        let r = at(&g);
        h.track(&g, r, false, None);
        assert!(!h.can(&g, r, false));
        assert_eq!(h.step(&g, true, 1).unwrap().name, "ab");
        // まとめて 2 段戻す
        assert_eq!(h.step(&f, true, 2).map(|c| c.name), Some(name0.clone()));
    }

    /// 段は時間では区切らない: 欄に入力している間は 1 段のまま、別の欄へ移るか欄を離れたら区切る。
    /// 操作の前後の commit も区切り（変わっていなければ段を足さない）
    #[test]
    fn steps_split_by_focus_not_time() {
        let mut h = ComboHist::default();
        let mut d = crate::combo_doc::ComboDoc::default();
        let mut at = |c: &Combo| {
            d.combo = c.clone();
            d.sync();
            d.rev
        };
        let (fa, fb) = (Some(egui::Id::new("a")), Some(egui::Id::new("b")));
        let mut c = Combo::default();
        let r = at(&c);
        h.track(&c, r, false, None);
        // 欄に入った（まだ変えていない）
        h.track(&c, r, true, fa);
        c.name = "a".into();
        let r = at(&c);
        h.track(&c, r, true, fa);
        std::thread::sleep(std::time::Duration::from_millis(900));
        c.name = "ab".into();
        let r = at(&c);
        h.track(&c, r, true, fa);
        assert!(h.hist.is_empty(true));
        // 別の欄へ移った: 前の欄の入力が 1 段
        c.name = "abc".into();
        let r = at(&c);
        h.track(&c, r, true, fb);
        assert_eq!(h.hist.len(true), 1);
        h.track(&c, r, false, None);
        assert_eq!(h.hist.len(true), 1);
        h.commit(&c);
        assert_eq!(h.hist.len(true), 1);
        c.name = "x".into();
        h.commit(&c);
        assert_eq!(h.hist.len(true), 2);
    }

    /// 写しが今の組合せと同じかは、版が同じ間は比べ直さず、組合せが変わって版が進めば比べ直す
    #[test]
    fn gpn13_snap_follows_rev() {
        use crate::combo_doc::{ComboDoc, Snap};
        let mut d = ComboDoc::default();
        d.sync();
        let s = Snap::new(Some(d.combo.clone()));
        assert!(s.same(&d));
        let name0 = d.combo.name.clone();
        d.combo.name = "changed".into();
        d.sync();
        assert!(!s.same(&d));
        d.combo.name = name0;
        d.sync();
        assert!(s.same(&d));
        assert!(!Snap::default().same(&d));
    }
}
