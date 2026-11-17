//! 展開した系にした面の編集（serve の `edit`・GUI の面の表）を、面を足したブロックの面の上書き・操作
//! （[`FaceMods`]）としてプロジェクトに書き戻す。面は展開の面の名前（[`FaceId`]）でブロックへ戻す

use crate::edit_spec::{BlockFace, BlockRef, EditOp, Ref};
use crate::{Block, FaceId, FaceMods, NodePath, NodeStep, Project, SurfaceEdit, SurfaceOp};
use lensbench_core::System;
use serde_json::{Map, Value};

/// 編集した系とその面の名前（[`crate::SurfOrigin::face`]、系の面と同じ並び）
#[derive(Clone, Copy)]
pub struct Edited<'a> {
    pub system: &'a System,
    pub faces: &'a [Option<FaceId>],
}

/// 木の位置のブロック
fn node_mut<'p>(blocks: &'p mut [Block], path: &NodePath) -> Option<&'p mut Block> {
    let (first, rest) = path.0.split_first()?;
    let b = match *first {
        NodeStep::Block(i) => blocks.get_mut(i)?,
        NodeStep::Arm(..) => return None,
    };
    let mut cur = b;
    for st in rest {
        cur = match (cur, *st) {
            (Block::Group { blocks, .. }, NodeStep::Block(i)) => blocks.get_mut(i)?,
            (Block::Splitter { spec, .. }, NodeStep::Arm(a, i)) => match a {
                0 => spec.t.get_mut(i)?,
                1 => spec.r.get_mut(i)?,
                2 => spec.d.get_mut(i)?,
                _ => return None,
            },
            _ => return None,
        };
    }
    Some(cur)
}

fn node<'p>(blocks: &'p [Block], path: &NodePath) -> Option<&'p Block> {
    let (first, rest) = path.0.split_first()?;
    let mut cur = match *first {
        NodeStep::Block(i) => blocks.get(i)?,
        NodeStep::Arm(..) => return None,
    };
    for st in rest {
        cur = match (cur, *st) {
            (Block::Group { blocks, .. }, NodeStep::Block(i)) => blocks.get(i)?,
            (Block::Splitter { spec, .. }, NodeStep::Arm(a, i)) => match a {
                0 => spec.t.get(i)?,
                1 => spec.r.get(i)?,
                2 => spec.d.get(i)?,
                _ => return None,
            },
            _ => return None,
        };
    }
    Some(cur)
}

/// 展開した系の面 j を書き戻せるブロックの面（往路の 1 回目の、座標ブレークでない面だけ）
fn face_of(x: Edited, r: &Ref) -> Result<(FaceId, usize), String> {
    let j = match r {
        Ref::Index(j) => *j,
        Ref::Name(n) => x.system.find_surface(n).map_err(|e| e.to_string())?,
    };
    let f = x
        .faces
        .get(j)
        .cloned()
        .flatten()
        .ok_or_else(|| format!("surface {j} is not a surface of a block"))?;
    if f.cb {
        return Err(format!(
            "surface {j} is a coordinate break of a block (use the block decenter and tilt)"
        ));
    }
    if f.back || f.pass > 0 {
        return Err(format!(
            "surface {j} is on a later pass of a block (edit the surface on its first forward pass)"
        ));
    }
    Ok((f.clone(), f.index))
}

/// 面のブロックが往復する（復路の面がある）
fn round_trip(x: Edited, f: &FaceId) -> bool {
    x.faces.iter().flatten().any(|g| g.node == f.node && g.back)
}

/// ほかの面を指す面（ピックアップの元・ソルブの参照の面）: id のあるブロックの面
fn other_ref(p: &Project, x: Edited, r: &Ref) -> Result<BlockRef, String> {
    let (f, i) = face_of(x, r)?;
    let id = node(&p.blocks, &f.node).map(crate::block_id).unwrap_or("");
    if id.is_empty() {
        return Err(format!(
            "surface {} is in a block without an id (give the block an id to refer to it)",
            match r {
                Ref::Index(j) => j.to_string(),
                Ref::Name(n) => n.clone(),
            }
        ));
    }
    Ok(BlockRef::Block(BlockFace {
        block: id.to_string(),
        surface: i,
    }))
}

/// 木の位置のブロックの面の上書き・操作
fn faces<'p>(p: &'p mut Project, n: &NodePath) -> Result<&'p mut FaceMods, String> {
    node_mut(&mut p.blocks, n)
        .ok_or("the block is not in the project")?
        .faces_mut()
        .ok_or_else(|| "this kind of block can't hold surface edits".into())
}

/// 書き戻す 1 つ: 面の上書きか操作
enum Mod {
    Edit(SurfaceEdit),
    Op(SurfaceOp),
}

/// 操作 1 つを、書き戻す先のブロックと上書き・操作に
fn to_mod(p: &Project, x: Edited, op: &EditOp) -> Result<(NodePath, Mod), String> {
    use EditOp as E;
    let edit = |r: &Ref, fields: Map<String, Value>| -> Result<(NodePath, Mod), String> {
        let (f, i) = face_of(x, r)?;
        // ブロックの最後の面の厚さには後ろの空気のブロックが入っている（状態で変わる間隔も）。上書きすると全部の
        // 状態で固定されるので、空気のブロックで変える。往復するブロックの最後の面の後の媒質（材料など）も同じ
        // （展開で通らない）
        let last = !x
            .faces
            .iter()
            .flatten()
            .any(|g| g.node == f.node && !g.cb && !g.back && g.pass == 0 && g.index > f.index);
        let gap = |k: &str| matches!(k, "t" | "material" | "dn" | "disp");
        if last && fields.contains_key("t") {
            return Err(
                "the thickness after the last surface of a block includes the following air (change the air block)"
                    .into(),
            );
        }
        if last && round_trip(x, &f) && fields.keys().any(|k| gap(k)) {
            return Err(
                "the medium after the last surface of a round-trip block is the following air (change the air block)"
                    .into(),
            );
        }
        Ok((f.node, Mod::Edit(SurfaceEdit { surface: i, fields })))
    };
    // 当てる面（surface・dst）はブロックの面の番号
    let own = |r: &Ref| -> Result<(NodePath, BlockRef), String> {
        let (f, i) = face_of(x, r)?;
        Ok((f.node, BlockRef::Sys(Ref::Index(i))))
    };
    Ok(match op {
        E::SetSurface { surface, fields } => edit(surface, fields.clone())?,
        E::SetSurfaceData { surface, data } => edit(surface, data.clone())?,
        E::SetMaterial { surface, material } => {
            let mut m = Map::new();
            m.insert("material".into(), Value::String(material.clone()));
            edit(surface, m)?
        }
        E::SetStop(o) => {
            let (n, r) = own(&o.surface)?;
            (
                n,
                Mod::Op(SurfaceOp::SetStop(crate::edit_spec::StopOp { surface: r })),
            )
        }
        E::AddSolve(o) => {
            let (n, r) = own(&o.surface)?;
            let other = o.other.as_ref().map(|q| other_ref(p, x, q)).transpose()?;
            (
                n,
                Mod::Op(SurfaceOp::AddSolve(crate::edit_spec::SolveOp {
                    surface: r,
                    kind: o.kind.clone(),
                    value: o.value,
                    other,
                    param: o.param,
                })),
            )
        }
        E::AddPickup(o) => {
            let (n, r) = own(&o.dst)?;
            let src = other_ref(p, x, &o.src)?;
            (
                n,
                Mod::Op(SurfaceOp::AddPickup(crate::edit_spec::PickupOp {
                    dst: r,
                    src,
                    what: o.what.clone(),
                    scale: o.scale,
                    offset: o.offset,
                    from: o.from.clone(),
                })),
            )
        }
        E::RemoveSolve { surface, what } => {
            let (n, r) = own(surface)?;
            (
                n,
                Mod::Op(SurfaceOp::RemoveSolve {
                    surface: r,
                    what: *what,
                }),
            )
        }
        E::RemovePickup { dst, what } => {
            let (n, r) = own(dst)?;
            (
                n,
                Mod::Op(SurfaceOp::RemovePickup {
                    dst: r,
                    what: what.clone(),
                }),
            )
        }
        E::SetParam { param, .. } => {
            return Err(format!(
                "parameter {param} can't be written back to blocks (only c, r, k and t of surfaces)"
            ))
        }
        _ => return Err("only surface edits can be written back to blocks".into()),
    })
}

/// 展開した系 x（プロジェクト p の展開。状態・経路は問わない、ブロックの木が同じこと）にした編集 ops を、面を足した
/// ブロックの [`FaceMods`] に足す。書き戻せない操作（面を入れる・消す、座標ブレーク・復路の面、面の上書きを
/// 持てないブロックの面、系の設定など）は書かずに、説明を返す。面を入れる・消す操作の後の操作は面の番号が
/// ずれるので書かない。check は足した後のプロジェクトを確かめる（展開できるか）。通らない上書きは外して説明を返す
/// ので、書き戻したプロジェクトは check を通る
pub fn write_back(
    p: &mut Project,
    x: Edited,
    ops: &[EditOp],
    check: &dyn Fn(&Project) -> Result<(), String>,
) -> Vec<String> {
    let mut notes = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        if matches!(
            op,
            EditOp::InsertSurface { .. } | EditOp::DeleteSurface { .. }
        ) {
            notes.push(format!(
                "edit {}: {} and the later edits are not written back (surfaces of blocks can't be inserted or deleted here)",
                k + 1,
                op.name()
            ));
            break;
        }
        let r = to_mod(p, x, op).and_then(|(n, m)| {
            let rt = matches!(m, Mod::Edit(_))
                && x.faces.iter().flatten().any(|g| g.node == n && g.back);
            let edit = matches!(m, Mod::Edit(_));
            let f = faces(p, &n)?;
            match m {
                Mod::Edit(e) => f.surface_edits.push(e),
                Mod::Op(o) => f.surface_ops.push(o),
            }
            if let Err(e) = check(p) {
                let f = faces(p, &n)?;
                if edit {
                    f.surface_edits.pop();
                } else {
                    f.surface_ops.pop();
                }
                return Err(format!("the project would not expand: {e}"));
            }
            Ok(rt)
        });
        match r {
            Ok(true) => notes.push(format!(
                "edit {}: {} also changes the return path of the round-trip block",
                k + 1,
                op.name()
            )),
            Ok(false) => {}
            Err(e) => notes.push(format!(
                "edit {}: {} is not written back ({e})",
                k + 1,
                op.name()
            )),
        }
    }
    notes
}
