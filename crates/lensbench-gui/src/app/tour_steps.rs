//! 通し確認（--tour）の各段の操作（app.rs から分けた）

use super::*;

impl App {
    /// 通し確認を 1 フレーム進める
    pub(super) fn tour_frame(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.tour.take() else {
            return;
        };
        ctx.request_repaint();
        // カタログの一覧の読み込み（下書きの worker）も待つ（待たないと、組合せの段がレンズの一覧の無いうちに進む）
        let busy = self.c.busy() || self.c.browser.lenses_pending || self.c.browser.glasses_pending;
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
        match step {
            Step::Open | Step::Done => {}
            Step::Analysis => self.run_all(false),
            Step::States => self.tour_states(ctx),
            Step::Coatings => self.tour_coatings(ctx),
            Step::UndoOpen => {
                // ズームの例を開く前の段まで戻す: 開く前の組合せ（干渉計）に戻る
                let n = self.c.undo_list(true).len();
                self.c.undo(true, n);
                self.dispatch(ctx, Msg::FocusKind(Kind::Layout));
            }
            Step::Zoom => self.tour_zoom(ctx, dir),
            // ズームの例の状態（wide・mid・tele）を多重構成の表で見せる
            Step::MultiConfig => self.dispatch(ctx, Msg::FocusKind(Kind::MultiConfig)),
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
            Step::Combo => self.tour_combo(),
            Step::Build => {
                let p = self.c.doc.combo.project();
                self.c.build_combo(p);
            }
            Step::Layout => {
                // レイアウト図: 2 枚目を選び、1 枚目の後の空気の寸法を選ぶ
                self.dispatch(ctx, Msg::FocusKind(Kind::Layout));
                // 1 枚目の後の空気の寸法を、その場で書き換えるところ
                self.c.p.combo_view.start_edit(&mut self.c.doc.combo, 0);
            }
            Step::Layout3d => {
                // 3D 表示: 2 枚目を選んだまま
                self.c.p.combo_view.select(1);
                self.c.ses.view_plane = ViewPlane::D3;
                self.c.request_layout();
            }
            Step::LensData => self.tour_lens_data(ctx),
            Step::LensDataTree => self.tour_lens_data_tree(ctx),
            Step::ComboSpot => self.tour_combo_spot(),
            Step::Optimize => self.tour_optimize(ctx),
            Step::OptApply => self.tour_opt_apply(),
            Step::Swap => self.tour_swap(ctx),
            Step::SwapInsert => self.tour_swap_insert(),
            Step::Tolerance => self.tour_tolerance(ctx),
            Step::Param => {
                // 1 枚目の後の空気を少し動かす（解析は自動で計算し直し、前との差が出る）
                self.c.ses.params = vec!["1.t".into()];
                self.c.read_params();
                self.c.pending_set.insert("1.t".into(), 2.5);
            }
            Step::Glasses => self.tour_glasses(),
            Step::Window => self.tour_window(),
            Step::Palette => {
                self.palette.show();
                self.palette.query = "spot".into();
            }
            Step::English => {
                self.palette.open = false;
                self.dispatch(ctx, Msg::Lang(Lang::En));
                self.dispatch(ctx, Msg::Theme(Mode::Light));
            }
            Step::Settings => self.tour_settings(ctx),
            Step::Interferometer => self.tour_interferometer(ctx),
            Step::FreeEdit => self.tour_free_edit(ctx),
        }
    }

    /// 通し確認の段 States
    fn tour_states(&mut self, ctx: &egui::Context) {
        // 干渉計の組合せに状態を足し、状態だけの系の値（物体距離）と経路（参照の腕）にする
        let c = &mut self.c.doc.combo;
        c.states = vec!["参照".into()];
        c.state = Some("参照".into());
        let mut s = c.snap();
        s.object_distance = Some(200.0);
        c.set_state_snap("参照", &s);
        c.state_path.insert("参照".into(), "RR".into());
        let p = c.project();
        self.c.build_combo(p);
        self.dispatch(ctx, Msg::FocusKind(Kind::SysSettings));
    }

    /// 通し確認の段 Coatings
    fn tour_coatings(&mut self, ctx: &egui::Context) {
        // 膜（フィルタ）の定義: バンドパスを定義して BS の膜にし、系の設定の膜の欄を開いて見せる
        let c = &mut self.c.doc.combo;
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
        self.dispatch(ctx, Msg::FocusKind(Kind::SysSettings));
    }

    /// 通し確認の段 Zoom
    fn tour_zoom(&mut self, ctx: &egui::Context, dir: &Path) {
        // 3 群のズーム（例の P17）を開き、群 2・3 の移動で焦点距離 140・160・175 mm に合わせ、間を 2 点ずつたどる
        // 前の組合せは文書の段に積む（最後の段で元に戻す）
        let p = crate::tour::zoom_sample(dir);
        let _ = self.c.push_doc(trf("Open {}", &[&"P17"]));
        // 組合せとして開く（その場で作るので、続けて頼むズームは組み直した系で計算する）
        if let Err(e) = self.c.open_combo_now(&p) {
            self.c.fail(e);
        }
        let z = &mut self.c.p.zoom;
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
                self.c.calc(Tag::Zoom, p);
            }
            Err(e) => self.c.fail(e),
        }
        self.dispatch(ctx, Msg::FocusKind(Kind::Zoom));
    }

    /// 通し確認の段 Combo
    fn tour_combo(&mut self) {
        // 正のレンズ（ガラスの玉。鏡は除く）を焦点距離の短い順に 2 枚。組合せの既定の入射瞳径 5 mm の光束が収まる径
        // （12 mm 以上）のものから選ぶ（メーカーのカタログには径 1〜2 mm の玉や凹面鏡があり、選ぶと光線がけられ・止まる）。
        // lensbench 形式のカタログの品（ファイルを参照する玉）は差し替え探索の段で差し替えられないので除く
        let b = &mut self.c.browser;
        let mut pos: Vec<catalog::LensRow> = b
            .lenses
            .iter()
            .filter(|l| {
                l.error.is_none()
                    && l.efl > 0.0
                    && l.diameter >= 12.0
                    && l.mirror.is_none()
                    && l.pieces >= 1
                    && l.source.is_none()
            })
            .cloned()
            .collect();
        pos.sort_by(|a, b| a.efl.total_cmp(&b.efl));
        let d = &mut self.c.doc;
        d.combo.picks.clear();
        for l in pos.iter().take(2) {
            b.add_pick(d, l);
        }
        if let Some(p) = d.combo.picks.first_mut() {
            p.air = 3.0;
        }
        b.lens_sel = b.lenses.iter().position(|l| Some(l) == pos.first());
    }

    /// 通し確認の段 LensData
    fn tour_lens_data(&mut self, ctx: &egui::Context) {
        // 面の表: 1 枚目の玉の前の面の半径を表から変え（組合せの面の上書きで作り直す）、その玉に入る
        self.c.p.combo_view.select(0);
        let ids = self.c.doc.combo.span_ids();
        let spans = lens_data::elem_spans(
            &ids,
            &self.c.sys.elem_blocks,
            self.c.doc.combo.lead_stop(),
            &self.c.sys.ld_rows,
        );
        if let Some(Some([a, _])) = spans.first().copied() {
            self.c.combo_surface_edit(&spans, a, "semi", json!(6.0));
            if !self.c.s.auto_build {
                let p = self.c.doc.combo.project();
                self.c.build_combo(p);
            }
        }
        // 最上位で 1 枚目の玉（ブロック）を選び、ブロックの設定（要素の設定）を出す
        self.c.p.ld.detail = true;
        if let Some(id) = ids.first() {
            self.c.p.ld.scope.clear();
            self.c.p.ld.sel = Some(id.clone());
            self.c.p.ld.cur = None;
        }
        self.dispatch(ctx, Msg::FocusKind(Kind::LensData));
    }

    /// 通し確認の段 LensDataTree
    fn tour_lens_data_tree(&mut self, ctx: &egui::Context) {
        // 折りたたみ表示: 1 枚目の玉をその場で開き、中の面を選ぶ
        let ids = self.c.doc.combo.span_ids();
        self.c.p.ld.tree = true;
        self.c.p.ld.scope.clear();
        if let Some(id) = ids.first() {
            self.c.p.ld.open.insert(id.clone());
            self.c.p.ld.sel = Some(id.clone());
            let first = self
                .c
                .sys
                .ld_blocks
                .iter()
                .find(|b| &b.name == id)
                .and_then(|b| b.surfs.first().copied());
            self.c.p.ld.cur = first.map(|j| (j, lens_data::Col::R));
        }
        self.dispatch(ctx, Msg::FocusKind(Kind::LensData));
    }

    /// 通し確認の段 ComboSpot
    fn tour_combo_spot(&mut self) {
        let spot = |k: &Kind| matches!(k, Kind::Analysis(a) if a.cmd == "spot");
        // 面の表の表示・見る面を戻す
        self.c.p.ld.tree = false;
        self.c.p.ld.detail = false;
        self.c.p.ld.open.clear();
        self.c.ses.view_plane = ViewPlane::Yz;
        self.c.request_layout();
        if let Some(id) = self.pane_of(spot) {
            self.focus(id);
        }
    }

    /// 通し確認の段 Optimize
    fn tour_optimize(&mut self, ctx: &egui::Context) {
        // 最適化: 像面の前の間隔を変数に、既定の RMS スポットと縁の光線の項で回す
        self.dispatch(ctx, Msg::FocusKind(Kind::Optimize));
        let img = self.c.sys.ld_rows.len().saturating_sub(1);
        let o = &mut self.c.p.opt;
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
            self.c.calc(Tag::Optimize, p);
        }
    }

    /// 通し確認の段 OptApply
    fn tour_opt_apply(&mut self) {
        // 結果を組合せに当てる（後の空気が変わり、系を作り直して解析も計算し直す）
        match self.c.p.opt.result.clone() {
            Some(r) => {
                let v = crate::optim_panel::result_values(&r, false);
                if self.c.apply_values(&v) {
                    self.c.p.opt.undo = Some(crate::optim_panel::result_values(&r, true));
                }
            }
            None => self.c.fail("最適化の結果がない"),
        }
    }

    /// 通し確認の段 Swap
    fn tour_swap(&mut self, ctx: &egui::Context) {
        // 差し替え探索: 2 枚目をカタログの全部（裏返しも）で差し替え、間の空気と像面までを最適化
        self.dispatch(ctx, Msg::FocusKind(Kind::Swap));
        let (proj, cands) = crate::swap_panel::combo_project(&self.c.doc.combo);
        let sw = &mut self.c.p.swap;
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
        let n = self.c.doc.combo.picks.len();
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
            self.c.swap_ids = self.c.doc.combo.ids();
            self.c.calc(Tag::Swap, v);
        }
    }

    /// 通し確認の段 SwapInsert
    fn tour_swap_insert(&mut self) {
        // 1 位の組を組合せに入れる（系を作り直して解析も計算し直す）
        let row = self
            .c
            .p
            .swap
            .result
            .as_ref()
            .and_then(|r| r.results.as_ref()?.first())
            .cloned();
        let before = self.c.doc.combo.clone();
        let r = row
            .ok_or_else(|| "差し替え探索の結果がない".to_string())
            .and_then(|row| {
                crate::swap_panel::insert_row(&mut self.c.doc.combo, &self.c.swap_ids, &row)
            });
        match r {
            Ok(()) => {
                self.c.p.swap.undo = Some(before);
                self.c.p.swap.inserted = Some(0);
                let p = self.c.doc.combo.project();
                self.c.build_combo(p);
            }
            Err(e) => self.c.fail(e),
        }
    }

    /// 通し確認の段 Tolerance
    fn tour_tolerance(&mut self, ctx: &egui::Context) {
        // モンテカルロ公差: 精密の組、像面の前の間隔を補償子に、30 回
        self.dispatch(ctx, Msg::FocusKind(Kind::Tolerance));
        let img = self.c.sys.ld_rows.len().saturating_sub(1);
        let t = &mut self.c.p.tol;
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
                self.c.calc(Tag::Tolerance, p);
            }
            Err(e) => self.c.fail(e),
        }
    }

    /// 通し確認の段 Glasses
    fn tour_glasses(&mut self) {
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

    /// 通し確認の段 Window
    fn tour_window(&mut self) {
        let spot = |k: &Kind| matches!(k, Kind::Analysis(a) if a.cmd == "spot");
        // MTF を別の窓に出す（本体の画面では MTF が消え、スポットが表に出る）
        if let Some(id) = self.pane_of(|k| matches!(k, Kind::Analysis(a) if a.cmd == "mtf")) {
            self.detach(id);
        }
        if let Some(id) = self.pane_of(spot) {
            self.focus(id);
        }
    }

    /// 通し確認の段 Settings
    fn tour_settings(&mut self, ctx: &egui::Context) {
        // 系の設定のパネル: 視野に重み、波長を 1 つ足し、エイミングを実光線に（系は自動で作り直す）
        self.dispatch(ctx, Msg::Lang(Lang::Ja));
        self.dispatch(ctx, Msg::Theme(Mode::Dark));
        self.dispatch(ctx, Msg::FocusKind(Kind::SysSettings));
        let c = &mut self.c.doc.combo;
        if let Some(f) = c.fields.last_mut() {
            f.w = 0.5;
        }
        c.waves.push(crate::sysset::WavePt { um: 0.55, w: 2.0 });
        c.aiming = crate::sysset::Aiming::Real;
    }

    /// 通し確認の段 Interferometer
    fn tour_interferometer(&mut self, ctx: &egui::Context) {
        // BS の分岐で組んだ Twyman-Green。経路 TR（試験の腕を往復して口 D へ）で、BS を選ぶ
        let mut c = combo::Combo::twyman_green();
        c.path = "TR".into();
        c.object_distance = Some(100.0);
        c.field_kind = crate::sysset::FieldKind::ObjectHeight;
        c.fields = [0.0, 0.5].map(crate::sysset::FieldPt::y).to_vec();
        c.aperture = crate::sysset::ApertureKind::Na;
        c.ap_value = 0.02;
        c.waves = vec![crate::sysset::WavePt::um(0.6328)];
        c.primary = 1;
        self.c.doc.combo = c;
        // 参照の腕（RR）も重ねて描く。光線は視野 1 の十字で、光線・書き出しの設定の欄も開いて見せる
        self.c.overlay.items = ["TR", "RR"]
            .map(|x| ("paths", x.to_string(), true))
            .to_vec();
        self.c.s.rays.fields = vec![1];
        self.c.s.rays.fan = "cross".into();
        self.c.rays_open = true;
        let p = self.c.doc.combo.project();
        self.c.build_combo(p);
        self.c.p.combo_view.select(1);
        self.dispatch(ctx, Msg::FocusKind(Kind::Layout));
    }

    /// 通し確認の段 FreeEdit
    fn tour_free_edit(&mut self, ctx: &egui::Context) {
        // 空の文書から面を足していく: 平面 1 枚の面の並びに半径と硝材を入れて玉にし、後ろに平面を足す
        let mut c = combo::Combo {
            epd: 10.0,
            ..Default::default()
        };
        c.insert_plane(0);
        c.picks[0].surfs[0].radius = 50.0;
        c.picks[0].air = 5.0;
        if let Err(e) = c.glass_into_surfaces(0, 0, "N-BK7") {
            self.c.fail(e);
        }
        if let Some(v) = c.gap_after(0) {
            *v = 40.0;
        }
        c.insert_plane(1);
        self.c.doc.combo = c;
        self.c.overlay.items.clear();
        let p = self.c.doc.combo.project();
        self.c.build_combo(p);
        self.c.p.combo_view.select(0);
        self.dispatch(ctx, Msg::FocusKind(Kind::LensData));
    }
}
