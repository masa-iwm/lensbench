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

/// App の試験は 1 つずつ回す（組合せの系を置くメモリの作業フォルダ・言語・ファイルの窓の文脈はプロセスで 1 つ）
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
    M.lock().unwrap_or_else(|e| e.into_inner())
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
    click(&mut h, "＋ Insert");
    assert_eq!(
        h.state().c.sys.ld_rows.len(),
        rows0 + 1,
        "{}",
        h.state().test_state()
    );
    click(&mut h, "－ Delete");
    assert_eq!(
        h.state().c.sys.ld_rows.len(),
        rows0,
        "{}",
        h.state().test_state()
    );
}
