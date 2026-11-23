//! 組合せの機構部品の欄（部品ごとに、名前・品番・種類と寸法の行と、置く所・保持・反転の行）。catalog.rs から分けた

use super::*;
use crate::combo::MechPick;

/// 組合せの機構部品: 置く要素・面・ずれ・保持する要素。部品はカタログの機構部品の行から足す
pub(super) fn mech_ui(ui: &mut Ui, ms: &mut Vec<MechPick>, ids: &[String]) {
    ui.label(RichText::new(tr("Mechanical parts")).strong()).on_hover_text(tr(
        "Tubes, cage plates and so on. Adding a mechanical part row from the catalog places it at the selected element. Check vignetting with the \"Mechanical vignetting\" analysis",
    ));
    let mut remove = None;
    let names: Vec<String> = ms.iter().map(|m| m.name.clone()).collect();
    let sel = crate::combo::mech::selected(ui.ctx());
    let scroll = crate::combo::mech::scroll_id();
    for (k, m) in ms.iter_mut().enumerate() {
        if k > 0 {
            ui.separator();
        }
        ui.push_id(("mech-row", k), |ui| {
            // 1 行目: 名前・品番・種類と寸法（収まらなければ切って、指すと全体）・✖
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(90.0))
                    .on_hover_text(tr("Name of the part in the project"));
                let found = m
                    .lookup()
                    .map(|it| (crate::combo::mech::mech_info(&it), it.description.clone()));
                let src = trf("Catalog {}", &[&m.catalog]);
                // 品番を押すと部品を選ぶ（レイアウト図でもその壁が目立つ。図で選んでもここが目立つ）
                let selected = sel.as_deref() == Some(m.name.as_str());
                let (text, hover) = match &found {
                    Ok((_, desc)) if desc.is_empty() => (RichText::new(&m.item).strong(), src),
                    Ok((_, desc)) => (RichText::new(&m.item).strong(), format!("{desc}\n{src}")),
                    Err(e) => (
                        RichText::new(format!("⚠ {}", m.item)).color(ui.visuals().warn_fg_color),
                        e.clone(),
                    ),
                };
                let r = ui.selectable_label(selected, text).on_hover_text(hover);
                if r.clicked() {
                    let name = (!selected).then_some(m.name.as_str());
                    crate::combo::mech::select(ui.ctx(), name);
                }
                if selected
                    && ui
                        .ctx()
                        .data_mut(|d| d.remove_temp::<bool>(scroll).is_some())
                {
                    r.scroll_to_me(Some(egui::Align::Center));
                }
                let w = (ui.available_width() - 28.0).max(0.0);
                if let Ok((info, _)) = &found {
                    ui.allocate_ui_with_layout(
                        egui::vec2(w, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add(egui::Label::new(RichText::new(info).weak()).truncate())
                                .on_hover_text(info);
                        },
                    );
                } else {
                    ui.add_space(w);
                }
                if ui.button("✖").on_hover_text(tr("Remove")).clicked() {
                    remove = Some(k);
                }
            });
            // 2 行目: 置く所・面と z・保持・座に合わせる・反転。幅が足りなければ部品ごとに折り返す
            // （字の途中では折り返さない。前は列の幅が足りずチェックの字が 1 字ずつ縦に並んでいた）
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                place_ui(ui, m, k, &names[..k], ids);
            });
        });
    }
    if let Some(k) = remove {
        ms.remove(k);
    }
}

fn place_ui(ui: &mut Ui, m: &mut MechPick, k: usize, prev: &[String], ids: &[String]) {
    let none = tr("(surface 1)").to_string();
    // 消した・名前の変わった要素は ⚠ を付けて選び直してもらう
    let show = |id: &str| {
        if stale_id(id, ids) {
            format!("⚠ {id}")
        } else {
            id.to_string()
        }
    };
    let gone = tr("The element is gone (removed or renamed). Choose it again");
    // 置く所: 要素か、前の機構部品の後ろ（つなぐ）
    let after_text = |n: &str| trf("Connect after {}", &[&n]);
    let stale_after = !m.after.is_empty() && !prev.contains(&m.after);
    egui::ComboBox::from_id_salt(("mech-at", k))
        .selected_text(if !m.after.is_empty() {
            if stale_after {
                format!("⚠ {}", after_text(&m.after))
            } else {
                after_text(&m.after)
            }
        } else if m.at.is_empty() {
            none.clone()
        } else {
            show(&m.at)
        })
        .show_ui(ui, |ui| {
            let mut pick = |ui: &mut Ui, at: &str, after: &str, label: String| {
                let sel = m.at == at && m.after == after;
                if ui.selectable_label(sel, label).clicked() {
                    m.at = at.to_string();
                    m.after = after.to_string();
                    if !after.is_empty() {
                        m.seat = false;
                    }
                }
            };
            pick(ui, "", "", none.clone());
            for id in ids {
                pick(ui, id, "", id.clone());
            }
            for n in prev {
                pick(ui, "", n, after_text(n));
            }
        })
        .response
        .on_hover_text(if stale_id(&m.at, ids) || stale_after {
            gone
        } else if !m.after.is_empty() {
            tr("Connects to the back end of the earlier part. If both ends have threads, checks that they mate and overlaps by the male thread length")
        } else {
            tr("Element to place at")
        });
    ui.horizontal(|ui| {
        ui.label(tr("Surface"));
        ui.add(crate::widgets::Num::new(&mut m.face).range(0..=99))
            .on_hover_text(tr("Which surface of the element to place at (0-based)"));
        ui.label("z");
        ui.add(crate::widgets::Num::new(&mut m.offset[2]).suffix(" mm"))
            .on_hover_text(tr(
                "Offset from the placement surface along the light direction",
            ));
    });
    let held = m.holds.first().cloned().unwrap_or_default();
    let mut sel = held.clone();
    egui::ComboBox::from_id_salt(("mech-holds", k))
        .selected_text(if sel.is_empty() {
            tr("Holds nothing").to_string()
        } else {
            show(&sel)
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut sel, String::new(), tr("Holds nothing"));
            for id in ids {
                ui.selectable_value(&mut sel, id.clone(), id);
            }
        })
        .response
        .on_hover_text(if stale_id(&held, ids) {
            gone
        } else {
            tr("Elements the part holds (checks protrusion and hold diameter)")
        });
    if sel != held {
        m.holds = if sel.is_empty() {
            Vec::new()
        } else {
            vec![sel]
        };
    }
    ui.add_enabled(
        !m.holds.is_empty() && m.after.is_empty(),
        egui::Checkbox::new(&mut m.seat, tr("Seat on element")),
    )
    .on_hover_text(tr("Places the part so that its seat (the face the held element rests on) meets the front of the held element (instead of the element and face to place on; z is an offset from there)"));
    ui.checkbox(&mut m.reversed, tr("Reverse"))
        .on_hover_text(tr("Places the part reversed (light enters from its back end; bores, seat and end threads are swapped too)"));
}
