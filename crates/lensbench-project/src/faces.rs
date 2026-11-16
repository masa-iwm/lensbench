//! ブロックの面の上書き・絞り・ソルブ・ピックアップ（[`FaceMods`]）を展開した系に当てる（PRJ-04）。
//! 面は [`SurfOrigin::mods`] でブロックに対応させる（id の無いブロックも）

use super::*;

/// プロジェクトのブロック・群の面の上書き・操作（[`FaceMods`]、木の順。.zmx の取り込みの群も）と、誤りの文で
/// 示す名前（id。無ければ種類）
pub(crate) fn face_sites<'p>(blocks: &'p [Block], out: &mut Vec<(String, &'p FaceMods)>) {
    let name = |id: &str, kind: &str| {
        if id.is_empty() {
            format!("({kind})")
        } else {
            id.to_string()
        }
    };
    for b in blocks {
        if let Some(f) = b.faces().filter(|f| !f.is_empty()) {
            let kind = match b {
                Block::Lens { .. } => "lens",
                Block::Stop { .. } => "stop",
                Block::Mirror { .. } => "mirror",
                Block::Surfaces { .. } => "surfaces",
                Block::Zmx { .. } => "zmx",
                Block::User { .. } => "user",
                _ => "group",
            };
            out.push((name(block_id(b), kind), f));
        }
        match b {
            Block::Group { blocks, .. } => face_sites(blocks, out),
            Block::Zmx { groups, .. } => {
                for g in groups.iter().filter(|g| !g.faces.is_empty()) {
                    out.push((name(&g.id, "group"), &g.faces));
                }
            }
            Block::Splitter { spec, .. } => {
                for l in [&spec.t, &spec.r, &spec.d] {
                    face_sites(l, out);
                }
            }
            _ => {}
        }
    }
}

/// 面の上書き・操作を持つブロック・群に、[`face_sites`] の順の番号（1 始まり、[`FaceMods::site`]）を付けた写し。
/// 持つものが無ければ None
pub(crate) fn number_sites(p: &Project) -> Option<Project> {
    fn walk(blocks: &mut [Block], n: &mut usize) {
        for b in blocks {
            if let Some(f) = b.faces_mut().filter(|f| !f.is_empty()) {
                *n += 1;
                f.site = *n;
            }
            match b {
                Block::Group { blocks, .. } => walk(blocks, n),
                Block::Zmx { groups, .. } => {
                    for g in groups.iter_mut().filter(|g| !g.faces.is_empty()) {
                        *n += 1;
                        g.faces.site = *n;
                    }
                }
                Block::Splitter { spec, .. } => {
                    for l in [&mut spec.t, &mut spec.r, &mut spec.d] {
                        walk(l, n);
                    }
                }
                _ => {}
            }
        }
    }
    let mut sites = Vec::new();
    face_sites(&p.blocks, &mut sites);
    if sites.is_empty() {
        return None;
    }
    let mut q = p.clone();
    let mut n = 0;
    walk(&mut q.blocks, &mut n);
    Some(q)
}

/// 範囲の面（座標ブレークを除く）
fn faces_in(s: &System, r: [usize; 2]) -> Vec<usize> {
    (r[0]..=r[1])
        .filter(|&j| !s.surfaces[j].is_coord_break())
        .collect()
}

/// id のブロック・群の往路の面 i（座標ブレークを除いて数える）。ブロックが系に無ければ None
pub(crate) fn block_face(
    s: &System,
    block: &str,
    i: usize,
    map: &[SurfOrigin],
) -> lensbench_core::Result<Option<usize>> {
    let Some(x) = fwd_run(map, block) else {
        return Ok(None);
    };
    let faces = faces_in(s, x);
    faces
        .get(i)
        .copied()
        .map(Some)
        .ok_or(format!("block {block} surface {i} ({} surfaces)", faces.len()).into())
}

/// 面の絞り・ソルブ・ピックアップを当てる（[`FaceMods::surface_ops`]。sites は [`face_sites`] で、その添字が
/// [`SurfOrigin::mods`] の番号）。当てなかった操作の注意と、set_stop を当てたかを返す
pub(crate) fn apply_surface_ops(
    s: &mut System,
    sites: &[(String, &FaceMods)],
    map: &[SurfOrigin],
) -> lensbench_core::Result<(Vec<String>, bool)> {
    let mut warn = Vec::new();
    let mut stopped = false;
    // 外す操作の面に、外すソルブ・ピックアップがあるか（名前で書いた面・読めない量は当てて誤りにする）
    let has_target = |s: &System, e: &EditOp| match e {
        EditOp::RemoveSolve {
            surface: Ref::Index(j),
            what,
        } => {
            let c = *what == crate::edit_spec::SolveWhat::C;
            s.solves
                .iter()
                .any(|q| q.surf == *j && q.kind.is_curv() == c)
        }
        EditOp::RemovePickup {
            dst: Ref::Index(j),
            what,
        } => match lensbench_core::pickup::pick_kind_parse(what) {
            Ok(w) => s.pickups.iter().any(|p| p.dst == *j && p.what == w),
            Err(_) => true,
        },
        _ => true,
    };
    for (site, (label, f)) in sites.iter().enumerate() {
        if f.surface_ops.is_empty() {
            continue;
        }
        // このブロックの往路の面（往路をたどらなければ当てない: 状態で外した・通らない経路など）
        let Some(own) = runs_where(map, |o| o.mods.contains(&site), Some(false))
            .first()
            .map(|&r| faces_in(s, r))
        else {
            continue;
        };
        let at = |k: usize, e: String| format!("block {} surface_ops {}: {e}", label, k);
        // ためた操作をまとめて当てる
        let flush = |s: &mut System,
                     out: &mut Vec<(usize, EditOp)>,
                     warn: &mut Vec<String>,
                     stopped: &mut bool|
         -> lensbench_core::Result<()> {
            if !out.is_empty() {
                *stopped |= out.iter().any(|o| matches!(o.1, EditOp::SetStop(_)));
                let ops: Vec<(usize, &EditOp)> = out.iter().map(|(k, o)| (*k, o)).collect();
                warn.extend(edit_ops::apply_labeled(s, &ops, Some(label))?);
                out.clear();
            }
            Ok(())
        };
        let mut out = Vec::new();
        for (k, op) in f.surface_ops.iter().enumerate() {
            let k = k + 1;
            // 当てる面（surface・dst）の番号はこのブロックの面
            let mut op = op.clone();
            if let BlockRef::Sys(Ref::Index(i)) = *op.target_mut() {
                let j = *own.get(i).ok_or_else(|| {
                    at(
                        k,
                        format!("surface {} ({} surfaces in the block)", i, own.len()),
                    )
                })?;
                *op.target_mut() = BlockRef::Sys(Ref::Index(j));
            }
            let mut missing = None;
            let e = op.to_edit(|r| match r {
                BlockRef::Sys(r) => Ok::<_, String>(Some(r)),
                BlockRef::Block(f) => {
                    match block_face(s, &f.block, f.surface, map)
                        .map_err(|x| at(k, x.to_string()))?
                    {
                        Some(j) => Ok(Some(Ref::Index(j))),
                        None => {
                            missing = Some(f.block);
                            Ok(None)
                        }
                    }
                }
            })?;
            match e {
                // 外す操作: 前の操作を当ててから、外すものが無ければ当てずに注意（ブロックの元が変わったなど）
                Some(e @ (EditOp::RemoveSolve { .. } | EditOp::RemovePickup { .. })) => {
                    flush(s, &mut out, &mut warn, &mut stopped)?;
                    if has_target(s, &e) {
                        flush(s, &mut vec![(k, e)], &mut warn, &mut stopped)?;
                    } else {
                        warn.push(at(
                            k,
                            format!("nothing to remove, so {} is not applied", op.name()),
                        ));
                    }
                }
                Some(e) => out.push((k, e)),
                None => warn.push(at(
                    k,
                    format!(
                        "block {} not found, so {} is not applied",
                        missing.unwrap_or_default(),
                        op.name()
                    ),
                )),
            }
        }
        flush(s, &mut out, &mut warn, &mut stopped)?;
    }
    Ok((warn, stopped))
}

/// 面の値の上書きを当てる（[`FaceMods::surface_edits`]。面はブロックの面のうち座標ブレークを除いた番号）。
/// 系に無い（状態で外した・通らない経路など）ブロックの上書きは当てない
pub(crate) fn apply_surface_edits(
    s: &mut System,
    sites: &[(String, &FaceMods)],
    map: &[SurfOrigin],
) -> lensbench_core::Result<()> {
    for (site, (label, f)) in sites.iter().enumerate() {
        if f.surface_edits.is_empty() {
            continue;
        }
        let here = |o: &SurfOrigin| o.mods.contains(&site);
        // 往路（と、同じ向きに通るほかの範囲）・復路
        let fwd: Vec<Vec<usize>> = runs_where(map, here, Some(false))
            .into_iter()
            .map(|r| faces_in(s, r))
            .collect();
        let back: Vec<Vec<usize>> = runs_where(map, here, Some(true))
            .into_iter()
            .map(|r| faces_in(s, r))
            .collect();
        let Some(n) = fwd.first().map(Vec::len).filter(|&n| n > 0) else {
            continue;
        };
        for e in &f.surface_edits {
            apply_surface_edit(s, label, e, n, &fwd, &back)?;
        }
    }
    Ok(())
}

/// 面の値の上書き 1 つ（n はブロックの面の数）
fn apply_surface_edit(
    s: &mut System,
    label: &str,
    e: &SurfaceEdit,
    n: usize,
    fwd: &[Vec<usize>],
    back: &[Vec<usize>],
) -> lensbench_core::Result<()> {
    let err = |why: lensbench_core::Error| {
        why.map_message(|why| {
            format!(
                "surface_edits: block {} surface {}: {why}",
                label, e.surface
            )
        })
    };
    {
        let i = e.surface;
        if i >= n {
            return Err(format!(
                "surface_edits: block {} surface {} ({n} surfaces)",
                label, e.surface
            )
            .into());
        }
        // 面の値の名前（SURF_FIELDS）と、それ以外の面のデータの項目（set_surface_data）
        let (named, data): (Map<String, Value>, Map<String, Value>) = e
            .fields
            .clone()
            .into_iter()
            .partition(|(k, _)| lensbench_core::edit::SURF_FIELDS.contains(&k.as_str()));
        let has_data = !data.is_empty();
        let f = edit_ops::surf_fields(&Value::Object(named))
            .map_err(|x| format!("surface_edits: {x}"))?;
        for v in fwd {
            let j = *v.get(i).ok_or_else(|| {
                err("the ranges in the same direction have different numbers of surfaces".into())
            })?;
            if !f.is_empty() {
                s.set_surface_fields(j, &f).map_err(err)?;
            }
            if has_data {
                edit_ops::set_surface_data(s, j, &data).map_err(err)?;
            }
        }
        if has_data && !back.is_empty() {
            if ["t", "mat", "dn", "disp"]
                .iter()
                .any(|k| data.get(*k).is_some())
            {
                return Err(err(
                    "change the thickness and material of a round-trip block with the names t and material".into(),
                ));
            }
            // 復路では面の前後が入れ替わり向きも逆になるので、前後の傾き偏心はそのまま当てられない
            if ["before", "after"].iter().any(|k| data.get(*k).is_some()) {
                return Err(err(
                    "tilts and decenters before or after surfaces of a round-trip block can't be changed (use the block decenter and tilt)"
                        .into(),
                ));
            }
            for v in back {
                if v.len() == n {
                    edit_ops::set_surface_data(s, v[n - 1 - i], &data).map_err(err)?;
                }
            }
        }
        // 復路は逆の順にたどる: 面の値（曲率・コーニック・半径など）は同じ面 n−1−i に同じ値、面の後の媒質の値
        // （厚さ・材料）は n−2−i に（厚さは符号が逆）。往路の最後の面の後はブロックの外（後の空気）なので、往復する
        // ブロックでは変えない
        for v in back {
            if v.len() != n {
                return Err(err(format!(
                    "{} surfaces on the return path ({n} on the way out)",
                    v.len()
                )
                .into()));
            }
            let (face, gap): (Vec<_>, Vec<_>) = f
                .iter()
                .cloned()
                .partition(|(k, _)| !matches!(k.as_str(), "t" | "material" | "dn" | "disp"));
            if !face.is_empty() {
                s.set_surface_fields(v[n - 1 - i], &face).map_err(err)?;
            }
            if !gap.is_empty() {
                if i + 1 == n {
                    return Err(err(
                        "change the thickness and material of the last surface of a round-trip block in the air block after it"
                            .into(),
                    ));
                }
                let gap: Vec<_> = gap
                    .into_iter()
                    .map(|(k, x)| match (k.as_str(), x) {
                        ("t", lensbench_core::edit::SurfValue::Num(t)) => {
                            (k, lensbench_core::edit::SurfValue::Num(-t))
                        }
                        (_, x) => (k, x),
                    })
                    .collect();
                s.set_surface_fields(v[n - 2 - i], &gap).map_err(err)?;
            }
        }
    }
    Ok(())
}
