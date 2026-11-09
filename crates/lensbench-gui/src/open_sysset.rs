//! 開いた系（.zmx・系ファイル・プロジェクト）の系の設定。
//!
//! serve の `system_data` の値（系ファイルと同じ形）をそのまま欄にし、変えた項目を `edit` の `set_system_data` で送る。
//! 項目: 名前・波長と重み・主波長・視野（種類・正規化・点・重み・ケラレ係数）・開口・テレセントリック・エイミング・
//! アフォーカル・環境（温度・気圧・屈折率の補正）・大域座標の基準面 GLRS・OPD の基準 ROPD・膜ファイル。

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
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

const FIELD_TYPES: [(&str, &str); 4] = [
    ("Angle", "角度"),
    ("ObjH", "物体高"),
    ("ParH", "近軸像高"),
    ("RealH", "実像高"),
];
const APERTURES: [(&str, &str, &str); 6] = [
    ("Epd", "入射瞳径", " mm"),
    ("FNum", "像空間 F 値", ""),
    ("Na", "物体空間 NA", ""),
    ("Float", "絞りの径で決まる", " mm"),
    ("Pwfn", "近軸作動 F 値", ""),
    ("Cone", "物体の円錐角", "°"),
];
const AIMING: [(&str, &str); 3] = [("Off", "なし"), ("Paraxial", "近軸"), ("Real", "実光線")];
const AFOCAL_UNITS: [&str; 6] = [
    "Microradians",
    "Milliradians",
    "Radians",
    "ArcSeconds",
    "ArcMinutes",
    "Degrees",
];
const VIGNETTING: [&str; 5] = ["vdx", "vdy", "vcx", "vcy", "van"];

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// 数の欄。変えたら true（ドラッグ中・入力中かは busy に）
fn drag(ui: &mut Ui, v: &mut f64, speed: f64, suffix: &str, busy: &mut bool) -> bool {
    let r = ui.add(
        egui::DragValue::new(v)
            .speed(speed)
            .max_decimals(8)
            .suffix(suffix),
    );
    *busy |= r.dragged() || r.has_focus();
    r.changed()
}

/// 欄を描く。送る edit の操作（set_system_data）があれば返す。ns は面の数
pub fn ui(ui: &mut Ui, st: &mut State, ns: usize) -> Option<Value> {
    let weak = theme::weak_text(ui.visuals());
    let Some(d) = st.data.as_mut() else {
        ui.weak(tr("読み込み中…"));
        return None;
    };
    let id = ui.id().with("open_sysset");
    let mut changed: Vec<&'static str> = Vec::new();
    let mut busy = false;

    // 名前
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("名前")).color(weak));
        let mut name = d["name"].as_str().unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
        busy |= r.has_focus();
        if r.changed() {
            d.insert("name".into(), json!(name));
            changed.push("name");
        }
    });

    // 開口
    ui.separator();
    ui.strong(tr("開口"));
    ui.horizontal_wrapped(|ui| {
        let ap = d["aperture"].as_array().cloned().unwrap_or_default();
        let mut kind = ap
            .first()
            .and_then(Value::as_str)
            .unwrap_or("Epd")
            .to_string();
        let mut val = ap.get(1).map(num).unwrap_or(0.0);
        let (_, label, unit) = APERTURES
            .iter()
            .find(|a| a.0 == kind)
            .copied()
            .unwrap_or(APERTURES[0]);
        let before = kind.clone();
        egui::ComboBox::from_id_salt(id.with("ap"))
            .selected_text(tr(label))
            .show_ui(ui, |ui| {
                for (k, l, _) in APERTURES {
                    ui.selectable_value(&mut kind, k.to_string(), tr(l));
                }
            });
        let mut ch = kind != before;
        if drag(ui, &mut val, 0.01, unit, &mut busy) {
            ch = true;
        }
        if ch {
            d.insert("aperture".into(), json!([kind, val]));
            changed.push("aperture");
        }
        let mut tele = d["telecentric"].as_bool().unwrap_or(false);
        if ui
            .checkbox(&mut tele, tr("物体側テレセントリック"))
            .changed()
        {
            d.insert("telecentric".into(), json!(tele));
            changed.push("telecentric");
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("光線エイミング")).color(weak));
        let mut a = d["aiming"].as_str().unwrap_or("Off").to_string();
        for (k, l) in AIMING {
            if ui.radio_value(&mut a, k.to_string(), tr(l)).changed() {
                d.insert("aiming".into(), json!(a.clone()));
                changed.push("aiming");
            }
        }
        ui.separator();
        let mut af = d["afocal"].as_bool().unwrap_or(false);
        if ui.checkbox(&mut af, tr("アフォーカル")).changed() {
            d.insert("afocal".into(), json!(af));
            changed.push("afocal");
        }
        if af {
            let mut u = d["afocal_unit"]
                .as_str()
                .unwrap_or("Milliradians")
                .to_string();
            let before = u.clone();
            egui::ComboBox::from_id_salt(id.with("afu"))
                .selected_text(u.clone())
                .show_ui(ui, |ui| {
                    for k in AFOCAL_UNITS {
                        ui.selectable_value(&mut u, k.to_string(), k);
                    }
                });
            if u != before {
                d.insert("afocal_unit".into(), json!(u));
                changed.push("afocal_unit");
            }
        }
    });

    // 視野
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.strong(tr("視野"));
        let ft = d["field_type"].clone();
        match ft.as_str() {
            Some(cur) => {
                let mut k = cur.to_string();
                let label = FIELD_TYPES
                    .iter()
                    .find(|x| x.0 == cur)
                    .map_or(cur.to_string(), |x| tr(x.1).to_string());
                egui::ComboBox::from_id_salt(id.with("ft"))
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (key, l) in FIELD_TYPES {
                            ui.selectable_value(&mut k, key.to_string(), tr(l));
                        }
                    });
                if k != cur {
                    d.insert("field_type".into(), json!(k));
                    changed.push("field_type");
                }
            }
            None => {
                // 条件で定める視野（面の番号の条件）。形は JSON で見せる
                ui.label(RichText::new(trf("条件で定める視野 {}", &[&ft])).color(weak));
            }
        }
        let mut norm = d["field_norm"].as_str().unwrap_or("Radial").to_string();
        let before = norm.clone();
        ui.label(RichText::new(tr("正規化")).color(weak));
        ui.radio_value(&mut norm, "Radial".into(), tr("半径"));
        ui.radio_value(&mut norm, "Rect".into(), tr("矩形"));
        if norm != before {
            d.insert("field_norm".into(), json!(norm));
            changed.push("field_norm");
        }
    });
    let mut fields: Vec<Map<String, Value>> = d["fields"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_object().cloned()).collect())
        .unwrap_or_default();
    let mut fch = false;
    let mut remove = None;
    egui::Grid::new(id.with("fields"))
        .num_columns(10)
        .spacing([6.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for h in ["#", "x", "y", tr("重み")] {
                ui.label(RichText::new(h).color(weak));
            }
            for h in VIGNETTING {
                ui.label(RichText::new(h).color(weak));
            }
            ui.end_row();
            for (i, f) in fields.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                for k in ["x", "y", "w"].into_iter().chain(VIGNETTING) {
                    let mut v = num(&f[k]);
                    if drag(ui, &mut v, 0.01, "", &mut busy) {
                        f.insert(k.into(), json!(v));
                        fch = true;
                    }
                }
                if ui
                    .add(egui::Button::new("✖").small())
                    .on_hover_text(tr("外す"))
                    .clicked()
                {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove.filter(|_| fields.len() > 1) {
        fields.remove(i);
        fch = true;
    }
    if ui.button(tr("＋ 視野を足す")).clicked() {
        let mut f = fields.last().cloned().unwrap_or_default();
        for k in ["x", "y"].into_iter().chain(VIGNETTING) {
            f.insert(k.into(), json!(0.0));
        }
        f.insert("w".into(), json!(1.0));
        fields.push(f);
        fch = true;
    }
    if fch {
        d.insert(
            "fields".into(),
            Value::Array(fields.into_iter().map(Value::Object).collect()),
        );
        changed.push("fields");
    }

    // 波長
    ui.separator();
    ui.strong(tr("波長"));
    let mut wl: Vec<f64> = d["wavelengths"]
        .as_array()
        .map(|a| a.iter().map(num).collect())
        .unwrap_or_default();
    let mut wt: Vec<f64> = d["weights"]
        .as_array()
        .map(|a| a.iter().map(num).collect())
        .unwrap_or_default();
    wt.resize(wl.len(), 1.0);
    let mut primary = d["primary"].as_u64().unwrap_or(0) as usize;
    let mut wch = false;
    let mut wremove = None;
    egui::Grid::new(id.with("waves"))
        .num_columns(4)
        .spacing([6.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for h in ["#", tr("波長 [µm]"), tr("重み"), tr("主")] {
                ui.label(RichText::new(h).color(weak));
            }
            ui.end_row();
            for i in 0..wl.len() {
                ui.label((i + 1).to_string());
                if drag(ui, &mut wl[i], 0.001, "", &mut busy) {
                    wch = true;
                }
                if drag(ui, &mut wt[i], 0.01, "", &mut busy) {
                    wch = true;
                }
                if ui.radio(primary == i, "").clicked() && primary != i {
                    primary = i;
                    wch = true;
                }
                if ui
                    .add(egui::Button::new("✖").small())
                    .on_hover_text(tr("外す"))
                    .clicked()
                {
                    wremove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = wremove.filter(|_| wl.len() > 1) {
        wl.remove(i);
        wt.remove(i);
        if primary >= i && primary > 0 {
            primary -= 1;
        }
        wch = true;
    }
    if ui.button(tr("＋ 波長を足す")).clicked() {
        wl.push(wl.last().copied().unwrap_or(0.55));
        wt.push(1.0);
        wch = true;
    }
    if wch {
        d.insert("wavelengths".into(), json!(wl));
        d.insert("weights".into(), json!(wt));
        d.insert(
            "primary".into(),
            json!(primary.min(wl.len().saturating_sub(1))),
        );
        changed.extend(["wavelengths", "weights", "primary"]);
    }

    // 環境
    ui.separator();
    ui.strong(tr("環境"));
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("温度")).color(weak));
        let mut t = num(&d["temperature"]);
        if drag(ui, &mut t, 0.1, " °C", &mut busy) {
            d.insert("temperature".into(), json!(t));
            changed.push("temperature");
        }
        ui.label(RichText::new(tr("気圧")).color(weak));
        let mut p = num(&d["pressure"]);
        if drag(ui, &mut p, 0.01, " atm", &mut busy) {
            d.insert("pressure".into(), json!(p));
            changed.push("pressure");
        }
        let mut adj = d["adjust_index"].as_bool().unwrap_or(false);
        if ui
            .checkbox(&mut adj, tr("屈折率を温度・気圧で補正"))
            .changed()
        {
            d.insert("adjust_index".into(), json!(adj));
            changed.push("adjust_index");
        }
    });

    // 座標・OPD・膜
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("大域座標の基準面 (GLRS)")).color(weak));
        let mut g = d["glrs"].as_u64().unwrap_or(0) as usize;
        let r = ui.add(egui::DragValue::new(&mut g).range(0..=ns.saturating_sub(1)));
        busy |= r.dragged() || r.has_focus();
        if r.changed() {
            d.insert("glrs".into(), json!(g));
            changed.push("glrs");
        }
        ui.label(RichText::new(tr("OPD の基準 (ROPD)")).color(weak));
        let mut ro = d["ropd"].as_i64().unwrap_or(0);
        let r = ui.add(egui::DragValue::new(&mut ro).range(-1..=4));
        busy |= r.dragged() || r.has_focus();
        if r.changed() {
            d.insert("ropd".into(), json!(ro));
            changed.push("ropd");
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("膜ファイル")).color(weak));
        let mut f = d["coating_file"].as_str().unwrap_or_default().to_string();
        let r = ui.add(
            egui::TextEdit::singleline(&mut f)
                .hint_text(tr("なし（COATING.DAT など）"))
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
