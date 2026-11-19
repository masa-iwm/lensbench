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
    /// 編集する前の系（プロジェクトを展開したまま。面の並びは system と同じ）。置くソルブ・ピックアップがブロックの
    /// 元（.zmx など）から出ていたかをこれで見る（編集した系では、編集で置いたものも出ている）
    pub before: &'a System,
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

/// 書き戻す 1 つ: 面の上書きか操作。ソルブ・ピックアップを置く操作は、系にその面のソルブ・ピックアップが
/// 出ていたか（ブロックの元のものを置き換えるか、[`crate::surface_ops`]）も持つ
enum Mod {
    Edit(SurfaceEdit),
    Op(SurfaceOp, bool),
}

/// 面の番号（名前の面は系で引く）
fn sys_index(x: Edited, r: &Ref) -> Option<usize> {
    match r {
        Ref::Index(j) => Some(*j),
        Ref::Name(n) => x.system.find_surface(n).ok(),
    }
}

/// ブロックの面の上書きに 1 つ足す。同じ面の同じ欄の前の上書きは外す（空になった上書きも）
fn put_edit(edits: &mut Vec<SurfaceEdit>, e: SurfaceEdit) {
    for q in edits
        .iter_mut()
        .filter(|q| q.surface == e.surface && q.coordbrk == e.coordbrk)
    {
        q.fields.retain(|k, _| !e.fields.contains_key(k));
    }
    edits.retain(|q| !q.fields.is_empty());
    edits.push(e);
}

/// ブロックの面の操作に 1 つ足す。ソルブ・ピックアップは同じ面・量のものを 1 つにまとめ、元のものを置き換える・
/// 外すときは外す操作も置く（[`crate::surface_ops`]）
fn put_op(ops: &mut Vec<SurfaceOp>, o: SurfaceOp, shown: bool) {
    use crate::surface_ops as so;
    match o {
        SurfaceOp::AddSolve(o) => so::put_solve(ops, o, shown),
        SurfaceOp::AddPickup(o) => so::put_pickup(ops, o, shown),
        SurfaceOp::RemoveSolve {
            surface: BlockRef::Sys(Ref::Index(i)),
            what,
        } => so::drop_solve(ops, i, what),
        SurfaceOp::RemovePickup {
            dst: BlockRef::Sys(Ref::Index(i)),
            what,
        } => so::drop_pickup(ops, i, what),
        o => ops.push(o),
    }
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
        Ok((
            f.node,
            Mod::Edit(SurfaceEdit {
                surface: i,
                coordbrk: false,
                fields,
            }),
        ))
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
                Mod::Op(
                    SurfaceOp::SetStop(crate::edit_spec::StopOp { surface: r }),
                    false,
                ),
            )
        }
        E::AddSolve(o) => {
            let (n, r) = own(&o.surface)?;
            let other = o.other.as_ref().map(|q| other_ref(p, x, q)).transpose()?;
            let j = sys_index(x, &o.surface);
            let shown = x
                .before
                .solves
                .iter()
                .any(|q| Some(q.surf) == j && q.kind.is_curv() == o.is_curv());
            (
                n,
                Mod::Op(
                    SurfaceOp::AddSolve(crate::edit_spec::SolveOp {
                        surface: r,
                        kind: o.kind.clone(),
                        value: o.value,
                        other,
                        param: o.param,
                        macro_ref: o.macro_ref.clone(),
                    }),
                    shown,
                ),
            )
        }
        E::AddPickup(o) => {
            let (n, r) = own(&o.dst)?;
            let src = other_ref(p, x, &o.src)?;
            let j = sys_index(x, &o.dst);
            let w = lensbench_core::pickup::pick_kind_parse(&o.what).ok();
            let shown = x
                .before
                .pickups
                .iter()
                .any(|q| Some(q.dst) == j && Some(q.what) == w);
            (
                n,
                Mod::Op(
                    SurfaceOp::AddPickup(crate::edit_spec::PickupOp {
                        dst: r,
                        src,
                        what: o.what.clone(),
                        scale: o.scale,
                        offset: o.offset,
                        from: o.from.clone(),
                    }),
                    shown,
                ),
            )
        }
        E::RemoveSolve { surface, what } => {
            let (n, r) = own(surface)?;
            (
                n,
                Mod::Op(
                    SurfaceOp::RemoveSolve {
                        surface: r,
                        what: *what,
                    },
                    false,
                ),
            )
        }
        E::RemovePickup { dst, what } => {
            let (n, r) = own(dst)?;
            (
                n,
                Mod::Op(
                    SurfaceOp::RemovePickup {
                        dst: r,
                        what: what.clone(),
                    },
                    false,
                ),
            )
        }
        E::SetParam { param, .. } => {
            return Err(format!(
                "parameter {param} can't be written back to blocks (only surface parameters)"
            ))
        }
        _ => return Err("only surface edits can be written back to blocks".into()),
    })
}

/// id のあるブロックの面の上書き・操作（木の全部のブロック、群の中・BS の口の先も）。書き戻しの前後で比べて、
/// 変わったブロックだけを返すのに使う（GUI の組合せの要素へ当てる）
pub fn face_mods(p: &Project) -> Vec<(&str, &FaceMods)> {
    fn walk<'p>(bs: &'p [Block], out: &mut Vec<(&'p str, &'p FaceMods)>) {
        for b in bs {
            let id = crate::block_id(b);
            if let (false, Some(f)) = (id.is_empty(), b.faces()) {
                out.push((id, f));
            }
            match b {
                Block::Group { blocks, .. } => walk(blocks, out),
                Block::Splitter { spec, .. } => {
                    for arm in [&spec.t, &spec.r, &spec.d] {
                        walk(arm, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&p.blocks, &mut out);
    out
}

/// パラメータの値を変える操作を、面の値・面のデータの上書きにする。厚さ・半径・曲率・コーニックは面の値、ほかの
/// 値（非球面係数・Zernike・偏心など）は、ここまでの操作をすべて順に当てた系の写し run に当てて、変わった面の
/// データの項目（同じ項目を先に変えた値も入った今の値）にする。ほかの操作はそのまま返し、run にも当てる。
/// 値の変わらないパラメータは None（書くものが無い）
fn param_op(run: &mut Option<System>, x: Edited, op: &EditOp) -> Result<Option<EditOp>, String> {
    let cur = run.get_or_insert_with(|| x.before.clone());
    let EditOp::SetParam { param, value } = op else {
        // 当てられない操作（書き戻せないものなど）は写しに当てずに進む。書けるかは後で to_mod が見る
        let mut next = cur.clone();
        if crate::edit_ops::apply(&mut next, std::slice::from_ref(op)).is_ok() {
            *cur = next;
        }
        return Ok(Some(op.clone()));
    };
    let no =
        || format!("parameter {param} can't be written back to blocks (only surface parameters)");
    let (s, k) = param.trim().rsplit_once('.').ok_or_else(no)?;
    let s = s.strip_prefix("surf:").unwrap_or(s);
    let j: usize = s.trim().parse().map_err(|_| no())?;
    use lensbench_core::param::{Kind, Param};
    let kind = Kind::parse(k).map_err(|e| e.to_string())?;
    let key = match kind {
        Kind::Curv => Some("c"),
        Kind::Radius => Some("r"),
        Kind::Conic => Some("k"),
        Kind::Thick => Some("t"),
        _ => None,
    };
    // 今と同じ値なら書くものは無い（注意も出さない）
    if cur.get_param(Param::new(j, kind)).ok() == Some(value.0) {
        return Ok(None);
    }
    let mut fields = Map::new();
    let next = cur
        .with_params(&[(Param::new(j, kind), value.0)])
        .map_err(|e| e.to_string())?;
    if let Some(key) = key {
        fields.insert(key.into(), Value::from(value.0));
        *cur = next;
        return Ok(Some(EditOp::SetSurface {
            surface: Ref::Index(j),
            fields,
        }));
    }
    let a = crate::edit_ops::surface_data(cur, j).map_err(|e| e.to_string())?;
    let b = crate::edit_ops::surface_data(&next, j).map_err(|e| e.to_string())?;
    for (k, v) in b["data"].as_object().into_iter().flatten() {
        if !matches!(k.as_str(), "semi" | "semi_auto") && a["data"].get(k) != Some(v) {
            fields.insert(k.clone(), v.clone());
        }
    }
    *cur = next;
    if fields.is_empty() {
        return Err(no());
    }
    Ok(Some(EditOp::SetSurfaceData {
        surface: Ref::Index(j),
        data: fields,
    }))
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
    let mut run = None;
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
        let op2 = match param_op(&mut run, x, op) {
            Ok(Some(o)) => o,
            Ok(None) => continue,
            Err(e) => {
                notes.push(format!(
                    "edit {}: {} is not written back ({e})",
                    k + 1,
                    op.name()
                ));
                continue;
            }
        };
        let r = to_mod(p, x, &op2).and_then(|(n, m)| {
            let rt = matches!(m, Mod::Edit(_))
                && x.faces.iter().flatten().any(|g| g.node == n && g.back);
            let f = faces(p, &n)?;
            let before = f.clone();
            match m {
                Mod::Edit(e) => put_edit(&mut f.surface_edits, e),
                Mod::Op(o, shown) => put_op(&mut f.surface_ops, o, shown),
            }
            if let Err(e) = check(p) {
                *faces(p, &n)? = before;
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
