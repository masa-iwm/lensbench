//! 組合せの系の絞り・ソルブ・ピックアップ: 面の表の操作を組合せの要素の surface_ops にする（app.rs から分けた）

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
                    let o = SolveOp {
                        surface: here(i),
                        other: o.other.as_ref().map(other).transpose()?,
                        kind: o.kind,
                        value: o.value,
                        param: o.param,
                    };
                    let c = o.is_curv();
                    let Some(p) = self.doc.combo.el_mut(k) else {
                        return Err(outside());
                    };
                    p.ops.retain(|q| {
                        !matches!(q, SurfaceOp::AddSolve(q) if q.surface == here(i) && q.is_curv() == c)
                    });
                    p.ops.push(SurfaceOp::AddSolve(o));
                }
                EditOp::RemoveSolve { surface, what } => {
                    let (k, i) = face(&surface)?;
                    let c = what == SolveWhat::C;
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .ops
                        .retain(|q| {
                            !matches!(q, SurfaceOp::AddSolve(q) if q.surface == here(i) && q.is_curv() == c)
                        });
                }
                EditOp::AddPickup(o) => {
                    let (k, i) = face(&o.dst)?;
                    let o = PickupOp {
                        dst: here(i),
                        src: other(&o.src)?,
                        what: o.what,
                        scale: o.scale,
                        offset: o.offset,
                        from: o.from,
                    };
                    let Some(p) = self.doc.combo.el_mut(k) else {
                        return Err(outside());
                    };
                    p.ops.retain(|q| {
                        !matches!(q, SurfaceOp::AddPickup(q) if q.dst == here(i) && q.what == o.what)
                    });
                    p.ops.push(SurfaceOp::AddPickup(o));
                }
                EditOp::RemovePickup { dst, what } => {
                    let (k, i) = face(&dst)?;
                    self.doc
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .ops
                        .retain(|q| {
                            !matches!(q, SurfaceOp::AddPickup(q) if q.dst == here(i) && q.what == what)
                        });
                }
                _ => return Err(trf("{} is not available in a combination", &[&name])),
            }
        }
        Ok(())
    }
}
