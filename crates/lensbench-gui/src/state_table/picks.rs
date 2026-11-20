//! 多重構成のピックアップ（OpticStudio の構成のピックアップ: 構成 j の値 × 倍率 ＋ オフセット）。
//! 行の持ち主（要素か組合せ）に [`McPick`] を持ち、値は求めて各状態に書く

use super::*;
use crate::combo::McPick;
use std::collections::BTreeMap;

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
        Item::Comment(k) => (None, format!("comment:{k}")),
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
        (None, "comment") => Item::Comment(num(rest)?),
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

/// ピックアップの元: 状態 from（空なら基本）と行 row（空なら同じ行。[`row_key`]）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Src {
    pub from: String,
    pub row: String,
}

impl Src {
    /// 同じ行の、状態 from（空なら基本）の値
    pub(crate) fn conf(from: &str) -> Src {
        Src {
            from: from.to_string(),
            row: String::new(),
        }
    }
}

/// 状態 to の行 it を、元 src の値 × scale ＋ offset にする（前のピックアップは置き換える）
pub(super) fn set_pick(c: &mut Combo, it: &Item, to: &str, src: &Src, scale: f64, offset: f64) {
    let (loc, key) = key_of(it);
    if let Some(v) = list_mut(c, &loc) {
        v.retain(|m| !(m.key == key && m.to == to));
        v.push(McPick {
            key,
            to: to.to_string(),
            from: src.from.clone(),
            row: src.row.clone(),
            scale,
            offset,
        });
    }
}

/// 行 it のピックアップの元にできる行 r の名前（同じ行は空。持ち主の違う要素の行は元にできないので None）
pub(super) fn row_key(it: &Item, r: &Item) -> Option<String> {
    if it == r {
        return Some(String::new());
    }
    // コメント行は別の行と行き来しない
    if matches!(it, Item::Comment(_)) || matches!(r, Item::Comment(_)) {
        return None;
    }
    let ((a, _), (b, k)) = (key_of(it), key_of(r));
    (b.is_none() || a == b).then_some(k)
}

/// ピックアップ m（持ち主 owner）の元の行
pub(super) fn src_item(owner: Option<&Loc>, m: &McPick) -> Option<Item> {
    if m.row.is_empty() {
        return item_of(owner, &m.key);
    }
    row_item(owner, &m.row)
}

/// 持ち主 owner の行の名前 row（同じ持ち主か組合せの行）の行
pub(super) fn row_item(owner: Option<&Loc>, row: &str) -> Option<Item> {
    item_of(owner, row).or_else(|| item_of(None, row))
}

/// 状態 to の行 it を、状態 from（空なら基本）の行 src から取ると輪になるか（src の値がピックアップをたどって
/// 状態 to の it から来る。自分自身も輪）
pub(super) fn would_loop(c: &Combo, it: &Item, to: &str, src: &Item, from: &str) -> bool {
    let (mut cur, mut st) = (src.clone(), from.to_string());
    // 行と状態ごとにピックアップは 1 つなので、たどる道は 1 本。長すぎる道も輪とみなす
    for _ in 0..256 {
        if st.is_empty() {
            return false;
        }
        if cur == *it && st == to {
            return true;
        }
        let Some(m) = pick_of(c, &cur, &st) else {
            return false;
        };
        let (owner, _) = key_of(&cur);
        let Some(s) = src_item(owner.as_ref(), m) else {
            return false;
        };
        (cur, st) = (s, m.from.clone());
    }
    true
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

/// 番号で指す行（群の移動 `shift:g`・`zshift:g`）の番号を map で付け替える（ピックアップの行と元の行の両方）。
/// 消えた番号（map が None）を指すピックアップは外す
pub(super) fn remap_index_picks(
    v: &mut Vec<McPick>,
    head: &str,
    map: &dyn Fn(usize) -> Option<usize>,
) {
    let re = |k: &str| -> Option<Option<String>> {
        let i = k
            .strip_prefix(head)?
            .strip_prefix(':')?
            .parse::<usize>()
            .ok()?;
        Some(map(i).map(|j| format!("{head}:{j}")))
    };
    v.retain_mut(|m| {
        for s in [&mut m.key, &mut m.row] {
            match re(s) {
                None => {}
                Some(None) => return false,
                Some(Some(n)) => *s = n,
            }
        }
        true
    });
}

/// 番号で指す行の名前 `head:i` の i
fn index_of(k: &str, head: &str) -> Option<usize> {
    k.strip_prefix(head)?.strip_prefix(':')?.parse().ok()
}

/// 行の順位の鍵（行の名前）を f で付け替える（f が None なら変えない、Some(None) なら外す）
pub(crate) fn remap_rank(
    m: &mut BTreeMap<String, u32>,
    f: &dyn Fn(&str) -> Option<Option<String>>,
) {
    *m = std::mem::take(m)
        .into_iter()
        .filter_map(|(k, r)| match f(&k) {
            None => Some((k, r)),
            Some(n) => n.map(|n| (n, r)),
        })
        .collect();
}

/// 番号で指す行（`head:i`）の順位の鍵を map で付け替える（消えた番号は外す）
pub(super) fn remap_index_rank(
    m: &mut BTreeMap<String, u32>,
    head: &str,
    map: &dyn Fn(usize) -> Option<usize>,
) {
    remap_rank(m, &|k| {
        index_of(k, head).map(|i| map(i).map(|j| format!("{head}:{j}")))
    });
}

fn rank_map<'a>(c: &'a Combo, loc: &Option<Loc>) -> Option<&'a BTreeMap<String, u32>> {
    match loc {
        None => Some(&c.mc_rank),
        Some(l) => c.get(l).map(|p| &p.mc_rank),
    }
}

/// 表の今の並び rows を行の順位にする（前の順位は全部捨てる）
pub(super) fn set_order(c: &mut Combo, rows: &[Item]) {
    c.mc_rank.clear();
    for l in c.all() {
        if let Some(p) = c.get_mut(&l) {
            p.mc_rank.clear();
        }
    }
    for (r, it) in rows.iter().enumerate() {
        let (loc, key) = key_of(it);
        let m = match &loc {
            None => Some(&mut c.mc_rank),
            Some(l) => c.get_mut(l).map(|p| &mut p.mc_rank),
        };
        if let Some(m) = m {
            m.insert(key, r as u32);
        }
    }
}

/// 行を順位の順に並べる（順位の無い行は元の順で後ろ）
pub(super) fn sort_rows(c: &Combo, rows: &mut [Item]) {
    let rank = |it: &Item| {
        let (loc, key) = key_of(it);
        rank_map(c, &loc)
            .and_then(|m| m.get(&key))
            .copied()
            .unwrap_or(u32::MAX)
    };
    rows.sort_by_cached_key(rank);
}

/// 保存の形: 順位の順に（要素の id（組合せの行は None）, 行の名前）
pub(crate) fn order_json(c: &Combo) -> Vec<(Option<String>, String)> {
    let mut v: Vec<(u32, Option<String>, String)> = c
        .mc_rank
        .iter()
        .map(|(k, r)| (*r, None, k.clone()))
        .collect();
    for (l, id) in c.ids_all() {
        if let Some(p) = c.get(&l) {
            v.extend(
                p.mc_rank
                    .iter()
                    .map(|(k, r)| (*r, Some(id.clone()), k.clone())),
            );
        }
    }
    v.sort();
    v.into_iter().map(|(_, id, k)| (id, k)).collect()
}

/// [`order_json`] の形から順位を戻す（無い要素の行は捨てる）
pub(crate) fn order_from_json(c: &mut Combo, v: &[(Option<String>, String)]) {
    let ids: HashMap<String, Loc> = c.ids_all().into_iter().map(|(l, id)| (id, l)).collect();
    for (r, (id, key)) in v.iter().enumerate() {
        let m = match id {
            None => Some(&mut c.mc_rank),
            Some(id) => ids
                .get(id)
                .and_then(|l| c.get_mut(l))
                .map(|p| &mut p.mc_rank),
        };
        if let Some(m) = m {
            m.insert(key.clone(), r as u32);
        }
    }
}

/// コメント行 k を消したとき、後ろのコメント行の順位の番号を詰める
pub(super) fn shift_comment_order(c: &mut Combo, k: usize) {
    remap_index_rank(&mut c.mc_rank, "comment", &|j| match j.cmp(&k) {
        std::cmp::Ordering::Less => Some(j),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(j - 1),
    });
}

/// コメント行 k を消したとき、後ろのコメント行のピックアップの番号を詰める
pub(super) fn shift_comment_picks(c: &mut Combo, k: usize) {
    for m in &mut c.mc_picks {
        if let Some(j) = m
            .key
            .strip_prefix("comment:")
            .and_then(|j| j.parse::<usize>().ok())
            .filter(|&j| j > k)
        {
            m.key = format!("comment:{}", j - 1);
        }
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
    // 1 回りで連なりが 1 段は進むので、ピックアップの数 + 1 回りで端まで届く（並びの順に依らない）
    for _ in 0..=all.len() {
        let mut any = false;
        for (loc, m) in &all {
            let Some(it) = item_of(loc.as_ref(), &m.key) else {
                continue;
            };
            if !c.states.contains(&m.to) || it.locked(c) {
                continue;
            }
            let from = (!m.from.is_empty()).then_some(m.from.as_str());
            let Some(src) = src_item(loc.as_ref(), m) else {
                continue;
            };
            // 輪になったピックアップ（前の版で作ったものなど）は当てない
            if would_loop(c, &it, &m.to, &src, &m.from) {
                continue;
            }
            let Some(v) = src.get(c, from, cx).and_then(|v| picked(m, v)) else {
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
            set_pick(c, it, &name, &Src::conf(from), 1.0, 0.0);
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

/// 取り込みの群 i を消した後: 群の移動の行は群を番号で指すので、後ろの群の番号を詰める（消した群のピックアップは外す）
pub(crate) fn zgroup_removed(p: &mut crate::combo::Pick, i: usize) {
    let map = |j: usize| match j.cmp(&i) {
        std::cmp::Ordering::Less => Some(j),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(j - 1),
    };
    remap_index_picks(&mut p.mc_picks, "zshift", &map);
    remap_index_rank(&mut p.mc_rank, "zshift", &map);
}

/// 使わなくなった既定のままの群（名前を打つ途中の群など）を消し、群の移動の行（群を番号で指す）を今の番号へ付け替える
pub(crate) fn drop_unused_groups(combo: &mut Combo, names: &[String]) {
    let before: Vec<String> = combo.groups.iter().map(|d| d.name.clone()).collect();
    combo.groups.retain(|d| {
        names.contains(&d.name) || !d.role.is_empty() || d.shift != 0.0 || !d.state_shift.is_empty()
    });
    if combo.groups.len() == before.len() {
        return;
    }
    let after: Vec<String> = combo.groups.iter().map(|d| d.name.clone()).collect();
    let map = |i: usize| {
        before
            .get(i)
            .and_then(|n| after.iter().position(|x| x == n))
    };
    remap_index_picks(&mut combo.mc_picks, "shift", &map);
    remap_index_rank(&mut combo.mc_rank, "shift", &map);
    for l in combo.all() {
        if let Some(p) = combo.get_mut(&l) {
            remap_index_picks(&mut p.mc_picks, "shift", &map);
        }
    }
}
