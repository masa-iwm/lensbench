//! 入力欄の下に出す候補の一覧（レンズデータの種類・硝材、解析の引数の名前で共通）。
//! 見える行だけ描き、↑↓ で選んだ行が見えるように位置を合わせる

use egui::{Id, Rect, Ui};

/// 候補の一覧の出し方
pub(crate) struct Choices<'a> {
    /// 候補（名前, 説明）。説明が空なら名前だけ
    pub items: &'a [(String, String)],
    /// 選んでいる行
    pub pick: usize,
    /// この行が見えるように位置を合わせる（↑↓・開いた直後）
    pub scroll_to: Option<usize>,
    pub min_width: f32,
    pub max_height: f32,
    /// 候補が無いときの文
    pub empty: Option<&'a str>,
    /// 一覧の下の小さな文（件数など）
    pub footer: Option<String>,
}

/// 一覧を出したときの結果
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    /// クリックした行
    pub clicked: Option<usize>,
    /// ポインタが一覧の上にある（欄のフォーカスが外れても決めない判断に使う）
    pub hovered: bool,
}

impl<'a> Choices<'a> {
    pub(crate) fn new(items: &'a [(String, String)], pick: usize) -> Self {
        Choices {
            items,
            pick,
            scroll_to: None,
            min_width: 240.0,
            max_height: 280.0,
            empty: None,
            footer: None,
        }
    }

    /// `anchor`（ふつうは入力欄）の左下に出す。候補が無く `empty` も無ければ何も出さない
    pub(crate) fn show(self, ui: &Ui, id: Id, anchor: Rect) -> Shown {
        if self.items.is_empty() && self.empty.is_none() {
            return Shown::default();
        }
        let mut clicked = None;
        let area = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(self.min_width);
                    if self.items.is_empty() {
                        if let Some(t) = self.empty {
                            ui.weak(t);
                        }
                        return;
                    }
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let row_h = row_height(ui);
                    let mut sa = egui::ScrollArea::vertical()
                        .id_salt("choices_scroll")
                        .max_height(self.max_height)
                        .auto_shrink([false, true]);
                    if let Some(k) = self.scroll_to {
                        sa = sa.vertical_scroll_offset(scroll_offset(k, row_h, self.max_height));
                    }
                    sa.show_rows(ui, row_h, self.items.len(), |ui, range| {
                        for k in range {
                            let (t, d) = &self.items[k];
                            let txt = if d.is_empty() {
                                t.clone()
                            } else {
                                format!("{t}  {d}")
                            };
                            if ui.selectable_label(k == self.pick, txt).clicked() {
                                clicked = Some(k);
                            }
                        }
                    });
                    if let Some(f) = &self.footer {
                        ui.label(egui::RichText::new(f).small().weak());
                    }
                });
            });
        Shown {
            clicked,
            hovered: area.response.contains_pointer(),
        }
    }
}

/// 1 行の高さ（selectable_label の高さに合わせる。interact_size.y が下限）
fn row_height(ui: &Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y)
        .max(ui.spacing().interact_size.y)
}

/// 行 `k` が一覧の真ん中あたりに来る位置
fn scroll_offset(k: usize, row_h: f32, max_h: f32) -> f32 {
    (k as f32 * row_h - max_h * 0.5 + row_h * 0.5).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_rows_do_not_scroll() {
        assert_eq!(scroll_offset(0, 18.0, 280.0), 0.0);
        assert_eq!(scroll_offset(5, 18.0, 280.0), 0.0);
        // 下の行は真ん中に来る
        let y = scroll_offset(100, 18.0, 280.0);
        assert!((y + 140.0 - 9.0 - 1800.0).abs() < 1e-3);
    }
}
