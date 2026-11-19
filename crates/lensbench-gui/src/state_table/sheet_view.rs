//! 状態の表を表の部品（[`crate::widgets::sheet`]）で描く: 列 = 値の名前・基本・状態（構成）、行 = 状態で変えた値。
//! 見出し・セルの右クリックの操作は貯めて、表を描いた後に当てる

use std::cell::RefCell;
use std::ops::RangeInclusive;

use super::*;
use crate::widgets::sheet::{Cell, Sheet};

/// 表の 1 行
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Row {
    It(Item),
    /// 表に行の無い系の値（温度・気圧など）を状態で変えたか（見るだけ）
    More,
}

/// 表を描いた後に当てる操作
#[derive(Debug, Clone)]
pub(super) enum Act {
    Head(HeadAct),
    /// 列（None は基本）を今の状態に
    Current(Option<String>),
    Rename(String),
    /// 行・状態の値を基本の値に
    Clear(usize, String),
    Pick(usize, String, PickAct),
    Remove(RangeInclusive<usize>),
    /// 行 a と a+1 を入れ替える
    Swap(usize),
}

pub(super) struct McSheet<'a> {
    pub c: &'a mut Combo,
    pub rows: Vec<Row>,
    /// 列 1.. の状態（None は基本）
    pub cols: Vec<Option<String>>,
    pub now: FaceNow<'a>,
    pub cache: &'a HashMap<Face, Value>,
    pub stops: &'a [(Loc, usize)],
    pub acts: RefCell<Vec<Act>>,
}

impl McSheet<'_> {
    fn cx(&self) -> Cx<'_> {
        Cx {
            now: self.now,
            cache: self.cache,
        }
    }
    fn col(&self, k: usize) -> Option<&Option<String>> {
        k.checked_sub(1).and_then(|i| self.cols.get(i))
    }
    fn act(&self, a: Act) {
        self.acts.borrow_mut().push(a);
    }
    /// 絞りの面の候補（表示の名前・値）
    fn stop_items(&self, base: bool) -> Vec<(String, Option<(Loc, usize)>)> {
        let mut v = Vec::new();
        // 先頭に自動で置く絞りは基本だけ（状態から指せるブロックが無い）
        if base {
            v.push((stop_label(self.c, &None), None));
        }
        for f in self.stops {
            let x = Some(f.clone());
            v.push((stop_label(self.c, &x), x));
        }
        v
    }
}

/// 打った文字を、今の値と同じ種類の値に
fn parse_as(
    old: &Val,
    t: &str,
    what: &str,
    stops: &[(String, Option<(Loc, usize)>)],
) -> Result<Val, String> {
    let num = |t: &str| -> Result<f64, String> {
        crate::widgets::parse_num(t, what)?.ok_or_else(|| trf("Enter a value for {}", &[&what]))
    };
    Ok(match old {
        Val::Bool(_) => Val::Bool(matches!(t, "true" | "1")),
        Val::Num(_) => Val::Num(num(t)?),
        Val::Int(_) => Val::Int(num(t)?.round().max(0.0) as usize),
        Val::Text(_) => Val::Text(t.to_string()),
        Val::Choice(_, keys) => match keys.iter().position(|k| *k == t) {
            Some(k) => Val::Choice(k, keys),
            None => return Err(trf("{} is not one of {}", &[&t, &keys.join(", ")])),
        },
        Val::Stop(_) => match stops.iter().find(|(l, _)| l == t) {
            Some((_, at)) => Val::Stop(at.clone()),
            None => return Err(trf("No surface {}", &[&t])),
        },
        // 読めない値も打ち込めば値になる（数・true/false・文字）
        Val::Unknown => match (t.parse::<f64>(), t) {
            (Ok(x), _) => Val::Num(x),
            (_, "true") => Val::Bool(true),
            (_, "false") => Val::Bool(false),
            _ => Val::Text(t.to_string()),
        },
    })
}

impl Sheet for McSheet<'_> {
    fn rows(&self) -> usize {
        self.rows.len()
    }
    fn cols(&self) -> usize {
        self.cols.len() + 1
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        match self.col(col) {
            None => tr("Value").into(),
            Some(st) => {
                let name = st.clone().unwrap_or_else(|| tr("Base").to_string());
                if self.c.state == *st {
                    format!("▶ {name}")
                } else {
                    name
                }
            }
        }
    }
    fn header_tip(&self, col: usize, _cur: Option<usize>) -> String {
        match self.col(col) {
            Some(Some(n)) => trf(
                "Double-click to make {} the current configuration; right-click to copy, insert, move, rename or delete",
                &[n],
            ),
            Some(None) => tr("Double-click to show the base values (no state)").into(),
            None => String::new(),
        }
    }
    fn width(&self, col: usize) -> f32 {
        if col == 0 {
            170.0
        } else {
            90.0
        }
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let ro = || Some(String::new());
        let it = match &self.rows[row] {
            Row::More => {
                return match self.col(col) {
                    None => Cell::text(tr("Other system settings")).ro(ro()),
                    Some(st) => {
                        let own = st
                            .as_ref()
                            .is_some_and(|n| self.c.state_more.contains_key(n));
                        Cell::text(if own { tr("changed") } else { "—" }).ro(ro())
                    }
                };
            }
            Row::It(it) => it,
        };
        let Some(st) = self.col(col) else {
            return Cell::text(it.label(self.c)).ro(ro());
        };
        let st = st.as_deref();
        let cx = self.cx();
        let Some(v) = it.get(self.c, st, &cx) else {
            return Cell::empty();
        };
        let own = st.is_some_and(|n| it.own(self.c, n));
        if let Some(m) = st.and_then(|n| picks::pick_of(self.c, it, n)) {
            let mut from = if m.from.is_empty() {
                tr("Base").to_string()
            } else {
                m.from.clone()
            };
            let (owner, _) = picks::key_of(it);
            if let Some(r) = (!m.row.is_empty())
                .then(|| picks::src_item(owner.as_ref(), m))
                .flatten()
            {
                from = format!("{from} {}", r.label(self.c));
            }
            let why = trf(
                "Pickup: {} × {} + {}",
                &[
                    &from,
                    &crate::json_view::fmt_num(m.scale),
                    &crate::json_view::fmt_num(m.offset),
                ],
            );
            return Cell::text(format!("{} P", val_text(self.c, &v))).ro(Some(why));
        }
        let locked = it
            .locked(self.c)
            .then(|| tr("The element is locked").to_string());
        let cell = match &v {
            Val::Bool(b) => Cell::boolean(*b),
            Val::Num(x) => Cell::num(crate::json_view::fmt_num(*x)),
            Val::Int(k) => Cell::int(k.to_string()),
            Val::Text(t) => Cell::text(t.clone()),
            Val::Choice(k, keys) => Cell::list(
                keys.get(*k).copied().unwrap_or_default(),
                keys.iter()
                    .map(|k| (k.to_string(), String::new()))
                    .collect(),
                false,
            ),
            Val::Stop(at) => Cell::list(
                stop_label(self.c, at),
                self.stop_items(st.is_none())
                    .into_iter()
                    .map(|(l, _)| (l, String::new()))
                    .collect(),
                false,
            ),
            Val::Unknown => Cell::text("").hint("—").tip(tr(
                "The base value is read from the built system; select Base once to read it",
            )),
        };
        cell.mark(own).ro(locked)
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        let Row::It(it) = self.rows[row].clone() else {
            return Err(String::new());
        };
        let Some(st) = self.col(col).cloned() else {
            return Err(String::new());
        };
        let t = text.trim();
        // 状態の列を空にすると基本の値に戻す
        if t.is_empty() {
            return match &st {
                Some(n) => {
                    it.clear(self.c, n);
                    Ok(())
                }
                None => Err(trf("Enter a value for {}", &[&it.label(self.c)])),
            };
        }
        let old = {
            let cx = self.cx();
            it.get(self.c, st.as_deref(), &cx).unwrap_or(Val::Unknown)
        };
        let v = parse_as(&old, t, &it.label(self.c), &self.stop_items(st.is_none()))?;
        let (now, cache) = (self.now, self.cache);
        it.set(self.c, st.as_deref(), v, &Cx { now, cache });
        Ok(())
    }
    fn can_insert(&self) -> bool {
        false
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        self.act(Act::Remove(rows));
    }
    fn swap(&mut self, a: usize) -> bool {
        self.act(Act::Swap(a));
        true
    }
    fn header_ui(&self, resp: &egui::Response, col: usize) {
        let Some(st) = self.col(col) else { return };
        // 切り替えはダブルクリック（クリックは列を選ぶのに残す）
        if resp.double_clicked() && self.c.state != *st {
            self.act(Act::Current(st.clone()));
        }
        let Some(n) = st else { return };
        resp.context_menu(|ui| {
            let item = |ui: &mut Ui, label: &str, a: Act| {
                if ui.button(label).clicked() {
                    self.act(a);
                    ui.close_menu();
                }
            };
            item(
                ui,
                tr("Copy to a new configuration"),
                Act::Head(HeadAct::Copy(n.clone())),
            );
            item(
                ui,
                tr("Insert a configuration with the base values"),
                Act::Head(HeadAct::NewBase(n.clone())),
            );
            item(
                ui,
                tr("Insert a configuration with pickups"),
                Act::Head(HeadAct::WithPicks(n.clone())),
            );
            ui.separator();
            item(ui, tr("Move left"), Act::Head(HeadAct::Move(n.clone(), -1)));
            item(ui, tr("Move right"), Act::Head(HeadAct::Move(n.clone(), 1)));
            ui.separator();
            item(ui, tr("Rename"), Act::Rename(n.clone()));
            item(ui, tr("Delete"), Act::Head(HeadAct::Remove(n.clone())));
        });
    }
    fn cell_menu(&self, ui: &mut Ui, row: usize, col: usize) {
        let (Row::It(it), Some(Some(n))) = (&self.rows[row], self.col(col)) else {
            return;
        };
        let pk = picks::pick_of(self.c, it, n);
        if it.own(self.c, n) && pk.is_none() && ui.button(tr("Use base value")).clicked() {
            self.act(Act::Clear(row, n.clone()));
            ui.close_menu();
        }
        if !it.locked(self.c) {
            // 同じ持ち主（要素か組合せ）の別の行、組合せの行（系の値・絞り）
            let rows: Vec<(String, String)> = self
                .rows
                .iter()
                .filter_map(|r| match r {
                    Row::It(r) => picks::row_key(it, r)
                        .filter(|k| !k.is_empty())
                        .map(|k| (r.label(self.c), k)),
                    Row::More => None,
                })
                .collect();
            // 輪になる元（自分自身・自分からピックアップしている値）は出さない
            let (owner, _) = picks::key_of(it);
            let ok = |row: &str, from: &str| {
                let src = if row.is_empty() {
                    Some(it.clone())
                } else {
                    picks::row_item(owner.as_ref(), row)
                };
                src.is_some_and(|s| !picks::would_loop(self.c, it, n, &s, from))
            };
            pick_menu(ui, &self.cols, &ok, &rows, pk, &mut |a| {
                self.act(Act::Pick(row, n.clone(), a))
            });
        }
        ui.separator();
    }
}
