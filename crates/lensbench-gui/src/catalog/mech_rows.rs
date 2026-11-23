//! 組合せの機構部品の欄（部品ごとに、名前・品番・種類と寸法の行と、置く所・保持・反転の行）。catalog.rs から分けた

use super::*;
use crate::combo::MechPick;

/// 組合せの機構部品: 置く要素・面・ずれ・保持する要素。部品はカタログの機構部品の行から足す
pub(super) fn mech_ui(
    ui: &mut Ui,
    ms: &mut Vec<MechPick>,
    ids: &[String],
    lenses: &[LensRow],
    held: &[Option<f64>],
) {
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
                let was = m.name.clone();
                let te = ui
                    .add(egui::TextEdit::singleline(&mut m.name).desired_width(90.0))
                    .on_hover_text(tr("Name of the part in the project"));
                // 選んでいる部品の名前を書き換えたら、選択も新しい名前に付け替える（名前で覚えているので）
                if te.changed() && sel.as_deref() == Some(was.as_str()) {
                    crate::combo::mech::select(ui.ctx(), Some(&m.name));
                }
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
                change_ui(ui, m, lenses, held.get(k).copied().flatten());
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

/// 部品ごとの、保持する要素（holds の最初）の径。自作の玉は面の半径の最大の 2 倍、カタログの玉は一覧の径
pub(super) fn held_diameters(
    c: &crate::combo::Combo,
    ids: &[String],
    lenses: &[LensRow],
) -> Vec<Option<f64>> {
    c.mechanics
        .iter()
        .map(|m| {
            let id = m.holds.first()?;
            let p = c.el(ids.iter().position(|x| x == id)?)?;
            let d = match p.kind {
                // 自作の玉は面の表があれば面の半径の最大、無ければ（面を書かない簡単な玉）玉の半径
                crate::combo::ElemKind::Lens => {
                    2.0 * p.surfs.iter().map(|r| r.semi).fold(p.semi, f64::max)
                }
                crate::combo::ElemKind::Catalog => {
                    lenses
                        .iter()
                        .find(|l| {
                            l.vendor == p.vendor && l.part == p.part && l.kind != RowKind::Mech
                        })?
                        .diameter
                }
                _ => 2.0 * p.semi,
            };
            (d.is_finite() && d > 0.0).then_some(d)
        })
        .collect()
}

/// 差し替えの候補: 部品と同じ種類の品（部品のカタログのファイルの品と、一覧の機構部品の行）。
/// （カタログのファイル、品番、種類と寸法の 1 行、保持する要素が保持径に入るか（径が分からなければ None））
pub(super) fn candidates(
    m: &MechPick,
    lenses: &[LensRow],
    held: Option<f64>,
) -> Vec<(String, String, String, Option<bool>)> {
    let Ok(cur) = m.lookup() else {
        return Vec::new();
    };
    let kind = cur.kind;
    let mut out: Vec<(String, String, String, Option<bool>)> = Vec::new();
    let fits = |raw: &Value| held.map(|d| lensbench_catalog::spec::fits_row(raw, d));
    if let Ok(c) = lensbench_project::UserCatalog::load_shared(std::path::Path::new(&m.catalog)) {
        for (name, it) in &c.items {
            if name == &m.item || it.mech().is_none() || it.kind != kind {
                continue;
            }
            let raw = serde_json::to_value(it).unwrap_or_default();
            out.push((
                m.catalog.clone(),
                name.clone(),
                crate::combo::mech::mech_info(it),
                fits(&raw),
            ));
        }
    }
    for r in lenses
        .iter()
        .filter(|r| r.kind == RowKind::Mech && !r.is_incomplete())
    {
        let Some(src) = r.source.as_deref() else {
            continue;
        };
        let same_kind = kind.is_none_or(|k| k.as_str() == r.category);
        let dup = out.iter().any(|o| o.0 == src && o.1 == r.part);
        if !same_kind || dup || (src == m.catalog && r.part == m.item) {
            continue;
        }
        out.push((src.to_string(), r.part.clone(), row_detail(r), fits(&r.raw)));
    }
    // 入るものを先に、品番の順
    out.sort_by(|a, b| {
        (a.3 == Some(false))
            .cmp(&(b.3 == Some(false)))
            .then(a.1.cmp(&b.1))
    });
    out
}

/// 「品を変える…」: 置く所・保持・座・つなぎ・反転は残し、カタログのファイルと品番だけ替える（Ctrl+Z で戻る）
fn change_ui(ui: &mut Ui, m: &mut MechPick, lenses: &[LensRow], held: Option<f64>) {
    ui.menu_button(tr("Change item…"), |ui| {
        let cands = candidates(m, lenses, held);
        if cands.is_empty() {
            ui.weak(tr("No other part of the same kind in the catalogs"));
            return;
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (cat, item, info, fit) in cands {
                let label = if fit == Some(false) {
                    format!("⚠ {item}  {info}")
                } else {
                    format!("{item}  {info}")
                };
                let hover = if fit == Some(false) {
                    format!("{}\n{cat}", tr("The held element does not fit the holding diameter"))
                } else {
                    cat.clone()
                };
                if ui.button(label).on_hover_text(hover).clicked() {
                    m.catalog = cat;
                    m.item = item;
                    ui.close();
                }
            }
        });
    })
    .response
    .on_hover_text(tr("Replaces the item with another part of the same kind; placement, held elements, seat, connection and reversal are kept"));
}
