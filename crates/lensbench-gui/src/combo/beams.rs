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
        if b.turn.iter().any(|t| t.block.trim().is_empty()) {
            out.push(trf("Beam {}: choose the lens to turn back at", &[&n]));
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

/// 迷光の光束（BS の経路 path・折り返す面の並び turn）を足す（非順次の経路の表から）。同じ光束があれば足さない。
/// 足した（か同じだった）光束の名前と、足したか
pub(crate) fn add_stray(
    c: &mut Combo,
    path: Option<String>,
    turn: Vec<BlockFace>,
) -> Result<(String, bool), String> {
    let mut v = beams_of(c)?;
    if let Some(b) = v
        .iter()
        .find(|b| b.path == path && b.turn == turn && b.state.is_none())
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
        turn,
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
    tool_window(ui, &title, "combo-beams", 560.0, |ui| {
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
                        .small_button("✖")
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
                turn: Vec::new(),
                off: false,
            });
        }
        for p in problems(c, &v) {
            ui.colored_label(theme::error(ui.visuals()), p);
        }
    });
    if v == before {
        return false;
    }
    set_beams(c, &v);
    true
}

/// tour の見本: 試験の腕（TR）と参照の腕（RR）の光束
pub(crate) fn tour_sample(c: &mut Combo) {
    let beam = |name: &str, path: &str, role| Beam {
        name: name.into(),
        path: Some(path.into()),
        state: None,
        role: Some(role),
        turn: Vec::new(),
        off: false,
    };
    set_beams(
        c,
        &[
            beam("test_arm", "TR", BeamRole::Sample),
            beam("ref_arm", "RR", BeamRole::Reference),
        ],
    );
}

/// 道具の窓（[`tool_window`]）の開いているかを覚える id（レイアウトのパネルは 1 枚だけなので、パネルによらない）
fn tool_window_id(key: &str) -> egui::Id {
    egui::Id::unique(("combo-tool-window", key))
}

/// 道具の窓（"combo-beams"・"combo-finishes"）を開く・閉じる（tour が窓を撮るとき）
pub(crate) fn set_tool_window(ctx: &egui::Context, key: &str, open: bool) {
    ctx.data_mut(|d| d.insert_temp(tool_window_id(key), open));
}

/// 表を開く窓: ボタンで開く・閉じる。中のクリック（入れ子の一覧を含む）では閉じない。メニューは中のクリックで
/// 閉じるので、欄を直す表には使わない
pub(super) fn tool_window(
    ui: &mut Ui,
    title: &str,
    key: &str,
    width: f32,
    add: impl FnOnce(&mut Ui),
) {
    let id = tool_window_id(key);
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let b = ui.selectable_label(open, title);
    if b.clicked() {
        open = !open;
    }
    // ボタンのすぐ下に出す（左上だとメニューの列にかぶる）。動かすまではボタンについて行く: 光束を足すと
    // レイアウトの上に光束を選ぶ行が出て道具の列が下がり、置いたままの窓がボタンにかぶって閉じられなくなる
    let below = b.rect.left_bottom() + egui::vec2(0.0, 4.0);
    let placed_id = id.with("placed");
    let placed = open && ui.data(|d| d.get_temp::<bool>(placed_id)).unwrap_or(false);
    let win_id = id.with("win");
    // egui 0.37 の窓は題の帯でつかむ形のとき current_pos を前の位置で上書きするので、覚えている位置を直に替える
    if !placed {
        if let Some(mut st) = egui::AreaState::load(ui.ctx(), win_id) {
            if st.pivot_pos != Some(below) {
                st.pivot_pos = Some(below);
                ui.ctx().memory_mut(|m| {
                    m.areas_mut()
                        .set_state(egui::LayerId::new(egui::Order::Middle, win_id), st)
                });
            }
        }
    }
    let w = egui::Window::new(title)
        .id(win_id)
        .default_pos(below)
        .open(&mut open)
        .default_width(width)
        .resizable(true)
        .collapsible(false);
    // 持って動かした・左か上の縁で大きさを変えたら（ドラッグの間に窓の角が動いたら）、その後は置いた所に
    let dragging = ui.input(|i| i.pointer.is_decidedly_dragging());
    let moved = w
        .show(ui.ctx(), add)
        .is_some_and(|r| dragging && r.response.rect.min.distance(below) > 2.0);
    ui.data_mut(|d| d.insert_temp(placed_id, placed || moved));
    ui.data_mut(|d| d.insert_temp(id, open));
}

/// 折り返す面の欄: 折り返しごとに玉の id・面の番号（座標ブレークを除いて往路の番号で 0 から）・前の折り返しから
/// 何回目か。向きは折り返すごとに往・復・往…と入れ替わる。「＋」で次の折り返しを足す
fn turn_cell(ui: &mut Ui, i: usize, b: &mut Beam, lenses: &[String]) {
    let face = |block: String| BlockFace {
        block,
        surface: 0,
        coordbrk: false,
        back: false,
        pass: 0,
    };
    ui.vertical(|ui| {
        let mut drop = None;
        let n = b.turn.len();
        for k in 0..n.max(1) {
            ui.horizontal(|ui| {
                let now = b.turn.get(k).map(|t| t.block.clone());
                let mut block = now.clone();
                egui::ComboBox::from_id_salt(("beam_turn", i, k))
                    .selected_text(now.clone().unwrap_or_else(|| tr("(none)").to_string()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut block, None, tr("(none)"));
                        for id in lenses {
                            ui.selectable_value(&mut block, Some(id.clone()), id.as_str());
                        }
                    })
                    .response
                    .accessible_name(if k == 0 {
                        tr("Turn back at").to_string()
                    } else {
                        trf("Turn {} at", &[&(k + 1).to_string()])
                    })
                    .on_hover_text(tr(
                        "The lens whose surface partly reflects the beam back along its way (ghost paths, back reflections in interferometers)",
                    ));
                if block != now {
                    match (block, b.turn.get_mut(k)) {
                        (None, _) => drop = Some(k),
                        (Some(id), Some(t)) => t.block = id,
                        (Some(id), None) => b.turn.push(face(id)),
                    }
                }
                if let Some(t) = b.turn.get_mut(k) {
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
                    .on_hover_text(tr(
                        "Which pass through the lens since the last turn (from 0)",
                    ));
                }
            });
        }
        if let Some(k) = drop {
            b.turn.remove(k);
        }
        if let Some(last) = b.turn.last().cloned() {
            if ui
                .small_button("+")
                .on_hover_text(tr(
                    "Turn back again at another surface (the direction alternates: out, back, out…)",
                ))
                .clicked()
            {
                b.turn.push(face(last.block));
            }
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
                turn: Vec::new(),
                off: false,
            },
            Beam {
                name: "ghost".into(),
                path: None,
                state: None,
                role: Some(BeamRole::Stray),
                turn: vec![BlockFace {
                    block: "L1".into(),
                    surface: 1,
                    coordbrk: false,
                    back: false,
                    pass: 0,
                }],
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
            add_stray(&mut c, Some("TT".into()), vec![t.clone()]),
            Ok(("stray1".into(), true))
        );
        assert_eq!(
            add_stray(&mut c, Some("TT".into()), vec![t.clone()]),
            Ok(("stray1".into(), false))
        );
        assert_eq!(
            add_stray(&mut c, None, vec![t.clone()]),
            Ok(("stray2".into(), true))
        );
        // 2 回の折り返しは別の光束で、プロジェクトには並びで書く
        let two = vec![
            t.clone(),
            BlockFace {
                surface: 0,
                ..t.clone()
            },
        ];
        assert_eq!(
            add_stray(&mut c, None, two.clone()),
            Ok(("stray3".into(), true))
        );
        assert_eq!(add_stray(&mut c, None, two), Ok(("stray3".into(), false)));
        assert_eq!(
            c.project()["beams"][2]["turn"],
            serde_json::json!([{"block": "L1", "surface": 1}, {"block": "L1", "surface": 0}])
        );
        let v = beams_of(&c).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].role, Some(BeamRole::Stray));
        assert!(problems(&c, &v).is_empty(), "{:?}", problems(&c, &v));
    }
}
