//! 下の行（状態・系の数値）と状態の履歴（app.rs から分けた）

use super::*;

impl App {
    /// 下の状態の行: 状態、面・視野・波長の数、焦点距離・F 値・NA・全長・像高、版
    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        ui.horizontal(|ui| {
            // 行の全体の幅（右の数値の置き方を前のフレームの左の幅に頼らずに決める）
            let row_w = ui.available_width();
            let busy = self.c.busy();
            let (dot, col) = if busy {
                ("●", theme::warn(ui.visuals()))
            } else if self.c.status.error {
                ("●", theme::error(ui.visuals()))
            } else {
                ("●", theme::ok(ui.visuals()))
            };
            ui.label(RichText::new(dot).color(col));
            if busy {
                ui.spinner();
                ui.label(RichText::new(trf("Computing ({})", &[&self.c.pending()])).color(weak));
            }
            let text = if self.c.status.text.is_empty() {
                tr("Ready").to_string()
            } else {
                self.c.status.text.clone()
            };
            // 今の誤りへの案内と符号（文が変わっていれば出さない）
            let hint = self
                .c
                .status_hint
                .as_ref()
                .filter(|(t, _)| self.c.status.error && t == &self.c.status.text)
                .map(|(_, h)| h.clone());
            // 履歴（直近 20 件。クリックで全文と履歴を開く）
            if !self.c.status.text.is_empty()
                && self.c.status_log.last().map(|x| &x.0) != Some(&self.c.status.text)
            {
                self.c.status_log.push((
                    self.c.status.text.clone(),
                    self.c.status.error,
                    hint.clone(),
                ));
                if self.c.status_log.len() > 20 {
                    self.c.status_log.remove(0);
                }
            }
            // 左の文は右の数値に重ねない: 前のフレームの右の幅を引いた所で切る
            let right_w: f32 = ui
                .data(|d| d.get_temp(egui::Id::new("status-right-w")))
                .unwrap_or(320.0);
            // 右の数値との間は 16 px 空ける（前は 180 px 引いていて、右に空きがあっても短く切れた）
            let room = (ui.available_width() - right_w - 16.0).max(80.0);
            let rich = if self.c.status.error {
                RichText::new(text.clone()).color(theme::error(ui.visuals()))
            } else {
                RichText::new(text.clone()).color(weak)
            };
            let r = ui
                .scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(
                        egui::Label::new(rich)
                            .truncate()
                            .sense(egui::Sense::click()),
                    )
                })
                .inner;
            // 誤りの全文と案内を浮かべる（下の行では切れるので）
            let r = if self.c.status.error {
                let mut t = text.clone();
                if let Some(h) = &hint {
                    t = format!("{t}\n\n{h}");
                }
                r.on_hover_text(format!(
                    "{t}\n\n{}",
                    tr("Click for the full text and history")
                ))
            } else {
                r.on_hover_text(tr("Click for the full text and history"))
            };
            if r.clicked() {
                self.c.status_log_open = !self.c.status_log_open;
            }
            let right = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(version()).small().color(weak))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(tr("Click to copy the version"))
                    .clicked()
                {
                    ui.ctx().copy_text(version());
                }
                if self.c.font.is_none() && i18n::is_ja() {
                    ui.label(
                        RichText::new(tr("No Japanese font found"))
                            .small()
                            .color(weak),
                    );
                }
                // 系の数値と面・視野・波長の数。左の状態の文に 260 px を残し、入らない分は大事でない方から出さない
                // （系のパネルと同じ名前にして長くなったので、狭い窓で重ならないように）
                let mut parts: Vec<(String, String)> = Vec::new();
                if self.c.sys.info.is_some() {
                    let n = |k: &str, d: f64| {
                        self.c
                            .info_num(k)
                            .map(|x| fmt_num((x * d).round() / d))
                            .unwrap_or("—".into())
                    };
                    // 大事な順
                    for (k, v, u) in [
                        (tr("Focal length"), n("effl", 1e3), " mm"),
                        (tr("Working F/#"), n("wfno", 1e2), ""),
                        (tr("Image space NA"), n("image_na", 1e4), ""),
                        (tr("Total track"), n("total_track", 1e2), " mm"),
                        (
                            tr("Paraxial image height"),
                            n("paraxial_image_height", 1e3),
                            " mm",
                        ),
                    ] {
                        parts.push((k.to_string(), format!("{v}{u}")));
                    }
                }
                if let Some(s) = &self.c.sys.summary {
                    parts.push((
                        trf(
                            "{} surfaces · {} fields · {} wavelengths",
                            &[&s["surfaces"], &self.c.n_fields(), &self.c.waves_um().len()],
                        ),
                        String::new(),
                    ));
                }
                let width = |ui: &Ui, t: &str, mono: bool| {
                    let font = if mono {
                        egui::TextStyle::Monospace.resolve(ui.style())
                    } else {
                        egui::TextStyle::Body.resolve(ui.style())
                    };
                    ui.fonts(|f| f.layout_no_wrap(t.to_string(), font, weak).size().x)
                };
                // 行の幅から、右に置いた版の欄（min_rect）と左の状態の文に残す分を引く
                let mut budget = row_w - ui.min_rect().width() - 300.0;
                let mut shown = 0;
                for (k, v) in &parts {
                    budget -= width(ui, k, false)
                        + width(ui, v, true)
                        + 3.0 * ui.spacing().item_spacing.x
                        + 6.0;
                    if budget < 0.0 {
                        break;
                    }
                    shown += 1;
                }
                // 右から左へ置くので、出すものを逆の順に（左から 焦点距離 … 面・視野・波長 の数）
                for (k, v) in parts[..shown].iter().rev() {
                    if v.is_empty() {
                        ui.label(RichText::new(k).color(weak));
                        ui.separator();
                        continue;
                    }
                    ui.label(RichText::new(v).monospace());
                    ui.label(RichText::new(k).color(weak));
                    ui.add_space(6.0);
                }
                ui.min_rect().width()
            });
            let w = right.inner;
            ui.data_mut(|d| d.insert_temp(egui::Id::new("status-right-w"), w));
        });
        if self.c.status_log_open {
            let mut open = true;
            egui::Window::new(tr("Status history"))
                .open(&mut open)
                .default_width(560.0)
                .max_height(ui.ctx().screen_rect().height() * 0.6)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (t, err, hint) in self.c.status_log.iter().rev() {
                            let rt = RichText::new(t);
                            let rt = if *err {
                                rt.color(theme::error(ui.visuals()))
                            } else {
                                rt
                            };
                            ui.add(egui::Label::new(rt).wrap());
                            if let Some(h) = hint {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(h)
                                            .small()
                                            .color(theme::weak_text(ui.visuals())),
                                    )
                                    .wrap(),
                                );
                            }
                            ui.separator();
                        }
                    });
                    if ui.button(tr("Copy full text")).clicked() {
                        ui.ctx().copy_text(self.c.status.text.clone());
                    }
                });
            self.c.status_log_open = open;
        }
    }
}

/// 計算の側の誤りの符号ごとの案内。符号の無い誤り・案内の要らない符号は None
pub(super) fn error_hint(e: &crate::worker::CallErr) -> Option<String> {
    use lensbench_core::error::code;
    let h = match e.code.as_deref()? {
        code::FILE_NOT_FOUND | code::IS_FOLDER | "io" => {
            "Check that the file exists and its name and folder are right"
        }
        // plain_error が文に足していれば出さない
        code::GLASS_NOT_FOUND => "(add glass catalogs in File > Catalog folders…)",
        code::LENS_NOT_FOUND => {
            "Check the catalog and part number (add lens catalogs in File > Catalog folders…)"
        }
        code::STATE_NOT_FOUND | code::BLOCK_NOT_FOUND => {
            "The project may have been changed elsewhere; reopen it"
        }
        code::PUPIL_VIGNETTED => {
            "Rays in the pupil don't reach the image: check the aperture, fields and surface semi-diameters"
        }
        "not_implemented" => {
            "Not supported yet: the manual lists the supported surfaces and items (Reading .zmx files)"
        }
        _ => return None,
    };
    Some(tr(h).to_string())
}

/// serve の誤りの文から、GUI の利用者に関係の無い内部の言葉を外す（「操作 1: 」、serve の操作名の案内）
pub(super) fn plain_error(e: &str) -> String {
    let mut t = e.to_string();
    // 編集の操作の番号（1 つだけ送ったときは要らない）
    for p in ["操作 1: ", "operation 1: "] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest.to_string();
        }
    }
    t = t.replace("不正: ", "").replace("invalid: ", "");
    let add = tr("(add glass catalogs in File > Catalog folders…)");
    t.replace("（カタログを追加: add_glasses・add_glass_catalogs）", add)
        .replace(
            " (add catalogs: add_glasses, add_glass_catalogs)",
            &format!(" {add}"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::CallErr;
    use lensbench_core::error::code;

    /// 瞳がけられて MTF を計算できない誤りの案内は「未対応」でなく開口・視野を確かめる案内
    #[test]
    fn gux33_pupil_vignetted_hint() {
        let h = error_hint(&CallErr::coded("MTF: …", code::PUPIL_VIGNETTED)).unwrap();
        assert!(h.contains(tr(
            "Rays in the pupil don't reach the image: check the aperture, fields and surface semi-diameters"
        )));
        assert_ne!(Some(h), error_hint(&CallErr::coded("x", "not_implemented")));
    }
}
