//! 組合せの状態の表（多重構成のエディタに当たる表）。行 = 状態で変えられる値（要素の ON/OFF・後の空気・偏心と傾き・
//! .zmx の取り込みの構成・群の移動・BS の経路）、列 = 基本と状態。状態の列のセルを変えるとその状態の値になり、
//! 右クリックの「基本の値に戻す」で外れる。系の値（開口・視野など）は状態ごとに変えたかだけを出す（変えるのは系の設定で）

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::combo::{Combo, ElemKind, Pick};
use crate::combo_tree::Loc;
use crate::i18n::{tr, trf};
use crate::theme;

/// 表の表示の設定
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    /// 状態で変えていない値の行も出す
    pub all: bool,
}

/// 表の 1 行が指す値
#[derive(Debug, Clone, PartialEq)]
enum Item {
    On(Loc),
    Air(Loc),
    Config(Loc),
    /// 偏心・傾き（0〜4 = dx dy tx ty tz）
    Place(Loc, usize),
    /// 組合せの群の移動（groups の番号）
    Shift(usize),
    /// .zmx の取り込みの中の群の移動（取り込み, 群の番号）
    ZShift(Loc, usize),
    /// BS の経路
    Path,
}

const PLACE: [&str; 5] = ["dx", "dy", "tx", "ty", "tz"];

/// 状態 st（None は基本）での行の値
enum Val {
    Bool(bool),
    Num(f64),
    Int(usize),
    Text(String),
}

fn place_get(p: &Pick, st: Option<&str>, k: usize) -> f64 {
    let pl = p.place_in(st);
    if k < 2 {
        pl.decenter[k]
    } else {
        pl.tilt[k - 2]
    }
}

impl Item {
    fn label(&self, c: &Combo) -> String {
        let el = |l: &Loc| c.get(l).map(Pick::label).unwrap_or_default();
        match self {
            Item::On(l) => format!("{} ON", el(l)),
            Item::Air(l) => format!("{} {}", el(l), tr("air")),
            Item::Config(l) => format!("{} {}", el(l), tr("Config")),
            Item::Place(l, k) => format!("{} {}", el(l), PLACE[*k]),
            Item::Shift(g) => format!(
                "{} {}",
                c.groups
                    .get(*g)
                    .map(|g| g.name.as_str())
                    .unwrap_or_default(),
                tr("shift")
            ),
            Item::ZShift(l, g) => format!(
                "{} {} {}",
                el(l),
                c.get(l)
                    .and_then(|p| p.zmx().zgroups.get(*g))
                    .map(|g| g.key().to_string())
                    .unwrap_or_default(),
                tr("shift")
            ),
            Item::Path => tr("BS path").to_string(),
        }
    }

    fn get(&self, c: &Combo, st: Option<&str>) -> Option<Val> {
        Some(match self {
            Item::On(l) => Val::Bool(c.get(l)?.on_in(st)),
            Item::Air(l) => Val::Num(c.get(l)?.air_in(st)),
            Item::Config(l) => Val::Int(c.get(l)?.config_in(st)),
            Item::Place(l, k) => Val::Num(place_get(c.get(l)?, st, *k)),
            Item::Shift(g) => {
                let g = c.groups.get(*g)?;
                Val::Num(
                    st.and_then(|n| g.state_shift.get(n).copied())
                        .unwrap_or(g.shift),
                )
            }
            Item::ZShift(l, g) => {
                let g = c.get(l)?.zmx().zgroups.get(*g)?;
                Val::Num(
                    st.and_then(|n| g.state_shift.get(n).copied())
                        .unwrap_or(0.0),
                )
            }
            Item::Path => Val::Text(
                st.and_then(|n| c.state_path.get(n).cloned())
                    .unwrap_or_else(|| c.path.clone()),
            ),
        })
    }

    /// 状態 n で値を変えているか
    fn own(&self, c: &Combo, n: &str) -> bool {
        match self {
            Item::On(l) => c.get(l).is_some_and(|p| p.state_on.contains_key(n)),
            Item::Air(l) => c.get(l).is_some_and(|p| p.state_air.contains_key(n)),
            Item::Config(l) => c
                .get(l)
                .is_some_and(|p| p.zmx().state_config.contains_key(n)),
            Item::Place(l, _) => c.get(l).is_some_and(|p| p.state_place.contains_key(n)),
            Item::Shift(g) => c
                .groups
                .get(*g)
                .is_some_and(|g| g.state_shift.contains_key(n)),
            Item::ZShift(l, g) => c
                .get(l)
                .and_then(|p| p.zmx().zgroups.get(*g))
                .is_some_and(|g| g.state_shift.contains_key(n)),
            Item::Path => c.state_path.contains_key(n),
        }
    }

    /// 状態 st（None は基本）の値を v に
    fn set(&self, c: &mut Combo, st: Option<&str>, v: Val) {
        let key = st.map(str::to_string);
        match (self, v) {
            (Item::On(l), Val::Bool(b)) => {
                if let Some(p) = c.get_mut(l) {
                    match key {
                        Some(n) => {
                            p.state_on.insert(n, b);
                        }
                        None => p.on = b,
                    }
                }
            }
            (Item::Air(l), Val::Num(x)) => {
                if let Some(p) = c.get_mut(l) {
                    *p.air_mut(st) = x.max(0.0);
                }
            }
            (Item::Config(l), Val::Int(k)) => {
                if let Some(p) = c.get_mut(l) {
                    let k = k.max(1);
                    match key {
                        Some(n) => {
                            p.zmx_mut().state_config.insert(n, k);
                        }
                        None => p.zmx_mut().config = k,
                    }
                }
            }
            (Item::Place(l, k), Val::Num(x)) => {
                if let Some(p) = c.get_mut(l) {
                    let (d, t) = p.place_mut(st);
                    if *k < 2 {
                        d[*k] = x;
                    } else {
                        t[*k - 2] = x;
                    }
                }
            }
            (Item::Shift(g), Val::Num(x)) => {
                if let Some(g) = c.groups.get_mut(*g) {
                    match key {
                        Some(n) => {
                            g.state_shift.insert(n, x);
                        }
                        None => g.shift = x,
                    }
                }
            }
            (Item::ZShift(l, g), Val::Num(x)) => {
                if let (Some(n), Some(g)) = (
                    key,
                    c.get_mut(l).and_then(|p| p.zmx_mut().zgroups.get_mut(*g)),
                ) {
                    g.state_shift.insert(n, x);
                }
            }
            (Item::Path, Val::Text(t)) => match key {
                Some(n) => {
                    c.state_path.insert(n, t);
                }
                None => c.path = t,
            },
            _ => {}
        }
    }

    /// 状態 n で変えた値をやめる（基本の値に戻す）
    fn clear(&self, c: &mut Combo, n: &str) {
        match self {
            Item::On(l) => {
                if let Some(p) = c.get_mut(l) {
                    p.state_on.remove(n);
                }
            }
            Item::Air(l) => {
                if let Some(p) = c.get_mut(l) {
                    p.state_air.remove(n);
                }
            }
            Item::Config(l) => {
                if let Some(p) = c.get_mut(l).filter(|p| p.kind == ElemKind::ZmxRange) {
                    p.zmx_mut().state_config.remove(n);
                }
            }
            Item::Place(l, _) => {
                if let Some(p) = c.get_mut(l) {
                    p.state_place.remove(n);
                }
            }
            Item::Shift(g) => {
                if let Some(g) = c.groups.get_mut(*g) {
                    g.state_shift.remove(n);
                }
            }
            Item::ZShift(l, g) => {
                if let Some(g) = c
                    .get_mut(l)
                    .filter(|p| p.kind == ElemKind::ZmxRange)
                    .and_then(|p| p.zmx_mut().zgroups.get_mut(*g))
                {
                    g.state_shift.remove(n);
                }
            }
            Item::Path => {
                c.state_path.remove(n);
            }
        }
    }
}

/// 表の行。all なら状態で変えていない値（要素の ON/OFF・後の空気・構成、群の移動）も
fn items(c: &Combo, all: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for l in c.all() {
        let Some(p) = c.get(&l) else { continue };
        if p.kind.has_on() && (all || !p.state_on.is_empty()) {
            out.push(Item::On(l.clone()));
        }
        if all || !p.state_air.is_empty() {
            out.push(Item::Air(l.clone()));
        }
        if p.kind == ElemKind::ZmxRange && (all || !p.zmx().state_config.is_empty()) {
            out.push(Item::Config(l.clone()));
        }
        if p.has_state_place() && !p.state_place.is_empty() {
            let ks: &[usize] = if p.kind == ElemKind::Mirror {
                &[2, 3]
            } else {
                &[0, 1, 2, 3, 4]
            };
            out.extend(ks.iter().map(|&k| Item::Place(l.clone(), k)));
        }
        for (g, z) in p.zmx().zgroups.iter().enumerate() {
            if p.kind == ElemKind::ZmxRange && !z.state_shift.is_empty() {
                out.push(Item::ZShift(l.clone(), g));
            }
        }
    }
    for (g, d) in c.groups.iter().enumerate() {
        if !d.state_shift.is_empty() || (all && !d.role.is_empty()) {
            out.push(Item::Shift(g));
        }
    }
    if !c.state_path.is_empty() || (all && !c.path.is_empty()) {
        out.push(Item::Path);
    }
    out
}

/// 状態の表。値を変えたら true（呼ぶ側が組み直す）
pub(crate) fn ui(ui: &mut Ui, c: &mut Combo, view: &mut View) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let mut changed = false;
    if c.states.is_empty() {
        ui.add_space(8.0);
        ui.label(tr(
            "No states yet. Add one in the Layout panel (states are the configurations of a combination: element ON/OFF, spacing, decenter and tilt, group shift, .zmx configuration).",
        ));
        return false;
    }
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut view.all, tr("Show values not changed in any state"));
        ui.label(
            RichText::new(tr(
                "Click a column header to make it the current state. Right-click a cell to use the base value",
            ))
            .small()
            .color(weak),
        );
    });
    let rows = items(c, view.all);
    let states = c.states.clone();
    let cols: Vec<Option<String>> = std::iter::once(None)
        .chain(states.iter().cloned().map(Some))
        .collect();
    let mut set: Option<(usize, Option<String>, Val)> = None;
    let mut clear: Option<(usize, String)> = None;
    let mut pick_state: Option<Option<String>> = None;
    egui::ScrollArea::horizontal()
        .id_salt("state_table_h")
        .show(ui, |ui| {
            let mut tb = TableBuilder::new(ui)
                .striped(true)
                // 低いパネルでもパネルの中に収める（既定の最低 200 だとはみ出し、最後の行までスクロールできない）
                .min_scrolled_height(0.0)
                .column(Column::auto().at_least(140.0).resizable(true));
            for _ in &cols {
                tb = tb.column(Column::auto().at_least(80.0));
            }
            tb.header(22.0, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new(tr("Value")).color(weak));
                });
                for col in &cols {
                    h.col(|ui| {
                        let name = col.clone().unwrap_or_else(|| tr("Base").to_string());
                        let cur = c.state == *col;
                        let text = if cur {
                            RichText::new(name).strong().color(accent)
                        } else {
                            RichText::new(name)
                        };
                        let hint = match col {
                            Some(n) if c.state_sys.contains_key(n) || c.state_more.contains_key(n) => trf(
                                "Make {} the current state (it also changes system settings such as aperture and fields)",
                                &[n],
                            ),
                            Some(n) => trf("Make {} the current state", &[n]),
                            None => tr("Show the base values (no state)").to_string(),
                        };
                        if ui.selectable_label(cur, text).on_hover_text(hint).clicked() && !cur {
                            pick_state = Some(col.clone());
                        }
                    });
                }
            })
            .body(|mut body| {
                // 系の値を状態で変えたか
                let sys_own = |n: &str| c.state_sys.contains_key(n) || c.state_more.contains_key(n);
                if states.iter().any(|n| sys_own(n)) {
                    body.row(20.0, |mut r| {
                        r.col(|ui| {
                            ui.label(RichText::new(tr("System settings")).color(weak));
                        });
                        for col in &cols {
                            r.col(|ui| {
                                let own = col.as_deref().is_some_and(sys_own);
                                ui.label(if own { tr("changed") } else { "—" });
                            });
                        }
                    });
                }
                for (i, it) in rows.iter().enumerate() {
                    body.row(22.0, |mut r| {
                        r.col(|ui| {
                            ui.label(it.label(c));
                        });
                        for col in &cols {
                            r.col(|ui| {
                                let st = col.as_deref();
                                let own = st.is_some_and(|n| it.own(c, n));
                                let Some(v) = it.get(c, st) else { return };
                                let resp = match v {
                                    Val::Bool(mut b) => {
                                        let r = ui.checkbox(&mut b, "");
                                        if r.changed() {
                                            set = Some((i, col.clone(), Val::Bool(b)));
                                        }
                                        r
                                    }
                                    Val::Num(mut x) => {
                                        let r = ui.add(egui::DragValue::new(&mut x).speed(0.1).max_decimals(6));
                                        if r.changed() {
                                            set = Some((i, col.clone(), Val::Num(x)));
                                        }
                                        r
                                    }
                                    Val::Int(mut k) => {
                                        let r = ui.add(egui::DragValue::new(&mut k).range(1..=999));
                                        if r.changed() {
                                            set = Some((i, col.clone(), Val::Int(k)));
                                        }
                                        r
                                    }
                                    Val::Text(mut t) => {
                                        let r = ui.add(egui::TextEdit::singleline(&mut t).desired_width(70.0));
                                        if r.changed() {
                                            set = Some((i, col.clone(), Val::Text(t)));
                                        }
                                        r
                                    }
                                };
                                if own {
                                    ui.painter().rect_stroke(
                                        resp.rect.expand(1.0),
                                        2.0,
                                        egui::Stroke::new(1.0_f32, accent),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                if let Some(n) = col.as_ref().filter(|_| own) {
                                    resp.context_menu(|ui| {
                                        if ui.button(tr("Use base value")).clicked() {
                                            clear = Some((i, n.clone()));
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                        }
                    });
                }
            });
        });
    if let Some((i, col, v)) = set {
        rows[i].set(c, col.as_deref(), v);
        changed = true;
    }
    if let Some((i, n)) = clear {
        rows[i].clear(c, &n);
        changed = true;
    }
    if let Some(s) = pick_state {
        c.state = s;
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状態で変えた値の行だけを出し、状態の列のセルを変えるとその状態の値、基本に戻すと外れる
    #[test]
    fn rows_follow_state_values() {
        let mut c = Combo::default();
        let mut z = Pick::new(ElemKind::ZmxRange);
        z.path = "a.zmx".into();
        c.picks.push(z);
        c.picks.push(Pick::new(ElemKind::Plate));
        c.states = vec!["Config 1".into(), "Config 2".into()];
        assert!(items(&c, false).is_empty());
        c.picks[0]
            .zmx_mut()
            .state_config
            .insert("Config 2".into(), 2);
        let rows = items(&c, false);
        assert_eq!(rows, vec![Item::Config(vec![0])]);
        rows[0].set(&mut c, Some("Config 1"), Val::Int(1));
        assert!(rows[0].own(&c, "Config 1"));
        assert_eq!(c.picks[0].config_in(Some("Config 2")), 2);
        rows[0].clear(&mut c, "Config 2");
        assert_eq!(c.picks[0].config_in(Some("Config 2")), 1);
        // 全部: 要素の ON/OFF・後の空気・構成
        let all = items(&c, true);
        assert!(all.contains(&Item::On(vec![1])) && all.contains(&Item::Air(vec![1])));
        Item::Air(vec![1]).set(&mut c, Some("Config 2"), Val::Num(7.0));
        assert_eq!(c.picks[1].air_in(Some("Config 2")), 7.0);
        assert_eq!(c.picks[1].air_in(None), c.picks[1].air);
    }
}
