//! 組合せ（GUI が持つ文書）と、カタログから足す位置。カタログのパネルは一覧だけを持ち、
//! 組合せは [`crate::app`] の Core が持つ（GPN-28）

use crate::catalog::{pick_from, LensRow, RowKind};
use crate::combo::Combo;

/// 組合せと、カタログから足すときの位置
#[derive(Default)]
pub struct ComboDoc {
    pub combo: Combo,
    /// 足す玉をこの要素の直後へ（組合せの図で選んでいる要素。無ければ後ろ）
    pub insert_after: Option<usize>,
    /// 最後に足した要素（経路の番号。呼び手が受け取る）
    pub added: Option<usize>,
    /// 最後に玉を足した: 系ができたら像面を近軸の焦点へ動かす（GUX-10）
    pub refocus: bool,
}

impl ComboDoc {
    pub fn new(combo: Combo) -> Self {
        Self {
            combo,
            ..Default::default()
        }
    }

    /// 素子 `lens` に合わせて機構部品を足す: 組合せにその素子があれば、そこに置いて保持する
    pub fn add_mech_for(&mut self, mech: &LensRow, lens: &LensRow, lenses: &[LensRow]) {
        let chain = self.combo.chain_view().picks;
        let same = |p: &crate::combo::Pick| {
            p.kind == crate::combo::ElemKind::Catalog
                && p.vendor == lens.vendor
                && p.part == lens.part
        };
        // 同じ玉が 2 つ以上あれば、選んでいる方を優先
        let k = self
            .insert_after
            .filter(|&k| chain.get(k).is_some_and(|p| same(p)))
            .or_else(|| chain.iter().position(|p| same(p)));
        let keep = self.insert_after;
        if k.is_some() {
            self.insert_after = k;
        }
        self.add_row(mech, lenses);
        self.insert_after = keep;
    }

    /// カタログの行を足す（未完の品は呼び手が除く）。`lenses` は機構部品が保持する玉を探す一覧
    pub fn add_row(&mut self, r: &LensRow, lenses: &[LensRow]) {
        // 機構部品は要素の並びでなく「機構部品」へ（選んでいる要素に置く）
        if r.kind == RowKind::Mech {
            let ids = self.combo.ids();
            let at = self
                .insert_after
                .and_then(|k| ids.get(k).cloned())
                .or_else(|| ids.iter().find(|x| !x.is_empty()).cloned())
                .unwrap_or_default();
            // 名前は組合せの中で別々に（つなぐ相手を名前で選ぶので）
            let mut name = r.part.clone();
            let mut n = 2;
            while self.combo.mechanics.iter().any(|m| m.name == name) {
                name = format!("{} {n}", r.part);
                n += 1;
            }
            // 置く要素がカタログの玉で、その径が部品の保持径に入るなら、その玉を保持して座に合わせる
            let held = ids
                .iter()
                .position(|x| !at.is_empty() && *x == at)
                .and_then(|k| self.combo.el(k))
                .filter(|p| p.kind == crate::combo::ElemKind::Catalog)
                .and_then(|p| {
                    lenses.iter().find(|l| {
                        l.vendor == p.vendor && l.part == p.part && l.kind != RowKind::Mech
                    })
                })
                .is_some_and(|l| lensbench_catalog::spec::fits_row(&r.raw, l.diameter));
            self.combo.mechanics.push(crate::combo::MechPick {
                name,
                catalog: r.source.clone().unwrap_or_default(),
                item: r.part.clone(),
                holds: if held { vec![at.clone()] } else { Vec::new() },
                seat: held,
                at,
                ..Default::default()
            });
            return;
        }
        let at = self.insert_after.map_or(self.combo.len(), |k| k + 1);
        let n = self.combo.len();
        let at = if at >= n && n > 0 {
            // 最後に足す: 前の最後の要素の後（像面までだった空気）は小さな間隔にし、足した玉の後の空気は
            // 系ができたら近軸の焦点まで（2 枚目で像面が遠くに残って像が崩れない、GUX-10）
            let chain = self.combo.chain();
            if let Some(g) = chain.last().and_then(|s| s.after.clone()) {
                if matches!(g, crate::combo_tree::Gap::Air(_)) {
                    if let Some(v) = self.combo.gap(&g) {
                        *v = 5.0;
                    }
                }
            }
            self.refocus = true;
            self.combo.insert_chain_with(at, pick_from(r), false)
        } else {
            if n == 0 {
                self.refocus = true;
            }
            crate::combo::insert_pick(&mut self.combo, at, pick_from(r))
        };
        // 続けて足すと、足した玉の後ろへ
        self.insert_after = Some(at);
        self.added = Some(at);
    }
}
