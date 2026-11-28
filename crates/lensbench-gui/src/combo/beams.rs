//! 光束の組（プロジェクトの `beams`）の一覧と、足す・消す・変える欄（組合せの道具の列の「光束」）。
//! 値は組合せの系の設定（[`SysExtra::beams`]）にプロジェクトの形のまま持ち、保存・読み込みはそのまま往復する

use egui::{RichText, Ui};
use lensbench_project::edit_ops::BlockFace;
use lensbench_project::{Beam, BeamRole};
use serde_json::Value;

use super::{Combo, ElemKind};

pub(crate) mod finishes;
use crate::i18n::{tr, trf};
use crate::theme;

/// 組合せの光束（無ければ空）。読めない値は誤り
pub(crate) fn beams_of(c: &Combo) -> Result<Vec<Beam>, String> {
    match &c.extra.beams {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
    }
}

/// 光束を組合せに書く（空なら欄ごと外す）
pub(crate) fn set_beams(c: &mut Combo, v: &[Beam]) {
    c.extra.beams = (!v.is_empty()).then(|| serde_json::to_value(v).unwrap_or(Value::Null));
}

/// 光束の組の誤り（プロジェクトの検査と同じ: 名前の空・重なり・状態の名前と同じ・干渉の参照球の名前・無い状態・
/// 経路の文字・折り返す面のブロック）
pub(crate) fn problems(c: &Combo, v: &[Beam]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, b) in v.iter().enumerate() {
        let n = b.name.trim();
        if n.is_empty() {
            out.push(trf("Beam {}: the name is empty", &[&(i + 1)]));
            continue;
        }
        if ["0", "ref", "reference"].contains(&n.to_ascii_lowercase().as_str()) {
            out.push(trf(
                "Beam {}: 0, ref and reference mean the reference sphere in interference",
                &[&n],
            ));
        }
        if v[..i].iter().any(|x| x.name.trim() == n) {
            out.push(trf("Two or more beams are named {}", &[&n]));
        }
        if c.states.iter().any(|s| s == n) {
            out.push(trf("Beam {}: a state has the same name", &[&n]));
        }
        if let Some(st) = &b.state {
            if !c.states.contains(st) {
                out.push(trf("Beam {}: no state {}", &[&n, st]));
            }
        }
        if let Some(t) = &b.turn {
            if t.block.trim().is_empty() {
                out.push(trf("Beam {}: choose the lens to turn back at", &[&n]));
            }
        }
    }
    out
}

/// 折り返せる要素（玉・自作の玉）の id
fn lens_ids(c: &Combo) -> Vec<String> {
    c.ids_all()
        .into_iter()
        .filter(|(l, id)| {
            !id.is_empty()
                && c.get(l)
                    .is_some_and(|p| matches!(p.kind, ElemKind::Lens | ElemKind::Catalog))
        })
        .map(|(_, id)| id)
        .collect()
}

fn role_label(r: Option<BeamRole>) -> &'static str {
    match r {
        None => tr("(none)"),
        Some(BeamRole::Sample) => tr("Sample beam"),
        Some(BeamRole::Reference) => tr("Reference beam"),
        Some(BeamRole::Stray) => tr("Stray light"),
    }
}

/// 新しい光束の名前（beam1、beam2…で、光束・状態と重ならないもの）
fn new_name(c: &Combo, v: &[Beam]) -> String {
    (1..)
        .map(|k| format!("beam{k}"))
        .find(|n| !v.iter().any(|b| b.name == *n) && !c.states.contains(n))
        .unwrap_or_default()
}

/// 迷光の光束（BS の経路 path・折り返す面 turn）を足す（非順次の経路の表から）。同じ光束があれば足さない。
/// 足した（か同じだった）光束の名前と、足したか
pub(crate) fn add_stray(
    c: &mut Combo,
    path: Option<String>,
    turn: BlockFace,
) -> Result<(String, bool), String> {
    let mut v = beams_of(c)?;
    if let Some(b) = v
        .iter()
        .find(|b| b.path == path && b.turn.as_ref() == Some(&turn) && b.state.is_none())
    {
        return Ok((b.name.clone(), false));
    }
    let name = (1..)
        .map(|k| format!("stray{k}"))
        .find(|n| !v.iter().any(|b| b.name == *n) && !c.states.contains(n))
        .unwrap_or_default();
    v.push(Beam {
        name: name.clone(),
        path,
        state: None,
        role: Some(BeamRole::Stray),
        turn: Some(turn),
        off: false,
    });
    set_beams(c, &v);
    Ok((name, true))
}

/// 道具の列の「光束」のメニュー: 光束の一覧と、足す・消す・変える欄。変えたら true
pub(crate) fn menu(ui: &mut Ui, c: &mut Combo) -> bool {
    let mut v = match beams_of(c) {
        Ok(v) => v,
        Err(e) => {
            ui.label(RichText::new(tr("Beams")).color(theme::weak_text(ui.visuals())))
                .on_hover_text(trf(
                    "The beams of this project can't be edited here ({})",
                    &[&e],
                ));
            return false;
        }
    };
    let title = if v.is_empty() {
        tr("Beams").to_string()
    } else {
        format!("{} ({})", tr("Beams"), v.len())
    };
    let before = v.clone();
    let lenses = lens_ids(c);
    let states = c.states.clone();
    let r = ui.menu_button(title, |ui| {
        ui.set_min_width(560.0);
        ui.label(
            RichText::new(tr(
                "Named beam splitter paths. Choose a beam by name like a state (analyses, interference, overlays in the layout). Turn back makes the beam return by the partial reflection at a lens surface",
            ))
            .small()
            .color(theme::weak_text(ui.visuals())),
        );
        let mut drop = None;
        egui::Grid::new("combo_beams")
            .striped(true)
            .num_columns(7)
            .show(ui, |ui| {
                for h in [
                    tr("Name"),
                    tr("BS path"),
                    tr("State"),
                    tr("Role"),
                    tr("Turn back at"),
                    tr("Off"),
                    "",
                ] {
                    ui.label(RichText::new(h).small().strong());
                }
                ui.end_row();
                for (i, b) in v.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(80.0));
                    });
                    let mut path = b.path.clone().unwrap_or_default();
                    let r = ui.push_id(("path", i), |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut path)
                                .desired_width(48.0)
                                .hint_text(tr("Default")),
                        )
                    });
                    r.inner.on_hover_text(tr(
                        "T (transmit) and R (reflect) at each BS in the order passed. Empty uses the path of the state or the combination",
                    ));
                    let path: String = path
                        .chars()
                        .map(|c| c.to_ascii_uppercase())
                        .filter(|c| matches!(c, 'T' | 'R'))
                        .collect();
                    b.path = (!path.is_empty()).then_some(path);
                    let shown = b.state.clone().unwrap_or_else(|| tr("(none)").to_string());
                    egui::ComboBox::from_id_salt(("beam_state", i))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut b.state, None, tr("(none)"));
                            for s in &states {
                                ui.selectable_value(&mut b.state, Some(s.clone()), s.as_str());
                            }
                        }).response.accessible_name(tr("State"));
                    egui::ComboBox::from_id_salt(("beam_role", i))
                        .selected_text(role_label(b.role))
                        .show_ui(ui, |ui| {
                            for r in [
                                None,
                                Some(BeamRole::Sample),
                                Some(BeamRole::Reference),
                                Some(BeamRole::Stray),
                            ] {
                                ui.selectable_value(&mut b.role, r, role_label(r));
                            }
                        }).response.accessible_name(tr("Role"));
                    turn_cell(ui, i, b, &lenses);
                    ui.checkbox(&mut b.off, "")
                        .on_hover_text(tr("Leave the beam out of the default set of the analyses"));
                    if ui
                        .small_button("✕")
                        .on_hover_text(tr("Remove the beam"))
                        .clicked()
                    {
                        drop = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = drop {
            v.remove(i);
        }
        if ui.button(trf("+ {}", &[&tr("Add a beam")])).clicked() {
            let name = new_name(c, &v);
            v.push(Beam {
                name,
                path: None,
                state: None,
                role: None,
                turn: None,
                off: false,
            });
        }
        for p in problems(c, &v) {
            ui.colored_label(theme::error(ui.visuals()), p);
        }
    });
    let _ = r;
    if v == before {
        return false;
    }
    set_beams(c, &v);
    true
}

/// 折り返す面の欄: 玉の id・面の番号（座標ブレークを除いて 0 から）・往路の何回目か
fn turn_cell(ui: &mut Ui, i: usize, b: &mut Beam, lenses: &[String]) {
    ui.horizontal(|ui| {
        let shown = b
            .turn
            .as_ref()
            .map(|t| t.block.clone())
            .unwrap_or_else(|| tr("(none)").to_string());
        let mut block = b.turn.as_ref().map(|t| t.block.clone());
        egui::ComboBox::from_id_salt(("beam_turn", i))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut block, None, tr("(none)"));
                for id in lenses {
                    ui.selectable_value(&mut block, Some(id.clone()), id.as_str());
                }
            })
            .response
            .accessible_name(tr("Turn back at"))
            .on_hover_text(tr(
                "The lens whose surface partly reflects the beam back along its way (ghost paths, back reflections in interferometers)",
            ));
        match (block, &mut b.turn) {
            (None, t) => *t = None,
            (Some(id), Some(t)) => t.block = id,
            (Some(id), t) => {
                *t = Some(BlockFace {
                    block: id,
                    surface: 0,
                    coordbrk: false,
                    back: false,
                    pass: 0,
                })
            }
        }
        if let Some(t) = &mut b.turn {
            ui.add(
                egui::DragValue::new(&mut t.surface)
                    .range(0..=99)
                    .prefix(format!("{} ", tr("Surface"))),
            )
            .on_hover_text(tr("Surface of the lens from 0"));
            ui.add(
                egui::DragValue::new(&mut t.pass)
                    .range(0..=9)
                    .prefix(format!("{} ", tr("Pass"))),
            )
            .on_hover_text(tr("Which forward pass through the lens (from 0)"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 光束はプロジェクトの形のまま持ち、組合せのプロジェクトに書いて読み戻せる。誤りは一覧に出る
    #[test]
    fn beams_round_trip_through_the_project() {
        let mut c = Combo::default();
        assert!(beams_of(&c).unwrap().is_empty());
        let v = vec![
            Beam {
                name: "test".into(),
                path: Some("TR".into()),
                state: None,
                role: Some(BeamRole::Sample),
                turn: None,
                off: false,
            },
            Beam {
                name: "ghost".into(),
                path: None,
                state: None,
                role: Some(BeamRole::Stray),
                turn: Some(BlockFace {
                    block: "L1".into(),
                    surface: 1,
                    coordbrk: false,
                    back: false,
                    pass: 0,
                }),
                off: true,
            },
        ];
        set_beams(&mut c, &v);
        let p = c.project();
        assert_eq!(
            p["beams"][1]["turn"],
            serde_json::json!({"block": "L1", "surface": 1})
        );
        let e = crate::combo::SysExtra::read(&p).unwrap();
        assert_eq!(e.beams, c.extra.beams);
        assert!(problems(&c, &v).is_empty());
        let mut bad = v.clone();
        bad[1].name = "test".into();
        bad[0].state = Some("near".into());
        let p = problems(&c, &bad);
        assert_eq!(p.len(), 2, "{p:?}");
        set_beams(&mut c, &[]);
        assert!(c.extra.beams.is_none() && c.project().get("beams").is_none());
        assert_eq!(new_name(&c, &v), "beam1");
    }

    /// 非順次の表から足す迷光の光束: 名前は stray1…、同じ光束は二度足さない
    #[test]
    fn add_stray_appends_once() {
        let mut c = Combo::default();
        let t = BlockFace {
            block: "L1".into(),
            surface: 1,
            coordbrk: false,
            back: false,
            pass: 0,
        };
        assert_eq!(
            add_stray(&mut c, Some("TT".into()), t.clone()),
            Ok(("stray1".into(), true))
        );
        assert_eq!(
            add_stray(&mut c, Some("TT".into()), t.clone()),
            Ok(("stray1".into(), false))
        );
        assert_eq!(add_stray(&mut c, None, t), Ok(("stray2".into(), true)));
        let v = beams_of(&c).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].role, Some(BeamRole::Stray));
        assert!(problems(&c, &v).is_empty(), "{:?}", problems(&c, &v));
    }
}
