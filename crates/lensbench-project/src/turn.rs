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

/// 面の並びを逆向きに（厚さは 1 つ前の面のもの、曲率の符号を変える）。最後の面の厚さは 0。面の後の媒質も
/// 1 つ前の面のもの（座標ブレークは媒質なしのまま）で、最後の面の後は往路で並びの前にあった媒質 before。
/// z 軸まわりの回転だけの座標ブレークは逆の回転に、Jones 面の行列は転置にする（相反性: 同じ x・y の基底で
/// 逆向きに通る行列は転置。直線の波長板・偏光子は対称で変わらない）
pub(crate) fn reverse_surfaces(
    v: &[Surface],
    before: &Material,
) -> lensbench_core::Result<Vec<Surface>> {
    let n = v.len();
    // 往路の各面の前の媒質（屈折率のずれ dn・分散の変化 disp も媒質と一緒に）
    let mut front = Vec::with_capacity(n);
    let mut cur = (before.clone(), 0.0, 0.0);
    for s in v {
        front.push(cur.clone());
        if s.mat != Material::Inherit {
            cur = (s.mat.clone(), s.dn, s.disp);
        }
    }
    (0..n)
        .map(|i| {
            let mut s = v[n - 1 - i].clone();
            if s.mat != Material::Inherit {
                (s.mat, s.dn, s.disp) = front[n - 1 - i].clone();
            }
            if let Some((cb, _)) = s.cb_mut() {
                if cb[..4].iter().any(|&x| x != 0.0) {
                    return Err(
                        "a coordinate break with a decenter or a tilt about x or y on the return path (only rotations about z)"
                            .into(),
                    );
                }
                cb[4] = -cb[4];
            }
            if let Some(j) = s.jones_mut() {
                j.swap(2, 4);
                j.swap(3, 5);
            }
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
    /// 折り返す面（ブロックの id・面の番号・前の折り返しから何回目か）。順に往路・戻る道・往路…で折り返す
    pub(crate) at: Vec<crate::edit_spec::BlockFace>,
    /// 次に折り返すブロックを、その向きで前の折り返しから通った回数
    pub(crate) seen: usize,
    /// 折り返した面（展開した面の並びの番号）と、その面の向こう側の元の媒質
    pub(crate) turned: Vec<(usize, Material)>,
    /// 戻る道で玉の前へ出た: 玉のブロックを覚えた後で前のブロックを戻る
    pub(crate) pending: bool,
}

/// 光束の `turn` で折り返しながらたどったブロック id の玉の面: (系の面, 玉の面の番号)
pub(crate) fn parts(map: &[crate::SurfOrigin], id: &str) -> Vec<(usize, usize)> {
    map.iter()
        .enumerate()
        .filter(|(_, o)| o.is_in(id))
        .filter_map(|(j, o)| Some((j, o.part?.0)))
        .collect()
}

impl TurnState {
    /// 展開の終わりに、たどれなかった折り返しの誤り
    pub(crate) fn left_over(&self) -> Option<String> {
        let t = self.next()?;
        let k = self.turned.len();
        let (n, b, f, q) = (k + 1, &t.block, t.surface, t.pass);
        Some(if k == 0 {
            format!("turn: block {} (pass {}) is not on the path", b, q)
        } else if k % 2 == 1 {
            format!(
                "turn {}: block {} surface {} (pass {}) is not on the way back",
                n, b, f, q
            )
        } else {
            format!(
                "turn {}: block {} surface {} (pass {}) is not on the way out",
                n, b, f, q
            )
        })
    }

    pub(crate) fn new(p: &Project) -> TurnState {
        TurnState {
            at: p.turn.clone(),
            ..Default::default()
        }
    }

    /// 次に折り返す面
    pub(crate) fn next(&self) -> Option<&crate::edit_spec::BlockFace> {
        self.at.get(self.turned.len())
    }

    /// 次の折り返しが戻る道か（往路で 1 回目、戻る道で 2 回目…）
    fn next_back(&self) -> bool {
        self.turned.len() % 2 == 1
    }
}

impl Ctx<'_, '_> {
    /// 鏡の `back`・`turn` の後: 前のブロックを戻る。BS で A 以外の口へ出たら止まり、並びの初めまで戻れば
    /// （BS の分岐なら）BS を通る
    pub(crate) fn go_back(&mut self, siblings: &[Block], idx: usize) -> lensbench_core::Result<()> {
        self.split.stopped = false;
        self.back_seq(siblings, idx, None)?;
        if self.split.stopped {
            return Ok(());
        }
        let lv = self.split.levels.last().cloned().unwrap_or(Level::Other);
        self.return_at(lv)
    }

    /// 戻る道で並び seq の upto より前のブロックを逆の順にたどる。戻る道で折り返す玉（光束の `turn`）があれば
    /// 折り返し、往路で玉を抜けたら seq の続きを往路でたどって経路を終える（stopped・done）。levels は seq を
    /// たどる段（今の段と違うとき: BS の腕から親の並びへ戻ったとき）
    pub(crate) fn back_seq(
        &mut self,
        seq: &[Block],
        upto: usize,
        levels: Option<&[Level]>,
    ) -> lensbench_core::Result<()> {
        for j in (0..upto).rev() {
            let x = &seq[j];
            let Some(k) = self.turn_back_here(x) else {
                self.back_block(x)?;
                if self.split.stopped {
                    return Ok(());
                }
                continue;
            };
            let Block::Lens {
                source,
                reversed,
                place,
                ..
            } = x
            else {
                return Err("turn: only a lens block can be turned in".into());
            };
            if !place.on {
                return Err("turn: the block is OFF".into());
            }
            let mut e = self.lens_element(source)?;
            if *reversed {
                e = elements::reverse(&e)?;
            }
            e.surfaces.iter_mut().for_each(|s| s.stop = false);
            let start = self.sf.len();
            let id = block_id(x).to_string();
            let out = self.in_node(x, |c| c.turn_walk(&id, &e, place, false, k))?;
            self.note(&id, x.faces(), start);
            if !out {
                continue;
            }
            // 往路で玉を抜けた: 並びの続きを往路でたどり、経路を終える
            let saved = levels.map(|l| std::mem::replace(&mut self.split.levels, l.to_vec()));
            let mut r = Ok(());
            for (i, y) in seq.iter().enumerate().skip(j + 1) {
                if self.split.done {
                    break;
                }
                r = self.block(y, seq, i);
                if r.is_err() {
                    break;
                }
            }
            if let Some(l) = saved {
                self.split.levels = l;
            }
            r?;
            self.split.done = true;
            self.split.stopped = true;
            return Ok(());
        }
        Ok(())
    }

    /// ブロック b を往路でたどるとき、そこで折り返すならその面の番号（ブロックの面、0 始まり）
    pub(crate) fn turn_here(&mut self, b: &Block) -> Option<usize> {
        if self.turn.next_back() {
            return None;
        }
        self.turn_at(b)
    }

    /// ブロック b を戻る道でたどるとき、そこで折り返すならその面の番号
    fn turn_back_here(&mut self, b: &Block) -> Option<usize> {
        if !self.turn.next_back() {
            return None;
        }
        self.turn_at(b)
    }

    fn turn_at(&mut self, b: &Block) -> Option<usize> {
        let at = self.turn.next()?;
        if at.block.is_empty() || block_id(b) != at.block {
            return None;
        }
        let (pass, surface) = (at.pass, at.surface);
        let k = self.turn.seen;
        self.turn.seen += 1;
        (k == pass).then_some(surface)
    }

    /// 折り返した玉 id の中で、次の折り返しがこの向き（back なら戻る道）の今のたどりにあればその面。from は
    /// このたどりで次に通る面（戻る道なら面 from から 0 へ、往路なら from から最後の面へ）
    fn turn_in_lens(&mut self, id: &str, back: bool, from: i64) -> Option<usize> {
        let t = self.turn.next()?;
        if t.block != id {
            return None;
        }
        // 玉の中を続けて通るのは、この向きで 0 回目
        let (pass, surface) = (t.pass, t.surface as i64);
        self.turn.seen = 1;
        let reach = if back {
            surface <= from
        } else {
            surface >= from
        };
        (pass == 0 && reach).then(|| {
            self.turn.seen = 0;
            surface as usize
        })
    }

    /// 玉の面を足し、玉の面の番号を覚える（back なら戻る道）
    fn push_part(&mut self, s: Surface, j: usize, n: usize, back: bool, mirror: bool) {
        if mirror {
            self.push_mirror(s);
        } else {
            self.push(s);
        }
        if let Some(o) = self.map.last_mut() {
            o.part = Some((j, n));
            o.back = back;
        }
    }

    /// 玉 e（往路の面の並び、置き方 place、ブロックの id）を、面の部分反射で折り返しながらたどる。fwd なら往路で
    /// 面 0 から入り面 k で、でなければ戻る道で最後の面から入り面 k で反射する。反射の係数は面の境界（膜込み）の反射
    /// （ゴーストと同じ、反対側の媒質つき）。反射の後、同じ玉の中で次の折り返しがあればそこでも反射する。往路で玉の
    /// 後ろへ抜けたら true、戻る道で前へ抜けたら false（前のブロックを戻るのは呼び手）
    pub(crate) fn turn_walk(
        &mut self,
        id: &str,
        e: &Element,
        place: &Place,
        fwd: bool,
        k: usize,
    ) -> lensbench_core::Result<bool> {
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
        if place.moved() {
            let again = self
                .turn
                .at
                .get(self.turn.turned.len() + 1)
                .is_some_and(|t| t.block == id);
            if !fwd || again {
                return Err(
                    "turn: a lens with a decenter or tilt is turned in only once, on the way out"
                        .into(),
                );
            }
            self.turn_lens_moved(e, place, k)?;
            return Ok(false);
        }
        let rev = elements::reverse_folded(e)?.surfaces;
        let first = self.turn.turned.is_empty();
        let (mut fwd, mut from, mut target) = (fwd, if fwd { 0 } else { n as i64 - 1 }, Some(k));
        loop {
            if fwd {
                let end = target.unwrap_or(n);
                for j in from.max(0) as usize..end {
                    let mut s = e.surfaces[j].clone();
                    if !(first && from == 0) {
                        s.stop = false;
                    }
                    self.push_part(s, j, n, false, false);
                }
                let Some(t) = target else {
                    return Ok(true);
                };
                let mut g = e.surfaces[t].clone();
                g.stop &= first && from == 0;
                let after = std::mem::replace(&mut g.mat, Material::Mirror);
                g.reflect = Some(Reflect {
                    mat: after.label(),
                    n: Vec::new(),
                });
                g.dn = 0.0;
                g.disp = 0.0;
                g.t = 0.0;
                g.after = After::None;
                self.push_part(g, t, n, false, true);
                self.turn.turned.push((self.sf.len() - 1, after));
                self.turn.seen = 0;
                if t > 0 {
                    self.gap(e.surfaces[t - 1].t);
                }
                fwd = false;
                from = t as i64 - 1;
                target = self.turn_in_lens(id, true, from);
            } else {
                let lo = target.map_or(0, |t| t as i64 + 1);
                let mut j = from;
                while j >= lo {
                    let mut s = rev[n - 1 - j as usize].clone();
                    s.stop = false;
                    self.push_part(s, j as usize, n, true, false);
                    j -= 1;
                }
                let Some(t) = target else {
                    return Ok(false);
                };
                let mut g = rev[n - 1 - t].clone();
                g.stop = false;
                let after = std::mem::replace(&mut g.mat, Material::Mirror);
                g.reflect = Some(Reflect {
                    mat: after.label(),
                    n: Vec::new(),
                });
                g.dn = 0.0;
                g.disp = 0.0;
                g.t = 0.0;
                g.after = After::None;
                self.push_part(g, t, n, true, true);
                self.turn.turned.push((self.sf.len() - 1, after));
                self.turn.seen = 0;
                self.gap(e.surfaces[t].t);
                fwd = true;
                from = t as i64 + 1;
                target = self.turn_in_lens(id, false, from);
                // 往路の続きの折り返しも玉の面の中（1 つ目と同じ誤り）
                if let Some(k) = target.filter(|&k| k >= n) {
                    return Err(format!("turn: surface {k} ({n} surfaces in the block)").into());
                }
            }
        }
    }

    /// 偏心・傾きのある玉 e の面 k で往路から 1 回だけ折り返して来た道を戻る（置き方の座標ブレークで入り、戻りで
    /// 元の軸へ戻す）
    fn turn_lens_moved(
        &mut self,
        e: &Element,
        place: &Place,
        k: usize,
    ) -> lensbench_core::Result<()> {
        let n = e.surfaces.len();
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        self.push(crate::cb([dx, dy, tx, ty, tz], 0, 0.0));
        self.mark_placed();
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
        self.turn.seen = 0;
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
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        self.put(crate::cb([-dx, -dy, -tx, -ty, -tz], 1, 0.0));
        self.mark_placed();
        for o in &mut self.map[start..] {
            o.back = true;
        }
        Ok(())
    }

    /// 戻る道で玉の前へ出た玉のブロックを覚えた後: 前のブロックを戻る。戻ったまま終われば展開を終える
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
