//! 元に戻す・やり直す。1 本の履歴で、今の文書の中の段（組合せは組合せの段、系で開いた文書は serve の段）の先に、
//! 文書を替えた段（新規・開く）が続く。文書を替えた段を戻すと前の文書（とその中の段）に戻る

use super::*;
use lensbench_core::history::History;

/// 組合せの元に戻す・やり直す段（組合せの系の変更は GUI の組合せのデータなので、ここで積む）。段ごとに変更の名前
#[derive(Default)]
pub(super) struct ComboHist {
    hist: History<Combo>,
    /// 最後に段に積んだ（落ち着いた）組合せ
    last: Option<Combo>,
    /// last が今の組合せと同じと確かめた版（版が同じ間は比べない）
    last_at: std::cell::Cell<Option<u64>>,
    /// 前の画面の組合せの版と、最後に変わった時刻（入力・ドラッグの途中は積まない）
    seen_rev: u64,
    changed_at: Option<web_time::Instant>,
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
    ld_dirty: bool,
    saved_combo: crate::combo_doc::Snap,
    hist: (usize, usize),
    hist_list: (Vec<Value>, Vec<Value>),
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
    /// 画面ごとに呼ぶ。組合せが変わって落ち着いたら（ポインタを離し、入力欄を離れるか 0.8 秒たったら）前を積む
    pub(super) fn track(&mut self, cur: &Combo, rev: u64, busy: bool) {
        if self.seen_rev != rev {
            self.seen_rev = rev;
            self.changed_at = Some(web_time::Instant::now());
        }
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
        let quiet = self
            .changed_at
            .is_some_and(|t| t.elapsed().as_secs_f64() > 0.8);
        if !busy || quiet {
            self.commit(cur);
            self.last_at.set(Some(rev));
        }
    }

    /// 前の落ち着いた組合せを積む
    pub(super) fn commit(&mut self, cur: &Combo) {
        self.last_at.set(None);
        if let Some(last) = self.last.replace(cur.clone()) {
            if &last != cur {
                let what = combo_change(&last, cur);
                self.hist.record(last, what);
            }
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
    pub(super) fn list(&self, back: bool) -> Vec<String> {
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
    /// 前の文書の写しと、その文書の serve のやり直す段のうち別の文書のものの数。段の名前は新規・開くなど
    hist: History<(NewSnap, usize)>,
    /// 文書の段を戻した・やり直した直後の組合せ（組合せを変えたら、やり直す段は使わない）
    at: Option<Combo>,
    /// serve の段を戻した・進めた後に当てる写し（系で開いた文書へ戻る）
    pending: Option<NewSnap>,
    /// serve のやり直す段のうち、別の文書のもの（今の文書の段としてはやり直さない）
    skip: usize,
    /// 開くの前に段を積み、まだ読めていない（読めなければ前の文書に戻す）
    pub(super) opening: bool,
}

impl DocHist {
    /// 文書の中の編集をした（やり直す段は捨てる）
    pub(super) fn edited(&mut self) {
        self.hist.clear_redo();
        self.skip = 0;
    }
}

impl Core {
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
            ld_dirty: std::mem::take(&mut self.loc.ld_dirty),
            saved_combo: std::mem::take(&mut self.loc.saved_combo),
            hist: std::mem::take(&mut self.sys.hist),
            hist_list: std::mem::take(&mut self.sys.hist_list),
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
            r.error = None;
        }
    }

    /// 文書を替える前に、今の文書を「元に戻す」の段に積む（空なら積まない）。label は段の名前。積んだか
    pub(super) fn push_doc(&mut self, label: String) -> bool {
        self.docs.opening = false;
        if self.doc_empty() {
            return false;
        }
        let skip = self.docs.skip;
        let snap = self.take_snap();
        self.docs.hist.record((snap, skip), label);
        self.docs.at = None;
        self.docs.skip = 0;
        true
    }

    /// 開こうとしたファイルが読めなかった: 開く前に積んだ文書に戻す（serve の系は前のまま）
    pub(super) fn revert_open(&mut self) {
        if !std::mem::take(&mut self.docs.opening) {
            return;
        }
        // 画面の表・図・解析は開く前のまま（写しを取っただけ）なので、欄を戻すだけ（読み直さず、誤りの表示も残す）
        if let Some(((snap, skip), _)) = self.docs.hist.pop(true) {
            (self.sys.hist, self.sys.hist_list) = (snap.hist, snap.hist_list.clone());
            self.docs.skip = skip;
            let (sum, file, dirty) = self.apply_snap(snap);
            (self.sys.summary, self.loc.file, self.loc.ld_dirty) = (sum, file, dirty);
        }
    }

    /// 組合せを段を変えずに作り直す（serve の段はそのまま。前後の系で開いた文書へ serve の段で行ける）
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
        // 新規は serve の系を変えない（新規を戻すと、その系に戻る）
        self.info(tr("New (Ctrl+Z to undo)"));
    }

    /// 写しの文書の欄（系の要約・ファイル・面の表の変更を除く）を戻す
    fn apply_snap(&mut self, n: NewSnap) -> (Option<Value>, Option<PathBuf>, bool) {
        self.doc.combo = n.combo;
        self.combo_hist = n.combo_hist;
        self.combo_len_prev = self.doc.combo.picks.len();
        self.loc.built = n.built;
        self.loc.built_combo = n.built_combo;
        self.loc.combo_path = n.combo_path;
        self.loc.doc_path = n.doc_path;
        self.loc.doc_from = n.doc_from;
        self.loc.saved_combo = n.saved_combo;
        (n.summary, n.file, n.ld_dirty)
    }

    /// 写しの文書に戻る。(depth, redo) は替える前の serve の段の数、skip は戻る文書の別の文書の段の数、
    /// was_empty は替える前の文書が空だったか
    fn restore_doc(
        &mut self,
        n: NewSnap,
        (depth, redo): (usize, usize),
        skip: usize,
        was_empty: bool,
    ) {
        let combo_doc = n.combo_path.is_some() && n.combo_path == n.file;
        let (hist, hist_list) = (n.hist, n.hist_list.clone());
        if !combo_doc && n.summary.is_none() {
            // 何も開いていない文書
            self.apply_snap(n);
            self.clear_doc_views();
            return;
        }
        if was_empty {
            // 新規を戻す・開けなかった: serve には前の系が残っている（読み直さずに表・解析を出し直す。組合せは段を
            // 変えずに作り直す）
            (self.sys.hist, self.sys.hist_list) = (hist, hist_list);
            self.docs.skip = skip;
            if combo_doc {
                self.apply_snap(n);
                self.rebuild_kept();
                return;
            }
            let (sum, file, dirty) = self.apply_snap(n);
            if let Some(sum) = sum {
                self.on_loaded(sum);
            }
            self.loc.file = file;
            self.loc.ld_dirty = dirty;
            return;
        }
        // serve の段を戻す・進めて、その文書の系に戻す（組合せはそこで段を変えずに作り直す。serve のやり直す段を
        // 残す）。届かなければファイルを読み直す（組合せは作り直して serve の段に積む）
        let back = hist.0 < depth;
        let steps = hist.0.abs_diff(depth);
        let reach = if back { depth } else { redo };
        if steps > 0 && steps <= reach {
            // 戻すと serve のやり直す段はすべて別の文書のもの
            self.docs.skip = if back { redo + steps } else { skip };
            self.docs.pending = Some(n);
            self.send_change(
                Tag::History { back },
                json!({ "back": back, "steps": steps }),
            );
        } else if steps == 0 && combo_doc {
            self.apply_snap(n);
            self.docs.skip = skip;
            self.rebuild_kept();
        } else {
            let (_, file, _) = self.apply_snap(n);
            self.docs.skip = 0;
            if combo_doc {
                self.build_combo(self.doc.combo.project());
            } else if let Some(f) = file {
                self.open(&f);
            }
        }
    }

    /// serve の段を動かした結果が届いた: 待っていた写しの欄を戻す
    pub(super) fn take_pending_doc(&mut self) {
        if let Some(n) = self.docs.pending.take() {
            let combo_doc = n.combo_path.is_some() && n.combo_path == n.file;
            let (_, _, dirty) = self.apply_snap(n);
            // 戻した文書の組合せ（step_doc で覚えたのは写しを当てる前の空の組合せ）
            self.docs.at = Some(self.doc.combo.clone());
            if combo_doc {
                self.rebuild_kept();
            } else {
                self.loc.ld_dirty = dirty;
            }
        }
    }

    /// 文書を替えた段を戻す（back）・やり直す
    fn step_doc(&mut self, back: bool) -> bool {
        let top = if back || self.doc_redo_ok() {
            self.docs.hist.pop(back)
        } else {
            None
        };
        let Some(((snap, skip), label)) = top else {
            return false;
        };
        self.flush_set();
        // 写しを取ると serve の段の数も移るので、戻す先の計算は替える前の数で
        let hist = self.sys.hist;
        let was_empty = self.doc_empty();
        // 今の文書を反対の段に（空の文書も: 新規をやり直せる）
        let cur = (self.take_snap(), self.docs.skip);
        self.docs.hist.pass(back, (cur, label));
        self.restore_doc(snap, hist, skip, was_empty);
        self.docs.at = Some(self.doc.combo.clone());
        self.info(tr(if back { "Undone" } else { "Redone" }));
        true
    }

    /// 文書を替えた段をやり直せるか（戻した後に組合せを変えていない）
    fn doc_redo_ok(&self) -> bool {
        !self.docs.hist.is_empty(false)
            && self.docs.at.as_ref().is_none_or(|c| *c == self.doc.combo)
    }

    /// 系で開いた文書の中の serve の段の数（元に戻す段は開いた段より上、やり直す段は別の文書の段を除く）
    pub(super) fn serve_in_doc(&self, back: bool) -> usize {
        if self.is_combo() || self.sys.summary.is_none() {
            return 0;
        }
        if back {
            let base = self.docs.hist.top(true).map_or(0, |d| d.0 .0.hist.0 + 1);
            self.sys.hist.0.saturating_sub(base)
        } else {
            self.sys.hist.1.saturating_sub(self.docs.skip)
        }
    }

    /// 元に戻す（back）・やり直すことができるか
    pub(super) fn can_undo(&self, back: bool) -> bool {
        (self.is_combo() && self.combo_hist.can(&self.doc.combo, self.doc.rev, back))
            || self.serve_in_doc(back) > 0
            || if back {
                !self.docs.hist.is_empty(true)
            } else {
                self.doc_redo_ok()
            }
    }

    /// serve の結果の段の数と一覧を覚える
    pub(super) fn take_history(&mut self, v: &Value) {
        if let Ok(h) = views::read::<views::HistoryView>(v) {
            (self.sys.hist, self.sys.hist_list) = ((h.undo, h.redo), (h.undo_list, h.redo_list));
        }
    }

    /// 今の文書の中の段の名前
    fn in_doc_list(&self, back: bool) -> Vec<String> {
        if self.is_combo() {
            return self.combo_hist.list(back);
        }
        let l = if back {
            &self.sys.hist_list.0
        } else {
            &self.sys.hist_list.1
        };
        l.iter()
            .take(self.serve_in_doc(back))
            .map(crate::hist_label::hist_label)
            .collect()
    }

    /// 元に戻す・やり直す段の名前（元に戻す段は新しい順、やり直す段は次の順）。文書の中の段の後に、文書を替えた段
    pub(super) fn undo_list(&self, back: bool) -> Vec<String> {
        let mut out = self.in_doc_list(back);
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
        let k = self.in_doc_list(back).len();
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
        if self.is_combo() {
            if let Some(c) = self.combo_hist.step(&self.doc.combo, back, n) {
                self.doc.combo = c;
                if !self.s.auto_build {
                    self.build_combo(self.doc.combo.project());
                }
                self.info(tr(if back { "Undone" } else { "Redone" }));
            }
            return;
        }
        self.flush_set();
        self.send_change(Tag::History { back }, json!({ "back": back, "steps": n }));
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
        h.track(&c, r, false);
        assert!(!h.can(&c, r, true));
        c.name = "a".into();
        let r = at(&c);
        h.track(&c, r, true);
        c.name = "ab".into();
        let r = at(&c);
        h.track(&c, r, true);
        assert!(h.hist.is_empty(true) && h.can(&c, r, true));
        h.track(&c, r, false);
        assert_eq!(h.hist.len(true), 1);
        // 同じ版のままなら比べ直さず、元に戻す段も増えない
        h.track(&c, r, false);
        assert_eq!(h.hist.len(true), 1);
        c.name = "x".into();
        let r = at(&c);
        h.track(&c, r, false);
        assert_eq!(h.list(true).len(), 2);
        assert_eq!(h.list(true)[0], "Combination settings");
        let b = h.step(&c, true, 1).unwrap();
        assert_eq!(b.name, "ab");
        let b = h.step(&b, true, 1).unwrap();
        assert_eq!(b.name, name0);
        assert!(h.step(&b, true, 1).is_none());
        let f = h.step(&b, false, 1).unwrap();
        assert_eq!(f.name, "ab");
        let r = at(&f);
        h.track(&f, r, false);
        assert!(h.can(&f, r, true));
        let mut g = f.clone();
        g.name = "y".into();
        let r = at(&g);
        h.track(&g, r, false);
        assert!(!h.can(&g, r, false));
        assert_eq!(h.step(&g, true, 1).unwrap().name, "ab");
        // まとめて 2 段戻す
        assert_eq!(h.step(&f, true, 2).map(|c| c.name), Some(name0.clone()));
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
