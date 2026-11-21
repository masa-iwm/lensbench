//! 系の設定の共通の欄。
//!
//! 組合せの設定（[`Combo`]）を [`SysForm`] に移して欄で描き、描いた後に元へ書き戻す。組合せにしかない項目は
//! [`Extra`] で差し込む。
//! 共通の項目: 開口の種類と値・エイミング・アフォーカル・テレセントリック・浮動開口・視野（種類・正規化・点・重み・
//! ケラレ係数）・波長（重み・主波長・既定の組）・環境（温度・気圧・屈折率の補正）。

use egui::{RichText, Ui};
use lensbench_core::system::{AfocalUnit, VigInterp};
use lensbench_project::Environment;

use super::{
    Aiming, ApertureKind, FieldKind, FieldPt, FieldSolve, SolveCond, WavePt, AFOCAL_UNITS, PRESETS,
};
use super::{ApertureField, Choice};
use crate::combo::Combo;
use crate::i18n::tr;
use crate::theme;

#[path = "form_sheets.rs"]
mod sheets;

/// 系の設定の共通の値
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SysForm {
    pub aperture: ApertureKind,
    /// 開口の値（F 値・NA など。Float の値は使わない）
    pub ap_value: f64,
    /// 入射瞳径 [mm]（種類を変えても残す）
    pub epd: f64,
    pub field_kind: FieldKind,
    /// 条件で定める視野の条件
    pub field_solve: FieldSolve,
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
    /// 定義した視野以外の点のケラレ係数を OpticStudio と同じ求め方にする（既定は連続）
    pub vig_os: bool,
}

/// 片方にしかない項目を共通の欄の間に差し込む
pub(crate) trait Extra {
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

impl SysForm {
    /// 組合せの値から
    pub(crate) fn of_combo(c: &Combo) -> SysForm {
        SysForm {
            aperture: c.aperture,
            ap_value: c.ap_value,
            epd: c.epd,
            field_kind: c.field_kind,
            field_solve: c.field_solve.clone(),
            field_rect: c.extra.field_rect,
            fields: c.fields.clone(),
            waves: c.waves.clone(),
            primary: c.primary,
            telecentric: c.telecentric,
            aiming: c.aiming,
            afocal: c.afocal.as_deref().and_then(AfocalUnit::from_code),
            floating_clip: c.floating_clip,
            env: c.extra.environment.unwrap_or_default(),
            vig_os: c.extra.vig_interp == Some(VigInterp::OpticStudio),
        }
    }

    /// 組合せへ書き戻す
    pub(crate) fn apply_to(&self, c: &mut Combo) {
        c.aperture = self.aperture;
        c.ap_value = self.ap_value;
        c.epd = self.epd;
        c.field_kind = self.field_kind;
        c.field_solve = self.field_solve.clone();
        c.extra.field_rect = self.field_rect;
        c.fields = self.fields.clone();
        c.waves = self.waves.clone();
        c.primary = self.primary;
        c.telecentric = self.telecentric;
        c.aiming = self.aiming;
        c.afocal = self.afocal.map(|u| u.code().to_ascii_lowercase());
        c.floating_clip = self.floating_clip;
        c.extra.environment = (self.env != Environment::default()).then_some(self.env);
        c.extra.vig_interp = self.vig_os.then_some(VigInterp::OpticStudio);
    }
}

/// 数の欄。ドラッグ中・入力中は busy に
fn drag(ui: &mut Ui, dv: crate::widgets::Num, busy: &mut bool) {
    let r = ui.add(dv);
    *busy |= r.dragged() || r.has_focus();
}

/// 共通の欄を描く。ドラッグ中・入力中なら true
pub(crate) fn ui(ui: &mut Ui, f: &mut SysForm, ex: &mut dyn Extra) -> bool {
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
                    for &k in ApertureKind::ALL {
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
                    crate::widgets::Num::new(&mut f.ap_value)
                        .range(1e-6..=1e6)
                        .max_decimals(6)
                        .suffix(f.aperture.unit()),
                    &mut busy,
                );
                ui.end_row();
            } else if f.aperture == ApertureKind::Epd {
                ui.label(tr("Entrance pupil diameter [mm]"));
                drag(
                    ui,
                    crate::widgets::Num::new(&mut f.epd)
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
                for &a in Aiming::ALL {
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
            crate::widgets::Num::new(&mut f.env.temperature_c)
                .range(-273.15..=1e4)
                .suffix(" °C"),
            &mut busy,
        );
        ui.label(RichText::new(tr("Pressure")).color(weak));
        drag(
            ui,
            crate::widgets::Num::new(&mut f.env.pressure_atm)
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
        egui::ComboBox::from_id_salt(id.with("fk"))
            .selected_text(f.field_kind.label())
            .show_ui(ui, |ui| {
                for &k in FieldKind::ALL {
                    ui.selectable_value(&mut f.field_kind, k, k.label());
                }
            });
        ui.label(RichText::new(tr("Normalization")).color(weak));
        ui.radio_value(&mut f.field_rect, false, tr("Radial"));
        ui.radio_value(&mut f.field_rect, true, tr("Rectangular"));
    });
    if f.field_kind == FieldKind::Solve {
        let s = &mut f.field_solve;
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
    if show_vig || f.vig_os {
        ui.checkbox(&mut f.vig_os, tr("OpticStudio-compatible interpolation"))
            .on_hover_text(tr(
                "Vignetting factors at points between the defined fields: as OpticStudio (steps between fields) instead of continuous. Use it to compare with OpticStudio",
            ));
    }
    let unit = super::field_unit(f.field_kind);
    let mut sh = sheets::FieldSheet {
        fields: &mut f.fields,
        vig: show_vig,
        unit,
    };
    let (_, b) = crate::widgets::sheet::show_temp(ui, id.with("fields"), &mut sh);
    *busy |= b;
}

/// 波長の表と既定の組
fn waves_ui(ui: &mut Ui, f: &mut SysForm, id: egui::Id, busy: &mut bool) {
    let weak = theme::weak_text(ui.visuals());
    ui.label(RichText::new(tr("Wavelengths")).strong());
    let (_, b) =
        crate::widgets::sheet::show_temp(ui, id.with("waves"), &mut sheets::WaveSheet { f });
    *busy |= b;
    ui.horizontal_wrapped(|ui| {
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

    #[test]
    fn combo_round_trips() {
        let c = Combo {
            afocal: Some("dg".into()),
            aperture: ApertureKind::Float,
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
        f.aperture = ApertureKind::FNum;
        f.ap_value = 5.0;
        f.apply_to(&mut c);
        let mut f = SysForm::of_combo(&c);
        f.aperture = ApertureKind::Epd;
        f.apply_to(&mut c);
        assert_eq!((c.epd, c.ap_value), (10.0, 5.0));
        // ケラレ係数の求め方はプロジェクトの vig_interp（既定の連続は書かない）
        let mut f = SysForm::of_combo(&c);
        f.vig_os = true;
        f.apply_to(&mut c);
        let mut v = serde_json::json!({});
        c.extra.put(&mut v);
        assert_eq!(v["vig_interp"], "opticstudio");
        let e = crate::combo::SysExtra::read(&v).unwrap();
        assert_eq!(e.vig_interp, Some(VigInterp::OpticStudio));
        let e =
            crate::combo::SysExtra::read(&serde_json::json!({"vig_interp": "continuous"})).unwrap();
        assert!(e.is_empty());
    }
}
