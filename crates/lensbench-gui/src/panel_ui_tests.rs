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
                    gaps: Vec::new(),
                    combo: true,
                    states: Vec::new(),
                    sys: s.clone(),
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

// ---------------------------------------------------------------- 多重構成の表

use crate::config_panel as cp;

fn config(table: Option<cp::Table>) -> Harness<'static, St<cp::State, cp::Act>> {
    let s = sys();
    let mut st = cp::State::default();
    st.table = table;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            move |ui, st: &mut St<cp::State, cp::Act>| {
                let a = cp::ui(ui, &mut st.p, false, &s);
                st.acts.extend(a);
            },
            St {
                p: st,
                acts: Vec::new(),
            },
        );
    h.run_steps(4);
    h
}

fn table2() -> cp::Table {
    cp::Table {
        n: 2,
        active: 1,
        rows: vec![("3.t".into(), vec![10.0, 12.0])],
        picks: vec![vec![]],
        zmx_cfgs: 0,
        zmx_cfg: 0,
    }
}

fn ops(acts: &[cp::Act]) -> Vec<Value> {
    acts.iter()
        .flat_map(|a| match a {
            cp::Act::Ops(v) => v.clone(),
            _ => Vec::new(),
        })
        .collect()
}

/// 系が無ければ案内だけ
#[test]
fn config_without_system() {
    let _g = serial();
    let h = config(None);
    h.get_by_label("Open a system to see its multi-configuration table.");
}

/// 構成の番号を押すとその構成を当てる操作が返る
#[test]
fn config_switch_configuration() {
    let _g = serial();
    let mut h = config(Some(table2()));
    h.get_by_label("2").click();
    h.run_steps(4);
    assert_eq!(
        ops(&h.state().acts),
        [serde_json::json!({ "op": "set_config", "k": 2 })]
    );
}

/// 名前の欄に打って「Insert row」で行を足す操作が返る。行を選ぶまで「Delete row」は押せない
#[test]
fn config_insert_row() {
    let _g = serial();
    let mut h = config(Some(table2()));
    assert!(is_disabled(&h, "Delete row"));
    let edit = h.get(
        by().role(egui::accesskit::Role::TextInput)
            .label("3.t, 2.c, STOP.k, G1.shift"),
    );
    edit.focus();
    edit.type_text("4.t");
    h.run_steps(4);
    h.get_by_label("Insert row").click();
    h.run_steps(4);
    assert_eq!(
        ops(&h.state().acts),
        [serde_json::json!({ "op": "add_config_row", "param": "4.t" })]
    );
}

fn zoom(
    p: crate::zoom_panel::ZoomPanel,
    cands: Vec<crate::zoom_panel::Cand>,
) -> Harness<'static, St<crate::zoom_panel::ZoomPanel, Value>> {
    let s = sys();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 1400.0))
        .build_ui_state(
            move |ui, st: &mut St<crate::zoom_panel::ZoomPanel, Value>| {
                let a = crate::zoom_panel::ui(ui, &mut st.p, &cands, &s);
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

/// 状態の無い系では案内が出て、「Solve track」を押しても走らずに理由が出る
#[test]
fn zoom_without_states() {
    let _g = serial();
    let mut h = zoom(Default::default(), vec![]);
    h.get_by_label(
        "Open a system with states (project states or .zmx configurations) to solve each state.",
    );
    h.get_by_label("Solve track").click();
    h.run_steps(4);
    assert!(h.state().acts.is_empty());
    h.get_by_label("Select at least one state");
}

/// 状態の目標を打ち、群の移動を足して「Solve track」を押すと serve の zoom の params が返る。走っている間は
/// ボタンが消えて「Solving…」になる
#[test]
fn zoom_solve_returns_params() {
    let _g = serial();
    let cands = vec![(true, "1".to_string()), (true, "2".to_string())];
    let mut h = zoom(Default::default(), cands);
    // 変数が無い間は理由が出て走らない
    h.get_by_label("Solve track").click();
    h.run_steps(4);
    assert!(h.state().acts.is_empty());
    h.get_by_label("Add parameters to solve (G2.shift, 7.t, etc.)");
    // 群の移動のメニューから足す
    h.get_by_label("Add group shift").click();
    h.run_steps(4);
    h.get_by_label("L1.shift").click();
    h.run_steps(4);
    assert_eq!(h.state().p.vars.len(), 1);
    assert_eq!(h.state().p.vars[0].name, "L1.shift");
    // 1 つ目の状態の目標を打つ（2 つ目は空のまま = 像だけ解く）
    let t = h.get_all(
        by().role(egui::accesskit::Role::TextInput)
            .label("No target"),
    );
    let first = t.into_iter().next().expect("目標の欄");
    first.focus();
    first.type_text("50");
    h.run_steps(4);
    h.get_by_label("Solve track").click();
    h.run_steps(4);
    assert_eq!(h.state().acts.len(), 1);
    let v = &h.state().acts[0];
    assert_eq!(v["configs"], serde_json::json!([1, 2]));
    assert_eq!(v["efl"], serde_json::json!([50.0, null]));
    assert_eq!(v["vars"][0]["name"], "L1.shift");
    // 走っている間
    h.state_mut().p.running = true;
    h.run_steps(4);
    assert!(h.query_by_label("Solve track").is_none());
    h.get_by_label("Solving…");
}

/// 状態の印を外すと、その状態は解かない。全部外すと走らない
#[test]
fn zoom_unchecked_state_is_skipped() {
    let _g = serial();
    let mut p = crate::zoom_panel::ZoomPanel::default();
    p.vars.push(crate::zoom_panel::VarRow {
        name: "3.t".into(),
        ..Default::default()
    });
    let cands = vec![(false, "wide".to_string()), (false, "tele".to_string())];
    let mut h = zoom(p, cands);
    h.get_by_label("wide").click();
    h.run_steps(4);
    h.get_by_label("Solve track").click();
    h.run_steps(4);
    assert_eq!(h.state().acts.len(), 1);
    assert_eq!(h.state().acts[0]["configs"], serde_json::json!(["tele"]));
    assert!(
        h.state().acts[0].get("efl").is_none(),
        "目標が全部空なら efl を付けない"
    );
    h.get_by_label("tele").click();
    h.run_steps(4);
    h.get_by_label("Solve track").click();
    h.run_steps(4);
    assert_eq!(h.state().acts.len(), 1);
    h.get_by_label("Select at least one state");
}
