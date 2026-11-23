//! レイアウト図・書き出しの光線と書き出しの設定（CLI の `layout` の --field・--wave・--fan・--rays・--from・--to・
//! --color・--width・--segments・--rings・--tube）。値が空・0 の項目は serve の既定（見る面の向きの扇など）。

use egui::Ui;
use lensbench_engine::layout::Pick;
use lensbench_engine::{LayoutExportReq, LayoutReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::{n_, tr};

/// 光線の並び（serve の fan。空は既定: 2D は見る面の向きの扇、3D は ring）
const FANS: &[(&str, &str)] = &[
    ("", n_("Default")),
    ("y", n_("y fan")),
    ("x", n_("x fan")),
    ("cross", n_("Cross")),
    ("grid", n_("Grid")),
    ("ring", n_("Ring")),
];

/// 色分け（空は既定: 重ね描きなら構成ごと、ほかは視野ごと）
const COLORS: &[(&str, &str)] = &[
    ("", n_("Default")),
    ("field", n_("Fields")),
    ("wave", n_("Wavelengths")),
    ("cfg", n_("Config")),
];

/// 表の表示名（翻訳の検査用）
#[cfg(test)]
pub(crate) const LABELS: [&str; 9] = [
    n_("Default"),
    n_("y fan"),
    n_("x fan"),
    n_("Cross"),
    n_("Grid"),
    n_("Ring"),
    n_("Fields"),
    n_("Wavelengths"),
    n_("Config"),
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RaySet {
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
    /// serve の layout の要求に足す。n_fields・n_waves は今の系の数（範囲の外の番号は落とす。0 なら数が
    /// わからないので落とさない）
    pub(crate) fn apply(&self, r: &mut LayoutReq, n_fields: usize, n_waves: usize) {
        let pick = |a: &[usize], n: usize| -> Option<Pick> {
            let v: Vec<usize> = a
                .iter()
                .copied()
                .filter(|&i| i >= 1 && (n == 0 || i <= n))
                .collect();
            (!v.is_empty()).then_some(Pick::List(v))
        };
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let pos = |x: usize| (x > 0).then_some(x);
        r.fields = pick(&self.fields, n_fields);
        r.waves = pick(&self.waves, n_waves);
        r.fan = some(&self.fan);
        r.rays = pos(self.rays);
        r.first = pos(self.first);
        r.last = (self.last > 0 && self.last >= self.first).then_some(self.last);
        r.color = some(&self.color);
        r.mech_detail = self.mech_detail;
    }

    /// layout_export の要求: ファイル file に、layout と書き出しの細かさ
    pub(crate) fn export_req(&self, layout: LayoutReq, file: String) -> LayoutExportReq {
        let pos = |x: usize| (x > 0).then_some(x);
        LayoutExportReq {
            file,
            layout,
            width: pos(self.width),
            segments: pos(self.segments),
            rings: pos(self.rings),
            tube: (self.tube > 0.0 && self.tube.is_finite()).then_some(self.tube),
        }
    }

    /// 既定から変えた項目があるか
    pub(crate) fn is_default(&self) -> bool {
        *self == RaySet::default()
    }

    /// 設定の欄。変えたら true（図を作り直す）。書き出しだけの項目を変えても false
    pub(crate) fn ui(&mut self, ui: &mut Ui, n_fields: usize, n_waves: usize) -> bool {
        let before = self.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Fields"));
            nums_ui(ui, &mut self.fields, n_fields, tr("All fields"));
            ui.separator();
            ui.label(tr("Wavelengths"));
            nums_ui(ui, &mut self.waves, n_waves, tr("Primary wavelength"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Ray pattern"));
            combo(ui, "lr_fan", &mut self.fan, FANS);
            ui.label(tr("Rays"));
            zero_auto(ui, &mut self.rays, 1..=200);
            ui.separator();
            ui.label(tr("Surfaces drawn"));
            zero_auto(ui, &mut self.first, 0..=9999);
            ui.label("–");
            zero_auto(ui, &mut self.last, 0..=9999);
            ui.separator();
            ui.label(tr("Color by"));
            combo(ui, "lr_color", &mut self.color, COLORS);
            ui.separator();
            #[cfg(not(target_arch = "wasm32"))]
            ui.checkbox(&mut self.mech_detail, tr("Detailed mech shapes"))
                .on_hover_text(tr("If the catalog item's STEP has been fetched (\"Fetch detailed shape (STEP)\" in the catalog details), mechanical parts are drawn with that shape in the 3D view and exports. Parts without it keep the simple outline"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(tr("Export"));
            ui.label(tr("PNG width [px]"));
            zero_auto(ui, &mut self.width, 100..=20000);
            ui.label(tr("Segments (circumference, surface lines)"));
            zero_auto(ui, &mut self.segments, 4..=1024);
            ui.label(tr("Segments (3D radial)"));
            zero_auto(ui, &mut self.rings, 1..=256);
            ui.label(tr("Ray tube radius [mm]"));
            ui.add(
                crate::widgets::Num::new(&mut self.tube)
                    .range(0.0..=100.0)
                    .custom_formatter(|x, _| {
                        if x <= 0.0 {
                            tr("Default").to_string()
                        } else {
                            format!("{x}")
                        }
                    }),
            );
            if !self.is_default() && ui.small_button(tr("Reset to default")).clicked() {
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
        crate::widgets::Num::new(x)
            .range(0..=hi)
            .custom_formatter(|v, _| {
                if v < 0.5 {
                    tr("Default").to_string()
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
pub(crate) fn ray_class(r: &Value, color: &str, by_cfg: bool) -> usize {
    let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
    match color {
        "wave" => us(&r["wave"]),
        "field" => us(&r["field"]),
        "cfg" => us(&r["cfg"]) + 1,
        _ if by_cfg => us(&r["cfg"]) + 1,
        _ => us(&r["field"]),
    }
}

/// 光線の色の番号に足す印: 機構部品に切られた後ろの、薄く描く部分（[`split_faint`]）
pub(crate) const FAINT: usize = 1 << 20;

/// 光線の色の番号（[`FAINT`] の印を除く）と、薄く描くか
pub(crate) fn class_of(c: usize) -> (usize, bool) {
    (c & !FAINT, c & FAINT != 0)
}

/// serve の光線 `r` の点を、機構部品に切られる点（`faint`）の前後に分ける。後ろの部分は色の番号に
/// [`FAINT`] の印を付ける（切られる点は両方に入る）
pub(crate) fn split_faint<T: Clone>(r: &Value, class: usize, pts: Vec<T>) -> Vec<(usize, Vec<T>)> {
    match r["faint"].as_u64().map(|k| k as usize) {
        Some(k) if k + 1 < pts.len() => {
            let back = pts[k..].to_vec();
            let mut front = pts;
            front.truncate(k + 1);
            let mut v = Vec::new();
            if front.len() >= 2 {
                v.push((class, front));
            }
            v.push((class | FAINT, back));
            v
        }
        _ => vec![(class, pts)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_faint_keeps_the_cut_point_in_both() {
        let r = serde_json::json!({ "faint": 1 });
        let v = split_faint(&r, 2, vec![0, 1, 2, 3]);
        assert_eq!(v, vec![(2, vec![0, 1]), (2 | FAINT, vec![1, 2, 3])]);
        assert_eq!(class_of(v[1].0), (2, true));
        // 印が無い・最後の点なら分けない
        assert_eq!(
            split_faint(&serde_json::json!({}), 1, vec![0, 1]),
            vec![(1, vec![0, 1])]
        );
        assert_eq!(split_faint(&r, 1, vec![0, 1]), vec![(1, vec![0, 1])]);
        // 最初の点で切られる光線は全部薄く
        let z = serde_json::json!({ "faint": 0 });
        assert_eq!(
            split_faint(&z, 1, vec![0, 1]),
            vec![(1 | FAINT, vec![0, 1])]
        );
    }

    #[test]
    fn apply_default_adds_nothing() {
        let mut r = LayoutReq::default();
        RaySet::default().apply(&mut r, 3, 2);
        assert_eq!(r, LayoutReq::default());
        let e = RaySet::default().export_req(LayoutReq::default(), "a.svg".into());
        assert_eq!(
            (e.width, e.segments, e.rings, e.tube),
            (None, None, None, None)
        );
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
        let mut q = LayoutReq::default();
        r.apply(&mut q, 3, 3);
        assert_eq!(
            q,
            LayoutReq {
                fields: Some(Pick::List(vec![1, 3])),
                waves: Some(Pick::List(vec![2])),
                fan: Some("grid".into()),
                rays: Some(9),
                first: Some(2),
                color: Some("wave".into()),
                ..Default::default()
            }
        );
        let mut q = LayoutReq::default();
        r.apply(&mut q, 0, 0);
        assert_eq!(q.fields, Some(Pick::List(vec![1, 3, 5])));
        let e = r.export_req(q, "a.png".into());
        assert_eq!(e.width, Some(800));
        assert_eq!(e.tube, Some(0.05));
        assert_eq!(e.segments, None);
    }

    #[test]
    fn class_by_color() {
        let r = serde_json::json!({ "field": 2, "wave": 3, "cfg": 0 });
        assert_eq!(ray_class(&r, "wave", false), 3);
        assert_eq!(ray_class(&r, "", true), 1);
        assert_eq!(ray_class(&r, "", false), 2);
        assert_eq!(ray_class(&r, "cfg", false), 1);
    }
}
