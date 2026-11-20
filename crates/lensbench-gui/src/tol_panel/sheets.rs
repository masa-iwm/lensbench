//! 公差のパネルの表（玉・群・個別の公差）と、個別の公差の行と JSON の行き来（tol_panel.rs から分けた）

use serde_json::{json, Value};

use super::{dist_label, preset_label, GroupRow, LensRow, TolRow, DISTS, PRESETS};
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::param_pick as pp;
use crate::widgets::sheet::{Cell, Sheet};

/// パラメータの欄の名前の並び（カンマ区切り）
fn names(param: &str) -> Vec<&str> {
    param
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect()
}

fn nums(s: &str, what: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| Ok(crate::widgets::parse_num(x, what)?.unwrap_or(0.0)))
        .collect()
}

/// 分布の名前が書けるか（"normal:3"・"disc_x" なども）
fn dist_ok(v: &str) -> bool {
    v.is_empty() || serde_json::from_value::<lensbench_design::montecarlo::Dist>(json!(v)).is_ok()
}

impl TolRow {
    /// 個別の公差の JSON（`lensbench tolerance --spec` の tols の 1 つ）。パラメータが空なら None
    pub(crate) fn to_json(&self) -> Result<Option<Value>, String> {
        let ns = names(&self.param);
        let Some(first) = ns.first() else {
            return Ok(None);
        };
        let p = self.param.trim();
        let mut o = if ns.len() > 1 || self.multi {
            json!({ "params": ns })
        } else {
            json!({ "param": first })
        };
        let num = |s: &str| crate::widgets::parse_num(s, p);
        match (num(&self.pm)?, num(&self.lo)?, num(&self.hi)?) {
            (Some(pm), None, None) => o["pm"] = json!(pm),
            (None, Some(lo), Some(hi)) => {
                o["lo"] = json!(lo);
                o["hi"] = json!(hi);
            }
            (None, None, None) => return Err(trf("{} has no ±", &[&p])),
            _ => return Err(trf("{}: give ± or both lo and hi", &[&p])),
        }
        if !self.dist.is_empty() {
            o["dist"] = json!(self.dist);
        }
        if self.relative {
            o["relative"] = json!(true);
        }
        let sc = nums(&self.scale, p)?;
        if !sc.is_empty() {
            o["scale"] = json!(sc);
        }
        if !self.label.trim().is_empty() {
            o["label"] = json!(self.label.trim());
        }
        Ok(Some(o))
    }

    /// 個別の公差の JSON から。欄で表せない書き方なら None（そのまま残す）
    pub(crate) fn from_json(t: &Value) -> Option<TolRow> {
        let o = t.as_object()?;
        let f = |k: &str| {
            o.get(k)
                .map(|x| x.as_f64().map(fmt_num))
                .unwrap_or(Some(String::new()))
        };
        let s = |k: &str| {
            o.get(k)
                .map(|x| x.as_str().map(String::from))
                .unwrap_or(Some(String::new()))
        };
        let (param, multi) = match (o.get("param"), o.get("params")) {
            (Some(Value::String(p)), None) if !p.contains(',') => (p.clone(), false),
            (None, Some(Value::Array(a))) => {
                let v: Option<Vec<&str>> = a.iter().map(Value::as_str).collect();
                let v = v.filter(|v| !v.is_empty() && v.iter().all(|x| !x.contains(',')))?;
                (v.join(", "), true)
            }
            _ => return None,
        };
        let scale = match o.get("scale") {
            None => String::new(),
            Some(Value::Array(a)) if a.iter().all(Value::is_number) => a
                .iter()
                .filter_map(Value::as_f64)
                .map(fmt_num)
                .collect::<Vec<_>>()
                .join(", "),
            _ => return None,
        };
        let known = [
            "param", "params", "pm", "lo", "hi", "dist", "relative", "scale", "label",
        ];
        if o.keys().any(|k| !known.contains(&k.as_str())) {
            return None;
        }
        Some(TolRow {
            param,
            multi,
            pm: f("pm")?,
            lo: f("lo")?,
            hi: f("hi")?,
            dist: s("dist")?,
            relative: match o.get("relative") {
                None => false,
                Some(x) => x.as_bool()?,
            },
            scale,
            label: s("label")?,
        })
    }
}

/// 玉ごとの組の表
pub(super) struct LensSheet<'a> {
    pub lenses: &'a mut Vec<LensRow>,
    pub units: Vec<(String, String)>,
}

impl Sheet for LensSheet<'_> {
    fn rows(&self) -> usize {
        self.lenses.len()
    }
    fn cols(&self) -> usize {
        2
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Lens"), tr("Set")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [100.0, 100.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let l = &self.lenses[row];
        if col == 0 {
            return Cell::list(l.name.clone(), self.units.clone(), true).hint("lens1");
        }
        let items = PRESETS
            .iter()
            .filter(|x| !x.0.is_empty())
            .map(|x| (x.0.to_string(), tr(x.1).to_string()))
            .collect();
        Cell::list(l.preset.clone(), items, false)
            .shown(preset_label(&l.preset))
            .hint(tr("Default"))
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let t = text.trim();
        let l = &mut self.lenses[row];
        if col == 0 {
            l.name = t.into();
        } else if PRESETS.iter().any(|x| x.0 == t) {
            l.preset = t.into();
        } else {
            return Err(trf("No set {}", &[&t]));
        }
        Ok(())
    }
    fn new_row(&self, full: bool) -> Option<Cell> {
        Some(Cell::list(
            "",
            if full { self.units.clone() } else { Vec::new() },
            true,
        ))
    }
    fn insert(&mut self, at: usize) -> bool {
        self.lenses.insert(
            at,
            LensRow {
                name: String::new(),
                preset: "none".into(),
            },
        );
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.lenses.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.lenses.swap(a, a + 1);
        true
    }
}

/// 群・ブロックの公差の表
pub(super) struct GroupSheet<'a> {
    pub groups: &'a mut Vec<GroupRow>,
    pub units: Vec<(String, String)>,
}

impl Sheet for GroupSheet<'_> {
    fn rows(&self) -> usize {
        self.groups.len()
    }
    fn cols(&self) -> usize {
        4
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [tr("Name"), tr("Decenter"), tr("Tilt"), tr("Shift")][col].into()
    }
    fn width(&self, col: usize) -> f32 {
        [80.0, 64.0, 64.0, 64.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let g = &self.groups[row];
        match col {
            0 => Cell::list(g.name.clone(), self.units.clone(), true).hint("G1"),
            1 => Cell::num(g.decenter.clone()).tip(tr("Decenter [mm]")),
            2 => Cell::num(g.tilt.clone()).tip(tr("Tilt [deg]")),
            _ => Cell::num(g.shift.clone()).tip(tr("Shift [mm]")),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let t = text.trim();
        if col > 0 {
            crate::widgets::parse_num(t, &self.header(col, None))?;
        }
        let g = &mut self.groups[row];
        match col {
            0 => g.name = t.into(),
            1 => g.decenter = t.into(),
            2 => g.tilt = t.into(),
            _ => g.shift = t.into(),
        }
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        self.groups.insert(at, GroupRow::default());
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.groups.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.groups.swap(a, a + 1);
        true
    }
}

/// 個別の公差の表
pub(super) struct TolSheet<'a> {
    pub tols: &'a mut Vec<TolRow>,
    pub names: Vec<(String, String)>,
}

impl Sheet for TolSheet<'_> {
    fn rows(&self) -> usize {
        self.tols.len()
    }
    fn cols(&self) -> usize {
        8
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        [
            tr("Parameter"),
            "±",
            tr("Min"),
            tr("Max"),
            tr("Distribution"),
            tr("Ratio"),
            tr("Scale"),
            tr("Label"),
        ][col]
            .into()
    }
    fn width(&self, col: usize) -> f32 {
        [90.0, 64.0, 64.0, 64.0, 100.0, 50.0, 70.0, 80.0][col]
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let t = &self.tols[row];
        match col {
            0 => Cell::list(t.param.clone(), self.names.clone(), true)
                .hint("3.t")
                .tip(if names(&t.param).len() > 1 {
                    tr("Parameters (comma-separated) get the same error").to_string()
                } else {
                    pp::param_desc(&t.param)
                }),
            1 => Cell::num(t.pm.clone()).tip(tr("± (or Min and Max)")),
            2 => Cell::num(t.lo.clone())
                .hint("—")
                .tip(tr("Lower limit (with Max, instead of ±)")),
            3 => Cell::num(t.hi.clone())
                .hint("—")
                .tip(tr("Upper limit (with Min, instead of ±)")),
            4 => Cell::list(
                t.dist.clone(),
                DISTS
                    .iter()
                    .map(|x| (x.0.to_string(), tr(x.1).to_string()))
                    .collect(),
                true,
            )
            .shown(dist_label(&t.dist))
            .hint(tr("Default")),
            5 => Cell::boolean(t.relative)
                .tip(tr("Make ± a ratio of the current value (0.01 = ±1%)")),
            6 => Cell::text(t.scale.clone()).hint("—").tip(tr(
                "Factor per parameter on the error (comma-separated, as many as the parameters)",
            )),
            _ => Cell::text(t.label.clone())
                .hint("—")
                .tip(tr("Name in the results (empty = the parameters)")),
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let v = text.trim();
        let t = &mut self.tols[row];
        match col {
            0 => t.param = v.into(),
            1..=3 => {
                crate::widgets::parse_num(v, &t.param)?;
                *[&mut t.pm, &mut t.lo, &mut t.hi][col - 1] = v.into();
            }
            4 => {
                if !dist_ok(v) {
                    return Err(trf("No distribution {}", &[&v]));
                }
                t.dist = v.into();
            }
            5 => t.relative = v == "true",
            6 => {
                nums(v, &t.param)?;
                t.scale = v.into();
            }
            _ => t.label = v.into(),
        }
        Ok(())
    }
    fn new_row(&self, full: bool) -> Option<Cell> {
        Some(Cell::list(
            "",
            if full { self.names.clone() } else { Vec::new() },
            true,
        ))
    }
    fn insert(&mut self, at: usize) -> bool {
        self.tols.insert(at, TolRow::default());
        true
    }
    fn remove(&mut self, rows: std::ops::RangeInclusive<usize>) {
        self.tols.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.tols.swap(a, a + 1);
        true
    }
}
