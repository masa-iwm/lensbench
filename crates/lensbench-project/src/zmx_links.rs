//! zmx ブロックの展開で、取り込んだ .zmx の ZPL マクロとマクロのソルブを展開した系へ持ち越す

use crate::blocks::ZmxLinks;
use lensbench_core::solves::SolveKind;
use lensbench_core::surface::Surface;
use lensbench_core::System;
use std::collections::HashMap;

/// ZPL マクロを取り込みごとに系のマクロの後ろに足し、パラメータ・円錐係数・半直径のマクロのソルブを、取り込んだ面に
/// 移して足す（at は（取り込み, 元の面）→ 展開した系の面）。取り込みごとのマクロの番号のずれを返す
pub(crate) fn carry_macros(
    s: &mut System,
    zpicks: &[ZmxLinks],
    at: &HashMap<(usize, usize), usize>,
) -> lensbench_core::Result<Vec<usize>> {
    let mut moff = Vec::with_capacity(zpicks.len());
    for z in zpicks {
        moff.push(s.macros.len());
        if !z.macros.is_empty() {
            s.edit_raw(|rx| {
                rx.macros.extend(z.macros.iter().cloned());
                Ok(())
            })?;
        }
    }
    for (inst, z) in zpicks.iter().enumerate() {
        for q in &z.macro_parms {
            let Some(&k) = at.get(&(inst, q.surf)) else {
                continue;
            };
            let q = lensbench_core::zpl_solve::MacroParm {
                surf: k,
                id: (q.id as usize + moff[inst]) as u16,
                ..*q
            };
            s.edit_raw(|rx| {
                rx.macro_parms.push(q);
                Ok(())
            })?;
        }
    }
    Ok(moff)
}

/// 曲率・厚さのマクロのソルブのマクロの番号を off だけずらす
pub(crate) fn shift_macro(kind: &mut SolveKind, off: usize) {
    if let SolveKind::MacroCurv { index } | SolveKind::MacroThick { index } = kind {
        *index = (*index as usize + off) as u16;
    }
}

/// 面 1 から系の面 1 に取り込んだ .zmx の物体面は、系の物体面（元の面 0 を指すピックアップを残す。値が合うかは
/// ピックアップを足すときに確かめる）。物体面のコメントも持ち越す。像面も同じ
pub(crate) fn carry_object(
    s: &mut System,
    zpicks: &[ZmxLinks],
    at: &mut HashMap<(usize, usize), usize>,
) -> lensbench_core::Result<()> {
    let img = s.image();
    for (inst, z) in zpicks.iter().enumerate() {
        // 系の最後の取り込みの .zmx の像面は、系の像面（像面の半直径のピックアップなど。値が合うかは
        // ピックアップを足すときに確かめる）
        let last = at
            .iter()
            .filter(|(k, _)| k.0 == inst)
            .map(|(_, &j)| j)
            .max();
        if let Some(l) = last {
            if l < img && !at.iter().any(|(k, &j)| k.0 != inst && j > l) {
                at.insert((inst, z.image), img);
            }
        }
        if at.get(&(inst, 1)) == Some(&1) {
            at.insert((inst, 0), 0);
            if !z.obj_comment.is_empty() && s.surfaces[0].comment.is_empty() {
                let c = z.obj_comment.clone();
                s.edit_raw(|rx| {
                    rx.surfaces[0].comment = c;
                    Ok(())
                })?;
            }
        }
    }
    Ok(())
}

/// 取り込んだ .zmx のマクロのソルブの注意（解けずに保存された値で固定したもの）を持ち越す
pub(crate) fn carry_notes(rx: &mut lensbench_core::Prescription, zpicks: &[ZmxLinks]) {
    for z in zpicks {
        for n in &z.macro_notes {
            if !rx.macro_notes.contains(n) {
                rx.macro_notes.push(n.clone());
            }
        }
    }
}

/// プロジェクトの `object` の行から物体面を作る（厚さは呼ぶ側が `object_distance` で入れる）。`medium` は物体空間の
/// 媒質（液浸など。省略は空気）
pub(crate) fn object_surface(r: Option<&crate::sysfile_v2::Row>) -> Result<Surface, String> {
    let Some(r) = r else {
        return Ok(Surface::default());
    };
    if r.stop
        || r.image
        || r.t != 0.0
        || r.coordinate_break.is_some()
        || r.before.is_some()
        || r.after.is_some()
    {
        return Err(
            "object: stop, image, t, coordinate_break, before and after can't be written".into(),
        );
    }
    let mut r = r.clone();
    r.object = false;
    crate::sysfile_v2::read::surface(r).map_err(|e| format!("object: {e}"))
}
