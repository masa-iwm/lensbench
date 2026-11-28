//! パネルの操作: 解析の設定・非順次の追跡・迷光の PST・系の設定・最適化・公差・ズーム

use super::*;

const P6: &str = "examples/projects/P6_states_focus_group.json";

/// 解析の設定: スポット図の「⚙ 設定」を開く → 輪の数を打つ → 新しい引数で計算し直す（設定は開いたまま）
#[test]
fn flow_analysis_settings() {
    let mut d = Drive::new("analysis settings", open(P6));
    let before = d.expect_result("spot");
    d.press_near("⚙ Settings", "Spot");
    d.type_into("Rings", 0, "2");
    let after = d.expect_result("spot");
    assert_ne!(before, after, "[analysis settings] not recomputed");
    assert!(after.to_string().contains("\"rings\":2"), "{after}");
    d.expect("Rings", "the settings closed after a change");
}

/// 非順次の追跡: 足す → 計算される → 設定で光線の格子を変える → 計算し直す
#[test]
fn flow_nsc() {
    let mut d = Drive::new("nsc", open(P6));
    d.add_analysis("Non-sequential trace");
    let before = d.expect_result("nsc");
    d.press_near("⚙ Settings", "Non-sequential trace");
    d.type_into("Pupil ray grid", 0, "9");
    let after = d.expect_result("nsc");
    assert_ne!(before, after, "[nsc] not recomputed");
    assert!(after.to_string().contains('9'), "{after}");
    d.expect("Pupil ray grid", "the settings closed after a change");
}

/// 迷光の PST: 足す → 計算される → 設定で光源の角度を変える → 計算し直す
#[test]
fn flow_pst() {
    let mut d = Drive::new("pst", open(P6));
    d.add_analysis("Stray light PST");
    let before = d.expect_result("pst");
    d.press_near("⚙ Settings", "Stray light PST");
    d.type_into("Source angles", 0, "0, 7");
    let after = d.expect_result("pst");
    assert_ne!(before, after, "[pst] not recomputed");
    assert!(after.to_string().contains('7'), "{after}");
    d.expect("Source angles", "the settings closed after a change");
}

/// 系の設定: パネルを開く → 波長を「F d C」に → 系の波長が 3 つになる → 入射瞳径を打つ → 系に入る
#[test]
fn flow_system_settings() {
    let mut d = Drive::new("system settings", open(P6));
    d.menu(&["Panels", "System settings"]);
    d.press("F d C");
    let w = &d.app().c.doc.combo.waves;
    assert_eq!(w.len(), 3, "[system settings] {w:?}");
    d.type_into("Entrance pupil diameter [mm]", 0, "4.5");
    let c = &d.app().c.doc.combo;
    assert!(
        c.ap_value == 4.5 || c.epd == 4.5,
        "[system settings] {:?} {} {}",
        c.aperture,
        c.ap_value,
        c.epd
    );
    d.expect("F d C", "the panel closed");
}

/// 最適化: パネルを開く → 既定の評価関数を「RMS spot」に → 評価する → 評価関数に項を足す
#[test]
fn flow_optimize() {
    let mut d = Drive::new("optimize", open(P6));
    d.menu(&["Panels", "Optimize"]);
    d.press("RMS spot");
    assert_eq!(
        d.app().c.p.opt.merit,
        "spot",
        "[optimize] the merit was not set"
    );
    d.press("Evaluate");
    assert!(
        d.app().c.p.opt.result.is_some(),
        "[optimize] no evaluation: {:?}",
        d.app().c.p.opt.error
    );
    let n = d.app().c.p.opt.terms.len();
    d.press("Add term");
    assert_eq!(
        d.app().c.p.opt.terms.len(),
        n + 1,
        "[optimize] the term was not added"
    );
    d.expect("Evaluate", "the panel closed");
}

/// 公差: パネルを開く → 試行の数を打つ → 公差を 1 行足す
#[test]
fn flow_tolerance() {
    let mut d = Drive::new("tolerance", open(P6));
    d.menu(&["Panels", "Tolerance"]);
    let spec = |d: &Drive| d.app().c.p.tol.spec().expect("tolerance spec");
    let n = d.app().c.p.tol.tols.len();
    d.type_into("Trials", 0, "7");
    assert!(
        spec(&d).to_string().contains("\"trials\":7"),
        "[tolerance] {}",
        spec(&d)
    );
    d.press("Add a tolerance");
    assert_eq!(
        d.app().c.p.tol.tols.len(),
        n + 1,
        "[tolerance] the row was not added"
    );
    d.expect("Run tolerancing", "the panel closed");
}

/// ズーム: パネルを開く → 目標を「倍率」に → 間の点の数を打つ
#[test]
fn flow_zoom() {
    let mut d = Drive::new("zoom", open(P6));
    d.menu(&["Panels", "Zoom track"]);
    d.press("Paraxial magnification");
    assert!(d.app().c.p.zoom.pmag, "[zoom] the target kind was not set");
    d.type_into("Points between states", 0, "3");
    assert_eq!(d.app().c.p.zoom.steps, 3, "[zoom] the points were not set");
    d.expect("Solve track", "the panel closed");
}
