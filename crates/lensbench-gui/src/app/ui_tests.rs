//! 画面の振る舞いのテスト。egui_kittest で窓も GPU も使わずに App を回し、
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
    // 書ける .zmx は要素に分けて開く
    assert!(
        h.state().c.loc.file.is_some() && h.state().c.loc.file == h.state().c.loc.combo_path,
        "{st}"
    );
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
    // 帯のアイコンの名前は字形・帯の幅に依らず字だけ
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
    let solve_ops = |h: &Harness<'static, App>| -> usize {
        let c = &h.state().c.doc.combo;
        c.all()
            .iter()
            .filter_map(|l| c.get(l))
            .flat_map(|p| p.ops.iter())
            .filter(|o| matches!(o, lensbench_project::edit_ops::SurfaceOp::AddSolve(_)))
            .count()
    };
    let n_ops = solve_ops(&h);
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
    // 外す操作は serve の書き戻し（write_back）で要素の面の操作になる（要素に分けたときのソルブを外す）
    assert_eq!(solve_ops(&h), n_ops - 1, "{st}");
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
/// 頼み直して値が入る
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

/// 読めるが組合せで同じ系を書けないプロジェクト（状態で玉を反転する）は開かず、理由を出す（開いた系にしない）
#[test]
fn file_not_writable_as_combo_is_refused() {
    let _g = serial();
    let r = root();
    let dir = std::env::temp_dir().join(format!("lb-gui-refuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    std::fs::write(
        &f,
        r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 6},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 5},
          {"type": "lens", "id": "L", "source": {"surfaces": [{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}},
          {"type": "air", "t": 30}],
        "states": [{"name": "flip", "set": {"L": {"reversed": true}}}]}"#,
    )
    .unwrap();
    let h = harness(Args {
        open: Some(f),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let st = h.state().test_state();
    assert_eq!(st["status_error"], true, "{st}");
    assert_eq!(st["system"], false, "{st}");
    assert!(h.state().c.loc.file.is_none(), "{st}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Ctrl+A: 面の表を使っているときは表の全部の行を選び、多重構成の構成は変えない。表を使っていなければ次の構成
#[test]
fn ctrl_a_selects_rows_in_the_table() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let state =
        |h: &Harness<'static, App>| h.state().c.doc.combo.active_state().map(str::to_string);
    assert!(
        h.state().c.doc.combo.states.len() >= 2,
        "{}",
        h.state().test_state()
    );
    let s0 = state(&h);
    // 表を使っていない: 次の構成
    h.press_key_modifiers(Modifiers::COMMAND, Key::A);
    settle(&mut h);
    let s1 = state(&h);
    assert_ne!(s1, s0, "{}", h.state().test_state());
    // 表のセルを押した後: 全部の行を選び、構成はそのまま
    h.get_by_label("Surface 1 Thickness").click();
    h.run_steps(2);
    h.press_key_modifiers(Modifiers::COMMAND, Key::A);
    settle(&mut h);
    assert_eq!(state(&h), s1, "{}", h.state().test_state());
    // 今のセルは最後の行へ（初めの行から最後の行までを選ぶ）
    assert!(
        h.state().c.p.ld.cur.is_some_and(|c| c.0 > 1),
        "{}",
        h.state().test_state()
    );
}

/// 操作は画面を描かずに当てられ、状態を変えて外への作用（計算の要求・見た目）を返す。返した作用は当てるまで
/// 順番待ちに入らないが、待っている数には入る
#[test]
fn update_applies_a_message_and_returns_effects() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let app = h.state_mut();
    // 見た目: 設定を変え、画面に当てる作用を返す
    let fx = app.update(Msg::Theme(Mode::Light));
    assert_eq!(app.c.s.look.mode, Mode::Light);
    assert_eq!(fx, vec![msg::Effect::Look]);
    // 公差: パネルを表に出し、公差の計算を頼む
    let fx = app.update(Msg::RunTol);
    assert!(app.pane_of(|k| matches!(k, Kind::Tolerance)).is_some());
    assert!(app.c.p.tol.running);
    assert!(
        fx.iter().any(|e| matches!(
            e,
            msg::Effect::Calc {
                pool: msg::Pool::Main,
                tag: Tag::Tolerance,
                ..
            }
        )),
        "{fx:?}"
    );
    // 多重構成の次の構成: 状態はすぐ替わる（系の作り直しは次のフレームで組合せの変更として）
    let s0 = app.c.doc.combo.active_state().map(str::to_string);
    app.update(Msg::CycleConfig(false));
    assert_ne!(app.c.doc.combo.active_state().map(str::to_string), s0);
    // まだ当てていない作用も待っている数に入る
    app.c.calc(Tag::Configs, json!({}));
    assert!(app.c.busy() && app.c.busy_core());
    let fx = app.c.fx.take();
    assert!(!app.c.fx.take().iter().any(|_| true));
    let ctx = h.ctx.clone();
    h.state_mut().apply(&ctx, fx);
    settle(&mut h);
}

/// Ctrl+Shift+Tab はパネルを逆順に出す（Ctrl+Tab が先に取っていた）
#[test]
fn ctrl_shift_tab_cycles_backwards() {
    let _g = serial();
    let mut h = harness(Args::default());
    let focused =
        |h: &mut Harness<'static, App>| h.state_mut().dock.find_active_focused().map(|(_, p)| p.id);
    h.press_key_modifiers(Modifiers::COMMAND, Key::Tab);
    h.run_steps(2);
    let a = focused(&mut h);
    h.press_key_modifiers(Modifiers::COMMAND, Key::Tab);
    h.run_steps(2);
    assert_ne!(focused(&mut h), a);
    h.press_key_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
    h.run_steps(2);
    assert_eq!(focused(&mut h), a);
}

/// パネルの操作も画面なしで当てられる: 面の表から最適化の変数に足すと、変数の行が増え、最適化のパネルを出す
/// 操作が続く（続く操作はパネルを描いた後に続けて当てる）
#[test]
fn panel_messages_apply_without_a_frame() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let app = h.state_mut();
    let fx = app.update(Msg::LensData(lens_data::Act::OptVar("T3".into())));
    assert!(fx.is_empty(), "{fx:?}");
    assert!(app.c.p.opt.vars.iter().any(|v| v.name == "T3"));
    assert!(app.pane_of(|k| matches!(k, Kind::Optimize)).is_none());
    assert!(matches!(
        app.c.msgs.as_slice(),
        [Msg::FocusKind(Kind::Optimize)]
    ));
    h.run_steps(1);
    assert!(h.state().pane_of(|k| matches!(k, Kind::Optimize)).is_some());
    // ズームのパネル: 軌跡を解く要求を出す
    let fx = h.state_mut().update(Msg::Zoom(json!({})));
    assert!(h.state().c.p.zoom.running);
    assert!(matches!(
        fx.as_slice(),
        [msg::Effect::Calc { tag: Tag::Zoom, .. }]
    ));
    h.state_mut().c.p.zoom.running = false;
}

/// ファイルの窓・解析の計算・見るパラメータも操作で当てる: 開く・名前を付けて保存は窓を出す作用を返し、解析の
/// パネルの計算は解析の要求を返す
#[test]
fn dialogs_and_analyses_are_effects() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    let app = h.state_mut();
    let fx = app.update(Msg::Open);
    assert!(
        matches!(
            fx.as_slice(),
            [msg::Effect::Dialog { id, kind: crate::files::DialogKind::Open, .. }] if id == "open-system"
        ),
        "{fx:?}"
    );
    let fx = app.update(Msg::Save(true));
    assert!(
        matches!(
            fx.as_slice(),
            [msg::Effect::Dialog {
                kind: crate::files::DialogKind::Save,
                ..
            }]
        ),
        "{fx:?}"
    );
    assert!(app.c.saving());
    // 窓を出す前にやめた扱いにする（試験では窓を出さない）
    app.c.saves = Default::default();
    let id = app.update(Msg::Add(Kind::Analysis(AnaCfg::new("spot"))));
    assert!(id.is_empty(), "{id:?}");
    let (id, _) = app
        .analyses()
        .into_iter()
        .find(|x| x.1.cmd == "spot")
        .expect("spot panel");
    let fx = app.update(Msg::Analyze(id));
    assert!(
        fx.iter()
            .any(|e| matches!(e, msg::Effect::Calc { tag: Tag::Analyze(x), .. } if *x == id)),
        "{fx:?}"
    );
    app.update(Msg::SetParam("1.t".into(), 2.5));
    assert_eq!(app.c.pending_set.get("1.t"), Some(&2.5));
    let ctx = h.ctx.clone();
    h.state_mut().apply(&ctx, fx);
    h.state_mut().c.pending_set.clear();
    settle(&mut h);
}

/// 低いパネルでも、レンズデータの表の最後の行まで送ると下の「詳細」の帯の上に全部見える（表が自分の矩形の下へ
/// はみ出さない）
#[test]
fn last_row_of_the_table_is_above_the_details_bar() {
    let _g = serial();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 560.0))
        .with_max_steps(1000)
        .build_eframe(move |cc| App::new(cc, a0()));
    i18n::set(i18n::Lang::En);
    settle(&mut h);
    let n = h.state().c.sys.ld_rows.len() - 1;
    h.get_by_label("Surface 1 Thickness").click();
    h.run_steps(2);
    for _ in 0..n {
        h.press_key(Key::ArrowDown);
        h.run_steps(2);
    }
    h.run_steps(4);
    // 帯の字（ラベルと字の並びの 2 つの節）
    let bar = h
        .query_all(
            egui_kittest::kittest::by()
                .predicate(|n| n.value().is_some_and(|l| l.contains("Details —"))),
        )
        .next()
        .and_then(|n| n.bounding_box())
        .unwrap();
    let last = h
        .get_by_label(&format!("Surface {n} Type"))
        .bounding_box()
        .unwrap();
    assert!(last.y1 <= bar.y0, "last row {last:?}, bar {bar:?}");
}

/// 図で選んだ BS・先頭に自動で置く絞り（表のブロックの名前を持たない）も、レンズデータの表でその面の行を選ぶ
/// （BS は往路と、鏡で戻って口 D へ出る復路の両方）
#[test]
fn layout_selection_marks_bs_and_lead_stop_rows() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P16_twyman_green.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    assert_eq!(h.state().c.doc.combo.ids(), ["BS", "TEST", "M_TEST"]);
    let comments = |h: &Harness<'static, App>| -> Vec<String> {
        let c = &h.state().c;
        c.p.ld
            .hl
            .iter()
            .flat_map(|r| r[0]..=r[1])
            .map(|j| c.sys.ld_rows[j].comment.clone())
            .collect()
    };
    h.state_mut().c.p.combo_view.select(0);
    settle(&mut h);
    assert_eq!(
        comments(&h),
        ["BS face", "", "BS split", "", "BS face", "BS face", "", "BS split", "", "BS face"],
        "{:?}",
        h.state().c.p.ld.hl
    );
    h.state_mut().c.p.combo_view.select_lead();
    settle(&mut h);
    let c = &h.state().c;
    assert!(c.p.ld.hl.len() == 1 && c.sys.ld_rows[c.p.ld.hl[0][0]].stop);
    // 表のブロックのある要素はブロックで選ぶ（印は付けない）
    h.state_mut().c.p.combo_view.select(1);
    settle(&mut h);
    let c = &h.state().c;
    assert!(c.p.ld.hl.is_empty());
    assert_eq!(c.p.ld.sel.as_deref(), Some("TEST"));
}

/// 欄の値の書き換えも操作で当てる: 組合せを変えた所だけが書き換えになり、当てると文書が変わって描き直しを
/// 頼む。元に戻す段は書き換えでは区切らず（入力の途中を 1 段にまとめる）、戻すと前の値に戻る
#[test]
fn field_edits_are_messages() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    // 何も触らずに描き続けても、ウィジェットは書き換えを出さない（文書の版が進まない）
    let rev = h.state().c.doc.rev;
    h.run_steps(4);
    assert_eq!(h.state().c.doc.rev, rev);
    let app = h.state_mut();
    let epd0 = app.c.doc.combo.epd;
    let mut want = app.c.doc.combo.clone();
    want.epd = epd0 + 1.0;
    let e = edit::Edit::between(&app.c.doc.combo, &want).expect("changed");
    assert_eq!(e.ops.len(), 1, "{e:?}");
    let fx = app.update(Msg::Edit(e.clone()));
    assert_eq!(fx, vec![msg::Effect::Repaint]);
    assert_eq!(app.c.doc.combo, want);
    // 2 つ目の書き換えも同じ段（入力の途中）
    let mut want2 = want.clone();
    want2.epd = epd0 + 2.0;
    app.update(Msg::Edit(edit::Edit::between(&want, &want2).unwrap()));
    h.run_steps(2);
    assert_eq!(h.state().c.doc.combo.epd, epd0 + 2.0);
    h.press_key_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(
        h.state().c.doc.combo.epd,
        epd0,
        "{}",
        h.state().test_state()
    );
    // 場所の無い書き換えは誤りを出し、文書を変えない
    let app = h.state_mut();
    let before = app.c.doc.combo.clone();
    let bad = edit::Edit {
        ops: vec![edit::Op {
            path: vec![edit::Seg::Key("picks".into()), edit::Seg::Index(999)],
            value: Some(json!({})),
        }],
    };
    let fx = app.update(Msg::Edit(bad));
    assert!(fx.is_empty(), "{fx:?}");
    assert_eq!(app.c.doc.combo, before);
    assert!(app.c.status.error, "{}", app.test_state());
}

/// 操作の記録: 欄の書き換え・元に戻す・構成の切り替えを記録してファイルの形にし、開き直した App で流し直すと同じ
/// 組合せになる。流し直す前に今の組合せを記録を始めた時へ戻す
#[test]
fn recorded_operations_replay_to_the_same_combination() {
    let _g = serial();
    let r = root();
    let args = || Args {
        open: Some(r.join("examples/projects/P6_states_focus_group.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    };
    let mut h = harness(args());
    let app = h.state_mut();
    let start = app.c.doc.combo.clone();
    assert!(app.update(Msg::Record).is_empty());
    assert!(app.c.rec.is_some());
    let edit = |app: &mut App, f: &dyn Fn(&mut Combo)| {
        let mut b = app.c.doc.combo.clone();
        f(&mut b);
        let e = edit::Edit::between(&app.c.doc.combo, &b).expect("changed");
        app.update(Msg::Edit(e));
    };
    edit(app, &|c| c.picks[0].air += 1.5);
    edit(app, &|c| c.picks[1].air += 0.25);
    app.update(Msg::Undo(true, 1));
    edit(app, &|c| c.picks[0].semi += 1.0);
    app.update(Msg::CycleConfig(false));
    // 返事を待って当たる変更（像面を焦点へ・差し替えの挿入など）は操作の外で組合せを変える: 次の操作の前に
    // 変わった所として残り、その後の書き換えより先に当たる
    app.c.doc.combo.picks[1].air += 3.0;
    edit(app, &|c| c.picks[1].semi += 0.5);
    // 文書を変えない操作は残らない
    app.update(Msg::Theme(Mode::Light));
    app.c.doc.combo.picks[0].air += 0.125;
    let end = app.c.doc.combo.clone();
    assert_ne!(end, start);
    // やめるときに最後の変更も残る（保存の窓は出さずに中身を取る）
    let mut rec = app.c.rec.take().expect("recording");
    rec.note(&app.c.doc.combo);
    let rec = rec.file;
    assert_eq!(rec.ops.len(), 8, "{:?}", rec.ops);
    assert!(rec
        .ops
        .iter()
        .all(|r| matches!(r, record::Rec::Edit { .. })));
    let rec = record::Record::read(&rec.text()).expect("reads back");
    settle(&mut h);
    drop(h);

    // 開き直して先に別の書き換えをしておく（流し直すと記録を始めた時へ戻ってから当てる）
    let mut h = harness(args());
    let app = h.state_mut();
    edit(app, &|c| c.picks[0].air += 7.0);
    app.c.replay(rec);
    let ctx = h.ctx.clone();
    h.state_mut().dispatch_queued(&ctx);
    assert_eq!(h.state().c.doc.combo, end);
    // 記録を始めた所より前へ戻す操作で止まり、残りは当てない
    let mut bad = record::Record::new(&Combo::default());
    bad.ops = vec![
        record::Rec::Undo {
            back: true,
            steps: 50,
        },
        record::Rec::CycleConfig { back: false },
    ];
    let state = h.state().c.doc.combo.state.clone();
    h.state_mut().c.replay(bad);
    h.state_mut().dispatch_queued(&ctx);
    let app = h.state();
    assert!(app.c.status.error, "{}", app.c.status.text);
    assert!(app.c.status.text.contains('1'), "{}", app.c.status.text);
    assert_eq!(app.c.doc.combo, end);
    assert_eq!(app.c.doc.combo.state, state);
    // 何も変えない書き換え（当たっていない）でも止まる
    let mut moved = end.clone();
    moved.picks[0].air += 1.0;
    let e = edit::Edit::between(&end, &moved).expect("changed");
    let mut noop = record::Record::new(&moved);
    noop.ops = vec![
        record::Rec::Edit { ops: e.ops },
        record::Rec::CycleConfig { back: false },
    ];
    h.state_mut().c.replay(noop);
    h.state_mut().dispatch_queued(&ctx);
    let app = h.state();
    assert!(app.c.status.error, "{}", app.c.status.text);
    assert_eq!(app.c.doc.combo, moved);
    assert_eq!(app.c.doc.combo.state, state);
    settle(&mut h);
}

/// 面のタイプを変える操作は元に戻す段を 1 つだけ積み、1 回で戻り、やり直せる（11/20 masa の報告: 面の上書きの印を
/// 後の画面で付けて段が 2 つ積まれ、1 段戻しても印を付け直して戻らなかった）
#[test]
fn surface_type_change_undoes_in_one_step() {
    let _g = serial();
    let mut h = harness(a0());
    let j = 3;
    let typ0 = h.state().c.sys.ld_rows[j].typ.clone();
    assert_ne!(typ0, "EVENASPH");
    let c0 = h.state().c.doc.combo.clone();
    let depth0 = h.state().c.combo_hist.list(true).len();
    h.state_mut().update(Msg::LensData(lens_data::Act::Edit {
        j,
        col: lens_data::Col::Type,
        value: json!("EVENASPH"),
    }));
    settle(&mut h);
    assert_eq!(h.state().c.sys.ld_rows[j].typ, "EVENASPH");
    let c1 = h.state().c.doc.combo.clone();
    assert_eq!(
        h.state().c.combo_hist.list(true).len(),
        depth0 + 1,
        "{:?}",
        h.state().c.combo_hist.list(true)
    );
    h.press_key_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(h.state().c.doc.combo, c0, "{}", h.state().test_state());
    assert_eq!(h.state().c.sys.ld_rows[j].typ, typ0);
    h.press_key_modifiers(Modifiers::COMMAND, Key::Y);
    settle(&mut h);
    assert_eq!(h.state().c.doc.combo, c1, "{}", h.state().test_state());
    assert_eq!(h.state().c.sys.ld_rows[j].typ, "EVENASPH");
}
