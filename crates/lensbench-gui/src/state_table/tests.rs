//! 状態の表（多重構成の表）の試験

use super::*;

/// 状態で変えた値の行だけを出し、状態の列のセルを変えるとその状態の値、基本に戻すと外れる
#[test]
fn rows_follow_state_values() {
    let mut c = Combo::default();
    let mut z = Pick::new(ElemKind::ZmxRange);
    z.path = "a.zmx".into();
    c.picks.push(z);
    c.picks.push(Pick::new(ElemKind::Plate));
    c.states = vec!["Config 1".into(), "Config 2".into()];
    assert!(items(&c, false, &[]).is_empty());
    c.picks[0]
        .zmx_mut()
        .state_config
        .insert("Config 2".into(), 2);
    let rows = items(&c, false, &[]);
    assert_eq!(rows, vec![Item::Config(vec![0])]);
    let none = |_: &Face| None;
    let cache = HashMap::new();
    let cx = Cx {
        now: &none,
        cache: &cache,
    };
    rows[0].set(&mut c, Some("Config 1"), Val::Int(1), &cx);
    assert!(rows[0].own(&c, "Config 1"));
    assert_eq!(c.picks[0].config_in(Some("Config 2")), 2);
    rows[0].clear(&mut c, "Config 2");
    assert_eq!(c.picks[0].config_in(Some("Config 2")), 1);
    // 全部: 要素の ON/OFF・後の空気・構成
    let all = items(&c, true, &[]);
    assert!(all.contains(&Item::On(vec![1])) && all.contains(&Item::Air(vec![1])));
    Item::Air(vec![1]).set(&mut c, Some("Config 2"), Val::Num(7.0), &cx);
    assert_eq!(c.picks[1].air_in(Some("Config 2")), 7.0);
    assert_eq!(c.picks[1].air_in(None), c.picks[1].air);
}

/// 系の値の行: 状態の列を変えるとその状態の系の値になり、基本に戻すと（ほかに違いが無ければ）状態の系の値は外れる
#[test]
fn system_rows() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    assert!(!items(&c, false, &[])
        .iter()
        .any(|i| matches!(i, Item::Sys(_))));
    let none = |_: &Face| None;
    let cache = HashMap::new();
    let cx = Cx {
        now: &none,
        cache: &cache,
    };
    let w2 = Item::Sys(SysQ::Wave(1));
    w2.set(&mut c, Some("A"), Val::Num(0.55), &cx);
    assert_eq!(c.state_sys["A"].waves[1].um, 0.55);
    assert!(w2.own(&c, "A") && !w2.own(&c, "B"));
    assert!(items(&c, false, &[]).contains(&w2));
    assert_eq!(w2.get(&c, Some("B"), &cx), Some(Val::Num(c.waves[1].um)));
    // 開口の種類を F 値にすると初めの値
    let at = Item::Sys(SysQ::ApType);
    at.set(&mut c, Some("B"), Val::Choice(1, &AP_KEYS), &cx);
    assert_eq!(c.state_sys["B"].aperture, ApertureKind::FNum);
    assert_eq!(
        Item::Sys(SysQ::ApValue).get(&c, Some("B"), &cx),
        Some(Val::Num(5.0))
    );
    // 物体距離
    let od = Item::Sys(SysQ::ObjDist);
    od.set(&mut c, Some("A"), Val::Text("100".into()), &cx);
    assert_eq!(c.state_sys["A"].object_distance, Some(100.0));
    w2.clear(&mut c, "A");
    od.clear(&mut c, "A");
    assert!(!c.state_sys.contains_key("A"));
    // 基本を変える
    w2.set(&mut c, None, Val::Num(0.6), &cx);
    assert_eq!(c.waves[1].um, 0.6);
}

/// 面の行: 足すと全部の状態に今の値が入り、状態の列を変えるとその状態だけ、行を消すと上書きが外れる
#[test]
fn face_rows() {
    let mut c = Combo::default();
    c.picks.push(Pick::new(ElemKind::Plate));
    let mut view = View::default();
    let now = |f: &Face| match f.field.as_str() {
        "t" => Some(json!(3.0)),
        "asph" => Some(json!([])),
        _ => None,
    };
    let f = Face {
        loc: vec![0],
        surf: 0,
        field: "t".into(),
        idx: None,
    };
    // 状態が無ければ構成 2 つを足す
    add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
    assert_eq!(c.states, ["Config 1", "Config 2"]);
    assert_eq!(c.picks[0].state_edit("Config 2", 0, "t"), Some(&json!(3.0)));
    let it = Item::Face(f.clone());
    assert!(items(&c, false, &view.pinned).contains(&it));
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    it.set(&mut c, Some("Config 2"), Val::Num(5.0), &cx);
    assert_eq!(it.get(&c, Some("Config 2"), &cx), Some(Val::Num(5.0)));
    assert_eq!(it.get(&c, Some("Config 1"), &cx), Some(Val::Num(3.0)));
    assert_eq!(
        c.picks[0].edits_in("Config 2").unwrap()[0].value,
        json!(5.0)
    );
    // 並びの値（非球面）は番号の所だけ変える
    let a = Item::Face(Face {
        field: "asph".into(),
        idx: Some(1),
        ..f.clone()
    });
    a.set(&mut c, Some("Config 1"), Val::Num(-2e-5), &cx);
    assert_eq!(
        c.picks[0].state_edit("Config 1", 0, "asph"),
        Some(&json!([0.0, -2e-5]))
    );
    assert!(items(&c, false, &[]).contains(&a));
    remove_row(&mut c, &mut view, &it);
    assert!(c.picks[0].state_edit("Config 2", 0, "t").is_none());
    assert!(!items(&c, false, &view.pinned).contains(&it));
    // 面の並びが変わると状態の上書きも外れる
    c.picks[0].reversed = !c.picks[0].reversed;
    assert!(c.picks[0].drop_stale_edits() > 0);
    assert!(c.picks[0].state_edits.is_empty());
}

/// 今の状態が上書きしている値は基本の値として読まない
#[test]
fn current_state_value_is_not_base() {
    // 今の状態 A が t を 5 に上書きし、今の系の t は 5（基本は 3 だが覚えていない）
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        state: Some("A".into()),
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    c.picks[0].set_state_edit("A", 0, "t", json!(5.0));
    let now = |f: &Face| (f.field == "t").then(|| json!(5.0));
    let f = Face {
        loc: vec![0],
        surf: 0,
        field: "t".into(),
        idx: None,
    };
    let mut view = View::default();
    add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
    // 基本が分からないので B には値を入れない
    assert_eq!(c.picks[0].state_edit("B", 0, "t"), None);
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    assert_eq!(
        Item::Face(f.clone()).get(&c, Some("B"), &cx),
        Some(Val::Unknown)
    );
    // 覚えた基本の値があればそれを入れる
    view.base_cache.insert(f.clone(), json!(3.0));
    add_row(&mut c, &mut view, Item::Face(f.clone()), &now);
    assert_eq!(c.picks[0].state_edit("B", 0, "t"), Some(&json!(3.0)));
    // ロックした要素の面の値は変えない
    c.picks[0].locked = true;
    let k = Face {
        field: "k".into(),
        ..f
    };
    add_row(&mut c, &mut view, Item::Face(k.clone()), &now);
    Item::Face(k).set(&mut c, Some("B"), Val::Num(-1.0), &cx);
    assert_eq!(c.picks[0].state_edit("B", 0, "k"), None);
}

/// 絞りの面・テレセントリックの行
#[test]
fn stop_tele_rows() {
    let mut c = Combo {
        states: vec!["A".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    let none = |_: &Face| None;
    let cache = HashMap::new();
    let cx = Cx {
        now: &none,
        cache: &cache,
    };
    let at = Some((vec![0], 1));
    Item::Stop.set(&mut c, Some("A"), Val::Stop(at.clone()), &cx);
    assert!(Item::Stop.own(&c, "A"));
    assert_eq!(Item::Stop.get(&c, Some("A"), &cx), Some(Val::Stop(at)));
    assert!(items(&c, false, &[]).contains(&Item::Stop));
    // 基本と同じ絞りにすると状態の値は外れる
    let base = c.base_stop();
    Item::Stop.set(&mut c, Some("A"), Val::Stop(base), &cx);
    assert!(!Item::Stop.own(&c, "A"));
    let tele = Item::Sys(SysQ::Telecentric);
    tele.set(&mut c, Some("A"), Val::Bool(true), &cx);
    assert!(c.state_sys["A"].telecentric && !c.telecentric);
}

/// ピックアップ: 状態 B の値を A の値 × 倍率 ＋ オフセットに保ち、状態の名前を変える・消すと付いていく
#[test]
fn config_pickups() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    let now = |f: &Face| (f.field == "t").then(|| json!(3.0));
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    let air = Item::Air(vec![0]);
    picks::set_pick(&mut c, &air, "B", &picks::Src::conf("A"), 2.0, 1.0);
    air.set(&mut c, Some("A"), Val::Num(3.0), &cx);
    assert!(picks::apply(&mut c, &cx));
    assert_eq!(c.picks[0].air_in(Some("B")), 7.0);
    assert!(!picks::apply(&mut c, &cx), "変わらなければ書かない");
    // 面の値・システムの値も（基本から）
    let t = Item::Face(Face {
        loc: vec![0],
        surf: 0,
        field: "t".into(),
        idx: None,
    });
    picks::set_pick(&mut c, &t, "A", &picks::Src::conf(""), 1.0, 0.5);
    let w = Item::Sys(SysQ::Wave(0));
    picks::set_pick(&mut c, &w, "B", &picks::Src::conf("A"), 1.0, 0.0);
    w.set(&mut c, Some("A"), Val::Num(0.6), &cx);
    picks::apply(&mut c, &cx);
    assert_eq!(c.picks[0].state_edit("A", 0, "t"), Some(&json!(3.5)));
    assert_eq!(w.get(&c, Some("B"), &cx), Some(Val::Num(0.6)));
    // ピックアップのある行は値が基本と同じでも出す
    assert!(items(&c, false, &[]).contains(&w));
    // 行の名前と行は行き来できる
    for it in [&air, &t, &w, &Item::Stop, &Item::Place(vec![0], 3)] {
        let (loc, key) = picks::key_of(it);
        assert_eq!(picks::item_of(loc.as_ref(), &key).as_ref(), Some(it));
    }
    // 保存の形
    let j = picks::to_json(&c).unwrap();
    let mut d = c.clone();
    d.mc_picks.clear();
    d.picks[0].mc_picks.clear();
    let id = c.ids_all()[0].1.clone();
    assert!(j[&id].is_array() && j[""].is_array());
    picks::from_json(&mut d, Some(&j));
    assert_eq!(d, c);
    // 名前を変える・消す
    c.rename_state("A", "C").unwrap();
    assert_eq!(picks::pick_of(&c, &air, "B").unwrap().from, "C");
    assert_eq!(picks::pick_of(&c, &t, "C").unwrap().from, "");
    c.remove_state("C");
    assert!(picks::pick_of(&c, &air, "B").is_none());
    assert!(c.mc_picks.is_empty());
}

/// 構成をピックアップで足す・基本の値で足す・入れ替える
#[test]
fn insert_and_move_configs() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    c.picks[0].state_air.insert("A".into(), 9.0);
    let air = Item::Air(vec![0]);
    let n = picks::add_state_with_picks(&mut c, "A", std::slice::from_ref(&air)).unwrap();
    assert_eq!(c.states, ["A", n.as_str(), "B"]);
    assert_eq!(c.picks[0].air_in(Some(&n)), 9.0);
    assert_eq!(picks::pick_of(&c, &air, &n).unwrap().from, "A");
    let m = c.next_config_name();
    c.add_state(&m, None);
    c.place_state_after(&m, "A");
    assert_eq!(c.states[1], m);
    assert!(c.move_state("B", -1));
    assert!(!c.move_state("A", -1));
    assert_eq!(c.states.last().map(String::as_str), Some(n.as_str()));
}

/// 開口の最大の半径: 基本の開口が分かるときだけ、無いか円形・遮蔽の最大の半径を変える
#[test]
fn aperture_max_radius() {
    assert_eq!(
        face_json("aperture", &Val::Num(0.0), None, Some(&Value::Null)),
        Some(Value::Null)
    );
    assert_eq!(face_json("aperture", &Val::Num(4.0), None, None), None);
    let ring = json!({ "type": "circular", "min": 1.0, "max": 5.0, "dx": 0.0, "dy": 0.0 });
    assert_eq!(
        face_json("aperture", &Val::Num(4.0), None, Some(&ring)),
        Some(json!({ "type": "circular", "min": 1.0, "max": 4.0, "dx": 0.0, "dy": 0.0 }))
    );
    let rect = json!({ "type": "rect", "hx": 1.0, "hy": 2.0 });
    assert_eq!(
        face_json("aperture", &Val::Num(4.0), None, Some(&rect)),
        None
    );
    assert_eq!(
        face_json("aperture", &Val::Num(0.0), None, Some(&rect)),
        None
    );
    assert_eq!(face_val("aperture", None, Some(&rect)), Val::Unknown);
    assert_eq!(face_val("aperture", None, Some(&ring)), Val::Num(5.0));
    assert_eq!(
        face_val("aperture", None, Some(&Value::Null)),
        Val::Num(0.0)
    );
    assert_eq!(
        face_json("aperture", &Val::Num(3.0), None, Some(&Value::Null)),
        Some(json!({ "type": "circular", "min": 0.0, "max": 3.0 }))
    );
}

/// 書けないピックアップ（主波長の範囲の外など）は当てても変えたことにしない
#[test]
fn unwritable_pickup_is_not_a_change() {
    let mut c = Combo {
        states: vec!["A".into()],
        ..Default::default()
    };
    let none = |_: &Face| None;
    let cache = HashMap::new();
    let cx = Cx {
        now: &none,
        cache: &cache,
    };
    let w = Item::Sys(SysQ::Wave(0));
    picks::set_pick(&mut c, &w, "A", &picks::Src::conf(""), 0.0, 0.0);
    let p = Item::Sys(SysQ::Primary);
    picks::set_pick(&mut c, &p, "A", &picks::Src::conf(""), 0.0, 50.0);
    picks::apply(&mut c, &cx);
    assert!(!picks::apply(&mut c, &cx));
    // 書けない主波長のピックアップでシステムの値の写しを作らない
    let mut d = Combo {
        states: vec!["A".into()],
        ..Default::default()
    };
    picks::set_pick(&mut d, &p, "A", &picks::Src::conf(""), 0.0, 50.0);
    picks::apply(&mut d, &cx);
    assert!(d.state_sys.is_empty());
}

/// 表の部品で描く状態の表: セルの種類・打った文字の読み方・空にすると基本へ・ピックアップは書けない・行の入れ替え
#[test]
fn sheet_cells_and_set() {
    use crate::widgets::sheet::{Kind, Sheet};
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    let none = |_: &Face| None;
    let cache = HashMap::new();
    let air = Item::Air(vec![0]);
    let tele = Item::Sys(SysQ::Telecentric);
    let ap = Item::Sys(SysQ::ApType);
    picks::set_pick(&mut c, &air, "B", &picks::Src::conf("A"), 2.0, 0.0);
    let mut sh = sheet_view::McSheet {
        c: &mut c,
        rows: [&air, &tele, &ap]
            .into_iter()
            .cloned()
            .map(sheet_view::Row::It)
            .collect(),
        cols: vec![None, Some("A".into()), Some("B".into())],
        now: &none,
        cache: &cache,
        stops: &[],
        acts: Default::default(),
    };
    assert_eq!(sh.cols(), 4);
    assert!(sh.cell(0, 0).ro.is_some());
    assert!(matches!(sh.cell(0, 2).kind, Kind::Num { .. }));
    assert_eq!(sh.cell(1, 2).kind, Kind::Bool);
    assert!(matches!(sh.cell(2, 2).kind, Kind::List { .. }));
    sh.set(0, 2, "4").unwrap();
    assert!(sh.cell(0, 2).mark);
    assert!(sh.set(0, 2, "x").is_err());
    assert!(sh.set(2, 2, "nope").is_err());
    sh.set(2, 2, "fnum").unwrap();
    // ピックアップのセルは書けない
    assert!(sh.cell(0, 3).ro.is_some());
    // 空にすると基本の値
    sh.set(0, 2, "").unwrap();
    assert!(!sh.cell(0, 2).mark);
    assert!(sh.swap(0));
    assert!(matches!(sh.acts.borrow()[0], sheet_view::Act::Swap(0)));
    assert!(sh.c.state_sys["A"].aperture == ApertureKind::FNum);
}

/// 表を描いて、セルを選んで Alt+↑ で 1 増やす（表の部品のキーが状態の値に届く）
#[test]
fn sheet_keys_reach_state_values() {
    let _g = crate::serve_lock();
    let mut c = Combo {
        states: vec!["A".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    c.picks[0].state_air.insert("A".into(), 7.0);
    let mut view = View::default();
    let none = |_: &Face| None;
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let mut frame =
        |c: &mut Combo, view: &mut View, events: Vec<egui::Event>, mods: egui::Modifiers| {
            t += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 500.0),
                )),
                time: Some(t),
                events,
                modifiers: mods,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    super::ui(ui, c, view, &none, &[]);
                });
            });
        };
    frame(&mut c, &mut view, vec![], Default::default());
    frame(&mut c, &mut view, vec![], Default::default());
    // 空気の行の A の列（行 1、列 3: 値の名前・基本・A）のセルを選ぶ
    view.sheet.select(0, 2);
    let key = |k: egui::Key, m: egui::Modifiers| egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: m,
    };
    frame(
        &mut c,
        &mut view,
        vec![key(egui::Key::ArrowUp, egui::Modifiers::ALT)],
        egui::Modifiers::ALT,
    );
    frame(&mut c, &mut view, vec![], Default::default());
    assert_eq!(c.picks[0].air_in(Some("A")), 8.0);
}

/// 別の行からのピックアップ（OpticStudio の構成のピックアップの行）: 同じ構成の同じ要素の行・組合せの行（系の値）を元にでき、
/// 面の番号を付け替えると元の面の行も付いていく。ほかの要素の行は元にできない
#[test]
fn pickups_from_another_row() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    c.picks.push(Pick::new(ElemKind::Plate));
    let now = |f: &Face| (f.field == "t").then(|| json!(3.0));
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    let face = |surf: usize, field: &str| {
        Item::Face(Face {
            loc: vec![0],
            surf,
            field: field.into(),
            idx: None,
        })
    };
    let (t0, air) = (face(0, "t"), Item::Air(vec![0]));
    let w = Item::Sys(SysQ::Wave(0));
    assert_eq!(picks::row_key(&t0, &air).as_deref(), Some("air"));
    assert_eq!(picks::row_key(&t0, &w).as_deref(), Some("sys:wave:0"));
    assert_eq!(picks::row_key(&t0, &t0).as_deref(), Some(""));
    assert_eq!(picks::row_key(&t0, &Item::Air(vec![1])), None);
    // 構成 A の厚さ = 構成 A の後の空気 × 0.5
    air.set(&mut c, Some("A"), Val::Num(4.0), &cx);
    let src = picks::Src {
        from: "A".into(),
        row: "air".into(),
    };
    picks::set_pick(&mut c, &t0, "A", &src, 0.5, 0.0);
    // 構成 B の後の空気 = 構成 B の波長 1 × 10
    let wb = w.get(&c, Some("B"), &cx);
    let Some(Val::Num(um)) = wb else {
        panic!("{wb:?}")
    };
    let src = picks::Src {
        from: "B".into(),
        row: "sys:wave:0".into(),
    };
    picks::set_pick(&mut c, &air, "B", &src, 10.0, 0.0);
    assert!(picks::apply(&mut c, &cx));
    assert_eq!(c.picks[0].state_edit("A", 0, "t"), Some(&json!(2.0)));
    assert_eq!(c.picks[0].air_in(Some("B")), um * 10.0);
    // 保存して戻しても同じ
    let j = picks::to_json(&c).unwrap();
    let mut d = c.clone();
    d.picks[0].mc_picks.clear();
    picks::from_json(&mut d, Some(&j));
    assert_eq!(d, c);
    // 面の番号の付け替え: 先も元も付いていき、元の面が無くなればピックアップを外す
    let k1 = face(1, "k");
    let src = picks::Src {
        from: "A".into(),
        row: "face:0:t".into(),
    };
    picks::set_pick(&mut c, &k1, "B", &src, 1.0, 0.0);
    c.picks[0].shift_face_picks(&|i| Some(i + 1));
    let m = picks::pick_of(&c, &face(2, "k"), "B").unwrap();
    assert_eq!(m.row, "face:1:t");
    assert!(picks::pick_of(&c, &face(1, "t"), "A").is_some());
    c.picks[0].shift_face_picks(&|i| (i != 1).then_some(i));
    assert!(picks::pick_of(&c, &face(2, "k"), "B").is_none());
    assert_eq!(c.picks[0].mc_picks.len(), 1, "{:?}", c.picks[0].mc_picks);
}

/// 行・「+ 行」の欄の要素の名前はプロジェクトの id を頭に付けて見分けられる
#[test]
fn element_names_have_ids() {
    let mut c = Combo::default();
    c.picks.push(Pick::new(ElemKind::Lens));
    c.picks.push(Pick::new(ElemKind::Lens));
    let (a, b) = (el_name(&c, &vec![0]), el_name(&c, &vec![1]));
    assert_ne!(a, b);
    let ids = c.ids_all();
    assert!(
        a.starts_with(&ids[0].1) && b.starts_with(&ids[1].1),
        "{a} {b}"
    );
}

/// 輪になるピックアップ（構成 A の厚さ ← A の空気 × 2、A の空気 ← A の厚さ × 2 など）は元に選べず、前の版で作った輪は
/// 当てない（当てるたびに値が増えない）
#[test]
fn looped_pickups_are_not_applied() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.picks.push(Pick::new(ElemKind::Plate));
    let now = |f: &Face| (f.field == "t").then(|| json!(3.0));
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    let t0 = Item::Face(Face {
        loc: vec![0],
        surf: 0,
        field: "t".into(),
        idx: None,
    });
    let air = Item::Air(vec![0]);
    air.set(&mut c, Some("A"), Val::Num(4.0), &cx);
    let src = picks::Src {
        from: "A".into(),
        row: "air".into(),
    };
    picks::set_pick(&mut c, &t0, "A", &src, 2.0, 0.0);
    // 自分自身・逆向きは輪
    assert!(picks::would_loop(&c, &t0, "A", &t0, "A"));
    assert!(picks::would_loop(&c, &air, "A", &t0, "A"));
    assert!(!picks::would_loop(&c, &air, "B", &t0, "A"));
    assert!(!picks::would_loop(&c, &air, "A", &t0, ""));
    // 前の版で作った輪（直に書く）は当てない
    let src = picks::Src {
        from: "A".into(),
        row: "face:0:t".into(),
    };
    picks::set_pick(&mut c, &air, "A", &src, 2.0, 0.0);
    picks::apply(&mut c, &cx);
    let a = c.picks[0].air_in(Some("A"));
    picks::apply(&mut c, &cx);
    assert_eq!(c.picks[0].air_in(Some("A")), a);
    assert_eq!(a, 4.0);
}

/// コメント行（MCOM）: 構成ごとに文を持ち、基本と同じ文は状態の値にしない。構成の名前を変える・写す・消すと付いていき、
/// 行を消すと後ろのコメント行のピックアップの番号を詰める
#[test]
fn comment_rows() {
    let mut c = Combo {
        states: vec!["A".into(), "B".into()],
        ..Default::default()
    };
    c.mc_comments.push(Default::default());
    c.mc_comments.push(Default::default());
    let now = |_: &Face| None;
    let cache = HashMap::new();
    let cx = Cx {
        now: &now,
        cache: &cache,
    };
    let (a, b) = (Item::Comment(0), Item::Comment(1));
    assert!(items(&c, false, &[]).contains(&a) && items(&c, false, &[]).contains(&b));
    b.set(&mut c, None, Val::Text("note".into()), &cx);
    b.set(&mut c, Some("A"), Val::Text("wide".into()), &cx);
    b.set(&mut c, Some("B"), Val::Text("note".into()), &cx);
    assert_eq!(b.get(&c, Some("A"), &cx), Some(Val::Text("wide".into())));
    assert_eq!(b.get(&c, Some("B"), &cx), Some(Val::Text("note".into())));
    assert!(b.own(&c, "A") && !b.own(&c, "B"));
    c.rename_state("A", "W").unwrap();
    assert_eq!(c.mc_comments[1].values["W"], "wide");
    c.add_state("W2", Some("W"));
    assert_eq!(c.mc_comments[1].values["W2"], "wide");
    c.remove_state("W2");
    assert!(!c.mc_comments[1].values.contains_key("W2"));
    // コメント行は別の行の元にならない
    assert_eq!(picks::row_key(&a, &Item::Stop), None);
    // 行を消すと詰まる（ピックアップも）
    picks::set_pick(&mut c, &b, "B", &picks::Src::conf("W"), 1.0, 0.0);
    let mut view = View::default();
    remove_row(&mut c, &mut view, &a);
    assert_eq!(c.mc_comments.len(), 1);
    assert_eq!(c.mc_comments[0].values["W"], "wide");
    assert!(picks::pick_of(&c, &Item::Comment(0), "B").is_some());
    let (loc, key) = picks::key_of(&Item::Comment(0));
    assert_eq!(picks::item_of(loc.as_ref(), &key), Some(Item::Comment(0)));
}
