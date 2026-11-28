//! 表の操作: レンズデータ・多重構成

use super::*;

const P6: &str = "examples/projects/P6_states_focus_group.json";

/// レンズデータ: 升を押して厚さを打つ → 組合せが変わり、系を組み直す（誤りなし）
#[test]
fn flow_lens_data_cell() {
    let mut d = Drive::new("lens data", open(P6));
    let before = d.app().c.doc.combo.clone();
    let gen = d.app().c.sys_gen;
    d.press_where("the thickness of surface 4", |l| l == "Surface 4 Thickness");
    d.type_cell("7.25");
    let c = &d.app().c;
    assert!(
        c.doc.combo != before,
        "[lens data] the combination did not change"
    );
    let air: Vec<f64> = c.doc.combo.picks.iter().map(|p| p.air).collect();
    assert!(air.contains(&7.25), "[lens data] {air:?}");
    assert!(c.sys_gen > gen, "[lens data] the system was not rebuilt");
    assert_eq!(
        d.app().test_state()["status_error"],
        false,
        "{}",
        d.app().test_state()
    );
}

/// 多重構成: 札を押して表を出す → 状態「far」の升を打つ → 組合せの状態の値が変わる
#[test]
fn flow_multi_config_cell() {
    let mut d = Drive::new("multi-config", open(P6));
    d.press_role("Multi-Config", egui::accesskit::Role::Tab);
    let before = d.app().c.doc.combo.clone();
    d.press_where("row 1 of far", |l| l == "Row 1 far");
    d.type_cell("0.55");
    let c = &d.app().c.doc.combo;
    assert!(
        *c != before,
        "[multi-config] the combination did not change"
    );
    assert!(
        format!("{:?} {:?}", c.groups, c.state_sys).contains("0.55"),
        "[multi-config] {:?}",
        c.groups
    );
    assert_eq!(
        d.app().test_state()["status_error"],
        false,
        "{}",
        d.app().test_state()
    );
}
