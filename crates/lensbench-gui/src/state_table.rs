//! 組合せの状態の表（OpticStudio の多重構成のエディタに当たる表）。列 = 基本と状態（構成）、行 = 状態で変えられる値:
//! 系の値（波長・波長の重み・主波長・開口の種類と値・視野の種類・視野・物体距離）、要素の ON/OFF・後の空気・偏心と傾き・
//! .zmx の取り込みの構成・群の移動・BS の経路、要素の面の値（曲率半径・厚さ・円錐係数・非球面係数・半直径・無視・材料など、
//! 状態の面の上書き [`Pick::state_edits`]）。状態の列のセルを変えるとその状態の値になり、右クリックの「基本の値に戻す」で
//! 外れる。行は「+ 行」で足し（面の行は面の表の右クリックからも）、行の名前の右クリックで消す。構成（状態）は列の見出しの
//! 右クリックで写して足す・名前を変える・消す

use egui::{RichText, Ui};

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::combo::{Combo, ElemKind, Pick};
use crate::combo_tree::{Loc, CB_FIELD};
use crate::i18n::{tr, trf};
use crate::sysset::{ApertureField, ApertureKind, Choice, FieldKind, SysSnap};
use crate::theme;

mod picks;
mod sheet_view;
pub(crate) use picks::{
    drop_unused_groups, from_json as picks_from_json, order_from_json, order_json, remap_rank,
    to_json as picks_json, zgroup_removed,
};
#[cfg(test)]
use picks::{remap_index_picks, remap_index_rank};

/// 表の表示の設定
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    /// 状態で変えていない値の行も出す
    pub all: bool,
    /// 足した行（どの状態も基本と同じ値の系の値の行も出す。保存はしない）
    pub pinned: Vec<Item>,
    /// 「+ 行」の、今の系に無い要素の面の番号
    pub add_surf: usize,
    /// 名前を変えている構成（元の名前・新しい名前）
    pub renaming: Option<(String, String)>,
    /// 名前を変えられなかった理由（次に名前を変えるか構成を足すまで出す）
    pub rename_err: Option<String>,
    /// 面の値の基本の値（今の系から読めたとき覚えておく。今の状態が上書きしているときに使う）
    pub base_cache: HashMap<Face, Value>,
    /// 面の表を受け取ってピックアップを当て直し、組み直したところ（次の面の表では当て直さない。輪で止まらなくならないよう）
    pub picks_rebuilt: bool,
    /// 表の選択と書き換え中の文字
    pub sheet: crate::widgets::sheet::SheetState,
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
    ("aperture", false),
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
    /// コメント行（[`Combo::mc_comments`] の番号。OpticStudio の MCOM）
    Comment(usize),
}

pub(crate) const PLACE: [&str; 5] = ["dx", "dy", "tx", "ty", "tz"];

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

/// 要素の面の、今の系の通しの面番号（OpticStudio の面番号、物体面が 0）。`cb` は座標ブレークだけを数えた要素の面の番号。
/// 今の系に無い面（面の表がまだ無いときも）は None
pub(crate) type SurfNo<'a> = &'a dyn Fn(&Loc, usize, bool) -> Option<usize>;

const AP_KEYS: [&str; 6] = ["epd", "fnum", "na", "float", "pwfn", "cone"];
const FT_KEYS: [&str; 5] = [
    "angle",
    "object_height",
    "paraxial_image_height",
    "real_image_height",
    "solve",
];

fn snap_in(c: &Combo, st: Option<&str>) -> SysSnap {
    st.and_then(|n| c.state_snap(n)).unwrap_or_else(|| c.snap())
}

impl SysQ {
    /// 「+ 行」の欄のグループ（波長・開口・視野・その他）
    fn group(self) -> usize {
        match self {
            SysQ::Wave(_) | SysQ::WaveW(_) | SysQ::Primary => 0,
            SysQ::ApType | SysQ::ApValue => 1,
            SysQ::FieldType | SysQ::FieldX(_) | SysQ::FieldY(_) | SysQ::FieldW(_) => 2,
            SysQ::ObjDist | SysQ::Telecentric => 3,
        }
    }

    pub(crate) fn label(self) -> String {
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
            .chain(c.state_sys.keys().filter_map(|n| c.state_snap(n)))
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
    let field = field.strip_prefix(CB_FIELD).unwrap_or(field);
    let v = match idx {
        Some(i) => match v.as_array() {
            Some(a) => a.get(i).unwrap_or(&Value::Null),
            None => return Val::Unknown,
        },
        None => v,
    };
    match field {
        // 円形の開口・遮蔽の最大の半径（多重構成の APMX）。開口が無ければ 0、ほかの形は読めない
        "aperture" if v.is_null() => Val::Num(0.0),
        "aperture" => match (circular(v), v.get("max").and_then(Value::as_f64)) {
            (true, Some(x)) => Val::Num(x),
            _ => Val::Unknown,
        },
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

/// 円形の開口か遮蔽（最大の半径を持つ形）
fn circular(v: &Value) -> bool {
    matches!(
        v["type"].as_str(),
        Some("circular" | "obsc" | "obscuration")
    )
}

/// 表の値を面の値の JSON に。並び（idx）は前の並び old の idx 番目を変える
fn face_json(field: &str, v: &Val, idx: Option<usize>, old: Option<&Value>) -> Option<Value> {
    let field = field.strip_prefix(CB_FIELD).unwrap_or(field);
    if field == "aperture" {
        // 基本の開口が分かり、無いか円形・遮蔽のときだけ最大の半径を変える（ほかの形は置き換えない）
        let (Val::Num(x), Some(old)) = (v, old) else {
            return None;
        };
        let mut a = match old {
            Value::Null => json!({ "type": "circular", "min": 0.0 }),
            o if circular(o) => o.clone(),
            _ => return None,
        };
        if *x <= 0.0 {
            return Some(Value::Null);
        }
        a["max"] = json!(x);
        return Some(a);
    }
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
        let what = field_label(&self.field, self.idx);
        if self.field.starts_with(CB_FIELD) {
            return trf(
                "{} coordinate break {} {}",
                &[&el_name(c, &self.loc), &(self.surf + 1), &what],
            );
        }
        trf(
            "{} surface {} {}",
            &[&el_name(c, &self.loc), &(self.surf + 1), &what],
        )
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

/// 面の値の名前の表示（idx は並びの値の何番目か）
pub(crate) fn field_label(field: &str, idx: Option<usize>) -> String {
    let field = field.strip_prefix(CB_FIELD).unwrap_or(field);
    match (field, idx) {
        ("cb", Some(i)) => PLACE.get(i).copied().unwrap_or_default().to_string(),
        ("cb_order", _) => tr("order").into(),
        ("asph", Some(i)) => trf("asphere {}", &[&(i + 1)]),
        ("asph", None) => tr("asphere").into(),
        ("r", _) => tr("radius").into(),
        ("t", _) => tr("thickness").into(),
        ("k", _) => tr("conic").into(),
        ("semi", _) => tr("semi-diameter").into(),
        ("ignore", _) => tr("ignore").into(),
        ("material", _) => tr("material").into(),
        ("coating", _) => tr("coating").into(),
        ("aperture", _) => tr("aperture max radius").into(),
        (f, _) => f.to_string(),
    }
}

/// 要素の表示の名前（プロジェクトの id と種類・品名。同じ種類の要素を見分けられるように）
fn el_name(c: &Combo, l: &Loc) -> String {
    let label = c.get(l).map(Pick::label).unwrap_or_default();
    match c.ids_all().into_iter().find(|(x, _)| x == l) {
        Some((_, id)) if !id.is_empty() => format!("{id} {label}"),
        _ => label,
    }
}

/// 絞りの面の表示（要素の名前と面の番号）
fn stop_label(c: &Combo, at: &Option<(Loc, usize)>) -> String {
    match at {
        Some((l, i)) => trf("{} surface {}", &[&el_name(c, l), &(i + 1)]),
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
        let el = |l: &Loc| el_name(c, l);
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
            Item::Comment(_) => tr("Comment").to_string(),
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
            Item::Comment(k) => {
                let m = c.mc_comments.get(*k)?;
                Val::Text(
                    st.and_then(|n| m.values.get(n).cloned())
                        .unwrap_or_else(|| m.base.clone()),
                )
            }
        })
    }

    /// ロックで変えられない行（ロックした要素の面の値、位置もロックした要素の空気・偏心・傾き）
    fn locked(&self, c: &Combo) -> bool {
        match self {
            Item::Face(f) => c.get(&f.loc).is_some_and(|p| p.locked),
            Item::Air(l) | Item::Place(l, _) => c.get(l).is_some_and(Pick::fixed),
            _ => false,
        }
    }

    /// 状態 n で値を変えているか
    fn own(&self, c: &Combo, n: &str) -> bool {
        match self {
            Item::Sys(q) => c
                .state_snap(n)
                .is_some_and(|s| q.get(&s) != q.get(&c.snap())),
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
            Item::Comment(k) => c
                .mc_comments
                .get(*k)
                .is_some_and(|m| m.values.contains_key(n)),
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
                    Some(n) => c.set_state_snap(&n, &s),
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
                let Some(x) = face_json(&f.field, &v, f.idx, old.as_ref()) else {
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
            (Item::Comment(k), Val::Text(t)) => {
                if let Some(m) = c.mc_comments.get_mut(*k) {
                    match key {
                        // 基本と同じ文は状態の値にしない
                        Some(n) if t == m.base => {
                            m.values.remove(&n);
                        }
                        Some(n) => {
                            m.values.insert(n, t);
                        }
                        None => m.base = t,
                    }
                }
            }
            _ => {}
        }
    }

    /// 状態 n で変えた値をやめる。並びの値（非球面係数など）の 1 つの番号の行は、その番号だけ基本の値に戻す
    /// （状態の上書きは並びごとなので、ほかの番号の状態の値は残す）
    fn clear_cx(&self, c: &mut Combo, n: &str, cx: &Cx) {
        if let Item::Face(f @ Face { idx: Some(i), .. }) = self {
            let base = f.raw(c, None, cx.now, cx.cache);
            let own = c
                .get(&f.loc)
                .and_then(|p| p.state_edit(n, f.surf, &f.field).cloned());
            if let (Some(Value::Array(mut a)), Some(Value::Array(b))) = (own, base) {
                let bi = b.get(*i).cloned().unwrap_or(json!(0.0));
                if *i < a.len() {
                    a[*i] = bi;
                }
                if let Some(p) = c.get_mut(&f.loc) {
                    if a == b {
                        p.clear_state_edit(n, f.surf, &f.field);
                    } else {
                        p.set_state_edit(n, f.surf, &f.field, Value::Array(a));
                    }
                }
                return;
            }
        }
        self.clear(c, n);
    }

    /// 状態 n で変えた値をやめる（基本の値に戻す）
    fn clear(&self, c: &mut Combo, n: &str) {
        match self {
            Item::Sys(q) => {
                let base = c.snap();
                if let (Some(mut s), Some(v)) = (c.state_snap(n), q.get(&base)) {
                    q.set(&mut s, &v);
                    c.set_state_snap(n, &s);
                    if c.state_sys.get(n).is_some_and(|d| d.is_empty()) {
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
            Item::Comment(k) => {
                if let Some(m) = c.mc_comments.get_mut(*k) {
                    m.values.remove(n);
                }
            }
        }
    }
}

/// 要素 p（場所 l）の面の行: どれかの状態が上書きしている面の値（asph は値の入った番号ごと）
fn face_items(l: &Loc, p: &Pick) -> Vec<Face> {
    let mut out: Vec<Face> = Vec::new();
    for e in p.state_edits.values().flatten() {
        // 並びの値（非球面の係数・座標ブレークの偏心と傾き）は 0 でない番号ごとの行
        let listed = e.field == "asph" || e.field == format!("{CB_FIELD}cb");
        let idxs: Vec<Option<usize>> = if listed {
            let n = p
                .state_edits
                .values()
                .flatten()
                .filter(|x| x.surface == e.surface && x.field == e.field)
                .filter_map(|x| x.value.as_array().map(Vec::len))
                .max()
                .unwrap_or(0);
            (0..n)
                .filter(|&i| {
                    p.state_edits.values().flatten().any(|x| {
                        x.surface == e.surface
                            && x.field == e.field
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

/// 状態で変えた面の行の表示（試験用。言語の設定によらず英語の名前）
#[cfg(test)]
pub(crate) fn face_rows(c: &Combo) -> Vec<String> {
    items(c, false, &[])
        .iter()
        .filter(|it| matches!(it, Item::Face(_)))
        .map(|it| it.label(c))
        .collect()
}

/// 表の行。all なら状態で変えていない値（要素の ON/OFF・後の空気・構成、群の移動、系の値）も。pinned は足した行
fn items(c: &Combo, all: bool, pinned: &[Item]) -> Vec<Item> {
    // ピックアップのある行は値が基本と同じでも出す
    let mut pinned = pinned.to_vec();
    pinned.extend(picks::rows(c));
    let pinned = &pinned[..];
    let pin = |it: &Item| pinned.contains(it);
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
        let on = Item::On(l.clone());
        if p.kind.has_on() && (all || !p.state_on.is_empty() || pin(&on)) {
            out.push(on);
        }
        let air = Item::Air(l.clone());
        if all || !p.state_air.is_empty() || pin(&air) {
            out.push(air);
        }
        let cf = Item::Config(l.clone());
        if p.kind == ElemKind::ZmxRange && (all || !p.zmx().state_config.is_empty() || pin(&cf)) {
            out.push(cf);
        }
        let placed = (0..5).any(|k| pin(&Item::Place(l.clone(), k)));
        if p.has_state_place() && (!p.state_place.is_empty() || placed) {
            let ks: &[usize] = if p.kind == ElemKind::Mirror {
                &[2, 3]
            } else {
                &[0, 1, 2, 3, 4]
            };
            out.extend(ks.iter().map(|&k| Item::Place(l.clone(), k)));
        }
        for (g, z) in p.zmx().zgroups.iter().enumerate() {
            if p.kind == ElemKind::ZmxRange
                && (!z.state_shift.is_empty() || pin(&Item::ZShift(l.clone(), g)))
            {
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
        if !d.state_shift.is_empty() || (all && !d.role.is_empty()) || pin(&Item::Shift(g)) {
            out.push(Item::Shift(g));
        }
    }
    if !c.state_path.is_empty() || (all && !c.path.is_empty()) || pin(&Item::Path) {
        out.push(Item::Path);
    }
    out.extend((0..c.mc_comments.len()).map(Item::Comment));
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

/// 表の行 r（描いた時の rows の番号）を消す。後ろから消す（コメント行を消すと後ろのコメント行の番号が詰まる）
fn remove_rows(
    c: &mut Combo,
    view: &mut View,
    rows: &[Item],
    r: std::ops::RangeInclusive<usize>,
    now: FaceNow,
) {
    let mut sel: Vec<&Item> = r.filter_map(|i| rows.get(i)).collect();
    // コメント行は番号で指し、消すと後ろの番号が詰まるので、表の並び（入れ替えた順）でなく番号の大きい順に消す
    sel.sort_by_key(|it| {
        std::cmp::Reverse(match it {
            Item::Comment(k) => Some(*k),
            _ => None,
        })
    });
    for it in sel {
        remove_row(c, view, it, now);
    }
}

/// 行を消す: どの状態の値も基本の値に戻す（コメント行は行ごと消す）
fn remove_row(c: &mut Combo, view: &mut View, it: &Item, now: FaceNow) {
    picks::remove_row_picks(c, it);
    if let Item::Comment(k) = it {
        if *k < c.mc_comments.len() {
            c.mc_comments.remove(*k);
            picks::shift_comment_picks(c, *k);
            picks::shift_comment_order(c, *k);
        }
        return;
    }
    let cx = Cx {
        now,
        cache: &view.base_cache,
    };
    for n in c.states.clone() {
        it.clear_cx(c, &n, &cx);
    }
    view.pinned.retain(|x| x != it);
}

/// 「+ 行」のメニュー: 系の値（波長・開口・視野・その他）、絞りの面、要素ごと（ON/OFF・後の空気・偏心と傾き・
/// 面ごとの値）、群の移動をグループの下の欄で選ぶ
fn add_row_menu(ui: &mut Ui, c: &mut Combo, view: &mut View, now: FaceNow) -> bool {
    let add: std::cell::RefCell<Option<Item>> = Default::default();
    let comment = std::cell::Cell::new(false);
    ui.menu_button(tr("+ Row"), |ui| {
        // 表にある行（欄を開いているときだけ求める）
        let shown = items(c, false, &view.pinned);
        // まだ表に無い行の欄（押すとその行を足す）
        let pick = |ui: &mut Ui, it: Item, label: String| {
            if !shown.contains(&it) && ui.button(label).clicked() {
                *add.borrow_mut() = Some(it);
                ui.close();
            }
        };
        let sys = SysQ::all(c);
        let groups = [
            tr("Wavelengths"),
            tr("Aperture"),
            tr("Fields"),
            tr("Other system values"),
        ];
        for (g, name) in groups.into_iter().enumerate() {
            ui.menu_button(name, |ui| {
                for q in sys.iter().filter(|q| q.group() == g) {
                    pick(ui, Item::Sys(*q), q.label());
                }
            });
        }
        pick(ui, Item::Stop, tr("Stop surface").into());
        if ui.button(tr("Comment row")).clicked() {
            comment.set(true);
            ui.close();
        }
        ui.separator();
        let locs: Vec<Loc> = c
            .all()
            .into_iter()
            .filter(|l| {
                c.get(l)
                    .is_some_and(|p| !matches!(p.kind, ElemKind::Splitter | ElemKind::Aperture))
            })
            .collect();
        if locs.is_empty() {
            ui.weak(tr("No elements"));
        }
        for l in &locs {
            let Some(p) = c.get(l) else { continue };
            ui.menu_button(el_name(c, l), |ui| {
                if p.kind.has_on() {
                    pick(ui, Item::On(l.clone()), tr("ON / OFF").into());
                }
                pick(ui, Item::Air(l.clone()), tr("Air after").into());
                if p.kind == ElemKind::ZmxRange {
                    pick(ui, Item::Config(l.clone()), tr("Config").into());
                }
                if p.has_state_place() {
                    let ks: &[usize] = if p.kind == ElemKind::Mirror {
                        &[2, 3]
                    } else {
                        &[0, 1, 2, 3, 4]
                    };
                    for &k in ks {
                        pick(ui, Item::Place(l.clone(), k), PLACE[k].to_string());
                    }
                }
                if p.locked {
                    ui.weak(tr("The element is locked"));
                    return;
                }
                ui.separator();
                face_menu(ui, l, now, view, &mut |f| {
                    *add.borrow_mut() = Some(Item::Face(f));
                });
            });
        }
        let gs: Vec<usize> = (0..c.groups.len()).collect();
        if !gs.is_empty() {
            ui.separator();
            for g in gs {
                let it = Item::Shift(g);
                let label = it.label(c);
                pick(ui, it, label);
            }
        }
    });
    if comment.get() {
        add_comment(c);
        return true;
    }
    match add.into_inner() {
        Some(it) => {
            add_row(c, view, it, now);
            true
        }
        None => false,
    }
}

/// コメントの行を足す（状態が無ければ構成を 2 つ足す）
fn add_comment(c: &mut Combo) {
    if c.states.is_empty() {
        c.add_state("Config 1", None);
        c.add_state("Config 2", Some("Config 1"));
    }
    c.mc_comments.push(Default::default());
}

/// 要素 l の面ごとの値の欄（面は今の系から読めた面。読めなければ番号を打つ）。選んだ面の値を add へ
fn face_menu(ui: &mut Ui, l: &Loc, now: FaceNow, view: &mut View, add: &mut dyn FnMut(Face)) {
    let face = |surf: usize, field: &str, idx: Option<usize>| Face {
        loc: l.clone(),
        surf,
        field: field.to_string(),
        idx,
    };
    let fields = |ui: &mut Ui, surf: usize, add: &mut dyn FnMut(Face)| {
        for (f, list) in FACE_FIELDS {
            if ui.button(field_label(f, None)).clicked() {
                add(face(surf, f, list.then_some(0)));
                ui.close();
            }
        }
    };
    let n = (0..999)
        .take_while(|&i| now(&face(i, "r", None)).is_some())
        .count();
    for i in 0..n {
        let r = now(&face(i, "r", None))
            .and_then(|v| v.as_f64())
            .filter(|r| r.is_finite() && *r != 0.0);
        let label = match r {
            Some(r) => format!(
                "{}  (r {})",
                trf("Surface {}", &[&(i + 1)]),
                crate::json_view::fmt_num(r)
            ),
            None => trf("Surface {}", &[&(i + 1)]),
        };
        ui.menu_button(label, |ui| fields(ui, i, add));
    }
    // 座標ブレーク（座標ブレークだけを数えた番号）: 無視と偏心・傾き
    let ncb = (0..999)
        .take_while(|&i| now(&face(i, &format!("{CB_FIELD}cb"), None)).is_some())
        .count();
    for i in 0..ncb {
        ui.menu_button(trf("Coordinate break {}", &[&(i + 1)]), |ui| {
            let ign = format!("{CB_FIELD}ignore");
            if ui.button(field_label(&ign, None)).clicked() {
                add(face(i, &ign, None));
                ui.close();
            }
            for (k, name) in PLACE.iter().enumerate() {
                if ui.button(*name).clicked() {
                    add(face(i, &format!("{CB_FIELD}cb"), Some(k)));
                    ui.close();
                }
            }
        });
    }
    if n == 0 {
        // 今の系に無い要素（OFF など）は面の番号を打つ
        ui.horizontal(|ui| {
            ui.label(tr("Surface"));
            let mut k = view.add_surf + 1;
            ui.add(crate::widgets::Num::new(&mut k).range(1..=999))
                .on_hover_text(tr(
                    "Surfaces are counted within the element from 1, without coordinate breaks. Every configuration starts with its current value",
                ));
            view.add_surf = k - 1;
        });
        fields(ui, view.add_surf, add);
    }
}

/// 構成（状態）の列の見出しの右クリック: 写して足す・名前を変える・消す
#[derive(Debug, Clone)]
enum HeadAct {
    Copy(String),
    /// 基本の値の構成をその右に足す
    NewBase(String),
    /// どの行もその構成のピックアップにした構成をその右に足す
    WithPicks(String),
    Move(String, isize),
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
    surf_no: SurfNo,
) -> bool {
    let weak = theme::weak_text(ui.visuals());
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
        changed |= rename_ui(ui, c, view);
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
            "Double-click a column header to make it the current configuration; right-click it to copy, insert, move, rename or delete. Right-click a cell to use the base value or pick up another configuration or row. Empty a cell (Delete) to use the base value",
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
    // 行の並び（右クリック・Ctrl+Shift+↑↓ で入れ替えた順。組合せに保存し、元に戻すの対象）
    let mut rows = rows;
    picks::sort_rows(c, &mut rows);
    let mut srows: Vec<sheet_view::Row> = rows.iter().cloned().map(sheet_view::Row::It).collect();
    if c.states.iter().any(|n| c.state_more.contains_key(n)) {
        srows.push(sheet_view::Row::More);
    }
    let cols: Vec<Option<String>> = std::iter::once(None)
        .chain(c.states.iter().cloned().map(Some))
        .collect();
    // 覚えた値は表を描く間だけ借りる（毎フレーム写さない）
    let mut sh = sheet_view::McSheet {
        c: &mut *c,
        rows: srows,
        cols,
        now,
        cache: &view.base_cache,
        stops,
        surf_no,
        acts: Default::default(),
    };
    changed |= crate::widgets::sheet::show(ui, "state_table", &mut view.sheet, &mut sh);
    let acts = sh.acts.take();
    let mut head = None;
    for a in acts {
        match a {
            sheet_view::Act::Head(h) => head = Some(h),
            sheet_view::Act::Current(s) => {
                c.state = s;
                changed = true;
            }
            sheet_view::Act::Rename(n) => {
                view.renaming = Some((n.clone(), n));
                view.rename_err = None;
            }
            sheet_view::Act::Clear(i, n) => {
                let cx = Cx {
                    now,
                    cache: &view.base_cache,
                };
                rows[i].clear_cx(c, &n, &cx);
                changed = true;
            }
            sheet_view::Act::Pick(i, n, a) => {
                match a {
                    Some((src, k, o)) => picks::set_pick(c, &rows[i], &n, &src, k, o),
                    None => picks::remove_pick(c, &rows[i], &n),
                }
                changed = true;
            }
            sheet_view::Act::Remove(r) => {
                remove_rows(c, view, &rows, r, now);
                changed = true;
            }
            sheet_view::Act::Add(Some(it)) => {
                add_row(c, view, it, now);
                changed = true;
            }
            sheet_view::Act::Add(None) => {
                add_comment(c);
                changed = true;
            }
            sheet_view::Act::Swap(a) => {
                // 並べた順は組合せに持つ（保存し、元に戻すの対象）
                if a + 1 < rows.len() {
                    rows.swap(a, a + 1);
                    picks::set_order(c, &rows);
                    changed = true;
                }
            }
        }
    }
    match head {
        Some(HeadAct::Copy(n)) => {
            let name = c.free_state_name(&n);
            c.add_state(&name, Some(&n));
            changed = true;
        }
        Some(HeadAct::NewBase(n)) => {
            let name = c.next_config_name();
            c.add_state(&name, None);
            c.place_state_after(&name, &n);
            changed = true;
        }
        Some(HeadAct::WithPicks(n)) => {
            changed |= picks::add_state_with_picks(c, &n, &rows).is_some();
        }
        Some(HeadAct::Move(n, d)) => changed |= c.move_state(&n, d),
        Some(HeadAct::Remove(n)) => {
            c.remove_state(&n);
            changed = true;
        }
        None => {}
    }
    // 変えた値をピックアップの先にも
    if changed {
        apply_picks(c, &view.base_cache, now);
    }
    changed
}

/// 名前を変えている構成の欄（Enter で決める、Esc でやめる）
fn rename_ui(ui: &mut Ui, c: &mut Combo, view: &mut View) -> bool {
    let Some((from, to)) = view.renaming.as_mut() else {
        return false;
    };
    let mut changed = false;
    ui.label(trf("Rename {}", &[&from.as_str()]));
    let r = ui.add(egui::TextEdit::singleline(to).desired_width(100.0));
    if !r.has_focus() && !r.lost_focus() {
        r.request_focus();
    }
    if r.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let (from, to) = (from.clone(), to.clone());
            match c.rename_state(&from, &to) {
                Ok(()) => changed = true,
                Err(e) => view.rename_err = Some(e),
            }
        }
        view.renaming = None;
    }
    changed
}

/// ピックアップを全部当てる（値を変えたあと）。変えたか
pub(crate) fn apply_picks(c: &mut Combo, cache: &HashMap<Face, Value>, now: FaceNow) -> bool {
    picks::apply(c, &Cx { now, cache })
}

/// ピックアップの操作: (元の構成（空は基本）, 倍率, オフセット)。None はやめる
type PickAct = Option<(picks::Src, f64, f64)>;

/// ピックアップの値の表示
fn val_text(c: &Combo, v: &Val) -> String {
    match v {
        Val::Bool(b) => if *b { "1" } else { "0" }.into(),
        Val::Num(x) => crate::json_view::fmt_num(*x),
        Val::Int(k) => k.to_string(),
        Val::Text(t) => t.clone(),
        Val::Choice(k, keys) => keys.get(*k).copied().unwrap_or("—").into(),
        Val::Stop(at) => stop_label(c, at),
        Val::Unknown => "—".into(),
    }
}

/// セルの右クリックのピックアップの項目: 元の構成を選ぶ（同じ行か、同じ持ち主の別の行）・倍率とオフセットを変える・
/// やめる。rows は元にできる別の行（表示の名前・行の名前）、ok(行の名前, 構成) は元にできるか（輪にならないか）。
/// act に (元, 倍率, オフセット) か、やめるなら None を渡す
fn pick_menu(
    ui: &mut Ui,
    cols: &[Option<String>],
    ok: &dyn Fn(&str, &str) -> bool,
    rows: &[(String, String)],
    pk: Option<&crate::combo::McPick>,
    act: &mut dyn FnMut(PickAct),
) {
    let (scale, offset) = pk.map_or((1.0, 0.0), |m| (m.scale, m.offset));
    // 行 row の元にできる構成の欄（輪になる構成は除く）
    let mut confs = |ui: &mut Ui, row: &str| {
        for col in cols {
            let from = col.clone().unwrap_or_default();
            if !ok(row, &from) {
                continue;
            }
            let label = col.clone().unwrap_or_else(|| tr("Base").to_string());
            let on = pk.is_some_and(|m| m.from == from && m.row == row);
            if ui.selectable_label(on, label).clicked() {
                let src = picks::Src {
                    from,
                    row: row.to_string(),
                };
                act(Some((src, scale, offset)));
                ui.close();
            }
        }
    };
    ui.menu_button(tr("Pick up from"), |ui| confs(ui, ""));
    if !rows.is_empty() {
        ui.menu_button(tr("Pick up from another row"), |ui| {
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    for (label, key) in rows {
                        let on = pk.is_some_and(|m| m.row == *key);
                        let label = if on {
                            RichText::new(label).strong()
                        } else {
                            RichText::new(label)
                        };
                        ui.menu_button(label, |ui| confs(ui, key));
                    }
                });
        });
    }
    if let Some(m) = pk {
        let (mut k, mut o) = (m.scale, m.offset);
        ui.horizontal(|ui| {
            ui.label("×");
            let a = ui.add(crate::widgets::Num::new(&mut k).max_decimals(6));
            ui.label("+");
            let b = ui.add(crate::widgets::Num::new(&mut o).max_decimals(6));
            if a.changed() || b.changed() {
                let src = picks::Src {
                    from: m.from.clone(),
                    row: m.row.clone(),
                };
                act(Some((src, k, o)));
            }
        });
        if ui.button(tr("Remove pickup")).clicked() {
            act(None);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests;
