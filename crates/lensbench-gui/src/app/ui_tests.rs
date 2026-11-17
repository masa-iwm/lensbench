//! 画面の振る舞いのテスト（GAR-07）。egui_kittest で窓も GPU も使わずに App を回し、
//! AccessKit の木（ボタンの文言）で探して押す。計算は本物の Worker（serve と同じ method をスレッドで回す）。
//!
//! `--tour` は画面を撮るだけで、押した結果を確かめない。ここでは押した後の文書と表の中身を確かめる

use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::time::{Duration, Instant};

/// リポジトリの根（public/）
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// App の試験は 1 つずつ回し、serve を直に呼ぶほかのテストとも重ねない（serve の系・組合せの系を置くメモリの作業
/// フォルダ・言語・ファイルの窓の文脈はプロセスで 1 つ）
fn serial() -> std::sync::MutexGuard<'static, ()> {
    crate::serve_lock()
}

/// 引数を付けて App を作る（設定は保存先から読まない: 試験の Harness は保存先を持たない）
fn harness(args: Args) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 820.0))
        .with_max_steps(1000)
        .build_eframe(move |cc| App::new(cc, args));
    // 言語は英語に揃える（文言で探す。App::new は設定の「自動」を当てる）
    i18n::set(i18n::Lang::En);
    settle(&mut h);
    h
}

/// 計算と下書きの要求が片付くまでフレームを回す（10 秒で諦める）
fn settle(h: &mut Harness<'static, App>) {
    let t0 = Instant::now();
    loop {
        h.run_steps(2);
        if !h.state().test_state()["busy"].as_bool().unwrap_or(false) {
            h.run_steps(2);
            if !h.state().test_state()["busy"].as_bool().unwrap_or(false) {
                return;
            }
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "still busy: {}",
            h.state().test_state()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 文言で探して押し、要求が片付くまで回す
fn click(h: &mut Harness<'static, App>, label: &str) {
    h.get_by_label(label).click();
    settle(h);
}

/// レンズデータの左の帯のアイコンを押す。文言は「記号 字」だが、記号の字形が無いフォント（日本語フォントの無い
/// Linux など）では字だけ、帯が狭いと記号だけになる（fonts::glyph_or）ので、どれでも探す
fn click_icon(h: &mut Harness<'static, App>, sym: &str, word: &str) {
    let both = format!("{sym} {word}");
    let (sym, word) = (sym.to_owned(), word.to_owned());
    h.get(egui_kittest::kittest::by().predicate(move |n| {
        n.label()
            .is_some_and(|l| l == both || l == word || l == sym)
    }))
    .click();
    settle(h);
}

/// 空の文書の「面を追加」で平面 1 枚の面の並びができ、表に出る（11/16 masa 決定: 表から自由に面を足す）
#[test]
fn empty_doc_add_surface() {
    let _g = serial();
    let mut h = harness(Args::default());
    assert!(h.state().c.doc.combo.picks.is_empty());
    assert!(h.state().c.sys.ld_rows.is_empty());
    click(&mut h, "+ Add a surface");
    let app = h.state();
    assert_eq!(
        app.c.doc.combo.picks.len(),
        1,
        "{:?}",
        app.c.doc.combo.picks
    );
    assert!(app.c.sys.summary.is_some(), "{}", app.test_state());
    assert!(!app.c.sys.ld_rows.is_empty(), "{}", app.test_state());
    // 表が出たら空の文書の案内は消える
    assert!(h.query_by_label("+ Add a surface").is_none());
}

/// 系のファイルを指定して開くと、状態の行に系の値が出て、誤りは出ない
#[test]
fn open_zmx_shows_system() {
    let _g = serial();
    let r = root();
    let args = Args {
        open: Some(r.join("tests/refs/A0/system.zmx")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    };
    let h = harness(args);
    let st = h.state().test_state();
    assert_eq!(st["system"], true, "{st}");
    assert_eq!(st["status_error"], false, "{st}");
    assert!(!h.state().c.sys.ld_rows.is_empty());
    // 書ける .zmx は要素に分けて開く（#486）
    assert!(h.state().c.is_combo(), "{st}");
    assert!(!h.state().c.doc.combo.picks.is_empty());
}

fn a0() -> Args {
    let r = root();
    Args {
        open: Some(r.join("tests/refs/A0/system.zmx")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    }
}

/// 解析パネルの「計算」で結果が出て、誤りは出ない
#[test]
fn compute_analysis() {
    let _g = serial();
    let mut h = harness(a0());
    click(&mut h, "▶ Compute (F5)");
    let st = h.state().test_state();
    let a = st["analyses"].as_array().expect("analyses");
    assert!(!a.is_empty(), "{st}");
    assert!(a.iter().any(|x| x["ok"] == true), "{st}");
    assert!(a.iter().all(|x| x["error"].is_null()), "{st}");
}

/// 空の文書から表の「挿入」「削除」で面を足し引きする。前の絞りの行の前に足すと面は絞りの後に入り、
/// 今のセルもそちらへ移る（移らないと続けて押した「削除」が絞りに当たる）
#[test]
fn free_edit_insert_delete() {
    let _g = serial();
    let mut h = harness(Args::default());
    click(&mut h, "+ Add a surface");
    let rows0 = h.state().c.sys.ld_rows.len();
    // 帯のアイコンの名前は字形・帯の幅に依らず字だけ（GPN-22）
    assert!(h.query_by_label("Insert").is_some());
    click_icon(&mut h, "＋", "Insert");
    assert_eq!(
        h.state().c.sys.ld_rows.len(),
        rows0 + 1,
        "{}",
        h.state().test_state()
    );
    click_icon(&mut h, "－", "Delete");
    assert_eq!(
        h.state().c.sys.ld_rows.len(),
        rows0,
        "{}",
        h.state().test_state()
    );
}

/// 面の追加を Ctrl+Z で戻し、Ctrl+Y でやり直す（組合せの文書の元に戻す）
#[test]
fn undo_redo_add_surface() {
    let _g = serial();
    let mut h = harness(Args::default());
    click(&mut h, "+ Add a surface");
    assert_eq!(h.state().c.doc.combo.picks.len(), 1);
    h.press_key_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert!(
        h.state().c.doc.combo.picks.is_empty(),
        "{}",
        h.state().test_state()
    );
    h.press_key_modifiers(Modifiers::COMMAND, Key::Y);
    settle(&mut h);
    assert_eq!(
        h.state().c.doc.combo.picks.len(),
        1,
        "{}",
        h.state().test_state()
    );
}

/// A0 の最後の厚さのソルブ（周辺光線の高さ 0）を、セルの右クリックの欄（Shift+F10）の「Remove」で外す。
/// 厚さは解いた値のまま残る（11/16 masa の報告: 要素に分けて開いた系でソルブを外せなかった）
#[test]
fn remove_thickness_solve() {
    let _g = serial();
    let mut h = harness(a0());
    let solved = |h: &Harness<'static, App>| -> Vec<usize> {
        h.state()
            .c
            .sys
            .ld_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.solves.iter().any(|s| s.0 == "t"))
            .map(|(i, _)| i)
            .collect()
    };
    let j = *solved(&h).last().expect("A0 has a thickness solve");
    let t = h.state().c.sys.ld_rows[j].t;
    // セルは AccessKit の木に「Surface 面 列」の名前で出る
    h.get_by_label(&format!("Surface {j} Thickness")).click();
    h.run_steps(2);
    assert_eq!(h.state().c.p.ld.cur, Some((j, lens_data::Col::T)));
    h.press_key_modifiers(Modifiers::SHIFT, Key::F10);
    h.run_steps(2);
    click(&mut h, "Remove");
    let st = h.state().test_state();
    assert!(!solved(&h).contains(&j), "{st}");
    assert_eq!(st["status_error"], false, "{st}");
    assert!((h.state().c.sys.ld_rows[j].t - t).abs() < 1e-9 * t.abs().max(1.0));
    // 元に戻すとソルブも戻る
    h.press_key_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert!(solved(&h).contains(&j), "{}", h.state().test_state());
}

/// ソルブの種類を選んでも [...] の欄は開いたままで、種類の一覧は閉じる（一覧を別の小窓にしていたときは、選ぶと
/// 欄が閉じて一覧だけが残った）
#[test]
fn solve_kind_pick_keeps_menu() {
    let _g = serial();
    let mut h = harness(a0());
    h.get_by_label("Surface 1 Thickness").click();
    h.run_steps(2);
    h.press_key_modifiers(Modifiers::SHIFT, Key::F10);
    h.run_steps(2);
    let kind_btn = |h: &Harness<'static, App>, k: &str| {
        let want = format!("Kind: {k} ");
        h.query_by(move |n| n.label().is_some_and(|l| l.starts_with(&want)))
            .is_some()
    };
    assert!(kind_btn(&h, "marginal_height"));
    assert!(h.query_by_label("edge").is_none());
    h.get(
        egui_kittest::kittest::by()
            .predicate(|n| n.label().is_some_and(|l| l.starts_with("Kind: "))),
    )
    .click();
    h.run_steps(2);
    h.get_by_label("edge").click();
    h.run_steps(2);
    assert!(kind_btn(&h, "edge"), "{}", h.state().test_state());
    // 一覧は閉じ、欄は残る
    assert!(h.query_by_label("chief_height").is_none());
    assert!(h.query_by_label("+ Make solve").is_some());
}

/// 見るパラメータの get は 1 つの要求に束ねる。先頭のパラメータが誤り（無い面）で止まっても、残りは 1 つずつ
/// 頼み直して値が入る（GAR-05）
#[test]
fn bundled_gets_survive_first_error() {
    let _g = serial();
    let mut h = harness(a0());
    for params in [["99.t", "1.t", "2.t"], ["1.t", "99.t", "2.t"]] {
        h.state_mut().c.ses.params = params.iter().map(|s| s.to_string()).collect();
        h.state_mut().c.sys.param_vals.clear();
        h.state_mut().c.read_params();
        settle(&mut h);
        let vals = &h.state().c.sys.param_vals;
        assert!(
            vals.contains_key("1.t") && vals.contains_key("2.t"),
            "{params:?}: {vals:?}"
        );
        assert!(!vals.contains_key("99.t"));
    }
}
