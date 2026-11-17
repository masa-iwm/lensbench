//! 系の設定の共通の欄。
//!
//! 組合せの設定（[`Combo`]）と開いた系の設定（serve の `system_data`）の両方を [`SysForm`] に移して同じ欄で描き、
//! 描いた後に元へ書き戻す。違うのは書き先と、片方にしかない項目（[`Extra`] の差し込み）だけ。
//! 共通の項目: 開口の種類と値・エイミング・アフォーカル・テレセントリック・浮動開口・視野（種類・正規化・点・重み・
//! ケラレ係数）・波長（重み・主波長・既定の組）・環境（温度・気圧・屈折率の補正）。

use egui::{RichText, Ui};
use lensbench_core::system::AfocalUnit;
use lensbench_project::Environment;
use serde_json::{json, Map, Value};

use super::{
    Aiming, ApKind, FieldKind, FieldPt, FieldSolve, SolveCond, WavePt, AFOCAL_UNITS, PRESETS,
};
use crate::combo::Combo;
use crate::i18n::{tr, trf};
use crate::theme;

/// 系の設定の共通の値
#[derive(Debug, Clone, PartialEq)]
pub struct SysForm {
    pub aperture: ApKind,
    /// 開口の値（F 値・NA など。Float の値は使わないが開いた系の値はそのまま戻す）
    pub ap_value: f64,
    /// 入射瞳径 [mm]（組合せでは種類を変えても残す。開いた系は読んだ値）
    pub epd: f64,
    pub field_kind: FieldKind,
    /// 条件で定める視野の条件。None なら選べない（開いた系）
    pub field_solve: Option<FieldSolve>,
    /// 開いた系の条件で定める視野（system_data の field_type をそのまま見せる。欄では変えない）
    pub field_cond: Option<Value>,
    pub field_rect: bool,
    pub fields: Vec<FieldPt>,
    pub waves: Vec<WavePt>,
    /// 主波長（1 から）
    pub primary: usize,
    pub telecentric: bool,
    pub aiming: Aiming,
    pub afocal: Option<AfocalUnit>,
    pub floating_clip: bool,
    pub env: Environment,
}

/// 片方にしかない項目を共通の欄の間に差し込む
pub trait Extra {
    /// 開口の表の終わりの行（2 列の Grid の中）
    fn aperture_rows(&mut self, _ui: &mut Ui, _f: &mut SysForm) {}
    /// 開口の表の後
    fn after_aperture(&mut self, _ui: &mut Ui, _f: &SysForm) {}
    /// エイミングなどの表の初めの行（2 列の Grid の中）
    fn option_rows(&mut self, _ui: &mut Ui) {}
    /// テレセントリックの欄を出すか
    fn telecentric(&self) -> bool {
        true
    }
}

/// 差し込みなし（開いた系）
pub struct NoExtra;
impl Extra for NoExtra {}

const VIG: [&str; 5] = ["vdx", "vdy", "vcx", "vcy", "van"];

fn ap_key(k: ApKind) -> &'static str {
    match k {
        ApKind::Epd => "epd",
        ApKind::Fnum => "fnum",
        ApKind::Na => "na",
        ApKind::Float => "float",
        ApKind::Pwfn => "pwfn",
        ApKind::Cone => "cone",
    }
}

fn field_key(k: FieldKind) -> Option<&'static str> {
    match k {
        FieldKind::Angle => Some("Angle"),
        FieldKind::ObjectHeight => Some("ObjH"),
        FieldKind::ParaxialImageHeight => Some("ParH"),
        FieldKind::RealImageHeight => Some("RealH"),
        FieldKind::Solve => None,
    }
}

fn aiming_key(a: Aiming) -> &'static str {
    match a {
        Aiming::Off => "off",
        Aiming::Paraxial => "paraxial",
        Aiming::Real => "real",
    }
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

impl SysForm {
    /// 組合せの値から
    pub fn of_combo(c: &Combo) -> SysForm {
        SysForm {
            aperture: c.aperture,
            ap_value: c.ap_value,
            epd: c.epd,
            field_kind: c.field_kind,
            field_solve: Some(c.field_solve.clone()),
            field_cond: None,
            field_rect: c.extra.field_rect,
            fields: c.fields.clone(),
            waves: c.waves.clone(),
            primary: c.primary,
            telecentric: c.telecentric,
            aiming: c.aiming,
            afocal: c.afocal.as_deref().and_then(AfocalUnit::from_code),
            floating_clip: c.floating_clip,
            env: c.extra.environment.unwrap_or_default(),
        }
    }

    /// 組合せへ書き戻す
    pub fn apply_to(&self, c: &mut Combo) {
        c.aperture = self.aperture;
        c.ap_value = self.ap_value;
        c.epd = self.epd;
        c.field_kind = self.field_kind;
        if let Some(s) = &self.field_solve {
            c.field_solve = s.clone();
        }
        c.extra.field_rect = self.field_rect;
        c.fields = self.fields.clone();
        c.waves = self.waves.clone();
        c.primary = self.primary;
        c.telecentric = self.telecentric;
        c.aiming = self.aiming;
        c.afocal = self.afocal.map(|u| u.code().to_ascii_lowercase());
        c.floating_clip = self.floating_clip;
        c.extra.environment = (self.env != Environment::default()).then_some(self.env);
    }

    /// 開いた系の設定（serve の `system_data` の data）から。読めない開口・波長なしは断る
    pub fn from_data(d: &Map<String, Value>) -> Result<SysForm, String> {
        let wl = nums(&d["wavelengths"]);
        if wl.is_empty() {
            return Err(tr("Cannot read the system wavelengths").into());
        }
        let ws = nums(&d["weights"]);
        let ap = d["aperture"][0].as_str();
        let aperture = ApKind::ALL
            .into_iter()
            .find(|&k| Some(ap_key(k)) == ap)
            .ok_or_else(|| tr("Cannot read the system aperture").to_string())?;
        let ft = d["field_type"].as_str();
        let field_kind = FieldKind::ALL
            .into_iter()
            .find(|&k| field_key(k).is_some() && field_key(k) == ft);
        let val = d["aperture"][1].as_f64().unwrap_or(0.0);
        let def = Combo::default();
        let env = Environment {
            temperature_c: d["temperature"].as_f64().unwrap_or(20.0),
            pressure_atm: d["pressure"].as_f64().unwrap_or(1.0),
            adjust_index: d["adjust_index"].as_bool().unwrap_or(false),
        };
        Ok(SysForm {
            aperture,
            // 読んだ値は種類の欄へ。もう一方は組合せの既定（種類を変えたときの初めの値）
            ap_value: if aperture == ApKind::Epd {
                def.ap_value
            } else {
                val
            },
            epd: if aperture == ApKind::Epd {
                val
            } else {
                def.epd
            },
            field_kind: field_kind.unwrap_or_default(),
            field_solve: None,
            field_cond: field_kind.is_none().then(|| d["field_type"].clone()),
            field_rect: d["field_norm"].as_str() == Some("rect"),
            fields: d["fields"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|f| FieldPt {
                            x: f["x"].as_f64().unwrap_or(0.0),
                            y: f["y"].as_f64().unwrap_or(0.0),
                            w: f["w"].as_f64().unwrap_or(1.0),
                            vig: VIG.map(|k| f[k].as_f64().unwrap_or(0.0)),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            waves: wl
                .iter()
                .enumerate()
                .map(|(i, &um)| WavePt {
                    um,
                    w: ws.get(i).copied().unwrap_or(1.0),
                })
                .collect(),
            primary: d["primary"].as_u64().map_or(1, |p| p as usize + 1),
            telecentric: d["telecentric"].as_bool().unwrap_or(false),
            aiming: match d["aiming"].as_str() {
                Some("paraxial") => Aiming::Paraxial,
                Some("real") => Aiming::Real,
                _ => Aiming::Off,
            },
            afocal: (d["afocal"].as_bool() == Some(true)).then(|| {
                d.get("afocal_unit")
                    .and_then(|u| serde_json::from_value(u.clone()).ok())
                    .unwrap_or_default()
            }),
            floating_clip: d["floating_clip"].as_bool().unwrap_or(true),
            env,
        })
    }

    /// 開口の値（EPD は入射瞳径）
    fn value(&self) -> f64 {
        if self.aperture == ApKind::Epd {
            self.epd
        } else {
            self.ap_value
        }
    }

    /// 開いた系の設定へ書き、変わった項目の名前を返す
    pub fn write_data(&self, d: &mut Map<String, Value>) -> Vec<&'static str> {
        let mut new: Vec<(&'static str, Value)> = vec![
            ("aperture", json!([ap_key(self.aperture), self.value()])),
            ("telecentric", json!(self.telecentric)),
            ("floating_clip", json!(self.floating_clip)),
            ("aiming", json!(aiming_key(self.aiming))),
            ("afocal", json!(self.afocal.is_some())),
            (
                "field_norm",
                json!(if self.field_rect { "rect" } else { "radial" }),
            ),
            (
                "wavelengths",
                json!(self.waves.iter().map(|w| w.um).collect::<Vec<_>>()),
            ),
            (
                "weights",
                json!(self.waves.iter().map(|w| w.w).collect::<Vec<_>>()),
            ),
            (
                "primary",
                json!(self.primary.clamp(1, self.waves.len().max(1)) - 1),
            ),
            ("temperature", json!(self.env.temperature_c)),
            ("pressure", json!(self.env.pressure_atm)),
            ("adjust_index", json!(self.env.adjust_index)),
        ];
        if let Some(u) = self.afocal {
            new.push(("afocal_unit", json!(u)));
        }
        if self.field_cond.is_none() {
            if let Some(k) = field_key(self.field_kind) {
                new.push(("field_type", json!(k)));
            }
        }
        // 視野の点は元の項目（ほかの鍵）を残して x・y・重み・ケラレ係数だけ替える
        let old = d["fields"].as_array().cloned().unwrap_or_default();
        let fields: Vec<Value> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut m = old
                    .get(i)
                    .or(old.last())
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                m.insert("x".into(), json!(p.x));
                m.insert("y".into(), json!(p.y));
                m.insert("w".into(), json!(p.w));
                for (k, v) in VIG.iter().zip(p.vig) {
                    m.insert((*k).into(), json!(v));
                }
                Value::Object(m)
            })
            .collect();
        new.push(("fields", Value::Array(fields)));
        let mut changed = Vec::new();
        for (k, v) in new {
            if !d.get(k).is_some_and(|o| same(o, &v)) {
                d.insert(k.into(), v);
                changed.push(k);
            }
        }
        changed
    }
}

/// 同じ値か（数の 1 と 1.0 のような書き方の違いは同じとみる）
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, a)| y.get(k).is_some_and(|b| same(a, b)))
        }
        _ => a == b,
    }
}

/// 数の欄。ドラッグ中・入力中は busy に
fn drag(ui: &mut Ui, dv: egui::DragValue, busy: &mut bool) {
    let r = ui.add(dv);
    *busy |= r.dragged() || r.has_focus();
}

/// 共通の欄を描く。ドラッグ中・入力中なら true（開いた系はその間は送らない）
pub fn ui(ui: &mut Ui, f: &mut SysForm, ex: &mut dyn Extra) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let id = ui.id().with("sysform");
    let mut busy = false;

    // 開口
    egui::Grid::new(id.with("ap"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("Aperture type"));
            let before = f.aperture;
            egui::ComboBox::from_id_salt(id.with("apk"))
                .selected_text(f.aperture.label())
                .show_ui(ui, |ui| {
                    for k in ApKind::ALL {
                        ui.selectable_value(&mut f.aperture, k, k.label());
                    }
                });
            if f.aperture != before && f.aperture.has_value() {
                f.ap_value = f.aperture.initial();
            }
            ui.end_row();
            if f.aperture.has_value() {
                ui.label(f.aperture.label());
                drag(
                    ui,
                    egui::DragValue::new(&mut f.ap_value)
                        .speed(0.01)
                        .range(1e-6..=1e6)
                        .max_decimals(6)
                        .suffix(f.aperture.unit()),
                    &mut busy,
                );
                ui.end_row();
            } else if f.aperture == ApKind::Epd {
                ui.label(tr("Entrance pupil diameter [mm]"));
                drag(
                    ui,
                    egui::DragValue::new(&mut f.epd)
                        .speed(0.1)
                        .range(1e-3..=1e4)
                        .max_decimals(4),
                    &mut busy,
                );
                ui.end_row();
            }
            ex.aperture_rows(ui, f);
        });
    ex.after_aperture(ui, f);
    ui.separator();

    // エイミング・像空間・テレセントリック・浮動開口
    egui::Grid::new(id.with("opt"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ex.option_rows(ui);
            if ex.telecentric() {
                ui.label("");
                ui.checkbox(&mut f.telecentric, tr("Telecentric in object space"));
                ui.end_row();
            }
            ui.label(tr("Ray aiming"));
            ui.horizontal(|ui| {
                for a in Aiming::ALL {
                    ui.radio_value(&mut f.aiming, a, a.label());
                }
            });
            ui.end_row();
            ui.label(tr("Image space"));
            ui.horizontal(|ui| {
                let mut on = f.afocal.is_some();
                if ui.checkbox(&mut on, tr("Afocal")).changed() {
                    f.afocal = on.then(AfocalUnit::default);
                }
                if let Some(u) = f.afocal.as_mut() {
                    egui::ComboBox::from_id_salt(id.with("afu"))
                        .selected_text(u.label())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for x in AFOCAL_UNITS {
                                ui.selectable_value(u, x, x.label());
                            }
                        });
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(
                &mut f.floating_clip,
                tr("Vignette rays outside fixed semi-diameters"),
            )
            .on_hover_text(tr(super::FLOATING_CLIP_HINT));
            ui.end_row();
        });
    ui.separator();

    fields_ui(ui, f, id, &mut busy);
    ui.separator();
    waves_ui(ui, f, id, &mut busy);
    ui.separator();

    // 環境
    ui.label(RichText::new(tr("Environment")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Temperature")).color(weak));
        drag(
            ui,
            egui::DragValue::new(&mut f.env.temperature_c)
                .speed(0.1)
                .range(-273.15..=1e4)
                .suffix(" °C"),
            &mut busy,
        );
        ui.label(RichText::new(tr("Pressure")).color(weak));
        drag(
            ui,
            egui::DragValue::new(&mut f.env.pressure_atm)
                .speed(0.01)
                .range(0.0..=1e3)
                .suffix(" atm"),
            &mut busy,
        );
        ui.checkbox(
            &mut f.env.adjust_index,
            tr("Adjust index to temperature and pressure"),
        );
    });
    busy
}

/// 視野の種類・正規化・点の表
fn fields_ui(ui: &mut Ui, f: &mut SysForm, id: egui::Id, busy: &mut bool) {
    let weak = theme::weak_text(ui.visuals());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Fields")).strong());
        match &f.field_cond {
            Some(ft) => {
                // 条件で定める視野（面の番号の条件）。形は JSON で見せる
                ui.label(RichText::new(trf("Field by condition {}", &[ft])).color(weak));
            }
            None => {
                let solve = f.field_solve.is_some();
                egui::ComboBox::from_id_salt(id.with("fk"))
                    .selected_text(f.field_kind.label())
                    .show_ui(ui, |ui| {
                        for k in FieldKind::ALL {
                            if k != FieldKind::Solve || solve {
                                ui.selectable_value(&mut f.field_kind, k, k.label());
                            }
                        }
                    });
            }
        }
        ui.label(RichText::new(tr("Normalization")).color(weak));
        ui.radio_value(&mut f.field_rect, false, tr("Radial"));
        ui.radio_value(&mut f.field_rect, true, tr("Rectangular"));
    });
    if let (FieldKind::Solve, Some(s)) = (f.field_kind, f.field_solve.as_mut()) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(id.with("sc"))
                .selected_text(s.cond.label())
                .show_ui(ui, |ui| {
                    for k in SolveCond::ALL {
                        ui.selectable_value(&mut s.cond, k, k.label());
                    }
                });
            let two = s.cond != SolveCond::Position;
            ui.label(if two { tr("Surface 1") } else { tr("Surface") });
            ui.add(
                egui::TextEdit::singleline(&mut s.s1)
                    .hint_text(tr("number or name"))
                    .desired_width(70.0),
            );
            if two {
                ui.label(tr("Surface 2"));
                ui.add(
                    egui::TextEdit::singleline(&mut s.s2)
                        .hint_text(tr("number or name"))
                        .desired_width(70.0),
                );
            }
        });
        ui.label(
            RichText::new(tr(
                "Solves the object so the primary-wavelength chief ray meets the condition (x, y below). Cannot be written to .zmx",
            ))
            .small()
            .color(weak),
        );
    }
    // ケラレ係数の列は持っているときか、選んだときだけ出す
    let vid = id.with("show_vig");
    let mut show_vig = ui
        .data(|d| d.get_temp::<bool>(vid))
        .unwrap_or_else(|| f.fields.iter().any(FieldPt::has_vig));
    if ui
        .checkbox(&mut show_vig, tr("Vignetting factors"))
        .changed()
    {
        ui.data_mut(|d| d.insert_temp(vid, show_vig));
    }
    let unit = f.field_kind.unit();
    let mut del = None;
    egui::Grid::new(id.with("fields"))
        .striped(true)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            for h in ["#", "x", "y", tr("Weight")] {
                ui.label(RichText::new(h).color(weak));
            }
            if show_vig {
                for h in VIG {
                    ui.label(RichText::new(h.to_ascii_uppercase()).color(weak));
                }
            }
            ui.end_row();
            for (i, p) in f.fields.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                for v in [&mut p.x, &mut p.y] {
                    drag(
                        ui,
                        egui::DragValue::new(v)
                            .speed(0.05)
                            .max_decimals(6)
                            .suffix(unit),
                        busy,
                    );
                }
                drag(
                    ui,
                    egui::DragValue::new(&mut p.w)
                        .speed(0.05)
                        .range(0.0..=1e6)
                        .max_decimals(4),
                    busy,
                );
                if show_vig {
                    for v in p.vig.iter_mut() {
                        drag(
                            ui,
                            egui::DragValue::new(v).speed(0.01).max_decimals(6),
                            busy,
                        );
                    }
                }
                if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del.filter(|_| f.fields.len() > 1) {
        f.fields.remove(i);
    }
    if ui.small_button(tr("+ Add field")).clicked() {
        let next = f
            .fields
            .last()
            .map_or(FieldPt::y(0.0), |p| FieldPt::xyw(p.x, p.y + 1.0, p.w));
        f.fields.push(next);
    }
}

/// 波長の表と既定の組
fn waves_ui(ui: &mut Ui, f: &mut SysForm, id: egui::Id, busy: &mut bool) {
    let weak = theme::weak_text(ui.visuals());
    ui.label(RichText::new(tr("Wavelengths")).strong());
    let mut del = None;
    egui::Grid::new(id.with("waves"))
        .num_columns(5)
        .striped(true)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            for h in ["#", tr("Wavelengths [µm]"), tr("Weight"), tr("Primary"), ""] {
                ui.label(RichText::new(h).color(weak));
            }
            ui.end_row();
            for (i, w) in f.waves.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                drag(
                    ui,
                    egui::DragValue::new(&mut w.um)
                        .speed(0.001)
                        .range(0.1..=30.0)
                        .max_decimals(7),
                    busy,
                );
                drag(
                    ui,
                    egui::DragValue::new(&mut w.w)
                        .speed(0.05)
                        .range(0.0..=1e6)
                        .max_decimals(4),
                    busy,
                );
                ui.radio_value(&mut f.primary, i + 1, "");
                if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del.filter(|_| f.waves.len() > 1) {
        f.waves.remove(i);
        if f.primary > i + 1 || f.primary > f.waves.len() {
            f.primary = f.primary.saturating_sub(1).max(1);
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.small_button(tr("+ Add wavelength")).clicked() {
            let next = f.waves.last().map_or(0.5875618, |w| w.um + 0.05);
            f.waves.push(WavePt::um(next));
        }
        ui.label(RichText::new(tr("Presets:")).small().color(weak));
        for (name, set) in PRESETS {
            if ui.small_button(*name).clicked() {
                f.waves = set.iter().map(|&u| WavePt::um(u)).collect();
                f.primary = if set.len() == 3 { 2 } else { 1 };
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Map<String, Value> {
        json!({
            "name": "t",
            "aperture": ["fnum", 4.0],
            "telecentric": false,
            "floating_clip": true,
            "aiming": "real",
            "afocal": true,
            "afocal_unit": "degrees",
            "field_type": "ObjH",
            "field_norm": "rect",
            "fields": [{"x": 0.0, "y": 1.0, "w": 2.0, "vdx": 0.1, "vdy": 0.0, "vcx": 0.0, "vcy": 0.0, "van": 0.0, "extra": 7}],
            "wavelengths": [0.5, 0.6],
            "weights": [1.0, 3.0],
            "primary": 1,
            "temperature": 25.0,
            "pressure": 1.0,
            "adjust_index": true,
            "glrs": 0,
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    #[test]
    fn data_round_trips() {
        let d = data();
        let f = SysForm::from_data(&d).unwrap();
        assert_eq!(f.aperture, ApKind::Fnum);
        assert_eq!(f.field_kind, FieldKind::ObjectHeight);
        assert!(f.field_rect);
        assert_eq!(f.primary, 2);
        assert_eq!(f.afocal, Some(AfocalUnit::Degrees));
        assert_eq!(f.fields[0].vig[0], 0.1);
        assert!(f.env.adjust_index);
        // 変えなければ何も送らない
        let mut d2 = d.clone();
        assert!(f.write_data(&mut d2).is_empty(), "{d2:?}");
        // 変えた項目だけ。視野のほかの鍵は残す
        let mut g = f.clone();
        g.fields[0].y = 2.0;
        g.env.temperature_c = 30.0;
        assert_eq!(g.write_data(&mut d2), vec!["temperature", "fields"]);
        assert_eq!(d2["fields"][0]["extra"], json!(7));
        assert_eq!(d2["fields"][0]["y"], json!(2.0));
    }

    /// 実際の系の system_data は、欄で何も変えなければ何も送らない
    #[test]
    fn real_systems_send_nothing() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let opt = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let mut n = 0;
        for e in std::fs::read_dir(root.join("tests/refs")).unwrap() {
            let zmx = e.unwrap().path().join("system.zmx");
            let Ok(a) = lensbench_core::zmx::load_with(&zmx, &opt) else {
                continue;
            };
            let v = lensbench_project::edit_ops::system_data(&a).unwrap();
            let d = v.as_object().unwrap();
            let f = SysForm::from_data(d).unwrap();
            let mut d2 = d.clone();
            assert!(f.write_data(&mut d2).is_empty(), "{}", zmx.display());
            n += 1;
        }
        assert!(n > 10, "{n}");
    }

    #[test]
    fn condition_field_is_kept() {
        let mut d = data();
        d.insert("field_type".into(), json!({"cond": "position"}));
        let f = SysForm::from_data(&d).unwrap();
        assert!(f.field_cond.is_some());
        let mut d2 = d.clone();
        assert!(f.write_data(&mut d2).is_empty());
    }

    #[test]
    fn combo_round_trips() {
        let c = Combo {
            afocal: Some("dg".into()),
            aperture: ApKind::Float,
            epd: 3.0,
            ..Default::default()
        };
        let f = SysForm::of_combo(&c);
        let mut t = c.clone();
        f.apply_to(&mut t);
        assert_eq!(t, c);
        // 種類を変えて戻しても入射瞳径は残る（EPD 10 → F 値 → EPD）
        let mut c = Combo {
            epd: 10.0,
            ..Default::default()
        };
        let mut f = SysForm::of_combo(&c);
        f.aperture = ApKind::Fnum;
        f.ap_value = 5.0;
        f.apply_to(&mut c);
        let mut f = SysForm::of_combo(&c);
        f.aperture = ApKind::Epd;
        f.apply_to(&mut c);
        assert_eq!((c.epd, c.ap_value), (10.0, 5.0));
    }
}
