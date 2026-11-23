//! 組合せの要素・機構部品を、カタログで選んだ品に差し替える（id・間隔・置き方などは残す）。catalog.rs から分けた

use super::*;

/// 選んだ品 `r` で、組合せの図で選んでいる玉（`doc.insert_after`）か、選んでいる機構部品を差し替えるボタン
pub(super) fn replace_ui(ui: &mut Ui, doc: &mut ComboDoc, r: &LensRow) -> Option<Action> {
    if r.error.is_some() || r.is_incomplete() {
        return None;
    }
    let res = match r.kind {
        RowKind::Lens => {
            let k = doc.insert_after?;
            let p = doc.combo.el(k).filter(|p| p.is_lens_like())?;
            let name = if p.id.is_empty() {
                p.label()
            } else {
                p.id.clone()
            };
            let b = ui
                .button(trf("⇄ Replace {}", &[&name]))
                .on_hover_text(tr("Replaces the lens selected in the layout figure with this lens. Its id, the gaps before and after, its group and its place are kept (undo with Ctrl+Z)"));
            if !b.clicked() {
                return None;
            }
            doc.replace_row(k, r)
        }
        RowKind::Mech => {
            let name = crate::combo::mech::selected(ui.ctx())
                .filter(|n| doc.combo.mechanics.iter().any(|m| &m.name == n))?;
            let b = ui
                .button(trf("⇄ Replace {}", &[&name]))
                .on_hover_text(tr("Replaces the item of the mechanical part selected in the layout figure (or in the mechanical parts list) with this part. Placement, held elements, seat, connection and reversal are kept (undo with Ctrl+Z)"));
            if !b.clicked() {
                return None;
            }
            doc.replace_mech(&name, r)
        }
        _ => return None,
    };
    Some(match res {
        Ok(t) => Action::Notice(t),
        Err(e) => Action::Fail(e),
    })
}
