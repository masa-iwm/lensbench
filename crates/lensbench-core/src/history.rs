//! 元に戻す・やり直すの段。前の状態と段の名前を積む。serve の系の段（[`System`](crate::System) の写しと
//! 変更の記録）と GUI の組合せの段が同じ型を使う。深さを超えたら古い段を捨てる。
//! 段をまとめる（続けて来た同じ変更を 1 段にする）かは呼ぶ側が決めて [`History::record_or_merge`] に渡す

use std::collections::VecDeque;

/// 元に戻せる段の既定の数
pub const DEPTH: usize = 100;

/// 元に戻す段（前の状態と、その状態から次への変更の名前 L）と、やり直す段
#[derive(Debug, Clone)]
pub struct History<T, L = String> {
    undo: VecDeque<(T, L)>,
    redo: VecDeque<(T, L)>,
    depth: usize,
}

impl<T, L> Default for History<T, L> {
    fn default() -> Self {
        History::new(DEPTH)
    }
}

impl<T, L> History<T, L> {
    /// 深さ depth（1 以上）の空の履歴
    pub fn new(depth: usize) -> Self {
        History {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            depth: depth.max(1),
        }
    }

    /// 変更の前の状態 prev を積む（やり直す段は捨てる）
    pub fn record(&mut self, prev: T, label: L) {
        self.undo.push_back((prev, label));
        while self.undo.len() > self.depth {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// merge なら前の段にまとめて積まない（元に戻す段が無ければ積む）。どちらでもやり直す段は捨てる
    pub fn record_or_merge(&mut self, prev: T, label: L, merge: bool) {
        if merge && !self.undo.is_empty() {
            self.redo.clear();
        } else {
            self.record(prev, label);
        }
    }

    /// 1 段戻す（back）・やり直す: 今の状態 cur を反対の段に積み、戻した先の状態を返す。段が無ければ None
    pub fn step(&mut self, cur: T, back: bool) -> Option<T> {
        let (x, label) = self.pop(back)?;
        self.pass(back, (cur, label));
        Some(x)
    }

    /// 戻す（back）・やり直す段の一番上を取り出す（反対の段には積まない。[`Self::pass`] と組で使う）
    pub fn pop(&mut self, back: bool) -> Option<(T, L)> {
        if back {
            self.undo.pop_back()
        } else {
            self.redo.pop_back()
        }
    }

    /// [`Self::pop`] で戻した（back）・やり直した段の代わりに、反対の段へ積む（やり直す段は捨てない）
    pub fn pass(&mut self, back: bool, item: (T, L)) {
        if back {
            self.redo.push_back(item);
        } else {
            self.undo.push_back(item);
            while self.undo.len() > self.depth {
                self.undo.pop_front();
            }
        }
    }

    /// やり直す段を捨てる
    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    /// 元に戻せる（back）・やり直せる段の数
    pub fn len(&self, back: bool) -> usize {
        if back {
            self.undo.len()
        } else {
            self.redo.len()
        }
    }

    /// 元に戻せる（back）・やり直せる段が無い
    pub fn is_empty(&self, back: bool) -> bool {
        self.len(back) == 0
    }

    /// 次に戻す（back）・やり直す段
    pub fn top(&self, back: bool) -> Option<&(T, L)> {
        if back {
            self.undo.back()
        } else {
            self.redo.back()
        }
    }

    /// 段の名前（元に戻す段は新しい順、やり直す段は次にやり直す順）
    pub fn labels(&self, back: bool) -> impl Iterator<Item = &L> {
        let v = if back { &self.undo } else { &self.redo };
        v.iter().rev().map(|x| &x.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_step_and_depth() {
        let mut h: History<i32> = History::new(3);
        for i in 0..5 {
            h.record(i, format!("to {}", i + 1));
        }
        // 深さ 3: 古い段 0・1 は捨てる
        assert_eq!(h.len(true), 3);
        assert_eq!(
            h.labels(true).cloned().collect::<Vec<_>>(),
            ["to 5", "to 4", "to 3"]
        );
        // 今は 5。戻すと 4、3。やり直すと 4
        assert_eq!(h.step(5, true), Some(4));
        assert_eq!(h.step(4, true), Some(3));
        assert_eq!(
            h.labels(false).cloned().collect::<Vec<_>>(),
            ["to 4", "to 5"]
        );
        assert_eq!(h.step(3, false), Some(4));
        assert_eq!(h.len(false), 1);
        // 新しい変更でやり直す段は捨てる
        h.record(4, "x".into());
        assert!(h.is_empty(false));
        assert_eq!(h.step(9, false), None);
        // まとめる: 段は増えず、やり直す段は捨てる
        h.step(5, true);
        h.record_or_merge(4, "y".into(), true);
        assert_eq!(h.len(true), 2);
        assert!(h.is_empty(false));
    }
}
