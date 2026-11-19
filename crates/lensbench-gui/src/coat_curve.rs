//! 膜の定義の曲線（波長・透過・反射）の表（[`crate::widgets::sheet`] の中身）。少なくとも 2 行は残す

use std::ops::RangeInclusive;

use crate::i18n::tr;
use crate::json_view::fmt_num;
use crate::widgets::sheet::{num_in, Cell, Sheet};

/// 曲線の表（r が空なら反射の列を出さない）
pub(super) struct CurveSheet<'a> {
    pub l: &'a mut Vec<f64>,
    pub t: &'a mut Vec<f64>,
    pub r: &'a mut Vec<f64>,
}

impl Sheet for CurveSheet<'_> {
    fn rows(&self) -> usize {
        self.l.len().min(self.t.len())
    }
    fn cols(&self) -> usize {
        if self.r.is_empty() {
            2
        } else {
            3
        }
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Wavelength [nm]"), tr("Transmission"), tr("Reflection")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [100.0, 90.0, 90.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let v = match col {
            0 => self.l[row],
            1 => self.t[row],
            _ => self.r.get(row).copied().unwrap_or(0.0),
        };
        Cell::num(fmt_num(v))
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let what = self.header(col, None);
        match col {
            0 => self.l[row] = num_in(text, &what, self.l[row], 1.0, 5000.0)?,
            1 => self.t[row] = num_in(text, &what, 0.0, 0.0, 1.0)?,
            _ => {
                if let Some(r) = self.r.get_mut(row) {
                    *r = num_in(text, &what, 0.0, 0.0, 1.0)?;
                }
            }
        }
        Ok(())
    }
    /// 前の行から 50 nm 長く、透過・反射は前の行と同じ
    fn insert(&mut self, at: usize) -> bool {
        let prev = at.checked_sub(1);
        let l = prev.and_then(|i| self.l.get(i)).map_or(500.0, |l| l + 50.0);
        let t = prev.and_then(|i| self.t.get(i)).copied().unwrap_or(0.9);
        self.l.insert(at, l);
        self.t.insert(at, t);
        if !self.r.is_empty() {
            let r = prev.and_then(|i| self.r.get(i)).copied().unwrap_or(0.0);
            self.r.insert(at.min(self.r.len()), r);
        }
        true
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        // 2 行は残す
        let n = self.rows();
        if n <= 2 {
            return;
        }
        let lo = (*rows.start()).min(n - 1);
        let hi = (*rows.end()).min(n - 1);
        if lo > hi {
            return;
        }
        let hi = lo + (hi - lo + 1).min(n - 2) - 1;
        self.l.drain(lo..=hi);
        self.t.drain(lo..=hi);
        if hi < self.r.len() {
            self.r.drain(lo..=hi);
        }
    }
    fn swap(&mut self, a: usize) -> bool {
        self.l.swap(a, a + 1);
        self.t.swap(a, a + 1);
        if a + 1 < self.r.len() {
            self.r.swap(a, a + 1);
        }
        true
    }
    fn new_row(&self, _full: bool) -> Option<Cell> {
        Some(Cell::num(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 範囲・前の行から足す・2 行は残す・反射も一緒に動く
    #[test]
    fn curve_sheet() {
        let (mut l, mut t, mut r) = (vec![400.0, 500.0], vec![0.9, 0.8], vec![0.1, 0.2]);
        let mut sh = CurveSheet {
            l: &mut l,
            t: &mut t,
            r: &mut r,
        };
        assert_eq!(sh.cols(), 3);
        assert!(sh.set(0, 1, "1.5").is_err());
        assert!(sh.set(0, 0, "0").is_err());
        sh.add_row("650").unwrap();
        assert_eq!((sh.l[2], sh.t[2], sh.r[2]), (650.0, 0.8, 0.2));
        sh.swap(0);
        assert_eq!((sh.l[0], sh.r[0]), (500.0, 0.2));
        sh.remove(0..=2);
        assert_eq!(sh.rows(), 2);
        sh.remove(0..=0);
        assert_eq!(sh.rows(), 2);
        assert_eq!(r.len(), 2);
    }
}
