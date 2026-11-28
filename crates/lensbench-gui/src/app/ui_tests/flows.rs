//! 操作の通し確認（筋書き）: 道具の窓・パネルごとに「開く → 値を 1 つ変える → 系や結果に反映される → 閉じる」を、
//! 本物のポインタとキーの入力だけで確かめる。
//!
//! ほかの `ui_tests` は AccessKit の操作（`click_accesskit`）で部品を直に押すので、メニューが中のクリックで閉じる・窓が
//! かぶって押せない・押した所に別の部品がある、を拾えない（0.12.0 の「光束・仕上げの表がマウスで変えられない」）。
//! ここではポインタを部品の真ん中に動かしてボタンを押し・離すだけにし、押す前に部品を見える所まで送って画面の中に
//! あることを見る。窓は開いたままかを、変えた後にもう一度部品を探して確かめる。
//!
//! 回し方: `cargo test --profile ci -p lensbench-gui --lib flow_`（リリース前の見直しの型 `review/README.md`）

use super::*;
use egui_kittest::kittest::by;

mod combo_tools;
mod file_menu;
mod panels;
mod tables;

/// 見本の系を開く引数（ガラスは試験のカタログ）
fn open(rel: &str) -> Args {
    let r = root();
    Args {
        open: Some(r.join(rel)),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    }
}

/// 筋書きを回す道具: App 全体を窓なしで回し、ポインタとキーで操作する
struct Drive {
    h: Harness<'static, App>,
    /// 筋書きの名前（落ちたときの文言に出す）
    flow: &'static str,
    _g: std::sync::MutexGuard<'static, ()>,
}

impl Drive {
    fn new(flow: &'static str, args: Args) -> Drive {
        // App の試験は 1 つずつ（serve の系・言語はプロセスで 1 つ）
        let g = crate::serve_lock();
        let h = Harness::builder()
            .allow_missing_glyphs()
            .with_size(SIZE)
            .with_max_steps(1000)
            .build_eframe(move |cc| App::new(cc, args));
        i18n::set(i18n::Lang::En);
        let mut d = Drive { h, flow, _g: g };
        d.settle();
        d
    }

    fn app(&self) -> &App {
        self.h.state()
    }

    /// 計算と下書きの要求が片付くまで回す（30 秒で諦める）
    fn settle(&mut self) {
        let t0 = Instant::now();
        loop {
            self.h.run_steps(2);
            if !self.app().test_state()["busy"].as_bool().unwrap_or(false) {
                self.h.run_steps(2);
                if !self.app().test_state()["busy"].as_bool().unwrap_or(false) {
                    return;
                }
            }
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "[{}] still busy: {}",
                self.flow,
                self.app().test_state()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 条件 f（文言・役割）に合う部品の位置の一覧（木の順、位置の無い部品は除く）
    fn rects(&self, f: &Match) -> Vec<egui::Rect> {
        let f = f.clone();
        self.h
            .query_all(by().predicate(move |n| n.label().is_some_and(|l| f(&l, n.role()))))
            .filter_map(|n| rect_of(&n))
            .collect()
    }

    /// 条件 f に合う部品のうち pick が選んだもの（what は落ちたときの文言）を見える所まで送って、その位置を返す。
    /// 送るのは AccessKit の「見える所まで送る」（利用者が送り棒を回すのに当たる）。画面の外なら落とす
    fn target(
        &mut self,
        what: &str,
        f: Match,
        pick: impl Fn(&[egui::Rect]) -> Option<usize>,
    ) -> egui::Rect {
        let Some(i) = pick(&self.rects(&f)) else {
            panic!(
                "[{}] no {what} on the screen; shown: {:?}",
                self.flow,
                self.labels()
            );
        };
        {
            let g = f.clone();
            if let Some(n) = self
                .h
                .query_all(by().predicate(move |n| n.label().is_some_and(|l| g(&l, n.role()))))
                .filter(|n| rect_of(n).is_some())
                .nth(i)
            {
                n.scroll_to_me();
            }
        }
        self.h.run_steps(3);
        let v = self.rects(&f);
        let r = *v
            .get(pick(&v).unwrap_or(i))
            .unwrap_or_else(|| panic!("[{}] {what} is gone after scrolling", self.flow));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE);
        assert!(
            screen.contains(r.center()),
            "[{}] {what} is off the screen at {r:?}",
            self.flow
        );
        r
    }

    /// 文言の部品の最初のもの（記号の字形が無いと文言が字だけになる（fonts::glyph_or）ので、記号を除いた文言でも
    /// 探す）。role を与えればその役割のものだけ
    fn find(&mut self, label: &str, role: Option<egui::accesskit::Role>) -> egui::Rect {
        let f = named(label, role);
        self.target(&format!("{label:?}"), f, |v| (!v.is_empty()).then_some(0))
    }

    /// 画面にある文言の一覧（落ちたときに何が出ていたか）
    fn labels(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .h
            .query_all(by().predicate(|n| n.label().is_some()))
            .filter_map(|n| n.accesskit_node().label())
            .filter(|l| !l.is_empty())
            .collect();
        v.dedup();
        v.truncate(150);
        v
    }

    /// 文言の部品の n 番目（0 から、木の順）の位置
    fn nth_rect(&mut self, label: &str, nth: usize) -> egui::Rect {
        self.target(&format!("{label:?} #{nth}"), named(label, None), |v| {
            (nth < v.len()).then_some(nth)
        })
    }

    /// 出ているか
    fn shown(&self, label: &str) -> bool {
        let want = label.to_owned();
        let bare = strip_symbols(label);
        self.h
            .query_all(by().predicate(move |n| {
                n.label()
                    .is_some_and(|l| l == want || (!bare.is_empty() && l == bare))
            }))
            .next()
            .is_some()
    }

    /// 出ていなければ落とす（窓が閉じた・表が消えた）
    fn expect(&self, label: &str, why: &str) {
        assert!(
            self.shown(label),
            "[{}] {label:?} is gone ({why}); shown: {:?}",
            self.flow,
            self.labels()
        );
    }

    /// 位置 p をポインタで押して離す
    fn click_at(&mut self, p: egui::Pos2) {
        self.h.hover_at(p);
        self.h.run_steps(1);
        for pressed in [true, false] {
            self.h.event(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        self.settle();
    }

    /// from から to へポインタで引きずる（窓の題の帯を持って動かすなど）
    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        self.h.hover_at(from);
        self.h.run_steps(1);
        self.h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        self.h.run_steps(1);
        for k in 1..=8 {
            self.h.hover_at(from + (to - from) * (k as f32 / 8.0));
            self.h.run_steps(1);
        }
        self.h.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        self.settle();
    }

    /// 文言の部品を押す
    fn press(&mut self, label: &str) {
        let r = self.find(label, None);
        self.click_at(r.center());
    }

    /// 役割と文言で部品を押す（同じ文言の見出しとボタンがあるとき）
    fn press_role(&mut self, label: &str, role: egui::accesskit::Role) {
        let r = self.find(label, Some(role));
        self.click_at(r.center());
    }

    /// 一覧（ComboBox）を開いて項目を選ぶ。n 番目（0 から）の同じ名前の一覧
    fn choose(&mut self, combo: &str, nth: usize, item: &str) {
        let r = self.target(
            &format!("list {combo:?} #{nth}"),
            named(combo, Some(egui::accesskit::Role::ComboBox)),
            |v| (nth < v.len()).then_some(nth),
        );
        self.click_at(r.center());
        self.press(item);
    }

    /// 文言の欄（字の欄・数の欄）の n 番目を押して中身を text に替え、Enter で決める
    fn type_into(&mut self, label: &str, nth: usize, text: &str) {
        let want = label.to_owned();
        let f: Match = std::rc::Rc::new(move |l: &str, r| {
            l == want
                && matches!(
                    r,
                    egui::accesskit::Role::TextInput | egui::accesskit::Role::SpinButton
                )
        });
        let r = self.target(&format!("field {label:?} #{nth}"), f, |v| {
            (nth < v.len()).then_some(nth)
        });
        self.click_at(r.center());
        self.type_enter(text);
    }

    /// 文言 label の部品のうち、文言 anchor の部品（パネルの札など）より下で一番近いものを押す（同じ名前のボタンが
    /// パネルごとにあるとき）
    fn press_near(&mut self, label: &str, anchor: &str) {
        let a = self.find(anchor, None);
        let r = self.target(
            &format!("{label:?} near {anchor:?}"),
            named(label, None),
            |v| {
                (0..v.len())
                    .filter(|&i| v[i].center().y > a.top())
                    .min_by(|&x, &y| {
                        v[x].center()
                            .distance(a.center())
                            .total_cmp(&v[y].center().distance(a.center()))
                    })
            },
        );
        self.click_at(r.center());
    }

    /// 解析のパネルを足す（Panels → Add analysis → + 名前）
    fn add_analysis(&mut self, name: &str) {
        self.press_role("Panels", egui::accesskit::Role::Button);
        self.press("Add analysis");
        self.press(&format!("+ {name}"));
    }

    /// 文言が条件 f に合う部品の真ん中を押す（表の升など、文言が「Row 1 Weight」の形のもの）
    fn press_where(&mut self, what: &str, f: impl Fn(&str) -> bool + 'static) {
        let r = self.target(what, std::rc::Rc::new(move |l: &str, _| f(l)), |v| {
            (!v.is_empty()).then_some(0)
        });
        self.click_at(r.center());
    }

    /// 欄の中身を全部選んでから字を打ち、Enter で決める（欄を押した後）
    fn type_enter(&mut self, text: &str) {
        self.h
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        self.h.run_steps(1);
        self.type_cell(text);
    }

    /// 字を打って Enter で決める（表の升を押した後: 表は打ち始めると升を書き換える。Ctrl+A は行を全部選ぶ）
    fn type_cell(&mut self, text: &str) {
        self.h.event(egui::Event::Text(text.to_owned()));
        self.h.run_steps(2);
        self.h.key_press(egui::Key::Enter);
        self.settle();
    }

    /// 種類 cmd の解析の結果（最初のパネル）: 出したときの引数と値
    fn result(&self, cmd: &str) -> Option<(&Value, Option<&Value>, Option<&String>)> {
        self.app()
            .c
            .results
            .values()
            .find(|r| r.cmd == cmd)
            .map(|r| (&r.params, r.value.as_ref(), r.error.as_ref()))
    }

    /// 解析 cmd の結果が誤りなく出ていること
    fn expect_result(&self, cmd: &str) -> Value {
        let Some((p, v, e)) = self.result(cmd) else {
            panic!(
                "[{}] no {cmd} result: {}",
                self.flow,
                self.app().test_state()
            );
        };
        assert!(e.is_none(), "[{}] {cmd} failed: {e:?}", self.flow);
        assert!(v.is_some(), "[{}] {cmd} has no value", self.flow);
        p.clone()
    }

    /// 上の帯のメニューから項目を選ぶ（例: `["Panels", "Optimization"]`）
    fn menu(&mut self, path: &[&str]) {
        for (i, p) in path.iter().enumerate() {
            if i == 0 {
                self.press_role(p, egui::accesskit::Role::Button);
            } else {
                self.press(p);
            }
        }
    }
}

/// 試験の画面の大きさ
const SIZE: egui::Vec2 = egui::vec2(1400.0, 900.0);

/// 部品を選ぶ条件（文言・役割）
type Match = std::rc::Rc<dyn Fn(&str, egui::accesskit::Role) -> bool>;

/// 文言 label（記号を除いた文言も）で、role を与えればその役割の部品
fn named(label: &str, role: Option<egui::accesskit::Role>) -> Match {
    let want = label.to_owned();
    let bare = strip_symbols(label);
    std::rc::Rc::new(move |l: &str, r| {
        (l == want || (!bare.is_empty() && l == bare)) && role.is_none_or(|x| x == r)
    })
}

/// 部品の位置（位置の無い部品は None。kittest の Node::rect は位置が無いと panic する）
fn rect_of(n: &egui_kittest::Node<'_>) -> Option<egui::Rect> {
    n.accesskit_node().bounding_box().map(|b| {
        egui::Rect::from_min_max(
            egui::pos2(b.x0 as f32, b.y0 as f32),
            egui::pos2(b.x1 as f32, b.y1 as f32),
        )
    })
}

impl Drop for Drive {
    /// 非順次の追跡がゴーストの系を書いた一時フォルダ（試験のプロセスのもの）を消す
    fn drop(&mut self) {
        crate::nsc_ghosts::finish();
    }
}

/// 字形の無いフォントで消える記号（全角の記号・矢印・三角など）を除き、前後の空白を詰める
fn strip_symbols(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_ascii_punctuation() || *c == ' ')
        .collect::<String>()
        .trim()
        .to_owned()
}
