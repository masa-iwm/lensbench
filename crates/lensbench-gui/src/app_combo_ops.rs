//! 組合せの系の絞り・ソルブ・ピックアップ・面の挿入削除: 面の表の操作を組合せの要素の surface_ops・中身の面にする
//! （app.rs から分けた）

use super::*;
use lensbench_project::edit_ops::{
    BlockFace, BlockRef, EditOp, PickupOp, Ref, SolveOp, SolveWhat, SurfaceOp,
};

impl Core {
    /// 組合せの系の絞り・ソルブ・ピックアップ: serve の edit の操作を組合せの要素の surface_ops にする
    pub(super) fn combo_ops(
        &mut self,
        spans: &[Option<[usize; 2]>],
        ids: &[String],
        ops: &[Value],
    ) -> Result<(), String> {
        let lead0 = self.doc.combo.lead_stop();
        let r = self.combo_ops_inner(spans, ids, ops);
        // 前の絞りが出入りすると面の番号がずれるので、選んだ面も合わせる
        let lead1 = self.doc.combo.lead_stop();
        if lead0 != lead1 {
            if let Some((j, c)) = self.p.ld.cur {
                self.p.ld.cur = Some(if lead1 {
                    (j + 1, c)
                } else {
                    (j.saturating_sub(1), c)
                });
            }
        }
        r
    }

    fn combo_ops_inner(
        &mut self,
        spans: &[Option<[usize; 2]>],
        ids: &[String],
        ops: &[Value],
    ) -> Result<(), String> {
        let outside = || tr("Surface outside the combination elements").to_string();
        for op in ops {
            // 絞りをやめる（GUI だけの操作）
            if op["op"] == "unset_stop" {
                self.doc.combo.set_stop_face(None);
                continue;
            }
            // 面の挿入・削除: 自作の玉・面の並びの中身の面を足し引きする（GPN-19）。"after" は面 at−1 の後に足す
            // （GUI だけの欄。要素の境目でどちらの要素に足すかを決める）
            if op["op"] == "insert_surface" || op["op"] == "delete_surface" {
                let ins = op["op"] == "insert_surface";
                let j = op[if ins { "at" } else { "surface" }]
                    .as_u64()
                    .ok_or_else(outside)? as usize;
                let after = ins && op["after"] == json!(true);
                let at = if after { j.saturating_sub(1) } else { j };
                let editable = |c: &Combo, k: usize| {
                    c.el(k).is_some_and(|p| {
                        matches!(
                            p.kind,
                            crate::combo::ElemKind::Lens | crate::combo::ElemKind::Surfaces
                        )
                    })
                };
                let (k, i) = match self.combo_face(spans, at) {
                    Some((k, i)) if editable(&self.doc.combo, k) => (k, i),
                    // 要素の外（前の絞り・像面の前・要素の間の空気）や、面を足せない要素の端: 平面 1 枚の面の並びを
                    // 置く（11/16 masa 決定: 表から自由に面を足す）
                    found if ins && j > 0 => {
                        let last = |k: usize| spans[k].map(|s| s[1]) == Some(at);
                        let first = |k: usize| spans[k].map(|s| s[0]) == Some(at);
                        let pos = match found {
                            Some((k, _)) if after && last(k) => k + 1,
                            Some((k, _)) if !after && first(k) => k,
                            Some(_) => {
                                return Err(tr(
                                    "Surfaces can be added or removed only in a custom lens or a surface list",
                                )
                                .into())
                            }
                            None => spans
                                .iter()
                                .position(|s| s.is_some_and(|s| s[0] > at))
                                .unwrap_or(spans.len()),
                        };
                        self.doc.combo.insert_plane(pos);
                        continue;
                    }
                    _ => return Err(tr(
                        "Surfaces can be added or removed only in a custom lens or a surface list",
                    )
                    .into()),
                };
                // 面の並びの最後の 1 面を消すと、要素ごと外す（後の空気は前の要素の後の空気に足す）
                if !ins
                    && self.doc.combo.el(k).is_some_and(|p| {
                        p.kind == crate::combo::ElemKind::Surfaces && p.surfs.len() == 1
                    })
                {
                    self.doc.combo.remove_plane(k)?;
                    continue;
                }
                let p = self.doc.combo.el_mut(k).ok_or_else(outside)?;
                if p.locked {
                    return Err(tr("A locked element cannot be changed").into());
                }
                // ほかの要素の操作がこの要素の面を指していれば、番号を合わせる
                if ins {
                    let pos = i + usize::from(after);
                    p.insert_face(pos)?;
                    self.doc
                        .combo
                        .shift_block_refs(&ids[k], &|x| Some(if x >= pos { x + 1 } else { x }));
                } else {
                    p.delete_face(i)?;
                    self.doc
                        .combo
                        .shift_block_refs(&ids[k], &|x| (x != i).then(|| x - usize::from(x > i)));
                }
                continue;
            }
            let name = op["op"].as_str().unwrap_or_default().to_string();
            let op = EditOp::deserialize(op).map_err(|e| format!("{name}: {e}"))?;
            let face = |r: &Ref| -> Result<(usize, usize), String> {
                let (k, i) = match r {
                    Ref::Index(j) => self.combo_face(spans, *j),
                    Ref::Name(_) => None,
                }
                .ok_or_else(outside)?;
                if self.doc.combo.el(k).is_some_and(|p| p.locked) {
                    return Err(tr("A locked element cannot be changed").into());
                }
                Ok((k, i))
            };
            // ほかの要素の面は {"block": id, "surface": i}
            let other = |r: &Ref| -> Result<BlockRef, String> {
                let (k, i) = match r {
                    Ref::Index(j) => self.combo_face(spans, *j),
                    Ref::Name(_) => None,
                }
                .ok_or_else(outside)?;
                Ok(BlockRef::Block(BlockFace {
                    block: ids[k].clone(),
                    surface: i,
                }))
            };
            let here = |i: usize| BlockRef::Sys(Ref::Index(i));
            // 面の表で今そのソルブ（solve）・ピックアップが出ているか
            let rows = &self.sys.ld_rows;
            let shown = |r: &Ref, w: &str, solve: bool| match r {
                Ref::Index(j) => rows.get(*j).is_some_and(|r| {
                    let l = if solve { &r.solves } else { &r.pickups };
                    l.iter().any(|x| x.0 == w)
                }),
                Ref::Name(_) => false,
            };
            match op {
                EditOp::SetStop(o) => {
                    let j = match o.surface {
                        Ref::Index(j) => j,
                        Ref::Name(_) => return Err(outside()),
                    };
                    let c = &self.doc.combo;
                    let lead = c
                        .all()
                        .iter()
                        .all(|l| !c.get(l).is_some_and(|p| p.kind == combo::ElemKind::Stop));
                    match self.combo_face(spans, j) {
                        Some((k, i)) => {
                            let c = &mut self.doc.combo;
                            if c.el(k).is_some_and(|p| p.kind == combo::ElemKind::Stop) {
                                // 絞りの要素そのもの
                                c.set_stop_face(None);
                            } else {
                                let loc = c.chain()[k].loc.clone();
                                c.set_stop_face(Some((loc, i)));
                            }
                        }
                        // 前の絞り（要素の外）に戻す
                        None if lead
                            && self.sys.ld_rows.get(j).is_some_and(|r| r.comment == "STOP") =>
                        {
                            self.doc.combo.set_stop_face(None)
                        }
                        None => return Err(outside()),
                    }
                }
                EditOp::AddSolve(o) => {
                    let (k, i) = face(&o.surface)?;
                    let shown = shown(&o.surface, if o.is_curv() { "c" } else { "t" }, true);
                    let o = SolveOp {
                        surface: here(i),
                        other: o.other.as_ref().map(other).transpose()?,
                        kind: o.kind,
                        value: o.value,
                        param: o.param,
                    };
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .put_solve(o, shown);
                }
                EditOp::RemoveSolve { surface, what } => {
                    let (k, i) = face(&surface)?;
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .drop_solve(i, what);
                }
                EditOp::AddPickup(o) => {
                    let (k, i) = face(&o.dst)?;
                    let shown = shown(&o.dst, &o.what, false);
                    let o = PickupOp {
                        dst: here(i),
                        src: other(&o.src)?,
                        what: o.what,
                        scale: o.scale,
                        offset: o.offset,
                        from: o.from,
                    };
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .put_pickup(o, shown);
                }
                EditOp::RemovePickup { dst, what } => {
                    let (k, i) = face(&dst)?;
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .drop_pickup(i, what);
                }
                _ => return Err(trf("{} is not available in a combination", &[&name])),
            }
        }
        Ok(())
    }
}

impl combo::Pick {
    /// 面にソルブを置く。shown は面の表にそのソルブが出ていたか。要素で置いたものでなければ元（.zmx など）の
    /// ソルブなので、外す操作も置く（置いたソルブを外したとき、元のソルブに戻らない）
    pub(crate) fn put_solve(&mut self, o: SolveOp<BlockRef>, shown: bool) {
        let (at, c) = (o.surface.clone(), o.is_curv());
        let own = |q: &SurfaceOp| matches!(q, SurfaceOp::AddSolve(q) if q.surface == at && q.is_curv() == c);
        let from_source = shown && !self.ops.iter().any(own);
        self.ops.retain(|q| !own(q));
        let removed = self.ops.iter().any(|q| solve_of(q, &at, c));
        if from_source && !removed {
            let what = if c { SolveWhat::C } else { SolveWhat::T };
            self.ops.push(SurfaceOp::RemoveSolve { surface: at, what });
        }
        self.ops.push(SurfaceOp::AddSolve(o));
    }

    /// 面にピックアップを置く（[`Self::put_solve`] と同じく、元のピックアップなら外す操作も置く）
    pub(crate) fn put_pickup(&mut self, o: PickupOp<BlockRef>, shown: bool) {
        let at = o.dst.clone();
        let what = o.what.clone();
        let own =
            |q: &SurfaceOp| matches!(q, SurfaceOp::AddPickup(q) if q.dst == at && q.what == what);
        let from_source = shown && !self.ops.iter().any(own);
        self.ops.retain(|q| !own(q));
        let removed = self.ops.iter().any(|q| pickup_of(q, &at, &what));
        if from_source && !removed {
            self.ops.push(SurfaceOp::RemovePickup { dst: at, what });
        }
        self.ops.push(SurfaceOp::AddPickup(o));
    }

    /// 面 i のソルブを外す: 要素で置いたソルブなら外し、無ければ要素の元（.zmx など）のソルブを外す操作を置く
    pub(crate) fn drop_solve(&mut self, i: usize, what: SolveWhat) {
        let here = BlockRef::Sys(Ref::Index(i));
        let c = what == SolveWhat::C;
        let own = |q: &SurfaceOp| matches!(q, SurfaceOp::AddSolve(q) if q.surface == here && q.is_curv() == c);
        if self.ops.iter().any(own) {
            self.ops.retain(|q| !own(q));
        } else {
            self.ops.retain(|q| !solve_of(q, &here, c));
            self.ops.push(SurfaceOp::RemoveSolve {
                surface: here,
                what,
            });
        }
    }

    /// 面 i のピックアップ what を外す（ソルブと同じく、要素で置いたものが無ければ要素の元のものを外す）
    pub(crate) fn drop_pickup(&mut self, i: usize, what: String) {
        let here = BlockRef::Sys(Ref::Index(i));
        let own =
            |q: &SurfaceOp| matches!(q, SurfaceOp::AddPickup(q) if q.dst == here && q.what == what);
        if self.ops.iter().any(own) {
            self.ops.retain(|q| !own(q));
        } else {
            self.ops.retain(|q| !pickup_of(q, &here, &what));
            self.ops.push(SurfaceOp::RemovePickup { dst: here, what });
        }
    }
}

/// 面 at の量（曲率か厚さ）のソルブを置く・外す操作か
fn solve_of(q: &SurfaceOp, at: &BlockRef, curv: bool) -> bool {
    match q {
        SurfaceOp::AddSolve(q) => q.surface == *at && q.is_curv() == curv,
        SurfaceOp::RemoveSolve { surface, what } => {
            surface == at && (*what == SolveWhat::C) == curv
        }
        _ => false,
    }
}

/// 面 at の量 what のピックアップを置く・外す操作か
fn pickup_of(q: &SurfaceOp, at: &BlockRef, what: &str) -> bool {
    match q {
        SurfaceOp::AddPickup(q) => q.dst == *at && q.what == what,
        SurfaceOp::RemovePickup { dst, what: w } => dst == at && w == what,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// .zmx を取り込んだ要素（A0）の、元の .zmx にある最後の厚さのソルブを外せる。置き直したソルブを外しても
    /// 元のソルブには戻らない（1 回で外れる）。保存して開き直しても同じ
    #[test]
    fn drop_source_solve_of_zmx_element() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let glass = vec![root.join("tests/glass")];
        let zmx = root.join("tests/refs/A0/system.zmx");
        let p = json!({ "wavelengths": [{ "um": 0.5875618 }], "aperture": { "type": "epd", "value": 10 },
            "blocks": [{ "type": "zmx", "id": "zmx", "path": zmx.display().to_string() }, { "type": "air", "t": 0 }] });
        let mut c = combo::Combo::from_project(&p, &root).unwrap();
        assert_eq!(c.picks[0].kind, combo::ElemKind::ZmxRange);
        let solves = |c: &combo::Combo| {
            let t = lensbench_project::Project::from_json(&c.project().to_string())
                .and_then(|q| {
                    lensbench_project::expand(
                        &q,
                        &lensbench_project::Env::in_dir(&root, &glass, None),
                        &Default::default(),
                    )
                })
                .map(|x| x.system)
                .unwrap();
            t.solves.iter().map(|q| q.surf).collect::<Vec<_>>()
        };
        assert_eq!(solves(&c), [7]);
        // 面 7 は要素の面 6
        c.picks[0].drop_solve(6, SolveWhat::T);
        assert_eq!(solves(&c), Vec::<usize>::new());
        let back = combo::Combo::from_project(&c.project(), &root).unwrap();
        assert_eq!(back.picks[0].ops, c.picks[0].ops);
        // 元のソルブが出ている面に置き直して外すと、1 回で外れる（外す操作は残る）
        let add = || SolveOp {
            surface: BlockRef::Sys(Ref::Index(6)),
            other: None,
            kind: Some("marginal_height".into()),
            value: None,
            param: None,
        };
        let mut d = combo::Combo::from_project(&p, &root).unwrap();
        d.picks[0].put_solve(add(), true);
        assert_eq!(d.picks[0].ops.len(), 2);
        assert_eq!(solves(&d), [7]);
        d.picks[0].drop_solve(6, SolveWhat::T);
        assert_eq!(solves(&d), Vec::<usize>::new());
        // 外したあとに置いて外しても同じ
        c.picks[0].put_solve(add(), false);
        assert_eq!(solves(&c), [7]);
        c.picks[0].drop_solve(6, SolveWhat::T);
        assert_eq!(solves(&c), Vec::<usize>::new());
        assert_eq!(c.picks[0].ops.len(), 1);
        // 外したあとにピックアップも外す操作を置ける（同じ面・量の操作は 1 つ）
        c.picks[0].drop_pickup(6, "t".into());
        c.picks[0].drop_pickup(6, "t".into());
        assert_eq!(c.picks[0].ops.len(), 2);
    }
}
