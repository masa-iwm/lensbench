//! レンズデータの表から自由に面を足す（11/16 masa 決定）: 要素の外の空気に面を足すと平面 1 枚の面の並びを置き、
//! 面の並びの面に硝材を入れると自作の玉に分け、自作の玉の中の硝材を外すと面の並び・2 つの玉に分け直す。
//! 系（光線の通り方）は変えず、要素の分け方だけを変える

use std::ops::Range;

use lensbench_project::edit_ops::{BlockRef, Ref, SurfaceOp};

use crate::combo::{Combo, ElemKind, Pick, SurfKind, SurfRow};
use crate::combo_tree::Gap;
use crate::i18n::tr;

/// 分けた後の 1 つ: 種類と面の行の範囲（後の空気は範囲の最後の行の厚さ）
type Part = (ElemKind, Range<usize>);

impl Combo {
    /// 経路の at 番目に、平面 1 枚の面の並びを足す（後の空気は 0。前の要素の後の空気はそのまま）。足した番号
    pub(crate) fn insert_plane(&mut self, at: usize) -> usize {
        let mut p = Pick::new(ElemKind::Surfaces);
        p.air = 0.0;
        self.insert_chain_with(at, p, false)
    }

    /// 経路の k 番目の 1 面の面の並びを外す。間隔（面の厚さと後の空気）は前の要素の後の空気（先頭なら前の絞りから
    /// の間隔）に足すので、後の要素の位置は変わらない
    pub(crate) fn remove_plane(&mut self, k: usize) -> Result<(), String> {
        let p = self.el(k).ok_or_else(outside)?;
        if p.locked {
            return Err(tr("A locked element cannot be changed").into());
        }
        let gap = p.surfs.iter().map(|r| r.t).sum::<f64>() + p.air;
        let id = self.ids().get(k).cloned().unwrap_or_default();
        let referred = self
            .all()
            .iter()
            .filter_map(|l| self.get(l))
            .flat_map(|q| q.ops.iter())
            .any(|o| refers_to(o, &id))
            || self
                .extra
                .glrs
                .as_ref()
                .is_some_and(|g| g["block"] == id.as_str());
        if referred || !p.ops.is_empty() {
            return Err(tr("This surface has a stop, solve or pickup; remove them first").into());
        }
        // OFF にできる面の並びは、外すと状態ごとの並びが変わるので外さない
        if !p.on || !p.state_on.is_empty() {
            return Err(tr(
                "This surface list is turned off in some states, so it cannot be removed",
            )
            .into());
        }
        let rows_t = p.surfs.iter().map(|r| r.t).sum::<f64>();
        let plane = p.clone();
        let before = k
            .checked_sub(1)
            .map(|b| self.chain().get(b).and_then(|x| x.after.clone()));
        match before {
            // 前の要素の後の空気: 状態ごとの後の空気にも、その状態での面の並びの間隔を足す
            Some(Some(Gap::Air(l))) => {
                let q = self.get_mut(&l).ok_or_else(outside)?;
                let names: std::collections::BTreeSet<String> = q
                    .state_air
                    .keys()
                    .chain(plane.state_air.keys())
                    .cloned()
                    .collect();
                for n in names {
                    let v = q.air_in(Some(&n)) + rows_t + plane.air_in(Some(&n));
                    q.state_air.insert(n, v);
                }
                q.air += gap;
            }
            // 状態ごとの値を持てない間隔（前の絞りから・BS の口の最初・最初の空気）
            _ if !plane.state_air.is_empty() => {
                return Err(tr(
                    "This surface list has spacings for states, and the spacing before it cannot hold them, so it cannot be removed",
                )
                .into())
            }
            Some(Some(Gap::Lead(l, port))) => {
                let b = self
                    .get_mut(&l)
                    .and_then(|q| q.ports.get_mut(port))
                    .ok_or_else(outside)?;
                b.air += gap;
            }
            Some(Some(Gap::Stop)) => self.stop_gap += gap,
            Some(None) => return Err(outside()),
            None if self.lead_stop() => self.stop_gap += gap,
            None => *self.lead_air.get_or_insert(0.0) += gap,
        }
        self.remove_chain(k);
        Ok(())
    }

    /// 経路の k 番目の面の並びの面 rel（座標ブレークを除く）の後を硝材 glass にして、その面と次の面を自作の玉に
    /// 分ける。次の面が無ければ平面の出口を足す（後の空気がガラスになり、後の要素の位置は変わらない）
    pub(crate) fn glass_into_surfaces(
        &mut self,
        k: usize,
        rel: usize,
        glass: &str,
    ) -> Result<(), String> {
        let p = self.el(k).ok_or_else(outside)?;
        // 行の厚さ = 次の面までの間隔（最後の面は要素の後の空気まで含める）
        let mut rows = p.surfs.clone();
        let n = rows.len();
        rows[n - 1].t += p.air;
        let i = nth_face(&rows, rel).ok_or_else(outside)?;
        let std = |r: &SurfRow| r.kind == SurfKind::Standard;
        if !std(&rows[i]) || rows.get(i + 1).is_some_and(|r| !std(r)) {
            return Err(tr("Only a standard surface can be followed by glass").into());
        }
        if i + 1 == n {
            // 出口の平面。玉の厚さは元の後の空気で、出口の後は 0（後の要素の位置は変わらない）
            rows.push(SurfRow {
                semi: rows[i].semi,
                semi_auto: rows[i].semi_auto,
                ..SurfRow::default()
            });
        }
        rows[i].glass = glass.trim().to_string();
        let m = rows.len();
        let mut parts = Vec::new();
        if i > 0 {
            parts.push((ElemKind::Surfaces, 0..i));
        }
        parts.push((ElemKind::Lens, i..i + 2));
        if i + 2 < m {
            parts.push((ElemKind::Surfaces, i + 2..m));
        }
        self.reshape(k, rows, parts)
    }

    /// 経路の k 番目の自作の玉の面 rel の後の硝材を外して（空気に）、玉をその前後に分ける。ガラスの無い部分は
    /// 面の並びになる。最後の面の後は要素の後の空気なので、ここでは扱わない
    pub(crate) fn glass_out_of_lens(&mut self, k: usize, rel: usize) -> Result<(), String> {
        let p = self.el(k).ok_or_else(outside)?;
        if p.reversed {
            return Err(tr("Unreverse the lens first to change its glass to air").into());
        }
        let mut rows = p.lens_rows();
        let n = rows.len();
        if rel + 1 >= n {
            return Err(outside());
        }
        // 玉の最後の面の厚さは使わないので、要素の後の空気にする
        rows[n - 1].t = p.air;
        rows[rel].glass.clear();
        let kind = |r: &[SurfRow]| {
            if r.len() >= 2 && r[..r.len() - 1].iter().any(|x| !x.glass.is_empty()) {
                ElemKind::Lens
            } else {
                ElemKind::Surfaces
            }
        };
        let parts = match (kind(&rows[..=rel]), kind(&rows[rel + 1..])) {
            // 両方とも空気だけなら、1 つの面の並びのまま
            (ElemKind::Surfaces, ElemKind::Surfaces) => vec![(ElemKind::Surfaces, 0..n)],
            (a, b) => vec![(a, 0..rel + 1), (b, rel + 1..n)],
        };
        self.reshape(k, rows, parts)
    }

    /// 経路の k 番目の要素を、面の行 rows（厚さは次の面までの間隔）を parts に分けた要素に置き換える。最初の部分が id・ロックなどを引き継ぎ、
    /// この要素の面の上書き・操作は分けた先の面へ移す。ほかの要素から面を指されている・置き方や状態を持つ要素は分けない
    fn reshape(&mut self, k: usize, rows: Vec<SurfRow>, parts: Vec<Part>) -> Result<(), String> {
        let ids = self.ids();
        let id = ids.get(k).cloned().unwrap_or_default();
        let p = self.el(k).ok_or_else(outside)?.clone();
        let moved = p.decenter != [0.0; 2]
            || p.tilt != [0.0; 3]
            || !p.state_on.is_empty()
            || !p.state_air.is_empty()
            || !p.state_place.is_empty()
            || !p.group.is_empty()
            || !p.on
            || p.mirror().back;
        if p.locked {
            return Err(tr("A locked element cannot be changed").into());
        }
        let referred = self
            .all()
            .iter()
            .filter_map(|l| self.get(l))
            .flat_map(|q| q.ops.iter())
            .any(|o| refers_to(o, &id))
            || self
                .extra
                .glrs
                .as_ref()
                .is_some_and(|g| g["block"] == id.as_str())
            // 機構部品は要素の id と面の番号で置くので、面が別の要素へ移るとずれる
            || self
                .mechanics
                .iter()
                .any(|m| m.at == id || m.holds.contains(&id));
        if moved || referred {
            return Err(tr(
                "This element has a placement, states or references from other elements, so it cannot be split into lenses and surface lists",
            )
            .into());
        }
        // 元の面（座標ブレークを除く番号）→ (部分, 部分の中の面)
        let mut face_of = Vec::new();
        for (pi, part) in parts.iter().enumerate() {
            let mut f = 0;
            for r in part.1.clone() {
                if rows[r].kind != SurfKind::Coordbrk {
                    face_of.push((pi, f));
                    f += 1;
                }
            }
        }
        let mut out: Vec<Pick> = parts
            .iter()
            .enumerate()
            .map(|(pi, part)| {
                let mut q = if pi == 0 {
                    let mut q = p.clone();
                    q.edits.clear();
                    q.ops.clear();
                    q
                } else {
                    Pick::new(part.0)
                };
                q.kind = part.0;
                // 行は光の進む順なので、反転は持たない（面の並びは反転を無視するが、古い保存では残っていることがある）
                q.reversed = false;
                q.surfs = rows[part.1.clone()].to_vec();
                // 最後の面の後の間隔は要素の後の空気
                q.air = q.surfs.last().map_or(0.0, |r| r.t);
                if let Some(r) = q.surfs.last_mut() {
                    r.t = 0.0;
                }
                if part.0 == ElemKind::Surfaces {
                    for r in &mut q.surfs {
                        r.glass.clear();
                    }
                }
                if let Some(r) = q.surfs.last_mut() {
                    r.glass.clear();
                }
                if pi > 0 {
                    q.air_id.clear();
                }
                q.edits_for.clear();
                q
            })
            .collect();
        // 1 面の玉は書けないので面の並びに（ガラスの無い 2 面も）
        for q in &mut out {
            if q.kind == ElemKind::Lens && q.surfs.len() < 2 {
                q.kind = ElemKind::Surfaces;
            }
        }
        // 面の上書き・操作を分けた先へ（ほかの部分の面を指す操作は移せないので分けない）
        for e in &p.edits {
            let &(pi, f) = face_of.get(e.surface).ok_or_else(outside)?;
            let mut e = e.clone();
            e.surface = f;
            out[pi].edits.push(e);
        }
        for o in &p.ops {
            let mut o = o.clone();
            let mut part = None;
            let mut ok = true;
            for_each_own_ref(&mut o, &mut |r| {
                if let BlockRef::Sys(Ref::Index(i)) = r {
                    match face_of.get(*i) {
                        Some(&(pi, f)) if part.is_none_or(|x| x == pi) => {
                            part = Some(pi);
                            *i = f;
                        }
                        _ => ok = false,
                    }
                }
            });
            match part {
                Some(pi) if ok => out[pi].ops.push(o),
                _ => {
                    return Err(tr(
                        "A solve or pickup links surfaces that would be in different elements, so the element cannot be split",
                    )
                    .into())
                }
            }
        }
        let s = self.chain().get(k).cloned().ok_or_else(outside)?;
        let (&i, pre) = s.loc.split_last().ok_or_else(outside)?;
        let l = self.list_at_mut(pre).ok_or_else(outside)?;
        l.splice(i..=i, out);
        Ok(())
    }
}

fn outside() -> String {
    tr("Surface outside the combination elements").into()
}

/// 座標ブレークを除いて数えた rel 番目の行
fn nth_face(rows: &[SurfRow], rel: usize) -> Option<usize> {
    (0..rows.len())
        .filter(|&i| rows[i].kind != SurfKind::Coordbrk)
        .nth(rel)
}

/// 操作がブロック id の面を指しているか
fn refers_to(o: &SurfaceOp, id: &str) -> bool {
    let mut hit = false;
    let mut o = o.clone();
    for_each_ref(&mut o, &mut |r| {
        hit |= matches!(r, BlockRef::Block(f) if f.block == id);
    });
    hit
}

/// 操作の面（surface・dst・src・other）を全部 f に渡す
fn for_each_ref(o: &mut SurfaceOp, f: &mut dyn FnMut(&mut BlockRef)) {
    match o {
        SurfaceOp::SetStop(s) => f(&mut s.surface),
        SurfaceOp::AddSolve(s) => {
            f(&mut s.surface);
            if let Some(x) = s.other.as_mut() {
                f(x);
            }
        }
        SurfaceOp::AddPickup(s) => {
            f(&mut s.dst);
            f(&mut s.src);
        }
        SurfaceOp::RemoveSolve { surface, .. } => f(surface),
        SurfaceOp::RemovePickup { dst, .. } => f(dst),
    }
}

/// 要素自身の面（番号で書いた面）だけを f に渡す
fn for_each_own_ref(o: &mut SurfaceOp, f: &mut dyn FnMut(&mut BlockRef)) {
    for_each_ref(o, &mut |r| {
        if matches!(r, BlockRef::Sys(Ref::Index(_))) {
            f(r);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    /// 組合せの系（面の行: 曲率半径・厚さ・硝材）
    fn rows(c: &Combo) -> Vec<(f64, f64, String)> {
        let glass = vec![root().join("tests/glass")];
        let t = lensbench_project::Project::from_json(&c.project().to_string())
            .and_then(|q| {
                let env = lensbench_project::Env::in_dir(&root(), &glass, None);
                lensbench_project::expand(&q, &env, &Default::default())
            })
            .unwrap()
            .system;
        t.surfaces
            .iter()
            .map(|s| ((s.c * 1e9).round() / 1e9, s.t, s.mat.name()))
            .collect()
    }

    fn base() -> Combo {
        let mut c = Combo {
            epd: 5.0,
            ..Default::default()
        };
        c.insert_plane(0);
        c
    }

    /// 空の組合せに平面を足し、硝材を入れると出口の平面が付いた玉になり、外すと面の並びに戻る
    #[test]
    fn plane_then_glass_then_air() {
        let mut c = base();
        assert_eq!(c.picks.len(), 1);
        c.picks[0].air = 20.0;
        c.picks[0].surfs[0].radius = 30.0;
        let before = rows(&c);
        c.glass_into_surfaces(0, 0, "N-BK7").unwrap();
        let kinds: Vec<_> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [ElemKind::Lens]);
        // 面が 1 つ増え（出口の平面）、後の空気がガラスになる。像面の位置は同じ
        let after = rows(&c);
        assert_eq!(after.len(), before.len() + 1);
        let z = |r: &[(f64, f64, String)]| r[1..r.len() - 1].iter().map(|x| x.1).sum::<f64>();
        assert_eq!(z(&after), z(&before));
        assert!(after.iter().any(|x| x.2 == "N-BK7"));
        // 外すと 2 面の面の並びに
        c.glass_out_of_lens(0, 0).unwrap();
        let kinds: Vec<_> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [ElemKind::Surfaces]);
        assert_eq!(c.picks[0].surfs.len(), 2);
        assert!(rows(&c).iter().all(|x| x.2.is_empty()));
        assert_eq!(z(&rows(&c)), z(&before));
    }

    /// 要素の間に足した平面を外すと、間隔は前の要素の後の空気に足され、像面の位置は変わらない
    #[test]
    fn insert_then_remove_plane_keeps_spacing() {
        let mut c = base();
        c.picks[0].surfs[0].t = 3.0;
        c.picks[0].air = 7.0;
        c.insert_plane(1);
        c.picks[1].air = 5.0;
        c.insert_plane(1);
        assert_eq!(c.picks.len(), 3);
        let z = |c: &Combo| rows(c).iter().map(|x| x.1).sum::<f64>();
        let before = z(&c);
        c.remove_plane(1).unwrap();
        assert_eq!(c.picks.len(), 2);
        assert_eq!(z(&c), before);
        c.remove_plane(0).unwrap();
        assert_eq!(z(&c), before);
    }

    /// 状態ごとの後の空気も足す（前の要素 20・状態 s1 で 10、平面 5・s1 で 7 → s1 で 17 のまま）
    #[test]
    fn remove_plane_keeps_state_spacing() {
        let mut c = base();
        c.picks[0].air = 20.0;
        c.picks[0].state_air.insert("s1".into(), 10.0);
        c.insert_plane(1);
        c.picks[1].air = 5.0;
        c.picks[1].state_air.insert("s1".into(), 7.0);
        c.insert_plane(2);
        c.remove_plane(1).unwrap();
        assert_eq!(c.picks[0].air, 25.0);
        assert_eq!(c.picks[0].state_air["s1"], 17.0);
        // 先頭の面の並びの状態ごとの間隔は、前の絞りからの間隔に持てないので断る
        c.picks[0].state_air.insert("s1".into(), 3.0);
        assert!(c.remove_plane(0).is_err());
    }

    /// 反転の印が残った面の並びにガラスを入れても、面の順と曲率は変わらない。機構部品が置かれた要素は分けない
    #[test]
    fn reversed_flag_and_mechanics() {
        let mut c = base();
        c.picks[0].surfs = (0..2)
            .map(|i| SurfRow {
                radius: [40.0, -60.0][i],
                t: [4.0, 0.0][i],
                ..SurfRow::default()
            })
            .collect();
        c.picks[0].air = 30.0;
        c.picks[0].reversed = true;
        let mut d = c.clone();
        c.glass_into_surfaces(0, 0, "N-BK7").unwrap();
        assert!(!c.picks[0].reversed);
        let r: Vec<f64> = rows(&c).iter().map(|x| x.0).collect();
        assert!(r.contains(&(1.0 / 40.0)), "{r:?}");
        let id = d.ids()[0].clone();
        d.mechanics.push(crate::combo::MechPick {
            at: id,
            ..Default::default()
        });
        assert!(d.glass_into_surfaces(0, 0, "N-BK7").is_err());
    }

    /// 面の並びの中の面に硝材を入れると、前後の面の並びと玉に分かれ、空気の間隔は変わらない
    #[test]
    fn glass_in_the_middle_splits_three() {
        let mut c = base();
        c.picks[0].surfs = (0..4)
            .map(|i| SurfRow {
                radius: [0.0, 40.0, -40.0, 0.0][i],
                t: [5.0, 4.0, 6.0, 0.0][i],
                ..SurfRow::default()
            })
            .collect();
        c.picks[0].air = 30.0;
        let before = rows(&c);
        c.glass_into_surfaces(0, 1, "N-BK7").unwrap();
        let kinds: Vec<_> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [ElemKind::Surfaces, ElemKind::Lens, ElemKind::Surfaces]
        );
        let after = rows(&c);
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(&before) {
            assert_eq!((a.0, a.1), (b.0, b.1));
        }
        assert_eq!(after[3].2, "N-BK7");
        // 玉の中を空気にすると、元の分け方に近い形（面の並び 4 つ分の面）に戻る
        c.glass_out_of_lens(1, 0).unwrap();
        assert!(c.picks.iter().all(|p| p.kind == ElemKind::Surfaces));
        let back = rows(&c);
        for (a, b) in back.iter().zip(&before) {
            assert_eq!((a.0, a.1, &a.2), (b.0, b.1, &b.2));
        }
    }

    /// 貼り合わせの玉の間を空気にすると 2 つの玉に分かれる。ソルブは分けた先の面へ移る
    #[test]
    fn cemented_splits_into_two_lenses() {
        let mut c = base();
        let mut p = Pick::new(ElemKind::Lens);
        p.surfs = vec![
            SurfRow {
                radius: 50.0,
                t: 4.0,
                glass: "N-BK7".into(),
                ..SurfRow::default()
            },
            SurfRow {
                radius: -30.0,
                t: 2.0,
                glass: "N-SF5".into(),
                ..SurfRow::default()
            },
            SurfRow {
                radius: -80.0,
                ..SurfRow::default()
            },
        ];
        p.air = 40.0;
        p.ops
            .push(SurfaceOp::AddSolve(lensbench_project::edit_ops::SolveOp {
                surface: BlockRef::Sys(Ref::Index(2)),
                other: None,
                kind: Some("marginal_height".into()),
                value: None,
                param: None,
            }));
        c.picks = vec![p];
        c.glass_out_of_lens(0, 1).unwrap();
        let kinds: Vec<_> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [ElemKind::Lens, ElemKind::Surfaces]);
        assert_eq!(c.picks[0].air, 2.0);
        // 2 枚目は面 1 つ（ガラスの無い出口）なので面の並び。ソルブは面 2 → 2 つ目の面 0
        assert!(
            matches!(&c.picks[1].ops[0], SurfaceOp::AddSolve(o) if o.surface == BlockRef::Sys(Ref::Index(0)))
        );
    }

    /// ほかの要素から面を指されている要素は分けない
    #[test]
    fn referred_elements_are_not_split() {
        let mut c = base();
        c.picks[0].id = "S".into();
        let mut q = Pick::new(ElemKind::Surfaces);
        q.ops.push(SurfaceOp::AddPickup(
            lensbench_project::edit_ops::PickupOp {
                dst: BlockRef::Sys(Ref::Index(0)),
                src: BlockRef::Block(lensbench_project::edit_ops::BlockFace {
                    block: "S".into(),
                    surface: 0,
                }),
                what: "c".into(),
                scale: None,
                offset: None,
                from: None,
            },
        ));
        c.picks.push(q);
        assert!(c.glass_into_surfaces(0, 0, "N-BK7").is_err());
        assert_eq!(c.picks.len(), 2);
    }
}
