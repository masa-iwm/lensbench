//! 状態（多重構成の構成に当たる）を足す・写す・名前を変える・消す（状態の表・配置のパネルから）

use super::*;

/// 状態の名前 → 値の表の、状態 from の値を to にも（from が無ければ to を外す）
fn copy_key<V: Clone>(m: &mut BTreeMap<String, V>, from: &str, to: &str) {
    match m.get(from).cloned() {
        Some(v) => {
            m.insert(to.to_string(), v);
        }
        None => {
            m.remove(to);
        }
    }
}

fn rename_key<V>(m: &mut BTreeMap<String, V>, from: &str, to: &str) {
    if let Some(v) = m.remove(from) {
        m.insert(to.to_string(), v);
    }
}

/// 多重構成のピックアップ: 状態 to の行 key の値を、状態 from（空なら基本）の値 × scale ＋ offset にする
/// （数でない値は写すだけ）。key は行の持ち主（要素か組合せ）の中での名前（`face:面:欄[:番号]`・`air` など）。
/// 保存はプロジェクトの `gui.view.mc_picks`（要素の id ごと）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct McPick {
    pub key: String,
    pub to: String,
    pub from: String,
    pub scale: f64,
    pub offset: f64,
}

impl McPick {
    /// 面の値の行なら要素の面の番号
    pub(crate) fn face_surf(&self) -> Option<usize> {
        self.key
            .strip_prefix("face:")?
            .split(':')
            .next()?
            .parse()
            .ok()
    }

    /// 面の値の行の面の番号を変える
    fn set_face_surf(&mut self, i: usize) {
        if let Some(rest) = self.key.strip_prefix("face:") {
            let tail = rest.split_once(':').map_or("", |(_, t)| t);
            self.key = format!("face:{i}:{tail}");
        }
    }
}

/// ピックアップの状態の名前を変える・消す・写す
fn picks_rename(v: &mut [McPick], from: &str, to: &str) {
    for m in v {
        if m.to == from {
            m.to = to.to_string();
        }
        if m.from == from {
            m.from = to.to_string();
        }
    }
}

fn picks_copy(v: &mut Vec<McPick>, from: &str, to: &str) {
    let add: Vec<McPick> = v
        .iter()
        .filter(|m| m.to == from)
        .map(|m| McPick {
            to: to.to_string(),
            ..m.clone()
        })
        .collect();
    v.retain(|m| m.to != to);
    v.extend(add);
}

impl Pick {
    /// 面の並びが edits・ops を付けたときから変わっていれば、上書きを外して外した数を返す。
    /// 上書きが無ければ印を消し、印が無い（前の保存）上書きは今の並びのものとみなす
    pub(crate) fn drop_stale_edits(&mut self) -> usize {
        if self.edits.is_empty()
            && self.ops.is_empty()
            && self.state_edits.is_empty()
            && self.state_stop.is_empty()
            && !self.mc_picks.iter().any(|m| m.face_surf().is_some())
        {
            self.edits_for.clear();
            return 0;
        }
        let sig = self.surf_sig();
        if self.edits_for.is_empty() {
            self.edits_for = sig;
            return 0;
        }
        if self.edits_for == sig {
            return 0;
        }
        let n = self.edits.len()
            + self.ops.len()
            + self.state_edits.values().map(Vec::len).sum::<usize>()
            + self.state_stop.len()
            + self
                .mc_picks
                .iter()
                .filter(|m| m.face_surf().is_some())
                .count();
        self.edits.clear();
        self.ops.clear();
        self.state_edits.clear();
        self.state_stop.clear();
        self.mc_picks.retain(|m| m.face_surf().is_none());
        self.edits_for.clear();
        n
    }

    /// 面の番号を map で付け替える（ピックアップの面の行）
    pub(crate) fn shift_face_picks(&mut self, map: &dyn Fn(usize) -> Option<usize>) {
        self.mc_picks.retain_mut(|m| match m.face_surf() {
            None => true,
            Some(i) => map(i).map(|j| m.set_face_surf(j)).is_some(),
        });
    }

    /// 状態 n で変えた値があるか
    pub(crate) fn has_state_values(&self, n: &str) -> bool {
        self.state_on.contains_key(n)
            || self.state_air.contains_key(n)
            || self.state_place.contains_key(n)
            || self.zmx().state_config.contains_key(n)
            || self.state_edits.contains_key(n)
            || self.state_stop.contains_key(n)
            || self.mc_picks.iter().any(|m| m.to == n)
    }

    /// 状態 n で変えた値をやめる
    pub(crate) fn clear_state(&mut self, n: &str) {
        self.state_on.remove(n);
        self.state_air.remove(n);
        self.state_place.remove(n);
        self.state_edits.remove(n);
        self.state_stop.remove(n);
        self.mc_picks.retain(|m| m.to != n);
        if let Some(KindData::Zmx(z)) = self.data.as_deref_mut() {
            z.state_config.remove(n);
        }
    }
}

impl Combo {
    /// ピックアップの並びの全部（組合せ・要素）に f を当てる
    pub(crate) fn each_pick_list(&mut self, f: &mut dyn FnMut(&mut Vec<McPick>)) {
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                f(&mut p.mc_picks);
            }
        }
        f(&mut self.mc_picks);
    }

    /// 状態ごとの値の表の全部に f を当てる（状態の名前 → 値の表）
    fn each_state_map(&mut self, f: &mut dyn FnMut(&mut dyn StateMap)) {
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                f(&mut p.state_on);
                f(&mut p.state_air);
                f(&mut p.state_place);
                f(&mut p.state_edits);
                f(&mut p.state_stop);
                if let Some(KindData::Zmx(z)) = p.data.as_deref_mut() {
                    f(&mut z.state_config);
                    for g in &mut z.zgroups {
                        f(&mut g.state_shift);
                        f(&mut g.state_place);
                    }
                }
            }
        }
        for g in &mut self.groups {
            f(&mut g.state_shift);
        }
        f(&mut self.state_sys);
        f(&mut self.state_path);
        f(&mut self.state_more);
    }

    /// 状態 name を足す（名前が空・同じ名前があれば何もしない）。from があればその状態の値を写す
    /// （OpticStudio の構成の挿入と同じく、今の構成の写し）。足した状態を今の状態にする。足したか
    pub(crate) fn add_state(&mut self, name: &str, from: Option<&str>) -> bool {
        let name = name.trim();
        if name.is_empty() || self.states.iter().any(|s| s == name) {
            return false;
        }
        if let Some(f) = from.filter(|f| self.states.iter().any(|s| s == f)) {
            let f = f.to_string();
            self.each_state_map(&mut |m| m.copy(&f, name));
            self.each_pick_list(&mut |v| picks_copy(v, &f, name));
            let at = self
                .states
                .iter()
                .position(|s| *s == f)
                .map_or(self.states.len(), |i| i + 1);
            self.states.insert(at, name.to_string());
        } else {
            self.states.push(name.to_string());
        }
        self.state = Some(name.to_string());
        true
    }

    /// 状態 n を消す（状態ごとの値も）。今の状態なら基本に
    pub(crate) fn remove_state(&mut self, n: &str) {
        self.states.retain(|s| s != n);
        self.each_state_map(&mut |m| m.remove(n));
        self.each_pick_list(&mut |v| v.retain(|m| m.to != n && m.from != n));
        if self.state.as_deref() == Some(n) {
            self.state = None;
        }
    }

    /// 状態 from の名前を to にする（空・同じ名前があれば誤り）
    pub(crate) fn rename_state(&mut self, from: &str, to: &str) -> Result<(), String> {
        let to = to.trim();
        if to.is_empty() {
            return Err(tr("The state name is empty").into());
        }
        if to == from {
            return Ok(());
        }
        if self.states.iter().any(|s| s == to) {
            return Err(trf("There is already a state named {}", &[&to]));
        }
        let Some(i) = self.states.iter().position(|s| s == from) else {
            return Err(trf("No state {}", &[&from]));
        };
        self.states[i] = to.to_string();
        self.each_state_map(&mut |m| m.rename(from, to));
        self.each_pick_list(&mut |v| picks_rename(v, from, to));
        if self.state.as_deref() == Some(from) {
            self.state = Some(to.to_string());
        }
        Ok(())
    }

    /// どれかの状態が絞りの面を変えているか
    pub(crate) fn has_state_stop(&self) -> bool {
        self.all()
            .iter()
            .any(|l| self.get(l).is_some_and(|p| !p.state_stop.is_empty()))
    }

    /// 基本の絞り（要素の場所と要素の面の番号）。None は先頭に自動で置く絞り
    pub(crate) fn base_stop(&self) -> Option<(Loc, usize)> {
        self.stop_face().or_else(|| {
            self.all()
                .into_iter()
                .find(|l| self.get(l).is_some_and(Pick::is_stop))
                .map(|l| (l, 0))
        })
    }

    /// 状態 st（None は基本）の絞り
    pub(crate) fn stop_in(&self, st: Option<&str>) -> Option<(Loc, usize)> {
        st.and_then(|n| {
            self.all().into_iter().find_map(|l| {
                let i = *self.get(&l)?.state_stop.get(n)?;
                Some((l, i))
            })
        })
        .or_else(|| self.base_stop())
    }

    /// 状態 n の絞りを場所 loc の要素の面 i に（None なら状態の絞りをやめて基本の絞りに）
    pub(crate) fn set_state_stop(&mut self, n: &str, at: Option<(Loc, usize)>) {
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                p.state_stop.remove(n);
            }
        }
        if let Some((l, i)) = at {
            if let Some(p) = self.get_mut(&l) {
                p.state_stop.insert(n.to_string(), i);
            }
        }
    }

    /// 状態 n で要素 p のブロックの surface_ops を置き換える並び（状態の絞りが無ければ None）: どこかの要素が状態の
    /// 絞りを持てば、基本の set_stop を外し、絞りの要素には set_stop を足す（set_stop はブロックの絞りより優先）
    pub(crate) fn state_ops(&self, p: &Pick, n: &str) -> Option<Vec<SurfaceOp>> {
        let moved = self
            .all()
            .iter()
            .any(|l| self.get(l).is_some_and(|q| q.state_stop.contains_key(n)));
        if !moved {
            return None;
        }
        let has_stop = p.ops.iter().any(|o| o.stop_index().is_some());
        let mine = p.state_stop.get(n);
        if !has_stop && mine.is_none() {
            return None;
        }
        let mut ops: Vec<SurfaceOp> = p
            .ops
            .iter()
            .filter(|o| o.stop_index().is_none())
            .cloned()
            .collect();
        if let Some(&i) = mine {
            ops.push(SurfaceOp::SetStop(StopOp {
                surface: BlockRef::Sys(Ref::Index(i)),
            }));
        }
        Some(ops)
    }

    /// 状態 n の set のブロック id の項目に、状態の絞りの面（surface_ops）と面の上書き（surface_edits、基本の上書きに
    /// 状態の上書きを重ねたもの。状態の set はブロックの項目を丸ごと置き換える）を書く
    pub(crate) fn state_faces(
        &self,
        p: &Pick,
        n: &str,
        id: &str,
        set: &mut serde_json::Map<String, Value>,
    ) {
        if let Some(ops) = self.state_ops(p, n) {
            let e = set.entry(id.to_string()).or_insert_with(|| json!({}));
            e["surface_ops"] = serde_json::to_value(ops).unwrap_or_default();
        }
        if let Some(es) = p.edits_in(n) {
            let e = set.entry(id.to_string()).or_insert_with(|| json!({}));
            e["surface_edits"] = crate::combo_tree::edits_json(&es);
        }
    }

    /// 状態 n の名前の写しに使える名前（「n 2」「n 3」…で、まだ無いもの）
    /// 状態 n を d だけ動かす（列の入れ替え。状態は名前で指すので値・ピックアップはそのまま）。動かしたか
    pub(crate) fn move_state(&mut self, n: &str, d: isize) -> bool {
        let Some(i) = self.states.iter().position(|s| s == n) else {
            return false;
        };
        let j = i as isize + d;
        if j < 0 || j as usize >= self.states.len() {
            return false;
        }
        let s = self.states.remove(i);
        self.states.insert(j as usize, s);
        true
    }

    /// 状態 n を状態 after のすぐ右に置く
    pub(crate) fn place_state_after(&mut self, n: &str, after: &str) {
        if let Some(i) = self.states.iter().position(|s| s == n) {
            let s = self.states.remove(i);
            let at = self
                .states
                .iter()
                .position(|x| x == after)
                .map_or(self.states.len(), |k| k + 1);
            self.states.insert(at, s);
        }
    }

    /// 「Config N」の形の空いた名前
    pub(crate) fn next_config_name(&self) -> String {
        let name = format!("Config {}", self.states.len() + 1);
        if self.states.contains(&name) {
            self.free_state_name(&name)
        } else {
            name
        }
    }

    pub(crate) fn free_state_name(&self, base: &str) -> String {
        (2..)
            .map(|k| format!("{base} {k}"))
            .find(|s| !self.states.contains(s))
            .unwrap_or_default()
    }
}

/// 状態の名前 → 値の表（値の型によらずに写す・外す・名前を変える）
trait StateMap {
    fn copy(&mut self, from: &str, to: &str);
    fn remove(&mut self, n: &str);
    fn rename(&mut self, from: &str, to: &str);
}

impl<V: Clone> StateMap for BTreeMap<String, V> {
    fn copy(&mut self, from: &str, to: &str) {
        copy_key(self, from, to)
    }
    fn remove(&mut self, n: &str) {
        BTreeMap::remove(self, n);
    }
    fn rename(&mut self, from: &str, to: &str) {
        rename_key(self, from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状態を写して足すと状態ごとの値も写り、名前を変える・消すと値も付いていく
    #[test]
    fn add_copy_rename_remove_state() {
        let mut c = Combo::default();
        c.picks.push(Pick::new(ElemKind::Plate));
        c.states = vec!["A".into()];
        c.picks[0].state_air.insert("A".into(), 7.0);
        c.picks[0].set_state_edit("A", 0, "r", serde_json::json!(40.0));
        assert!(c.add_state("B", Some("A")));
        assert_eq!(c.states, ["A", "B"]);
        assert_eq!(c.state.as_deref(), Some("B"));
        assert_eq!(c.picks[0].air_in(Some("B")), 7.0);
        assert_eq!(
            c.picks[0].state_edit("B", 0, "r"),
            Some(&serde_json::json!(40.0))
        );
        assert!(!c.add_state("B", None));
        c.rename_state("B", "C").unwrap();
        assert!(c.rename_state("C", "A").is_err());
        assert_eq!(c.picks[0].air_in(Some("C")), 7.0);
        assert!(!c.picks[0].state_air.contains_key("B"));
        c.remove_state("C");
        assert_eq!(c.states, ["A"]);
        assert_eq!(c.state, None);
        assert!(!c.picks[0].state_edits.contains_key("C"));
    }
}
