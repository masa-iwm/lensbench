//! 読み込んだ後の変更を別に読んだ系へ当て直す

use super::*;

/// 別に読んだ光束の系に、読み込んだ後の変更（set・edit）を順に当てる。set のパラメータの値が、読み込んだ系で
/// 変える前の値と違うもの（構成・状態で値が違うもの）は当てずに notes に書く。
/// プロジェクトの別の状態・経路では、面は番号でなく面の名前で探す（from は読み込んだ系、to は s の面の名前。
/// [`lensbench_project::retarget::Retarget`]）。s に無い面への変更は当てずに notes に書く
pub(super) fn replay(
    mut s: System,
    log: &[Change],
    (from, to): (&[Option<FaceId>], &[Option<FaceId>]),
    beam: &str,
    notes: &mut Vec<String>,
) -> lensbench_core::Result<System> {
    let e = |e: lensbench_core::Error| e.at(beam);
    // 面の名前が両方にあるときだけ名前で探す（.zmx の構成などは番号のまま）
    let mut rt = (!from.is_empty() && !to.is_empty()).then(|| Retarget::new(from, to));
    let skip = |op: String, why: String, notes: &mut Vec<String>| {
        let n = format!("{what}: {op} is not applied ({why})", what = beam);
        if !notes.contains(&n) {
            notes.push(n);
        }
    };
    for c in log {
        match c {
            Change::Set(v) => {
                let mut ch = Vec::with_capacity(v.len());
                for (name, before, value) in v {
                    let name = match rt.as_ref().map(|r| r.param(name)) {
                        None => name.clone(),
                        Some(Ok(n)) => n,
                        Some(Err(why)) => {
                            skip(format!("set {name}"), why, notes);
                            continue;
                        }
                    };
                    let name = &name;
                    let q = s.param(name).map_err(e)?;
                    let now = s.get_param(q).map_err(e)?;
                    if now == *before {
                        ch.push((q, *value));
                    } else {
                        let n = format!(
                            "{what}: {name} differs from the loaded system ({now}, loaded system {before}), so set is not applied",
                            what = beam
                        );
                        if !notes.contains(&n) {
                            notes.push(n);
                        }
                    }
                }
                if !ch.is_empty() {
                    s = s.with_params(&ch).map_err(e)?;
                }
            }
            Change::Edit(ops) => {
                let ops = match rt.as_mut() {
                    None => ops.clone(),
                    Some(r) => {
                        let mut out = Vec::with_capacity(ops.len());
                        for o in ops {
                            match r.op(o) {
                                Ok(x) => out.push(x),
                                Err(why) => skip(o.name(), why, notes),
                            }
                        }
                        out
                    }
                };
                lensbench_project::edit_ops::apply(&mut s, &ops)
                    .map_err(|x| format!("{beam}: {x}"))?;
            }
        }
    }
    Ok(s)
}
