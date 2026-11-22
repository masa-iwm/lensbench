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

/// 面の行の ZPL マクロのソルブを依頼の表にする（面の順、面の中は曲率・厚さ・円錐係数・半直径・機械半径・パラメータの順）。
/// マクロはここで探して読み、今の系で試すのは [`System::from_import`]
pub(super) fn requests(raw: &Raw, opt: &LoadOptions, base: Option<&Path>) -> Vec<MacroRequest> {
    let dirs = lensbench_core::zpl_solve::macro_dirs(&opt.macro_dirs, base);
    let mut out = Vec::new();
    for (k, r) in raw.surfs.iter().enumerate() {
        let mut todo: Vec<(MacroTarget, &String)> = Vec::new();
        todo.extend(r.cmacro.iter().map(|n| (MacroTarget::Curv, n)));
        todo.extend(r.tmacro.iter().map(|n| (MacroTarget::Thick, n)));
        todo.extend(r.kmacro.iter().map(|n| (MacroTarget::Col(Kind::Conic), n)));
        todo.extend(r.smacro.iter().map(|n| (MacroTarget::Col(Kind::Semi), n)));
        todo.extend(
            r.mmacro
                .iter()
                .map(|n| (MacroTarget::Col(Kind::MechSemi), n)),
        );
        todo.extend(
            r.pmacros
                .iter()
                .map(|(p, n)| (MacroTarget::Col(Kind::Parm(*p)), n)),
        );
        for (target, name) in todo {
            out.push(MacroRequest {
                surf: k,
                target,
                name: name.clone(),
                found: lensbench_core::zpl_solve::find_macro(name, &dirs),
            });
        }
    }
    out
}
