//! 読込（.zmx など、書式ごとの読み手）で作った文書から系を作る: ピックアップの解決、ZPL マクロのソルブを今の系で
//! 試して付ける（だめなら保存された値で固定して理由を残す）、ソルブの解き直し、自動の半直径

use crate::error::Result;
use crate::param::Kind;
use crate::solves::{Solve, SolveKind};
use crate::system::{Prescription, System};
use crate::zpl_solve::{Macro, MacroParm};

/// マクロのソルブを付ける列
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MacroTarget {
    /// 曲率（ソルブ）
    Curv,
    /// 厚さ（ソルブ）
    Thick,
    /// ほかの列（[`MacroParm`]）
    Col(Kind),
}

/// 読込で付けたいマクロのソルブ（面 surf の列 target に、名前 name のマクロ）。found は読み手が探して読んだ結果
/// （見つからない・読めなければ誤り）
#[derive(Debug)]
pub struct MacroRequest {
    pub surf: usize,
    pub target: MacroTarget,
    pub name: String,
    pub found: Result<Macro>,
}

impl System {
    /// 読み手が作った文書 rx（ピックアップ・ソルブは書いたまま、値は解いていない）から系を作る。ピックアップを
    /// 解き、マクロのソルブの依頼 macros を順に今の系で試して付け、ソルブを解き直し、自動の半直径を光線から
    /// 求める（OpticStudio が読込でするのと同じ）。付けられないマクロは保存された値で固定し、理由を
    /// [`System::macro_warnings`] に残す
    pub fn from_import(rx: Prescription, macros: Vec<MacroRequest>) -> Result<System> {
        let mut s = System::from_rx(rx)?;
        s.check()?;
        if !s.pickups.is_empty() {
            // OpticStudio は読込でピックアップを解き直す（保存された値が古くても元の面に合わせる）
            s.resolve_pickups()?;
            s.changed()?;
        }
        for q in macros {
            attach_macro(&mut s, q);
        }
        // OpticStudio は読込でソルブを解き直す（保存した値が古くても同じ系になる）
        crate::zpl_solve::resolve_on_load(&mut s);
        s.solve_thickness()?;
        // 自動の半直径を光線から求める（OpticStudio は読込で求め直す）
        s.update_semi();
        // 玉モードの玉（lens1, lens2, …）を玉・群のパラメータで使えるように
        s.rx_mut().units = s.lens_units();
        Ok(s)
    }
}

/// マクロのソルブを 1 つ試して付ける。対応していない列・命令や、今の系で値が出ないときは付けずに理由を残す
fn attach_macro(s: &mut System, q: MacroRequest) {
    let MacroRequest {
        surf: k,
        target: t,
        name,
        found,
    } = q;
    let what = match t {
        MacroTarget::Curv => "curvature".to_string(),
        MacroTarget::Thick => "thickness".to_string(),
        MacroTarget::Col(c) => MacroParm {
            surf: k,
            id: 0,
            what: c,
        }
        .label(),
    };
    let got = found.and_then(|m| {
        if let MacroTarget::Col(c) = t {
            if crate::pickup::pick_get(&s.surfaces[k], c).is_none() {
                crate::notimpl!("surface type {:?} {what}", s.surfaces[k].typ());
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
                MacroTarget::Curv | MacroTarget::Thick => s.rx_mut().solves.push(Solve {
                    surf: k,
                    kind: if t == MacroTarget::Curv {
                        SolveKind::MacroCurv { index: id }
                    } else {
                        SolveKind::MacroThick { index: id }
                    },
                }),
                MacroTarget::Col(what) => {
                    s.rx_mut().macro_parms.push(MacroParm { surf: k, id, what })
                }
            }
        }
        Err(e) => s.rx_mut().macro_notes.push(format!(
            "removed the {what} macro solve {name} of surface {k} and fixed the saved value: {e}"
        )),
    }
}
