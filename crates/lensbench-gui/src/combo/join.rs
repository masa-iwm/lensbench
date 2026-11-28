//! 合流（`join`）の欄と、BS を大域の座標に置く（`at`）欄。合流は組合せでは [`ElemKind::Raw`] の要素のまま持ち
//! （[`Pick::is_join`]）、入る BS と口をここで選ぶ。BS の `at` は BS の要素の [`Pick::extra`] に持つ

use egui::{Color32, RichText, Ui};
use serde_json::{json, Value};

use super::{num, Combo, ElemKind, Pick};
use crate::combo_tree::Loc;
use crate::i18n::tr;

/// 合流の口（BS の口 A・B・C・D）
const PORTS: [&str; 4] = ["a", "b", "c", "d"];

impl Pick {
    /// 合流の要素（BS splitter の口 port へ入る）
    pub(crate) fn join(splitter: &str, port: &str) -> Pick {
        Pick::raw(&json!({ "type": "join", "splitter": splitter, "port": port }))
    }
}

/// 組合せの BS の id（木の順）
pub(crate) fn splitter_ids(combo: &Combo) -> Vec<String> {
    combo
        .ids_all()
        .into_iter()
        .filter(|(l, _)| combo.get(l).is_some_and(|p| p.kind == ElemKind::Splitter))
        .map(|(_, id)| id)
        .collect()
}

/// 合流の欄: 入る BS と口
pub(super) fn join_fields(ui: &mut Ui, combo: &mut Combo, loc: &[usize], weak: Color32) {
    let ids = splitter_ids(combo);
    let Some(p) = combo.get_mut(loc) else {
        return;
    };
    if !p.is_join() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let lbl = ui.label(RichText::new(tr("Join BS")).color(weak));
        let now = p.extra["splitter"].as_str().unwrap_or_default().to_string();
        let mut sel = now.clone();
        egui::ComboBox::from_id_salt(("join_bs", loc.to_vec()))
            .selected_text(if sel.is_empty() { "—".to_string() } else { sel.clone() })
            .show_ui(ui, |ui| {
                for id in &ids {
                    ui.selectable_value(&mut sel, id.clone(), id);
                }
            })
            .response
            .labelled_by(lbl.id)
            .on_hover_text(tr(
                "The beam splitter this arm enters (placed on another arm, or the same BS for a ring)",
            ));
        if sel != now {
            p.extra["splitter"] = json!(sel);
        }
        let lbl = ui.label(RichText::new(tr("Port")).color(weak));
        let now = p.extra["port"].as_str().unwrap_or("d").to_string();
        let mut port = now.clone();
        egui::ComboBox::from_id_salt(("join_port", loc.to_vec()))
            .selected_text(port.to_uppercase())
            .show_ui(ui, |ui| {
                for q in PORTS {
                    ui.selectable_value(&mut port, q.to_string(), q.to_uppercase());
                }
            })
            .response
            .labelled_by(lbl.id)
            .on_hover_text(tr("Port of the BS the beam enters (it must head into the port)"));
        if port != now {
            p.extra["port"] = json!(port);
        }
    });
}

/// 分岐の口の並びの最後に足せる要素の「＋」のメニュー。足したら true（合流は並びの最後に 1 つだけ）
pub(super) fn add_menu(ui: &mut Ui, picks: &mut Vec<Pick>, bs_ids: &[String]) {
    for kind in [
        ElemKind::Lens,
        ElemKind::Plate,
        ElemKind::Mirror,
        ElemKind::Stop,
        ElemKind::Aperture,
        ElemKind::Splitter,
    ] {
        if ui.button(kind.label()).clicked() {
            let mut e = Pick::new(kind);
            if kind == ElemKind::Mirror {
                // 腕の端の鏡: 光軸に垂直で往路を戻る
                e.tilt = [0.0; 3];
                e.mirror_mut().back = true;
            }
            let at = picks.len() - usize::from(picks.last().is_some_and(Pick::is_join));
            picks.insert(at, e);
            ui.close();
        }
    }
    let ends = picks.last().is_some_and(|p| p.is_join() || p.is_branched());
    if ui
        .add_enabled(
            !ends && !bs_ids.is_empty(),
            egui::Button::new(tr("Join a BS")),
        )
        .on_hover_text(tr(
            "End this arm by entering a port of a beam splitter (Mach-Zehnder, Sagnac or ring)",
        ))
        .clicked()
    {
        picks.push(Pick::join(&bs_ids[0], "d"));
        ui.close();
    }
}

/// BS の `at`（大域の座標の置き方）の JSON の値: [原点 x・y・z, 傾き x・y・z]
fn at_values(p: &Pick) -> Option<[f64; 6]> {
    let a = p.extra.get("at")?;
    let f = |v: &Value, i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    Some([
        f(&a["o"], 0),
        f(&a["o"], 1),
        f(&a["o"], 2),
        f(&a["tilt"], 0),
        f(&a["tilt"], 1),
        f(&a["tilt"], 2),
    ])
}

fn set_at(p: &mut Pick, v: [f64; 6]) {
    if !p.extra.is_object() {
        p.extra = json!({});
    }
    p.extra["at"] = json!({ "o": [v[0], v[1], v[2]], "tilt": [v[3], v[4], v[5]] });
}

/// BS を大域に置く欄。入れると、今の置き方（[`lensbench_project::splitter_frames`]、組合せを展開して求める）から
/// 始めるよう App に頼む（[`take_at_request`]）
pub(super) fn at_fields(ui: &mut Ui, combo: &mut Combo, loc: &[usize], weak: Color32) {
    let Some(p) = combo.get_mut(loc) else {
        return;
    };
    let now = at_values(p);
    let mut on = now.is_some();
    ui.horizontal_wrapped(|ui| {
        if ui
            .checkbox(&mut on, tr("Place in global coordinates"))
            .on_hover_text(tr(
                "Fix the BS in the global coordinates (port A axis), so it stays when the elements before it move. Starts from the current place",
            ))
            .changed()
        {
            if on {
                request_at(ui.ctx(), loc.to_vec());
            } else if let Some(o) = p.extra.as_object_mut() {
                o.remove("at");
                if o.is_empty() {
                    p.extra = Value::Null;
                }
            }
        }
        let Some(mut v) = now.filter(|_| on) else {
            return;
        };
        let names = [
            tr("Origin x"),
            tr("Origin y"),
            tr("Origin z"),
            tr("Tilt x"),
            tr("Tilt y"),
            tr("Tilt z"),
        ];
        for (i, n) in names.iter().enumerate() {
            num(ui, n, &mut v[i], if i < 3 { " mm" } else { "°" });
        }
        if Some(v) != now {
            set_at(p, v);
        }
        ui.label(RichText::new(tr("(port A axis)")).small().color(weak));
    });
}

fn request_id() -> egui::Id {
    egui::Id::unique("lensbench-bs-at-request")
}

fn request_at(ctx: &egui::Context, loc: Loc) {
    ctx.data_mut(|d| d.insert_temp(request_id(), Some(loc)));
}

/// 「大域の座標に置く」を入れた BS の場所（1 回だけ返す）
pub(crate) fn take_at_request(ctx: &egui::Context) -> Option<Loc> {
    ctx.data_mut(|d| d.remove_temp::<Option<Loc>>(request_id()))
        .flatten()
}

/// 場所 loc の BS を、今の置き方 frames（BS の id → 置き方）で大域に置く。BS の偏心・傾きは置き方に入っているので
/// 外す（`at` とは一緒に書けない）
pub(crate) fn place_at(
    combo: &mut Combo,
    loc: &[usize],
    frames: &std::collections::BTreeMap<String, lensbench_project::At>,
) -> Result<(), String> {
    let id = combo
        .ids_all()
        .into_iter()
        .find(|(l, _)| l == loc)
        .map(|(_, id)| id)
        .ok_or_else(|| tr("The beam splitter is no longer there").to_string())?;
    let f = frames
        .get(&id)
        .ok_or_else(|| format!("BS {id}: {}", tr("not on any path")))?;
    let p = combo
        .get_mut(loc)
        .filter(|p| p.kind == ElemKind::Splitter)
        .ok_or_else(|| tr("The beam splitter is no longer there").to_string())?;
    let r = |x: f64| (x * 1e9).round() / 1e9;
    set_at(
        p,
        [f.o[0], f.o[1], f.o[2], f.tilt[0], f.tilt[1], f.tilt[2]].map(r),
    );
    p.decenter = [0.0; 2];
    p.tilt = [0.0; 3];
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 足した合流の要素は、ファイルの合流のブロックと同じに書く（見本 P20 の反射の腕の合流を外して足し直す）
    #[test]
    fn added_join_writes_the_join_block() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("examples/projects/P20_mach_zehnder.json")).unwrap(),
        )
        .unwrap();
        let c = Combo::from_project(&v, &root).unwrap();
        assert_eq!(splitter_ids(&c), ["BS1", "BS2"]);
        let want = c.project();
        let mut d = c.clone();
        let arm = &mut d.picks[0].ports[1].picks;
        let j = arm.pop().expect("the join ends the reflected arm");
        assert!(j.is_join() && j.label() == "Join BS2 port D");
        arm.push(Pick::join("BS2", "d"));
        assert_eq!(d.project(), want);
    }
}
