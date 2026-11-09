//! 組合せの要素・範囲をユーザーカタログ（lensbench 形式の .json）に保存する窓と、カタログの項目の
//! 名前・説明の変更・削除。
//!
//! 保存は serve の `catalog_add`（読み込んだ組合せのプロジェクトのブロックを項目にする。面の表の変更は焼き込む）、
//! 一覧・変更は `catalog_items`・`catalog_edit`。カタログをレンズカタログのフォルダに置けば、カタログブラウザに
//! 品として並ぶ。

use egui::{RichText, Ui};
use serde_json::{json, Value};

use crate::catalog::LensRow;
use crate::combo::Combo;
use crate::i18n::{tr, trf};
use crate::theme;

/// カタログの項目（一覧の 1 行）
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub description: String,
    pub blocks: usize,
    /// 名前・説明を書き換え中の値
    pub new_name: String,
    pub new_desc: String,
}

/// 窓の状態
#[derive(Debug, Default)]
pub struct CatSave {
    pub open: bool,
    /// 範囲（要素の番号、両端を含む）
    pub from: usize,
    pub to: usize,
    pub catalog: String,
    pub name: String,
    pub description: String,
    pub vendor: String,
    pub replace: bool,
    /// 読んだカタログの項目（catalog の一覧）
    pub items: Vec<Item>,
    /// items を読んだカタログ
    pub listed: String,
    /// 消す前の確かめ（項目の名前）
    confirm_delete: Option<String>,
    /// 最後の操作の結果（誤りなら true）
    pub note: Option<(String, bool)>,
}

/// 窓から頼む操作（serve の method・params）
pub type Req = (&'static str, Value);

impl CatSave {
    /// 要素 k を選んで開く
    pub fn open_for(&mut self, k: usize, combo: &Combo) {
        self.open = true;
        self.from = k;
        self.to = k;
        self.confirm_delete = None;
        if let Some(p) = combo.picks.get(k) {
            self.name = p.label();
        }
    }

    /// serve の一覧・保存・変更の結果（`catalog_items` と同じ形）を受け取る
    pub fn take(&mut self, v: &Value) {
        self.listed = v["catalog"].as_str().unwrap_or_default().to_string();
        if let Some(x) = v["vendor"].as_str() {
            self.vendor = x.to_string();
        }
        self.items = v["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| {
                        let name = x["name"].as_str().unwrap_or_default().to_string();
                        let description = x["description"].as_str().unwrap_or_default().to_string();
                        Item {
                            new_name: name.clone(),
                            new_desc: description.clone(),
                            name,
                            description,
                            blocks: x["blocks"].as_u64().unwrap_or(0) as usize,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// 窓を描く。`ready` は読み込んだ系が今の組合せ（保存できる）か。頼む操作を返す
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        combo: &Combo,
        rows: &[LensRow],
        ready: bool,
    ) -> Option<Req> {
        let mut open = self.open;
        let mut req = None;
        egui::Window::new(tr("ユーザーカタログに保存"))
            .open(&mut open)
            .default_width(480.0)
            .show(ctx, |ui| req = self.ui(ui, combo, rows, ready));
        self.open = open;
        req
    }

    fn ui(&mut self, ui: &mut Ui, combo: &Combo, rows: &[LensRow], ready: bool) -> Option<Req> {
        let weak = theme::weak_text(ui.visuals());
        let n = combo.picks.len();
        if n == 0 {
            ui.weak(tr("組合せに要素がありません"));
            return None;
        }
        self.from = self.from.min(n - 1);
        self.to = self.to.clamp(self.from, n - 1);
        let label = |k: usize| format!("{}. {}", k + 1, combo.picks[k].label());
        let mut req = None;
        egui::Grid::new("catsave")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new(tr("要素")).color(weak));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("catsave_from")
                        .selected_text(label(self.from))
                        .show_ui(ui, |ui| {
                            for k in 0..n {
                                ui.selectable_value(&mut self.from, k, label(k));
                            }
                        });
                    ui.label("〜");
                    egui::ComboBox::from_id_salt("catsave_to")
                        .selected_text(label(self.to.max(self.from)))
                        .show_ui(ui, |ui| {
                            for k in self.from..n {
                                ui.selectable_value(&mut self.to, k, label(k));
                            }
                        });
                });
                ui.end_row();
                ui.label(RichText::new(tr("カタログ")).color(weak));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.catalog)
                            .hint_text(tr("ユーザーカタログの .json"))
                            .desired_width(260.0),
                    );
                    // レンズカタログのフォルダにある lensbench 形式のカタログ
                    let mut known: Vec<&str> =
                        rows.iter().filter_map(|r| r.source.as_deref()).collect();
                    known.sort();
                    known.dedup();
                    if !known.is_empty() {
                        egui::ComboBox::from_id_salt("catsave_known")
                            .selected_text("⏷")
                            .width(24.0)
                            .show_ui(ui, |ui| {
                                for c in known {
                                    if ui.selectable_label(self.catalog == c, c).clicked() {
                                        self.catalog = c.to_string();
                                    }
                                }
                            })
                            .response
                            .on_hover_text(tr("カタログブラウザに並んでいるユーザーカタログ"));
                    }
                    if ui
                        .small_button("…")
                        .on_hover_text(tr("新しいカタログか既存のカタログを選ぶ"))
                        .clicked()
                    {
                        if let Some(p) = crate::files::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_file_name("my_catalog.json")
                            .save_file()
                        {
                            self.catalog = p.display().to_string();
                        }
                    }
                });
                ui.end_row();
                ui.label(RichText::new(tr("名前")).color(weak));
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label(RichText::new(tr("説明")).color(weak));
                ui.text_edit_singleline(&mut self.description);
                ui.end_row();
                ui.label(RichText::new(tr("メーカー")).color(weak))
                    .on_hover_text(tr(
                        "カタログブラウザに出すメーカー（空ならファイルのあるフォルダの名前）",
                    ));
                ui.text_edit_singleline(&mut self.vendor);
                ui.end_row();
            });
        let cat = self.catalog.trim().to_string();
        let exists = self.listed == cat && self.items.iter().any(|i| i.name == self.name.trim());
        ui.horizontal(|ui| {
            if exists {
                ui.checkbox(&mut self.replace, tr("同じ名前の項目を置き換える"));
            }
            let ok = ready
                && !cat.is_empty()
                && !self.name.trim().is_empty()
                && (!exists || self.replace);
            let ids = combo.ids();
            let blocks: Vec<&String> = if self.from == self.to {
                vec![&ids[self.from]]
            } else {
                vec![&ids[self.from], &ids[self.to]]
            };
            if ui
                .add_enabled(ok, egui::Button::new(tr("💾 保存")))
                .on_hover_text(tr("面の表で変えた値は玉の面に焼き込みます"))
                .clicked()
            {
                req = Some((
                    "catalog_add",
                    json!({
                        "catalog": cat, "name": self.name.trim(), "blocks": blocks,
                        "description": self.description, "replace": self.replace,
                        "vendor": self.vendor.trim(),
                    }),
                ));
            }
            if !cat.is_empty() && ui.button(tr("一覧を読む")).clicked() {
                req = Some(("catalog_items", json!({ "catalog": cat })));
            }
        });
        if let Some((t, err)) = &self.note {
            let c = if *err {
                ui.visuals().error_fg_color
            } else {
                weak
            };
            ui.colored_label(c, t);
        }
        if !ready {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                tr("組合せの系を作り直してから保存できます（自動で作り直すか「▶ 系を作る」）"),
            );
        }
        let edits: usize = combo.picks[self.from..=self.to]
            .iter()
            .map(|p| p.edits.len())
            .sum();
        if edits > 0 {
            ui.label(
                RichText::new(trf(
                    "面の表で変えた値 {} 件を焼き込みます（玉だけ）",
                    &[&edits],
                ))
                .small()
                .color(weak),
            );
        }
        // 読んだカタログの項目: 名前・説明の変更と削除
        if self.listed == cat && !cat.is_empty() {
            ui.separator();
            ui.label(RichText::new(trf("{} の項目（{} 個）", &[&cat, &self.items.len()])).strong());
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(220.0)
                .show(ui, |ui| {
                    for it in &mut self.items {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [130.0, 20.0],
                                egui::TextEdit::singleline(&mut it.new_name),
                            )
                            .on_hover_text(tr("名前"));
                            ui.add_sized(
                                [170.0, 20.0],
                                egui::TextEdit::singleline(&mut it.new_desc).hint_text(tr("説明")),
                            );
                            ui.label(RichText::new(trf("{} ブロック", &[&it.blocks])).small().color(weak));
                            let changed = it.new_name.trim() != it.name || it.new_desc != it.description;
                            if ui
                                .add_enabled(
                                    changed && !it.new_name.trim().is_empty(),
                                    egui::Button::new(tr("変更")),
                                )
                                .clicked()
                            {
                                req = Some((
                                    "catalog_edit",
                                    json!({ "catalog": cat, "name": it.name,
                                            "rename": it.new_name.trim(), "description": it.new_desc }),
                                ));
                            }
                            if self.confirm_delete.as_deref() == Some(it.name.as_str()) {
                                if ui
                                    .button(RichText::new(tr("本当に消す")).color(ui.visuals().error_fg_color))
                                    .clicked()
                                {
                                    req = Some((
                                        "catalog_edit",
                                        json!({ "catalog": cat, "name": it.name, "delete": true }),
                                    ));
                                }
                            } else if ui.button("🗑").on_hover_text(tr("項目を消す")).clicked() {
                                self.confirm_delete = Some(it.name.clone());
                            }
                        });
                    }
                });
        }
        if req.is_some() {
            self.confirm_delete = None;
            self.note = None;
        }
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_listing() {
        let mut c = CatSave::default();
        c.take(&json!({ "catalog": "a.json", "vendor": "MY", "items": [
            { "name": "L1", "description": "d", "blocks": 1 }] }));
        assert_eq!(c.listed, "a.json");
        assert_eq!(c.vendor, "MY");
        assert_eq!(c.items[0].new_name, "L1");
        assert_eq!(c.items[0].blocks, 1);
    }
}
