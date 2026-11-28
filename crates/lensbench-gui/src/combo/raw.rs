//! 組合せの欄で書けないブロックをそのまま持つ要素（[`ElemKind::Raw`]）。開いたプロジェクトのブロックを失わない
//! ために使う（文書モデル 1 段目）。中身は [`Pick::extra`]（id を除くブロックの JSON）

use egui::{RichText, Ui};
use serde_json::{json, Value};

use super::{ElemKind, Pick};
use crate::i18n::{tr, trf};

impl Pick {
    /// ブロックの JSON をそのまま持つ要素
    pub(crate) fn raw(block: &Value) -> Pick {
        let mut extra = block.clone();
        if let Some(o) = extra.as_object_mut() {
            o.remove("id");
        }
        Pick {
            extra,
            ..Pick::new(ElemKind::Raw)
        }
    }

    /// そのまま持つブロック（id 付き）
    pub(super) fn raw_block(&self, id: &str) -> Value {
        let mut b = self.extra.clone();
        if !b.is_object() {
            b = json!({});
        }
        b["id"] = json!(id);
        b
    }

    /// そのまま持つ合流のブロック（並びの最後に置き、後に空気を置かない）
    pub(crate) fn is_join(&self) -> bool {
        self.kind == ElemKind::Raw && self.extra["type"] == "join"
    }

    /// そのまま持つブロックが絞りか（絞りのブロック・前面を絞りにした玉・絞りを残す取り込み）
    pub(super) fn raw_is_stop(&self) -> bool {
        let b = &self.extra;
        match b["type"].as_str() {
            Some("stop") => true,
            Some("lens") => b["stop"] == true,
            Some("zmx") => b["stop"] != false,
            _ => false,
        }
    }

    /// 一覧・図の名前（ブロックの種類）
    pub(super) fn raw_label(&self) -> String {
        if self.is_join() {
            let bs = self.extra["splitter"].as_str().unwrap_or("?");
            let port = self.extra["port"].as_str().unwrap_or("?").to_uppercase();
            return trf("Join {} port {}", &[&bs, &port]);
        }
        trf("Block ({})", &[&self.extra["type"].as_str().unwrap_or("?")])
    }
}

/// そのまま持つブロックの欄: JSON を書き換える（読めない JSON は当てずに印を出す）
pub(super) fn raw_fields(ui: &mut Ui, p: &mut Pick, k: usize) {
    let id = ui.scope_id().with(("raw", k));
    let now = serde_json::to_string_pretty(&p.extra).unwrap_or_default();
    let mut text: String = ui
        .data_mut(|d| d.get_temp(id))
        .unwrap_or_else(|| now.clone());
    let block = |t: &str| {
        serde_json::from_str::<Value>(t)
            .ok()
            .filter(|v| v.get("type").is_some_and(Value::is_string))
            .filter(|v| !crate::combo_from::is_branched_bs(v))
            .map(|mut v| {
                v.as_object_mut().map(|o| o.remove("id"));
                v
            })
    };
    // 欄はパネルの下に隠れやすいので、JSON は別の窓で直す（要素の欄を 1 つにするまで）
    let open_id = id.with("open");
    let mut open: bool = ui.data_mut(|d| d.get_temp(open_id)).unwrap_or(false);
    // 狭い行でも押せるように、説明はボタンのホバーに
    if ui
        .button(tr("Edit JSON…"))
        .on_hover_text(tr(
            "Kept as written in the file (the combination has no fields for it)",
        ))
        .clicked()
    {
        open = true;
    }
    let mut keep = open;
    egui::Window::new(p.raw_label())
        .id(id.with("win"))
        .open(&mut keep)
        .default_size([420.0, 260.0])
        .resizable(true)
        .collapsible(false)
        .show(ui.ctx(), |ui| {
            let lbl = ui.label(RichText::new(tr("Edit the JSON of the block:")).weak());
            let r = egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .id(id.with("text"))
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(lbl.id)
                })
                .inner;
            match block(&text) {
                Some(v) if r.changed() => p.extra = v,
                // 外で変わった（元に戻すなど）: 書き直す
                Some(v) if !r.has_focus() && v != p.extra => text = now.clone(),
                Some(_) => {}
                None => {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        tr("Not a block (JSON with \"type\", no branches)"),
                    );
                }
            }
        });
    // 窓を閉じている間は今の中身に合わせる
    if !keep {
        text = now;
    }
    ui.data_mut(|d| d.insert_temp(open_id, keep));
    ui.data_mut(|d| d.insert_temp(id, text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_keeps_the_block() {
        let b = json!({ "type": "lens", "id": "X", "stop": true, "source": { "surfaces": [] } });
        let p = Pick::raw(&b);
        assert!(p.extra.get("id").is_none());
        assert_eq!(p.raw_block("Y")["id"], "Y");
        assert_eq!(p.raw_block("Y")["source"], b["source"]);
        assert!(p.is_stop());
        assert!(!Pick::raw(&json!({ "type": "zmx", "path": "a.zmx", "stop": false })).is_stop());
        assert!(Pick::raw(&json!({ "type": "zmx", "path": "a.zmx" })).is_stop());
        assert_eq!(p.label(), "Block (lens)");
    }

    /// 窓の JSON を書き換えると要素に入る。読めない JSON は入れない。外で変わったら欄を書き直す
    #[test]
    fn raw_window_edits() {
        let ctx = egui::Context::default();
        let mut p = Pick::raw(&json!({ "type": "stop", "semi": 1.0 }));
        let mut base = egui::Id::NULL;
        let mut t = 0.0;
        let mut frame = |p: &mut Pick, events: Vec<egui::Event>, base: &mut egui::Id| {
            t += 1.0;
            let input = egui::RawInput {
                events,
                time: Some(t),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    *base = ui.scope_id().with(("raw", 0usize));
                    raw_fields(ui, p, 0)
                });
            });
        };
        frame(&mut p, vec![], &mut base);
        ctx.data_mut(|d| d.insert_temp(base.with("open"), true));
        frame(&mut p, vec![], &mut base);
        let text_id = base.with("text");
        ctx.memory_mut(|m| m.request_focus(text_id));
        frame(&mut p, vec![], &mut base);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let typed = |s: &str| vec![select_all.clone(), egui::Event::Text(s.into())];
        frame(&mut p, typed(r#"{"type": "stop", "semi": 2.5}"#), &mut base);
        assert_eq!(p.extra, json!({ "type": "stop", "semi": 2.5 }));
        // 読めない JSON・分岐のある BS は入れない
        frame(&mut p, typed(r#"{"semi": 3}"#), &mut base);
        assert_eq!(p.extra["semi"], 2.5);
        frame(&mut p, typed(r#"{"type": "splitter", "t": []}"#), &mut base);
        assert_eq!(p.extra["type"], "stop");
        // 外で変わった（元に戻す）: 欄を離れていれば書き直す
        frame(&mut p, typed(r#"{"type": "stop", "semi": 2.5}"#), &mut base);
        ctx.memory_mut(|m| m.surrender_focus(text_id));
        p.extra = json!({ "type": "stop", "semi": 1.0 });
        frame(&mut p, vec![], &mut base);
        let text: String = ctx.data_mut(|d| d.get_temp(base).unwrap());
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), p.extra);
    }
}
