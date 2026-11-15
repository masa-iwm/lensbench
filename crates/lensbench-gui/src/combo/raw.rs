//! 組合せの欄で書けないブロックをそのまま持つ要素（[`ElemKind::Raw`]）。開いたプロジェクトのブロックを失わない
//! ために使う（文書モデル 1 段目、GAR-02）。中身は [`Pick::extra`]（id を除くブロックの JSON）

use egui::{RichText, Ui};
use serde_json::{json, Value};

use super::{ElemKind, Pick};
use crate::i18n::{tr, trf};

impl Pick {
    /// ブロックの JSON をそのまま持つ要素
    pub fn raw(block: &Value) -> Pick {
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
        trf("Block ({})", &[&self.extra["type"].as_str().unwrap_or("?")])
    }
}

/// そのまま持つブロックの欄: JSON を書き換える（読めない JSON は当てずに印を出す）
pub(super) fn raw_fields(ui: &mut Ui, p: &mut Pick, k: usize) {
    let id = ui.id().with(("raw", k));
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
    ui.vertical(|ui| {
        ui.label(
            RichText::new(tr(
                "Kept as written in the file (the combination has no fields for it). Edit the JSON of the block:",
            ))
            .weak(),
        );
        let r = ui.add(
            egui::TextEdit::multiline(&mut text)
                .code_editor()
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        match block(&text) {
            Some(v) if r.changed() => p.extra = v,
            // 外で変わった（元に戻すなど）: 書き直す
            Some(v) if !r.has_focus() && v != p.extra => text = now,
            Some(_) => {}
            None => {
                ui.colored_label(ui.visuals().error_fg_color, tr("Not a block (JSON with \"type\", no branches)"));
            }
        }
    });
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
}
