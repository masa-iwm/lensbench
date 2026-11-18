//! .zmx のソルブ: 曲率・厚さのソルブと、ZPL マクロのソルブ

use super::*;

/// 面の行の曲率・厚さのソルブ（値つき）
pub(super) fn solves(raw: &Raw) -> Vec<Solve> {
    raw.surfs
        .iter()
        .enumerate()
        .flat_map(|(k, r)| {
            [r.csolve, r.tsolve]
                .into_iter()
                .flatten()
                .map(move |(kind, h)| Solve {
                    surf: k,
                    kind: kind.with_value(h),
                })
        })
        .collect()
}

/// ZPL Macro Solveのマクロを探して系に付ける。マクロが見つからない・読めない・対応していない命令を使う・
/// 今の系で値が出ないときは、ソルブを付けずに保存された値で固定し、理由を [`System::macro_notes`] に残す
pub(super) fn attach_macros(s: &mut System, raw: &Raw, opt: &LoadOptions, base: Option<&Path>) {
    let dirs = crate::zpl_solve::macro_dirs(&opt.macro_dirs, base);
    for (k, r) in raw.surfs.iter().enumerate() {
        // 曲率・厚さはソルブ（Solve）、ほかの列は MacroParm
        enum T {
            Curv,
            Thick,
            Col(crate::param::Kind),
        }
        let mut todo: Vec<(T, &String)> = Vec::new();
        todo.extend(r.cmacro.iter().map(|n| (T::Curv, n)));
        todo.extend(r.tmacro.iter().map(|n| (T::Thick, n)));
        todo.extend(
            r.kmacro
                .iter()
                .map(|n| (T::Col(crate::param::Kind::Conic), n)),
        );
        todo.extend(
            r.smacro
                .iter()
                .map(|n| (T::Col(crate::param::Kind::Semi), n)),
        );
        todo.extend(
            r.pmacros
                .iter()
                .map(|(p, n)| (T::Col(crate::param::Kind::Parm(*p)), n)),
        );
        for (t, name) in todo {
            let what = match &t {
                T::Curv => "curvature".to_string(),
                T::Thick => "thickness".to_string(),
                T::Col(c) => crate::zpl_solve::MacroParm {
                    surf: k,
                    id: 0,
                    what: *c,
                }
                .label(),
            };
            let got = crate::zpl_solve::find_macro(name, &dirs).and_then(|m| {
                if let T::Col(c) = &t {
                    if crate::pickup::pick_get(&s.surfaces[k], *c).is_none() {
                        notimpl!("surface type {:?} {what}", s.surfaces[k].typ());
                    }
                }
                let o = s.run_macro_outcome(&m, k)?;
                Ok((m, o))
            });
            match got {
                Ok((m, o)) => {
                    // 途中で止まったマクロ: 値は OpticStudio と同じ（それまでの値か 0、または変えない）だが黙らない
                    if let Some(e) = &o.error {
                        s.rx_mut().macro_notes.push(match o.value {
                            Some(v) => format!(
                                "the {what} macro solve {name} of surface {k} stopped with an error and used {v} as OpticStudio does: {e}"
                            ),
                            None => format!(
                                "the {what} macro solve {name} of surface {k} stopped with an error and left the value as OpticStudio does: {e}"
                            ),
                        });
                    }
                    let id = match s
                        .macros
                        .iter()
                        .position(|x| x.name == m.name && x.source == m.source)
                    {
                        Some(i) => i,
                        None => {
                            s.rx_mut().macros.push(m);
                            s.macros.len() - 1
                        }
                    } as u16;
                    match t {
                        T::Curv | T::Thick => s.rx_mut().solves.push(Solve {
                            surf: k,
                            kind: if matches!(t, T::Curv) {
                                SolveKind::MacroCurv { index: id }
                            } else {
                                SolveKind::MacroThick { index: id }
                            },
                        }),
                        T::Col(what) => s.rx_mut().macro_parms.push(crate::zpl_solve::MacroParm {
                            surf: k,
                            id,
                            what,
                        }),
                    }
                }
                Err(e) => s.rx_mut().macro_notes.push(format!(
                    "removed the {what} macro solve {name} of surface {k} and fixed the saved value: {e}"
                )),
            }
        }
    }
}
