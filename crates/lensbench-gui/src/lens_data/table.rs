//! 面の表を描く（[`ui`]）。1 つの関数だった描画・キー・編集の確定・メニューを、表の段（[`View`]・[`Out`]）と
//! セル・キー・確定・詳細設定の関数に分けた（lens_data.rs から分けた）

use std::collections::{BTreeSet, HashMap};

use egui::{Color32, Key, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

mod cells;
mod input;

use super::lines::{lines_of, run_end, visible_key, Line};
use super::strip::{strip_ui, StripCtx, StripOut};
use super::*;
use cells::{block_cell, cell, num_cell, row_end};
use input::{apply, kbd_menu, key_input, surfaces};

/// このフレームの表（行・列・選択など、描く前に一度だけ求めるもの）
struct View {
    /// 入っているブロック
    scope: Option<usize>,
    lines: Vec<Line>,
    /// 面 → 行（行ごとに何度も引くので表にする）
    line_ix: HashMap<usize, usize>,
    /// 行の鍵の面
    keys: Vec<usize>,
    cols: Vec<Col>,
    /// 選んだ行（Shift+クリック・Shift+↑↓ で範囲。コピー・貼り付け・削除・玉や群を作る）
    selected: Vec<usize>,
    /// 選んだ行の面の範囲（たたんだ行はブロックの最初の面から）
    sel_span: Option<[usize; 2]>,
    sel_block: Option<String>,
    /// 像面の番号
    last: usize,
    tree: bool,
    weak: Color32,
    accent: Color32,
    shift_down: bool,
    /// 面の複屈折の値（硝材の追加設定。serve の surface_data で読む）
    bire: Option<(usize, Value)>,
    cur_typ: Option<String>,
}

const ROW_H: f32 = 20.0;

impl View {
    fn line(&self, j: usize) -> Option<&Line> {
        self.line_ix.get(&j).map(|&i| &self.lines[i])
    }

    fn folded(&self, j: usize) -> Option<(usize, usize)> {
        self.line(j).and_then(|l| l.fold)
    }

    /// 面 j の行の属するブロック（選ぶ・入る・開くブロック）
    fn child_of(&self, j: usize) -> Option<usize> {
        self.line(j).and_then(|l| l.blk)
    }

    fn bire_of(&self, j: usize) -> Option<&Value> {
        self.bire.as_ref().filter(|b| b.0 == j).map(|b| &b.1)
    }

    /// 面 j の列 col を変えられない理由（ロック・組合せ・その面タイプに無いパラメータ）
    fn cell_ro(&self, c: &Ctx, j: usize, col: Col) -> Option<String> {
        let r = c.rows.get(j)?;
        if let Col::Par(i) = col {
            if par_label(&r.typ, i).is_none() {
                return Some(trf("{} has no parameter {}", &[&r.typ, &i]));
            }
        }
        if col == Col::Type && (j == 0 || j == self.last) {
            return Some(tr("The object and image surface types cannot be changed").into());
        }
        locked_cell(c.blocks, c.locked, j, col)
            .map(|n| trf("{} is locked", &[&n]))
            .or_else(|| (c.read_only)(j, col))
    }

    /// 面 j の行の右クリックの欄（面の番号・ブロックの列・セル・キーボードの 4 か所で同じ）
    fn row_menu(&self, ui: &mut Ui, menu: &mut CellMenu, c: &Ctx, j: usize) -> RowMenu {
        let row_ro = locked_at(c.blocks, c.locked, j).map(|n| trf("{} is locked", &[&n]));
        row_menu(
            ui,
            menu,
            j,
            &c.rows[j],
            row_ro.as_deref(),
            self.last,
            self.sel_span,
            c.rows,
            c.blocks,
            c.groups,
        )
    }
}

/// このフレームで決めたこと（セル・キー・メニューから。描き終えてから当てる）
#[derive(Default)]
struct Out {
    acts: Vec<Act>,
    /// 右クリックの欄・帯・キーで決めた操作（足す・消す・貼る・詳細設定）
    fx: RowMenu,
    /// 書き換えを始めるセル（面、列、打った文字。None は今の値を全部選んで）
    start: Option<(usize, Col, Option<String>)>,
    /// 決めた書き換え（面、列、文字）と、その後に今のセルを動かす向き（行、列）
    commit: Option<(usize, Col, String)>,
    after: Option<(i32, i32)>,
    cancel: bool,
    clicked_any: bool,
    cur_pos: Option<egui::Pos2>,
    /// 横に見える所へスクロールする今のセル（表の中の縦のスクロールは横の行き先を捨てるので、外の横のスクロールで）
    hscroll_to: Option<egui::Rect>,
    /// 開く・たたむブロック（折りたたみ表示）
    toggle: Option<String>,
    /// 入るブロック
    enter: Option<String>,
    go: Option<usize>,
    export_btn: bool,
}

impl Out {
    /// ブロック i を選ぶ（変わったときだけ知らせる）
    fn select(&mut self, st: &mut LensData, c: &Ctx, i: usize) {
        if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
            st.sel = Some(c.blocks[i].name.clone());
            self.acts.push(Act::Select(st.sel.clone()));
        }
    }

    /// ブロックを選ばない（面単位の選択でブロックの中の面・ブロックの外の面を選んだとき。選んでいたときだけ知らせる）
    fn unselect(&mut self, st: &mut LensData) {
        if st.sel.take().is_some() {
            self.acts.push(Act::Select(None));
        }
    }

    /// ブロック i を開く・たたむ（折りたたみ表示）か、中へ入る
    fn open_or_enter(&mut self, c: &Ctx, tree: bool, i: usize) {
        if tree {
            self.toggle = Some(c.blocks[i].name.clone());
        } else {
            self.enter = Some(c.blocks[i].name.clone());
        }
    }
}

/// 返す Ui は設定の欄（ブロックの設定はパネルの側でここに描く）
pub(crate) fn ui(ui: &mut Ui, st: &mut LensData, c: &Ctx) -> (Vec<Act>, Ui) {
    let mut out = Out::default();
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    // 消えたブロックに入っていたら外へ
    while !st.scope.is_empty() && st.scope_index(c.blocks).is_none() {
        st.scope.pop();
    }
    if st
        .sel
        .as_ref()
        .is_some_and(|s| !c.blocks.iter().any(|b| &b.name == s))
    {
        st.sel = None;
    }
    // マウスの戻るボタン（パネルの上）で親のブロックへ
    let mouse_back = ui.rect_contains_pointer(ui.max_rect())
        && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1));
    if mouse_back && !st.scope.is_empty() {
        st.sel = st.scope.pop();
        st.cur = None;
        st.edit = None;
        st.active = true;
    }
    let scope = st.scope_index(c.blocks);
    let surfs: Vec<usize> = match scope {
        Some(i) => c.blocks[i].surfs.clone(),
        None => (0..c.rows.len()).collect(),
    };
    let children: Vec<usize> = (0..c.blocks.len())
        .filter(|&i| c.blocks[i].parent == scope)
        .collect();
    // 消えたブロックは開いた組から外す。新しいブロックは開く（折りたたみ表示で面が全部見える）
    st.open.retain(|n| c.blocks.iter().any(|b| &b.name == n));
    for b in c.blocks {
        if st.seen.insert(b.name.clone()) {
            st.open.insert(b.name.clone());
        }
    }
    // 表の行: 今の階層の面と、直下のブロックを 1 行にたたんだもの（OpticStudio の Black Box のように。中は入って見る）。
    // 折りたたみ表示では開いたブロックの中の面もその場に並べる
    let tree = st.tree;
    let lines = {
        let open_set = &st.open;
        let is_open = |i: usize| tree && open_set.contains(&c.blocks[i].name);
        lines_of(c.blocks, scope, &surfs, &is_open, 0)
    };
    let keys: Vec<usize> = lines.iter().map(|l| l.key).collect();
    let line_ix: HashMap<usize, usize> =
        lines.iter().enumerate().map(|(i, l)| (l.key, i)).collect();
    // 面を足した後は、新しい面の表が届いたら今のセルを足した面へ（届くまでは今のセルを動かさない）
    if let Some((at, n)) = st.ins_at {
        if c.rows.len() != n {
            st.ins_at = None;
            st.cur = Some((at, st.cur.map_or(Col::R, |x| x.1)));
            st.anchor = None;
        }
    }
    // 今のセルが見えない面（たたんだブロックの中・表示の切り替え・全部たたむの後）なら、それを含むたたんだ行へ
    if let (Some((j, col)), None) = (st.cur, st.ins_at) {
        let vis = visible_key(&lines, c.blocks, j);
        if vis != Some(j) {
            st.cur = vis.map(|k| (k, col));
            st.edit = None;
        }
    }
    // 今の階層の中のブロック（全部開く・たたむ）
    let scope_surfs: BTreeSet<usize> = surfs.iter().copied().collect();
    let inside: Vec<String> = c
        .blocks
        .iter()
        .enumerate()
        .filter(|&(i, b)| match scope {
            Some(s) => i != s && b.surfs.iter().all(|x| scope_surfs.contains(x)),
            None => true,
        })
        .map(|(_, b)| b.name.clone())
        .collect();

    // 左端の縦の帯: 階層・ブロック・面の挿入削除・詳細設定・玉群・保存のアイコン。押したときだけ中身（一覧・ドロップダウン）を出す
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    // 幅があるときは帯のアイコンに字も出す（絵だけでは意味が分からない）
    // 記号がフォントに無い（豆腐 □ になる）ときも字を出す
    let tofu = |t: &&str| !ui.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(15.0), t));
    let wide = full.width() >= 760.0 || ICONS.iter().any(tofu);
    // 字を出すときの幅は、いちばん長い字（言語で変わる）に合わせる（訳語が長いと折り返す）
    let strip_w = if wide {
        let pad = 2.0 * ui.spacing().button_padding.x + 6.0;
        let longest = ICONS
            .iter()
            .filter_map(|t| icon_label(t).map(|l| format!("{t} {}", tr(l))))
            .map(|s| {
                ui.fonts_mut(|f| {
                    f.layout_no_wrap(s, egui::FontId::proportional(12.0), Color32::WHITE)
                        .size()
                        .x
                })
            })
            .fold(0.0_f32, f32::max);
        (longest + pad).clamp(74.0, 120.0)
    } else {
        30.0
    };
    let strip_r = egui::Rect::from_min_size(full.min, egui::vec2(strip_w, full.height()));
    let body_r = egui::Rect::from_min_max(
        egui::pos2(full.left() + strip_w + 5.0, full.top()),
        full.max,
    );
    ui.painter().vline(
        strip_r.right() + 2.0,
        strip_r.y_range(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    let mut strip = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("ld_strip")
            .max_rect(strip_r)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("ld_body")
            .max_rect(body_r)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let mut so = StripOut::default();
    strip_ui(
        &mut strip,
        st,
        c,
        &StripCtx {
            inside: &inside,
            wide,
            strip_w,
            weak,
            children: &children,
            tree,
        },
        &mut so,
        &mut out.acts,
    );
    out.go = so.go;
    out.enter = so.enter;
    out.export_btn = so.export_btn;
    out.toggle = so.toggle;
    out.fx = so.fx;
    let ui = &mut body;
    if let Some(n) = out.go {
        st.scope.truncate(n);
        st.cur = None;
        st.edit = None;
    }

    let bire = st
        .insp
        .data
        .as_ref()
        .map(|(dj, v)| (*dj, v["data"]["bire"].clone()));
    let selected = selection(&keys, st.anchor, st.cur.map(|x| x.0));
    // パラメータの列は系の面タイプの最大の数（OpticStudio の Par 1〜）
    let npar = c.rows.iter().map(|r| par_count(&r.typ)).max().unwrap_or(0);
    let mut v = View {
        scope,
        lines,
        line_ix,
        keys,
        cols: Col::all(npar),
        selected,
        sel_span: None,
        sel_block: st.sel.clone(),
        last: c.rows.len().saturating_sub(1),
        tree,
        weak,
        accent,
        shift_down: ui.input(|i| i.modifiers.shift),
        bire,
        cur_typ: st
            .cur
            .and_then(|(j, _)| c.rows.get(j))
            .map(|r| r.typ.clone()),
    };
    v.sel_span = {
        let lo = v
            .selected
            .iter()
            .map(|&j| v.folded(j).map_or(j, |(_, a)| a))
            .min();
        let hi = v.selected.iter().copied().max();
        lo.zip(hi).map(|(a, b)| [a, b])
    };
    if let Some((j, Col::Material)) = st.cur {
        if j < c.rows.len() && v.bire_of(j).is_none() && st.insp.asked != Some(j) {
            st.insp.asked = Some(j);
            out.acts.push(Act::NeedData(j));
        }
    }
    // 詳細設定を出すときは、表と設定に残りを分ける（パネルが低くても設定の欄が押し出されないように。
    // 横に広く低いパネルでは左右に並べる。境はドラッグで動かせる）。出さないときは表が残りを全部使う
    let (mut top, bottom) = if st.detail {
        crate::split::two(ui, "lens_data", (0.5, 0.55), 90.0)
    } else {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let child = |ui: &mut Ui, r: egui::Rect, salt: &str| {
            ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("lens_data_full", salt))
                    .max_rect(r)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            )
        };
        // 下に詳細設定を折りたたんだ帯（クリックで開く）
        let bar_h = 20.0;
        let (table_r, bar_r) = if rect.height() > 3.0 * bar_h {
            let (a, b) = rect.split_top_bottom_at_y(rect.bottom() - bar_h);
            (a, b)
        } else {
            (
                rect,
                egui::Rect::from_min_size(rect.left_bottom(), egui::Vec2::ZERO),
            )
        };
        let mut a = child(ui, table_r, "a");
        // 表は自分の矩形の外（下の帯）に描かない
        a.set_clip_rect(table_r.intersect(a.clip_rect()));
        (a, child(ui, bar_r, "b"))
    };
    table(&mut top, st, c, &v, npar, &mut out);
    if out.cancel {
        st.edit = None;
        st.active = true;
    }
    if !out.fx.acts.is_empty() {
        st.active = true;
    }
    out.acts.append(&mut out.fx.acts);
    if out.fx.detail {
        st.detail = true;
    }
    if out.clicked_any {
        st.active = true;
    } else if ui.input(|i| i.pointer.any_click()) && !ui.ui_contains_pointer() {
        st.active = false;
    }
    // キー（表をクリックした後。書き換え中は入力欄が使う）
    if st.active
        && st.edit.is_none()
        && st.kbd_menu.is_none()
        && ui.memory(|m| m.focused().is_none())
    {
        key_input(ui, st, c, &v, &mut out);
    }
    kbd_menu(ui, st, c, &v, &mut out);
    surfaces(st, c, &v, &mut out);
    apply(ui, st, c, &v, &mut out);
    let mut acts = out.acts;
    let bottom = detail(bottom, st, c, &v, &mut acts);
    (acts, bottom)
}

/// 表（見出しと行）。横に流す ScrollArea の中
fn table(top: &mut Ui, st: &mut LensData, c: &Ctx, v: &View, npar: u8, out: &mut Out) {
    let cols = &v.cols;
    let cur_typ = v.cur_typ.as_deref();
    // 幅が足りなければ横に流す（見出しと横のスクロールバーの分を引く）
    let gap = {
        let s = top.spacing().scroll;
        s.bar_width + s.bar_inner_margin + s.bar_outer_margin + 2.0
    };
    let table_h = (top.available_height() - 22.0 - gap).max(20.0);
    egui::ScrollArea::horizontal()
        .id_salt("lens_data_h")
        .show(top, |ui| {
            // 文字を選べるラベルはクリックを取ってしまう（セルの空き・行のクリックが行に届かない）
            ui.style_mut().interaction.selectable_labels = false;
            // 面の列の幅は見出しが入る幅から（英語の「Surface」が切れない）
            let surf_w =
                ui.fonts_mut(|f| {
                    f.layout_no_wrap(
                        tr("Surface").into(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        egui::Color32::WHITE,
                    )
                })
                .size()
                .x + 2.0 * ui.spacing().item_spacing.x
                    + 4.0;
            let mut table = TableBuilder::new(ui)
                .id_salt(("lens_data", npar, crate::i18n::is_ja()))
                .max_scroll_height(table_h)
                // 低いパネルでも表は自分の矩形に収める（既定の 200 だと矩形の下へはみ出し、最後の行が下の帯に隠れる）
                .min_scrolled_height(0.0)
                .auto_shrink([false, true])
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .column(Column::initial(surf_w.max(40.0)).at_least(30.0))
                .column(Column::initial(70.0).at_least(30.0));
            for (n, &col) in cols.iter().enumerate() {
                table = table.column(if n + 1 == cols.len() {
                    Column::remainder().at_least(50.0)
                } else {
                    match col {
                        Col::Type => Column::initial(80.0).at_least(40.0),
                        Col::Par(_) => Column::initial(70.0).at_least(40.0),
                        c if c.numeric() => Column::initial(80.0).at_least(40.0),
                        _ => Column::initial(90.0).at_least(40.0),
                    }
                });
            }
            let cur = st.cur.map(|x| x.0);
            if cur != st.scrolled {
                if let Some(pos) = cur.and_then(|j| v.keys.iter().position(|&x| x == j)) {
                    table = table.scroll_to_row(pos, None);
                }
                st.scrolled = cur;
            }
            table
                .header(22.0, |mut h| {
                    // 見出しは折り返さない（2 行に折れて行に重ならない）
                    h.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(tr("Surface")).strong()).truncate());
                    });
                    h.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(tr("Blocks")).strong()).truncate());
                    });
                    for &col in cols {
                        h.col(|ui| {
                            let t = col.header(cur_typ);
                            let r = ui.add(egui::Label::new(RichText::new(&t).strong()).truncate());
                            if let Col::Par(i) = col {
                                r.on_hover_text(trf("Par {}: {}", &[&i, &t]));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_H, v.keys.len(), |mut row| {
                        let j = v.keys[row.index()];
                        let blk = v.child_of(j).filter(|&i| Some(i) != v.scope);
                        let in_sel =
                            blk.is_some_and(|i| Some(&c.blocks[i].name) == v.sel_block.as_ref());
                        let in_hl = st.hl.iter().any(|r| r[0] <= j && j <= r[1]);
                        row.set_selected(in_sel || in_hl || v.selected.contains(&j));
                        row.col(|ui| num_cell(ui, st, c, v, out, j));
                        row.col(|ui| block_cell(ui, st, c, v, out, j));
                        for &col in &v.cols {
                            row.col(|ui| cell(ui, st, c, v, out, j, col));
                        }
                        row_end(&row.response(), st, c, v, out, j);
                    });
                });
            if let Some(r) = out.hscroll_to {
                // 縦は表が scroll_to_row で合わせるので、横だけ（縦の範囲は今の見える所）
                let vis = ui.clip_rect();
                ui.scroll_to_rect(
                    egui::Rect::from_x_y_ranges(r.x_range(), vis.y_range()),
                    None,
                );
            }
            // 横のスクロールバーが最後の行に重ならない（上の table_h でこの分を引いてある）
            crate::widgets::scroll_gap(ui);
        });
}

/// 表の下: 詳細設定の帯と、選んだ面の設定（たたんだブロックの行・ブロックだけを選んだときは、ブロックの設定をパネルの側で）
fn detail(mut bottom: Ui, st: &mut LensData, c: &Ctx, v: &View, acts: &mut Vec<Act>) -> Ui {
    st.block_view = None;
    // 詳細設定の帯: 閉じているときは下に折りたたんだ帯、開いているときは欄の上の見出し（どちらもクリックで開閉）
    let what = match st.cur.map(|x| x.0).filter(|&j| j < c.rows.len()) {
        Some(j) => match v.folded(j) {
            Some((i, _)) => trf("block {}", &[&c.blocks[i].name]),
            None => trf("Surface {}", &[&j]),
        },
        None => st
            .sel
            .clone()
            .map_or(String::new(), |b| trf("block {}", &[&b])),
    };
    if bottom.max_rect().height() > 1.0 || st.detail {
        let label = format!(
            "{} {}{}",
            if st.detail { "▼" } else { "▶" },
            tr("Details"),
            if what.is_empty() {
                String::new()
            } else {
                format!(" — {what}")
            }
        );
        // 表の下の 1 行: 詳細設定の開閉と、表の使い方の案内（長ければ切る。ポインタを当てると全部）
        bottom.horizontal(|ui| {
            let r = ui
                .add(
                    egui::Label::new(RichText::new(label).small().color(v.weak))
                        .sense(Sense::click()),
                )
                .on_hover_text(format!(
                    "{}\n\n{}",
                    tr("Aperture, tilt/decenter before and after, shape data, all-fields JSON and block settings (click to open/close)"),
                    tr("Solves/pickups: […] at the right of a cell; dn, dispersion, birefringence: the material's […]; stop, ignore, insert, paste: right-click (Shift+F10). Keys: F1"),
                ));
            if r.clicked() {
                st.detail = !st.detail;
            }
        });
    }
    if !st.detail {
        return bottom;
    }
    match st.cur.map(|x| x.0).filter(|&j| j < c.rows.len()) {
        Some(j) if v.folded(j).is_none() => {
            if st.insp.data.as_ref().is_none_or(|d| d.0 != j) && st.insp.asked != Some(j) {
                st.insp.asked = Some(j);
                acts.push(Act::NeedData(j));
            }
            let ro = locked_at(c.blocks, c.locked, j)
                .map(|n| trf("{} is locked", &[&n]))
                .or_else(|| (c.read_only)(j, Col::R));
            egui::ScrollArea::vertical()
                .id_salt("surf_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    acts.extend(crate::surf_edit::ui(ui, &mut st.insp, j, ro));
                });
        }
        Some(j) => {
            st.block_view = v.folded(j).map(|(i, _)| c.blocks[i].name.clone());
        }
        None => match st.sel.clone() {
            Some(b) => st.block_view = Some(b),
            None => {
                bottom.weak(tr(
                    "Select a surface or block to change its settings here (double-click or Enter goes into a block; the mouse back button or Backspace goes out).",
                ));
            }
        },
    }
    bottom
}
