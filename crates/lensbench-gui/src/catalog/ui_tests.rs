//! カタログのパネル（[`Browser::ui`]）を egui_kittest で回し、AccessKit の木の文言で探して押す。返す操作と
//! 絞り込みの状態を確かめる（serve は呼ばない。品は選ばないので断面・STEP の取得は走らない）

use super::*;
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;

/// 言語はプロセスで 1 つなので、ほかの画面の試験と重ねない
fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = crate::serve_lock();
    crate::i18n::set(crate::i18n::Lang::En);
    g
}

fn lens(v: &str, p: &str, efl: f64) -> LensRow {
    LensRow {
        vendor: v.into(),
        part: p.into(),
        efl,
        diameter: 25.4,
        thickness: 3.0,
        pieces: 1,
        ..Default::default()
    }
}

fn glass(c: &str, n: &str) -> GlassRow {
    GlassRow {
        name: n.into(),
        catalog: c.into(),
        nd: 1.5,
        vd: 60.0,
        range: None,
    }
}

/// ブラウザ・組合せと、これまでに返した操作（Action は比べられないので名前で持つ）
struct St {
    b: Browser,
    doc: ComboDoc,
    dirs: (usize, usize),
    acts: Vec<&'static str>,
}

fn name(a: &Action) -> &'static str {
    match a {
        Action::Refresh => "refresh",
        Action::Build(_) => "build",
        Action::Settings => "settings",
        Action::SysSettings => "sys_settings",
        Action::New => "new",
        Action::Notice(_) => "notice",
        Action::Fail(_) => "fail",
    }
}

fn harness(b: Browser, dirs: (usize, usize)) -> Harness<'static, St> {
    harness_w(b, dirs, 1100.0)
}

/// 幅 w の画面のブラウザ
fn harness_w(b: Browser, dirs: (usize, usize), w: f32) -> Harness<'static, St> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(w, 900.0))
        .build_ui_state(
            |ui, st: &mut St| {
                if let Some(a) = st.b.ui(ui, &mut st.doc, st.dirs.0, st.dirs.1) {
                    st.acts.push(name(&a));
                }
            },
            St {
                b,
                doc: ComboDoc::default(),
                dirs,
                acts: Vec::new(),
            },
        );
    // 読み込み中は spinner が描き直しを頼み続けるので、決まった数だけ回す
    h.run_steps(4);
    h
}

fn loaded() -> Browser {
    let mut b = Browser::default();
    b.set_lenses(vec![
        lens("Acme", "A-50", 50.0),
        lens("Acme", "A-m30", -30.0),
        lens("Bolt", "B-100", 100.0),
    ]);
    b.set_glasses(vec![glass("SCHOTT", "N-BK7"), glass("OHARA", "S-BSL7")]);
    b
}

/// タブの件数、「Reload」「Folders…」が返す操作、タブの切り替え
#[test]
fn catalog_tabs_and_buttons() {
    let _g = serial();
    let mut h = harness(loaded(), (2, 1));
    h.get_by_label("Catalog lenses (3)");
    h.get_by_label("⟳ Reload").click();
    h.run_steps(4);
    h.get_by_label("Folders… (glass 2, lens 1)").click();
    h.run_steps(4);
    assert_eq!(h.state().acts, ["refresh", "settings"]);
    h.get_by_label("Glasses (2)").click();
    h.run_steps(4);
    assert_eq!(h.state().b.tab, Tab::Glasses);
}

/// 読み込み中はタブに「loading…」と案内が出る。フォルダも品も無ければ足し方の案内が出る
#[test]
fn catalog_loading_and_empty_hints() {
    let _g = serial();
    let b = Browser {
        lenses_pending: true,
        ..Default::default()
    };
    let h = harness(b, (0, 1));
    h.get_by_label("Catalog lenses (loading…)");
    h.get_by_label("Loading catalog lenses (tens of thousands of lenses take tens of seconds)");
    drop(h);
    let h = harness(Browser::default(), (0, 0));
    assert!(h
        .query_by_label_contains("Add lens catalog folders")
        .is_some());
}

/// 検索の欄に打つと一覧の絞り込みが変わり、符号の選びで負の玉だけになる
#[test]
fn catalog_search_and_sign_filter() {
    let _g = serial();
    let mut h = harness(loaded(), (1, 1));
    assert_eq!(h.state_mut().b.lens_idx().len(), 3);
    let q = h.get(
        by().role(egui::accesskit::Role::TextInput)
            .label("Part, vendor, glass"),
    );
    q.focus();
    q.type_text("Acme");
    h.run_steps(4);
    assert_eq!(h.state().b.lens_q.text, "Acme");
    assert_eq!(h.state_mut().b.lens_idx().len(), 2);
    h.get_by_label("Negative").click();
    h.run_steps(4);
    let st = h.state_mut();
    let idx = st.b.lens_idx();
    assert_eq!(idx.len(), 1);
    assert_eq!(st.b.lenses[idx[0]].part, "A-m30");
}

/// 読めない仕様の条件は欄の横に誤りが出る（一覧は絞らない）
#[test]
fn catalog_bad_spec_shows_error() {
    let _g = serial();
    let bad = ["=", "a=..", ">1", "a>="]
        .into_iter()
        .find(|s| parse_spec_filters(s).is_err())
        .expect("読めない条件の例");
    let e = parse_spec_filters(bad).unwrap_err();
    let mut h = harness(loaded(), (1, 1));
    let s = h.get(
        by().role(egui::accesskit::Role::TextInput)
            .label("e.g. wavelength_nm=633"),
    );
    s.focus();
    s.type_text(bad);
    h.run_steps(4);
    h.get_by_label(&lensbench_core::i18n::tr(&e));
}

/// ガラスマップの図は AccessKit の木に名前（件数つき）で出る
#[test]
fn catalog_glass_map_is_named() {
    let _g = serial();
    let mut h = harness(loaded(), (1, 1));
    h.get_by_label("Glasses (2)").click();
    h.run_steps(4);
    h.get(
        by().role(egui::accesskit::Role::Image)
            .label("Glass map (2 glasses)"),
    );
}

/// 狭いパネルでも硝材の表の行は 1 行の高さのまま（残りの波長の列が細くなって文字が折り返し、行が高くなっていた）
#[test]
fn catalog_glass_rows_stay_short_in_a_narrow_panel() {
    let _g = serial();
    let mut b = loaded();
    let mut g = vec![glass("SCHOTT", "N-BK7"), glass("OHARA", "S-BSL7")];
    for x in &mut g {
        x.range = Some([0.31, 2.5]);
    }
    b.set_glasses(g);
    let mut h = harness_w(b, (1, 1), 280.0);
    h.get_by_label("Glasses (2)").click();
    h.run_steps(4);
    let y = |h: &Harness<'static, St>, n: &str| {
        let r = h.get_by_label(n).bounding_box().unwrap();
        (r.y0 + r.y1) / 2.0
    };
    let gap = y(&h, "S-BSL7") - y(&h, "N-BK7");
    assert!(gap > 0.0 && gap < 24.0, "{gap}");
}

/// 組合せの機構部品の欄: 品番と種類・寸法が出て、狭くしてもチェックの字が縦に並ばず、部品の行が 2 行分に収まる
#[test]
fn mech_rows_show_item_and_stay_short_when_narrow() {
    let _g = serial();
    let cat = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/lenses/LBP/parts.json")
        .display()
        .to_string();
    let ms = vec![
        crate::combo::MechPick {
            name: "tube".into(),
            catalog: cat.clone(),
            item: "TUBE-25-30".into(),
            at: "stop".into(),
            holds: vec!["L1".into()],
            ..Default::default()
        },
        crate::combo::MechPick {
            name: "cage".into(),
            catalog: cat.clone(),
            item: "CP-30".into(),
            at: "L1".into(),
            face: 1,
            ..Default::default()
        },
        crate::combo::MechPick {
            name: "gone".into(),
            catalog: cat,
            item: "NO-SUCH".into(),
            ..Default::default()
        },
    ];
    let ids = vec!["stop".to_string(), "L1".to_string()];
    for w in [900.0, 320.0] {
        let mut h = Harness::builder()
            .with_size(egui::vec2(w, 600.0))
            .build_ui_state(
                |ui, ms: &mut Vec<crate::combo::MechPick>| {
                    mech_rows::mech_ui(ui, ms, &ids, &[], &[])
                },
                ms.clone(),
            );
        h.run_steps(2);
        h.get_by_label("TUBE-25-30");
        h.get_by_label("CP-30");
        h.get_by_label("⚠ NO-SUCH");
        let tube_info = "Tube: length 30 mm, bore 23 mm, outer diameter 30.5 mm, thread M25";
        h.get_by_label(tube_info);
        h.get_by_label(
            "Cage plate: length 8 mm, bore 22 mm, outer diameter 41 mm, cage rods 30 mm apart",
        );
        let rows: Vec<_> = h.query_all_by_label("Seat on element").collect();
        assert_eq!(rows.len(), 3);
        for r in &rows {
            let b = r.bounding_box().unwrap();
            assert!(b.y1 - b.y0 < 30.0, "w {w}: {b:?}");
        }
        // 品番の行から次の部品の品番の行まで（部品 1 つ分の高さ）
        let y = |n: &str| h.get_by_label(n).bounding_box().unwrap().y0;
        let step = y("CP-30") - y("TUBE-25-30");
        let max = if w > 500.0 { 70.0 } else { 120.0 };
        assert!(step > 0.0 && step < max, "w {w}: {step}");
    }
}

/// 組合せの図で玉を選び（insert_after）、一覧でカタログの玉を選ぶと「⇄ Replace L1」が出て、押すと差し替えて
/// 知らせを返す。機構部品は選んだ部品（図・欄の品番）を一覧の機構部品で差し替える
#[test]
fn replace_buttons_swap_the_selected_lens_and_part() {
    let _g = serial();
    let ex = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir = ex.join("projects");
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("P19_tube_cage.json")).unwrap())
            .unwrap();
    let combo = crate::combo::Combo::from_project(&v, &dir).unwrap();
    let k = combo.ids().iter().position(|x| x == "L1").unwrap();
    let cat = ex.join("lenses/LBP/parts.json").display().to_string();
    let mut b = Browser::default();
    let mut sl = lens("LB", "SL-254-50", 50.0);
    sl.source = Some(cat.clone());
    let cp = LensRow {
        vendor: "LB".into(),
        part: "CP-30".into(),
        source: Some(cat.clone()),
        kind: RowKind::Mech,
        ..Default::default()
    };
    b.set_lenses(vec![sl, cp]);
    b.lens_sel = Some(0);
    let mut doc = ComboDoc::new(combo);
    doc.insert_after = Some(k);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(
            |ui, st: &mut St| {
                st.doc.insert_after = Some(k);
                if let Some(a) = st.b.ui(ui, &mut st.doc, st.dirs.0, st.dirs.1) {
                    st.acts.push(name(&a));
                }
            },
            St {
                b,
                doc,
                dirs: (1, 1),
                acts: Vec::new(),
            },
        );
    h.run_steps(4);
    h.get_by_label("⇄ Replace L1").click();
    h.run_steps(2);
    assert_eq!(h.state().acts, ["notice"]);
    let p = h.state().doc.combo.el(k).unwrap().clone();
    assert_eq!((p.part.as_str(), p.id.as_str()), ("SL-254-50", "L1"));
    // 機構部品: 選ばなければボタンは出ない。tube を選ぶと出て、押すと品番だけ替わる
    h.state_mut().b.lens_sel = Some(1);
    h.run_steps(2);
    assert!(h.query_by_label("⇄ Replace tube").is_none());
    crate::combo::mech::select(&h.ctx, Some("tube"));
    h.run_steps(2);
    h.get_by_label("⇄ Replace tube").click();
    h.run_steps(2);
    let m = &h.state().doc.combo.mechanics[0];
    assert_eq!((m.item.as_str(), m.at.as_str()), ("CP-30", "stop"));
    assert_eq!(h.state().acts, ["notice", "notice"]);
}

/// 「品を変える…」の候補: 同じ種類の品だけ、保持する要素が入るものを先に
#[test]
fn change_item_candidates_are_same_kind() {
    let cat = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/lenses/LBP/parts.json")
        .display()
        .to_string();
    let m = crate::combo::MechPick {
        name: "cage".into(),
        catalog: cat.clone(),
        item: "CP-30".into(),
        ..Default::default()
    };
    // 同じファイルに他のケージ板は無い。一覧のケージ板の行は候補、チューブの行は候補でない
    assert!(mech_rows::candidates(&m, &[], None).is_empty());
    let row = |part: &str, cat_kind: &str, hold: f64| LensRow {
        part: part.into(),
        source: Some("/x/other.json".into()),
        kind: RowKind::Mech,
        category: cat_kind.into(),
        raw: json!({ "mech": { "holds": { "diameter": hold } } }),
        ..Default::default()
    };
    let rows = vec![
        row("CP-B", "cage_plate", 12.7),
        row("CP-A", "cage_plate", 25.4),
        row("T-1", "tube", 25.4),
    ];
    let c = mech_rows::candidates(&m, &rows, Some(25.4));
    let names: Vec<(&str, Option<bool>)> = c.iter().map(|x| (x.1.as_str(), x.3)).collect();
    assert_eq!(names, [("CP-A", Some(true)), ("CP-B", Some(false))]);
}
