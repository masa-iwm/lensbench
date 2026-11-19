//! 最適化のパネルの表（変数・評価関数の項）。[`crate::widgets::sheet`] の中身

use super::*;
use crate::widgets::sheet::{Cell, Sheet};

/// 変数の表（名前・今の値・下限・上限・構成）
pub(super) struct VarSheet<'a> {
    pub vars: &'a mut Vec<VarRow>,
    pub rows: &'a [crate::lens_data::Row],
    pub names: Vec<(String, String)>,
    pub multi: bool,
}

impl Sheet for VarSheet<'_> {
    fn rows(&self) -> usize {
        self.vars.len()
    }
    fn cols(&self) -> usize {
        if self.multi {
            5
        } else {
            4
        }
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [
            tr("Name"),
            tr("Current"),
            tr("Min"),
            tr("Max"),
            tr("Config"),
        ][col]
            .into()
    }
    fn width(&self, col: usize) -> f32 {
        [110.0, 80.0, 64.0, 64.0, 56.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let v = &self.vars[row];
        match col {
            0 => Cell::list(v.name.clone(), self.names.clone(), true)
                .hint("7.t")
                .tip(pp::param_desc(&v.name)),
            1 => Cell::num(var_value(self.rows, &v.name).map_or(String::new(), fmt_num))
                .hint("—")
                .ro(Some(String::new())),
            2 => Cell::num(v.lo.clone()).hint("−∞"),
            3 => Cell::num(v.hi.clone()).hint("∞"),
            _ => Cell::text(v.configs.clone()).hint(tr("All")).tip(tr(
                "Configurations it applies to (comma separated, empty = all)",
            )),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let v = &mut self.vars[row];
        let t = text.trim();
        match col {
            0 => v.name = t.into(),
            2 | 3 => {
                crate::widgets::parse_num(t, &v.name)?;
                if col == 2 {
                    v.lo = t.into();
                } else {
                    v.hi = t.into();
                }
            }
            4 => {
                int_list(t)?;
                v.configs = t.into();
            }
            _ => return Err(String::new()),
        }
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        self.vars.insert(at, VarRow::default());
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.vars.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.vars.swap(a, a + 1);
        true
    }
}

/// 評価関数の項の表の P の列の数（種類の鍵のいちばん多い数）
pub(super) fn par_cols() -> usize {
    TERM_TYPES.iter().map(|t| keys(t).len()).max().unwrap_or(0)
}

/// 評価関数の項の表（OpticStudio の評価関数エディタのように 行＝項、列＝種類・P1..Pn・重み・構成）。
/// P の列の見出しは今の行の種類の鍵の名前
pub(super) struct TermSheet<'a> {
    pub terms: &'a mut Vec<TermRow>,
    /// 足す行の種類
    pub new_ty: &'a str,
    pub npar: usize,
    /// 項ごとの値と評価関数への寄与 [%]（最適化のパネルだけ。None は列を出さない、行の None は値が無い）
    pub vals: Option<&'a [Option<(f64, f64)>]>,
}

impl TermSheet<'_> {
    fn key(&self, row: usize, col: usize) -> Option<(&'static str, KeyKind)> {
        let t = self.terms.get(row)?;
        (1..=self.npar)
            .contains(&col)
            .then(|| keys(&t.ty).get(col - 1).copied())
            .flatten()
    }
}

/// 鍵の値が書けるか（空は既定の値）
fn check_key(k: &str, kind: KeyKind, v: &str) -> Result<(), String> {
    let v = v.trim();
    if v.is_empty() {
        return Ok(());
    }
    let ok = match kind {
        KeyKind::Num => v.parse::<f64>().is_ok(),
        KeyKind::Int => v.parse::<u64>().is_ok(),
        KeyKind::Field => v.parse::<u64>().is_ok() || pair(v).is_some(),
        KeyKind::Pair => pair(v).is_some(),
        KeyKind::Choice(o) => o.contains(&v),
        KeyKind::Bool => v == "true" || v == "false",
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(trf("Value of {}: {}", &[&k, &v]))
    }
}

impl Sheet for TermSheet<'_> {
    fn rows(&self) -> usize {
        self.terms.len()
    }
    fn cols(&self) -> usize {
        self.npar + if self.vals.is_some() { 5 } else { 3 }
    }
    fn header(&self, col: usize, cur: Option<usize>) -> String {
        if col == 0 {
            return tr("Type").into();
        }
        if col == self.npar + 1 {
            return tr("Weight").into();
        }
        if col == self.npar + 2 {
            return tr("Config").into();
        }
        if col == self.npar + 3 {
            return tr("Value").into();
        }
        if col == self.npar + 4 {
            return tr("% Contrib").into();
        }
        match cur.and_then(|r| self.key(r, col)) {
            Some((k, _)) => k.into(),
            None => format!("P{col}"),
        }
    }
    fn header_tip(&self, col: usize, _cur: Option<usize>) -> String {
        if (1..=self.npar).contains(&col) {
            trf("Parameter {} of the current row's term type", &[&col])
        } else if col == self.npar + 3 {
            tr("Value of the term at the last Evaluate or Optimize (after optimizing). With a target it is the quantity itself, otherwise √Σr²").into()
        } else if col == self.npar + 4 {
            tr("Share of the merit function: weight × Σ residual² of the term over the sum of all terms").into()
        } else {
            String::new()
        }
    }
    fn width(&self, col: usize) -> f32 {
        if col == 0 {
            90.0
        } else {
            54.0
        }
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let t = &self.terms[row];
        if col == 0 {
            let items = TERM_TYPES
                .iter()
                .map(|x| (x.to_string(), type_hint(x).to_string()))
                .collect();
            return Cell::list(t.ty.clone(), items, false).tip(type_hint(&t.ty));
        }
        if col == self.npar + 1 {
            return Cell::num(t.weight.clone()).hint("1");
        }
        if col >= self.npar + 3 {
            let v = self.vals.and_then(|v| v.get(row).copied().flatten());
            let text = match v {
                Some((x, _)) if col == self.npar + 3 => fmt_g(x),
                Some((_, c)) => format!("{c:.3}"),
                None => String::new(),
            };
            return Cell::num(text).hint("—").ro(Some(String::new()));
        }
        if col == self.npar + 2 {
            return Cell::int(t.config.clone())
                .hint("—")
                .tip(tr("Configuration number (empty = the current one)"));
        }
        let Some((k, kind)) = self.key(row, col) else {
            return Cell::empty();
        };
        let v = t.get(k).to_string();
        let c = match kind {
            KeyKind::Num => Cell::num(v),
            KeyKind::Int => Cell::int(v),
            KeyKind::Bool => Cell::boolean(v == "true"),
            KeyKind::Choice(o) => Cell::list(
                v,
                o.iter().map(|x| (x.to_string(), String::new())).collect(),
                false,
            ),
            _ => Cell::text(v),
        };
        c.tip(k)
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let npar = self.npar;
        let t = &mut self.terms[row];
        let v = text.trim();
        if col == 0 {
            if !TERM_TYPES.contains(&v) {
                return Err(trf("Unknown term type {}", &[&v]));
            }
            if t.ty != v {
                *t = TermRow::new(v);
            }
            return Ok(());
        }
        if col == npar + 1 {
            crate::widgets::parse_num(v, tr("Weight"))?;
            t.weight = v.into();
            return Ok(());
        }
        if col > npar + 2 {
            return Err(String::new());
        }
        if col == npar + 2 {
            if !v.is_empty() && v.parse::<u64>().is_err() {
                return Err(trf("Configuration number {} is not an integer", &[&v]));
            }
            t.config = v.into();
            return Ok(());
        }
        let Some((k, kind)) = keys(&t.ty).get(col - 1).copied() else {
            return Err(String::new());
        };
        check_key(k, kind, v)?;
        t.set(k, v);
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        let ty = if self.new_ty.is_empty() {
            "paraxial"
        } else {
            self.new_ty
        };
        self.terms.insert(at, TermRow::new(ty));
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.terms.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.terms.swap(a, a + 1);
        true
    }
}
