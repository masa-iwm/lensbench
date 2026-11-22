//! .zmx への書き出しと、熱の構成を作って .zmx に書き出す窓（OpticStudio の Make Thermal、[`lensbench_core::thermal`]）。
//! 読み込んだ系は変えず、今の構成を構成 1 にし、温度ごとに構成を足した系を書き出す

use super::*;
use lensbench_engine::ThermalReq;

/// 熱の構成の窓の欄
#[derive(Default)]
pub(super) struct ThermalDlg {
    /// 足す構成の温度 [°C]（コンマ区切り）
    temps: String,
    /// 足す構成の気圧 [atm]（コンマ区切り、空なら今の気圧）
    pressures: String,
    /// 欄を読めなかった理由
    error: Option<String>,
}

/// コンマ区切りの数（空は空の並び）
fn numbers(s: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| {
            x.parse::<f64>()
                .map_err(|_| trf("{} is not a number", &[&x]))
        })
        .collect()
}

impl ThermalDlg {
    /// 欄から書き出しの熱の構成を作る
    fn req(&self) -> Result<ThermalReq, String> {
        let temps = numbers(&self.temps)?;
        if temps.is_empty() {
            return Err(tr("Give one or more temperatures").to_string());
        }
        Ok(ThermalReq {
            temps,
            pressures: numbers(&self.pressures)?,
        })
    }
}

impl Core {
    /// 今の系を .zmx に書き出す
    pub(super) fn write_zmx(&mut self) {
        let d = crate::files::FileDialog::new().add_filter("Zemax", &["zmx"]);
        self.save_as(d, |c, p| {
            c.call(
                Tag::Write,
                Call::Write {
                    path: p,
                    thermal: None,
                },
            )
        });
    }

    /// 熱の構成の窓を開く
    pub(super) fn open_thermal(&mut self) {
        if self.thermal.is_none() {
            self.thermal = Some(ThermalDlg::default());
        }
    }

    /// 熱の構成の窓
    pub(super) fn thermal_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.thermal else {
            return;
        };
        let mut open = true;
        let mut go = None;
        egui::Window::new(tr("Make thermal configurations"))
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(tr(
                        "Writes a .zmx whose configuration 1 is the current system and the others are the system at the given temperatures (curvatures, thicknesses, aspheric terms and sizes scaled with the thermal expansion, indices from the temperature). The opened system is not changed",
                    ))
                    .small()
                    .weak(),
                );
                egui::Grid::new("thermal-grid").num_columns(2).show(ui, |ui| {
                    ui.label(tr("Temperatures [°C]"));
                    ui.add(egui::TextEdit::singleline(&mut dlg.temps).hint_text("-20, 60"));
                    ui.end_row();
                    ui.label(tr("Pressures [atm]"));
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.pressures)
                            .hint_text(tr("(current pressure)")),
                    );
                    ui.end_row();
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                if ui.button(tr("Export to .zmx…")).clicked() {
                    match dlg.req() {
                        Ok(r) => go = Some(r),
                        Err(e) => dlg.error = Some(e),
                    }
                }
            });
        if !open {
            self.thermal = None;
        }
        if let Some(r) = go {
            if let Some(d) = &mut self.thermal {
                d.error = None;
            }
            let d = crate::files::FileDialog::new().add_filter("Zemax", &["zmx"]);
            self.save_as(d, move |c, p| {
                c.call(
                    Tag::Write,
                    Call::Write {
                        path: p,
                        thermal: Some(r),
                    },
                )
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_fields_are_read() {
        let d = ThermalDlg {
            temps: " -20, 60 ".into(),
            pressures: "".into(),
            error: None,
        };
        let r = d.req().unwrap();
        assert_eq!((r.temps, r.pressures), (vec![-20.0, 60.0], vec![]));
        let d = ThermalDlg {
            temps: "".into(),
            ..Default::default()
        };
        assert!(d.req().is_err());
        let d = ThermalDlg {
            temps: "20, x".into(),
            ..Default::default()
        };
        assert!(d.req().is_err());
    }
}
