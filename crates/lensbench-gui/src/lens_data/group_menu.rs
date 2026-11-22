//! レンズデータの表の右クリックの欄の群（組合せの群にする・解除する）

use egui::{RichText, Ui};

use super::{Act, Block, CellMenu, RowMenu};
use crate::i18n::{tr, trf};

/// 右クリックの欄の群: 選んだ範囲（Shift で選ぶ）の要素を群にする・行の入っている群を解除する（内の群から）
pub(super) fn group_menu(
    ui: &mut Ui,
    cm: &mut CellMenu,
    m: &mut RowMenu,
    j: usize,
    span: Option<[usize; 2]>,
    blocks: &[Block],
    groups: &[(String, Option<String>)],
) {
    let span = span.unwrap_or([j, j]);
    ui.label(
        RichText::new(if span[0] == span[1] {
            trf("Group the element of surface {}", &[&j])
        } else {
            trf(
                "Group the elements of surfaces {}–{}",
                &[&span[0], &span[1]],
            )
        })
        .strong(),
    );
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut cm.group_name)
                .hint_text(tr("Group name"))
                .desired_width(110.0),
        );
        crate::widgets::name_field(&r, tr("Group name"), &cm.group_name);
        let enter = r.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter));
        if ui
            .button(tr("Make group"))
            .on_hover_text(tr(
                "Select whole elements with Shift. Inside a group makes a nested group; a range containing whole groups wraps them",
            ))
            .clicked()
            || enter
        {
            m.acts.push(Act::MakeGroup {
                span,
                name: std::mem::take(&mut cm.group_name).trim().to_string(),
            });
        }
    });
    // 行 j の入っている群（内から外へ）
    let mut inside: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.surfs.contains(&j) && groups.iter().any(|g| g.0 == b.name))
        .collect();
    inside.sort_by_key(|b| b.surfs.len());
    for b in inside {
        let lost = groups
            .iter()
            .find(|g| g.0 == b.name)
            .and_then(|g| g.1.clone());
        let r = ui.button(trf("Ungroup {}", &[&b.name]));
        let r = match &lost {
            Some(l) => r.on_hover_text(trf(
                "The elements and inner groups stay. Also removes: {}",
                &[l],
            )),
            None => r.on_hover_text(tr("The elements and inner groups stay")),
        };
        if r.clicked() {
            m.acts.push(Act::Ungroup(b.name.clone()));
        }
        if let Some(l) = lost {
            ui.label(
                RichText::new(trf("Ungrouping {} also removes: {}", &[&b.name, &l]))
                    .small()
                    .color(ui.visuals().warn_fg_color),
            );
        }
    }
}
