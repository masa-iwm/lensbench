//! レイアウト図の当たり判定とクリック・ドラッグ・キーの試験

use super::*;

fn sq(x0: f32, y0: f32, w: f32) -> Vec<Pos2> {
    vec![
        Pos2::new(x0, y0),
        Pos2::new(x0 + w, y0),
        Pos2::new(x0 + w, y0 + w),
        Pos2::new(x0, y0 + w),
    ]
}

/// 当たり判定は描かずに決まり、後に描くもの（BS → 玉 → 開口・絞り・名前）ほど上
#[test]
fn hits_follow_paint_order() {
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| {
        let galley = ctx
            .fonts(|f| f.layout_no_wrap("L1".into(), FontId::proportional(11.0), Color32::WHITE));
        let mark = |k, kind, pos: Pos2, label: Rect| Mark {
            k,
            kind,
            pos,
            u: Vec2::X,
            n: Vec2::new(0.0, -1.0),
            r: 10.0,
            ap: if kind == ElemKind::Aperture {
                vec![(10.0, 20.0), (-20.0, -10.0)]
            } else {
                Vec::new()
            },
            label,
            galley: galley.clone(),
            sel: false,
        };
        let sc = Scene {
            cubes: vec![Cube {
                k: 0,
                sq: sq(0.0, 0.0, 20.0),
                diag: [Pos2::ZERO, Pos2::new(20.0, 20.0)],
            }],
            // 玉 1 は BS に重なる
            bodies: vec![Body {
                k: Some(1),
                poly: sq(10.0, 0.0, 20.0),
                mirror: false,
                surfaces: [3, 4],
            }],
            hl: Vec::new(),
            hl_vertex: None,
            mechs: vec![("tube".into(), vec![sq(100.0, 0.0, 10.0)])],
            stop: None,
            lead: false,
            lead_sel: false,
            marks: vec![
                // 名前が玉 1 の上にある要素 2
                mark(
                    2,
                    ElemKind::Lens,
                    Pos2::new(60.0, 60.0),
                    Rect::from_min_size(Pos2::new(25.0, 5.0), egui::vec2(10.0, 10.0)),
                ),
                mark(
                    3,
                    ElemKind::Aperture,
                    Pos2::new(200.0, 100.0),
                    Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(1.0, 1.0)),
                ),
                mark(
                    4,
                    ElemKind::Stop,
                    Pos2::new(250.0, 100.0),
                    Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(1.0, 1.0)),
                ),
            ],
            dims: vec![Dim {
                g: 7,
                sel: false,
                thin: [[Pos2::ZERO; 2]; 2],
                seg: [Pos2::new(0.0, 200.0), Pos2::new(100.0, 200.0)],
                dir: Vec2::X,
                arrows: true,
                text: Rect::from_min_size(Pos2::new(45.0, 185.0), egui::vec2(10.0, 12.0)),
                galley: galley.clone(),
            }],
            sel: None,
            axes: Vec::new(),
            mids: Vec::new(),
        };
        let at = |x, y| hits(&sc, Some(Pos2::new(x, y)));
        assert_eq!(hits(&sc, None), Hits::default());
        assert_eq!(at(5.0, 10.0).el, Some(0));
        let h = at(15.0, 10.0);
        assert_eq!((h.el, h.surfs), (Some(1), Some([3, 4])));
        let h = at(27.0, 10.0);
        assert_eq!((h.el, h.surfs), (Some(2), Some([3, 4])));
        assert_eq!(at(105.0, 5.0).mechs, vec![(0, 0)]);
        // 開口: 光を遮る区間の上（n は上向き）と、区間の外（光が通る所）
        assert_eq!(at(200.0, 85.0).el, Some(3));
        assert_eq!(at(200.0, 100.0).el, None);
        // 絞り: 面の線の近く（半径 + 14 まで）
        assert_eq!(at(252.0, 78.0).el, Some(4));
        assert_eq!(at(260.0, 100.0).el, None);
        // 寸法: 数字の上は数字と線、線の近くは線だけ
        let h = at(50.0, 190.0);
        assert_eq!((h.num, h.dim), (Some(7), Some(7)));
        let h = at(80.0, 204.0);
        assert_eq!((h.num, h.dim), (None, Some(7)));
        assert_eq!(at(80.0, 210.0).dim, None);
        // 先頭に自動で置く絞りが BS の平らな前面に重なる（間隔 0）: 面の線の上は絞り、少し中は BS
        let mut sc = sc;
        sc.lead = true;
        sc.stop = Some((Pos2::new(0.0, 10.0), Vec2::new(0.0, -1.0), 5.0));
        let at = |x, y| hits(&sc, Some(Pos2::new(x, y)));
        let h = at(1.0, 10.0);
        assert_eq!((h.lead, h.el), (true, None));
        let h = at(5.0, 10.0);
        assert_eq!((h.lead, h.el), (true, Some(0)));
    });
}

/// 図を画面無しで回す。組合せ（玉 2 枚、絞りから 5 mm・10 mm・20 mm）と、serve の layout の結果
struct Fig {
    ctx: egui::Context,
    combo: Combo,
    view: View,
    d: LayoutData,
    t: f64,
}

/// 1 フレームの後の画面の上の形（寸法の数字の位置を知るため）
struct Shot {
    sc: Scene,
}

impl Fig {
    fn new() -> Fig {
        let mut c = Combo {
            epd: 4.0,
            stop_gap: 5.0,
            ..Default::default()
        };
        for air in [10.0, 20.0] {
            let mut l = Pick::new(ElemKind::Lens);
            l.air = air;
            c.picks.push(l);
        }
        Fig::of(c)
    }

    fn of(c: Combo) -> Fig {
        let mut proj = c.project();
        proj["glass_catalogs"] = json!(["SCHOTT"]);
        // 試験はスレッドで並べて回るので、ファイルは呼ぶたびに別の名前
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let f =
            std::env::temp_dir().join(format!("lensbench-figtest-{}-{n}.json", std::process::id()));
        lensbench_core::vfs::write(&f, proj.to_string()).unwrap();
        let gd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass");
        let v = lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dirs": [gd.display().to_string()] }),
            &json!({}),
        )
        .unwrap();
        lensbench_core::vfs::remove_file(&f).ok();
        let mut f = Fig {
            ctx: egui::Context::default(),
            combo: c,
            view: View::default(),
            d: LayoutData::parse(&v),
            t: 0.0,
        };
        f.frame(Vec::new());
        f
    }

    /// 1 フレーム回す（図と、レイアウトのパネルと同じく図のキー）
    fn frame(&mut self, events: Vec<egui::Event>) -> Shot {
        self.t += 0.05;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 500.0))),
            time: Some(self.t),
            modifiers,
            events,
            ..Default::default()
        };
        let (combo, view, d) = (&mut self.combo, &mut self.view, &self.d);
        let mut shot = None;
        let _ = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                let accent = Color32::from_rgb(10, 120, 250);
                // レイアウトのパネル（combo::ui）と同じく、図にフォーカスがあるときだけ図のキー
                if resp.clicked() || resp.drag_started() {
                    resp.request_focus();
                }
                let focused = resp.has_focus();
                figure(ui, rect, &resp, Some(combo), d, view, accent);
                if focused {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            resp.id,
                            egui::EventFilter {
                                tab: false,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: true,
                            },
                        )
                    });
                    keys(ui, combo, view);
                }
                let painter = ui.painter_at(rect);
                let (xf, inner) = xform(ui, rect, &resp, combo, d, view).unwrap();
                shot = Some(Shot {
                    sc: scene(&painter, rect, inner, xf, combo, d, view, accent, accent),
                });
            });
        });
        shot.unwrap()
    }

    /// クリック（前のクリックと続けてダブルクリックにならないよう間をあける）
    fn click(&mut self, p: Pos2) {
        self.t += 1.0;
        self.frame(vec![egui::Event::PointerMoved(p)]);
        self.frame(button(p, true));
        self.frame(button(p, false));
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
    }

    fn mid(&self, k: usize) -> Pos2 {
        self.view.mids[k].unwrap()
    }

    fn dim_text(&mut self, g: usize) -> Pos2 {
        let sh = self.frame(Vec::new());
        sh.sc.dims.iter().find(|d| d.g == g).unwrap().text.center()
    }

    /// 寸法線の上で数字から離れた所（線の 1/4）
    fn dim_line(&mut self, g: usize) -> Pos2 {
        let sh = self.frame(Vec::new());
        let d = sh.sc.dims.iter().find(|d| d.g == g).unwrap();
        let p = d.seg[0] + (d.seg[1] - d.seg[0]) / 4.0;
        assert!(!d.text.expand(4.0).contains(p));
        p
    }
}

fn button(pos: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }]
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

/// クリックで要素・寸法を選ぶ、ダブルクリックで面の表を開く、何も無い所で外す
#[test]
fn click_selects_and_double_click_opens() {
    let mut f = Fig::new();
    let m1 = f.mid(1);
    f.click(m1);
    assert_eq!((f.view.sel, f.view.dim), (Some(1), None));
    // 寸法線（数字の外）は選ぶだけ
    let on_line = f.dim_line(0);
    f.click(on_line);
    assert_eq!(
        (f.view.sel, f.view.dim, f.view.edit.is_none()),
        (None, Some(0), true)
    );
    f.click(Pos2::new(5.0, 5.0));
    assert_eq!((f.view.sel, f.view.dim), (None, None));
    // ダブルクリック
    let m0 = f.mid(0);
    f.click(m0);
    f.frame(button(m0, true));
    f.frame(button(m0, false));
    assert_eq!((f.view.sel, f.view.open), (Some(0), Some(0)));
}

/// 先頭に自動で置く絞り（厚さの無い、要素でない絞り）もクリックで選べ、要素を選ぶと外れる
#[test]
fn click_selects_lead_stop() {
    let mut f = Fig::new();
    let sh = f.frame(Vec::new());
    assert!(sh.sc.lead);
    let (c, n, r) = sh.sc.stop.unwrap();
    // 面の線の上（光軸の上と、絞りの縁の短い線の上）
    for p in [c, c + n * (r + 7.0)] {
        f.click(Pos2::new(5.0, 5.0));
        assert!(!f.view.lead());
        f.click(p);
        assert!(f.view.lead() && f.view.sel.is_none(), "{p:?}");
        assert!(f.frame(Vec::new()).sc.lead_sel);
    }
    let m0 = f.mid(0);
    f.click(m0);
    assert!(!f.view.lead() && f.view.sel == Some(0));
    // 絞りを要素にした後は、絞りの要素として選ぶ
    f.combo.make_stop_element();
    assert!(!f.frame(Vec::new()).sc.lead);
}

/// 寸法の数字をクリックして打ち、Enter で決める。負の値は欄を残し、Esc でやめる
#[test]
fn dim_number_edit() {
    let mut f = Fig::new();
    let num = f.dim_text(0);
    f.click(num);
    assert_eq!(f.view.edit.as_ref().map(|e| e.0), Some(0));
    f.frame(Vec::new());
    f.frame(vec![egui::Event::Text("12.5".into())]);
    f.key(Key::Enter, Modifiers::NONE);
    f.frame(Vec::new());
    assert!(f.view.edit.is_none());
    assert!(close(gap_get(&f.combo, 0), 12.5));
    // 負の値: 欄が残り、値は変えない
    let num = f.dim_text(0);
    f.click(num);
    f.frame(Vec::new());
    f.frame(vec![egui::Event::Text("-1".into())]);
    f.key(Key::Enter, Modifiers::NONE);
    f.frame(Vec::new());
    assert!(f.view.edit.is_some());
    assert!(close(gap_get(&f.combo, 0), 12.5));
    f.key(Key::Escape, Modifiers::NONE);
    f.frame(Vec::new());
    f.frame(Vec::new());
    assert!(f.view.edit.is_none());
    assert!(close(gap_get(&f.combo, 0), 12.5));
}

/// 要素を光軸に沿ってドラッグすると前の間隔が増え後の間隔が減る（合計は変わらない）
#[test]
fn drag_moves_along_axis() {
    let mut f = Fig::new();
    let (g0, g1) = (f.combo.stop_gap, gap_get(&f.combo, 0).unwrap());
    let p = f.mid(0);
    f.frame(vec![egui::Event::PointerMoved(p)]);
    f.frame(button(p, true));
    for x in [10.0, 20.0, 30.0] {
        f.frame(vec![egui::Event::PointerMoved(p + egui::vec2(x, 0.0))]);
    }
    f.frame(button(p + egui::vec2(30.0, 0.0), false));
    assert_eq!(f.view.sel, Some(0));
    let (a, b) = (f.combo.stop_gap, gap_get(&f.combo, 0).unwrap());
    assert!(a > g0 + 0.1, "{g0} → {a}");
    assert!(((a - g0) + (b - g1)).abs() < 1e-9, "{a} {b}");
    assert!(f.view.drag0.is_none());
}

/// 寸法の数字のドラッグでその間隔だけ増減する（表のセルの Alt+ドラッグと同じ量）。要素は動かさない
#[test]
fn drag_dim_number_changes_gap() {
    let mut f = Fig::new();
    let v = gap_get(&f.combo, 0).unwrap();
    let g = f.combo.stop_gap;
    let p = f.dim_text(0);
    f.frame(vec![egui::Event::PointerMoved(p)]);
    f.frame(button(p, true));
    for x in [10.0, 20.0, 30.0] {
        f.frame(vec![egui::Event::PointerMoved(p + egui::vec2(x, 0.0))]);
    }
    f.frame(button(p + egui::vec2(30.0, 0.0), false));
    let want = crate::widgets::num::drag_value(v, 30.0, false);
    assert!(
        close(gap_get(&f.combo, 0), want),
        "{v} → {:?} ({want})",
        gap_get(&f.combo, 0)
    );
    assert!(want > v);
    assert_eq!(
        (f.combo.stop_gap, f.view.dim, f.view.edit.is_none()),
        (g, Some(0), true)
    );
}

/// 図のキー: [ ] で選ぶ、矢印で動かす（刻みは表と同じ: 0.1、Shift 1、Ctrl 0.01）、↑ で偏心・Shift+↑ で傾き、
/// 寸法は ← → で変える・Enter で書き換え、Ctrl+L・R・Delete
#[test]
fn figure_keys() {
    let mut f = Fig::new();
    let none = Modifiers::NONE;
    // 図にフォーカスが無いとキーは効かない
    f.key(Key::CloseBracket, none);
    assert_eq!(f.view.sel, None);
    f.click(Pos2::new(5.0, 5.0));
    f.key(Key::CloseBracket, none);
    assert_eq!(f.view.sel, Some(0));
    f.key(Key::CloseBracket, none);
    f.key(Key::CloseBracket, none);
    assert_eq!(f.view.sel, Some(0));
    f.key(Key::OpenBracket, none);
    assert_eq!(f.view.sel, Some(1));
    let g0 = gap_get(&f.combo, 0).unwrap();
    f.key(Key::ArrowRight, none);
    assert!(close(gap_get(&f.combo, 0), g0 + 0.1));
    // 刻みは表と同じ: 素で 0.1、Shift で 1、Ctrl で 0.01
    f.key(Key::ArrowLeft, Modifiers::COMMAND);
    assert!(close(gap_get(&f.combo, 0), g0 + 0.09));
    f.key(Key::ArrowRight, Modifiers::SHIFT);
    assert!(close(gap_get(&f.combo, 0), g0 + 1.09));
    f.key(Key::ArrowUp, none);
    assert!((f.combo.picks[1].decenter[1] - 0.1).abs() < 1e-9);
    f.key(Key::ArrowUp, Modifiers::SHIFT);
    assert!((f.combo.picks[1].tilt[0] - 1.0).abs() < 1e-9);
    f.key(Key::R, none);
    assert!(f.combo.picks[1].reversed);
    // ロックした要素は裏返さない（動かすのはできる）
    f.key(Key::L, Modifiers::COMMAND);
    assert!(f.combo.picks[1].locked);
    f.key(Key::R, none);
    assert!(f.combo.picks[1].reversed);
    f.key(Key::L, Modifiers::COMMAND);
    f.key(Key::Escape, none);
    assert_eq!(f.view.sel, None);
    // 寸法: 線をクリックして選び、→ で広げる、Enter でその場の書き換え
    let on_line = f.dim_line(1);
    f.click(on_line);
    assert_eq!(f.view.dim, Some(1));
    let v = gap_get(&f.combo, 1).unwrap();
    f.key(Key::ArrowRight, none);
    assert!(close(gap_get(&f.combo, 1), v + 0.1));
    // PageUp/PageDown は矢印の 10 倍
    f.key(Key::PageUp, Modifiers::SHIFT);
    assert!(close(gap_get(&f.combo, 1), v + 10.1));
    f.key(Key::PageDown, none);
    assert!(close(gap_get(&f.combo, 1), v + 9.1));
    f.key(Key::Enter, none);
    assert_eq!(f.view.edit.as_ref().map(|e| e.0), Some(1));
    f.frame(Vec::new());
    f.key(Key::Escape, none);
    f.frame(Vec::new());
    assert!(f.view.edit.is_none());
    // 寸法を選んで数を打てば、その字から書き換えが始まる（表のセルと同じ）
    f.frame(vec![egui::Event::Text("7".into())]);
    assert_eq!(
        f.view.edit.as_ref().map(|e| (e.0, e.1.as_str())),
        Some((1, "7"))
    );
    f.frame(Vec::new());
    f.key(Key::Escape, none);
    f.frame(Vec::new());
    assert!(f.view.edit.is_none());
    // 図にフォーカスを戻して Delete で外す
    f.click(Pos2::new(5.0, 5.0));
    f.key(Key::CloseBracket, none);
    f.key(Key::Delete, none);
    assert_eq!((f.combo.len(), f.view.sel), (1, None));
}

/// 名前・寸法を図の中へ寄せる計算（光軸がどの向きでも同じ: 横・縦・斜め）
#[test]
fn labels_and_dims_stay_in_view() {
    let area = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 50.0));
    // 名前: 寄せる軸だけずらす（もう一方の軸ははみ出たまま）
    let r = Rect::from_min_size(Pos2::new(120.0, 70.0), egui::vec2(20.0, 10.0));
    let x = keep_in(r, area, 0);
    assert_eq!((x.min.x, x.min.y), (80.0, 70.0));
    let y = keep_in(r, area, 1);
    assert_eq!((y.min.x, y.min.y), (120.0, 40.0));
    let l = keep_in(r.translate(egui::vec2(-150.0, 0.0)), area, 0);
    assert_eq!(l.min.x, 0.0);
    // 寸法の数字: 見える部分の中へ。区間が見えなければそのまま
    assert_eq!(along_in(-200.0, 100.0, area.x_range(), 10.0), 5.0);
    assert_eq!(along_in(-400.0, -200.0, area.x_range(), 10.0), -300.0);
    // 斜めの寸法線: ずらす量を詰める
    let n = egui::vec2(1.0, 1.0).normalized();
    let t = fit_along(Pos2::new(10.0, 10.0), n, 200.0, area);
    let p = Pos2::new(10.0, 10.0) + n * t;
    assert!(area.contains(p) && (p.y - 50.0).abs() < 1e-3);
    // 横向きの n で、点が縦に外れていればそのまま
    assert_eq!(
        fit_along(Pos2::new(10.0, 80.0), egui::Vec2::X, 200.0, area),
        200.0
    );
    assert_eq!(
        fit_along(Pos2::new(10.0, 10.0), egui::Vec2::X, 200.0, area),
        90.0
    );
}

/// 鏡で光軸を縦へ折った後の要素も、図を横へ動かして要素が少しはみ出たとき名前と寸法線が図の中に残る
/// （光軸が横の要素で、図を縦へ動かしたときと同じ）。全部図の外へ出たら寄せない
#[test]
fn folded_labels_follow_view() {
    let mut c = Combo {
        epd: 4.0,
        stop_gap: 5.0,
        ..Default::default()
    };
    let mut l = Pick::new(ElemKind::Lens);
    l.air = 10.0;
    c.picks.push(l);
    let mut m = Pick::new(ElemKind::Mirror);
    m.air = 15.0;
    c.picks.push(m);
    let mut l = Pick::new(ElemKind::Lens);
    l.air = 20.0;
    c.picks.push(l);
    let mut f = Fig::of(c);
    let sh = f.frame(Vec::new());
    // 折った後の玉の光軸は縦
    assert!(sh.sc.axes[2].y.abs() > 0.9, "{:?}", sh.sc.axes);
    let area = f.ctx.screen_rect();
    let m = sh.sc.marks.iter().find(|m| m.k == 2).unwrap();
    let (x, r) = (m.pos.x, m.r);
    let xs = area.x_range();
    // 玉の真ん中が図の左・右の端の少し外（玉の縁は見えている）
    for dx in [area.left() - r / 2.0 - x, area.right() + r / 2.0 - x] {
        f.view.pan = egui::vec2(dx, 0.0);
        let sh = f.frame(Vec::new());
        let m = sh.sc.marks.iter().find(|m| m.k == 2).unwrap();
        assert!(
            xs.contains(m.label.min.x) && xs.contains(m.label.max.x),
            "{dx}: {:?}",
            m.label
        );
        // 縦の寸法線（鏡から玉まで）も図の中
        let d = sh.sc.dims.iter().find(|d| d.g == 1).unwrap();
        assert!(xs.contains(d.seg[0].x), "{dx}: {:?}", d.seg);
        assert!(xs.contains(d.text.max.x), "{dx}: {:?}", d.text);
    }
    // 全部図の外: 名前は寄せず、寸法線は描かない
    for dx in [-3000.0, 3000.0] {
        f.view.pan = egui::vec2(dx, 0.0);
        let sh = f.frame(Vec::new());
        let m = sh.sc.marks.iter().find(|m| m.k == 2).unwrap();
        assert!(!m.label.intersects(area), "{dx}: {:?}", m.label);
        assert!(sh.sc.dims.iter().all(|d| d.g != 1), "{dx}");
    }
    // 光軸が横の要素も同じ: 図を縦へ大きく動かすと先頭の玉の名前は寄せない
    f.view.pan = egui::vec2(0.0, 3000.0);
    let sh = f.frame(Vec::new());
    let m = sh.sc.marks.iter().find(|m| m.k == 0).unwrap();
    assert!(!m.label.intersects(area), "{:?}", m.label);
}

/// 機構部品の壁をクリックすると部品を選び（要素の選択は外れる）、図の右下の説明に品番・種類と寸法・置いた所が出る。
/// 要素・何も無い所をクリックすると外れる
#[test]
fn click_selects_mechanical_part() {
    // 説明の文言を英語で比べる（言語はプロセスで 1 つ）
    let _g = crate::serve_lock();
    crate::i18n::set(crate::i18n::Lang::En);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/projects");
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("P19_tube_cage.json")).unwrap())
            .unwrap();
    let c = Combo::from_project(&v, &dir).unwrap();
    let mut f = Fig::of(c);
    let sh = f.frame(Vec::new());
    let names: Vec<&str> = sh.sc.mechs.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(names, ["tube", "cage"]);
    // 壁の真ん中（要素に重ならない所）
    let wall = |name: &str| {
        let polys = &sh.sc.mechs.iter().find(|m| m.0 == name).unwrap().1;
        polys
            .iter()
            .map(|p| {
                let n = p.len() as f32;
                Pos2::new(
                    p.iter().map(|q| q.x).sum::<f32>() / n,
                    p.iter().map(|q| q.y).sum::<f32>() / n,
                )
            })
            .find(|&q| {
                let h = hits(&sh.sc, Some(q));
                h.el.is_none() && h.mech.as_deref() == Some(name)
            })
            .unwrap()
    };
    let (tube, cage) = (wall("tube"), wall("cage"));
    let m1 = f.mid(0);
    f.click(m1);
    assert_eq!(f.view.sel, Some(0));
    f.click(tube);
    assert_eq!(super::mech::selected(&f.ctx).as_deref(), Some("tube"));
    assert_eq!(f.view.sel, None);
    f.click(cage);
    assert_eq!(super::mech::selected(&f.ctx).as_deref(), Some("cage"));
    let card = f.combo.mechanics[1].card();
    assert_eq!(card[0], "cage  CP-30");
    assert!(
        card.iter()
            .any(|l| l.starts_with("Cage plate: length 8 mm")),
        "{card:?}"
    );
    assert!(
        card.iter().any(|l| l == "At L1 surface 1, z 25 mm"),
        "{card:?}"
    );
    let tube_card = f.combo.mechanics[0].card();
    assert!(tube_card.iter().any(|l| l == "Holds L1"), "{tube_card:?}");
    assert!(
        tube_card
            .iter()
            .any(|l| l == "Holding diameter 25.4 mm, seat at z 0 mm"),
        "{tube_card:?}"
    );
    f.click(m1);
    assert_eq!((super::mech::selected(&f.ctx), f.view.sel), (None, Some(0)));
    f.click(cage);
    f.click(Pos2::new(5.0, 5.0));
    assert_eq!(super::mech::selected(&f.ctx), None);
}

/// 凹面のある玉（P17 の真ん中の両凹 G2）の塗り: 三角形がどれも断面の中で、面積の和が断面の面積（
/// convex_polygon の扇では縁の角から凹面へ張った三角形が面の外を塗っていた）
#[test]
fn concave_body_fill_stays_inside() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let v = lensbench_engine::preview_layout(
        &json!({
            "file": root.join("examples/projects/P17_zoom_three_groups.json").display().to_string(),
            "glass_dirs": [root.join("tests/glass").display().to_string()],
        }),
        &json!({}),
    )
    .unwrap();
    let d = LayoutData::parse(&v);
    let area = |p: &[Pos2]| {
        let n = p.len();
        (0..n)
            .map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y)
            .sum::<f32>()
            .abs()
            / 2.0
    };
    let mut concave = 0;
    for sd in &d.solids {
        // mm を 10 倍して画面の点に見立てる
        let poly: Vec<Pos2> = sd
            .pts
            .iter()
            .map(|q| Pos2::new(10.0 * q[0] as f32, 10.0 * q[1] as f32))
            .collect();
        let tris = poly_fill::triangulate(&poly);
        let mut sum = 0.0;
        for t in &tris {
            let tri = [poly[t[0]], poly[t[1]], poly[t[2]]];
            let g = Pos2::new(
                (tri[0].x + tri[1].x + tri[2].x) / 3.0,
                (tri[0].y + tri[1].y + tri[2].y) / 3.0,
            );
            assert!(inside(&poly, g), "{:?} {t:?}", sd.surfaces);
            sum += area(&tri);
        }
        let a = area(&poly);
        assert!((sum - a).abs() < 1e-3 * a, "{:?} {sum} {a}", sd.surfaces);
        // 扇（先頭の点から）で塗ると面積の和が断面より大きくなる玉を数える
        let fan: f32 = (1..poly.len() - 1)
            .map(|w| area(&[poly[0], poly[w], poly[w + 1]]))
            .sum();
        if fan > a * 1.001 {
            concave += 1;
        }
    }
    assert!(concave >= 1, "P17 に凹面のある玉が無い");
}
