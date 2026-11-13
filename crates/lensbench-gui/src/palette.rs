//! コマンドパレット（Ctrl+K）: 操作を名前で探して実行する。キーボードだけで、↑↓ で選び Enter で実行、Esc で閉じる。

use egui::{Key, RichText};

use crate::i18n::tr;

/// パレットの 1 行
pub struct Item<C> {
    pub label: String,
    /// 探すときにも見る別名（英語名・キー）
    pub hint: String,
    pub cmd: C,
    /// 今の系・組合せを捨てる操作（新規・読み直す）。何も打っていないときは一覧の最後に置く
    pub danger: bool,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    pub sel: usize,
    focus: bool,
    /// 開く欄（上の帯の「探す」）の場所。あればその真下に右をそろえて出す
    pub anchor: Option<egui::Rect>,
    /// 最近実行した行の名前（新しい順）。何も打っていないときは先頭に出す
    pub recent: Vec<String>,
}

/// 何も打っていないときの並び: 最近使った操作 → ほか → 捨てる操作（GUX-25: Enter で新規が走らない）
fn order<'a, C>(items: &'a [Item<C>], query: &str, recent: &[String]) -> Vec<&'a Item<C>> {
    let mut hits: Vec<&Item<C>> = items
        .iter()
        .filter(|i| matches(query, &i.label, &i.hint))
        .collect();
    if query.trim().is_empty() {
        let rank = |i: &Item<C>| -> (usize, usize) {
            match recent.iter().position(|r| r == &i.label) {
                Some(k) if !i.danger => (0, k),
                _ if i.danger => (2, 0),
                _ => (1, 0),
            }
        };
        hits.sort_by_key(|i| rank(i));
    }
    hits
}

/// 空白で区切った語がすべて含まれる（大文字・小文字を区別しない）
pub fn matches(q: &str, label: &str, hint: &str) -> bool {
    let hay = format!("{label} {hint}").to_lowercase();
    q.split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// パレットの左上。開く欄があればその真下に右端をそろえ（画面からはみ出さないように）、無ければ中央
fn pos_under(screen: egui::Rect, anchor: Option<egui::Rect>, width: f32) -> egui::Pos2 {
    match anchor {
        Some(a) => {
            let lo = screen.left() + 16.0;
            let hi = (screen.right() - 16.0 - width).max(lo);
            egui::pos2((a.right() - width).clamp(lo, hi), a.bottom() + 4.0)
        }
        None => egui::pos2(screen.center().x - width / 2.0, screen.top() + 70.0),
    }
}

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.query.clear();
        self.sel = 0;
        self.focus = true;
    }

    /// 表示して、選ばれたコマンドを返す
    pub fn ui<C: Clone>(&mut self, ctx: &egui::Context, items: &[Item<C>]) -> Option<C> {
        if !self.open {
            return None;
        }
        let hits = order(items, &self.query, &self.recent);
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if esc {
            self.open = false;
            return None;
        }
        if down && !hits.is_empty() {
            self.sel = (self.sel + 1) % hits.len();
        }
        if up && !hits.is_empty() {
            self.sel = (self.sel + hits.len() - 1) % hits.len();
        }
        self.sel = self.sel.min(hits.len().saturating_sub(1));
        let mut chosen = None;
        let mut chosen_label = None;
        if enter {
            chosen = hits.get(self.sel).map(|i| i.cmd.clone());
            chosen_label = hits.get(self.sel).map(|i| i.label.clone());
        }
        let screen = ctx.screen_rect();
        let width = (screen.width() - 32.0).clamp(240.0, 640.0);
        egui::Area::new(egui::Id::new("palette"))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .fixed_pos(pos_under(screen, self.anchor, width))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(tr("操作・解析・パネル・並びを探す"))
                            .desired_width(f32::INFINITY),
                    );
                    if self.focus {
                        r.request_focus();
                        self.focus = false;
                    }
                    if r.changed() {
                        self.sel = 0;
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            if hits.is_empty() {
                                ui.weak(tr("見つからない"));
                            }
                            for (k, it) in hits.iter().enumerate() {
                                let on = k == self.sel;
                                let resp = ui.selectable_label(on, RichText::new(&it.label));
                                if on && (up || down) {
                                    resp.scroll_to_me(None);
                                }
                                if resp.clicked() {
                                    chosen = Some(it.cmd.clone());
                                    chosen_label = Some(it.label.clone());
                                }
                            }
                        });
                    ui.weak(tr("↑↓ で選ぶ · Enter で実行 · Esc で閉じる"));
                });
            });
        if chosen.is_some() {
            self.open = false;
            if let Some(l) = chosen_label {
                self.recent.retain(|x| x != &l);
                self.recent.insert(0, l);
                self.recent.truncate(5);
            }
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 何も打っていないときは、最近使った操作が先頭、捨てる操作（新規）は最後（GUX-25）
    #[test]
    fn empty_query_puts_danger_last() {
        let it = |l: &str, danger: bool| Item {
            label: l.into(),
            hint: String::new(),
            cmd: l.to_string(),
            danger,
        };
        let items = vec![it("新規", true), it("開く", false), it("保存", false)];
        let o: Vec<&str> = order(&items, "", &[])
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(o, ["開く", "保存", "新規"]);
        let o: Vec<&str> = order(&items, "", &["保存".into()])
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(o, ["保存", "開く", "新規"]);
        // 打ったときは一覧の順のまま
        let o: Vec<&str> = order(&items, "新", &[])
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(o, ["新規"]);
    }

    #[test]
    fn query() {
        assert!(matches("spot", "スポットを足す", "add spot"));
        assert!(matches("ス 足", "スポットを足す", ""));
        assert!(!matches("mtf", "スポットを足す", "add spot"));
        assert!(matches("", "x", ""));
    }
}

#[cfg(test)]
mod pos_tests {
    use super::*;

    /// 欄の真下に右端をそろえ、狭い画面でははみ出さない
    #[test]
    fn under_anchor() {
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(2000.0, 900.0));
        let a = egui::Rect::from_min_max(egui::pos2(1500.0, 10.0), egui::pos2(1800.0, 38.0));
        let p = pos_under(screen, Some(a), 640.0);
        assert_eq!(p, egui::pos2(1160.0, 42.0));
        let narrow = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 900.0));
        let a = egui::Rect::from_min_max(egui::pos2(100.0, 10.0), egui::pos2(400.0, 38.0));
        let p = pos_under(narrow, Some(a), 468.0);
        assert_eq!(p.x, 16.0);
        assert_eq!(pos_under(screen, None, 640.0).x, 680.0);
    }
}
