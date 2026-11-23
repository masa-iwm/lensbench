//! ガラスの置き換えの「システムに反映」と元に戻す（状態の材料の上書き）

use super::*;

/// ガラスの置き換えの「システムに反映」は、多重構成の材料の行（状態の材料の上書き）も書き換える。上書きの無い状態が
/// どれも同じ硝材に置き換わったら要素の面の材料を、上書きのある状態はその上書きを。元に戻すと前の材料
#[test]
fn glass_substitution_applies_to_state_materials() {
    use lensbench_engine::session::views::{GlassChangeView, GlassView, OptimizeView};
    let _g = serial();
    let r = root();
    let tmp = std::env::temp_dir().join(format!("lb_gui_glass_states_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let f = tmp.join("zoom.zmx");
    std::fs::write(
        &f,
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.015\n  DISZ 3\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n\
MNUM 2 1\n\
THIC 3 1 20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 2 12 0 0 0 1 1 1 0 0 \"\" 0\n\
GLSS 4 1 N-BK7 0 0 0 1 1 0 0.0 \"\" 0\nGLSS 4 2 F2 0 0 0 1 1 0 0.0 \"\" 0\n\
MCOM 0 1 \"wide\" 0 0 0 1 1 0 0.0 \"\" 0\nMCOM 0 2 \"tele\" 0 0 0 1 1 0 0.0 \"\" 0\n\
LTTL 0 1 Wide 0 0 0 1 1 0 0.0 \"\" 0\nLTTL 0 2 Tele 0 0 0 1 1 0 0.0 \"\" 0\n",
    )
    .unwrap();
    let mut h = harness(Args {
        open: Some(f.clone()),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let st = h.state().test_state();
    assert_eq!(st["status_error"], false, "{st}");
    let lens = |h: &Harness<'static, App>| {
        let c = &h.state().c.doc.combo;
        let k = c
            .picks
            .iter()
            .rposition(|p| p.kind == combo::ElemKind::Lens)
            .unwrap();
        c.picks[k].clone()
    };
    assert_eq!(
        lens(&h).state_edit("Tele", 0, "material"),
        Some(&json!("F2"))
    );
    // 2 つの構成を一緒に置き換えた結果: 構成 1 は N-BK7 → N-SK16、構成 2 は F2 → N-LAK9
    let result = |configs: Option<Vec<usize>>, per: Vec<&str>| OptimizeView {
        glass: Some(GlassView {
            changes: vec![
                GlassChangeView {
                    surface: 4,
                    from: "N-BK7".into(),
                    to: "N-SK16".into(),
                    configs: configs.clone().map(|_| vec![1]),
                    ..Default::default()
                },
                GlassChangeView {
                    surface: 4,
                    from: "F2".into(),
                    to: "N-LAK9".into(),
                    configs: configs.map(|_| vec![2]),
                    ..Default::default()
                },
            ],
            materials: vec![(4, per[0].into())],
            config_materials: vec![(4, per.iter().map(|x| x.to_string()).collect())],
            ..Default::default()
        }),
        ..Default::default()
    };
    let apply = |h: &mut Harness<'static, App>, r: OptimizeView, cfgs: &[&str]| {
        let c = &mut h.state_mut().c;
        c.p.opt.result = Some(std::sync::Arc::new(r));
        c.p.opt.run_configs = cfgs.iter().map(|x| x.to_string()).collect();
        c.p.opt.gen = c.sys_gen;
        c.on_opt(crate::optim_panel::Action::Apply);
        settle(h);
    };
    apply(
        &mut h,
        result(Some(vec![]), vec!["N-SK16", "N-LAK9"]),
        &["Wide", "Tele"],
    );
    let st = h.state().test_state();
    assert_eq!(st["status_error"], false, "{st}");
    assert!(
        h.state().c.p.opt.error.is_none(),
        "{:?}",
        h.state().c.p.opt.error
    );
    let l = lens(&h);
    assert_eq!(l.state_edit("Tele", 0, "material"), Some(&json!("N-LAK9")));
    assert_eq!(l.state_edit("Wide", 0, "material"), None);
    // 開いている系（基本 = 構成 1）の面 4 は N-SK16
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-SK16");
    // 元に戻す
    h.state_mut().c.on_opt(crate::optim_panel::Action::Undo);
    settle(&mut h);
    assert_eq!(
        lens(&h).state_edit("Tele", 0, "material"),
        Some(&json!("F2"))
    );
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
    // 状態 Tele を開いて構成を並べずに置き換えた: Tele の上書きだけ替え、面の材料（Wide）はそのまま
    h.state_mut().c.doc.combo.state = Some("Tele".into());
    let p = h.state().c.doc.combo.project();
    h.state_mut().c.build_combo(p);
    settle(&mut h);
    assert_eq!(h.state().c.sys.ld_rows[4].material, "F2");
    let mut one = result(None, vec!["N-LAK9"]);
    if let Some(g) = one.glass.as_mut() {
        g.changes.truncate(1);
        g.changes[0].from = "F2".into();
        g.changes[0].to = "N-LAK9".into();
        g.config_materials.clear();
    }
    apply(&mut h, one, &[]);
    assert_eq!(
        lens(&h).state_edit("Tele", 0, "material"),
        Some(&json!("N-LAK9"))
    );
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-LAK9");
    h.state_mut().c.doc.combo.state = None;
    let p = h.state().c.doc.combo.project();
    h.state_mut().c.build_combo(p);
    settle(&mut h);
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
    std::fs::remove_dir_all(&tmp).ok();
}

/// 上書きの無い状態が 2 つあって違う硝材に置き換わったら、面の材料は替えずに両方の状態の上書きを書く。元に戻すと上書きが消える
#[test]
fn glass_substitution_splits_states_without_overrides() {
    use lensbench_engine::session::views::{GlassChangeView, GlassView, OptimizeView};
    let _g = serial();
    let r = root();
    let tmp = std::env::temp_dir().join(format!("lb_gui_glass_split_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let f = tmp.join("zoom3.zmx");
    std::fs::write(
        &f,
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.015\n  DISZ 3\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n\
MNUM 3 1\n\
THIC 3 1 20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 2 16 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 3 12 0 0 0 1 1 1 0 0 \"\" 0\n\
GLSS 4 1 N-BK7 0 0 0 1 1 0 0.0 \"\" 0\nGLSS 4 2 N-BK7 0 0 0 1 1 0 0.0 \"\" 0\nGLSS 4 3 F2 0 0 0 1 1 0 0.0 \"\" 0\n\
LTTL 0 1 Wide 0 0 0 1 1 0 0.0 \"\" 0\nLTTL 0 2 Mid 0 0 0 1 1 0 0.0 \"\" 0\nLTTL 0 3 Tele 0 0 0 1 1 0 0.0 \"\" 0\n",
    )
    .unwrap();
    let mut h = harness(Args {
        open: Some(f.clone()),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let st = h.state().test_state();
    assert_eq!(st["status_error"], false, "{st}");
    let lens = |h: &Harness<'static, App>| {
        let c = &h.state().c.doc.combo;
        let k = c
            .picks
            .iter()
            .rposition(|p| p.kind == combo::ElemKind::Lens)
            .unwrap();
        c.picks[k].clone()
    };
    assert_eq!(lens(&h).state_edit("Wide", 0, "material"), None);
    assert_eq!(lens(&h).state_edit("Mid", 0, "material"), None);
    // 構成 1 は N-BK7 → N-SK16、構成 2 は N-BK7 → N-LAK9、構成 3 はそのまま
    let change = |to: &str, cfg: usize| GlassChangeView {
        surface: 4,
        from: "N-BK7".into(),
        to: to.into(),
        configs: Some(vec![cfg]),
        ..Default::default()
    };
    let r = OptimizeView {
        glass: Some(GlassView {
            changes: vec![change("N-SK16", 1), change("N-LAK9", 2)],
            materials: vec![(4, "N-SK16".into())],
            config_materials: vec![(4, vec!["N-SK16".into(), "N-LAK9".into(), "F2".into()])],
            ..Default::default()
        }),
        ..Default::default()
    };
    {
        let c = &mut h.state_mut().c;
        c.p.opt.result = Some(std::sync::Arc::new(r));
        c.p.opt.run_configs = ["Wide", "Mid", "Tele"].map(String::from).to_vec();
        c.p.opt.gen = c.sys_gen;
        c.on_opt(crate::optim_panel::Action::Apply);
    }
    settle(&mut h);
    assert!(
        h.state().c.p.opt.error.is_none(),
        "{:?}",
        h.state().c.p.opt.error
    );
    let l = lens(&h);
    assert_eq!(l.state_edit("Wide", 0, "material"), Some(&json!("N-SK16")));
    assert_eq!(l.state_edit("Mid", 0, "material"), Some(&json!("N-LAK9")));
    assert_eq!(l.state_edit("Tele", 0, "material"), Some(&json!("F2")));
    // 面の材料（状態を開かない系）は N-BK7 のまま
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
    h.state_mut().c.on_opt(crate::optim_panel::Action::Undo);
    settle(&mut h);
    let l = lens(&h);
    assert_eq!(l.state_edit("Wide", 0, "material"), None);
    assert_eq!(l.state_edit("Mid", 0, "material"), None);
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
    std::fs::remove_dir_all(&tmp).ok();
}
