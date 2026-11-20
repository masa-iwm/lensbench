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
    /// 最後の空の行（左端に書くと行を足す）
    fn new_row(&self, full: bool) -> Option<Cell> {
        Some(Cell::list(
            "",
            if full { self.names.clone() } else { Vec::new() },
            true,
        ))
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
/// P の列の見出しは今の行の種類の鍵の名前。fold なら P の列を畳んで要約の 1 列にする
pub(super) struct TermSheet<'a> {
    pub terms: &'a mut Vec<TermRow>,
    /// 足す行の種類
    pub new_ty: &'a str,
    pub npar: usize,
    /// 項ごとの値と評価関数への寄与 [%]（最適化のパネルだけ。None は列を出さない、行の None は値が無い）
    pub vals: Option<&'a [Option<(f64, f64)>]>,
    pub fold: bool,
}

/// 表の列の役
#[derive(Clone, Copy, PartialEq, Debug)]
enum Col {
    Type,
    /// P1..Pn（0 始まり）
    Par(usize),
    /// 畳んだ P の列の要約（target を除く）
    Summary,
    /// 畳んだときの target（畳まずに出す）
    Target,
    Weight,
    Config,
    Value,
    Contrib,
}

impl TermSheet<'_> {
    fn col(&self, col: usize) -> Col {
        let np = if self.fold { 2 } else { self.npar };
        match col {
            0 => Col::Type,
            1 if self.fold => Col::Summary,
            2 if self.fold => Col::Target,
            c if c <= np => Col::Par(c - 1),
            c => [Col::Weight, Col::Config, Col::Value, Col::Contrib][(c - np - 1).min(3)],
        }
    }
    fn key(&self, row: usize, i: usize) -> Option<(&'static str, KeyKind)> {
        keys(&self.terms.get(row)?.ty).get(i).copied()
    }
}

/// 行の P の値の要約（「field 1 · wave 2」。target は別の列なので除く。コメントの行は文）
pub(super) fn summary(t: &TermRow) -> String {
    if t.ty == "comment" {
        return t.get("text").to_string();
    }
    keys(&t.ty)
        .iter()
        .filter(|(k, _)| *k != "target")
        .filter_map(|(k, _)| {
            let v = t.get(k).trim();
            (!v.is_empty()).then(|| format!("{k} {v}"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
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
        (if self.fold { 2 } else { self.npar }) + if self.vals.is_some() { 5 } else { 3 }
    }
    fn header(&self, col: usize, cur: Option<usize>) -> String {
        match self.col(col) {
            Col::Type => tr("Type").into(),
            Col::Weight => tr("Weight").into(),
            Col::Config => tr("Config").into(),
            Col::Value => tr("Value").into(),
            Col::Contrib => tr("% Contrib").into(),
            Col::Summary => tr("Parameters").into(),
            Col::Target => "target".into(),
            Col::Par(i) => match cur.and_then(|r| self.key(r, i)) {
                Some((k, _)) => k.into(),
                None => format!("P{}", i + 1),
            },
        }
    }
    fn header_tip(&self, col: usize, _cur: Option<usize>) -> String {
        match self.col(col) {
            Col::Par(i) => trf("Parameter {} of the current row's term type", &[&(i + 1)]),
            Col::Summary => tr("The parameters of each term (show the parameter columns to edit them)").into(),
            Col::Value => tr("Value of the term at the last Evaluate or Optimize (after optimizing). With a target it is the quantity itself, otherwise √Σr²").into(),
            Col::Contrib => tr("Share of the merit function: weight × Σ residual² of the term over the sum of all terms").into(),
            _ => String::new(),
        }
    }
    fn width(&self, col: usize) -> f32 {
        match self.col(col) {
            Col::Type => 90.0,
            Col::Summary => 240.0,
            _ => 54.0,
        }
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let t = &self.terms[row];
        let comment = t.ty == "comment";
        match self.col(col) {
            Col::Type => {
                let items = TERM_TYPES
                    .iter()
                    .map(|x| (x.to_string(), type_hint(x).to_string()))
                    .collect();
                Cell::list(t.ty.clone(), items, false).tip(type_hint(&t.ty))
            }
            Col::Summary if comment => Cell::text(t.get("text")).tip(tr("Comment")),
            Col::Summary => {
                let s = summary(t);
                Cell::text(s.clone()).tip(s).ro(Some(
                    tr("Show the parameter columns to edit the parameters").into(),
                ))
            }
            _ if comment && col > 1 => Cell::empty(),
            Col::Target => match keys(&t.ty).iter().find(|(k, _)| *k == "target") {
                Some(_) => Cell::num(t.get("target").to_string()).tip("target"),
                None => Cell::empty(),
            },
            Col::Weight => Cell::num(t.weight.clone()).hint("1"),
            c @ (Col::Value | Col::Contrib) => {
                let v = self.vals.and_then(|v| v.get(row).copied().flatten());
                let text = match v {
                    Some((x, _)) if c == Col::Value => fmt_g(x),
                    Some((_, c)) => format!("{c:.3}"),
                    None => String::new(),
                };
                Cell::num(text).hint("—").ro(Some(String::new()))
            }
            Col::Config => Cell::int(t.config.clone())
                .hint("—")
                .tip(tr("Configuration number (empty = the current one)")),
            Col::Par(i) => {
                let Some((k, kind)) = self.key(row, i) else {
                    return Cell::empty();
                };
                let v = t.get(k).to_string();
                if comment {
                    return Cell::text(v).overflow().tip(tr("Comment"));
                }
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
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let c = self.col(col);
        let t = &mut self.terms[row];
        let v = text.trim();
        match c {
            Col::Type => {
                if !TERM_TYPES.contains(&v) {
                    return Err(trf("Unknown term type {}", &[&v]));
                }
                // 種類を変えると鍵を作り直す（ウェイト・構成と、新しい種類にもある target は残す）
                if t.ty != v {
                    let mut n = TermRow::new(v);
                    n.weight = std::mem::take(&mut t.weight);
                    n.config = std::mem::take(&mut t.config);
                    let target = t.get("target").to_string();
                    if !target.is_empty() && keys(v).iter().any(|(k, _)| *k == "target") {
                        n.set("target", &target);
                    }
                    *t = n;
                }
            }
            Col::Summary if t.ty == "comment" => t.set("text", text),
            Col::Summary | Col::Value | Col::Contrib => return Err(String::new()),
            Col::Weight | Col::Config | Col::Target if t.ty == "comment" => {
                return Err(String::new())
            }
            Col::Target => {
                let Some((k, kind)) = keys(&t.ty).iter().copied().find(|(k, _)| *k == "target")
                else {
                    return Err(String::new());
                };
                check_key(k, kind, v)?;
                t.set(k, v);
            }
            Col::Weight => {
                crate::widgets::parse_num(v, tr("Weight"))?;
                t.weight = v.into();
            }
            Col::Config => {
                if !v.is_empty() && v.parse::<u64>().is_err() {
                    return Err(trf("Configuration number {} is not an integer", &[&v]));
                }
                t.config = v.into();
            }
            Col::Par(i) => {
                let Some((k, kind)) = keys(&t.ty).get(i).copied() else {
                    return Err(String::new());
                };
                // コメントの文は前後の空白も残す
                if kind == KeyKind::Text && t.ty == "comment" {
                    t.set(k, text);
                    return Ok(());
                }
                check_key(k, kind, v)?;
                t.set(k, v);
            }
        }
        Ok(())
    }
    /// 最後の空の行（左端に書くと行を足す）
    fn new_row(&self, full: bool) -> Option<Cell> {
        Some(Cell::list(
            "",
            TERM_TYPES
                .iter()
                .filter(|_| full)
                .map(|x| (x.to_string(), type_hint(x).to_string()))
                .collect(),
            false,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// コメントの行: JSON は type と text だけ、要約は文、結果の値はコメントを飛ばした順に当てる
    #[test]
    fn comment_rows() {
        let mut c = TermRow::new("comment");
        c.set("text", "image quality");
        assert_eq!(
            term_json(&c).unwrap(),
            json!({"type": "comment", "text": "image quality"})
        );
        assert_eq!(summary(&c), "image quality");
        let back = term_rows(&json!([{"type": "comment", "text": "x"}])).unwrap();
        assert_eq!(back[0].get("text"), "x");
        let mut q = TermRow::new("paraxial");
        q.set("target", "50");
        // target は要約に入れず、畳んでも列で出す
        assert_eq!(summary(&q), "q efl");
        {
            use crate::widgets::sheet::Sheet;
            let mut terms = vec![q.clone()];
            let mut sh = TermSheet {
                terms: &mut terms,
                new_ty: "",
                npar: par_cols(),
                vals: None,
                fold: true,
            };
            assert_eq!(sh.cols(), 5);
            assert_eq!(sh.header(2, None), "target");
            assert_eq!(sh.cell(0, 2).text, "50");
            sh.set(0, 2, "60").unwrap();
            assert!(sh.set(0, 2, "x").is_err());
            assert_eq!(sh.terms[0].get("target"), "60");
            // 最後の空の行にタイプを書くとその項を足す
            sh.add_row("ray").unwrap();
            assert!(sh.add_row("nope").is_err());
            assert_eq!(sh.terms.len(), 2);
            assert_eq!(sh.terms[1].ty, "ray");
        }
        let mut p = OptPanel::new();
        p.terms = vec![c, q];
        p.run_terms = p.terms.clone();
        let tv = |default: bool, value: f64| TermValue {
            default,
            value0: value,
            value,
            value_kind: "quantity".into(),
            contrib0: 100.0,
            contrib: 100.0,
            rays: None,
        };
        p.result = Some(std::sync::Arc::new(OptimizeView {
            terms: vec![tv(true, 1.0), tv(false, 49.0)],
            ..Default::default()
        }));
        assert_eq!(p.term_vals(), vec![None, Some((49.0, 100.0))]);
    }
}
