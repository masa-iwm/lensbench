//! 復路: 往路を戻る面の並び（鏡の `back`）と、光束の `turn`（玉の面の部分反射で来た道を戻る、[`Beam::turn`]）

use super::*;
use lensbench_core::surface::Reflect;
use splitter::Level;

/// 前の座標ブレーク（順序 0 で偏心 d・傾き t）で傾けた座標の z に l 進んだ点から、元の座標の z に l 進んだ点へ戻る
/// 座標ブレーク（順序 0 で −t、偏心 (dx2, dy2)、厚さ t2）の値: (dx2, dy2, t2) = l e_z − d − R e_z l（R = Rx Ry Rz）
pub(crate) fn cb_return(place: &Place, l: f64) -> (f64, f64, f64) {
    let [dx, dy] = place.decenter;
    let [tx, ty, tz] = place.tilt;
    let r: M3 = lensbench_core::math::mm(
        &lensbench_core::math::mm(&rot(0, tx), &rot(1, ty)),
        &rot(2, tz),
    );
    let rz = mv(&r, [0.0, 0.0, l]);
    (-dx - rz[0], -dy - rz[1], l - rz[2])
}

/// 空気中の面の並びを逆向きに（厚さは 1 つ前の面のもの、曲率の符号を変える）。最後の面の厚さは 0
pub(crate) fn reverse_surfaces(v: &[Surface]) -> lensbench_core::Result<Vec<Surface>> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut s = v[n - 1 - i].clone();
            s.flip_z()?;
            s.t = if i + 1 < n { v[n - 2 - i].t } else { 0.0 };
            s.stop = false;
            Ok(s)
        })
        .collect()
}

/// 光束の `turn` の展開中の状態（[`Ctx::turn`]）
#[derive(Debug, Default)]
pub(crate) struct TurnState {
    /// 折り返す面（ブロックの id・面の番号・往路の何回目か）
    pub(crate) at: Option<crate::edit_spec::BlockFace>,
    /// at のブロックを往路でたどった回数
    pub(crate) seen: usize,
    /// 折り返した面（展開した面の並びの番号）と、その面の後ろの元の媒質
    pub(crate) turned: Vec<(usize, Material)>,
    /// 折り返した玉のブロックを覚えた後で前のブロックを戻る
    pub(crate) pending: bool,
}

impl TurnState {
    pub(crate) fn new(p: &Project) -> TurnState {
        TurnState {
            at: p.turn.clone(),
            ..Default::default()
        }
    }
}

impl Ctx<'_, '_> {
    /// 鏡の `back`・`turn` の後: 前のブロックを戻る。BS で A 以外の口へ出たら止まり、並びの初めまで戻れば
    /// （BS の分岐なら）BS を通る
    pub(crate) fn go_back(&mut self, siblings: &[Block], idx: usize) -> lensbench_core::Result<()> {
        self.split.stopped = false;
        for x in siblings[..idx].iter().rev() {
            self.back_block(x)?;
            if self.split.stopped {
                return Ok(());
            }
        }
        let lv = self.split.levels.last().cloned().unwrap_or(Level::Other);
        self.return_at(lv)
    }

    /// ブロック b を往路でたどるとき、そこで折り返すならその面の番号（ブロックの面、0 始まり）
    pub(crate) fn turn_here(&mut self, b: &Block) -> Option<usize> {
        let at = self.turn.at.as_ref()?;
        if at.block.is_empty() || block_id(b) != at.block {
            return None;
        }
        let k = self.turn.seen;
        self.turn.seen += 1;
        (k == at.pass).then_some(at.surface)
    }

    /// 玉 e（往路の面の並び、置き方 place）の面 k で部分反射して来た道を戻る。反射の係数は面の境界（膜込み）の
    /// 反射（ゴーストと同じ、反対側の媒質つき）。戻りは鏡の `back` と同じく前のブロックをたどり、BS の経路の文字で
    /// 口を選ぶ。戻った先で展開を終える
    pub(crate) fn turn_lens(
        &mut self,
        e: &Element,
        place: &Place,
        k: usize,
    ) -> lensbench_core::Result<()> {
        let n = e.surfaces.len();
        if self.split.levels.iter().any(|l| matches!(l, Level::Other)) {
            return Err("turn: a lens in a group can't be turned in (put the turn on a lens outside groups)".into());
        }
        if k >= n {
            return Err(format!("turn: surface {k} ({n} surfaces in the block)").into());
        }
        if e.surfaces
            .iter()
            .any(|s| s.is_mirror() || s.is_coord_break())
        {
            return Err(
                "turn: a lens with a mirror or a coordinate break can't be turned in".into(),
            );
        }
        let moved = place.moved();
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        if moved {
            self.push(crate::cb([dx, dy, tx, ty, tz], 0, 0.0));
            self.mark_placed();
        }
        for s in &e.surfaces[..k] {
            self.push(s.clone());
        }
        let mut g = e.surfaces[k].clone();
        let after = std::mem::replace(&mut g.mat, Material::Mirror);
        g.reflect = Some(Reflect {
            mat: after.label(),
            n: Vec::new(),
        });
        g.dn = 0.0;
        g.disp = 0.0;
        g.t = 0.0;
        g.after = After::None;
        self.push_mirror(g);
        self.turn.turned.push((self.sf.len() - 1, after));
        // 玉の中を面 k−1 … 0 と戻る。最後に戻る面の後ろは玉の外の媒質
        let start = self.sf.len();
        if k > 0 {
            self.gap(e.surfaces[k - 1].t);
            let mut part = Element {
                surfaces: e.surfaces[..k].to_vec(),
                n: e.n.iter().take(k).cloned().collect(),
                ..e.clone()
            };
            if let Some(l) = part.surfaces.last_mut() {
                l.mat = e.surfaces[n - 1].mat.clone();
                l.t = 0.0;
            }
            if e.n.len() == n {
                part.n[k - 1] = e.n[n - 1].clone();
            }
            for mut s in elements::reverse_folded(&part)?.surfaces {
                s.stop = false;
                self.push(s);
            }
        }
        if moved {
            if let Some(s) = self.sf.last_mut() {
                s.t = 0.0;
            }
            self.put(crate::cb([-dx, -dy, -tx, -ty, -tz], 1, 0.0));
            self.mark_placed();
        }
        for o in &mut self.map[start..] {
            o.back = true;
        }
        // 前のブロックを戻るのは、このブロックの面を覚えてから（[`Ctx::block`] が [`Ctx::finish_turn`] を呼ぶ）。
        // 戻りの面がこのブロックの面・出どころにならないように
        self.turn.pending = true;
        Ok(())
    }

    /// 折り返した玉のブロックを覚えた後: 前のブロックを戻り、展開を終える
    pub(crate) fn finish_turn(
        &mut self,
        siblings: &[Block],
        idx: usize,
    ) -> lensbench_core::Result<()> {
        self.turn.pending = false;
        self.split.loose = false;
        self.go_back(siblings, idx)?;
        if self.split.loose {
            return Err("turn: the return path leaves a BS without branches (write the arms as the BS branches t, r and d)".into());
        }
        self.split.stopped = false;
        self.split.done = true;
        Ok(())
    }
}

/// 折り返した面の反射の反対側の媒質の屈折率（波長ごと）を入れる。媒質は元の面の後ろのもの
pub(crate) fn fill_turned(
    s: &mut System,
    turned: &[(usize, Material)],
) -> lensbench_core::Result<()> {
    for (j, mat) in turned {
        let j = *j + 1;
        if s.prescription().surfaces[j]
            .reflect
            .as_ref()
            .is_some_and(|r| !r.n.is_empty())
        {
            continue;
        }
        // 折り返す面までの往路だけの系で、元の面の後ろの媒質を解く
        let mut rx = s.prescription().clone();
        rx.surfaces.truncate(j + 1);
        rx.surfaces[j].mat = mat.clone();
        rx.surfaces[j].reflect = None;
        rx.surfaces.push(Surface::default());
        rx.pickups.clear();
        rx.solves.clear();
        rx.units.clear();
        rx.field_type = FieldType::Angle;
        let n = rx.build_raw()?.n(j).to_vec();
        s.edit_raw(|rx| {
            if let Some(r) = rx.surfaces[j].reflect.as_mut() {
                r.n = n;
            }
            Ok(())
        })?;
    }
    Ok(())
}
