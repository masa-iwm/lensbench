//! 多重構成のピックアップ（OpticStudio の構成のピックアップ: 構成 j の値 × 倍率 ＋ オフセット）。
//! 行の持ち主（要素か組合せ）に [`McPick`] を持ち、値は求めて各状態に書く

use super::*;
use crate::combo::McPick;

fn sys_key(q: SysQ) -> String {
    match q {
        SysQ::Wave(i) => format!("wave:{i}"),
        SysQ::WaveW(i) => format!("wwt:{i}"),
        SysQ::Primary => "primary".into(),
        SysQ::ApType => "atype".into(),
        SysQ::ApValue => "aper".into(),
        SysQ::FieldType => "ftype".into(),
        SysQ::FieldX(i) => format!("fx:{i}"),
        SysQ::FieldY(i) => format!("fy:{i}"),
        SysQ::FieldW(i) => format!("fwt:{i}"),
        SysQ::ObjDist => "objdist".into(),
        SysQ::Telecentric => "tele".into(),
    }
}

fn sys_of(k: &str) -> Option<SysQ> {
    let (h, i) = match k.split_once(':') {
        Some((h, i)) => (h, i.parse::<usize>().ok()),
        None => (k, None),
    };
    Some(match (h, i) {
        ("wave", Some(i)) => SysQ::Wave(i),
        ("wwt", Some(i)) => SysQ::WaveW(i),
        ("primary", None) => SysQ::Primary,
        ("atype", None) => SysQ::ApType,
        ("aper", None) => SysQ::ApValue,
        ("ftype", None) => SysQ::FieldType,
        ("fx", Some(i)) => SysQ::FieldX(i),
        ("fy", Some(i)) => SysQ::FieldY(i),
        ("fwt", Some(i)) => SysQ::FieldW(i),
        ("objdist", None) => SysQ::ObjDist,
        ("tele", None) => SysQ::Telecentric,
        _ => return None,
    })
}

/// 行の持ち主（要素の場所。組合せの行は None）と、持ち主の中での行の名前
pub(super) fn key_of(it: &Item) -> (Option<Loc>, String) {
    match it {
        Item::Sys(q) => (None, format!("sys:{}", sys_key(*q))),
        Item::Stop => (None, "stop".into()),
        Item::Path => (None, "path".into()),
        Item::Shift(g) => (None, format!("shift:{g}")),
        Item::Face(f) => (
            Some(f.loc.clone()),
            match f.idx {
                Some(i) => format!("face:{}:{}:{i}", f.surf, f.field),
                None => format!("face:{}:{}", f.surf, f.field),
            },
        ),
        Item::On(l) => (Some(l.clone()), "on".into()),
        Item::Air(l) => (Some(l.clone()), "air".into()),
        Item::Config(l) => (Some(l.clone()), "config".into()),
        Item::Place(l, k) => (Some(l.clone()), format!("place:{k}")),
        Item::ZShift(l, g) => (Some(l.clone()), format!("zshift:{g}")),
    }
}

/// [`key_of`] の逆
pub(super) fn item_of(loc: Option<&Loc>, key: &str) -> Option<Item> {
    let (h, rest) = key.split_once(':').unwrap_or((key, ""));
    let num = |s: &str| s.parse::<usize>().ok();
    Some(match (loc, h) {
        (None, "sys") => Item::Sys(sys_of(rest)?),
        (None, "stop") => Item::Stop,
        (None, "path") => Item::Path,
        (None, "shift") => Item::Shift(num(rest)?),
        (Some(l), "face") => {
            let mut it = rest.split(':');
            let surf = num(it.next()?)?;
            let field = it.next()?.to_string();
            let idx = match it.next() {
                Some(i) => Some(num(i)?),
                None => None,
            };
            Item::Face(Face {
                loc: l.clone(),
                surf,
                field,
                idx,
            })
        }
        (Some(l), "on") => Item::On(l.clone()),
        (Some(l), "air") => Item::Air(l.clone()),
        (Some(l), "config") => Item::Config(l.clone()),
        (Some(l), "place") => Item::Place(l.clone(), num(rest)?),
        (Some(l), "zshift") => Item::ZShift(l.clone(), num(rest)?),
        _ => return None,
    })
}

fn list<'a>(c: &'a Combo, loc: &Option<Loc>) -> Option<&'a Vec<McPick>> {
    match loc {
        None => Some(&c.mc_picks),
        Some(l) => c.get(l).map(|p| &p.mc_picks),
    }
}

fn list_mut<'a>(c: &'a mut Combo, loc: &Option<Loc>) -> Option<&'a mut Vec<McPick>> {
    match loc {
        None => Some(&mut c.mc_picks),
        Some(l) => c.get_mut(l).map(|p| &mut p.mc_picks),
    }
}

/// 状態 to の行 it のピックアップ
pub(super) fn pick_of<'a>(c: &'a Combo, it: &Item, to: &str) -> Option<&'a McPick> {
    let (loc, key) = key_of(it);
    list(c, &loc)?.iter().find(|m| m.key == key && m.to == to)
}

/// 状態 to の行 it を、状態 from（空なら基本）の値 × scale ＋ offset にする（前のピックアップは置き換える）
pub(super) fn set_pick(c: &mut Combo, it: &Item, to: &str, from: &str, scale: f64, offset: f64) {
    let (loc, key) = key_of(it);
    if let Some(v) = list_mut(c, &loc) {
        v.retain(|m| !(m.key == key && m.to == to));
        v.push(McPick {
            key,
            to: to.to_string(),
            from: from.to_string(),
            scale,
            offset,
        });
    }
}

/// 状態 to の行 it のピックアップをやめる（値はそのまま）
pub(super) fn remove_pick(c: &mut Combo, it: &Item, to: &str) {
    let (loc, key) = key_of(it);
    if let Some(v) = list_mut(c, &loc) {
        v.retain(|m| !(m.key == key && m.to == to));
    }
}

/// 行 it のピックアップを全部やめる（行を消すとき）
pub(super) fn remove_row_picks(c: &mut Combo, it: &Item) {
    let (loc, key) = key_of(it);
    if let Some(v) = list_mut(c, &loc) {
        v.retain(|m| m.key != key);
    }
}

/// ピックアップのある行（表に出す）
pub(super) fn rows(c: &Combo) -> Vec<Item> {
    let mut out: Vec<Item> = c
        .mc_picks
        .iter()
        .filter_map(|m| item_of(None, &m.key))
        .collect();
    for l in c.all() {
        if let Some(p) = c.get(&l) {
            out.extend(p.mc_picks.iter().filter_map(|m| item_of(Some(&l), &m.key)));
        }
    }
    out.dedup();
    out
}

/// ピックアップの値 v（状態 from の値）から状態 to の値
fn picked(m: &McPick, v: Val) -> Option<Val> {
    match v {
        Val::Num(x) => Some(Val::Num(x * m.scale + m.offset)),
        Val::Unknown => None,
        v => Some(v),
    }
}

/// ピックアップを全部当てる（ピックアップの先からのピックアップもあるので、変わらなくなるまで数回）。変えたか
pub(super) fn apply(c: &mut Combo, cx: &Cx) -> bool {
    let mut all: Vec<(Option<Loc>, McPick)> =
        c.mc_picks.iter().map(|m| (None, m.clone())).collect();
    for l in c.all() {
        if let Some(p) = c.get(&l) {
            all.extend(p.mc_picks.iter().map(|m| (Some(l.clone()), m.clone())));
        }
    }
    let mut changed = false;
    for _ in 0..4 {
        let mut any = false;
        for (loc, m) in &all {
            let Some(it) = item_of(loc.as_ref(), &m.key) else {
                continue;
            };
            if !c.states.contains(&m.to) || it.locked(c) {
                continue;
            }
            let from = (!m.from.is_empty()).then_some(m.from.as_str());
            let Some(v) = it.get(c, from, cx).and_then(|v| picked(m, v)) else {
                continue;
            };
            let before = it.get(c, Some(&m.to), cx);
            if before.as_ref() != Some(&v) {
                // 書けない値（範囲の外・読めない基本の値など）もあるので、読み直して変わったときだけ変えたとする
                // 書けなかったときは書く前に戻す（システムの値の写しなどを残さない）
                let saved = c.clone();
                it.set(c, Some(&m.to), v, cx);
                if it.get(c, Some(&m.to), cx) == before {
                    *c = saved;
                } else {
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
        changed = true;
    }
    changed
}

/// 状態 from を写し、どの行も from のピックアップにした状態を from の右に足す（OpticStudio の
/// Insert Config with Pickups）。足した状態の名前
pub(super) fn add_state_with_picks(c: &mut Combo, from: &str, rows: &[Item]) -> Option<String> {
    let name = c.free_state_name(from);
    if !c.add_state(&name, Some(from)) {
        return None;
    }
    for it in rows {
        if !it.locked(c) {
            set_pick(c, it, &name, from, 1.0, 0.0);
        }
    }
    Some(name)
}

/// 保存の形: 要素の id（組合せの行は ""）→ ピックアップの並び
pub(crate) fn to_json(c: &Combo) -> Option<Value> {
    let mut m = serde_json::Map::new();
    if !c.mc_picks.is_empty() {
        m.insert(String::new(), serde_json::to_value(&c.mc_picks).ok()?);
    }
    for (loc, id) in c.ids_all() {
        if let Some(p) = c.get(&loc).filter(|p| !p.mc_picks.is_empty()) {
            m.insert(id, serde_json::to_value(&p.mc_picks).ok()?);
        }
    }
    (!m.is_empty()).then_some(Value::Object(m))
}

/// [`to_json`] の形を組合せに戻す（無い id・読めない並びは飛ばす）
pub(crate) fn from_json(c: &mut Combo, v: Option<&Value>) {
    let Some(m) = v.and_then(Value::as_object) else {
        return;
    };
    let read = |v: &Value| serde_json::from_value::<Vec<McPick>>(v.clone()).ok();
    if let Some(p) = m.get("").and_then(read) {
        c.mc_picks = p;
    }
    for (loc, id) in c.ids_all() {
        if let (Some(v), Some(p)) = (m.get(&id).and_then(read), c.get_mut(&loc)) {
            p.mc_picks = v;
        }
    }
}
