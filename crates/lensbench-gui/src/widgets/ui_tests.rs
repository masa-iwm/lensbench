//! widgets の部品を egui の 1 フレームずつ回して確かめる（ポインタ・キーを入れる）

use std::cell::Cell;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Ui, Vec2};

/// 部品を回す小さな台（画面 600x400、時刻はフレームごとに進める）
struct Bench {
    ctx: egui::Context,
    t: f64,
}

impl Bench {
    fn new() -> Self {
        Bench {
            ctx: egui::Context::default(),
            t: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>, mut body: impl FnMut(&mut Ui)) {
        self.t += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| body(ui));
        });
    }

    /// p でクリックする 3 フレーム（動かす・押す・離す）。各フレームで body を回す
    fn click(&mut self, p: Pos2, button: PointerButton, mut body: impl FnMut(&mut Ui)) {
        self.t += 1.0;
        self.frame(vec![Event::PointerMoved(p)], &mut body);
        for pressed in [true, false] {
            self.frame(
                vec![Event::PointerButton {
                    pos: p,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                }],
                &mut body,
            );
        }
    }
}

fn key(k: Key) -> Event {
    Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// 候補の一覧: 行のクリックはその番号、ポインタが一覧の上にあれば hovered
#[test]
fn choices_click_and_hover() {
    let items: Vec<(String, String)> = (0..50)
        .map(|i| (format!("G{i}"), format!("glass {i}")))
        .collect();
    let mut b = Bench::new();
    let anchor = Rect::from_min_size(Pos2::new(20.0, 20.0), egui::vec2(100.0, 20.0));
    let shown = Cell::new(super::choices::Shown::default());
    let body = |ui: &mut Ui| {
        shown.set(super::choices::Choices::new(&items, 0).show(ui, egui::Id::new("c"), anchor));
    };
    b.frame(Vec::new(), &body);
    // 一覧は欄の下に出る。2 行目の真ん中（行の高さは interact_size.y の下限）
    let row_h = b.ctx.style().spacing.interact_size.y;
    let p = Pos2::new(60.0, anchor.bottom() + 8.0 + row_h * 1.5);
    b.click(p, PointerButton::Primary, &body);
    // 離したフレームでクリック
    let s = shown.get();
    assert_eq!(s.clicked, Some(1), "{s:?}");
    assert!(s.hovered);
    // 外に動かすと hovered でない
    b.frame(vec![Event::PointerMoved(Pos2::new(500.0, 380.0))], &body);
    assert!(!shown.get().hovered);
    assert_eq!(shown.get().clicked, None);
}

/// 候補が無ければ empty の文だけ。empty も無ければ何も出さない
#[test]
fn choices_empty() {
    let mut b = Bench::new();
    let anchor = Rect::from_min_size(Pos2::new(20.0, 20.0), egui::vec2(100.0, 20.0));
    let mut shown = super::choices::Shown::default();
    b.frame(vec![Event::PointerMoved(Pos2::new(30.0, 52.0))], |ui| {
        shown = super::choices::Choices::new(&[], 0).show(ui, egui::Id::new("c"), anchor);
    });
    assert!(!shown.hovered);
    // 出した最初のフレームは大きさを測るだけなので 2 フレーム
    for _ in 0..2 {
        b.frame(vec![Event::PointerMoved(Pos2::new(30.0, 48.0))], |ui| {
            shown = super::choices::Choices {
                empty: Some("none"),
                ..super::choices::Choices::new(&[], 0)
            }
            .show(ui, egui::Id::new("e"), anchor);
        });
    }
    assert!(shown.hovered);
}

/// Shift+F10 の欄: 開いたフレームの外のクリックでは閉じず、Esc・外のクリック・選んだときに閉じる
#[test]
fn float_menu_closes() {
    let mut b = Bench::new();
    let pos = Pos2::new(100.0, 100.0);
    let mut close = false;
    let run = |b: &mut Bench, events: Vec<Event>, fresh: bool, pick: bool| {
        let mut close = false;
        b.frame(events, |ui| {
            close = super::menu::float_menu(ui, egui::Id::new("m"), pos, 200.0, fresh, |ui| {
                ui.label("item");
                pick
            });
        });
        close
    };
    assert!(!run(&mut b, Vec::new(), true, false));
    assert!(!run(&mut b, Vec::new(), false, false));
    assert!(run(&mut b, vec![key(Key::Escape)], false, false));
    assert!(run(&mut b, Vec::new(), false, true));
    // 外のクリック（開いたフレームは数えない）
    b.click(Pos2::new(500.0, 350.0), PointerButton::Primary, |ui| {
        close |= super::menu::float_menu(ui, egui::Id::new("m"), pos, 200.0, false, |ui| {
            ui.label("item");
            false
        });
    });
    assert!(close);
}

/// 図の操作: ホイールはポインタの所で拡大して外のスクロールに渡さない、中・右ドラッグは移動、左は left_pans のときだけ
#[test]
fn pan_zoom_reads_input() {
    let mut b = Bench::new();
    let pz = Cell::new(None);
    let left_pans = Cell::new(false);
    let rest = Cell::new(Vec2::ZERO);
    let body = |ui: &mut Ui| {
        let (_, resp) = ui.allocate_exact_size(egui::vec2(300.0, 300.0), Sense::click_and_drag());
        pz.set(Some(super::pan_zoom(ui, &resp, left_pans.get())));
        rest.set(ui.input(|i| i.smooth_scroll_delta));
    };
    let at = Pos2::new(100.0, 100.0);
    b.frame(vec![Event::PointerMoved(at)], &body);
    // ホイール（なめらかなスクロールは数フレームに分かれて届く）
    b.frame(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 120.0),
            modifiers: Modifiers::NONE,
        }],
        &body,
    );
    let mut zoomed = false;
    for _ in 0..10 {
        let z = pz.get().unwrap();
        if z.zoom > 1.0 {
            zoomed = true;
            assert_eq!(z.at, Some(at));
        }
        assert_eq!(rest.get(), Vec2::ZERO, "ホイールを外へ渡さない");
        b.frame(Vec::new(), &body);
    }
    assert!(zoomed);
    // ドラッグ: ボタンごと
    for (button, left, pans) in [
        (PointerButton::Middle, false, true),
        (PointerButton::Secondary, false, true),
        (PointerButton::Primary, false, false),
        (PointerButton::Primary, true, true),
    ] {
        left_pans.set(left);
        b.t += 1.0;
        b.frame(vec![Event::PointerMoved(at)], &body);
        b.frame(
            vec![Event::PointerButton {
                pos: at,
                button,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            &body,
        );
        let mut moved = Vec2::ZERO;
        for k in 1..=4 {
            b.frame(
                vec![Event::PointerMoved(at + Vec2::new(10.0 * k as f32, 0.0))],
                &body,
            );
            moved += pz.get().unwrap().pan;
        }
        b.frame(
            vec![Event::PointerButton {
                pos: at + Vec2::new(40.0, 0.0),
                button,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            &body,
        );
        assert_eq!(moved.x > 0.0, pans, "{button:?} left_pans={left}");
    }
    // Home は図の上にあるとき全体表示
    b.frame(vec![key(Key::Home)], &body);
    assert!(pz.get().unwrap().reset);
}
