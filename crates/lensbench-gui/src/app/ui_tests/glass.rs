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

/// 上書きの無い状態が 2 つあって違う硝材に置き換わったら、面の材料（基本の系）は基本の構成の状態 Wide の新しい硝材に、
/// Mid はその上書きに。「状態だけに書く」なら面の材料は替えずに両方の状態の上書きを書く。元に戻すと上書きが消える
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
    let apply = |h: &mut Harness<'static, App>, only: bool| {
        let c = &mut h.state_mut().c;
        c.p.opt.result = Some(std::sync::Arc::new(r.clone()));
        c.p.opt.run_configs = ["Wide", "Mid", "Tele"].map(String::from).to_vec();
        c.p.opt.glass_states_only = only;
        c.p.opt.gen = c.sys_gen;
        c.on_opt(crate::optim_panel::Action::Apply);
        settle(h);
        assert!(
            h.state().c.p.opt.error.is_none(),
            "{:?}",
            h.state().c.p.opt.error
        );
    };
    apply(&mut h, false);
    let l = lens(&h);
    assert_eq!(l.state_edit("Wide", 0, "material"), None);
    assert_eq!(l.state_edit("Mid", 0, "material"), Some(&json!("N-LAK9")));
    assert_eq!(l.state_edit("Tele", 0, "material"), Some(&json!("F2")));
    // 面の材料（状態を開かない系）は Wide の新しい硝材
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-SK16");
    h.state_mut().c.on_opt(crate::optim_panel::Action::Undo);
    settle(&mut h);
    let l = lens(&h);
    assert_eq!(l.state_edit("Mid", 0, "material"), None);
    assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
    // 状態だけに書く
    apply(&mut h, true);
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

/// 要素に分けられない多重構成の .zmx（構成ごとの IGNR で取り込みの中の表になる）: 2 つの構成を一緒に置き換えたら、
/// 基本の系（構成 1）の面も構成 1 の新しい硝材に、構成 2 の状態はその上書きに。構成 3 を置き換えなかったら、
/// 構成 3 の前の硝材が分からないので状態の上書きだけにして知らせる
#[test]
fn glass_substitution_writes_base_of_unsplit_multiconfig() {
    use lensbench_engine::session::views::{GlassChangeView, GlassView, OptimizeView};
    let _g = serial();
    let r = root();
    let tmp = std::env::temp_dir().join(format!("lb_gui_glass_ignr_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let zmx = |n: usize| {
        let mut s = String::from(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.015\n  DISZ 3\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        );
        let names = ["Wide", "Tele", "Far"];
        let glass = ["N-BK7", "F2", "F2"];
        s += &format!("MNUM {n} 1\n");
        for i in 0..n {
            let c = i + 1;
            s += &format!("THIC 3 {c} {} 0 0 0 1 1 1 0 0 \"\" 0\n", 20 - 4 * i);
            s += &format!("GLSS 4 {c} {} 0 0 0 1 1 0 0.0 \"\" 0\n", glass[i]);
            s += &format!("IGNR 2 {c} {} 0 0 0 1 1 0 0.0 \"\" 0\n", usize::from(i > 0));
            s += &format!("LTTL 0 {c} {} 0 0 0 1 1 0 0.0 \"\" 0\n", names[i]);
        }
        s
    };
    let change = |from: &str, to: &str, cfg: usize| GlassChangeView {
        surface: 4,
        from: from.into(),
        to: to.into(),
        configs: Some(vec![cfg]),
        ..Default::default()
    };
    let lens_states = |h: &Harness<'static, App>| {
        let c = &h.state().c.doc.combo;
        let p = c
            .picks
            .iter()
            .find(|p| !p.state_edits.is_empty() || !p.edits.is_empty());
        p.map(|p| (p.state_edits.clone(), p.edits.clone()))
    };
    for (n, cfgs) in [(2, vec!["Wide", "Tele"]), (3, vec!["Wide", "Tele"])] {
        let f = tmp.join(format!("ignr{n}.zmx"));
        std::fs::write(&f, zmx(n)).unwrap();
        let mut h = harness(Args {
            open: Some(f.clone()),
            glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
            ..Args::default()
        });
        let st = h.state().test_state();
        assert_eq!(st["status_error"], false, "{st}");
        let kinds: Vec<_> = h.state().c.doc.combo.picks.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&combo::ElemKind::ZmxRange), "{kinds:?}");
        let res = OptimizeView {
            glass: Some(GlassView {
                changes: vec![change("N-BK7", "N-SK16", 1), change("F2", "N-LAK9", 2)],
                materials: vec![(4, "N-SK16".into())],
                config_materials: vec![(4, vec!["N-SK16".into(), "N-LAK9".into()])],
                ..Default::default()
            }),
            ..Default::default()
        };
        {
            let c = &mut h.state_mut().c;
            c.p.opt.result = Some(std::sync::Arc::new(res));
            c.p.opt.run_configs = cfgs.iter().map(|x| x.to_string()).collect();
            c.p.opt.gen = c.sys_gen;
            c.on_opt(crate::optim_panel::Action::Apply);
        }
        settle(&mut h);
        assert!(
            h.state().c.p.opt.error.is_none(),
            "{:?}",
            h.state().c.p.opt.error
        );
        let (states, base) = lens_states(&h).expect("written");
        let mat = |n: &str| {
            states
                .get(n)
                .and_then(|v| v.iter().find(|e| e.field == "material"))
                .map(|e| e.value.clone())
        };
        assert_eq!(mat("Tele"), Some(json!("N-LAK9")), "{states:?}");
        if n == 2 {
            // 基本の系（構成 1）も N-SK16、Wide は面の材料に従う
            assert_eq!(mat("Wide"), None, "{states:?}");
            assert!(base.iter().any(|e| e.field == "material"), "{base:?}");
            assert_eq!(h.state().c.sys.ld_rows[4].material, "N-SK16");
        } else {
            // 構成 3（Far）の前の硝材が分からないので、基本の系は替えず状態の上書きだけ
            assert_eq!(mat("Wide"), Some(json!("N-SK16")), "{states:?}");
            assert_eq!(mat("Far"), None, "{states:?}");
            assert!(!base.iter().any(|e| e.field == "material"), "{base:?}");
            assert_eq!(h.state().c.sys.ld_rows[4].material, "N-BK7");
            let log = &h.state().c.status_log;
            assert!(
                log.iter().any(|x| x.0.contains("only to the states")),
                "{log:?}"
            );
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
}
