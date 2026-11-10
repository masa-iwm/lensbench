//! レイアウト図・書き出しの光線と書き出しの設定（CLI の `layout` の --field・--wave・--fan・--rays・--from・--to・
//! --color・--width・--segments・--rings・--tube）。値が空・0 の項目は serve の既定（見る面の向きの扇など）。

use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::i18n::tr;

/// 光線の並び（serve の fan。空は既定: 2D は見る面の向きの扇、3D は ring）
const FANS: &[(&str, &str)] = &[
    ("", "既定"),
    ("y", "y の扇"),
    ("x", "x の扇"),
    ("cross", "十字"),
    ("grid", "格子"),
    ("ring", "周（ring）"),
];

/// 色分け（空は既定: 重ね描きなら構成ごと、ほかは視野ごと）
const COLORS: &[(&str, &str)] = &[
    ("", "既定"),
    ("field", "視野"),
    ("wave", "波長"),
    ("cfg", "構成"),
];

/// 表の表示名（翻訳の検査用）
#[cfg(test)]
pub const LABELS: [&str; 9] = [
    "既定",
    "y の扇",
    "x の扇",
    "十字",
    "格子",
    "周（ring）",
    "視野",
    "波長",
    "構成",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RaySet {
    /// 描く視野の番号（1 始まり）。空は全視野
    pub fields: Vec<usize>,
    /// 描く波長の番号（1 始まり）。空は主波長
    pub waves: Vec<usize>,
    pub fan: String,
    /// 光線の本数（0 は既定）
    pub rays: usize,
    /// 描き始める面・描き終える面（0 は既定）
    pub first: usize,
    pub last: usize,
    pub color: String,
    /// 書き出しの PNG の横幅 [px]（0 は既定 1600）
    pub width: usize,
    /// 書き出しの周方向・面の線の分割数（0 は既定 64）
    pub segments: usize,
    /// 書き出しの 3D の半径方向の分割数（0 は既定 16）
    pub rings: usize,
    /// 書き出しの 3D の光線の管の半径 [mm]（0 は既定）
    pub tube: f64,
    /// 機構部品を、取った品の STEP の詳しい形で描く（3D の表示と、3D・STEP の書き出し）
    pub mech_detail: bool,
}

impl RaySet {
    /// serve の layout（export なら layout_export）の params に足す。n_fields・n_waves は今の系の数
    /// （範囲の外の番号は落とす。0 なら数がわからないので落とさない）
    pub fn apply(&self, v: &mut Value, n_fields: usize, n_waves: usize, export: bool) {
        let pick = |a: &[usize], n: usize| -> Vec<usize> {
            a.iter()
                .copied()
                .filter(|&i| i >= 1 && (n == 0 || i <= n))
                .collect()
        };
        let f = pick(&self.fields, n_fields);
        if !f.is_empty() {
            v["field"] = json!(f);
        }
        let w = pick(&self.waves, n_waves);
        if !w.is_empty() {
            v["wave"] = json!(w);
        }
        if !self.fan.is_empty() {
            v["fan"] = json!(self.fan);
        }
        if self.rays > 0 {
            v["rays"] = json!(self.rays);
        }
        if self.first > 0 {
            v["first"] = json!(self.first);
        }
        if self.last > 0 && self.last >= self.first {
            v["last"] = json!(self.last);
        }
        if !self.color.is_empty() {
            v["color"] = json!(self.color);
        }
        if self.mech_detail {
            v["mech_detail"] = json!(true);
        }
        if export {
            for (k, x) in [
                ("width", self.width),
                ("segments", self.segments),
                ("rings", self.rings),
            ] {
                if x > 0 {
                    v[k] = json!(x);
                }
            }
            if self.tube > 0.0 && self.tube.is_finite() {
                v["tube"] = json!(self.tube);
            }
        }
    }

    /// 既定から変えた項目があるか
    pub fn is_default(&self) -> bool {
        *self == RaySet::default()
    }

    /// 設定の欄。変えたら true（図を作り直す）。書き出しだけの項目を変えても false
    pub fn ui(&mut self, ui: &mut Ui, n_fields: usize, n_waves: usize) -> bool {
        let before = self.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("視野"));
            nums_ui(ui, &mut self.fields, n_fields, tr("全視野"));
            ui.separator();
            ui.label(tr("波長"));
            nums_ui(ui, &mut self.waves, n_waves, tr("主波長"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("並び"));
            combo(ui, "lr_fan", &mut self.fan, FANS);
            ui.label(tr("本数"));
            zero_auto(ui, &mut self.rays, 1..=200);
            ui.separator();
            ui.label(tr("描く面"));
            zero_auto(ui, &mut self.first, 0..=9999);
            ui.label("–");
            zero_auto(ui, &mut self.last, 0..=9999);
            ui.separator();
            ui.label(tr("色分け"));
            combo(ui, "lr_color", &mut self.color, COLORS);
            ui.separator();
            #[cfg(not(target_arch = "wasm32"))]
            ui.checkbox(&mut self.mech_detail, tr("機構部品を詳しい形で"))
                .on_hover_text(tr("カタログの品の STEP を取ってあれば（カタログの詳細の「詳しい形（STEP）を取る」）、3D の表示と書き出しで機構部品をその形で描きます。無い部品は簡易外形のまま"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(tr("書き出し"));
            ui.label(tr("PNG の横幅 [px]"));
            zero_auto(ui, &mut self.width, 100..=20000);
            ui.label(tr("分割数（周・面の線）"));
            zero_auto(ui, &mut self.segments, 4..=1024);
            ui.label(tr("分割数（3D の半径）"));
            zero_auto(ui, &mut self.rings, 1..=256);
            ui.label(tr("光線の管の半径 [mm]"));
            ui.add(
                egui::DragValue::new(&mut self.tube)
                    .range(0.0..=100.0)
                    .speed(0.01)
                    .custom_formatter(|x, _| {
                        if x <= 0.0 {
                            tr("既定").to_string()
                        } else {
                            format!("{x}")
                        }
                    }),
            );
            if !self.is_default() && ui.small_button(tr("既定に戻す")).clicked() {
                *self = RaySet::default();
            }
        });
        let mut a = before;
        let mut b = self.clone();
        for r in [&mut a, &mut b] {
            r.width = 0;
            r.segments = 0;
            r.rings = 0;
            r.tube = 0.0;
        }
        a != b
    }
}

/// 番号のトグル（1〜n）。全部外すと既定（label）
fn nums_ui(ui: &mut Ui, sel: &mut Vec<usize>, n: usize, label: &str) {
    if n == 0 {
        ui.weak(label);
        return;
    }
    if ui.selectable_label(sel.is_empty(), label).clicked() {
        sel.clear();
    }
    for i in 1..=n.min(64) {
        let on = sel.contains(&i);
        if ui.selectable_label(on, i.to_string()).clicked() {
            if on {
                sel.retain(|&x| x != i);
            } else {
                sel.push(i);
                sel.sort_unstable();
            }
        }
    }
}

/// 0 を「既定」と表示する整数の欄
fn zero_auto(ui: &mut Ui, x: &mut usize, range: std::ops::RangeInclusive<usize>) {
    let (lo, hi) = (*range.start(), *range.end());
    let mut r = ui.add(
        egui::DragValue::new(x)
            .range(0..=hi)
            .speed(1.0)
            .custom_formatter(|v, _| {
                if v < 0.5 {
                    tr("既定").to_string()
                } else {
                    format!("{v:.0}")
                }
            }),
    );
    // 下限より小さい値（0 は既定）は下限に
    if r.changed() && *x > 0 && *x < lo {
        *x = lo;
        r.mark_changed();
    }
}

fn combo(ui: &mut Ui, id: &str, v: &mut String, items: &[(&str, &'static str)]) {
    let cur = items
        .iter()
        .find(|x| x.0 == v.as_str())
        .map(|x| tr(x.1))
        .unwrap_or(v.as_str())
        .to_string();
    egui::ComboBox::from_id_salt(id)
        .selected_text(cur)
        .show_ui(ui, |ui| {
            for (k, label) in items {
                ui.selectable_value(v, k.to_string(), tr(label));
            }
        });
}

/// 光線の色の番号（パレットの番号）: color は layout の結果の `color`（field・wave・cfg、空なら by_cfg で決める）
pub fn ray_class(r: &Value, color: &str, by_cfg: bool) -> usize {
    let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
    match color {
        "wave" => us(&r["wave"]),
        "field" => us(&r["field"]),
        "cfg" => us(&r["cfg"]) + 1,
        _ if by_cfg => us(&r["cfg"]) + 1,
        _ => us(&r["field"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_default_adds_nothing() {
        let mut v = json!({});
        RaySet::default().apply(&mut v, 3, 2, true);
        assert_eq!(v, json!({}));
    }

    #[test]
    fn apply_drops_out_of_range() {
        let r = RaySet {
            fields: vec![1, 3, 5],
            waves: vec![2, 4],
            fan: "grid".into(),
            rays: 9,
            first: 2,
            last: 1,
            color: "wave".into(),
            width: 800,
            tube: 0.05,
            ..Default::default()
        };
        let mut v = json!({});
        r.apply(&mut v, 3, 3, false);
        assert_eq!(
            v,
            json!({ "field": [1, 3], "wave": [2], "fan": "grid", "rays": 9, "first": 2, "color": "wave" })
        );
        let mut v = json!({});
        r.apply(&mut v, 0, 0, true);
        assert_eq!(v["field"], json!([1, 3, 5]));
        assert_eq!(v["width"], json!(800));
        assert_eq!(v["tube"], json!(0.05));
        assert!(v.get("segments").is_none());
    }

    #[test]
    fn class_by_color() {
        let r = json!({ "field": 2, "wave": 3, "cfg": 0 });
        assert_eq!(ray_class(&r, "wave", false), 3);
        assert_eq!(ray_class(&r, "", true), 1);
        assert_eq!(ray_class(&r, "", false), 2);
        assert_eq!(ray_class(&r, "cfg", false), 1);
    }
}
