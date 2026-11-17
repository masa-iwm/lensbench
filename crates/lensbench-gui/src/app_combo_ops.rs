//! 組合せの系の絞り・ソルブ・ピックアップ・面の挿入削除: 面の表の操作を組合せの要素の surface_ops・中身の面にする
//! （app.rs から分けた）

use super::*;
use lensbench_project::edit_ops::{EditOp, Ref};

impl Core {
    /// 組合せの系の面の操作（serve の edit の操作）を組合せの要素にする。絞り・面の挿入削除はここで、ソルブ・
    /// ピックアップは serve の write_back でブロックの面の操作にして、返事で要素に当てる（[`Self::take_write_back`]）。
    /// write_back を頼んだら true（組み直すのは返事の後）
    pub(super) fn combo_ops(
        &mut self,
        spans: &[Option<[usize; 2]>],
        ids: &[String],
        ops: &[Value],
    ) -> Result<bool, String> {
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
    ) -> Result<bool, String> {
        let outside = || tr("Surface outside the combination elements").to_string();
        let mut remote = Vec::new();
        for op in ops {
            // 絞りをやめる（GUI だけの操作）
            if op["op"] == "unset_stop" {
                self.doc.combo.set_stop_face(None);
                continue;
            }
            // 面の挿入・削除: 自作の玉・面の並びの中身の面を足し引きする。"after" は面 at−1 の後に足す
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
                            None => {
                                let pos = spans
                                    .iter()
                                    .position(|s| s.is_some_and(|s| s[0] > at))
                                    .unwrap_or(spans.len());
                                // 足した面の行は、次の要素の最初の面の行（無ければ像面の行）。前の絞りの行の前に
                                // 足したときも面は絞りの後に入るので、今のセルをそちらへ
                                let row = spans[pos..]
                                    .iter()
                                    .find_map(|s| s.map(|s| s[0]))
                                    .unwrap_or(self.sys.ld_rows.len().saturating_sub(1));
                                self.p.ld.insert_lands_at(row);
                                pos
                            }
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
            let raw = op;
            let name = op["op"].as_str().unwrap_or_default().to_string();
            let op = EditOp::deserialize(op).map_err(|e| format!("{name}: {e}"))?;
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
                // ソルブ・ピックアップ: serve で要素のブロックの面の操作にする（面の名前で要素と面を決める）
                EditOp::AddSolve(_)
                | EditOp::RemoveSolve { .. }
                | EditOp::AddPickup(_)
                | EditOp::RemovePickup { .. } => remote.push(raw.clone()),
                _ => return Err(trf("{} is not available in a combination", &[&name])),
            }
        }
        if remote.is_empty() {
            return Ok(false);
        }
        self.worker
            .send(Tag::WriteBack, "write_back", json!({ "ops": remote }));
        Ok(true)
    }

    /// serve の write_back の返事: 面の操作が変わったブロックを、同じ id の要素の edits・ops にする。固定した要素は
    /// 変えない
    pub(super) fn take_write_back(&mut self, v: &Value) -> Result<bool, String> {
        let ids = self.doc.combo.ids_all();
        let mut changed = false;
        let mut locked = false;
        for b in v["blocks"].as_array().into_iter().flatten() {
            let Some(loc) = ids
                .iter()
                .find(|(_, id)| b["id"].as_str() == Some(id.as_str()))
                .map(|(l, _)| l.clone())
            else {
                continue;
            };
            let Some(p) = self.doc.combo.get_mut(&loc) else {
                continue;
            };
            if p.locked {
                locked = true;
                continue;
            }
            let f = &b["faces"];
            let ops = match f.get("surface_ops") {
                Some(x) => serde_json::from_value(x.clone()).map_err(|e| e.to_string())?,
                None => Vec::new(),
            };
            let mut edits = Vec::new();
            for e in f["surface_edits"].as_array().into_iter().flatten() {
                let surface = e["surface"].as_u64().unwrap_or(0) as usize;
                for (k, x) in e["fields"].as_object().into_iter().flatten() {
                    edits.push(combo::SurfEdit {
                        surface,
                        field: k.clone(),
                        value: x.clone(),
                    });
                }
            }
            // 上書きは面ごとにまとめて返るので、並びだけ違うなら今のままに
            let key = |e: &combo::SurfEdit| (e.surface, e.field.clone(), e.value.to_string());
            let mut a: Vec<_> = p.edits.iter().map(key).collect();
            let mut b: Vec<_> = edits.iter().map(key).collect();
            a.sort();
            b.sort();
            if a != b {
                p.edits = edits;
                changed = true;
            }
            if p.ops != ops {
                p.ops = ops;
                changed = true;
            }
        }
        if locked {
            return Err(tr("A locked element cannot be changed").into());
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lensbench_project::edit_ops::{BlockRef, SolveOp, SolveWhat};
    use lensbench_project::surface_ops;

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
                    .map_err(lensbench_core::Error::from)
                })
                .map(|x| x.system)
                .unwrap();
            t.solves.iter().map(|q| q.surf).collect::<Vec<_>>()
        };
        assert_eq!(solves(&c), [7]);
        // 面 7 は要素の面 6
        surface_ops::drop_solve(&mut c.picks[0].ops, 6, SolveWhat::T);
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
        surface_ops::put_solve(&mut d.picks[0].ops, add(), true);
        assert_eq!(d.picks[0].ops.len(), 2);
        assert_eq!(solves(&d), [7]);
        surface_ops::drop_solve(&mut d.picks[0].ops, 6, SolveWhat::T);
        assert_eq!(solves(&d), Vec::<usize>::new());
        // 外したあとに置いて外しても同じ
        surface_ops::put_solve(&mut c.picks[0].ops, add(), false);
        assert_eq!(solves(&c), [7]);
        surface_ops::drop_solve(&mut c.picks[0].ops, 6, SolveWhat::T);
        assert_eq!(solves(&c), Vec::<usize>::new());
        assert_eq!(c.picks[0].ops.len(), 1);
        // 外したあとにピックアップも外す操作を置ける（同じ面・量の操作は 1 つ）
        surface_ops::drop_pickup(&mut c.picks[0].ops, 6, "t".into());
        surface_ops::drop_pickup(&mut c.picks[0].ops, 6, "t".into());
        assert_eq!(c.picks[0].ops.len(), 2);
    }
}
