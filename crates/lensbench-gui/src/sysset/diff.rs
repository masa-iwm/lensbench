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
    /// 基本の並びの番号が map（前の番号 → 今の番号）で変わった（old 個から new 個へ）。状態の点を付いていかせる
    fn remap(&mut self, map: &[Option<usize>], old: usize, new: usize) {
        let at = std::mem::take(&mut self.at);
        self.at = at
            .into_iter()
            .filter_map(|(i, x)| match map.get(i) {
                Some(j) => j.map(|j| (j, x)),
                // 基本より後ろの状態だけの点は、数の差だけずらす
                None => (i + new).checked_sub(old).map(|j| (j, x)),
            })
            .collect();
        if let Some(n) = self.len {
            self.len = Some((n + new).saturating_sub(old).max(1));
        }
    }

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

/// 基本の並び old が new になったときの番号の対応（old の番号 → new の番号、消えた点は None）。
/// 隣の 2 点の入れ替えと、点を足す・消す（同じ値の点を前から合わせる）を見分ける。値を変えただけなら None
fn list_map<T: PartialEq>(old: &[T], new: &[T]) -> Option<Vec<Option<usize>>> {
    if old.len() == new.len() {
        let a = old.iter().zip(new).position(|(x, y)| x != y)?;
        let swapped = a + 1 < old.len()
            && old[a] == new[a + 1]
            && old[a + 1] == new[a]
            && old[a + 2..] == new[a + 2..];
        return swapped.then(|| {
            (0..old.len())
                .map(|i| {
                    Some(if i == a {
                        a + 1
                    } else if i == a + 1 {
                        a
                    } else {
                        i
                    })
                })
                .collect()
        });
    }
    // 最長の共通の並びで合わせる（点の数は少ない）
    let (n, m) = (old.len(), new.len());
    let mut t = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if old[i] == new[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let mut out = vec![None; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            out[i] = Some(j);
            (i, j) = (i + 1, j + 1);
        } else if t[i + 1][j] >= t[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Some(out)
}

impl crate::combo::Combo {
    /// 基本の視野・波長を足す・消す・入れ替えた後（before は変える前の基本）: 状態で変えた点を同じ点に付いていかせる
    pub(crate) fn base_lists_changed(&mut self, before: &SysSnap) {
        let now = self.snap();
        let fm = list_map(&before.fields, &now.fields);
        let wm = list_map(&before.waves, &now.waves);
        if fm.is_none() && wm.is_none() {
            return;
        }
        for d in self.state_sys.values_mut() {
            if let Some(m) = &fm {
                d.fields.remap(m, before.fields.len(), now.fields.len());
            }
            if let Some(m) = &wm {
                d.waves.remap(m, before.waves.len(), now.waves.len());
            }
        }
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
    /// 状態で無限遠にした（基本は有限）は Some(None)。保存では null と項目なしを分ける
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "some")]
    pub object_distance: Option<Option<f64>>,
    pub telecentric: Option<bool>,
    pub aiming: Option<Aiming>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "some")]
    pub afocal: Option<Option<String>>,
}

/// 項目があれば（null でも）Some
fn some<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 基本の点を消す・足す・入れ替えると、状態で変えた点は同じ点に付いていく
    #[test]
    fn state_points_follow_base_points() {
        let w = |v: &[f64]| v.iter().map(|&u| WavePt::um(u)).collect::<Vec<_>>();
        let mut base = SysSnap {
            waves: w(&[0.4, 0.5, 0.6]),
            ..Default::default()
        };
        let mut st = base.clone();
        st.waves[2].um = 0.65;
        let mut d = SysDiff::of(&base, &st);
        // 基本の 2 番目を消す
        let before = base.clone();
        base.waves.remove(1);
        let m = list_map(&before.waves, &base.waves).unwrap();
        assert_eq!(m, [Some(0), None, Some(1)]);
        d.waves.remap(&m, 3, 2);
        let um: Vec<f64> = d.apply(&base).waves.iter().map(|x| x.um).collect();
        assert_eq!(um, [0.4, 0.65]);
        // 前に足す
        let before = base.clone();
        base.waves.insert(0, WavePt::um(0.3));
        d.waves
            .remap(&list_map(&before.waves, &base.waves).unwrap(), 2, 3);
        let um: Vec<f64> = d.apply(&base).waves.iter().map(|x| x.um).collect();
        assert_eq!(um, [0.3, 0.4, 0.65]);
        // 入れ替える
        let before = base.clone();
        base.waves.swap(1, 2);
        d.waves
            .remap(&list_map(&before.waves, &base.waves).unwrap(), 3, 3);
        let um: Vec<f64> = d.apply(&base).waves.iter().map(|x| x.um).collect();
        assert_eq!(um, [0.3, 0.65, 0.4]);
        // 値を変えただけなら対応は無い（番号のまま）
        let mut b2 = base.clone();
        b2.waves[0].um = 0.35;
        assert!(list_map(&base.waves, &b2.waves).is_none());
    }

    /// 状態で無限遠にした物体距離は、JSON に書いて読んでも（元に戻す・記録の書き換えの形）残る
    #[test]
    fn infinite_object_distance_round_trips() {
        let d = SysDiff {
            object_distance: Some(None),
            ..Default::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["object_distance"], serde_json::Value::Null);
        let back: SysDiff = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
        let e: SysDiff = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(e.object_distance, None);
    }
}
