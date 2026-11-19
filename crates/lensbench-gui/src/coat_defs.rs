//! 膜（フィルタ）の定義: プロジェクトの `coatings`（名前 → フィルタの透過特性、[`CoatSpec`]）を欄で書き、
//! 透過の曲線（0°・45°）を図で見せる。面・鏡・BS の膜の欄にその名前を書いて使う。

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot};
use lensbench_project::filter::{CoatSpec, Ideal};

use crate::i18n::{n_, tr, trf};
use crate::theme;

#[path = "coat_curve.rs"]
mod curve;

/// 次の描画で膜の定義の欄を開いて見せる（通し確認）
pub(crate) fn focus(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("lensbench_coat_focus"), true));
}

/// [`focus`] を受け取る（一度だけ true）
pub(crate) fn take_focus(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("lensbench_coat_focus")))
        .unwrap_or(false)
}

/// 種類の名前（serde の type）と表示名
const KINDS: &[(&str, &str)] = &[
    ("longpass", n_("Longpass")),
    ("shortpass", n_("Shortpass")),
    ("bandpass", n_("Bandpass")),
    ("notch", n_("Notch")),
    ("nd", "ND"),
    ("curve", n_("Transmission per wavelength")),
    ("table", n_("Table (per angle and wavelength)")),
];

/// 表示名の表（翻訳の検査用）
#[cfg(test)]
pub(crate) const LABELS: [&str; 7] = [
    n_("Longpass"),
    n_("Shortpass"),
    n_("Bandpass"),
    n_("Notch"),
    "ND",
    n_("Transmission per wavelength"),
    n_("Table (per angle and wavelength)"),
];

fn ideal() -> Ideal {
    Ideal {
        t: 0.95,
        od: 4.0,
        width_nm: None,
        neff: 2.0,
        absorptive: false,
    }
}

fn kind_of(s: &CoatSpec) -> &'static str {
    match s {
        CoatSpec::Longpass { .. } => "longpass",
        CoatSpec::Shortpass { .. } => "shortpass",
        CoatSpec::Bandpass { .. } => "bandpass",
        CoatSpec::Notch { .. } => "notch",
        CoatSpec::Nd { .. } => "nd",
        CoatSpec::Curve { .. } => "curve",
        CoatSpec::Table { .. } => "table",
    }
}

/// 種類を変えた膜（近い値を引き継ぐ）
fn with_kind(s: &CoatSpec, k: &str) -> CoatSpec {
    let (edge, center, fwhm, id) = match s {
        CoatSpec::Longpass { edge_nm, ideal } | CoatSpec::Shortpass { edge_nm, ideal } => {
            (*edge_nm, *edge_nm, 0.1 * edge_nm, ideal.clone())
        }
        CoatSpec::Bandpass {
            center_nm,
            fwhm_nm,
            ideal,
        }
        | CoatSpec::Notch {
            center_nm,
            fwhm_nm,
            ideal,
        } => (*center_nm, *center_nm, *fwhm_nm, ideal.clone()),
        _ => (550.0, 550.0, 40.0, ideal()),
    };
    match k {
        "longpass" => CoatSpec::Longpass {
            edge_nm: edge,
            ideal: id,
        },
        "shortpass" => CoatSpec::Shortpass {
            edge_nm: edge,
            ideal: id,
        },
        "bandpass" => CoatSpec::Bandpass {
            center_nm: center,
            fwhm_nm: fwhm,
            ideal: id,
        },
        "notch" => CoatSpec::Notch {
            center_nm: center,
            fwhm_nm: fwhm,
            ideal: id,
        },
        "nd" => CoatSpec::Nd {
            od: 1.0,
            absorptive: true,
        },
        "curve" => CoatSpec::Curve {
            wavelengths_nm: vec![400.0, 700.0],
            t: vec![0.9, 0.9],
            r: Vec::new(),
        },
        _ => CoatSpec::Table {
            rows: vec![[0.0, 0.55, 0.05, 0.05, 0.95, 0.95]],
        },
    }
}

/// 膜の定義の欄。変えたら true
pub(crate) fn ui(ui: &mut Ui, coatings: &mut BTreeMap<String, CoatSpec>) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let before = coatings.clone();
    ui.label(
        RichText::new(tr(
            "Use it by writing the name in the coating field of a mirror or BS, or the coating of a surface in the surface table. Takes precedence over the same name in coating files and user catalogs",
        ))
        .small()
        .color(weak),
    );
    let id = ui.id().with("coat_defs");
    let mut rename: Option<(String, String)> = None;
    let mut remove = None;
    for (name, spec) in coatings.iter_mut() {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let mut n = name.clone();
            ui.add(egui::TextEdit::singleline(&mut n).desired_width(90.0))
                .on_hover_text(tr("Coating name"));
            if n != *name && !n.trim().is_empty() {
                rename = Some((name.clone(), n.trim().to_string()));
            }
            let k = kind_of(spec);
            let mut k2 = k;
            egui::ComboBox::from_id_salt(id.with(("kind", name.as_str())))
                .selected_text(tr(KINDS.iter().find(|x| x.0 == k).map_or(k, |x| x.1)))
                .show_ui(ui, |ui| {
                    for (key, label) in KINDS {
                        ui.selectable_value(&mut k2, key, tr(label));
                    }
                });
            if k2 != k {
                *spec = with_kind(spec, k2);
            }
            if ui
                .small_button("✖")
                .on_hover_text(tr("Delete this coating"))
                .clicked()
            {
                remove = Some(name.clone());
            }
        });
        fields(ui, id.with(name.as_str()), spec);
        curve_plot(ui, id.with(("plot", name.as_str())), spec);
    }
    if let Some(n) = remove {
        coatings.remove(&n);
    }
    if let Some((a, b)) = rename {
        if !coatings.contains_key(&b) {
            if let Some(s) = coatings.remove(&a) {
                coatings.insert(b, s);
            }
        }
    }
    ui.separator();
    if ui.button(tr("＋ Add coating")).clicked() {
        let mut k = 1;
        while coatings.contains_key(&format!("FILTER{k}")) {
            k += 1;
        }
        coatings.insert(
            format!("FILTER{k}"),
            CoatSpec::Longpass {
                edge_nm: 550.0,
                ideal: ideal(),
            },
        );
    }
    *coatings != before
}

fn num(ui: &mut Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64) {
    ui.label(label);
    ui.add(
        egui::DragValue::new(v)
            .range(range)
            .speed(speed)
            .max_decimals(6),
    );
}

fn ideal_fields(ui: &mut Ui, i: &mut Ideal) {
    num(ui, tr("In-band transmission"), &mut i.t, 0.0..=1.0, 0.005);
    num(ui, tr("Out-of-band OD"), &mut i.od, 0.0..=12.0, 0.05);
    let mut auto = i.width_nm.is_none();
    if ui
        .checkbox(&mut auto, tr("Automatic edge width"))
        .on_hover_text(tr(
            "Width from 10 % to 90 % transmission. Automatic is 1 % of the edge wavelength, or 10 % of the FWHM for band filters",
        ))
        .changed()
    {
        i.width_nm = if auto { None } else { Some(5.0) };
    }
    if let Some(w) = &mut i.width_nm {
        num(ui, tr("Edge width [nm]"), w, 0.01..=1000.0, 0.1);
    }
    num(ui, tr("Effective index"), &mut i.neff, 1.0..=5.0, 0.01);
    ui.checkbox(&mut i.absorptive, tr("Absorptive (no reflection)"));
}

fn fields(ui: &mut Ui, id: egui::Id, spec: &mut CoatSpec) {
    match spec {
        CoatSpec::Longpass { edge_nm, ideal } | CoatSpec::Shortpass { edge_nm, ideal } => {
            ui.horizontal_wrapped(|ui| {
                num(ui, tr("Edge [nm]"), edge_nm, 1.0..=5000.0, 1.0);
                ideal_fields(ui, ideal);
            });
        }
        CoatSpec::Bandpass {
            center_nm,
            fwhm_nm,
            ideal,
        }
        | CoatSpec::Notch {
            center_nm,
            fwhm_nm,
            ideal,
        } => {
            ui.horizontal_wrapped(|ui| {
                num(ui, tr("Center [nm]"), center_nm, 1.0..=5000.0, 1.0);
                num(ui, tr("FWHM [nm]"), fwhm_nm, 0.01..=5000.0, 0.5);
                ideal_fields(ui, ideal);
            });
        }
        CoatSpec::Nd { od, absorptive } => {
            ui.horizontal_wrapped(|ui| {
                num(ui, "OD", od, 0.0..=12.0, 0.05);
                ui.checkbox(absorptive, tr("Absorptive (no reflection)"));
            });
        }
        CoatSpec::Curve {
            wavelengths_nm,
            t,
            r,
        } => {
            // 行: 波長 [nm]・透過・反射（反射を使わなければ空）
            let use_r = !r.is_empty();
            let mut sh = curve::CurveSheet {
                l: &mut *wavelengths_nm,
                t: &mut *t,
                r: &mut *r,
            };
            crate::widgets::sheet::show_temp(ui, id.with("curve"), &mut sh);
            ui.horizontal(|ui| {
                let mut ur = use_r;
                if ui
                    .checkbox(&mut ur, tr("Also give reflection"))
                    .on_hover_text(tr("Without it, reflection is 0 (absorptive)"))
                    .changed()
                {
                    *r = if ur {
                        t.iter().map(|x| (1.0 - x).max(0.0)).collect()
                    } else {
                        Vec::new()
                    };
                }
            });
        }
        CoatSpec::Table { rows } => {
            // 表は JSON の行で（[入射角 deg, 波長 µm, Rs, Rp, Ts, Tp]）
            let key = id.with("table_text");
            let mut txt: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| {
                rows.iter()
                    .map(|r| serde_json::to_string(r).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            ui.label(
                RichText::new(tr(
                    "One row per line: [incidence angle deg, wavelength µm, Rs, Rp, Ts, Tp] (e.g. a vendor table)",
                ))
                .small(),
            );
            let r = ui.add(
                egui::TextEdit::multiline(&mut txt)
                    .code_editor()
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            let parsed: Result<Vec<[f64; 6]>, String> = txt
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    serde_json::from_str::<[f64; 6]>(l.trim().trim_end_matches(','))
                        .map_err(|e| format!("{l}: {e}"))
                })
                .collect();
            match parsed {
                Ok(v) if !v.is_empty() => {
                    if r.changed() {
                        *rows = v;
                    }
                }
                Ok(_) => {
                    ui.colored_label(ui.visuals().warn_fg_color, tr("No rows"));
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, lensbench_core::i18n::tr(&e));
                }
            }
            if r.has_focus() {
                ui.ctx().data_mut(|d| d.insert_temp(key, txt));
            } else {
                ui.ctx().data_mut(|d| d.remove::<String>(key));
            }
        }
    }
}

/// 透過の曲線を描く範囲 [nm]。理想フィルタは 45° でエッジが短波長側へずれた所も入れる
fn view_range(spec: &CoatSpec) -> (f64, f64) {
    // 45° のエッジの位置の比 √(1 − (sin45° / neff)²)
    let shift = |i: &Ideal| (1.0 - 0.5 / (i.neff * i.neff)).max(0.0).sqrt();
    match spec {
        CoatSpec::Longpass { edge_nm, ideal } | CoatSpec::Shortpass { edge_nm, ideal } => {
            (edge_nm * 0.9 * shift(ideal), edge_nm * 1.1)
        }
        CoatSpec::Bandpass {
            center_nm,
            fwhm_nm,
            ideal,
        }
        | CoatSpec::Notch {
            center_nm,
            fwhm_nm,
            ideal,
        } => (
            (center_nm - 3.0 * fwhm_nm) * shift(ideal),
            center_nm + 3.0 * fwhm_nm,
        ),
        CoatSpec::Nd { .. } => (400.0, 700.0),
        CoatSpec::Curve { wavelengths_nm, .. } => (
            wavelengths_nm.first().copied().unwrap_or(400.0),
            wavelengths_nm.last().copied().unwrap_or(700.0),
        ),
        CoatSpec::Table { rows } => {
            let (lo, hi) = rows.iter().fold((f64::MAX, f64::MIN), |(a, b), r| {
                (a.min(r[1] * 1e3), b.max(r[1] * 1e3))
            });
            if lo < hi {
                (lo, hi)
            } else {
                (lo - 50.0, lo + 50.0)
            }
        }
    }
}

/// 入射角 a に近い角の行の、波長 [nm] と透過（Ts・Tp の平均）
fn curve_at(spec: &CoatSpec, a: f64) -> Result<Vec<[f64; 2]>, String> {
    let t = spec.table()?;
    let Some((_, rows)) = t
        .iter()
        .min_by(|x, y| (x.0 - a).abs().total_cmp(&(y.0 - a).abs()))
    else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .map(|r| [r[0] * 1e3, 0.5 * (r[3] + r[4])])
        .collect())
}

fn curve_plot(ui: &mut Ui, id: egui::Id, spec: &CoatSpec) {
    let p0 = match curve_at(spec, 0.0) {
        Ok(p) => p,
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, lensbench_core::i18n::tr(&e));
            return;
        }
    };
    // 角度で変わる膜（理想フィルタ・表）だけ 45° も
    let tilted = matches!(
        spec,
        CoatSpec::Longpass { .. }
            | CoatSpec::Shortpass { .. }
            | CoatSpec::Bandpass { .. }
            | CoatSpec::Notch { .. }
            | CoatSpec::Table { .. }
    );
    let p45 = if tilted {
        curve_at(spec, 45.0).unwrap_or_default()
    } else {
        Vec::new()
    };
    let (lo, hi) = view_range(spec);
    let clip = |p: Vec<[f64; 2]>| -> Vec<[f64; 2]> {
        let m = (hi - lo) * 0.05;
        p.into_iter()
            .filter(|q| q[0] >= lo - m && q[0] <= hi + m)
            .collect()
    };
    Plot::new(id)
        .height(140.0)
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(1.0)
        .include_x(lo)
        .include_x(hi)
        .x_axis_label(tr("Wavelength [nm]"))
        .y_axis_label(tr("Transmission"))
        .allow_scroll(false)
        .show(ui, |pl| {
            pl.line(
                Line::new(clip(p0))
                    .color(theme::series(0))
                    .name(trf("{}°", &[&0])),
            );
            if !p45.is_empty() {
                pl.line(
                    Line::new(clip(p45))
                        .color(theme::series(1))
                        .style(LineStyle::Dashed { length: 6.0 })
                        .name(trf("{}°", &[&45])),
                );
            }
        });
}

/// 膜の名前の欄（鏡・BS の分ける面）
pub(crate) fn coat_edit(
    ui: &mut Ui,
    coat: &mut String,
    k: usize,
    weak: egui::Color32,
    coats: &[String],
) {
    ui.label(RichText::new(tr("Coating")).color(weak));
    ui.add(
        egui::TextEdit::singleline(coat)
            .hint_text(tr("no coating"))
            .desired_width(70.0),
    )
    .on_hover_text(tr(
        "Name of a coating in a coating file (COATING.DAT, etc.), a coating model, or one defined in system settings. Empty keeps an ideal mirror or the split ratio",
    ));
    // 系の設定で定義した膜から選ぶ
    if !coats.is_empty() {
        egui::ComboBox::from_id_salt(("coat_pick", k))
            .selected_text("▼")
            .width(24.0)
            .show_ui(ui, |ui| {
                for n in coats {
                    if ui.selectable_label(coat == n, n.as_str()).clicked() {
                        *coat = n.clone();
                    }
                }
            })
            .response
            .on_hover_text(tr("Choose a defined coating"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_curves() {
        let s = CoatSpec::Bandpass {
            center_nm: 550.0,
            fwhm_nm: 20.0,
            ideal: ideal(),
        };
        for (k, _) in KINDS {
            let t = with_kind(&s, k);
            assert_eq!(kind_of(&t), *k);
            assert!(t.table().is_ok(), "{k}");
        }
        // 帯域の中心で透過が高く、外で低い。45° では短波長側へずれる
        let c0 = curve_at(&s, 0.0).unwrap();
        let at = |c: &[[f64; 2]], l: f64| {
            c.iter()
                .min_by(|a, b| (a[0] - l).abs().total_cmp(&(b[0] - l).abs()))
                .unwrap()[1]
        };
        assert!(at(&c0, 550.0) > 0.9);
        assert!(at(&c0, 600.0) < 1e-3);
        let c45 = curve_at(&s, 45.0).unwrap();
        assert!(at(&c45, 550.0) < at(&c0, 550.0));
        let (lo, hi) = view_range(&s);
        assert!(
            (lo - 490.0 * (0.875f64).sqrt()).abs() < 1e-9 && hi == 610.0,
            "{lo} {hi}"
        );
    }

    #[test]
    fn project_coatings_json() {
        let mut m = BTreeMap::new();
        m.insert(
            "LP".to_string(),
            CoatSpec::Longpass {
                edge_nm: 600.0,
                ideal: ideal(),
            },
        );
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["LP"]["type"], "longpass");
        assert_eq!(v["LP"]["edge_nm"], 600.0);
    }
}
