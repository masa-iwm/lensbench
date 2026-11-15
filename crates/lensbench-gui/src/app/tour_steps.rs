//! 通し確認（--tour）の各段の操作（GAR-18 で app.rs から分けた）

use super::*;

impl App {
    /// 通し確認を 1 フレーム進める
    pub(super) fn tour_frame(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.tour.take() else {
            return;
        };
        ctx.request_repaint();
        let busy = self.c.worker.busy();
        if t.timed_out() {
            t.errors.push(format!("{} で時間切れ", t.step.name()));
            t.step = Step::Done;
        }
        if t.step != Step::Done {
            if t.take_shot(ctx) {
                // 撮れた。次の段へ
            } else if t.ready(busy) {
                if self.c.status.error {
                    t.errors
                        .push(format!("{}: {}", t.step.name(), self.c.status.text));
                    self.c.status.error = false;
                }
                if !t.acted {
                    t.acted = true;
                    let dir = t.dir.clone();
                    self.tour_act(ctx, t.step, &dir);
                } else {
                    t.request_shot(ctx);
                }
            }
        }
        if t.step == Step::Done {
            t.finish();
            // 通し確認は保存していない変更があっても聞かずに閉じる（ここで tour を外すので close_guard の tour の除外は
            // 効かない）
            self.c.close_ok = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.tour = Some(t);
        }
    }

    /// 通し確認の各段の操作
    pub(super) fn tour_act(&mut self, ctx: &egui::Context, step: Step, dir: &Path) {
        let spot = |k: &Kind| matches!(k, Kind::Analysis(a) if a.cmd == "spot");
        match step {
            Step::Open | Step::Done => {}
            Step::Analysis => self.run_all(false),
            Step::States => {
                // 干渉計の組合せに状態を足し、状態だけの系の値（物体距離）と経路（参照の腕）にする
                let c = &mut self.c.browser.combo;
                c.states = vec!["参照".into()];
                c.state = Some("参照".into());
                let mut s = c.snap();
                s.object_distance = Some(200.0);
                c.state_sys.insert("参照".into(), s);
                c.state_path.insert("参照".into(), "RR".into());
                let p = c.project();
                self.c.build_combo(p);
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
            }
            Step::Coatings => {
                // 膜（フィルタ）の定義: バンドパスを定義して BS の膜にし、系の設定の膜の欄を開いて見せる
                let c = &mut self.c.browser.combo;
                c.coatings.insert(
                    "BP633".into(),
                    lensbench_project::filter::CoatSpec::Bandpass {
                        center_nm: 633.0,
                        fwhm_nm: 10.0,
                        ideal: lensbench_project::filter::Ideal {
                            t: 0.9,
                            od: 4.0,
                            width_nm: None,
                            neff: 2.0,
                            absorptive: false,
                        },
                    },
                );
                let p = c.project();
                self.c.build_combo(p);
                crate::coat_defs::focus(ctx);
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
            }
            Step::Units => {
                // 開いた系（P17）に群 G23（群 2・3）を足し、機構部品（筒）を置いて、群 G2 の設定と欄を見せる
                let u = |n: &str| {
                    self.c
                        .ld_units
                        .iter()
                        .find(|u| u.name == n)
                        .map(|u| u.spans.clone())
                };
                let mut ops = Vec::new();
                if let (Some(a), Some(b)) = (u("G2"), u("G3")) {
                    ops.push(json!({ "op": "define_unit", "name": "G23", "spans": [[a[0][0], b[0][1]]] }));
                }
                ops.push(json!({ "op": "add_mech", "part": {
                    "name": "tube", "surface": 1, "offset": [0.0, 0.0, -2.0],
                    "bores": [{ "z0": 0.0, "z1": 40.0, "radius": 15.0 }], "outer_radius": 18.0,
                }}));
                self.c.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                self.c.ld.scope.clear();
                self.c.ld.sel = Some("G2".into());
                self.c.ld.block_view = Some("G2".into());
                self.c.um_open = true;
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::Zoom => {
                // 3 群のズーム（例の P17）を開き、群 2・3 の移動で焦点距離 140・160・175 mm に合わせ、間を 2 点ずつたどる
                self.c.open(&crate::tour::zoom_sample(dir));
                let z = &mut self.c.zoom;
                z.vars = ["G2.shift", "G3.shift"]
                    .map(|n| crate::zoom_panel::VarRow {
                        name: n.into(),
                        ..Default::default()
                    })
                    .to_vec();
                z.steps = 2;
                let cands: Vec<crate::zoom_panel::Cand> =
                    [("wide", "140"), ("mid", "160"), ("tele", "175")]
                        .map(|(n, t)| {
                            z.states.insert(
                                n.into(),
                                crate::zoom_panel::StateRow {
                                    on: true,
                                    target: t.into(),
                                },
                            );
                            (false, n.to_string())
                        })
                        .to_vec();
                match z.params(&cands) {
                    Ok(p) => {
                        z.running = true;
                        self.c.worker.send(Tag::Zoom, "zoom", p);
                    }
                    Err(e) => self.c.fail(e),
                }
                self.exec(ctx, Cmd::FocusKind(Kind::Zoom));
            }
            Step::Pop => {
                // 簡易 POP のパネルを足して計算する（像面の前の面から像面まで）
                let cfg = AnaCfg::new("pop");
                let id = self.add_pane(Kind::Analysis(cfg.clone()));
                self.focus(id);
                self.c.run(id, &cfg);
            }
            Step::Catalog => {
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Catalog)) {
                    self.focus(id);
                }
                self.c.browser.tab = catalog::Tab::Lenses;
                self.c.refresh_catalog();
            }
            Step::Combo => {
                // 正のレンズを焦点距離の短い順に 2 枚
                let b = &mut self.c.browser;
                let mut pos: Vec<catalog::LensRow> = b
                    .lenses
                    .iter()
                    .filter(|l| l.error.is_none() && l.efl > 0.0)
                    .cloned()
                    .collect();
                pos.sort_by(|a, b| a.efl.total_cmp(&b.efl));
                b.combo.picks.clear();
                for l in pos.iter().take(2) {
                    b.add_pick(l);
                }
                if let Some(p) = b.combo.picks.first_mut() {
                    p.air = 3.0;
                }
                b.lens_sel = b.lenses.iter().position(|l| Some(l) == pos.first());
            }
            Step::Build => {
                let p = self.c.browser.combo.project();
                self.c.build_combo(p);
            }
            Step::Layout => {
                // レイアウト図: 2 枚目を選び、1 枚目の後の空気の寸法を選ぶ
                self.exec(ctx, Cmd::FocusKind(Kind::Layout));
                // 1 枚目の後の空気の寸法を、その場で書き換えるところ
                self.c.combo_view.start_edit(&mut self.c.browser.combo, 0);
            }
            Step::Layout3d => {
                // 3D 表示: 2 枚目を選んだまま
                self.c.combo_view.select(1);
                self.c.s.view_plane = ViewPlane::D3;
                self.c.request_layout();
            }
            Step::LensData => {
                // 面の表: 1 枚目の玉の前の面の半径を表から変え（組合せの面の上書きで作り直す）、その玉に入る
                self.c.combo_view.select(0);
                let ids = self.c.browser.combo.span_ids();
                let spans = lens_data::elem_spans(
                    &ids,
                    &self.c.ld_units,
                    self.c.browser.combo.lead_stop(),
                    &self.c.ld_rows,
                );
                if let Some(Some([a, _])) = spans.first().copied() {
                    self.c.combo_surface_edit(&spans, a, "semi", json!(6.0));
                    if !self.c.s.auto_build {
                        let p = self.c.browser.combo.project();
                        self.c.build_combo(p);
                    }
                }
                // 最上位で 1 枚目の玉（ブロック）を選び、ブロックの設定（要素の設定）を出す
                self.c.s.ld_detail = true;
                if let Some(id) = ids.first() {
                    self.c.ld.scope.clear();
                    self.c.ld.sel = Some(id.clone());
                    self.c.ld.cur = None;
                }
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::LensDataTree => {
                // 折りたたみ表示: 1 枚目の玉をその場で開き、中の面を選ぶ
                let ids = self.c.browser.combo.span_ids();
                self.c.s.ld_tree = true;
                self.c.ld.scope.clear();
                if let Some(id) = ids.first() {
                    self.c.ld.open.insert(id.clone());
                    self.c.ld.sel = Some(id.clone());
                    let first = self
                        .c
                        .ld_blocks
                        .iter()
                        .find(|b| &b.name == id)
                        .and_then(|b| b.surfs.first().copied());
                    self.c.ld.cur = first.map(|j| (j, lens_data::Col::R));
                }
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::ComboSpot => {
                // 面の表の表示・見る面を戻す
                self.c.s.ld_tree = false;
                self.c.s.ld_detail = false;
                self.c.ld.open.clear();
                self.c.s.view_plane = ViewPlane::Yz;
                self.c.request_layout();
                if let Some(id) = self.pane_of(spot) {
                    self.focus(id);
                }
            }
            Step::Optimize => {
                // 最適化: 像面の前の間隔を変数に、既定の RMS スポットと縁の光線の項で回す
                self.exec(ctx, Cmd::FocusKind(Kind::Optimize));
                let img = self.c.ld_rows.len().saturating_sub(1);
                let o = &mut self.c.opt;
                o.vars = vec![crate::optim_panel::VarRow {
                    name: format!("{}.t", img.saturating_sub(1)),
                    lo: "0".into(),
                    ..Default::default()
                }];
                let mut t = crate::optim_panel::TermRow::new("ray");
                t.weight = "0.1".into();
                o.terms = vec![t];
                if let Ok(p) = o.params() {
                    o.running = true;
                    o.gen = self.c.sys_gen;
                    self.c.worker.send(Tag::Optimize, "optimize", p);
                }
            }
            Step::OptApply => {
                // 結果を組合せに当てる（後の空気が変わり、系を作り直して解析も計算し直す）
                match self.c.opt.result.clone() {
                    Some(r) => {
                        let v = crate::optim_panel::result_values(&r, false);
                        if self.c.apply_values(&v) {
                            self.c.opt.undo = Some(crate::optim_panel::result_values(&r, true));
                        }
                    }
                    None => self.c.fail("最適化の結果がない"),
                }
            }
            Step::Swap => {
                // 差し替え探索: 2 枚目をカタログの全部（裏返しも）で差し替え、間の空気と像面までを最適化
                self.exec(ctx, Cmd::FocusKind(Kind::Swap));
                let (proj, cands) = crate::swap_panel::combo_project(&self.c.browser.combo);
                let sw = &mut self.c.swap;
                sw.slots = cands
                    .blocks
                    .iter()
                    .skip(1)
                    .take(1)
                    .map(|b| crate::swap_panel::SlotRow {
                        block: b.0.clone(),
                        flip: true,
                        ..Default::default()
                    })
                    .collect();
                let n = self.c.browser.combo.picks.len();
                sw.vars = [(1, "0.5"), (n, "1")]
                    .iter()
                    .map(|(k, lo)| crate::swap_panel::VarRow {
                        name: format!("d{k}.t"),
                        lo: lo.to_string(),
                        hi: String::new(),
                    })
                    .collect();
                if let Ok(mut v) = sw.spec() {
                    v["project"] = proj;
                    v["top"] = json!(sw.top);
                    sw.running = true;
                    self.c.swap_ids = self.c.browser.combo.ids();
                    self.c.worker.send(Tag::Swap, "swap", v);
                }
            }
            Step::SwapInsert => {
                // 1 位の組を組合せに入れる（系を作り直して解析も計算し直す）
                let row = self
                    .c
                    .swap
                    .result
                    .as_ref()
                    .and_then(|r| r.results.as_ref()?.first())
                    .cloned();
                let before = self.c.browser.combo.clone();
                let r = row
                    .ok_or_else(|| "差し替え探索の結果がない".to_string())
                    .and_then(|row| {
                        crate::swap_panel::insert_row(
                            &mut self.c.browser.combo,
                            &self.c.swap_ids,
                            &row,
                        )
                    });
                match r {
                    Ok(()) => {
                        self.c.swap.undo = Some(before);
                        self.c.swap.inserted = Some(0);
                        let p = self.c.browser.combo.project();
                        self.c.build_combo(p);
                    }
                    Err(e) => self.c.fail(e),
                }
            }
            Step::Tolerance => {
                // モンテカルロ公差: 精密の組、像面の前の間隔を補償子に、30 回
                self.exec(ctx, Cmd::FocusKind(Kind::Tolerance));
                let img = self.c.ld_rows.len().saturating_sub(1);
                let t = &mut self.c.tol;
                t.trials = 30;
                t.preset = "precision".into();
                // カタログの玉の公差: どのメーカーも中心厚 ±0.05 mm、外径 10 mm 未満は焦点距離 ±1%
                t.catalog_tols = crate::tol_catalog::from_json(&serde_json::json!({
                    "*": { "thickness": 0.05, "rules": [{ "dia_lt": 10, "focal_rel": 0.01 }] }
                }))
                .unwrap_or_default();
                t.percentiles = "50, 90, 98".into();
                t.opt.vars = vec![crate::optim_panel::VarRow {
                    name: format!("{}.t", img.saturating_sub(1)),
                    ..Default::default()
                }];
                match t.spec() {
                    Ok(p) => {
                        t.running = true;
                        self.c.worker.send(Tag::Tolerance, "tolerance", p);
                    }
                    Err(e) => self.c.fail(e),
                }
            }
            Step::Param => {
                // 1 枚目の後の空気を少し動かす（解析は自動で計算し直し、前との差が出る）
                self.c.s.params = vec!["1.t".into()];
                self.c.read_params();
                self.c.pending_set.insert("1.t".into(), 2.5);
            }
            Step::Glasses => {
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Catalog)) {
                    self.focus(id);
                }
                let b = &mut self.c.browser;
                b.tab = catalog::Tab::Glasses;
                b.glass_sel = (!b.glasses.is_empty()).then_some(0);
                // アッベ図は選んだガラスの辺りを 3 倍に（目盛りが範囲に合うかを見る）
                b.glass_view = catalog::MapView::FULL;
                if let Some(g) = b.glass_sel.and_then(|i| b.glasses.get(i)) {
                    if g.vd.is_finite() && g.nd.is_finite() {
                        let (vd, nd) = (g.vd, g.nd);
                        b.glass_view.zoom(vd, nd, 3.0);
                    }
                }
            }
            Step::Window => {
                // MTF を別の窓に出す（本体の画面では MTF が消え、スポットが表に出る）
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Analysis(a) if a.cmd == "mtf"))
                {
                    self.detach(id);
                }
                if let Some(id) = self.pane_of(spot) {
                    self.focus(id);
                }
            }
            Step::Palette => {
                self.palette.show();
                self.palette.query = "spot".into();
            }
            Step::English => {
                self.palette.open = false;
                self.exec(ctx, Cmd::Lang(Lang::En));
                self.exec(ctx, Cmd::Theme(Mode::Light));
            }
            Step::Settings => {
                // 系の設定のパネル: 視野に重み、波長を 1 つ足し、エイミングを実光線に（系は自動で作り直す）
                self.exec(ctx, Cmd::Lang(Lang::Ja));
                self.exec(ctx, Cmd::Theme(Mode::Dark));
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
                let c = &mut self.c.browser.combo;
                if let Some(f) = c.fields.last_mut() {
                    f.w = 0.5;
                }
                c.waves.push(crate::sysset::WavePt { um: 0.55, w: 2.0 });
                c.aiming = crate::sysset::Aiming::Real;
            }
            Step::Interferometer => {
                // BS の分岐で組んだ Twyman-Green。経路 TR（試験の腕を往復して口 D へ）で、BS を選ぶ
                let mut c = combo::Combo::twyman_green();
                c.path = "TR".into();
                c.object_distance = Some(100.0);
                c.field_kind = crate::sysset::FieldKind::ObjectHeight;
                c.fields = [0.0, 0.5].map(crate::sysset::FieldPt::y).to_vec();
                c.aperture = crate::sysset::ApKind::Na;
                c.ap_value = 0.02;
                c.waves = vec![crate::sysset::WavePt::um(0.6328)];
                c.primary = 1;
                self.c.browser.combo = c;
                // 参照の腕（RR）も重ねて描く。光線は視野 1 の十字で、光線・書き出しの設定の欄も開いて見せる
                self.c.overlay.items = ["TR", "RR"]
                    .map(|x| ("paths", x.to_string(), true))
                    .to_vec();
                self.c.s.rays.fields = vec![1];
                self.c.s.rays.fan = "cross".into();
                self.c.rays_open = true;
                let p = self.c.browser.combo.project();
                self.c.build_combo(p);
                self.c.combo_view.select(1);
                self.exec(ctx, Cmd::FocusKind(Kind::Layout));
            }
        }
    }
}
