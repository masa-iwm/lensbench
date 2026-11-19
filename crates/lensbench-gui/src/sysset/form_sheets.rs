//! 系の設定の視野・波長の表（[`crate::widgets::sheet`] の中身）。行の挿入・複製・移動・貼り付け・最後の空の行で足すのは
//! 表の部品のとおり。少なくとも 1 行は残す

use std::ops::RangeInclusive;

use super::{FieldPt, SysForm, WavePt};
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::widgets::sheet::{keep_range, num_in, Cell, Sheet};

/// 行の範囲を消す（全部の行なら最初の行だけ残す）
fn drain_keep<T>(v: &mut Vec<T>, rows: RangeInclusive<usize>) {
    if let Some((lo, hi)) = keep_range(v.len(), rows) {
        v.drain(lo..=hi);
    }
}

const VIG: [&str; 5] = ["VDX", "VDY", "VCX", "VCY", "VAN"];

/// 視野の表（x・y・ウェイト、vig ならケラレ係数）
pub(super) struct FieldSheet<'a> {
    pub fields: &'a mut Vec<FieldPt>,
    pub vig: bool,
    /// x・y の単位（「°」「 mm」、無ければ空）
    pub unit: &'static str,
}

impl FieldSheet<'_> {
    fn name(&self, col: usize) -> String {
        let u = self.unit.trim();
        match col {
            0 | 1 if !u.is_empty() => format!("{} [{u}]", ["x", "y"][col]),
            0 | 1 => ["x", "y"][col].to_string(),
            2 => tr("Weight").to_string(),
            c => VIG[(c - 3).min(4)].to_string(),
        }
    }
}

impl Sheet for FieldSheet<'_> {
    fn rows(&self) -> usize {
        self.fields.len()
    }
    fn cols(&self) -> usize {
        if self.vig {
            8
        } else {
            3
        }
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        self.name(col)
    }
    fn width(&self, col: usize) -> f32 {
        if col < 2 {
            70.0
        } else {
            56.0
        }
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let p = &self.fields[row];
        let v = match col {
            0 => p.x,
            1 => p.y,
            2 => p.w,
            c => p.vig[(c - 3).min(4)],
        };
        Cell::num(fmt_num(v))
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let what = self.name(col);
        let p = &mut self.fields[row];
        match col {
            0 => p.x = num_in(text, &what, 0.0, f64::MIN, f64::MAX)?,
            1 => p.y = num_in(text, &what, 0.0, f64::MIN, f64::MAX)?,
            2 => p.w = num_in(text, &what, 1.0, 0.0, 1e6)?,
            c => p.vig[(c - 3).min(4)] = num_in(text, &what, 0.0, f64::MIN, f64::MAX)?,
        }
        Ok(())
    }
    /// 前の行から y を 1 進めた点
    fn insert(&mut self, at: usize) -> bool {
        let p = at
            .checked_sub(1)
            .and_then(|i| self.fields.get(i))
            .map_or(FieldPt::y(0.0), |p| FieldPt::xyw(p.x, p.y + 1.0, p.w));
        self.fields.insert(at, p);
        true
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        drain_keep(self.fields, rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.fields.swap(a, a + 1);
        true
    }
    fn new_row(&self, _full: bool) -> Option<Cell> {
        Some(Cell::num(""))
    }
}

/// 波長の表（波長・ウェイト・主波長）
pub(super) struct WaveSheet<'a> {
    pub f: &'a mut SysForm,
}

impl Sheet for WaveSheet<'_> {
    fn rows(&self) -> usize {
        self.f.waves.len()
    }
    fn cols(&self) -> usize {
        3
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Wavelengths [µm]"), tr("Weight"), tr("Primary")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [90.0, 56.0, 56.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let w = &self.f.waves[row];
        match col {
            0 => Cell::num(fmt_num(w.um)),
            1 => Cell::num(fmt_num(w.w)),
            _ => Cell::boolean(self.f.primary == row + 1),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        match col {
            0 => {
                let what = tr("Wavelengths [µm]");
                let t = text.trim();
                if t.is_empty() {
                    return Err(trf("Enter a value for {}", &[&what]));
                }
                self.f.waves[row].um = num_in(t, what, 0.0, 0.1, 30.0)?;
            }
            1 => self.f.waves[row].w = num_in(text, tr("Weight"), 1.0, 0.0, 1e6)?,
            // 主波長は 1 つ（外すのは別の行を選んで）
            _ => {
                if text.trim() == "true" {
                    self.f.primary = row + 1;
                }
            }
        }
        Ok(())
    }
    /// 前の行から 0.05 µm 長い波長
    fn insert(&mut self, at: usize) -> bool {
        let um = at
            .checked_sub(1)
            .and_then(|i| self.f.waves.get(i))
            .map_or(0.5875618, |w| w.um + 0.05);
        self.f.waves.insert(at, WavePt::um(um));
        if self.f.primary > at {
            self.f.primary += 1;
        }
        true
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        let n = self.f.waves.len();
        let Some((lo, hi)) = keep_range(n, rows) else {
            return;
        };
        self.f.waves.drain(lo..=hi);
        // 主波長が消えた行なら消した所の行、後ろなら消した数だけ前へ
        let p = self.f.primary.saturating_sub(1);
        let p = if p < lo {
            p
        } else if p <= hi {
            lo
        } else {
            p - (hi - lo + 1)
        };
        self.f.primary = p.min(self.f.waves.len() - 1) + 1;
    }
    fn swap(&mut self, a: usize) -> bool {
        self.f.waves.swap(a, a + 1);
        if self.f.primary == a + 1 {
            self.f.primary = a + 2;
        } else if self.f.primary == a + 2 {
            self.f.primary = a + 1;
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

    /// 視野: 書いた値・誤り・前の行から足す・全部消しても 1 行残す
    #[test]
    fn field_sheet() {
        let mut v = vec![FieldPt::y(0.0), FieldPt::y(5.0)];
        let mut sh = FieldSheet {
            fields: &mut v,
            vig: true,
            unit: "°",
        };
        assert_eq!(sh.cols(), 8);
        assert_eq!(sh.header(0, None), "x [°]");
        sh.set(1, 0, "2").unwrap();
        assert!(sh.set(1, 2, "-1").is_err());
        assert!(sh.set(1, 1, "a").is_err());
        sh.set(1, 7, "0.5").unwrap();
        sh.add_row("3").unwrap();
        assert_eq!((sh.fields[2].x, sh.fields[2].y), (3.0, 6.0));
        assert!(sh.add_row("x").is_err());
        assert_eq!(sh.rows(), 3);
        sh.remove(0..=2);
        assert_eq!(sh.rows(), 1);
        assert_eq!(v[0], FieldPt::y(0.0));
    }

    /// 波長: 範囲・主波長は 1 つで、挿入・入れ替え・削除に付いていく
    #[test]
    fn wave_sheet() {
        let mut f = SysForm::of_combo(&Default::default());
        f.waves = [0.48, 0.55, 0.65].map(WavePt::um).to_vec();
        f.primary = 2;
        let mut sh = WaveSheet { f: &mut f };
        assert!(sh.set(0, 0, "40").is_err());
        assert!(sh.set(0, 0, "").is_err());
        sh.set(2, 2, "true").unwrap();
        assert_eq!(sh.f.primary, 3);
        sh.set(2, 2, "false").unwrap();
        assert_eq!(sh.f.primary, 3);
        sh.insert(0);
        assert_eq!(sh.f.primary, 4);
        sh.swap(2);
        assert_eq!(sh.f.primary, 3);
        sh.remove(0..=0);
        assert_eq!(sh.f.primary, 2);
        assert_eq!(sh.f.waves[1].um, 0.65);
        sh.remove(1..=1);
        assert_eq!(sh.f.primary, 2);
        sh.remove(0..=1);
        assert_eq!((sh.rows(), sh.f.primary), (1, 1));
    }
}
