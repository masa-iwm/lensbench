//! 計算のパネル（最適化・公差・差し替え・設定・ズーム）の ui を egui_kittest で回し、AccessKit の木（ボタン・欄の
//! 文言）で探して押す。パネルが返す操作（Action）と、押した後のパネルの状態を確かめる（serve は呼ばない）

use std::rc::Rc;

use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use serde_json::Value;

use crate::i18n;
use crate::optim_panel as op;
use crate::param_pick::Sys;
use crate::tol_panel as tp;

/// 言語はプロセスで 1 つなので、App の画面の試験と重ねない
fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = crate::serve_lock();
    i18n::set(i18n::Lang::En);
    g
}

fn sys() -> Rc<Sys> {
    Rc::new(Sys {
        surfs: vec![
            ("STANDARD".into(), String::new(), false),
            ("STANDARD".into(), String::new(), false),
            ("STANDARD".into(), String::new(), false),
            ("STANDARD".into(), String::new(), false),
        ],
        units: vec!["L1".into()],
        waves: 1,
    })
}

/// パネルと、これまでに返した操作
struct St<P, A> {
    p: P,
    acts: Vec<A>,
}

fn optim(p: op::OptPanel) -> Harness<'static, St<op::OptPanel, op::Action>> {
    let s = sys();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 1400.0))
        .build_ui_state(
            move |ui, st: &mut St<op::OptPanel, op::Action>| {
                let c = op::Cands {
                    units: vec!["L1".into()],
                    combo: false,
                    sys: s.clone(),
                    rows: &[],
                };
                let a = op::ui(ui, &mut st.p, &c, None);
                st.acts.extend(a);
            },
            St {
                p,
                acts: Vec::new(),
            },
        );
    // 走っている間は spinner が描き直しを頼み続けるので、決まった数だけ回す
    h.run_steps(4);
    h
}

fn tol(p: tp::TolPanel) -> Harness<'static, St<tp::TolPanel, tp::Action>> {
    let s = sys();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 2000.0))
        .build_ui_state(
            move |ui, st: &mut St<tp::TolPanel, tp::Action>| {
                let c = op::Cands {
                    units: vec!["L1".into()],
                    combo: false,
                    sys: s.clone(),
                    rows: &[],
                };
                let a = tp::ui(ui, &mut st.p, &c, None);
                st.acts.extend(a);
            },
            St {
                p,
                acts: Vec::new(),
            },
        );
    // 走っている間は spinner が描き直しを頼み続けるので、決まった数だけ回す
    h.run_steps(4);
    h
}

fn is_disabled<S>(h: &Harness<'_, S>, label: &str) -> bool {
    h.get_by_label(label).is_disabled()
}

/// 変数が無い間は「Optimize」を押せず、理由が出る
#[test]
fn optim_needs_a_variable() {
    let _g = serial();
    let h = optim(op::OptPanel::new());
    assert!(is_disabled(&h, "Optimize"));
    h.get_by_label("Add at least one variable to enable");
}

/// 名前の欄に打って「Add」で変数が増え、「Optimize」で設定の JSON が返る
#[test]
fn optim_add_variable_and_run() {
    let _g = serial();
    let mut h = optim(op::OptPanel::new());
    let edit = h.get(
        by().role(egui::accesskit::Role::TextInput)
            .label("7.t, lens2.tx, …"),
    );
    edit.focus();
    edit.type_text("3.t");
    h.run_steps(4);
    h.get_by_label("Add").click();
    h.run_steps(4);
    assert_eq!(h.state().p.vars.len(), 1);
    assert_eq!(h.state().p.vars[0].name, "3.t");
    assert!(!is_disabled(&h, "Optimize"));
    h.get_by_label("Optimize").click();
    h.run_steps(4);
    let run: Vec<&Value> = h
        .state()
        .acts
        .iter()
        .filter_map(|a| match a {
            op::Action::Run(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(run.len(), 1, "Optimize は 1 回だけ Run を返す");
    assert_eq!(run[0]["vars"][0]["name"], "3.t");
}

/// 走っている間は「Stop」だけが出て、押すと Stop が返る
#[test]
fn optim_stop_while_running() {
    let _g = serial();
    let mut p = op::OptPanel::new();
    p.running = true;
    let mut h = optim(p);
    assert!(h.query_by_label("Optimize").is_none());
    h.get_by_label("Stop").click();
    h.run_steps(4);
    assert!(matches!(h.state().acts.as_slice(), [op::Action::Stop]));
}

/// 既定の公差は欄の誤りが無く、「Run tolerancing」で試行の数などの入った設定が返る
#[test]
fn tol_run_returns_spec() {
    let _g = serial();
    let mut h = tol(tp::TolPanel::default());
    assert!(!is_disabled(&h, "Run tolerancing"));
    h.get_by_label("Run tolerancing").click();
    h.run_steps(4);
    match h.state().acts.as_slice() {
        [tp::Action::Run(v)] => assert_eq!(v["trials"], 100),
        _ => panic!("Run が 1 つ返るはず"),
    }
}

/// 欄に誤りがあると「Run tolerancing」を押せず、理由が出る
#[test]
fn tol_invalid_field_disables_run() {
    let _g = serial();
    let p = tp::TolPanel {
        percentiles: "abc".into(),
        ..Default::default()
    };
    let err = p.spec().expect_err("百分位の誤り");
    let h = tol(p);
    assert!(is_disabled(&h, "Run tolerancing"));
    h.get_by_label(&format!("⚠ {err}"));
}

/// 「Save settings…」は今の設定を返し、走っている間は押せない
#[test]
fn tol_save_and_running() {
    let _g = serial();
    let mut h = tol(tp::TolPanel::default());
    h.get_by_label("Save settings…").click();
    h.run_steps(4);
    assert!(matches!(h.state().acts.as_slice(), [tp::Action::Save(_)]));
    let p = tp::TolPanel {
        running: true,
        ..Default::default()
    };
    let mut h = tol(p);
    assert!(is_disabled(&h, "Save settings…"));
    h.get_by_label("Stop").click();
    h.run_steps(4);
    assert!(matches!(h.state().acts.as_slice(), [tp::Action::Stop]));
}

// ---------------------------------------------------------------- 差し替え

use crate::swap_panel as sp;

fn swap(p: sp::SwapPanel, blocks: &[&str]) -> Harness<'static, St<sp::SwapPanel, sp::Action>> {
    swap_in(p, blocks, Vec::new(), Vec::new(), Vec::new())
}

/// 間隔 (id, 説明)・状態・変数のプロジェクトの値のある系の差し替え探索のパネル
fn swap_in(
    p: sp::SwapPanel,
    blocks: &[&str],
    gaps: Vec<(String, String)>,
    states: Vec<String>,
    base: Vec<(String, f64)>,
) -> Harness<'static, St<sp::SwapPanel, sp::Action>> {
    let s = sys();
    let blocks: Vec<(String, String)> = blocks
        .iter()
        .map(|b| (b.to_string(), format!("{b} lens")))
        .collect();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 2000.0))
        .build_ui_state(
            move |ui, st: &mut St<sp::SwapPanel, sp::Action>| {
                let c = sp::Cands {
                    blocks: blocks.clone(),
                    gaps: gaps.clone(),
                    states: states.clone(),
                    sys: s.clone(),
                    base: base.clone(),
                };
                let a = sp::ui(ui, &mut st.p, &c, None);
                st.acts.extend(a);
            },
            St {
                p,
                acts: Vec::new(),
            },
        );
    h.run_steps(4);
    h
}

/// 玉が無い間は「Run search」「Count sets」を押せず、「Add lens」で玉が入ると押せる。それぞれ設定を返す
#[test]
fn swap_add_lens_then_run_and_count() {
    let _g = serial();
    let mut h = swap(sp::SwapPanel::default(), &["L1", "L2"]);
    assert!(is_disabled(&h, "Run search"));
    assert!(is_disabled(&h, "Count sets"));
    h.get_by_label("Add lens").click();
    h.run_steps(4);
    assert_eq!(h.state().p.slots.len(), 1);
    assert_eq!(h.state().p.slots[0].block, "L1");
    assert!(!is_disabled(&h, "Run search"));
    h.get_by_label("Count sets").click();
    h.run_steps(4);
    h.get_by_label("Run search").click();
    h.run_steps(4);
    match h.state().acts.as_slice() {
        [sp::Action::Count(c), sp::Action::Run(r)] => {
            assert_eq!(c["slots"][0]["block"], "L1");
            assert_eq!(r["slots"][0]["block"], "L1");
        }
        _ => panic!("Count と Run が順に返るはず"),
    }
}

/// 玉を全部入れたら「Add lens」は押せない（同じ玉を 2 度入れない）
#[test]
fn swap_add_lens_until_full() {
    let _g = serial();
    let mut h = swap(sp::SwapPanel::default(), &["L1", "L2"]);
    for _ in 0..2 {
        h.get_by_label("Add lens").click();
        h.run_steps(4);
    }
    let names: Vec<&str> = h.state().p.slots.iter().map(|s| s.block.as_str()).collect();
    assert_eq!(names, ["L1", "L2"]);
    assert!(is_disabled(&h, "Add lens"));
}

/// 出発点の表: 「Add project values」で変数のプロジェクトの値の行を足す。状態ごとの変数は補完の欄から足し、
/// 表の列に加わる。変数でない名前の列は表の下に出し、ボタンで消せる
#[test]
fn swap_starts_table_and_state_vars() {
    let _g = serial();
    let mut p = sp::SwapPanel::default();
    p.vars.push(sp::VarRow {
        name: "d1.t".into(),
        lo: "0".into(),
        hi: String::new(),
    });
    p.states = vec!["wide".into()];
    p.new_state_var = "L1.shift".into();
    p.starts
        .push([("zz.t".to_string(), 1.0)].into_iter().collect());
    let mut h = swap_in(
        p,
        &["L1"],
        vec![("d1".into(), "after L1".into())],
        vec!["wide".into(), "tele".into()],
        vec![("d1.t".into(), 5.0)],
    );
    let warn = "Starting points have values for names that are not variables: zz.t";
    assert!(h.query_by_label_contains(warn).is_some());
    h.get_by_label("Clear the columns that are not variables")
        .click();
    h.run_steps(4);
    assert!(h.query_by_label_contains(warn).is_none());
    assert!(h.state().p.starts[0].is_empty());
    h.get_by_label("Add project values").click();
    h.run_steps(4);
    assert_eq!(h.state().p.starts[1].get("d1.t"), Some(&5.0));
    h.get_by_label("Add per-state variable").click();
    h.run_steps(4);
    assert_eq!(h.state().p.state_vars[0].name, "L1.shift");
    assert!(h.state().p.new_state_var.is_empty());
    // 下限は付けない（群の移動は負にもなる）
    assert_eq!(h.state().p.state_vars[0].lo, "");
    let s = h.state().p.spec().unwrap();
    assert_eq!(s["state_vars"], serde_json::json!(["L1.shift"]));
    assert_eq!(s["starts"], serde_json::json!([{}, {"d1.t": 5.0}]));
}
