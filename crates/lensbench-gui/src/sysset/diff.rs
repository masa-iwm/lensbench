//! 状態で変えた系の値（[`super::SysSnap`] との差）。変えていない値は基本の値に従う

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Aiming, ApertureKind, FieldKind, FieldPt, FieldSolve, SysSnap, WavePt};

/// 並びの値（視野・波長）の差: 数を変えたか（len）と、変えた番号の点
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ListDiff<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len: Option<usize>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub at: BTreeMap<usize, T>,
}

impl<T> Default for ListDiff<T> {
    fn default() -> Self {
        ListDiff {
            len: None,
            at: BTreeMap::new(),
        }
    }
}

impl<T: Clone + PartialEq> ListDiff<T> {
    fn of(base: &[T], s: &[T]) -> Self {
        let at = s
            .iter()
            .enumerate()
            .filter(|(i, x)| base.get(*i) != Some(*x))
            .map(|(i, x)| (i, x.clone()))
            .collect();
        ListDiff {
            len: (s.len() != base.len()).then_some(s.len()),
            at,
        }
    }

    /// 基本の並び base に当てる。基本より長くした分で点の無い番号は、すぐ前の点を写す
    fn apply(&self, base: &[T]) -> Vec<T> {
        let n = self.len.unwrap_or(base.len());
        let mut out: Vec<T> = Vec::with_capacity(n);
        for i in 0..n {
            let x = match (self.at.get(&i), base.get(i), out.last()) {
                (Some(x), _, _) | (None, Some(x), _) | (None, None, Some(x)) => x.clone(),
                (None, None, None) => break,
            };
            out.push(x);
        }
        out
    }
}

/// 状態で変えた系の値。None の項目は基本の値
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SysDiff {
    pub aperture: Option<ApertureKind>,
    pub ap_value: Option<f64>,
    pub epd: Option<f64>,
    pub field_kind: Option<FieldKind>,
    pub field_solve: Option<FieldSolve>,
    pub fields: ListDiff<FieldPt>,
    pub waves: ListDiff<WavePt>,
    pub primary: Option<usize>,
    pub object_distance: Option<Option<f64>>,
    pub telecentric: Option<bool>,
    pub aiming: Option<Aiming>,
    pub afocal: Option<Option<String>>,
}

fn d<T: PartialEq + Clone>(b: &T, s: &T) -> Option<T> {
    (b != s).then(|| s.clone())
}

impl SysDiff {
    /// 基本の値 base と状態の値 s の差
    pub(crate) fn of(base: &SysSnap, s: &SysSnap) -> SysDiff {
        SysDiff {
            aperture: d(&base.aperture, &s.aperture),
            ap_value: d(&base.ap_value, &s.ap_value),
            epd: d(&base.epd, &s.epd),
            field_kind: d(&base.field_kind, &s.field_kind),
            field_solve: d(&base.field_solve, &s.field_solve),
            fields: ListDiff::of(&base.fields, &s.fields),
            waves: ListDiff::of(&base.waves, &s.waves),
            primary: d(&base.primary, &s.primary),
            object_distance: d(&base.object_distance, &s.object_distance),
            telecentric: d(&base.telecentric, &s.telecentric),
            aiming: d(&base.aiming, &s.aiming),
            afocal: d(&base.afocal, &s.afocal),
        }
    }

    /// 基本の値 base に当てた状態の値
    pub(crate) fn apply(&self, base: &SysSnap) -> SysSnap {
        SysSnap {
            aperture: self.aperture.unwrap_or(base.aperture),
            ap_value: self.ap_value.unwrap_or(base.ap_value),
            epd: self.epd.unwrap_or(base.epd),
            field_kind: self.field_kind.unwrap_or(base.field_kind),
            field_solve: self
                .field_solve
                .clone()
                .unwrap_or_else(|| base.field_solve.clone()),
            fields: self.fields.apply(&base.fields),
            waves: self.waves.apply(&base.waves),
            primary: self.primary.unwrap_or(base.primary),
            object_distance: self.object_distance.unwrap_or(base.object_distance),
            telecentric: self.telecentric.unwrap_or(base.telecentric),
            aiming: self.aiming.unwrap_or(base.aiming),
            afocal: self.afocal.clone().unwrap_or_else(|| base.afocal.clone()),
        }
    }

    /// 何も変えていないか
    pub(crate) fn is_empty(&self) -> bool {
        *self == SysDiff::default()
    }
}
