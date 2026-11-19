//! 表の部品を egui の 1 フレームずつ回して確かめる（キー・クリック・ドラッグ）

use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

/// 2 列（文字・数）の表。.1 なら最後の空の行を出し、列を見出しのドラッグで動かせる
#[derive(Default)]
struct Rows(Vec<[String; 2]>, bool);

impl Sheet for Rows {
    fn rows(&self) -> usize {
        self.0.len()
    }
    fn cols(&self) -> usize {
        2
    }
    fn header(&self, col: usize, _cur: Option<usize>) -> String {
        ["Name", "Value"][col].into()
    }
    fn cell(&self, row: usize, col: usize) -> Cell {
        let t = self.0[row][col].clone();
        if col == 1 {
            Cell::num(t)
        } else {
            Cell::text(t)
        }
    }
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String> {
        if col == 1 && !text.is_empty() && text.parse::<f64>().is_err() {
            return Err("not a number".into());
        }
        self.0[row][col] = text.into();
        Ok(())
    }
    fn insert(&mut self, at: usize) -> bool {
        self.0.insert(at, Default::default());
        true
    }
    fn remove(&mut self, rows: RangeInclusive<usize>) {
        self.0.drain(rows);
    }
    fn swap(&mut self, a: usize) -> bool {
        self.0.swap(a, a + 1);
        true
    }
    fn new_row(&self, _full: bool) -> Option<Cell> {
        self.1.then(|| Cell::text(""))
    }
    fn can_move_col(&self, _col: usize) -> bool {
        self.1
    }
    fn move_col(&mut self, from: usize, to: usize) -> bool {
        for r in &mut self.0 {
            r.swap(from, to);
        }
        from != to
    }
}

struct Bench {
    ctx: egui::Context,
    t: f64,
    st: SheetState,
    rows: Rows,
    changed: bool,
}

impl Bench {
    fn new(n: usize) -> Bench {
        Bench {
            ctx: egui::Context::default(),
            t: 0.0,
            st: SheetState::default(),
            rows: Rows(
                (0..n)
                    .map(|i| [format!("v{i}"), format!("{}", i as f64 + 0.5)])
                    .collect(),
                false,
            ),
            changed: false,
        }
    }

    fn frame_mods(&mut self, events: Vec<Event>, mods: Modifiers) {
        self.t += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
            time: Some(self.t),
            events,
            modifiers: mods,
            ..Default::default()
        };
        let (st, rows, ch) = (&mut self.st, &mut self.rows, &mut self.changed);
        let _ = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                *ch |= show(ui, "t", st, rows);
            });
        });
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.frame_mods(events, Modifiers::NONE);
    }

    /// セルを AccessKit の名前で探した中心
    fn cell_pos(&self, r: usize, col: &str) -> Pos2 {
        // 列の位置: 番号の列 26 と各列の初めの幅 70（間の余白は小さいので真ん中を狙う）
        let x = 8.0 + 26.0 + if col == "Name" { 35.0 } else { 70.0 + 45.0 };
        let y = 8.0 + ROW_H * (r as f32 + 1.5);
        Pos2::new(x, y)
    }

    fn click(&mut self, p: Pos2) {
        self.t += 1.0;
        self.frame(vec![Event::PointerMoved(p)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
        self.frame(vec![]);
    }

    fn key_mods(&mut self, k: Key, mods: Modifiers) {
        self.frame_mods(
            vec![Event::Key {
                key: k,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            mods,
        );
        self.frame(vec![]);
    }

    fn key(&mut self, k: Key) {
        self.key_mods(k, Modifiers::NONE);
    }

    fn text(&mut self, t: &str) {
        self.frame(vec![Event::Text(t.into())]);
        self.frame(vec![]);
    }
}

/// クリックで選び、矢印で移る
#[test]
fn click_and_arrows() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Name"));
    assert_eq!(b.st.cur(), Some((0, 0)));
    b.key(Key::ArrowDown);
    b.key(Key::ArrowRight);
    assert_eq!(b.st.cur(), Some((1, 1)));
    // Tab は行の端で次の行へ
    b.key(Key::Tab);
    assert_eq!(b.st.cur(), Some((2, 0)));
}

/// 打った文字から書き始め、Enter で確定（数でなければ理由を出して書き換えを続ける）
#[test]
fn type_and_commit() {
    let mut b = Bench::new(2);
    b.frame(vec![]);
    b.click(b.cell_pos(1, "Value"));
    b.text("7");
    assert!(b.st.editing());
    b.text(".25");
    b.key(Key::Enter);
    assert!(!b.st.editing());
    assert_eq!(b.rows.0[1][1], "7.25");
    assert!(b.changed);
    // 誤り
    b.text("x");
    b.key(Key::Enter);
    assert!(b.st.editing());
    assert_eq!(b.rows.0[1][1], "7.25");
    b.key(Key::Escape);
    assert!(!b.st.editing());
}

/// ↓ で確定して下のセルへ
#[test]
fn arrow_commits_and_moves() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Name"));
    b.key(Key::F2);
    assert!(b.st.editing());
    b.text("z");
    b.key(Key::ArrowDown);
    assert!(!b.st.editing());
    assert_eq!(b.st.cur(), Some((1, 0)));
}

/// Delete で空に、Insert で行を足し、Ctrl+Delete で消す
#[test]
fn delete_insert_remove() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(1, "Name"));
    b.key(Key::Delete);
    assert_eq!(b.rows.0[1][0], "");
    b.key(Key::Insert);
    assert_eq!(b.rows.0.len(), 4);
    assert_eq!(b.rows.0[1], [String::new(), String::new()]);
    b.key_mods(Key::Delete, Modifiers::COMMAND);
    assert_eq!(b.rows.0.len(), 3);
    assert_eq!(b.rows.0[2][0], "v2");
}

/// Alt+↑↓ で数を 1 ずつ（Shift で 10）
#[test]
fn alt_arrows_nudge() {
    let mut b = Bench::new(2);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Value"));
    b.key_mods(Key::ArrowUp, Modifiers::ALT);
    assert_eq!(b.rows.0[0][1], "1.5");
    b.key_mods(
        Key::ArrowDown,
        Modifiers {
            alt: true,
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(b.rows.0[0][1], "-8.5");
    assert_eq!(b.st.cur(), Some((0, 1)));
}

/// 貼り付けはタブ区切りで右下へ、足りない行は足す
#[test]
fn paste_grows() {
    let mut b = Bench::new(1);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Name"));
    b.frame(vec![Event::Paste("a\t1\nb\t2\n".into())]);
    b.frame(vec![]);
    assert_eq!(b.rows.0.len(), 2);
    assert_eq!(b.rows.0[1], ["b".to_string(), "2".to_string()]);
}

/// Alt+ドラッグで数が変わる
#[test]
fn alt_drag_changes_number() {
    let mut b = Bench::new(1);
    b.frame(vec![]);
    let p = b.cell_pos(0, "Value");
    let alt = Modifiers::ALT;
    b.frame_mods(vec![Event::PointerMoved(p)], alt);
    b.frame_mods(
        vec![Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: alt,
        }],
        alt,
    );
    for k in 1..=5 {
        b.frame_mods(
            vec![Event::PointerMoved(p + egui::vec2(10.0 * k as f32, 0.0))],
            alt,
        );
    }
    b.frame_mods(
        vec![Event::PointerButton {
            pos: p + egui::vec2(50.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: alt,
        }],
        alt,
    );
    let v: f64 = b.rows.0[0][1].parse().unwrap();
    assert!(v > 0.5, "{v}");
}

#[test]
fn drag_steps() {
    assert_eq!(drag_value(25.4, 10.0, false), 26.4);
    assert_eq!(drag_value(0.0, 3.0, false), 0.03);
    assert_eq!(drag_value(3.0, 25.0, true), 6.0);
    assert_eq!(drag_value(1.0, -100.0, true), 0.0);
    assert_eq!(nudge(0.1, 0.2, false), 0.3);
}

#[test]
fn filter_puts_prefix_first() {
    let items: Vec<(String, String)> = ["trax", "x", "tanx"]
        .iter()
        .map(|s| (s.to_string(), String::new()))
        .collect();
    let f = filter_items(&items, "t");
    assert_eq!(f[0].0, "trax");
    assert_eq!(f[1].0, "tanx");
    assert_eq!(f.len(), 2);
}

/// Ctrl+D で今の行の写しを下に挿入する
#[test]
fn duplicate_row() {
    let mut b = Bench::new(2);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Value"));
    b.key_mods(Key::D, Modifiers::COMMAND);
    assert_eq!(b.rows.0.len(), 3);
    assert_eq!(b.rows.0[1], b.rows.0[0]);
    assert_eq!(b.rows.0[2][0], "v1");
    assert_eq!(b.st.cur(), Some((1, 1)));
}

/// Ctrl+Shift+↑↓ で今の行を上下へ動かす（今のセルも付いていく）
#[test]
fn move_rows() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Name"));
    let cs = Modifiers {
        command: true,
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    b.key_mods(Key::ArrowDown, cs);
    assert_eq!(b.rows.0[1][0], "v0");
    assert_eq!(b.st.cur(), Some((1, 0)));
    b.key_mods(Key::ArrowDown, cs);
    assert_eq!(b.rows.0[2][0], "v0");
    // 最後の行はそれ以上動かない
    b.key_mods(Key::ArrowDown, cs);
    assert_eq!(b.rows.0[2][0], "v0");
    b.key_mods(Key::ArrowUp, cs);
    assert_eq!(b.rows.0[1][0], "v0");
}

/// Shift+矢印で四角に広げ、コピーは範囲、Delete は範囲を空に
#[test]
fn range_select_copy_delete() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Name"));
    let sh = Modifiers::SHIFT;
    b.key_mods(Key::ArrowDown, sh);
    b.key_mods(Key::ArrowRight, sh);
    assert_eq!(b.st.selection(), Some((0..=1, 0..=1)));
    b.key(Key::Delete);
    assert_eq!(b.rows.0[0], [String::new(), String::new()]);
    assert_eq!(b.rows.0[1], [String::new(), String::new()]);
    assert_eq!(b.rows.0[2][0], "v2");
}

/// 1 つの値を範囲に貼ると範囲を埋める
#[test]
fn paste_fills_range() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    b.click(b.cell_pos(0, "Value"));
    b.key_mods(Key::ArrowDown, Modifiers::SHIFT);
    b.key_mods(Key::ArrowDown, Modifiers::SHIFT);
    b.frame(vec![Event::Paste("9\n".into())]);
    b.frame(vec![]);
    for r in 0..3 {
        assert_eq!(b.rows.0[r][1], "9");
    }
    assert_eq!(b.rows.0[1][0], "v1");
}

/// 行の番号で行全体、列の見出しで列全体、Ctrl+A で表全体
#[test]
fn select_rows_cols_all() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    let num = |r: usize| Pos2::new(8.0 + 10.0, 8.0 + ROW_H * (r as f32 + 1.5));
    b.click(num(1));
    assert_eq!(b.st.selection(), Some((1..=1, 0..=1)));
    let head = Pos2::new(8.0 + 26.0 + 70.0 + 45.0, 8.0 + ROW_H * 0.5);
    b.click(head);
    assert_eq!(b.st.selection(), Some((0..=2, 1..=1)));
    b.key_mods(Key::A, Modifiers::COMMAND);
    assert_eq!(b.st.selection(), Some((0..=2, 0..=1)));
    // 表全体を選んで Ctrl+Delete で全部の行を消す
    b.key_mods(Key::Delete, Modifiers::COMMAND);
    assert!(b.rows.0.is_empty());
}

#[test]
fn completion_is_prefix() {
    let items: Vec<(String, String)> = ["trax", "tray", "x"]
        .iter()
        .map(|s| (s.to_string(), String::new()))
        .collect();
    assert_eq!(completion(&items, "TR").as_deref(), Some("trax"));
    assert_eq!(completion(&items, "x"), None);
    assert_eq!(completion(&items, ""), None);
    // 同じ名前の候補があれば伸ばさない
    let items: Vec<(String, String)> = ["lens10", "lens1"]
        .iter()
        .map(|s| (s.to_string(), String::new()))
        .collect();
    assert_eq!(completion(&items, "LENS1"), None);
    assert_eq!(completion(&items, "len").as_deref(), Some("lens10"));
}

/// 候補の 1 列の表
struct Pick(Vec<String>);

impl Sheet for Pick {
    fn rows(&self) -> usize {
        self.0.len()
    }
    fn cols(&self) -> usize {
        1
    }
    fn header(&self, _col: usize, _cur: Option<usize>) -> String {
        "Lens".into()
    }
    fn cell(&self, row: usize, _col: usize) -> Cell {
        let items = ["lens10", "lens1", "trax"]
            .iter()
            .map(|s| (s.to_string(), String::new()))
            .collect();
        Cell::list(self.0[row].clone(), items, true)
    }
    fn set(&mut self, row: usize, _col: usize, text: &str) -> Result<(), String> {
        self.0[row] = text.into();
        Ok(())
    }
}

/// 打つたびに補い（1 文字おきにならない）、同じ名前の候補があれば長い候補に伸ばさない
#[test]
fn typing_completes_each_char() {
    let ctx = egui::Context::default();
    let mut st = SheetState::default();
    let mut sh = Pick(vec![String::new()]);
    st.select(0, 0);
    let mut t = 0.0;
    let mut run = |st: &mut SheetState, sh: &mut Pick, events: Vec<Event>| {
        t += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, "p", st, sh);
            });
        });
    };
    let key = |k: Key| Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    run(&mut st, &mut sh, vec![]);
    for (typed, want) in [("t", "trax"), ("r", "trax")] {
        run(&mut st, &mut sh, vec![Event::Text(typed.into())]);
        run(&mut st, &mut sh, vec![]);
        assert_eq!(st.edit.as_ref().unwrap().text, want, "{typed}");
    }
    run(&mut st, &mut sh, vec![key(Key::Escape)]);
    run(&mut st, &mut sh, vec![]);
    for c in ["l", "e", "n", "s", "1"] {
        run(&mut st, &mut sh, vec![Event::Text(c.into())]);
        run(&mut st, &mut sh, vec![]);
    }
    run(&mut st, &mut sh, vec![key(Key::Enter)]);
    run(&mut st, &mut sh, vec![]);
    assert_eq!(sh.0[0], "lens1");
}

/// Alt なしのドラッグは範囲を選ぶ（数は変えない）
#[test]
fn drag_selects_range() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    let (p, q) = (b.cell_pos(0, "Name"), b.cell_pos(2, "Value"));
    b.frame(vec![Event::PointerMoved(p)]);
    b.frame(vec![Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        b.frame(vec![Event::PointerMoved(p + (q - p) * t)]);
    }
    b.frame(vec![Event::PointerButton {
        pos: q,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    b.frame(vec![]);
    assert_eq!(b.st.selection(), Some((0..=2, 0..=1)));
    assert_eq!(b.rows.0[2][1], "2.5");
    assert!(!b.changed);
}

/// 候補のあるセルの右の ▼ を押すと候補の一覧で書き始める
#[test]
fn arrow_opens_choices() {
    let ctx = egui::Context::default();
    let mut st = SheetState::default();
    let mut sh = Pick(vec!["trax".into()]);
    let mut t = 0.0;
    let mut run = |st: &mut SheetState, sh: &mut Pick, events: Vec<Event>| {
        t += 0.3;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, "p", st, sh);
            });
        });
    };
    run(&mut st, &mut sh, vec![]);
    // 列の初めの幅 70 の右端の少し内側
    let p = Pos2::new(8.0 + 26.0 + 70.0 - 6.0, 8.0 + ROW_H * 1.5);
    run(&mut st, &mut sh, vec![Event::PointerMoved(p)]);
    for pressed in [true, false] {
        run(
            &mut st,
            &mut sh,
            vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    run(&mut st, &mut sh, vec![]);
    assert!(st.editing());
    assert_eq!(st.edit.as_ref().unwrap().text, "trax");
}

/// 列だけを選んだときの行の操作は今の行だけ（全部の行を消さない）
#[test]
fn column_selection_row_ops_use_current_row() {
    let mut b = Bench::new(3);
    b.frame(vec![]);
    let head = Pos2::new(8.0 + 26.0 + 70.0 + 45.0, 8.0 + ROW_H * 0.5);
    b.click(head);
    assert_eq!(b.st.selection(), Some((0..=2, 1..=1)));
    b.key_mods(Key::Delete, Modifiers::COMMAND);
    assert_eq!(b.rows.0.len(), 2);
    assert_eq!(b.rows.0[0][0], "v1");
}

/// 押して q まで動かして放す
fn drag(b: &mut Bench, p: Pos2, q: Pos2) {
    b.frame(vec![Event::PointerMoved(p)]);
    b.frame(vec![Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        b.frame(vec![Event::PointerMoved(p + (q - p) * t)]);
    }
    b.frame(vec![Event::PointerButton {
        pos: q,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    b.frame(vec![]);
}

/// 最後の行から ↓ で空の行へ、打って Enter でその値の行を足す（空のままの Enter は足さない）
#[test]
fn ghost_row_adds() {
    let mut b = Bench::new(2);
    b.rows.1 = true;
    b.frame(vec![]);
    b.click(b.cell_pos(1, "Name"));
    b.key(Key::ArrowDown);
    assert_eq!(b.st.cur(), Some((2, 0)));
    b.key(Key::ArrowRight);
    assert_eq!(b.st.cur(), Some((2, 0)));
    b.key(Key::Enter);
    assert!(b.st.editing());
    b.key(Key::Enter);
    assert!(!b.st.editing());
    assert_eq!(b.rows.0.len(), 2);
    b.text("w");
    b.key(Key::Enter);
    assert_eq!(b.rows.0.len(), 3);
    assert_eq!(b.rows.0[2][0], "w");
    assert!(b.changed);
    // 空の行はクリックでも選べ、↑ で表へ
    let mut b = Bench::new(1);
    b.rows.1 = true;
    b.frame(vec![]);
    b.click(b.cell_pos(1, "Name"));
    assert_eq!(b.st.cur(), Some((1, 0)));
    b.key(Key::ArrowUp);
    assert_eq!(b.st.cur(), Some((0, 0)));
}

/// 行の番号のドラッグで行を動かし、列の見出しのドラッグで列を動かす
#[test]
fn drag_headers_move() {
    let mut b = Bench::new(3);
    b.rows.1 = true;
    b.frame(vec![]);
    let num = |r: usize| Pos2::new(8.0 + 13.0, 8.0 + ROW_H * (r as f32 + 1.5));
    drag(&mut b, num(0), num(2));
    let names: Vec<&str> = b.rows.0.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(names, ["v1", "v2", "v0"]);
    assert_eq!(b.st.cur(), Some((2, 0)));
    // 上へ
    drag(&mut b, num(2), num(1));
    let names: Vec<&str> = b.rows.0.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(names, ["v1", "v0", "v2"]);
    let head = |c: &str| Pos2::new(b.cell_pos(0, c).x, 8.0 + ROW_H * 0.5);
    let (h0, h1) = (head("Name"), head("Value"));
    drag(&mut b, h0, h1);
    assert_eq!(b.rows.0[0][1], "v1");
    assert_eq!(b.st.selection(), Some((0..=2, 1..=1)));
}
