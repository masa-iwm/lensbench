//! 下の行（状態・系の数値）と状態の履歴（GAR-18 で app.rs から分けた）

use super::*;

impl App {
    /// 下の状態の行: 状態、面・視野・波長の数、焦点距離・F 値・NA・全長・像高、版
    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        ui.horizontal(|ui| {
            // 行の全体の幅（右の数値の置き方を前のフレームの左の幅に頼らずに決める）
            let row_w = ui.available_width();
            let busy = self.c.worker.busy();
            let (dot, col) = if busy {
                ("●", Color32::from_rgb(0xF2, 0xC9, 0x4C))
            } else if self.c.status.error {
                ("●", Color32::from_rgb(0xFF, 0x7A, 0x6B))
            } else {
                ("●", Color32::from_rgb(0x8F, 0xD1, 0x9E))
            };
            ui.label(RichText::new(dot).color(col));
            if busy {
                ui.spinner();
            }
            let text = if self.c.status.text.is_empty() {
                tr("準備完了").to_string()
            } else {
                self.c.status.text.clone()
            };
            // 履歴（直近 20 件。クリックで全文と履歴を開く）
            if !self.c.status.text.is_empty()
                && self.c.status_log.last().map(|x| &x.0) != Some(&self.c.status.text)
            {
                self.c
                    .status_log
                    .push((self.c.status.text.clone(), self.c.status.error));
                if self.c.status_log.len() > 20 {
                    self.c.status_log.remove(0);
                }
            }
            // 左の文は右の数値に重ねない: 前のフレームの右の幅を引いた所で切る（GUX-06）
            let right_w: f32 = ui
                .data(|d| d.get_temp(egui::Id::new("status-right-w")))
                .unwrap_or(320.0);
            let room = (ui.available_width() - right_w - 180.0).max(80.0);
            let rich = if self.c.status.error {
                RichText::new(text.clone()).color(Color32::from_rgb(0xFF, 0x7A, 0x6B))
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
                .inner
                .on_hover_text(tr("クリックで全文と履歴"));
            if r.clicked() {
                self.c.status_log_open = !self.c.status_log_open;
            }
            let right = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(version()).small().color(weak))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(tr("クリックで版をコピー"))
                    .clicked()
                {
                    ui.ctx().copy_text(version());
                }
                if self.c.font.is_none() && i18n::is_ja() {
                    ui.label(
                        RichText::new(tr("日本語のフォントが見つからない"))
                            .small()
                            .color(weak),
                    );
                }
                // 系の数値と面・視野・波長の数。左の状態の文に 260 px を残し、入らない分は大事でない方から出さない
                // （系のパネルと同じ名前にして長くなったので、狭い窓で重ならないように。GUX-28）
                let mut parts: Vec<(String, String)> = Vec::new();
                if self.c.info.is_some() {
                    let n = |k: &str, d: f64| {
                        self.c
                            .info_num(k)
                            .map(|x| fmt_num((x * d).round() / d))
                            .unwrap_or("—".into())
                    };
                    // 大事な順
                    for (k, v, u) in [
                        (tr("焦点距離"), n("effl", 1e3), " mm"),
                        (tr("作動 F 値"), n("wfno", 1e2), ""),
                        (tr("像空間 NA"), n("image_na", 1e4), ""),
                        (tr("全長"), n("total_track", 1e2), " mm"),
                        (tr("近軸像高"), n("paraxial_image_height", 1e3), " mm"),
                    ] {
                        parts.push((k.to_string(), format!("{v}{u}")));
                    }
                }
                if let Some(s) = &self.c.summary {
                    parts.push((
                        trf(
                            "{} 面 · {} 視野 · {} 波長",
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
            egui::Window::new(tr("状態の履歴"))
                .open(&mut open)
                .default_width(560.0)
                .max_height(ui.ctx().screen_rect().height() * 0.6)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (t, err) in self.c.status_log.iter().rev() {
                            let rt = RichText::new(t);
                            let rt = if *err {
                                rt.color(Color32::from_rgb(0xFF, 0x7A, 0x6B))
                            } else {
                                rt
                            };
                            ui.add(egui::Label::new(rt).wrap());
                            ui.separator();
                        }
                    });
                    if ui.button(tr("全文をコピー")).clicked() {
                        ui.ctx().copy_text(self.c.status.text.clone());
                    }
                });
            self.c.status_log_open = open;
        }
    }
}
