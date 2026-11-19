//! 組合せの状態の表（OpticStudio の多重構成のエディタに当たる表）。列 = 基本と状態（構成）、行 = 状態で変えられる値:
//! 系の値（波長・波長の重み・主波長・開口の種類と値・視野の種類・視野・物体距離）、要素の ON/OFF・後の空気・偏心と傾き・
//! .zmx の取り込みの構成・群の移動・BS の経路、要素の面の値（曲率半径・厚さ・円錐係数・非球面係数・半直径・無視・材料など、
//! 状態の面の上書き [`Pick::state_edits`]）。状態の列のセルを変えるとその状態の値になり、右クリックの「基本の値に戻す」で
//! 外れる。行は「+ 行」で足し（面の行は面の表の右クリックからも）、行の名前の右クリックで消す。構成（状態）は列の見出しの
//! 右クリックで写して足す・名前を変える・消す

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::combo::{Combo, ElemKind, Pick};
use crate::combo_tree::Loc;
use crate::i18n::{tr, trf};
use crate::sysset::{ApertureField, ApertureKind, Choice, FieldKind, SysSnap};
use crate::theme;

/// 表の表示の設定
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    /// 状態で変えていない値の行も出す
    pub all: bool,
    /// 足した行（どの状態も基本と同じ値の系の値の行も出す。保存はしない）
    pub pinned: Vec<Item>,
    /// 「+ 行」の面の行の選び（要素の番号・面・値）
    pub add_el: usize,
    pub add_surf: usize,
    pub add_field: usize,
    /// 名前を変えている構成（元の名前・新しい名前）
    pub renaming: Option<(String, String)>,
    /// 名前を変えられなかった理由（次に名前を変えるか構成を足すまで出す）
    pub rename_err: Option<String>,
    /// 面の値の基本の値（今の系から読めたとき覚えておく。今の状態が上書きしているときに使う）
    pub base_cache: HashMap<Face, Value>,
}

/// 要素の面の値（多重構成の面の行）。面は要素の面のうち座標ブレークを除いた番号、field は面の値の名前
/// （[`lensbench_core::edit::SURF_FIELDS`]）、idx は並びの値（asph）の何番目か
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Face {
    pub loc: Loc,
    pub surf: usize,
    pub field: String,
    pub idx: Option<usize>,
}

/// 面の行にできる値（名前・並びか）
pub(crate) const FACE_FIELDS: &[(&str, bool)] = &[
    ("r", false),
    ("t", false),
    ("k", false),
    ("asph", true),
    ("semi", false),
    ("ignore", false),
    ("material", false),
    ("coating", false),
];

/// 状態ごとに変えられる系の値（[`SysSnap`] の項目）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SysQ {
    Wave(usize),
    WaveW(usize),
    Primary,
    ApType,
    ApValue,
    FieldType,
    FieldX(usize),
    FieldY(usize),
    FieldW(usize),
    ObjDist,
    Telecentric,
}

/// 表の 1 行が指す値
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    Sys(SysQ),
    /// 絞りの面（多重構成の STPS）
    Stop,
    Face(Face),
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
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Bool(bool),
    Num(f64),
    Int(usize),
    Text(String),
    /// 選ぶ値（番号と並び）
    Choice(usize, &'static [&'static str]),
    /// 絞りの面（要素の場所と要素の面の番号。None は先頭に自動で置く絞り）
    Stop(Option<(Loc, usize)>),
    /// 読めない（今の系に無い面の基本の値など）。打ち込めば値になる
    Unknown,
}

/// 面の値を読む: 今の系の面の値（基本の値。読めなければ None）
pub(crate) type FaceNow<'a> = &'a dyn Fn(&Face) -> Option<Value>;

const AP_KEYS: [&str; 6] = ["epd", "fnum", "na", "float", "pwfn", "cone"];
const FT_KEYS: [&str; 5] = [
    "angle",
    "object_height",
    "paraxial_image_height",
    "real_image_height",
    "solve",
];

fn snap_in(c: &Combo, st: Option<&str>) -> SysSnap {
    st.and_then(|n| c.state_sys.get(n).cloned())
        .unwrap_or_else(|| c.snap())
}

impl SysQ {
    fn label(self) -> String {
        match self {
            SysQ::Wave(i) => trf("Wavelength {} [µm]", &[&(i + 1)]),
            SysQ::WaveW(i) => trf("Wavelength {} weight", &[&(i + 1)]),
            SysQ::Primary => tr("Primary wavelength").into(),
            SysQ::ApType => tr("Aperture type").into(),
            SysQ::ApValue => tr("Aperture value").into(),
            SysQ::FieldType => tr("Field type").into(),
            SysQ::FieldX(i) => trf("Field {} x", &[&(i + 1)]),
            SysQ::FieldY(i) => trf("Field {} y", &[&(i + 1)]),
            SysQ::FieldW(i) => trf("Field {} weight", &[&(i + 1)]),
            SysQ::ObjDist => tr("Object distance").into(),
            SysQ::Telecentric => tr("Object space telecentric").into(),
        }
    }

    fn get(self, s: &SysSnap) -> Option<Val> {
        Some(match self {
            SysQ::Wave(i) => Val::Num(s.waves.get(i)?.um),
            SysQ::WaveW(i) => Val::Num(s.waves.get(i)?.w),
            SysQ::Primary => Val::Int(s.primary),
            SysQ::ApType => Val::Choice(
                ApertureKind::ALL.iter().position(|k| *k == s.aperture)?,
                &AP_KEYS,
            ),
            SysQ::ApValue if s.aperture.has_value() => Val::Num(s.ap_value),
            SysQ::ApValue => Val::Num(s.epd),
            SysQ::FieldType => Val::Choice(
                FieldKind::ALL.iter().position(|k| *k == s.field_kind)?,
                &FT_KEYS,
            ),
            SysQ::FieldX(i) => Val::Num(s.fields.get(i)?.x),
            SysQ::FieldY(i) => Val::Num(s.fields.get(i)?.y),
            SysQ::FieldW(i) => Val::Num(s.fields.get(i)?.w),
            SysQ::ObjDist => Val::Text(match s.object_distance {
                Some(d) => crate::json_view::fmt_num(d),
                None => "inf".into(),
            }),
            SysQ::Telecentric => Val::Bool(s.telecentric),
        })
    }

    /// s の値を v に（変えられなければ何もしない）
    fn set(self, s: &mut SysSnap, v: &Val) {
        match (self, v) {
            (SysQ::Wave(i), Val::Num(x)) if *x > 0.0 => {
                if let Some(w) = s.waves.get_mut(i) {
                    w.um = *x;
                }
            }
            (SysQ::WaveW(i), Val::Num(x)) if *x >= 0.0 => {
                if let Some(w) = s.waves.get_mut(i) {
                    w.w = *x;
                }
            }
            (SysQ::Primary, Val::Int(k)) => s.primary = (*k).clamp(1, s.waves.len().max(1)),
            (SysQ::ApType, Val::Choice(i, _)) => {
                if let Some(&k) = ApertureKind::ALL.get(*i) {
                    if k != s.aperture && k.has_value() {
                        s.ap_value = k.initial();
                    }
                    s.aperture = k;
                }
            }
            (SysQ::ApValue, Val::Num(x)) if *x > 0.0 => {
                if s.aperture.has_value() {
                    s.ap_value = *x;
                } else {
                    s.epd = *x;
                }
            }
            (SysQ::FieldType, Val::Choice(i, _)) => {
                if let Some(&k) = FieldKind::ALL.get(*i) {
                    s.field_kind = k;
                }
            }
            (SysQ::FieldX(i), Val::Num(x)) => {
                if let Some(f) = s.fields.get_mut(i) {
                    f.x = *x;
                }
            }
            (SysQ::FieldY(i), Val::Num(x)) => {
                if let Some(f) = s.fields.get_mut(i) {
                    f.y = *x;
                }
            }
            (SysQ::FieldW(i), Val::Num(x)) if *x >= 0.0 => {
                if let Some(f) = s.fields.get_mut(i) {
                    f.w = *x;
                }
            }
            (SysQ::Telecentric, Val::Bool(b)) => s.telecentric = *b,
            (SysQ::ObjDist, Val::Text(t)) => {
                let t = t.trim();
                if t.eq_ignore_ascii_case("inf") || t == "∞" || t.is_empty() {
                    s.object_distance = None;
                } else if let Ok(x) = t.parse::<f64>() {
                    s.object_distance = Some(x);
                }
            }
            _ => {}
        }
    }

    /// 系の値の行の全部（波長・視野は基本と状態の最大の数まで）
    fn all(c: &Combo) -> Vec<SysQ> {
        let snaps: Vec<SysSnap> = std::iter::once(c.snap())
            .chain(c.state_sys.values().cloned())
            .collect();
        let nw = snaps.iter().map(|s| s.waves.len()).max().unwrap_or(0);
        let nf = snaps.iter().map(|s| s.fields.len()).max().unwrap_or(0);
        let mut out = Vec::new();
        out.extend((0..nw).map(SysQ::Wave));
        out.extend((0..nw).map(SysQ::WaveW));
        out.push(SysQ::Primary);
        out.extend([SysQ::ApType, SysQ::ApValue, SysQ::FieldType]);
        for i in 0..nf {
            out.extend([SysQ::FieldX(i), SysQ::FieldY(i), SysQ::FieldW(i)]);
        }
        out.extend([SysQ::ObjDist, SysQ::Telecentric]);
        out
    }
}

/// 面の値の JSON を表の値に（field で型を決める）
fn face_val(field: &str, idx: Option<usize>, v: Option<&Value>) -> Val {
    let Some(v) = v else { return Val::Unknown };
    let v = match idx {
        Some(i) => match v.as_array() {
            Some(a) => a.get(i).unwrap_or(&Value::Null),
            None => return Val::Unknown,
        },
        None => v,
    };
    match field {
        "material" | "coating" | "comment" => Val::Text(v.as_str().unwrap_or_default().to_string()),
        "ignore" => Val::Bool(v == true),
        _ => match v.as_f64() {
            Some(x) => Val::Num(x),
            None if v.is_null() && idx.is_some() => Val::Num(0.0),
            None => match v.get("$f").and_then(Value::as_str) {
                Some("inf") => Val::Num(f64::INFINITY),
                Some("-inf") => Val::Num(f64::NEG_INFINITY),
                _ => Val::Unknown,
            },
        },
    }
}

/// 表の値を面の値の JSON に。並び（idx）は前の並び old の idx 番目を変える
fn face_json(v: &Val, idx: Option<usize>, old: Option<&Value>) -> Option<Value> {
    let x = match v {
        Val::Num(x) if x.is_infinite() => json!({ "$f": if *x > 0.0 { "inf" } else { "-inf" } }),
        Val::Num(x) => json!(x),
        Val::Bool(b) => json!(b),
        Val::Text(t) => json!(t.trim()),
        _ => return None,
    };
    Some(match idx {
        // 前の並びが分からなければ（基本の値が読めない）ほかの番号を消さないよう書かない
        Some(i) => {
            let mut a: Vec<Value> = old?.as_array()?.clone();
            if a.len() <= i {
                a.resize(i + 1, json!(0.0));
            }
            a[i] = x;
            Value::Array(a)
        }
        None => x,
    })
}

impl Face {
    fn label(&self, c: &Combo) -> String {
        let el = c.get(&self.loc).map(Pick::label).unwrap_or_default();
        let what = match (self.field.as_str(), self.idx) {
            ("asph", Some(i)) => trf("asphere {}", &[&(i + 1)]),
            ("r", _) => tr("radius").into(),
            ("t", _) => tr("thickness").into(),
            ("k", _) => tr("conic").into(),
            ("semi", _) => tr("semi-diameter").into(),
            ("ignore", _) => tr("ignore").into(),
            ("material", _) => tr("material").into(),
            ("coating", _) => tr("coating").into(),
            (f, _) => f.to_string(),
        };
        trf("{} surface {} {}", &[&el, &(self.surf + 1), &what])
    }

    /// 基本の値（基本の上書き、無ければ今の系の値か覚えた値）。今の状態がこの値を上書きしているときは
    /// 今の系の値は基本の値ではないので読まない（覚えた値が無ければ分からない）
    fn base(
        &self,
        c: &Combo,
        p: &Pick,
        now: FaceNow,
        cache: &HashMap<Face, Value>,
    ) -> Option<Value> {
        let key = Face {
            idx: None,
            ..self.clone()
        };
        let cur_own = c
            .active_state()
            .is_some_and(|n| p.state_edit(n, self.surf, &self.field).is_some());
        p.base_edit(self.surf, &self.field)
            .cloned()
            .or_else(|| if cur_own { None } else { now(&key) })
            .or_else(|| cache.get(&key).cloned())
    }

    /// 状態 st（None は基本）の値の JSON（並びは並びのまま）
    fn raw(
        &self,
        c: &Combo,
        st: Option<&str>,
        now: FaceNow,
        cache: &HashMap<Face, Value>,
    ) -> Option<Value> {
        let p = c.get(&self.loc)?;
        st.and_then(|n| p.state_edit(n, self.surf, &self.field).cloned())
            .or_else(|| self.base(c, p, now, cache))
    }
}

/// 表を描くときに読むもの
struct Cx<'a> {
    now: FaceNow<'a>,
    cache: &'a HashMap<Face, Value>,
}

/// 絞りの面の表示（要素の名前と面の番号）
fn stop_label(c: &Combo, at: &Option<(Loc, usize)>) -> String {
    match at {
        Some((l, i)) => trf(
            "{} surface {}",
            &[&c.get(l).map(Pick::label).unwrap_or_default(), &(i + 1)],
        ),
        None => tr("Before the first element (automatic)").into(),
    }
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
            Item::Sys(q) => q.label(),
            Item::Stop => tr("Stop surface").into(),
            Item::Face(f) => f.label(c),
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

    fn get(&self, c: &Combo, st: Option<&str>, cx: &Cx) -> Option<Val> {
        Some(match self {
            Item::Sys(q) => q.get(&snap_in(c, st))?,
            Item::Stop => Val::Stop(c.stop_in(st)),
            Item::Face(f) => face_val(&f.field, f.idx, f.raw(c, st, cx.now, cx.cache).as_ref()),
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
            Item::Sys(q) => c
                .state_sys
                .get(n)
                .is_some_and(|s| q.get(s) != q.get(&c.snap())),
            Item::Stop => c
                .all()
                .iter()
                .any(|l| c.get(l).is_some_and(|p| p.state_stop.contains_key(n))),
            Item::Face(f) => c
                .get(&f.loc)
                .is_some_and(|p| p.state_edit(n, f.surf, &f.field).is_some()),
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
    fn set(&self, c: &mut Combo, st: Option<&str>, v: Val, cx: &Cx) {
        let key = st.map(str::to_string);
        match (self, v) {
            (Item::Sys(q), v) => {
                let mut s = snap_in(c, st);
                q.set(&mut s, &v);
                match key {
                    Some(n) => {
                        c.state_sys.insert(n, s);
                    }
                    None => c.apply_snap(&s),
                }
            }
            (Item::Stop, Val::Stop(at)) => match st {
                Some(n) if at == c.base_stop() => c.set_state_stop(n, None),
                Some(n) => c.set_state_stop(n, at),
                None => c.set_stop_face(at),
            },
            (Item::Face(f), v) => {
                if c.get(&f.loc).is_none_or(|p| p.locked) {
                    return;
                }
                let old = f.raw(c, st, cx.now, cx.cache);
                let Some(x) = face_json(&v, f.idx, old.as_ref()) else {
                    return;
                };
                if let Some(p) = c.get_mut(&f.loc) {
                    match st {
                        Some(n) => p.set_state_edit(n, f.surf, &f.field, x),
                        None => p.write_face(f.surf, &f.field, x),
                    }
                }
            }
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
            Item::Sys(q) => {
                let base = c.snap();
                if let (Some(s), Some(v)) = (c.state_sys.get_mut(n), q.get(&base)) {
                    q.set(s, &v);
                    if *s == base {
                        c.state_sys.remove(n);
                    }
                }
            }
            Item::Face(f) => {
                if let Some(p) = c.get_mut(&f.loc) {
                    p.clear_state_edit(n, f.surf, &f.field);
                }
            }
            Item::Stop => c.set_state_stop(n, None),
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

/// 要素 p（場所 l）の面の行: どれかの状態が上書きしている面の値（asph は値の入った番号ごと）
fn face_items(l: &Loc, p: &Pick) -> Vec<Face> {
    let mut out: Vec<Face> = Vec::new();
    for e in p.state_edits.values().flatten() {
        let idxs: Vec<Option<usize>> = if e.field == "asph" {
            let n = p
                .state_edits
                .values()
                .flatten()
                .filter(|x| x.surface == e.surface && x.field == "asph")
                .filter_map(|x| x.value.as_array().map(Vec::len))
                .max()
                .unwrap_or(0);
            (0..n)
                .filter(|&i| {
                    p.state_edits.values().flatten().any(|x| {
                        x.surface == e.surface
                            && x.field == "asph"
                            && x.value
                                .get(i)
                                .and_then(Value::as_f64)
                                .is_some_and(|v| v != 0.0)
                    })
                })
                .map(Some)
                .collect()
        } else {
            vec![None]
        };
        for idx in idxs {
            let f = Face {
                loc: l.clone(),
                surf: e.surface,
                field: e.field.clone(),
                idx,
            };
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out.sort_by(|a, b| (a.surf, &a.field, a.idx).cmp(&(b.surf, &b.field, b.idx)));
    out
}

/// 表の行。all なら状態で変えていない値（要素の ON/OFF・後の空気・構成、群の移動、系の値）も。pinned は足した行
fn items(c: &Combo, all: bool, pinned: &[Item]) -> Vec<Item> {
    let mut out = Vec::new();
    for q in SysQ::all(c) {
        let it = Item::Sys(q);
        if all || pinned.contains(&it) || c.states.iter().any(|n| it.own(c, n)) {
            out.push(it);
        }
    }
    let stop = Item::Stop;
    if all || pinned.contains(&stop) || c.states.iter().any(|n| stop.own(c, n)) {
        out.push(stop);
    }
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
        let mut faces: Vec<Item> = face_items(&l, p).into_iter().map(Item::Face).collect();
        for it in pinned {
            if matches!(it, Item::Face(f) if f.loc == l) && !faces.contains(it) {
                faces.push(it.clone());
            }
        }
        out.extend(faces);
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

/// 行を足す: どの状態も今の値（状態の値・無ければ基本の値）を持つ行にする（OpticStudio で行を足したときと同じ）。
/// 状態が無ければ構成を 2 つ（Config 1・Config 2）足す
pub(crate) fn add_row(c: &mut Combo, view: &mut View, it: Item, now: FaceNow) {
    // ロックした要素の面の値は変えない
    if let Item::Face(f) = &it {
        if c.get(&f.loc).is_none_or(|p| p.locked) {
            return;
        }
    }
    if c.states.is_empty() {
        c.add_state("Config 1", None);
        c.add_state("Config 2", Some("Config 1"));
    }
    if let Item::Face(f) = &it {
        let cx = Cx {
            now,
            cache: &view.base_cache,
        };
        for n in c.states.clone() {
            let key = Face {
                idx: None,
                ..f.clone()
            };
            if let Some(v) = key.raw(c, Some(&n), cx.now, cx.cache) {
                if let Some(p) = c.get_mut(&f.loc) {
                    if p.state_edit(&n, f.surf, &f.field).is_none() {
                        p.set_state_edit(&n, f.surf, &f.field, v);
                    }
                }
            }
        }
    }
    if !view.pinned.contains(&it) {
        view.pinned.push(it);
    }
}

/// 行を消す: どの状態の値も基本の値に戻す
fn remove_row(c: &mut Combo, view: &mut View, it: &Item) {
    for n in c.states.clone() {
        it.clear(c, &n);
    }
    view.pinned.retain(|x| x != it);
}

/// 「+ 行」のメニュー: 系の値の行と、要素の面の行
fn add_row_menu(ui: &mut Ui, c: &mut Combo, view: &mut View, now: FaceNow) -> bool {
    let mut add: Option<Item> = None;
    ui.menu_button(tr("+ Row"), |ui| {
        ui.label(RichText::new(tr("System value")).strong());
        let shown = items(c, false, &view.pinned);
        for q in SysQ::all(c) {
            let it = Item::Sys(q);
            if !shown.contains(&it) && ui.button(q.label()).clicked() {
                add = Some(it);
                ui.close_menu();
            }
        }
        if !shown.contains(&Item::Stop) && ui.button(tr("Stop surface")).clicked() {
            add = Some(Item::Stop);
            ui.close_menu();
        }
        ui.separator();
        ui.label(RichText::new(tr("Surface value of an element")).strong());
        let locs: Vec<Loc> = c
            .all()
            .into_iter()
            .filter(|l| {
                c.get(l)
                    .is_some_and(|p| {
                        !p.locked && !matches!(p.kind, ElemKind::Splitter | ElemKind::Aperture)
                    })
            })
            .collect();
        if locs.is_empty() {
            ui.weak(tr("No elements"));
            return;
        }
        view.add_el = view.add_el.min(locs.len() - 1);
        egui::ComboBox::from_id_salt("mce_add_el")
            .selected_text(c.get(&locs[view.add_el]).map(Pick::label).unwrap_or_default())
            .show_ui(ui, |ui| {
                for (i, l) in locs.iter().enumerate() {
                    let name = c.get(l).map(Pick::label).unwrap_or_default();
                    ui.selectable_value(&mut view.add_el, i, name);
                }
            });
        ui.horizontal(|ui| {
            ui.label(tr("Surface"));
            let mut k = view.add_surf + 1;
            ui.add(egui::DragValue::new(&mut k).range(1..=999));
            view.add_surf = k - 1;
            egui::ComboBox::from_id_salt("mce_add_field")
                .selected_text(FACE_FIELDS[view.add_field.min(FACE_FIELDS.len() - 1)].0)
                .show_ui(ui, |ui| {
                    for (i, (f, _)) in FACE_FIELDS.iter().enumerate() {
                        ui.selectable_value(&mut view.add_field, i, *f);
                    }
                });
        });
        let (field, list) = FACE_FIELDS[view.add_field.min(FACE_FIELDS.len() - 1)];
        if ui
            .button(tr("Add"))
            .on_hover_text(tr(
                "Surfaces are counted within the element from 1, without coordinate breaks. Every configuration starts with its current value",
            ))
            .clicked()
        {
            add = Some(Item::Face(Face {
                loc: locs[view.add_el].clone(),
                surf: view.add_surf,
                field: field.to_string(),
                idx: list.then_some(0),
            }));
            ui.close_menu();
        }
    });
    match add {
        Some(it) => {
            add_row(c, view, it, now);
            true
        }
        None => false,
    }
}

/// 構成（状態）の列の見出しの右クリック: 写して足す・名前を変える・消す
enum HeadAct {
    Copy(String),
    Rename(String),
    Remove(String),
}

/// 状態の表。値を変えたら true（呼ぶ側が組み直す）。now は今の系の面の値（面の行の基本の値）、stops は絞りにできる
/// 要素の面（今の系の要素の面）
pub(crate) fn ui(
    ui: &mut Ui,
    c: &mut Combo,
    view: &mut View,
    now: FaceNow,
    stops: &[(Loc, usize)],
) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        let from = c.active_state().map(str::to_string);
        let hint = match &from {
            Some(n) => trf("Add a configuration (state) copied from {}", &[n]),
            None => tr("Add a configuration (state) with the base values").to_string(),
        };
        if ui
            .button(tr("+ Configuration"))
            .on_hover_text(hint)
            .clicked()
        {
            let name = format!("Config {}", c.states.len() + 1);
            let name = if c.states.contains(&name) {
                c.free_state_name(&name)
            } else {
                name
            };
            c.add_state(&name, from.as_deref());
            changed = true;
        }
        changed |= add_row_menu(ui, c, view, now);
        if !c.states.is_empty() {
            ui.checkbox(&mut view.all, tr("Show values not changed in any state"));
        }
        if let Some(e) = &view.rename_err {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    });
    if c.states.is_empty() {
        ui.add_space(8.0);
        ui.label(tr(
            "No configurations yet. Add one with + Configuration, or add a row (states are the configurations of a combination: system values, element ON/OFF, spacing, decenter and tilt, group shift, surface values, .zmx configuration).",
        ));
        return changed;
    }
    ui.label(
        RichText::new(tr(
            "Click a column header to make it the current configuration; right-click it to copy, rename or delete. Right-click a cell to use the base value, a row name to delete the row",
        ))
        .small()
        .color(weak),
    );
    // 面の行の基本の値を覚える（今の状態が上書きしていないときの今の系の値）
    let cur = c.active_state().map(str::to_string);
    let rows = items(c, view.all, &view.pinned);
    for it in &rows {
        if let Item::Face(f) = it {
            let key = Face {
                idx: None,
                ..f.clone()
            };
            let own = cur.as_deref().is_some_and(|n| it.own(c, n));
            if !own {
                if let Some(v) = now(&key) {
                    view.base_cache.insert(key, v);
                }
            }
        }
    }
    let cx = Cx {
        now,
        cache: &view.base_cache,
    };
    let states = c.states.clone();
    let cols: Vec<Option<String>> = std::iter::once(None)
        .chain(states.iter().cloned().map(Some))
        .collect();
    let mut set: Option<(usize, Option<String>, Val)> = None;
    let mut clear: Option<(usize, String)> = None;
    let mut del_row: Option<usize> = None;
    let mut pick_state: Option<Option<String>> = None;
    let mut head: Option<HeadAct> = None;
    let mut renaming = view.renaming.clone();
    let mut rename_err = view.rename_err.clone();
    egui::ScrollArea::horizontal()
        .id_salt("state_table_h")
        .show(ui, |ui| {
            let mut tb = TableBuilder::new(ui)
                .striped(true)
                // 低いパネルでもパネルの中に収める（既定の最低 200 だとはみ出し、最後の行までスクロールできない）
                .min_scrolled_height(0.0)
                .column(Column::auto().at_least(160.0).resizable(true));
            for _ in &cols {
                tb = tb.column(Column::auto().at_least(80.0));
            }
            tb.header(22.0, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new(tr("Value")).color(weak));
                });
                for col in &cols {
                    h.col(|ui| {
                        // 名前を変えている列は欄
                        if let (Some(n), Some((from, to))) = (col, renaming.as_mut()) {
                            if from == n {
                                let r = ui.add(egui::TextEdit::singleline(to).desired_width(80.0));
                                r.request_focus();
                                if r.lost_focus() {
                                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                        head = Some(HeadAct::Rename(n.clone()));
                                    } else {
                                        head = Some(HeadAct::Rename(String::new()));
                                    }
                                }
                                return;
                            }
                        }
                        let name = col.clone().unwrap_or_else(|| tr("Base").to_string());
                        let is_cur = c.state == *col;
                        let text = if is_cur {
                            RichText::new(name).strong().color(accent)
                        } else {
                            RichText::new(name)
                        };
                        let hint = match col {
                            Some(n) => trf("Make {} the current state", &[n]),
                            None => tr("Show the base values (no state)").to_string(),
                        };
                        let r = ui.selectable_label(is_cur, text).on_hover_text(hint);
                        if r.clicked() && !is_cur {
                            pick_state = Some(col.clone());
                        }
                        if let Some(n) = col {
                            r.context_menu(|ui| {
                                if ui.button(tr("Copy to a new configuration")).clicked() {
                                    head = Some(HeadAct::Copy(n.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(tr("Rename")).clicked() {
                                    renaming = Some((n.clone(), n.clone()));
                                    rename_err = None;
                                    ui.close_menu();
                                }
                                if ui.button(tr("Delete")).clicked() {
                                    head = Some(HeadAct::Remove(n.clone()));
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                }
            })
            .body(|mut body| {
                // 表に行の無い系の値（温度・気圧・GLRS など）を状態で変えたか
                let more_own = |n: &str| c.state_more.contains_key(n);
                if states.iter().any(|n| more_own(n)) {
                    body.row(20.0, |mut r| {
                        r.col(|ui| {
                            ui.label(RichText::new(tr("Other system settings")).color(weak));
                        });
                        for col in &cols {
                            r.col(|ui| {
                                let own = col.as_deref().is_some_and(more_own);
                                ui.label(if own { tr("changed") } else { "—" });
                            });
                        }
                    });
                }
                for (i, it) in rows.iter().enumerate() {
                    body.row(22.0, |mut r| {
                        r.col(|ui| {
                            let resp = ui.label(it.label(c));
                            resp.context_menu(|ui| {
                                if ui.button(tr("Delete row")).on_hover_text(tr("Every configuration uses the base value")).clicked() {
                                    del_row = Some(i);
                                    ui.close_menu();
                                }
                            });
                        });
                        for col in &cols {
                            r.col(|ui| {
                                let st = col.as_deref();
                                let own = st.is_some_and(|n| it.own(c, n));
                                let Some(v) = it.get(c, st, &cx) else { return };
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
                                    Val::Choice(mut k, keys) => {
                                        let was = k;
                                        let r = egui::ComboBox::from_id_salt(("mce_choice", i, col.clone()))
                                            .selected_text(keys.get(k).copied().unwrap_or("—"))
                                            .width(80.0)
                                            .show_ui(ui, |ui| {
                                                for (j, key) in keys.iter().enumerate() {
                                                    ui.selectable_value(&mut k, j, *key);
                                                }
                                            })
                                            .response;
                                        if k != was {
                                            set = Some((i, col.clone(), Val::Choice(k, keys)));
                                        }
                                        r
                                    }
                                    Val::Unknown => {
                                        // 読めない値も打ち込めば値になる（数・true/false・文字）。Enter かフォーカスが
                                        // 外れたときに確定し、Esc で取り消す（打ち込み中の文字は覚えておく）
                                        let buf = ui.id().with(("mce_unknown", i, col.clone()));
                                        let mut t = ui.data_mut(|d| d.get_temp::<String>(buf)).unwrap_or_default();
                                        let r = ui
                                            .add(egui::TextEdit::singleline(&mut t).hint_text("—").desired_width(70.0))
                                            .on_hover_text(tr(
                                                "The base value is read from the built system; select Base once to read it",
                                            ));
                                        let done = r.lost_focus();
                                        if done {
                                            ui.data_mut(|d| d.remove::<String>(buf));
                                        } else if r.changed() {
                                            ui.data_mut(|d| d.insert_temp(buf, t.clone()));
                                        }
                                        let esc = ui.input(|inp| inp.key_pressed(egui::Key::Escape));
                                        if done && !esc && !t.trim().is_empty() {
                                            let v = match (t.trim().parse::<f64>(), t.trim()) {
                                                (Ok(x), _) => Val::Num(x),
                                                (_, "true") => Val::Bool(true),
                                                (_, "false") => Val::Bool(false),
                                                _ => Val::Text(t),
                                            };
                                            set = Some((i, col.clone(), v));
                                        }
                                        r
                                    }
                                    Val::Stop(at) => {
                                        let mut pick = at.clone();
                                        let r = egui::ComboBox::from_id_salt(("mce_stop", col.clone()))
                                            .selected_text(stop_label(c, &at))
                                            .width(120.0)
                                            .show_ui(ui, |ui| {
                                                // 先頭に自動で置く絞りは基本だけ（状態から指せるブロックが無い）
                                                if col.is_none() {
                                                    ui.selectable_value(&mut pick, None, stop_label(c, &None));
                                                }
                                                for f in stops {
                                                    let x = Some(f.clone());
                                                    let name = stop_label(c, &x);
                                                    ui.selectable_value(&mut pick, x, name);
                                                }
                                            })
                                            .response;
                                        if pick != at {
                                            set = Some((i, col.clone(), Val::Stop(pick)));
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
        let cache = view.base_cache.clone();
        let cx = Cx { now, cache: &cache };
        rows[i].set(c, col.as_deref(), v, &cx);
        changed = true;
    }
    if let Some((i, n)) = clear {
        rows[i].clear(c, &n);
        changed = true;
    }
    if let Some(i) = del_row {
        remove_row(c, view, &rows[i]);
        changed = true;
    }
    view.renaming = renaming;
    view.rename_err = rename_err;
    match head {
        Some(HeadAct::Copy(n)) => {
            let name = c.free_state_name(&n);
            c.add_state(&name, Some(&n));
            changed = true;
        }
        Some(HeadAct::Rename(n)) => {
            if let Some((from, to)) = view.renaming.take() {
                if !n.is_empty() {
                    match c.rename_state(&from, &to) {
                        Ok(()) => changed = true,
                        Err(e) => view.rename_err = Some(e),
                    }
                }
            }
        }
        Some(HeadAct::Remove(n)) => {
            c.remove_state(&n);
            changed = true;
        }
        None => {}
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
        assert!(items(&c, false, &[]).is_empty());
        c.picks[0]
            .zmx_mut()
            .state_config
            .insert("Config 2".into(), 2);
        let rows = items(&c, false, &[]);
        assert_eq!(rows, vec![Item::Config(vec![0])]);
        let none = |_: &Face| None;
        let cache = HashMap::new();
        let cx = Cx {
            now: &none,
            cache: &cache,
        };
        rows[0].set(&mut c, Some("Config 1"), Val::Int(1), &cx);
        assert!(rows[0].own(&c, "Config 1"));
        assert_eq!(c.picks[0].config_in(Some("Config 2")), 2);
        rows[0].clear(&mut c, "Config 2");
        assert_eq!(c.picks[0].config_in(Some("Config 2")), 1);
        // 全部: 要素の ON/OFF・後の空気・構成
        let all = items(&c, true, &[]);
        assert!(all.contains(&Item::On(vec![1])) && all.contains(&Item::Air(vec![1])));
        Item::Air(vec![1]).set(&mut c, Some("Config 2"), Val::Num(7.0), &cx);
        assert_eq!(c.picks[1].air_in(Some("Config 2")), 7.0);
        assert_eq!(c.picks[1].air_in(None), c.picks[1].air);
    }

    /// 系の値の行: 状態の列を変えるとその状態の系の値になり、基本に戻すと（ほかに違いが無ければ）状態の系の値は外れる
    #[test]
    fn system_rows() {
        let mut c = Combo {
            states: vec!["A".into(), "B".into()],
            ..Default::default()
        };
        assert!(!items(&c, false, &[])
            .iter()
            .any(|i| matches!(i, Item::Sys(_))));
        let none = |_: &Face| None;
        let cache = HashMap::new();
        let cx = Cx {
            now: &none,
            cache: &cache,
        };
        let w2 = Item::Sys(SysQ::Wave(1));
        w2.set(&mut c, Some("A"), Val::Num(0.55), &cx);
        assert_eq!(c.state_sys["A"].waves[1].um, 0.55);
        assert!(w2.own(&c, "A") && !w2.own(&c, "B"));
        assert!(items(&c, false, &[]).contains(&w2));
        assert_eq!(w2.get(&c, Some("B"), &cx), Some(Val::Num(c.waves[1].um)));
        // 開口の種類を F 値にすると初めの値
        let at = Item::Sys(SysQ::ApType);
        at.set(&mut c, Some("B"), Val::Choice(1, &AP_KEYS), &cx);
        assert_eq!(c.state_sys["B"].aperture, ApertureKind::FNum);
        assert_eq!(
            Item::Sys(SysQ::ApValue).get(&c, Some("B"), &cx),
            Some(Val::Num(5.0))
        );
        // 物体距離
        let od = Item::Sys(SysQ::ObjDist);
        od.set(&mut c, Some("A"), Val::Text("100".into()), &cx);
        assert_eq!(c.state_sys["A"].object_distance, Some(100.0));
        w2.clear(&mut c, "A");
        od.clear(&mut c, "A");
        assert!(!c.state_sys.contains_key("A"));
        // 基本を変える
        w2.set(&mut c, None, Val::Num(0.6), &cx);
        assert_eq!(c.waves[1].um, 0.6);
    }

    /// 面の行: 足すと全部の状態に今の値が入り、状態の列を変えるとその状態だけ、行を消すと上書きが外れる
    #[test]
    fn face_rows() {
        let mut c = Combo::default();
        c.picks.push(Pick::new(ElemKind::Plate));
        let mut view = View::default();
        let now = |f: &Face| match f.field.as_str() {
            "t" => Some(json!(3.0)),
            "asph" => Some(json!([])),
            _ => None,
        };
        let f = Face {
            loc: vec![0],
            surf: 0,
            field: "t".into(),
            idx: None,
        };
        // 状態が無ければ構成 2 つを足す
        add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
        assert_eq!(c.states, ["Config 1", "Config 2"]);
        assert_eq!(c.picks[0].state_edit("Config 2", 0, "t"), Some(&json!(3.0)));
        let it = Item::Face(f.clone());
        assert!(items(&c, false, &view.pinned).contains(&it));
        let cache = HashMap::new();
        let cx = Cx {
            now: &now,
            cache: &cache,
        };
        it.set(&mut c, Some("Config 2"), Val::Num(5.0), &cx);
        assert_eq!(it.get(&c, Some("Config 2"), &cx), Some(Val::Num(5.0)));
        assert_eq!(it.get(&c, Some("Config 1"), &cx), Some(Val::Num(3.0)));
        assert_eq!(
            c.picks[0].edits_in("Config 2").unwrap()[0].value,
            json!(5.0)
        );
        // 並びの値（非球面）は番号の所だけ変える
        let a = Item::Face(Face {
            field: "asph".into(),
            idx: Some(1),
            ..f.clone()
        });
        a.set(&mut c, Some("Config 1"), Val::Num(-2e-5), &cx);
        assert_eq!(
            c.picks[0].state_edit("Config 1", 0, "asph"),
            Some(&json!([0.0, -2e-5]))
        );
        assert!(items(&c, false, &[]).contains(&a));
        remove_row(&mut c, &mut view, &it);
        assert!(c.picks[0].state_edit("Config 2", 0, "t").is_none());
        assert!(!items(&c, false, &view.pinned).contains(&it));
        // 面の並びが変わると状態の上書きも外れる
        c.picks[0].reversed = !c.picks[0].reversed;
        assert!(c.picks[0].drop_stale_edits() > 0);
        assert!(c.picks[0].state_edits.is_empty());
    }

    /// 今の状態が上書きしている値は基本の値として読まない
    #[test]
    fn current_state_value_is_not_base() {
        // 今の状態 A が t を 5 に上書きし、今の系の t は 5（基本は 3 だが覚えていない）
        let mut c = Combo {
            states: vec!["A".into(), "B".into()],
            state: Some("A".into()),
            ..Default::default()
        };
        c.picks.push(Pick::new(ElemKind::Plate));
        c.picks[0].set_state_edit("A", 0, "t", json!(5.0));
        let now = |f: &Face| (f.field == "t").then(|| json!(5.0));
        let f = Face {
            loc: vec![0],
            surf: 0,
            field: "t".into(),
            idx: None,
        };
        let mut view = View::default();
        add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
        // 基本が分からないので B には値を入れない
        assert_eq!(c.picks[0].state_edit("B", 0, "t"), None);
        let cache = HashMap::new();
        let cx = Cx {
            now: &now,
            cache: &cache,
        };
        assert_eq!(
            Item::Face(f.clone()).get(&c, Some("B"), &cx),
            Some(Val::Unknown)
        );
        // 覚えた基本の値があればそれを入れる
        view.base_cache.insert(f.clone(), json!(3.0));
        add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
        assert_eq!(c.picks[0].state_edit("B", 0, "t"), Some(&json!(3.0)));
        // ロックした要素の面の値は変えない
        c.picks[0].locked = true;
        let k = Face {
            field: "k".into(),
            ..f
        };
        add_row(&mut c, &mut view, Item::Face(k.clone()), &now);
        Item::Face(k).set(&mut c, Some("B"), Val::Num(-1.0), &cx);
        assert_eq!(c.picks[0].state_edit("B", 0, "k"), None);
    }

    /// 絞りの面・テレセントリックの行
    #[test]
    fn stop_tele_rows() {
        let mut c = Combo {
            states: vec!["A".into()],
            ..Default::default()
        };
        c.picks.push(Pick::new(ElemKind::Plate));
        let none = |_: &Face| None;
        let cache = HashMap::new();
        let cx = Cx {
            now: &none,
            cache: &cache,
        };
        let at = Some((vec![0], 1));
        Item::Stop.set(&mut c, Some("A"), Val::Stop(at.clone()), &cx);
        assert!(Item::Stop.own(&c, "A"));
        assert_eq!(Item::Stop.get(&c, Some("A"), &cx), Some(Val::Stop(at)));
        assert!(items(&c, false, &[]).contains(&Item::Stop));
        // 基本と同じ絞りにすると状態の値は外れる
        let base = c.base_stop();
        Item::Stop.set(&mut c, Some("A"), Val::Stop(base), &cx);
        assert!(!Item::Stop.own(&c, "A"));
        let tele = Item::Sys(SysQ::Telecentric);
        tele.set(&mut c, Some("A"), Val::Bool(true), &cx);
        assert!(c.state_sys["A"].telecentric && !c.telecentric);
    }
}
