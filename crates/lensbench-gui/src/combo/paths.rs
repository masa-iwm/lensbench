//! 組合せの道具の列の BS の経路を選ぶ欄（combo.rs から分けた）

use super::*;

/// BS の経路（T・R の並び）を選ぶ欄（BS が無ければ出さない）。変えたら true
pub(super) fn path_select(ui: &mut Ui, combo: &mut Combo, paths: &[String]) -> bool {
    let any_bs = combo
        .all()
        .iter()
        .any(|l| combo.get(l).is_some_and(|p| p.kind == ElemKind::Splitter));
    if !any_bs {
        return false;
    }
    let st = combo.active_state().map(str::to_string);
    let base = combo.path.clone();
    let mut path = match &st {
        Some(n) => combo.state_path.get(n).cloned().unwrap_or(base.clone()),
        None => base.clone(),
    };
    let was = path.clone();
    ui.separator();
    let lbl = ui.label(RichText::new(tr("Path")).color(theme::weak_text(ui.visuals())))
        .on_hover_text(tr("Transmit T / reflect R at each BS in the order passed (missing letters use each BS's default port). The figure and analyses use this path; other paths can be drawn with \"Overlay configurations\""));
    let shown = if path.is_empty() {
        tr("Default").to_string()
    } else {
        path.clone()
    };
    egui::ComboBox::from_id_salt("combo_path")
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), tr("Default"));
            for p in paths {
                ui.selectable_value(&mut path, p.clone(), p.as_str());
            }
        })
        .response
        .labelled_by(lbl.id);
    ui.add(
        egui::TextEdit::singleline(&mut path)
            .desired_width(48.0)
            .hint_text("TR…"),
    );
    path = path
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| matches!(c, 'T' | 'R'))
        .collect();
    if let Some(n) = &st {
        let own = combo.state_path.contains_key(n);
        if own {
            if ui
                .small_button(tr("Use the base path"))
                .on_hover_text(trf("Drop the path changed in state {}", &[n]))
                .clicked()
            {
                combo.state_path.remove(n);
                return true;
            }
        } else {
            ui.label(
                RichText::new(tr("(same as base)"))
                    .small()
                    .color(theme::weak_text(ui.visuals())),
            )
            .on_hover_text(trf("Changing it makes a path for state {} only", &[n]));
        }
    }
    if path == was {
        return false;
    }
    match st {
        Some(n) => {
            combo.state_path.insert(n, path);
        }
        None => combo.path = path,
    }
    true
}
