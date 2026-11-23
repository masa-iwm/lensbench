//! widgets の部品を egui の 1 フレームずつ回して確かめる（ポインタ・キーを入れる）

use std::cell::Cell;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Ui, Vec2};

/// 部品を回す小さな台（画面 600x400、時刻はフレームごとに進める）
struct Bench {
    ctx: egui::Context,
    t: f64,
    /// 押している修飾キー（RawInput に入れる）
    mods: Modifiers,
}

impl Bench {
    fn new() -> Self {
        Bench {
            ctx: egui::Context::default(),
            t: 0.0,
            mods: Modifiers::NONE,
        }
    }

    fn frame(&mut self, events: Vec<Event>, mut body: impl FnMut(&mut Ui)) {
        self.t += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
            time: Some(self.t),
            // egui 0.37 から修飾キーは RawInput の欄でなく出来事で渡す
            events: std::iter::once(egui::Event::ModifiersChanged(self.mods))
                .chain(events)
                .collect(),
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |root| {
            egui::CentralPanel::default().show(root, |ui| body(ui));
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
        shown.set(super::choices::Choices::new(&items, 0).show(ui, egui::Id::unique("c"), anchor));
    };
    b.frame(Vec::new(), &body);
    // 一覧は欄の下に出る。2 行目の真ん中（行の高さは interact_size.y の下限）
    let row_h = b.ctx.global_style().spacing.interact_size.y;
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
        shown = super::choices::Choices::new(&[], 0).show(ui, egui::Id::unique("c"), anchor);
    });
    assert!(!shown.hovered);
    // 出した最初のフレームは大きさを測るだけ、egui 0.37 はポインタの当たりを前のフレームの窓で決めるので 3 フレーム
    for _ in 0..3 {
        b.frame(vec![Event::PointerMoved(Pos2::new(30.0, 48.0))], |ui| {
            shown = super::choices::Choices {
                empty: Some("none"),
                ..super::choices::Choices::new(&[], 0)
            }
            .show(ui, egui::Id::unique("e"), anchor);
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
            close = super::menu::float_menu(ui, egui::Id::unique("m"), pos, 200.0, fresh, |ui| {
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
        close |= super::menu::float_menu(ui, egui::Id::unique("m"), pos, 200.0, false, |ui| {
            ui.label("item");
            false
        });
    });
    assert!(close);
}

/// 図の操作: Ctrl（⌘）＋ホイールはポインタの所で拡大して外のスクロールに渡さない（素のホイールは渡す）、中・右ドラッグは移動、左は left_pans のときだけ
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
    // 素のホイールは拡大せず外のスクロールに残す
    b.frame(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        }],
        &body,
    );
    let mut passed = false;
    for _ in 0..10 {
        assert_eq!(pz.get().unwrap().zoom, 1.0);
        passed |= rest.get() != Vec2::ZERO;
        b.frame(Vec::new(), &body);
    }
    assert!(passed, "素のホイールは外へ渡す");
    // Ctrl（⌘）＋ホイール
    b.mods = Modifiers::COMMAND;
    b.frame(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::COMMAND,
        }],
        &body,
    );
    b.mods = Modifiers::NONE;
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

/// 左ドラッグ（押す・4 回動かす・離す）。各フレームの pan_zoom の結果を返す
fn left_drag(
    b: &mut Bench,
    from: Pos2,
    to: Pos2,
    pz: &Cell<Option<super::PanZoom>>,
    body: &impl Fn(&mut Ui),
) -> Vec<super::PanZoom> {
    let mods = b.mods;
    let mut out = Vec::new();
    b.t += 1.0;
    b.frame(vec![Event::PointerMoved(from)], body);
    b.frame(
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        }],
        body,
    );
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        b.frame(vec![Event::PointerMoved(p)], body);
        out.push(pz.get().unwrap());
    }
    b.frame(
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: mods,
        }],
        body,
    );
    out.push(pz.get().unwrap());
    out
}

/// 囲んで拡大: Ctrl＋左ドラッグ・「囲んで拡大」の後の左ドラッグは、囲んだ所を図いっぱいに広げる（途中は移動しない）。
/// 右上の＋・－は図の中心で 1 段ずつ、「全体」は全体表示
#[test]
fn pan_zoom_box_and_buttons() {
    let mut b = Bench::new();
    let pz = Cell::new(None);
    let rect = Cell::new(Rect::NOTHING);
    let boxing = Cell::new(false);
    let body = |ui: &mut Ui| {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(300.0, 300.0), Sense::click_and_drag());
        rect.set(r);
        // 左ドラッグで移動する図でも、囲む間は移動しない
        pz.set(Some(super::pan_zoom(ui, &resp, true)));
        boxing.set(super::box_zooming(ui, &resp));
        super::paint_zoom_tools(ui, &resp);
    };
    b.frame(Vec::new(), &body);
    let r = rect.get();
    let c = r.center();
    let check_box = |frames: &[super::PanZoom], from: Pos2, to: Pos2| {
        for f in &frames[..frames.len() - 1] {
            assert_eq!(f.pan, Vec2::ZERO, "囲む間は移動しない");
            assert_eq!(f.zoom, 1.0);
        }
        let last = frames.last().unwrap();
        let bx = Rect::from_two_pos(from, to);
        let want = (r.width() / bx.width()).min(r.height() / bx.height());
        assert!((last.zoom - want).abs() < 1e-4, "{last:?}");
        assert_eq!(last.at, Some(c));
        assert!((last.pan - (c - bx.center())).length() < 1e-3, "{last:?}");
    };
    // Ctrl＋左ドラッグ
    let (from, to) = (
        r.min + Vec2::new(40.0, 100.0),
        r.min + Vec2::new(100.0, 140.0),
    );
    b.mods = Modifiers::COMMAND;
    let frames = left_drag(&mut b, from, to, &pz, &body);
    b.mods = Modifiers::NONE;
    check_box(&frames, from, to);
    assert!(!boxing.get());
    // 修飾キーなしの左ドラッグは移動（この図は left_pans）
    let frames = left_drag(&mut b, from, to, &pz, &body);
    assert!(frames.iter().map(|f| f.pan.x).sum::<f32>() > 0.0);
    assert!(frames.iter().all(|f| f.zoom == 1.0));
    // 右下の道具: ＋・－・囲んで拡大・全体（右下から 8 px 内側に、幅 22・22・22・44、間 2）
    let y = r.bottom() - 8.0 - 11.0;
    let x0 = r.right() - 8.0 - (22.0 * 3.0 + 44.0 + 6.0);
    let at = |i: usize| Pos2::new(x0 + 24.0 * i as f32 + 11.0, y);
    let clicked = |b: &mut Bench, p: Pos2| {
        let mut got = Vec::new();
        b.click(p, PointerButton::Primary, |ui| {
            body(ui);
            got.push(pz.get().unwrap());
        });
        got
    };
    let z = clicked(&mut b, at(0));
    assert!(
        z.iter()
            .any(|z| z.zoom == super::ZOOM_STEP && z.at == Some(c)),
        "{z:?}"
    );
    let z = clicked(&mut b, at(1));
    assert!(
        z.iter()
            .any(|z| (z.zoom - 1.0 / super::ZOOM_STEP).abs() < 1e-6 && z.at == Some(c)),
        "{z:?}"
    );
    let z = clicked(&mut b, Pos2::new(x0 + 72.0 + 22.0, y));
    assert!(z.iter().any(|z| z.reset), "{z:?}");
    // 「囲んで拡大」を押すと、次の左ドラッグ 1 回だけ囲む
    let _ = clicked(&mut b, at(2));
    let frames = left_drag(&mut b, from, to, &pz, &body);
    check_box(&frames, from, to);
    let frames = left_drag(&mut b, from, to, &pz, &body);
    assert!(
        frames.iter().map(|f| f.pan.x).sum::<f32>() > 0.0,
        "1 回で戻る"
    );
    // 小さすぎる枠（クリックに近い）は拡大しない
    b.mods = Modifiers::COMMAND;
    let frames = left_drag(&mut b, from, from + Vec2::new(10.0, 2.0), &pz, &body);
    b.mods = Modifiers::NONE;
    assert!(frames.iter().all(|f| f.zoom == 1.0 && f.pan == Vec2::ZERO));
    // 囲む途中の Esc はやめる。ボタンを離すまでの残りのドラッグも移動にしない
    b.mods = Modifiers::COMMAND;
    b.t += 1.0;
    b.frame(vec![Event::PointerMoved(from)], &body);
    let press = |pressed| Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::COMMAND,
    };
    b.frame(vec![press(true)], &body);
    b.frame(
        vec![Event::PointerMoved(from + Vec2::new(30.0, 20.0))],
        &body,
    );
    assert!(boxing.get());
    b.mods = Modifiers::NONE;
    b.frame(vec![key(Key::Escape)], &body);
    let mut rest = Vec::new();
    for k in 2..=4 {
        b.frame(
            vec![Event::PointerMoved(from + Vec2::new(30.0 * k as f32, 20.0))],
            &body,
        );
        rest.push(pz.get().unwrap());
    }
    b.frame(vec![press(false)], &body);
    rest.push(pz.get().unwrap());
    assert!(
        rest.iter()
            .all(|f| f.zoom == 1.0 && f.pan == Vec2::ZERO && f.area.is_none()),
        "{rest:?}"
    );
    assert!(!boxing.get());
}

/// 解析のグラフ（plot_tools::nav）: 枠の中のホイールは両方の軸、横軸・縦軸の目盛りの上のホイールはその軸だけ拡大し、
/// 右ドラッグは移動（囲んで拡大ではない）、ダブルクリックで全体に戻る
#[test]
fn plot_nav_axis_wheel_and_drag() {
    use egui_plot::{Line, Plot, PlotBounds};
    let mut b = Bench::new();
    let bounds = Cell::new(None::<PlotBounds>);
    let frame = Cell::new(Rect::NOTHING);
    let body = |ui: &mut Ui| {
        let plot = Plot::new("nav_test")
            .width(400.0)
            .height(300.0)
            .x_axis_label("x")
            .y_axis_label("y");
        let r = crate::plot_tools::nav(ui, plot, true, |p| {
            p.line(Line::new("", vec![[0.0, 0.0], [10.0, 5.0]]));
        });
        bounds.set(Some(*r.transform.bounds()));
        frame.set(*r.transform.frame());
    };
    b.frame(vec![], &body);
    b.frame(vec![], &body);
    let start = bounds.get().unwrap();
    let fr = frame.get();
    // Ctrl（⌘）＋ホイール（素のホイールはパネルのスクロールに残す）
    let wheel_with = |b: &mut Bench, at: Pos2, mods: Modifiers| {
        b.t += 1.0;
        b.mods = mods;
        b.frame(vec![Event::PointerMoved(at)], &body);
        b.frame(
            vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, 120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: mods,
            }],
            &body,
        );
        for _ in 0..12 {
            b.frame(vec![], &body);
        }
        b.mods = Modifiers::NONE;
        bounds.get().unwrap()
    };
    let plain = wheel_with(&mut b, fr.center(), Modifiers::NONE);
    assert_eq!(plain, start, "素のホイールでは拡大しない");
    let wheel = |b: &mut Bench, at: Pos2| wheel_with(b, at, Modifiers::COMMAND);
    // 目盛りの帯の奥（枠の端から 20）: egui_plot 0.38 は拡大の途中で目盛りの字の高さが変わり、枠の下端が一時 15 ほど下がる
    // 横軸の目盛りの上: 横だけ
    let x_only = wheel(&mut b, egui::pos2(fr.center().x, fr.bottom() + 20.0));
    assert!(x_only.width() < start.width() * 0.95, "{x_only:?}");
    assert!((x_only.height() - start.height()).abs() < 1e-9);
    // 縦軸の目盛りの上: 縦だけ
    let y_only = wheel(&mut b, egui::pos2(fr.left() - 20.0, fr.center().y));
    assert!((y_only.width() - x_only.width()).abs() < 1e-9);
    assert!(y_only.height() < x_only.height() * 0.95);
    // 枠の中: 両方
    let both = wheel(&mut b, fr.center());
    assert!(both.width() < y_only.width() * 0.95 && both.height() < y_only.height() * 0.95);
    // 右ドラッグは移動（範囲の幅は変わらない）
    let at = fr.center();
    b.t += 1.0;
    b.frame(vec![Event::PointerMoved(at)], &body);
    for (pressed, p) in [(true, at), (false, at + Vec2::new(60.0, 0.0))] {
        if !pressed {
            for k in 1..=4 {
                b.frame(
                    vec![Event::PointerMoved(at + Vec2::new(15.0 * k as f32, 0.0))],
                    &body,
                );
            }
        }
        b.frame(
            vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
            &body,
        );
    }
    b.frame(vec![], &body);
    let moved = bounds.get().unwrap();
    assert!((moved.width() - both.width()).abs() < 1e-6 * both.width());
    assert!(
        moved.min()[0] < both.min()[0],
        "右へドラッグすると左の値が見える"
    );
    // ダブルクリックで全体
    b.t += 1.0;
    for _ in 0..2 {
        for pressed in [true, false] {
            b.frame(
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }],
                &body,
            );
        }
    }
    b.frame(vec![], &body);
    b.frame(vec![], &body);
    let fit = bounds.get().unwrap();
    assert!(
        (fit.width() - start.width()).abs() < 1e-6 * start.width(),
        "{fit:?} {start:?}"
    );

    // 横軸の目盛りの帯の上のドラッグ: 横だけ動く
    let band = egui::pos2(fr.center().x, fr.bottom() + 20.0);
    b.t += 1.0;
    b.frame(vec![Event::PointerMoved(band)], &body);
    b.frame(
        vec![Event::PointerButton {
            pos: band,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        &body,
    );
    for k in 1..=4 {
        b.frame(
            vec![Event::PointerMoved(
                band + Vec2::new(10.0 * k as f32, -5.0 * k as f32),
            )],
            &body,
        );
    }
    b.frame(
        vec![Event::PointerButton {
            pos: band + Vec2::new(40.0, -20.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        &body,
    );
    b.frame(vec![], &body);
    let dragged = bounds.get().unwrap();
    assert!(dragged.min()[0] < fit.min()[0], "{dragged:?} {fit:?}");
    assert!(
        (dragged.min()[1] - fit.min()[1]).abs() < 1e-9,
        "縦は動かない"
    );
    assert!((dragged.width() - fit.width()).abs() < 1e-6 * fit.width());

    // Ctrl+左ドラッグで囲んだ所が範囲になる（縦横は別）
    let before = bounds.get().unwrap();
    b.mods = Modifiers::COMMAND;
    let (p0, p1) = (
        fr.center() - Vec2::new(80.0, 20.0),
        fr.center() + Vec2::new(40.0, 60.0),
    );
    let pz = Cell::new(None);
    let body2 = |ui: &mut Ui| {
        body(ui);
        pz.set(Some(super::PanZoom {
            zoom: 1.0,
            at: None,
            pan: Vec2::ZERO,
            reset: false,
            area: None,
        }));
    };
    left_drag(&mut b, p0, p1, &pz, &body2);
    b.mods = Modifiers::NONE;
    b.frame(vec![], &body);
    let boxed = bounds.get().unwrap();
    let rx = (p1.x - p0.x) as f64 / fr.width() as f64;
    let ry = (p1.y - p0.y) as f64 / fr.height() as f64;
    assert!(
        (boxed.width() / before.width() - rx).abs() < 0.02,
        "{boxed:?}"
    );
    assert!(
        (boxed.height() / before.height() - ry).abs() < 0.02,
        "{boxed:?}"
    );
}

/// 図・グラフの道具は右下（右上は egui_plot の凡例）
#[test]
fn zoom_tools_sit_bottom_right() {
    let r = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
    let t = super::tool_rects(r);
    assert!(!t.is_empty());
    assert!(t
        .iter()
        .all(|(_, b)| b.bottom() > 280.0 && b.right() <= r.right()));
}

/// パネルの数の欄（widgets::Num）: 書き換え中（フォーカスがある）だけ Alt+↑↓ で 0.1（Shift 1、Ctrl 0.01）、
/// PageUp/PageDown は 10 倍。ポインタが上にあるだけでは取らない（キーの行き先は 1 つ）。ドラッグは今の値の桁の
/// 1/100 ずつ（表のセルと同じ）
#[test]
fn num_field_keys_and_drag() {
    let mut b = Bench::new();
    let v = Cell::new(5.0_f64);
    let other = Cell::new(1.0_f64);
    let rect = Cell::new(Rect::NOTHING);
    let rect_b = Cell::new(Rect::NOTHING);
    let body = |ui: &mut Ui| {
        // 先に描く欄 B（ポインタを載せるだけ）と、書き換える欄
        let mut y = other.get();
        rect_b.set(ui.add(super::Num::new(&mut y).range(0.0..=100.0)).rect);
        other.set(y);
        let mut x = v.get();
        let r = ui.add(super::Num::new(&mut x).range(0.0..=100.0));
        rect.set(r.rect);
        v.set(x);
    };
    b.frame(Vec::new(), &body);
    let at = rect.get().center();
    let k = |key: Key, modifiers: Modifiers| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    // ポインタが上にあるだけでは取らない
    b.frame(vec![Event::PointerMoved(at)], &body);
    b.frame(vec![k(Key::ArrowUp, Modifiers::ALT)], &body);
    b.frame(vec![k(Key::PageUp, Modifiers::NONE)], &body);
    assert_eq!((v.get(), other.get()), (5.0, 1.0));
    // クリックで書き換えを始める。ポインタは先に描く欄 B の上へ動かしても、変わるのは書き換え中の欄だけ
    b.click(at, PointerButton::Primary, &body);
    b.frame(vec![Event::PointerMoved(rect_b.get().center())], &body);
    b.frame(vec![k(Key::ArrowUp, Modifiers::ALT)], &body);
    assert!((v.get() - 5.1).abs() < 1e-9, "{}", v.get());
    b.frame(
        vec![k(Key::ArrowDown, Modifiers::ALT | Modifiers::SHIFT)],
        &body,
    );
    assert!((v.get() - 4.1).abs() < 1e-9, "{}", v.get());
    b.frame(vec![k(Key::PageUp, Modifiers::NONE)], &body);
    assert!((v.get() - 5.1).abs() < 1e-9, "{}", v.get());
    b.frame(vec![k(Key::PageDown, Modifiers::COMMAND)], &body);
    assert!((v.get() - 5.0).abs() < 1e-9, "{}", v.get());
    assert_eq!(other.get(), 1.0);
    // 上限で止まる
    v.set(99.95);
    b.frame(vec![k(Key::PageUp, Modifiers::NONE)], &body);
    assert_eq!(v.get(), 100.0);
    b.frame(vec![k(Key::Escape, Modifiers::NONE)], &body);
    // ドラッグ: 5 の桁は 1 なので 1 点で 0.01
    v.set(5.0);
    b.t += 1.0;
    b.frame(vec![Event::PointerMoved(at)], &body);
    let press = |pressed, pos| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    b.frame(vec![press(true, at)], &body);
    for x in [10.0, 20.0] {
        b.frame(vec![Event::PointerMoved(at + Vec2::new(x, 0.0))], &body);
    }
    b.frame(vec![press(false, at + Vec2::new(20.0, 0.0))], &body);
    assert!((v.get() - 5.2).abs() < 0.011, "{}", v.get());
}

/// 名前の欄の補完: 打った文字で候補を絞り（前方一致を先に）、Tab で選んだ候補を入れる。ガラスの欄は候補が
/// 無ければ頼み、カタログに無い名前を見分ける（モデルガラス・媒質の屈折率は受ける）
#[test]
fn name_edit_completes_and_checks_glass() {
    use super::name_edit as ne;
    use crate::lens_data::GlassChoice;
    let items: Vec<String> = ["BAK1", "N-SF11", "N-BK7"].map(String::from).to_vec();
    let got: Vec<String> = ne::filter(&items, "n-").into_iter().map(|x| x.0).collect();
    assert_eq!(got, ["N-SF11", "N-BK7"]);
    let got: Vec<String> = ne::filter(&items, "bk").into_iter().map(|x| x.0).collect();
    assert_eq!(got, ["N-BK7"]);
    let mut b = Bench::new();
    let s = std::cell::RefCell::new(String::new());
    let rect = Cell::new(Rect::NOTHING);
    let changed = Cell::new(false);
    let body = |ui: &mut Ui| {
        let r = ne::name_edit(ui, &mut s.borrow_mut(), 80.0, "", &items, |_| true, "");
        rect.set(r.rect);
        changed.set(changed.get() || r.changed());
    };
    b.frame(Vec::new(), &body);
    b.click(rect.get().center(), PointerButton::Primary, &body);
    b.frame(vec![Event::Text("SF".into())], &body);
    b.frame(vec![key(Key::Tab)], &body);
    b.frame(Vec::new(), &body);
    assert_eq!(*s.borrow(), "N-SF11");
    assert!(changed.get());
    // Enter は打った名前のまま（候補に無い新しい名前も書ける）。↑↓ で選んだ後だけ候補を入れる
    for (text, keys, want) in [
        ("", vec![Key::Enter], ""),
        ("N-", vec![Key::Enter], "N-"),
        ("N-", vec![Key::ArrowDown, Key::Enter], "N-BK7"),
    ] {
        s.borrow_mut().clear();
        b.click(rect.get().center(), PointerButton::Primary, &body);
        if !text.is_empty() {
            b.frame(vec![Event::Text(text.into())], &body);
        }
        for k in keys {
            b.frame(vec![key(k)], &body);
        }
        b.frame(Vec::new(), &body);
        assert_eq!(*s.borrow(), want, "{text}");
    }
    // 候補のクリック: 押したフレームで欄がフォーカスを失い、離したフレームでクリックが立つ
    s.borrow_mut().clear();
    changed.set(false);
    b.click(rect.get().center(), PointerButton::Primary, &body);
    b.frame(vec![Event::Text("N-".into())], &body);
    b.frame(Vec::new(), &body);
    let row_h = b.ctx.global_style().spacing.interact_size.y;
    let p = Pos2::new(
        rect.get().left() + 30.0,
        rect.get().bottom() + 8.0 + row_h * 1.5,
    );
    b.click(p, PointerButton::Primary, &body);
    b.frame(Vec::new(), &body);
    assert_eq!(*s.borrow(), "N-BK7");
    assert!(changed.get());
    // 入れたら一覧は閉じる（ポインタが同じ所にあっても、もう一度押しても変わらない）
    b.click(p, PointerButton::Primary, &body);
    b.frame(Vec::new(), &body);
    assert_eq!(*s.borrow(), "N-BK7");
    // ガラスの欄: 出しただけでは頼まず、候補が無いまま欄に入ったら頼む
    let mut b = Bench::new();
    let t = std::cell::RefCell::new(String::from("N-BK7"));
    let glass = |ui: &mut Ui| {
        rect.set(ne::glass_edit(ui, &mut t.borrow_mut(), 80.0, "", false).rect);
    };
    b.frame(Vec::new(), &glass);
    assert!(!ne::take_glass_request(&b.ctx));
    b.click(rect.get().center(), PointerButton::Primary, &glass);
    assert!(ne::take_glass_request(&b.ctx));
    assert!(!ne::take_glass_request(&b.ctx));
    let g = vec![GlassChoice {
        name: "N-BK7".into(),
        desc: String::new(),
    }];
    // 読む前（候補が空）は何でも受ける
    assert!(ne::glass_known("XX", &[], false));
    assert!(ne::glass_known("n-bk7", &g, false));
    assert!(ne::glass_known("1.5,60", &g, false));
    assert!(!ne::glass_known("N-BK8", &g, false));
    assert!(!ne::glass_known("1.33", &g, false));
    assert!(ne::glass_known("1.33", &g, true));
    ne::set_glasses(&b.ctx, std::sync::Arc::new(g));
    b.frame(Vec::new(), &glass);
    assert!(!ne::take_glass_request(&b.ctx));
}
