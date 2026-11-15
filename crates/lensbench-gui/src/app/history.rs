//! 元に戻す・やり直す（組合せの段・serve の段・新規の段）

use super::*;

/// 組合せの元に戻す・やり直す段（組合せの系の変更は GUI の組合せのデータなので、ここで積む）。段ごとに変更の名前
#[derive(Default)]
pub(super) struct ComboHist {
    undo: Vec<(Combo, String)>,
    redo: Vec<(Combo, String)>,
    /// 最後に段に積んだ（落ち着いた）組合せ
    last: Option<Combo>,
    /// 前の画面の組合せと、最後に変わった時刻（入力・ドラッグの途中は積まない）
    seen: Option<Combo>,
    changed_at: Option<web_time::Instant>,
}

/// 元に戻せる段の数
pub(super) const UNDO_DEPTH: usize = 100;

/// 新規の前の状態（新規を元に戻す。serve には前の系が残っているので、GUI の状態だけ戻す）
pub(super) struct NewSnap {
    combo: Combo,
    combo_hist: ComboHist,
    file: Option<PathBuf>,
    summary: Option<Value>,
    built: Option<Value>,
    built_combo: Option<Combo>,
    combo_path: Option<PathBuf>,
    doc_path: Option<PathBuf>,
    doc_from: Option<PathBuf>,
    ld_dirty: bool,
    saved_combo: Option<Combo>,
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
    pub(super) fn track(&mut self, cur: &Combo, busy: bool) {
        if self.seen.as_ref() != Some(cur) {
            self.seen = Some(cur.clone());
            self.changed_at = Some(web_time::Instant::now());
        }
        let Some(last) = &self.last else {
            self.last = Some(cur.clone());
            return;
        };
        if last == cur {
            return;
        }
        let quiet = self
            .changed_at
            .is_some_and(|t| t.elapsed().as_secs_f64() > 0.8);
        if !busy || quiet {
            self.commit(cur);
        }
    }

    /// 前の落ち着いた組合せを積む
    pub(super) fn commit(&mut self, cur: &Combo) {
        if let Some(last) = self.last.replace(cur.clone()) {
            if &last != cur {
                let what = combo_change(&last, cur);
                self.undo.push((last, what));
                if self.undo.len() > UNDO_DEPTH {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
        }
    }

    pub(super) fn can(&self, cur: &Combo, back: bool) -> bool {
        if back {
            !self.undo.is_empty() || self.last.as_ref().is_some_and(|l| l != cur)
        } else {
            !self.redo.is_empty()
        }
    }

    /// 段の名前（元に戻す段は新しい順、やり直す段は次の順）
    pub(super) fn list(&self, back: bool) -> Vec<String> {
        let v = if back { &self.undo } else { &self.redo };
        v.iter().rev().map(|x| x.1.clone()).collect()
    }

    /// n 段戻す（back）・やり直す。戻した後の組合せ
    pub(super) fn step(&mut self, cur: &Combo, back: bool, n: usize) -> Option<Combo> {
        self.commit(cur);
        let mut c = cur.clone();
        let mut moved = false;
        for _ in 0..n.max(1) {
            let (from, to) = if back {
                (&mut self.undo, &mut self.redo)
            } else {
                (&mut self.redo, &mut self.undo)
            };
            let Some((x, what)) = from.pop() else {
                break;
            };
            to.push((std::mem::replace(&mut c, x), what));
            moved = true;
        }
        if !moved {
            return None;
        }
        self.last = Some(c.clone());
        self.seen = Some(c.clone());
        Some(c)
    }
}

impl Core {
    /// 新規: 組合せと開いている系を空にする（前の状態は新規を元に戻す段に。確かめずに空にし、元に戻せる）
    pub(super) fn new_system(&mut self) {
        // すでに空なら何もしない（新規の前の状態を空で上書きしない）
        if self.sys.summary.is_none() && self.doc.combo.picks.is_empty() {
            return;
        }
        self.new_undo = Some(NewSnap {
            combo: std::mem::take(&mut self.doc.combo),
            combo_hist: {
                // 新規の前のやり直す段は捨てる（新規の後にやり直すと順がおかしくなる）
                let mut h = std::mem::take(&mut self.combo_hist);
                h.redo.clear();
                h
            },
            file: self.loc.file.take(),
            summary: self.sys.summary.take(),
            built: self.loc.built.take(),
            built_combo: self.loc.built_combo.take(),
            combo_path: self.loc.combo_path.take(),
            doc_path: self.loc.doc_path.take(),
            doc_from: self.loc.doc_from.take(),
            ld_dirty: self.loc.ld_dirty,
            saved_combo: self.loc.saved_combo.take(),
            hist: std::mem::take(&mut self.sys.hist),
            hist_list: std::mem::take(&mut self.sys.hist_list),
        });
        self.new_redo = None;
        self.p.combo_view = combo::View::default();
        self.sys.summary = None;
        self.sys.info = None;
        self.rev += 1;
        self.loc.built = None;
        self.loc.built_combo = None;
        self.loc.combo_path = None;
        self.layout_data = None;
        self.overlay = Overlay::default();
        self.load_stale = false;
        self.p.ld.reset();
        self.sys.ld_rows.clear();
        self.sys.ld_units.clear();
        self.sys.ld_blocks.clear();
        self.loc.ld_dirty = false;
        self.p.mc = Default::default();
        self.p.oss = Default::default();
        self.sys.param_vals.clear();
        self.pending_set.clear();
        for r in self.results.values_mut() {
            r.value = None;
            r.prev = None;
            r.error = None;
        }
        self.info(tr("New (Ctrl+Z to undo)"));
    }

    /// 新規を元に戻す（back）・やり直す
    pub(super) fn undo_new(&mut self, back: bool) {
        if !back {
            self.new_system();
            self.info(tr("Redone"));
            return;
        }
        let Some(n) = self.new_undo.take() else {
            return;
        };
        self.doc.combo = n.combo;
        self.combo_hist = n.combo_hist;
        self.combo_len_prev = self.doc.combo.picks.len();
        self.loc.built = n.built;
        self.loc.built_combo = n.built_combo;
        self.loc.combo_path = n.combo_path;
        self.loc.doc_path = n.doc_path;
        self.loc.doc_from = n.doc_from;
        self.loc.saved_combo = n.saved_combo;
        self.sys.hist = n.hist;
        self.sys.hist_list = n.hist_list;
        self.new_redo = Some(self.sys.hist.1);
        if let Some(sum) = n.summary {
            // serve には新規の前の系が残っている（読み直さずに表・解析を出し直す）
            self.on_loaded(sum);
            self.loc.file = n.file;
            self.loc.ld_dirty = n.ld_dirty;
        }
        self.info(tr("Undone"));
    }

    /// 元に戻す（back）・やり直すことができるか。組合せの系は組合せの段、開いた系は serve の段
    pub(super) fn can_undo(&self, back: bool) -> bool {
        if let Some(n) = self.new_undo_step(back) {
            return n;
        }
        if self.is_combo() && self.combo_hist.can(&self.doc.combo, back) {
            return true;
        }
        if self.sys.summary.is_some() {
            if back {
                self.sys.hist.0 > 0
            } else {
                self.serve_redo() > 0
            }
        } else {
            false
        }
    }

    /// 新規の直後（元に戻す）・新規を戻した直後（やり直す）なら Some（戻せるか）。それ以外は None
    pub(super) fn new_undo_step(&self, back: bool) -> Option<bool> {
        if self.sys.summary.is_none() && self.doc.combo.picks.is_empty() {
            return Some(back && self.new_undo.is_some());
        }
        (!back
            && self.new_redo.is_some()
            && self.serve_redo() == 0
            && !self.combo_hist.can(&self.doc.combo, false))
        .then_some(true)
    }

    /// serve のやり直せる段の数（新規を戻した後は、新規の前の段を除く）
    pub(super) fn serve_redo(&self) -> usize {
        self.sys.hist.1.saturating_sub(self.new_redo.unwrap_or(0))
    }

    /// serve の結果の段の数と一覧を覚える
    pub(super) fn take_history(&mut self, v: &Value) {
        if let Ok(h) = views::read::<views::HistoryView>(v) {
            (self.sys.hist, self.sys.hist_list) = ((h.undo, h.redo), (h.undo_list, h.redo_list));
        }
    }

    /// 元に戻す・やり直す段の名前（元に戻す段は新しい順、やり直す段は次の順）
    pub(super) fn undo_list(&self, back: bool) -> Vec<String> {
        match self.new_undo_step(back) {
            Some(true) => return vec![tr("New").into()],
            Some(false) => return Vec::new(),
            None => {}
        }
        // 組合せの系は組合せの段の後に、組合せを作る前の serve の段（開く など）
        let mut out = if self.is_combo() {
            self.combo_hist.list(back)
        } else {
            Vec::new()
        };
        let l = if back {
            &self.sys.hist_list.0
        } else {
            &self.sys.hist_list.1
        };
        if self.is_combo() && !out.is_empty() && !back {
            return out;
        }
        let m = if back { l.len() } else { self.serve_redo() };
        out.extend(l.iter().take(m).map(crate::hist_label::hist_label));
        if !back && self.new_redo.is_some() {
            out.push(tr("New").into());
        }
        out
    }

    /// n 段元に戻す（back）・やり直す（Ctrl+Z・Ctrl+Y、編集メニューの一覧）
    pub(super) fn undo(&mut self, back: bool, n: usize) {
        // 戻して要素が減っても「外しました」は出さない
        self.combo_census = None;
        if self.new_undo_step(back) == Some(true) {
            self.undo_new(back);
            return;
        }
        // 組合せの段を使い切ったら serve の段（組合せを作る前に開いた系など）
        let k = if self.is_combo() {
            self.combo_hist.list(back).len()
        } else {
            0
        };
        if k > 0 {
            if n > k && k > 0 && back {
                // 組合せの段を越えて選んだ: 組合せの段は捨てて、残りを serve で戻す
                self.combo_hist = Default::default();
                self.undo(back, n - k);
                return;
            }
            match self.combo_hist.step(&self.doc.combo, back, n) {
                Some(c) => {
                    self.doc.combo = c;
                    if !self.s.auto_build {
                        self.build_combo(self.doc.combo.project());
                    }
                    self.info(tr(if back { "Undone" } else { "Redone" }));
                }
                None => self.info(tr(if back {
                    "Nothing to undo"
                } else {
                    "Nothing to redo"
                })),
            }
            return;
        }
        if self.sys.summary.is_some() {
            self.flush_set();
            let mut n = n;
            if !back && self.new_redo.is_some() && n > self.serve_redo() {
                // serve の段の後に新規までやり直す
                n = self.serve_redo();
                self.redo_new_pending = true;
            }
            let m = if back { "undo" } else { "redo" };
            self.worker
                .send(Tag::History, m, json!({ "back": back, "steps": n }));
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
        let mut c = Combo::default();
        let name0 = c.name.clone();
        h.track(&c, false);
        assert!(!h.can(&c, true));
        c.name = "a".into();
        h.track(&c, true);
        c.name = "ab".into();
        h.track(&c, true);
        assert!(h.undo.is_empty() && h.can(&c, true));
        h.track(&c, false);
        assert_eq!(h.undo.len(), 1);
        c.name = "x".into();
        h.track(&c, false);
        assert_eq!(h.list(true).len(), 2);
        assert_eq!(h.list(true)[0], "Combination settings");
        let b = h.step(&c, true, 1).unwrap();
        assert_eq!(b.name, "ab");
        let b = h.step(&b, true, 1).unwrap();
        assert_eq!(b.name, name0);
        assert!(h.step(&b, true, 1).is_none());
        let f = h.step(&b, false, 1).unwrap();
        assert_eq!(f.name, "ab");
        let mut g = f.clone();
        g.name = "y".into();
        h.track(&g, false);
        assert!(!h.can(&g, false));
        assert_eq!(h.step(&g, true, 1).unwrap().name, "ab");
        // まとめて 2 段戻す
        assert_eq!(h.step(&f, true, 2).map(|c| c.name), Some(name0.clone()));
    }
}
