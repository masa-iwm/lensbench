//! 開いた系（組合せでない系）の玉・群の定義と機構部品の欄（serve の edit の define_unit・set_unit・remove_unit・
//! add_mech・set_mech・remove_mech）。面の表のパネルの下に出す。

use egui::{RichText, Ui};
use lensbench_core::mech::{Bore, MechPart};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};

/// 範囲の文字（"3-5, 9-12"・"7"）から [[最初, 最後], …]
pub fn parse_spans(s: &str) -> Result<Vec<[usize; 2]>, String> {
    let mut v = Vec::new();
    for t in s
        .split([',', '、'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let n = |x: &str| {
            x.trim()
                .parse::<usize>()
                .map_err(|_| trf("面の範囲 {} が読めない（3-5, 9-12 のように）", &[&t]))
        };
        let (a, b) = match t.split_once(['-', '–', '〜', '~']) {
            Some((a, b)) => (n(a)?, n(b)?),
            None => (n(t)?, n(t)?),
        };
        v.push([a, b]);
    }
    if v.is_empty() {
        return Err(tr("面の範囲が空").into());
    }
    Ok(v)
}

pub fn spans_text(s: &[[usize; 2]]) -> String {
    s.iter()
        .map(|x| {
            if x[0] == x[1] {
                x[0].to_string()
            } else {
                format!("{}-{}", x[0], x[1])
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 選んだ玉・群の名前・範囲の書きかけ（for は今の玉・群の名前。変われば読み直す）
#[derive(Debug, Clone, Default)]
pub struct UnitBuf {
    pub of: String,
    pub name: String,
    pub spans: String,
}

/// 新しい玉・群の書きかけと、機構部品の書きかけ
#[derive(Debug, Clone, Default)]
pub struct Edits {
    pub unit: UnitBuf,
    pub new_name: String,
    pub new_spans: String,
    /// serve の今の機構部品
    pub mechs: Vec<MechPart>,
    /// 欄で変えている機構部品（serve の分と同じ数）
    pub draft: Vec<MechPart>,
    pub error: Option<String>,
}

impl Edits {
    /// serve の surface の mechanics
    pub fn set_mechs(&mut self, v: &Value) {
        self.mechs = serde_json::from_value(v.clone()).unwrap_or_default();
        self.draft = self.mechs.clone();
    }
}

/// 選んだ玉・群の名前・範囲を変える・外す欄。moved は動かしてある（範囲を変える・外すのは 0 のときだけ）
pub fn unit_ui(
    ui: &mut Ui,
    e: &mut Edits,
    name: &str,
    spans: &[[usize; 2]],
    moved: bool,
) -> Vec<Value> {
    let mut ops = Vec::new();
    let weak = crate::theme::weak_text(ui.visuals());
    if e.unit.of != name {
        e.unit = UnitBuf {
            of: name.into(),
            name: name.into(),
            spans: spans_text(spans),
        };
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("名前")).color(weak));
        let r = ui.add(egui::TextEdit::singleline(&mut e.unit.name).desired_width(90.0));
        if r.lost_focus() && e.unit.name.trim() != name && !e.unit.name.trim().is_empty() {
            ops.push(json!({ "op": "set_unit", "unit": name, "name": e.unit.name.trim() }));
        }
        ui.label(RichText::new(tr("面の範囲")).color(weak));
        ui.add_enabled_ui(!moved, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut e.unit.spans)
                    .desired_width(110.0)
                    .hint_text("3-5, 9-12"),
            );
            if r.lost_focus() && e.unit.spans != spans_text(spans) {
                match parse_spans(&e.unit.spans) {
                    Ok(s) => ops.push(json!({ "op": "set_unit", "unit": name, "spans": s })),
                    Err(x) => e.error = Some(x),
                }
            }
            if ui
                .small_button(tr("外す"))
                .on_hover_text(tr("玉・群の定義を外す（面はそのまま）"))
                .clicked()
            {
                ops.push(json!({ "op": "remove_unit", "unit": name }));
            }
        })
        .response
        .on_disabled_hover_text(tr(
            "動き・移動を 0 にすると、範囲を変えたり外したりできます",
        ));
    });
    ops
}

/// 玉・群を足す・機構部品の欄。n_surf は面の数
pub fn ui(ui: &mut Ui, e: &mut Edits, n_surf: usize) -> Vec<Value> {
    let mut ops = Vec::new();
    let weak = crate::theme::weak_text(ui.visuals());
    if let Some(x) = &e.error {
        ui.colored_label(ui.visuals().error_fg_color, x);
    }
    ui.label(RichText::new(tr("玉・群を足す")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("名前")).color(weak));
        ui.add(
            egui::TextEdit::singleline(&mut e.new_name)
                .desired_width(80.0)
                .hint_text("G1"),
        );
        ui.label(RichText::new(tr("面の範囲")).color(weak));
        ui.add(
            egui::TextEdit::singleline(&mut e.new_spans)
                .desired_width(110.0)
                .hint_text("3-5, 9-12"),
        );
        if ui.button(tr("＋ 玉・群")).clicked() {
            match parse_spans(&e.new_spans) {
                Ok(s) if !e.new_name.trim().is_empty() => {
                    ops.push(json!({ "op": "define_unit", "name": e.new_name.trim(), "spans": s }));
                    e.error = None;
                    e.new_name.clear();
                    e.new_spans.clear();
                }
                Ok(_) => e.error = Some(tr("玉・群の名前が要る").into()),
                Err(x) => e.error = Some(x),
            }
        }
    });
    ui.label(
        RichText::new(tr(
            "ズーム群・フォーカス群などを面の範囲で作ると、G1.shift・G1.dx などで動かせます（範囲は互いに含むか離れているか。ダブルパスは範囲を 2 つ）。",
        ))
        .small()
        .color(weak),
    );
    ui.separator();
    ui.label(RichText::new(tr("機構部品")).strong());
    if e.draft.len() != e.mechs.len() {
        e.draft = e.mechs.clone();
    }
    let max_s = n_surf.saturating_sub(1).max(1);
    let mut del = None;
    for (i, m) in e.draft.iter_mut().enumerate() {
        ui.push_id(("mech", i), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(90.0));
                ui.label(RichText::new(tr("面")).color(weak));
                ui.add(egui::DragValue::new(&mut m.surface).range(1..=max_s));
                ui.label(RichText::new(tr("ずれ [mm]")).color(weak));
                for x in m.offset.iter_mut() {
                    ui.add(egui::DragValue::new(x).speed(0.1).max_decimals(4));
                }
                ui.label(RichText::new(tr("傾き [°]")).color(weak));
                for x in m.tilt.iter_mut() {
                    ui.add(egui::DragValue::new(x).speed(0.1).max_decimals(4));
                }
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (l, x) in [
                    (tr("長さ"), &mut m.length),
                    (tr("外径の半径"), &mut m.outer_radius),
                    (tr("保持の半径"), &mut m.hold_radius),
                    (tr("ロッドの間隔"), &mut m.rod_spacing),
                ] {
                    ui.label(RichText::new(l).color(weak));
                    ui.add(
                        egui::DragValue::new(x)
                            .range(0.0..=1e5)
                            .speed(0.1)
                            .max_decimals(4)
                            .custom_formatter(|v, _| {
                                if v <= 0.0 {
                                    tr("自動").to_string()
                                } else {
                                    format!("{v}")
                                }
                            }),
                    );
                }
            });
            let mut bdel = None;
            for (k, b) in m.bores.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(trf("穴 {}", &[&(k + 1)])).color(weak));
                    ui.label("z");
                    ui.add(egui::DragValue::new(&mut b.z0).speed(0.1).max_decimals(4));
                    ui.label("–");
                    ui.add(egui::DragValue::new(&mut b.z1).speed(0.1).max_decimals(4));
                    ui.label(RichText::new(tr("半径")).color(weak));
                    ui.add(
                        egui::DragValue::new(&mut b.radius)
                            .range(1e-6..=1e5)
                            .speed(0.1)
                            .max_decimals(4),
                    );
                    if ui.small_button("✖").clicked() {
                        bdel = Some(k);
                    }
                });
            }
            if let Some(k) = bdel {
                m.bores.remove(k);
            }
            ui.horizontal(|ui| {
                if ui.small_button(tr("＋ 穴")).clicked() {
                    let z = m.bores.last().map_or(0.0, |b| b.z1);
                    m.bores.push(Bore {
                        z0: z,
                        z1: z + 10.0,
                        radius: m.bores.last().map_or(12.7, |b| b.radius),
                    });
                }
                if let Some(cur) = e.mechs.get(i) {
                    if cur != m {
                        if ui.button(tr("変更を当てる")).clicked() {
                            let part = serde_json::to_value(&*m).unwrap_or_default();
                            ops.push(json!({ "op": "set_mech", "mech": i, "part": part }));
                        }
                        if ui.small_button(tr("戻す")).clicked() {
                            *m = cur.clone();
                        }
                    }
                }
            });
        });
        ui.separator();
    }
    if let Some(i) = del {
        ops.push(json!({ "op": "remove_mech", "mech": i }));
    }
    if ui.button(tr("＋ 機構部品")).clicked() {
        let n = (1..)
            .map(|k| format!("mech{k}"))
            .find(|x| !e.mechs.iter().any(|m| &m.name == x))
            .unwrap_or_default();
        let part = json!({
            "name": n, "surface": 1, "offset": [0.0, 0.0, 0.0], "tilt": [0.0, 0.0],
            "bores": [{ "z0": 0.0, "z1": 10.0, "radius": 12.7 }],
        });
        ops.push(json!({ "op": "add_mech", "part": part }));
    }
    ui.label(
        RichText::new(tr(
            "機構部品は面の局所座標に置き、穴（z の範囲・半径）の外に出た光線をケラレにします（光線の追跡そのものは変えない）。",
        ))
        .small()
        .color(weak),
    );
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_text_round_trip() {
        assert_eq!(
            parse_spans("3-5, 9–12, 7").unwrap(),
            vec![[3, 5], [9, 12], [7, 7]]
        );
        assert_eq!(spans_text(&[[3, 5], [7, 7]]), "3-5, 7");
        assert!(parse_spans("").is_err());
        assert!(parse_spans("a-3").is_err());
    }
}
