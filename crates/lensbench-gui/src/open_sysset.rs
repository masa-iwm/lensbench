//! 開いた系（.zmx・系ファイル・プロジェクト）の系の設定。
//!
//! serve の `system_data` の値（系ファイルと同じ形）を組合せと同じ欄（[`crate::sysset::form`]）で描き、
//! 変えた項目を `edit` の `set_system_data` で送る。開いた系だけの項目: 名前・大域座標の基準面 GLRS・
//! OPD の基準 ROPD・膜ファイル。

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::tr;
use crate::sysset::form::{self, SysForm};
use crate::theme;

/// 欄の状態
#[derive(Debug, Clone, Default)]
pub struct State {
    /// serve の system_data の data（欄で変えた値も入る）。読み直す間も前の値を見せる
    pub data: Option<Map<String, Value>>,
    /// 読みに行っている（返事待ち）
    asked: bool,
    /// 系が変わったので読み直す
    need: bool,
    /// まだ送っていない変更（項目の名前）
    pending: Vec<String>,
}

impl State {
    /// 系が変わった（読み直す）
    pub fn stale(&mut self) {
        self.need = true;
    }

    /// 読みに行くなら true（呼んだ側が serve の system_data を送る）
    pub fn want(&mut self) -> bool {
        if (self.data.is_none() || self.need) && !self.asked {
            self.asked = true;
            self.need = false;
            return true;
        }
        false
    }

    pub fn set(&mut self, v: &Value) {
        self.asked = false;
        if let Some(m) = v["data"].as_object() {
            if self.pending.is_empty() {
                self.data = Some(m.clone());
            }
        }
    }
}

/// 欄を描く。送る edit の操作（set_system_data）があれば返す。ns は面の数
pub fn ui(ui: &mut Ui, st: &mut State, ns: usize) -> Option<Value> {
    let weak = theme::weak_text(ui.visuals());
    let Some(d) = st.data.as_mut() else {
        ui.weak(tr("Loading…"));
        return None;
    };
    let mut changed: Vec<&'static str> = Vec::new();
    let mut busy = false;

    // 名前
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Name")).color(weak));
        let mut name = d["name"].as_str().unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
        busy |= r.has_focus();
        if r.changed() {
            d.insert("name".into(), json!(name));
            changed.push("name");
        }
    });
    ui.separator();

    // 組合せと同じ欄
    match SysForm::from_data(d) {
        Ok(mut f) => {
            busy |= form::ui(ui, &mut f, &mut form::NoExtra);
            changed.extend(f.write_data(d));
        }
        Err(e) => {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 60), e);
        }
    }

    // 座標・OPD・膜
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Global coordinate reference (GLRS)")).color(weak));
        let mut g = d["glrs"].as_u64().unwrap_or(0) as usize;
        let r = ui.add(egui::DragValue::new(&mut g).range(0..=ns.saturating_sub(1)));
        busy |= r.dragged() || r.has_focus();
        if r.changed() {
            d.insert("glrs".into(), json!(g));
            changed.push("glrs");
        }
        ui.label(RichText::new(tr("OPD reference (ROPD)")).color(weak));
        let mut ro = d["ropd"].as_i64().unwrap_or(0);
        let r = ui.add(egui::DragValue::new(&mut ro).range(-1..=4));
        busy |= r.dragged() || r.has_focus();
        if r.changed() {
            d.insert("ropd".into(), json!(ro));
            changed.push("ropd");
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Coating file")).color(weak));
        let mut f = d["coating_file"].as_str().unwrap_or_default().to_string();
        let r = ui.add(
            egui::TextEdit::singleline(&mut f)
                .hint_text(tr("None (e.g. COATING.DAT)"))
                .desired_width(280.0),
        );
        busy |= r.has_focus();
        if r.changed() {
            let v = if f.trim().is_empty() {
                Value::Null
            } else {
                json!(f)
            };
            d.insert("coating_file".into(), v);
            changed.push("coating_file");
        }
    });

    for k in changed {
        if !st.pending.iter().any(|p| p == k) {
            st.pending.push(k.to_string());
        }
    }
    // ドラッグ・入力の途中は送らない（離した・確定したときにまとめて）
    if busy || st.pending.is_empty() {
        return None;
    }
    let d = st.data.as_ref()?;
    let data: Map<String, Value> = st
        .pending
        .drain(..)
        .filter_map(|k| d.get(&k).map(|v| (k, v.clone())))
        .collect();
    Some(json!({ "op": "set_system_data", "data": data }))
}
