//! 解析パネルの結果（見せ方の選択・時間・書き出しと、図・表・JSON）。計算中は図を薄くしてスピナーを重ねる（GUX-27）

use super::*;

/// 頼んでからこれだけたっても返らなければ「計算中」を重ねる（速い解析でちらつかせない）
const BUSY_AFTER: f64 = 0.25;

/// 解析パネルの結果
#[derive(Default)]
pub(super) struct AnaRes {
    pub(super) cmd: String,
    /// 結果を出したときの引数（設定を変えただけなら前の結果は出さない）
    pub(super) params: Value,
    pub(super) value: Option<Value>,
    /// 値を変える前の結果（同じ解析のとき）
    pub(super) prev: Option<Value>,
    pub(super) ms: f64,
    pub(super) rays: Vec<SpotRays>,
    /// 光線の世代（届いたときの rev。スポット図の中心・縮尺を作り直す目安、GPN-07）
    pub(super) rays_gen: u64,
    pub(super) error: Option<String>,
    /// 結果の世代（届くたびに変わる。図の格子を作り直す目安、GPN-07）
    pub(super) gen: u64,
    /// 返事を待っている要求の数と、待ち始めた時刻（GUX-27）
    pub(super) inflight: u32,
    pub(super) since: Option<web_time::Instant>,
}

impl Core {
    /// 解析を頼んだ（返事が来るまで「計算中」）
    pub(super) fn ana_sent(&mut self, id: u64) {
        let r = self.results.entry(id).or_default();
        if r.inflight == 0 {
            r.since = Some(web_time::Instant::now());
        }
        r.inflight += 1;
    }

    /// 解析の返事が来た（捨てられた分も）
    pub(super) fn ana_back(&mut self, id: u64) {
        if let Some(r) = self.results.get_mut(&id) {
            r.inflight = r.inflight.saturating_sub(1);
            if r.inflight == 0 {
                r.since = None;
            }
        }
    }

    /// 解析の結果。前の結果を見せている間に計算し直しているなら、薄くしてスピナーを重ねる
    pub(super) fn result_ui(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        let busy = self.results.get(&id).is_some_and(|r| {
            r.value.is_some()
                && r.since
                    .is_some_and(|t| t.elapsed().as_secs_f64() > BUSY_AFTER)
        });
        let top = ui.cursor().min;
        self.result_body(ui, id, cfg);
        if self.results.get(&id).is_some_and(|r| r.inflight > 0) {
            // 時間がたったら重ねるので、返事を待つ間も描き直す
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if busy {
            let rect = egui::Rect::from_min_max(top, ui.max_rect().max).intersect(ui.clip_rect());
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().panel_fill.gamma_multiply(0.6));
            let c = rect.center();
            ui.put(
                egui::Rect::from_center_size(c, egui::vec2(28.0, 28.0)),
                egui::Spinner::new().size(28.0),
            )
            .on_hover_text(tr("計算しています…"));
        }
    }

    /// 解析の結果（見せ方の選択・時間・書き出しと、図・表・JSON）
    fn result_body(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        let weak = theme::weak_text(ui.visuals());
        let view_row = |ui: &mut Ui, cfg: &mut AnaCfg| {
            ui.selectable_value(&mut cfg.view, View::Figure, tr("図"))
                .on_hover_text(tr("結果を図で見る（図の無い解析は表）"));
            ui.selectable_value(&mut cfg.view, View::Table, tr("表"))
                .on_hover_text(tr("結果を表と木で見る"));
            ui.selectable_value(&mut cfg.view, View::Json, "JSON")
                .on_hover_text(tr("結果の JSON をそのまま見る（CLI の --json と同じ形）"));
        };
        let Some(r) = self.results.get(&id) else {
            ui.horizontal(|ui| view_row(ui, cfg));
            if !self.worker.busy_short() {
                self.run(id, cfg);
            }
            return;
        };
        if let Some(e) = &r.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let Some(v) = &r.value else {
            ui.horizontal(|ui| view_row(ui, cfg));
            ui.spinner();
            return;
        };
        ui.horizontal(|ui| {
            view_row(ui, cfg);
            ui.separator();
            ui.label(
                RichText::new(format!("{} ms", fmt_num((r.ms * 10.0).round() / 10.0)))
                    .small()
                    .color(weak),
            );
            if r.cmd != cfg.cmd {
                ui.label(RichText::new(tr("（前の解析の結果）")).small().color(weak));
            }
            let fig_now = cfg.view == View::Figure && figures::has_figure(&r.cmd);
            match result_export_menu(ui, &r.cmd, v, fig_now) {
                Some(ResOut::Shot(copy)) => {
                    self.shot = Some((id, copy, r.cmd.clone()));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                Some(ResOut::Error(e)) => {
                    self.status = Status {
                        text: e,
                        error: true,
                    }
                }
                None => {}
            }
        });
        // 図の矩形（画像にするときに切り出す）
        self.fig_rects.insert(id, ui.available_rect_before_wrap());
        let fig = cfg.view == View::Figure && figures::has_figure(&r.cmd);
        // MTF の凡例に出す視野の座標
        let coords: Vec<String> = self
            .sys
            .summary
            .as_ref()
            .and_then(|s| s["fields"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|f| {
                format!(
                    "({}, {})",
                    fmt_num(f[0].as_f64().unwrap_or(0.0)),
                    fmt_num(f[1].as_f64().unwrap_or(0.0))
                )
            })
            .collect();
        ui.data_mut(|d| d.insert_temp(egui::Id::new(plots::FIELD_LABELS), coords));
        // 図はパネルの残りに合わせるが最低の高さがあるので、パネルが低いときは縦にスクロールして見る
        if fig && (r.cmd == "mtf" || r.cmd == "huygens-mtf") {
            egui::ScrollArea::vertical()
                .id_salt(("fig", id))
                .auto_shrink([false, false])
                .show(ui, |ui| plots::mtf(ui, v, r.prev.as_ref(), id, r.gen));
            return;
        }
        // スポット以外の図（描けない結果なら表）
        if fig && r.cmd != "spot" {
            let shown = egui::ScrollArea::vertical()
                .id_salt(("fig", id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    figures::show(ui, id, r.gen, &r.cmd, v, r.prev.as_ref())
                })
                .inner;
            if shown {
                return;
            }
        }
        egui::ScrollArea::both()
            .id_salt(("res", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if fig && r.cmd == "spot" {
                    let waves = self.waves_um();
                    let sys = SysInfo {
                        waves_um: &waves,
                        primary: self.primary(),
                        wfno: self.info_num("wfno"),
                    };
                    plots::spot(ui, &r.rays, r.rays_gen, v, r.prev.as_ref(), &sys);
                    ui.separator();
                    json_view::show(ui, v);
                } else if cfg.view == View::Json {
                    // 文字列は結果が変わったときだけ作る。大きい結果は頭だけ見せる（全文はコピー・保存で、GAR-09）
                    let key = egui::Id::new(("res-json", id));
                    let t = match ui.data(|d| d.get_temp::<(u64, std::sync::Arc<String>)>(key)) {
                        Some((r, t)) if r == self.rev => t,
                        _ => {
                            let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
                            if s.len() > JSON_VIEW_MAX {
                                let mut cut = JSON_VIEW_MAX;
                                while !s.is_char_boundary(cut) {
                                    cut -= 1;
                                }
                                s.truncate(cut);
                                s.push('\n');
                                s.push_str(&trf(
                                    "…（長いので {} 字で切った。全文は「コピー」「保存」で）",
                                    &[&JSON_VIEW_MAX],
                                ));
                            }
                            let t = std::sync::Arc::new(s);
                            ui.data_mut(|d| d.insert_temp(key, (self.rev, t.clone())));
                            t
                        }
                    };
                    ui.add(
                        egui::TextEdit::multiline(&mut t.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                } else {
                    json_view::show(ui, v);
                }
            });
    }
}
