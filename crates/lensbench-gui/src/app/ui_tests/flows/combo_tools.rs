//! 組合せの道具の列（レイアウトのパネルの上の帯）の窓: 光束・仕上げ

use super::*;
use crate::combo::beams::{self, finishes};

/// 光束: 開く → 足す → 役割を選ぶ → 窓は開いたまま、もう 1 つ足せる → 閉じる。組合せの光束に入る
#[test]
fn flow_combo_beams() {
    let mut d = Drive::new("beams", open("examples/projects/P5_triplet_blocks.json"));
    let beams = |d: &Drive| beams::beams_of(&d.app().c.doc.combo).expect("beams");
    assert!(beams(&d).is_empty());
    d.press("Beams");
    d.press("+ Add a beam");
    assert_eq!(beams(&d).len(), 1, "[beams] the row was not added");
    d.choose("Role", 0, "Reference beam");
    d.expect("+ Add a beam", "the window closed after choosing in a list");
    assert_eq!(
        beams(&d)[0].role,
        Some(lensbench_project::BeamRole::Reference),
        "[beams] the role was not set: {:?}",
        beams(&d)
    );
    d.press("+ Add a beam");
    assert_eq!(beams(&d).len(), 2, "[beams] the second row was not added");
    // 行を足すと道具の列が下がる。窓はボタンにかぶらず、ボタンで閉じられる（先に見る: 動かした窓では確かめにならない）
    let button = d.find("Beams (2)", Some(egui::accesskit::Role::Button));
    let w = d.find("Beams (2)", Some(egui::accesskit::Role::Window));
    assert!(
        !w.intersects(button),
        "[beams] the window covers its button: {w:?} {button:?}"
    );
    d.press_role("Beams (2)", egui::accesskit::Role::Button);
    assert!(!d.shown("+ Add a beam"), "[beams] the window did not close");
    // 開き直して、題の帯を持って下へ動かした窓は、ボタンについて戻らない
    d.press_role("Beams (2)", egui::accesskit::Role::Button);
    d.expect("+ Add a beam", "the window did not open again");
    let w = d.find("Beams (2)", Some(egui::accesskit::Role::Window));
    let grip = egui::pos2(w.center().x, w.top() + 10.0);
    d.drag(grip, grip + egui::vec2(0.0, 150.0));
    let moved = d.find("Beams (2)", Some(egui::accesskit::Role::Window));
    assert!(
        (moved.top() - w.top() - 150.0).abs() < 3.0,
        "[beams] the window did not stay where it was dragged: {w:?} -> {moved:?}"
    );
    d.press("+ Add a beam");
    let again = d.find("Beams (3)", Some(egui::accesskit::Role::Window));
    assert!(
        (again.top() - moved.top()).abs() < 3.0,
        "[beams] the dragged window went back: {moved:?} -> {again:?}"
    );
    d.press_role("Beams (3)", egui::accesskit::Role::Button);
    assert!(!d.shown("+ Add a beam"), "[beams] the window did not close");
    assert_eq!(d.app().test_state()["status_error"], false);
}

/// 仕上げ: 開く → 足す → 対象を「縁」に → 窓は開いたまま、もう 1 つ足す → 1 行目を消す → 閉じる
#[test]
fn flow_combo_finishes() {
    let mut d = Drive::new("finishes", open("examples/projects/P5_triplet_blocks.json"));
    let fin = |d: &Drive| finishes::finishes_of(&d.app().c.doc.combo).expect("finishes");
    assert!(fin(&d).is_empty());
    d.press("Finishes");
    d.press("+ Add a finish");
    assert_eq!(fin(&d).len(), 1, "[finishes] the row was not added");
    d.choose("Applies to", 0, "Lens edge");
    d.expect(
        "+ Add a finish",
        "the window closed after choosing in a list",
    );
    assert!(fin(&d)[0].edge.is_some(), "[finishes] {:?}", fin(&d));
    d.press("+ Add a finish");
    assert_eq!(fin(&d).len(), 2, "[finishes] the second row was not added");
    // 1 行目の ✖ で消すと、縁の行が消えて面の行が残る
    let x = d.nth_rect("✖", 0);
    d.click_at(x.center());
    assert_eq!(fin(&d).len(), 1, "[finishes] the row was not removed");
    assert!(fin(&d)[0].face.is_some(), "[finishes] {:?}", fin(&d));
    d.press_role("Finishes (1)", egui::accesskit::Role::Button);
    assert!(
        !d.shown("+ Add a finish"),
        "[finishes] the window did not close"
    );
}

/// 状態: 名前を打って Enter で足す → その状態が選ばれる → 一覧で「Base」に戻す
#[test]
fn flow_combo_states() {
    let mut d = Drive::new("states", open("examples/projects/P5_triplet_blocks.json"));
    d.type_into("State name", 0, "wide");
    let c = &d.app().c.doc.combo;
    assert!(
        c.states.iter().any(|s| s == "wide"),
        "[states] {:?}",
        c.states
    );
    assert_eq!(c.state.as_deref(), Some("wide"), "[states] not selected");
    d.choose("State", 0, "Base");
    assert_eq!(
        d.app().c.doc.combo.state,
        None,
        "[states] Base was not chosen"
    );
}

/// 機構部品（カタログのパネル）: 品番を押して選ぶ → 軸の位置を打つ → 部品が動き、系を組み直す
#[test]
fn flow_mech_parts() {
    let mut d = Drive::new(
        "mechanical parts",
        open("examples/projects/P19_tube_cage.json"),
    );
    d.press_role("Catalog", egui::accesskit::Role::Tab);
    let name = d.app().c.doc.combo.mechanics[0].name.clone();
    let item = d.app().c.doc.combo.mechanics[0].item.clone();
    d.press(&item);
    assert_eq!(
        crate::combo::mech::selected(&d.h.ctx).as_deref(),
        Some(name.as_str()),
        "[mechanical parts] the part was not selected"
    );
    let gen = d.app().c.sys_gen;
    d.type_into("z", 0, "1.5");
    let m = &d.app().c.doc.combo.mechanics[0];
    assert_eq!(m.offset[2], 1.5, "[mechanical parts] {:?}", m.offset);
    assert!(
        d.app().c.sys_gen > gen,
        "[mechanical parts] the system was not rebuilt"
    );
}
