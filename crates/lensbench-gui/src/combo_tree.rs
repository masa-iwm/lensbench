//! 組合せの分岐: BS の口 B・C・D の先の要素の並び（分岐の木）、BS を通る順の経路、
//! 今の経路に沿った要素の並び（[`Combo::chain`]）。
//!
//! 組合せの要素は木になる。最上位の並び [`Combo::picks`] の BS が分岐を持つと（[`Pick::branched`]）、
//! その BS は並びの最後に置き、口 B（透過）・C（反射）・D（B から戻る光の反射）の先にそれぞれの並び
//! [`Branch`] を持つ。図・キー・面の表は「今の経路に沿った要素の並び」を番号で扱う（分岐が無ければ
//! 最上位の並びそのもの）。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::combo::{Combo, ElemKind, GroupDef, Pick};

/// BS の口の先の並び
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Branch {
    /// BS の口から最初の要素まで（要素が無ければ像面まで）[mm]
    pub air: f64,
    pub picks: Vec<Pick>,
}

impl Default for Branch {
    fn default() -> Self {
        Branch {
            air: 20.0,
            picks: Vec::new(),
        }
    }
}

/// 口の名前（[`Pick::ports`] の番号の順）
pub const PORTS: [&str; 3] = ["B", "C", "D"];

/// 要素の場所: 並びの中の番号と口の番号（0 = B, 1 = C, 2 = D）を交互に並べる。
/// `[i]` は最上位の i 番目、`[i, p, j]` は最上位の i 番目の BS の口 p の先の j 番目
pub type Loc = Vec<usize>;

/// 間隔（空気）の場所
#[derive(Debug, Clone, PartialEq)]
pub enum Gap {
    /// 前の絞りから最初の要素まで
    Stop,
    /// 要素の後の空気
    Air(Loc),
    /// BS（場所）の口の先の最初の空気
    Lead(Loc, usize),
}

/// 今の経路に沿った要素の 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub loc: Loc,
    /// BS: 光が出る口（0 = B, 1 = C）
    pub port: Option<usize>,
    /// 前の間隔（無ければ動かしても前は変えない）
    pub before: Option<Gap>,
    /// 後の間隔（往路を戻る鏡は無し）
    pub after: Option<Gap>,
}

enum Flow {
    /// 並びの終わりまで進んだ
    On,
    /// 往路を戻る鏡で戻り始めた（並びの中で通った分岐の無い BS の数）
    Back(usize),
}

impl Pick {
    /// 分岐を持つ BS
    pub fn is_branched(&self) -> bool {
        self.kind == ElemKind::Splitter && self.branched
    }
}

/// 経路の文字（T・R）。足りなければ BS の既定
fn next_letter(letters: &mut std::vec::IntoIter<char>, p: &Pick) -> char {
    letters.next().unwrap_or(if p.arm_r { 'R' } else { 'T' })
}

fn walk(
    list: &[Pick],
    prefix: &[usize],
    letters: &mut std::vec::IntoIter<char>,
    out: &mut Vec<Step>,
) -> Flow {
    let mut passed = 0;
    for (i, p) in list.iter().enumerate() {
        let mut loc = prefix.to_vec();
        loc.push(i);
        if p.kind == ElemKind::Splitter {
            let port = usize::from(next_letter(letters, p) == 'R');
            if !p.is_branched() {
                passed += 1;
                out.push(Step {
                    loc: loc.clone(),
                    port: Some(port),
                    before: None,
                    after: Some(Gap::Air(loc)),
                });
                continue;
            }
            out.push(Step {
                loc: loc.clone(),
                port: Some(port),
                before: None,
                after: Some(Gap::Lead(loc.clone(), port)),
            });
            let mut sub = loc.clone();
            sub.push(port);
            return match walk(&p.ports[port].picks, &sub, letters, out) {
                Flow::On => Flow::On,
                Flow::Back(n) => {
                    // 戻る途中で通る分岐の無い BS の文字（A へ戻るとする）
                    for _ in 0..n {
                        letters.next();
                    }
                    // B から戻る: 透過で A・反射で D。C から戻る: 透過で D・反射で A
                    let l = next_letter(letters, p);
                    if (port == 0) == (l == 'R') {
                        let start = out.len();
                        let mut d = loc.clone();
                        d.push(2);
                        walk(&p.ports[2].picks, &d, letters, out);
                        if let Some(s) = out.get_mut(start) {
                            s.before = Some(Gap::Lead(loc, 2));
                        }
                        Flow::On
                    } else {
                        Flow::Back(passed)
                    }
                }
            };
        }
        let back = p.kind == ElemKind::Mirror && p.back;
        out.push(Step {
            loc: loc.clone(),
            port: None,
            before: None,
            after: (!back).then_some(Gap::Air(loc)),
        });
        if back {
            return Flow::Back(passed);
        }
    }
    Flow::On
}

impl Combo {
    /// 場所の並び（場所の長さは偶数: 番号と口の組の並び）
    pub fn list_at(&self, prefix: &[usize]) -> Option<&Vec<Pick>> {
        let mut l = &self.picks;
        for c in prefix.chunks(2) {
            l = &l.get(c[0])?.ports.get(c[1])?.picks;
        }
        Some(l)
    }

    pub fn list_at_mut(&mut self, prefix: &[usize]) -> Option<&mut Vec<Pick>> {
        let mut l = &mut self.picks;
        for c in prefix.chunks(2) {
            l = &mut l.get_mut(c[0])?.ports.get_mut(c[1])?.picks;
        }
        Some(l)
    }

    pub fn get(&self, loc: &[usize]) -> Option<&Pick> {
        let (last, prefix) = loc.split_last()?;
        self.list_at(prefix)?.get(*last)
    }

    pub fn get_mut(&mut self, loc: &[usize]) -> Option<&mut Pick> {
        let (last, prefix) = loc.split_last()?;
        self.list_at_mut(prefix)?.get_mut(*last)
    }

    /// 全部の要素の場所（木を深さ優先で。BS の後に口 B・C・D の先）
    pub fn all(&self) -> Vec<Loc> {
        fn go(list: &[Pick], prefix: &[usize], out: &mut Vec<Loc>) {
            for (i, p) in list.iter().enumerate() {
                let mut loc = prefix.to_vec();
                loc.push(i);
                out.push(loc.clone());
                if p.is_branched() {
                    for (k, b) in p.ports.iter().enumerate() {
                        let mut sub = loc.clone();
                        sub.push(k);
                        go(&b.picks, &sub, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        go(&self.picks, &[], &mut out);
        out
    }

    /// 今の経路（[`Combo::path`]）に沿った要素の並び。往路を戻る鏡で戻り、BS で口 D へ出れば D の先へ続く
    pub fn chain(&self) -> Vec<Step> {
        let letters: Vec<char> = self
            .path
            .chars()
            .map(|c| c.to_ascii_uppercase())
            .filter(|c| matches!(c, 'T' | 'R'))
            .collect();
        let mut it = letters.into_iter();
        let mut out = Vec::new();
        walk(&self.picks, &[], &mut it, &mut out);
        for k in 0..out.len() {
            if out[k].before.is_some() {
                continue;
            }
            out[k].before = if k == 0 {
                self.lead_stop().then_some(Gap::Stop)
            } else {
                out[k - 1].after.clone()
            };
        }
        out
    }

    /// 経路に沿った要素の数
    pub fn len(&self) -> usize {
        self.chain().len()
    }

    pub fn is_empty(&self) -> bool {
        self.picks.is_empty()
    }

    /// 経路に沿った k 番目の要素
    pub fn el(&self, k: usize) -> Option<&Pick> {
        let c = self.chain();
        self.get(&c.get(k)?.loc)
    }

    pub fn el_mut(&mut self, k: usize) -> Option<&mut Pick> {
        let c = self.chain();
        let loc = c.get(k)?.loc.clone();
        self.get_mut(&loc)
    }

    /// 経路に沿った要素（写し）
    pub fn chain_picks(&self) -> Vec<Pick> {
        self.chain()
            .iter()
            .filter_map(|s| self.get(&s.loc).cloned())
            .collect()
    }

    /// 間隔を変える。要素の後の空気は、状態を選んでいればその状態の値（無ければ作る）
    pub fn gap(&mut self, g: &Gap) -> Option<&mut f64> {
        let st = self.active_state().map(str::to_string);
        match g {
            Gap::Stop => Some(&mut self.stop_gap),
            Gap::Air(l) => self.get_mut(l).map(|p| p.air_mut(st.as_deref())),
            Gap::Lead(l, k) => self.get_mut(l)?.ports.get_mut(*k).map(|b| &mut b.air),
        }
    }

    /// 間隔の今の値（状態の値。読むだけで状態の値を作らない）
    pub fn gap_val(&self, g: &Gap) -> Option<f64> {
        match g {
            Gap::Stop => Some(self.stop_gap),
            Gap::Air(l) => self.get(l).map(|p| p.air_in(self.active_state())),
            Gap::Lead(l, k) => self.get(l)?.ports.get(*k).map(|b| b.air),
        }
    }

    /// 経路に沿った k 番目の要素の後の間隔の値
    pub fn gap_after_val(&self, k: usize) -> Option<f64> {
        let g = self.chain().get(k)?.after.clone()?;
        self.gap_val(&g)
    }

    /// 経路に沿った k 番目の要素の後の間隔
    pub fn gap_after(&mut self, k: usize) -> Option<&mut f64> {
        let g = self.chain().get(k)?.after.clone()?;
        self.gap(&g)
    }

    /// 像面までの空気（経路の最後の要素の後。最上位の往路を戻る鏡なら、戻った後の空気）
    pub fn image_gap(&mut self) -> Option<&mut f64> {
        let last = self.chain().pop()?;
        match last.after {
            Some(g) => self.gap(&g),
            None if last.loc.len() == 1 => {
                let st = self.active_state().map(str::to_string);
                self.get_mut(&last.loc).map(|p| p.air_mut(st.as_deref()))
            }
            None => None,
        }
    }

    /// 要素のブロックの id（場所と組）。カタログの玉は型番（重なれば）、ほかは種類の頭と番号。
    /// 木の全部の要素で重ならない
    pub fn ids_all(&self) -> Vec<(Loc, String)> {
        let mut ids: Vec<(Loc, String)> = Vec::new();
        for loc in self.all() {
            let p = self.get(&loc).expect("all の場所");
            let base = if p.kind == ElemKind::Catalog && !p.part.is_empty() {
                p.part.clone()
            } else {
                p.kind.prefix().to_string()
            };
            let numbered = p.kind != ElemKind::Catalog;
            let mut k = 1;
            let mut id = if numbered {
                format!("{base}{k}")
            } else {
                base.clone()
            };
            while ids.iter().any(|x| x.1 == id) {
                k += 1;
                id = if numbered {
                    format!("{base}{k}")
                } else {
                    format!("{base}#{k}")
                };
            }
            ids.push((loc, id));
        }
        ids
    }

    /// 要素のブロックの並び（分岐は BS のブロックの t・r・d に）と面の上書き
    /// 木のブロックを blocks へ。群のブロックにした名前を返す
    pub(crate) fn blocks(
        &self,
        blocks: &mut Vec<Value>,
        edits: &mut Vec<Value>,
    ) -> std::collections::BTreeSet<String> {
        let ids = self.ids_all();
        let id_of = |loc: &[usize]| {
            ids.iter()
                .find(|x| x.0 == loc)
                .map(|x| x.1.clone())
                .unwrap_or_default()
        };
        let names: Vec<String> = ids.iter().map(|x| x.1.clone()).collect();
        let mut used = std::collections::BTreeSet::new();
        let cx = Cx {
            id_of: &id_of,
            names: &names,
            groups: &self.groups,
        };
        go(&self.picks, &[], &cx, &mut used, blocks, edits);
        used
    }

    /// 分岐のある BS の後の要素は、その BS の既定の口（透過なら B、反射なら C）の先の頭へ移す。
    /// カタログの BS の中身（`extra`）のうち欄のある項目は欄へ
    pub fn normalize(&mut self) {
        fn go(list: &mut Vec<Pick>) {
            if let Some(i) = list.iter().position(Pick::is_branched) {
                if i + 1 < list.len() {
                    let rest: Vec<Pick> = list.drain(i + 1..).collect();
                    let bs = &mut list[i];
                    let port = usize::from(bs.arm_r);
                    bs.ports[port].picks.splice(0..0, rest);
                }
            }
            for p in list.iter_mut() {
                p.absorb_extra();
                for b in p.ports.iter_mut() {
                    go(&mut b.picks);
                }
            }
        }
        go(&mut self.picks);
    }

    /// BS（場所）の分岐を作る・やめる。作るときは、並びの BS の後の要素を既定の口の先へ移す
    /// （BS の後の空気は口の先の最初の空気に）。やめるときは既定の口の先を BS の後へ戻し、ほかの口の先は捨てる
    pub fn set_branched(&mut self, loc: &[usize], on: bool) {
        let Some((&i, prefix)) = loc.split_last() else {
            return;
        };
        let Some(list) = self.list_at_mut(prefix) else {
            return;
        };
        if i >= list.len() || list[i].kind != ElemKind::Splitter || list[i].branched == on {
            return;
        }
        let port = usize::from(list[i].arm_r);
        if on {
            let rest: Vec<Pick> = list.drain(i + 1..).collect();
            let bs = &mut list[i];
            bs.branched = true;
            bs.ports[port] = Branch {
                air: bs.air,
                picks: rest,
            };
        } else {
            let bs = &mut list[i];
            bs.branched = false;
            let b = std::mem::take(&mut bs.ports[port]);
            bs.air = b.air;
            for x in bs.ports.iter_mut() {
                *x = Branch::default();
            }
            list.extend(b.picks);
        }
    }

    /// 経路に沿った at 番目の前に差し込む（at が数以上なら最後の要素の後）。間に入れるときは前の空気を
    /// 半分ずつ分ける。差し込んだ要素の、経路に沿った番号を返す
    pub fn insert_chain(&mut self, at: usize, p: Pick) -> usize {
        self.insert_chain_with(at, p, true)
    }

    /// [`Combo::insert_chain`]。split が false なら前の空気を分けない
    pub fn insert_chain_with(&mut self, at: usize, mut p: Pick, split: bool) -> usize {
        let chain = self.chain();
        if chain.is_empty() {
            self.picks.push(p);
            return 0;
        }
        let mut at = at.min(chain.len());
        // 往路を戻る鏡の後ろには置けない（鏡の前、腕の中へ）
        if at == chain.len() && chain[at - 1].after.is_none() {
            at -= 1;
        }
        // 差し込む並びと位置、前の空気
        let (prefix, idx, before) = if at < chain.len() {
            let loc = &chain[at].loc;
            let (i, pre) = loc.split_last().expect("場所");
            (pre.to_vec(), *i, chain[at].before.clone())
        } else {
            let last = &chain[at - 1];
            match (
                self.get(&last.loc).is_some_and(Pick::is_branched),
                last.port,
            ) {
                // 分岐のある BS の後: 今の経路の口の先の頭
                (true, Some(port)) => {
                    let mut pre = last.loc.clone();
                    pre.push(port);
                    (pre, 0, last.after.clone())
                }
                _ => {
                    let (i, pre) = last.loc.split_last().expect("場所");
                    (pre.to_vec(), i + 1, None)
                }
            }
        };
        if let Some(g) = before.filter(|_| split) {
            if at > 0 || g != Gap::Stop {
                if let Some(v) = self.gap(&g) {
                    let half = *v / 2.0;
                    *v = half;
                    p.air = half;
                }
            }
        }
        if let Some(list) = self.list_at_mut(&prefix) {
            let idx = idx.min(list.len());
            list.insert(idx, p);
        }
        let mut loc = prefix;
        loc.push(idx);
        self.chain().iter().position(|s| s.loc == loc).unwrap_or(at)
    }

    /// 経路に沿った k 番目の要素を外す（分岐のある BS なら分岐ごと）
    pub fn remove_chain(&mut self, k: usize) {
        if let Some(s) = self.chain().get(k) {
            let (i, pre) = s.loc.split_last().expect("場所");
            if let Some(l) = self.list_at_mut(pre) {
                l.remove(*i);
            }
        }
    }

    /// 経路に沿った k 番目の要素を、同じ並びの中で前（-1）・後（+1）と入れ替える。入れ替えた先の、経路に
    /// 沿った番号を返す
    pub fn swap_chain(&mut self, k: usize, dir: i32) -> Option<usize> {
        let s = self.chain().get(k)?.clone();
        let (&i, pre) = s.loc.split_last()?;
        let l = self.list_at_mut(pre)?;
        let j = if dir < 0 { i.checked_sub(1)? } else { i + 1 };
        if j >= l.len() || l[i].is_branched() || l[j].is_branched() {
            return None;
        }
        l.swap(i, j);
        let mut loc = pre.to_vec();
        loc.push(j);
        self.chain().iter().position(|x| x.loc == loc)
    }

    /// 経路に沿った k 番目の要素を同じ並びの中で前・後と入れ替えられるか
    pub fn can_swap(&self, k: usize, dir: i32) -> bool {
        let Some(s) = self.chain().get(k).cloned() else {
            return false;
        };
        let Some((&i, pre)) = s.loc.split_last() else {
            return false;
        };
        let Some(l) = self.list_at(pre) else {
            return false;
        };
        let j = if dir < 0 {
            match i.checked_sub(1) {
                Some(j) => j,
                None => return false,
            }
        } else {
            i + 1
        };
        j < l.len() && !l[i].is_branched() && !l[j].is_branched()
    }

    /// 場所の分かりやすい名前（「BS1 の C の先」など）
    pub fn where_label(&self, loc: &[usize]) -> String {
        let ids = self.ids_all();
        let mut v = Vec::new();
        for i in (0..loc.len().saturating_sub(1)).step_by(2) {
            let bs = &loc[..=i];
            let id = ids
                .iter()
                .find(|x| x.0 == bs)
                .map(|x| x.1.clone())
                .unwrap_or_default();
            v.push(format!("{id}:{}", PORTS[loc[i + 1].min(2)]));
        }
        v.join(" › ")
    }
}

impl Combo {
    /// 例: Twyman-Green 干渉計（examples の P16 と同じ並び。通し確認・テスト）。絞り → キューブ BS、
    /// 口 B の先に試験レンズと往路を戻る鏡、口 C の先に参照の鏡、口 D の先は像面まで 50 mm
    pub fn twyman_green() -> Combo {
        let mut c = Combo {
            name: "Twyman-Green".into(),
            ..Combo::default()
        };
        let mut bs = Pick::new(ElemKind::Splitter);
        bs.t = 20.0;
        bs.branched = true;
        bs.ports[0].air = 30.0;
        bs.ports[0].picks = vec![
            {
                let mut l = Pick::new(ElemKind::Lens);
                l.r1 = 60.0;
                l.r2 = -60.0;
                l.t = 5.0;
                l.semi = 10.0;
                l.air = 40.0;
                l
            },
            {
                let mut m = Pick::new(ElemKind::Mirror);
                m.tilt = [0.0; 3];
                m.back = true;
                m
            },
        ];
        bs.ports[1].air = 75.0;
        bs.ports[1].picks = vec![{
            let mut m = Pick::new(ElemKind::Mirror);
            m.tilt = [0.0; 3];
            m.back = true;
            m
        }];
        bs.ports[2].air = 50.0;
        let mut stop = Pick::new(ElemKind::Stop);
        stop.semi = 3.0;
        stop.air = 20.0;
        c.picks.push(stop);
        c.picks.push(bs);
        c
    }
}

/// 木からブロックを作るときの共通の値
struct Cx<'a> {
    id_of: &'a dyn Fn(&[usize]) -> String,
    /// 要素の id（群の名前と同じなら群にしない）
    names: &'a [String],
    groups: &'a [GroupDef],
}

/// 並び list のブロックを out へ。続く同じ群の要素は群のブロックにまとめる（群の最後の要素の後の空気は群の外）。
/// 同じ名前の群は最初の続きだけ（離れた要素・分岐のある BS・分岐の中の往路を戻る鏡は群にしない）
fn go(
    list: &[Pick],
    prefix: &[usize],
    cx: &Cx,
    used: &mut std::collections::BTreeSet<String>,
    out: &mut Vec<Value>,
    edits: &mut Vec<Value>,
) {
    // 要素ごとに（群の名前、ブロック、後の空気）
    let mut parts: Vec<(String, Value, Option<Value>)> = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let mut loc = prefix.to_vec();
        loc.push(i);
        let id = (cx.id_of)(&loc);
        let mut b = p.block(&id);
        for e in &p.edits {
            edits.push(json!({ "block": id, "surface": e.surface, "fields": { e.field.clone(): e.value } }));
        }
        if p.is_branched() {
            for (k, key) in ["t", "r", "d"].into_iter().enumerate() {
                let mut sub = loc.clone();
                sub.push(k);
                let mut v = vec![json!({ "type": "air", "t": p.ports[k].air })];
                go(&p.ports[k].picks, &sub, cx, used, &mut v, edits);
                b[key] = Value::Array(v);
            }
            // 分岐のある BS は並びの最後
            parts.push((String::new(), b, None));
            break;
        }
        // 分岐の中の往路を戻る鏡の後は、BS へ戻る（後の空気は置かない）
        if p.kind == ElemKind::Mirror && p.back && !prefix.is_empty() {
            parts.push((String::new(), b, None));
            continue;
        }
        let mut air = json!({ "type": "air", "t": p.air });
        // 状態で変える空気には id を付ける
        if !p.state_air.is_empty() {
            air["id"] = json!(format!("{id}_air"));
        }
        let g = p.group.trim();
        let g = if g.is_empty() || cx.names.iter().any(|n| n == g) {
            String::new()
        } else {
            g.to_string()
        };
        parts.push((g, b, Some(air)));
    }
    let mut i = 0;
    while i < parts.len() {
        let g = parts[i].0.clone();
        let e = (i..parts.len())
            .find(|&j| parts[j].0 != g)
            .unwrap_or(parts.len());
        if g.is_empty() || !used.insert(g.clone()) {
            for (_, b, air) in &parts[i..e] {
                out.push(b.clone());
                out.extend(air.clone());
            }
            i = e;
            continue;
        }
        let mut inner = Vec::new();
        for (j, (_, b, air)) in parts[i..e].iter().enumerate() {
            inner.push(b.clone());
            if i + j + 1 < e {
                inner.extend(air.clone());
            }
        }
        let mut b = json!({ "type": "group", "id": g, "blocks": inner });
        if let Some(d) = cx.groups.iter().find(|d| d.name == g) {
            if !d.role.is_empty() {
                b["role"] = json!(d.role);
            }
            if d.shift != 0.0 {
                b["shift"] = json!(d.shift);
            }
        }
        out.push(b);
        out.extend(parts[e - 1].2.clone());
        i = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn twyman() -> Combo {
        let mut c = Combo::twyman_green();
        c.picks[0].air = Pick::new(ElemKind::Stop).air;
        c
    }

    #[test]
    fn chain_follows_the_path() {
        let mut c = twyman();
        let locs = |c: &Combo| c.chain().into_iter().map(|s| s.loc).collect::<Vec<_>>();
        // 既定（T）: 透過の腕へ、戻って透過で A（D へは出ない）
        assert_eq!(
            locs(&c),
            vec![vec![0], vec![1], vec![1, 0, 0], vec![1, 0, 1]]
        );
        c.path = "R".into();
        assert_eq!(locs(&c), vec![vec![0], vec![1], vec![1, 1, 0]]);
        // TR: 透過の腕で戻り、反射で D へ（D は空なので要素は増えない）
        c.picks[1].ports[2].picks.push(Pick::new(ElemKind::Lens));
        c.path = "TR".into();
        let ch = c.chain();
        assert_eq!(ch.last().unwrap().loc, vec![1, 2, 0]);
        assert_eq!(ch.last().unwrap().before, Some(Gap::Lead(vec![1], 2)));
        // BS の後の間隔は今の経路の口の先の最初の空気
        assert_eq!(ch[1].after, Some(Gap::Lead(vec![1], 0)));
        assert_eq!(ch[2].before, Some(Gap::Lead(vec![1], 0)));
        // 往路を戻る鏡の後は間隔なし
        assert_eq!(ch[3].after, None);
    }

    #[test]
    fn project_has_branches() {
        let mut c = twyman();
        c.path = "TR".into();
        let v = c.project();
        let b = &v["blocks"];
        assert_eq!(v["path"], "TR");
        // 前の絞りは無い（絞りの要素がある）
        assert_eq!(b[0]["type"], "stop");
        assert_eq!(b[2]["type"], "splitter");
        assert_eq!(
            b.as_array().unwrap().len(),
            3,
            "分岐のある BS の後に空気は無い"
        );
        let bs = &b[2];
        assert_eq!(bs["t"][0], json!({ "type": "air", "t": 30.0 }));
        assert_eq!(bs["t"][1]["type"], "lens");
        assert_eq!(bs["t"][3]["type"], "mirror");
        assert_eq!(bs["t"][3]["back"], true);
        assert_eq!(
            bs["t"].as_array().unwrap().len(),
            4,
            "戻る鏡の後に空気は無い"
        );
        assert_eq!(bs["r"][0]["t"], 75.0);
        assert_eq!(bs["d"], json!([{ "type": "air", "t": 50.0 }]));
        // id は木の全部で重ならない
        let ids: Vec<String> = c.ids_all().into_iter().map(|x| x.1).collect();
        assert_eq!(ids, ["STO1", "BS1", "L1", "M1", "M2"]);
    }

    #[test]
    fn branching_moves_the_rest() {
        let mut c = Combo {
            picks: vec![
                Pick::new(ElemKind::Splitter),
                Pick::new(ElemKind::Lens),
                Pick::new(ElemKind::Plate),
            ],
            ..Combo::default()
        };
        c.picks[0].air = 12.0;
        c.set_branched(&[0], true);
        assert_eq!(c.picks.len(), 1);
        assert_eq!(c.picks[0].ports[0].air, 12.0);
        assert_eq!(c.picks[0].ports[0].picks.len(), 2);
        assert_eq!(c.len(), 3);
        c.set_branched(&[0], false);
        assert_eq!(c.picks.len(), 3);
        assert_eq!(c.picks[0].air, 12.0);
        // 分岐のある BS の後に要素があれば（並べ替えなど）、既定の口の先へ
        c.set_branched(&[0], true);
        c.picks[0].arm_r = true;
        c.picks.push(Pick::new(ElemKind::Stop));
        c.normalize();
        assert_eq!(c.picks.len(), 1);
        assert_eq!(c.picks[0].ports[1].picks[0].kind, ElemKind::Stop);
    }

    /// 組合せから作ったプロジェクトを serve で読み、図の要素の範囲を当てる
    #[test]
    fn builds_through_serve() {
        use crate::combo::{BsForm, BsSplit, LayoutData};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let dir = std::env::temp_dir().join(format!("lensbench-tree-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("combo.json");
        let build = |c: &Combo| {
            lensbench_core::vfs::write(&f, c.project().to_string()).unwrap();
            let load = json!({ "file": f.display().to_string(), "glass_dir": [glass] });
            lensbench_engine::preview_layout(&load, &json!({}))
        };
        let mut c = twyman();
        c.object_distance = Some(100.0);
        c.fields = [0.0, 0.5].map(crate::sysset::FieldPt::y).to_vec();
        c.field_kind = crate::sysset::FieldKind::ObjectHeight;
        c.aperture = crate::sysset::ApKind::Na;
        c.ap_value = 0.02;
        for path in ["", "T", "R", "TR", "RT", "RR"] {
            c.path = path.into();
            let v = build(&c).unwrap_or_else(|e| panic!("{path}: {e:?}"));
            let d = LayoutData::parse(&v);
            let spans = d.spans_for(&c.ids(), c.lead_stop());
            // 経路に沿った要素は全部図の面に当たり、前から順に並ぶ（戻りの面は当てない）
            assert!(spans.iter().all(Option::is_some), "{path}: {spans:?}");
            let starts: Vec<usize> = spans.iter().map(|s| s.unwrap()[0]).collect();
            assert!(starts.windows(2).all(|w| w[0] < w[1]), "{path}: {spans:?}");
        }
        // BS の形・分け方・膜・くさび
        let mut c = Combo::default();
        let mut bs = Pick::new(ElemKind::Splitter);
        bs.air = 20.0;
        c.picks.push(bs);
        c.picks.push(Pick::new(ElemKind::Lens));
        for form in [BsForm::Cube, BsForm::Plate, BsForm::Ideal] {
            for split in [BsSplit::Ratio, BsSplit::Coat, BsSplit::Pbs] {
                let bs = &mut c.picks[0];
                bs.form = form;
                bs.t = if form == BsForm::Plate { 2.0 } else { 12.7 };
                bs.split = Some(split);
                bs.coat = "I.s=0.3,p=0.4".into();
                bs.wedge = [0.5, 0.0];
                bs.bs_angle = [40.0, 3.0];
                bs.ar = "I.s=1,p=1".into();
                bs.faces.insert("b".into(), "I.s=1,p=1".into());
                bs.faces.insert("back".into(), "I.s=1,p=1".into());
                let b = &c.project()["blocks"][2];
                match split {
                    BsSplit::Ratio => assert_eq!(b["ratio"], 0.5),
                    BsSplit::Coat => assert_eq!(b["coat"], "I.s=0.3,p=0.4"),
                    BsSplit::Pbs => assert_eq!(b["pbs"], json!({ "tp": 1.0, "ts": 0.0 })),
                }
                for arm in [false, true] {
                    c.picks[0].arm_r = arm;
                    let r = build(&c);
                    assert!(r.is_ok(), "{form:?} {split:?} {arm}: {:?}", r.err());
                }
            }
        }
        let b = &c.project()["blocks"][2];
        assert_eq!(b["form"], "ideal");
        assert_eq!(b["angle"], json!([40.0, 3.0]));
        c.picks[0].form = BsForm::Plate;
        let b = &c.project()["blocks"][2];
        assert_eq!(b["wedge"], json!([0.5, 0.0]));
        // 面ごとの膜は形の面だけ
        assert_eq!(b["faces"], json!({ "back": "I.s=1,p=1" }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 差し替え探索の空気の id（dk）: 最上位の要素の後の空気だけ。分岐のある BS（最上位の最後）の後には無い
    #[test]
    fn swap_gaps_with_branches() {
        let c = Combo::twyman_green();
        let (p, cands) = crate::swap_panel::combo_project(&c);
        let ids: Vec<&str> = cands.gaps.iter().map(|g| g.0.as_str()).collect();
        assert_eq!(ids, ["d1"]);
        assert_eq!(
            p["blocks"][1],
            json!({ "type": "air", "t": 20.0, "id": "d1" })
        );
        assert_eq!(p["blocks"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn insert_into_branch() {
        let mut c = twyman();
        // 経路の最後（戻る鏡）の後ろではなく、鏡の前に足す
        let n = c.len();
        let k = c.insert_chain(n, Pick::new(ElemKind::Plate));
        assert_eq!(c.chain()[k].loc, vec![1, 0, 1]);
        assert_eq!(c.chain()[k + 1].loc, vec![1, 0, 2]);
        // BS の直後（今の経路の口の先の頭）に、間の空気を半分ずつ
        let mut c = twyman();
        let k = c.insert_chain(2, Pick::new(ElemKind::Plate));
        assert_eq!(c.chain()[k].loc, vec![1, 0, 0]);
        assert_eq!(c.picks[1].ports[0].air, 15.0);
        assert_eq!(c.picks[1].ports[0].picks[0].air, 15.0);
        // 空の口の先へ: 経路の最後の BS の後
        let mut c = Combo::default();
        c.picks.push(Pick::new(ElemKind::Splitter));
        c.set_branched(&[0], true);
        c.path = "R".into();
        let k = c.insert_chain(1, Pick::new(ElemKind::Lens));
        assert_eq!(c.chain()[k].loc, vec![0, 1, 0]);
        // 外す・入れ替え
        c.remove_chain(k);
        assert!(c.picks[0].ports[1].picks.is_empty());
    }
}
