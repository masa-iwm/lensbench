//! 系の設定・物体・解析のパネル（app.rs から分けた、GAR-07）

use super::*;

impl Core {
    /// 系の要約（大きな数）とパラメータ
    pub(super) fn system_ui(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        let Some(sum) = self.sys.summary.clone() else {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system file (.zmx or project .json), or drop it onto the window.",
            ));
            ui.label(tr("To combine catalog lenses, go to the Catalog panel."));
            if ui.button(tr("Open… (Ctrl+O)")).clicked() {
                self.pick_and_open();
            }
            if !self.s.recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("Recent files")).small().color(weak));
                for f in self.s.recent.clone().iter().take(6) {
                    let name = Path::new(f)
                        .file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| f.clone());
                    if ui.link(name).on_hover_text(f).clicked() {
                        self.open(Path::new(f));
                    }
                }
            }
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            let name = sum["name"].as_str().unwrap_or_default();
            if !name.is_empty() {
                ui.label(RichText::new(name).strong());
            }
            ui.add_space(4.0);
            let card = |ui: &mut Ui, label: &str, v: Option<f64>, unit: &str, digits: f64| {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(label).small().color(weak));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    v.map(|x| fmt_num((x * digits).round() / digits))
                                        .unwrap_or("—".into()),
                                )
                                .size(19.0)
                                .monospace()
                                .strong(),
                            );
                            if !unit.is_empty() {
                                ui.label(RichText::new(unit).small().color(weak));
                            }
                        });
                    });
            };
            for row in [
                [
                    (tr("Focal length"), self.info_num("effl"), "mm", 1e3),
                    (tr("Working F/#"), self.info_num("wfno"), "", 1e3),
                ],
                [
                    (tr("Image space NA"), self.info_num("image_na"), "", 1e4),
                    (tr("Total track"), self.info_num("total_track"), "mm", 1e3),
                ],
            ] {
                ui.columns(2, |cols| {
                    for (ui, (l, v, u, d)) in cols.iter_mut().zip(row) {
                        card(ui, l, v, u, d);
                    }
                });
            }
            ui.add_space(4.0);
            egui::Grid::new("summary")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    // 値は折り返す（視野が多くてもパネルの右で切れない、GUX-14）
                    let mut row = |k: &str, v: RichText| {
                        ui.label(RichText::new(k).color(weak));
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    };
                    let n = |x: Option<f64>| {
                        x.map(|x| fmt_num((x * 1e4).round() / 1e4))
                            .unwrap_or("—".into())
                    };
                    row(
                        tr("Entrance pupil diameter [mm]"),
                        RichText::new(n(self.info_num("epd"))),
                    );
                    row(
                        tr("Paraxial image height [mm]"),
                        RichText::new(n(self.info_num("paraxial_image_height"))),
                    );
                    row(
                        tr("Number of surfaces"),
                        RichText::new(format!("{}", sum["surfaces"])),
                    );
                    let fields: Vec<String> = sum["fields"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|f| {
                                    format!(
                                        "({}, {})",
                                        fmt_num(f[0].as_f64().unwrap_or(0.0)),
                                        fmt_num(f[1].as_f64().unwrap_or(0.0))
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    row(tr("Fields"), RichText::new(fields.join(" ")));
                });
            // 波長（色付き）
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Wavelengths [µm]")).color(weak));
                let p = self.primary();
                for (i, w) in self.waves_um().iter().enumerate() {
                    let t = RichText::new(fmt_num(*w)).color(theme::wave_color(*w));
                    ui.label(if i + 1 == p {
                        t.strong().underline()
                    } else {
                        t
                    });
                }
            });
            ui.separator();
            ui.label(RichText::new(tr("Parameters")).small().strong().color(weak));
            ui.label(
                RichText::new(tr(
                    "Changing a value recomputes the open analyses right away (↑↓ keys also work)",
                ))
                .small()
                .color(weak),
            );
            let mut remove = None;
            for (i, p) in self.ses.params.clone().iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(p).monospace());
                    match self.sys.param_vals.get(p).copied() {
                        Some(mut x) => {
                            let step = (x.abs() * 1e-3).max(1e-3);
                            let r =
                                ui.add(egui::DragValue::new(&mut x).speed(step).max_decimals(8));
                            if r.changed() {
                                self.sys.param_vals.insert(p.clone(), x);
                                self.pending_set.insert(p.clone(), x);
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                    if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let p = self.ses.params.remove(i);
                self.sys.param_vals.remove(&p);
            }
            ui.horizontal_wrapped(|ui| {
                let sys = self.param_sys();
                let (_, enter) = crate::param_pick::edit_pick(
                    ui,
                    &mut self.new_param,
                    120.0,
                    "3.t · STOP.r · G1.shift",
                    &sys,
                    crate::param_pick::Set::Any,
                );
                if (ui.button(tr("+ Add parameter")).clicked() || enter)
                    && !self.new_param.trim().is_empty()
                {
                    let p = self.new_param.trim().to_string();
                    self.worker.send(Tag::Get, "get", json!({ "param": p }));
                    self.new_param.clear();
                }
            });
            ui.label(
                RichText::new(tr(
                    "Names are the same as in Python and serve (3.t, STOP.r, G1.shift, …)",
                ))
                .small()
                .color(weak),
            );
            if ui
                .button(tr("Reload"))
                .on_hover_text(tr("Reload (discard changed values; Ctrl+Z to undo)"))
                .clicked()
            {
                self.reload();
            }
        });
    }

    /// 解析の引数の欄に並べる項目（視野・波長を上で選ぶ解析はそれを除く。図に要る引数は図のときは GUI が足すので除く）
    pub(super) fn shown_specs(&self, cfg: &AnaCfg) -> Vec<argform::ArgSpec> {
        let object = crate::ext::get(&cfg.cmd).and_then(|a| a.object);
        let sel = layout::takes_sel(&cfg.cmd);
        let fig = cfg.view == View::Figure;
        argform::analysis_specs(&cfg.cmd)
            .into_iter()
            // 結果の書き出し（save）は「JSON をコピー」で足りるので出さない
            // 設定のオブジェクトの引数（とそのファイル）は下の欄（JSON を読む）で
            .filter(|s| s.name != "save" && !object.is_some_and(|o| o.covers(&s.name)))
            .filter(|s| !(sel && (s.name == "field" || s.name == "wave")))
            .filter(|s| {
                !(fig
                    && matches!(
                        (cfg.cmd.as_str(), s.name.as_str()),
                        ("wavefront", "map") | ("interference" | "pop", "grid")
                    ))
            })
            .map(|mut s| {
                // GUI が既定で足す周波数は欄の薄い字に
                if matches!(cfg.cmd.as_str(), "mtf-field" | "mtf-focus") && s.name == "freqs" {
                    s.default = json!(figures::default_freqs(self.cutoff()));
                    s.required = false;
                }
                s
            })
            .collect()
    }

    /// 上の行の「⚙ 設定」（開閉と変えた数）と「すべて既定に」。計算し直すなら true
    pub(super) fn args_head(&self, ui: &mut Ui, cfg: &mut AnaCfg) -> bool {
        let shown = self.shown_specs(cfg);
        let set = shown
            .iter()
            .filter(|s| cfg.args.contains_key(&s.name))
            .count()
            + cfg.sensor.len()
            + usize::from(!cfg.extra.trim().is_empty());
        // 変えた数はボタンの中の小さい印にして幅を変えない（押そうとした欄が動かない、GUX-23）
        let head = if set > 0 {
            format!("{} ●{set}", tr("⚙ Settings"))
        } else {
            tr("⚙ Settings").to_string()
        };
        let mut rerun = false;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let w = ui.fonts(|f| {
            f.layout_no_wrap(format!("{} ●99", tr("⚙ Settings")), font, Color32::WHITE)
                .size()
                .x
        }) + 2.0 * ui.spacing().button_padding.x;
        let tip = if set > 0 {
            trf("Open / close the analysis settings ({} changed)", &[&set])
        } else {
            tr("Show or hide the analysis settings").to_string()
        };
        if ui
            .add_sized(
                [w, ui.spacing().interact_size.y],
                egui::SelectableLabel::new(cfg.args_open, head),
            )
            .on_hover_text(tip)
            .clicked()
        {
            cfg.args_open = !cfg.args_open;
        }
        if ui
            .add_enabled(set > 0, egui::Button::new("↺").small())
            .on_hover_text(tr("Reset all"))
            .clicked()
        {
            cfg.args.clear();
            cfg.sensor.clear();
            cfg.extra.clear();
            rerun = true;
        }
        rerun
    }

    /// 解析の設定の欄、拡張の解析なら設定のオブジェクトの欄、詳細（追加の引数の JSON）。計算し直すなら true
    pub(super) fn args_body(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) -> bool {
        let mut rerun = false;
        let specs = self.shown_specs(cfg);
        let shown: Vec<&argform::ArgSpec> = specs.iter().collect();
        let object = crate::ext::get(&cfg.cmd).and_then(|a| a.object);
        let weak = theme::weak_text(ui.visuals());
        if shown.is_empty() && object.is_none() {
            ui.weak(tr("This analysis has no settings"));
        } else if argform::form(ui, ("a", id), &shown, &mut cfg.args) {
            rerun = true;
        }
        if let Some(o) = object {
            if self.object_ui(ui, id, cfg, &o) {
                rerun = true;
            }
        }
        let extra_set = !cfg.extra.trim().is_empty();
        egui::CollapsingHeader::new(RichText::new(tr("Advanced (extra arguments as JSON)")).small().color(weak))
            .id_salt(("args-extra", id))
            .default_open(extra_set)
            .show(ui, |ui| {
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut cfg.extra)
                            .hint_text("{\"freqs\": [10, 20]}")
                            .desired_width(f32::INFINITY),
                    )
                    .on_hover_text(tr(
                        "The CLI arguments as JSON (_ in keys becomes -; see analyze in docs/serve.md). Overrides the fields",
                    ));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    rerun = true;
                }
            });
        rerun
    }

    /// 設定のオブジェクトの欄（拡張の解析の [`crate::ext::ObjectArg`]: センサの設定など）。JSON に保存・読込できる
    pub(super) fn object_ui(
        &mut self,
        ui: &mut Ui,
        id: u64,
        cfg: &mut AnaCfg,
        o: &crate::ext::ObjectArg,
    ) -> bool {
        let specs = argform::object_specs(o);
        let mut changed = false;
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(tr(o.title));
            let start = ui
                .small_button(tr("Load JSON…"))
                .on_hover_text(tr("JSON with the same items as the fields"))
                .clicked();
            // パネルの番号で区別する（ui.id() は兄弟の Ui で同じになる）
            let key = format!("object-{}-{id}", o.arg);
            {
                if let Some(p) = crate::files::open(&key, start, || {
                    crate::files::FileDialog::new().add_filter("JSON", &["json"])
                }) {
                    match lensbench_core::vfs::read_to_string(&p)
                        .map_err(|e| e.to_string())
                        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                    {
                        Ok(v) => {
                            let f = argform::flatten(&v);
                            let unknown: Vec<&str> = f
                                .keys()
                                .filter(|k| !specs.iter().any(|s| &s.name == *k))
                                .map(String::as_str)
                                .collect();
                            if unknown.is_empty() {
                                cfg.sensor = f;
                                changed = true;
                            } else {
                                self.fail(trf(
                                    "{}: unknown items {}",
                                    &[&p.display(), &unknown.join(", ")],
                                ));
                            }
                        }
                        Err(e) => self.fail(format!("{}: {e}", p.display())),
                    }
                }
            }
            if ui.small_button(tr("Save JSON…")).clicked() {
                match argform::to_map(&specs, &cfg.sensor) {
                    Ok(m) => {
                        if let Some(p) = crate::files::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_file_name(o.file_name)
                            .save_file()
                        {
                            let t = serde_json::to_string_pretty(&argform::nest(&m))
                                .unwrap_or_default();
                            if let Err(e) = lensbench_core::vfs::write(&p, t + "\n") {
                                self.fail(format!("{}: {e}", p.display()));
                            }
                        }
                    }
                    Err(e) => self.fail(e),
                }
            }
        });
        ui.label(
            RichText::new(tr(o.note))
                .small()
                .color(theme::weak_text(ui.visuals())),
        );
        let mut done = vec![false; specs.len()];
        let mut groups: Vec<(String, &str, Vec<usize>)> = Vec::new();
        for (prefix, title) in o.groups {
            let idx: Vec<usize> = (0..specs.len())
                .filter(|&k| !done[k] && specs[k].name.starts_with(prefix))
                .collect();
            for &k in &idx {
                done[k] = true;
            }
            groups.push((prefix.to_string(), tr(title), idx));
        }
        let rest: Vec<usize> = (0..specs.len()).filter(|&k| !done[k]).collect();
        groups.push((String::new(), tr("Other"), rest));
        for (gi, (prefix, title, idx)) in groups.iter().enumerate() {
            if idx.is_empty() {
                continue;
            }
            let group: Vec<&argform::ArgSpec> = idx.iter().map(|&k| &specs[k]).collect();
            let n = group
                .iter()
                .filter(|s| cfg.sensor.contains_key(&s.name))
                .count();
            let title = if n > 0 {
                format!("{title} ({n})")
            } else {
                title.to_string()
            };
            egui::CollapsingHeader::new(title)
                .id_salt(("object", id, prefix.as_str()))
                .default_open(gi == 0)
                .show(ui, |ui| {
                    if argform::form(ui, ("s", id, prefix.as_str()), &group, &mut cfg.sensor) {
                        changed = true;
                    }
                });
        }
        changed
    }

    /// 解析の種類の選択: 群に分けた一覧と、打って絞り込む欄（Enter で最初の候補）
    pub(super) fn analysis_picker(ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        let fkey = egui::Id::new(("ana-filter", id));
        let r = egui::ComboBox::from_id_salt(("cmd", id))
            .selected_text(layout::analysis_label(&cfg.cmd))
            .height(480.0)
            .show_ui(ui, |ui| {
                let mut q: String = ui.data(|d| d.get_temp(fkey)).unwrap_or_default();
                let te = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text(tr("Filter…"))
                        .desired_width(200.0),
                );
                // 開いたときに絞り込みの欄へ
                if !ui.data(|d| d.get_temp::<bool>(fkey.with("f")).unwrap_or(false)) {
                    te.request_focus();
                    ui.data_mut(|d| d.insert_temp(fkey.with("f"), true));
                }
                let ql = q.to_lowercase();
                let hit = |c: &str, l: &'static str| {
                    ql.is_empty()
                        || c.to_lowercase().contains(&ql)
                        || tr(l).to_lowercase().contains(&ql)
                        || layout::analysis_help(c)
                            .is_some_and(|h| tr(h).to_lowercase().contains(&ql))
                };
                let mut list: Vec<(&str, &str)> = layout::analyses()
                    .into_iter()
                    .filter(|(c, l, _)| hit(c, l))
                    .map(|(c, l, _)| (c, l))
                    .collect();
                list.sort_by_key(|(c, _)| {
                    let g = layout::analysis_group(c);
                    layout::ANALYSIS_GROUPS.iter().position(|x| *x == g)
                });
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    if let Some((c, _)) = list.first() {
                        cfg.cmd = c.to_string();
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
                ui.data_mut(|d| d.insert_temp(fkey, q.clone()));
                let weak = theme::weak_text(ui.visuals());
                let mut last = "";
                for (c, l) in &list {
                    let g = layout::analysis_group(c);
                    if g != last {
                        ui.label(RichText::new(tr(g)).small().color(weak));
                        last = g;
                    }
                    let r = ui.selectable_value(&mut cfg.cmd, c.to_string(), tr(l));
                    if let Some(h) = layout::analysis_help(c) {
                        r.on_hover_text(tr(h));
                    }
                }
                if list.is_empty() {
                    ui.weak(tr("No matching analysis"));
                }
            });
        let r = {
            // 閉じたら絞り込みを忘れる
            if r.inner.is_none() {
                ui.data_mut(|d| {
                    d.remove::<String>(fkey);
                    d.remove::<bool>(fkey.with("f"));
                });
            }
            r.response
        };
        if let Some(h) = layout::analysis_help(&cfg.cmd) {
            r.on_hover_text(tr(h));
        }
    }

    pub(super) fn analysis_ui(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        if self.sys.summary.is_none() {
            ui.weak(tr("Open a system to analyze it."));
            return;
        }
        let nf = self.n_fields();
        let waves = self.waves_um();
        let fields: Vec<String> = self
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
        let before = cfg.clone();
        let mut rerun = false;
        ui.horizontal_wrapped(|ui| {
            Self::analysis_picker(ui, id, cfg);
            // 視野・波長の 2 つの選択は 1 つの塊なので、行の残りに入らなければ先に折り返す（塊の中で折り返すと右が切れる、GUX-14）
            let need = crate::widgets::combos_width(ui, 2, 60.0);
            crate::widgets::wrap_before(ui, need);
            ui.add_enabled_ui(layout::takes_sel(&cfg.cmd), |ui| {
                let fname = |i: usize| match fields.get(i - 1) {
                    Some(f) => trf("Field {} {}", &[&i, f]),
                    None => trf("Field {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("field", id))
                    .selected_text(if cfg.field == 0 {
                        tr("All fields").to_string()
                    } else {
                        fname(cfg.field)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.field, 0, tr("All fields"));
                        for i in 1..=nf {
                            ui.selectable_value(&mut cfg.field, i, fname(i));
                        }
                    });
                let wname = |i: usize| match waves.get(i - 1) {
                    Some(w) => trf("Wave {} {} µm", &[&i, &fmt_num(*w)]),
                    None => trf("Wave {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("wave", id))
                    .selected_text(if cfg.wave == 0 {
                        tr("Polychromatic").to_string()
                    } else {
                        wname(cfg.wave)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.wave, 0, tr("Polychromatic"));
                        for i in 1..=waves.len() {
                            let w = waves[i - 1];
                            ui.selectable_value(
                                &mut cfg.wave,
                                i,
                                RichText::new(wname(i)).color(theme::wave_color(w)),
                            );
                        }
                    });
            });
            ui.checkbox(&mut cfg.live, tr("Auto"))
                .on_hover_text(tr("Recompute when the system changes"));
            if ui
                .button(tr("▶ Compute"))
                .on_hover_text(tr(
                    "Compute this analysis with the current settings (F5 computes all open analyses)",
                ))
                .clicked()
            {
                rerun = true;
            }
            rerun |= self.args_head(ui, cfg);
        });
        ui.separator();
        if cfg.args_open {
            let rect = ui.available_rect_before_wrap();
            if split::side_by_side(rect.width(), rect.height()) {
                // 横に広いパネル: 設定を左、結果を右に（境はドラッグ）
                let (mut a, mut b) = split::two(ui, ("ana-args", id), (0.4, 0.35), 220.0);
                egui::ScrollArea::vertical()
                    .id_salt(("args", id))
                    .auto_shrink([false, false])
                    .show(&mut a, |ui| rerun |= self.args_body(ui, id, cfg));
                self.result_ui(&mut b, id, cfg);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("args", id))
                    .max_height(ui.available_height() * 0.45)
                    .show(ui, |ui| rerun |= self.args_body(ui, id, cfg));
                ui.separator();
                self.result_ui(ui, id, cfg);
            }
        } else {
            self.result_ui(ui, id, cfg);
        }
        // 設定を変えたら計算し直す（追加の引数は Enter か計算ボタン）
        if rerun
            || cfg.cmd != before.cmd
            || cfg.field != before.field
            || cfg.wave != before.wave
            || (cfg.view != before.view
                && (matches!(&*cfg.cmd, "spot" | "wavefront" | "interference" | "pop")
                    || crate::ext::get(&cfg.cmd).is_some_and(|a| a.figure_params.is_some())))
        {
            self.run(id, cfg);
        }
    }
}
