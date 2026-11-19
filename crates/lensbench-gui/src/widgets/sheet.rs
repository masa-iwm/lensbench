//! 表の部品（最適化の変数・評価関数の項、公差、差し替え探索、ズームなどで共通）。レンズデータの表と同じ操作にする:
//! クリックでセルを選び、矢印・Tab・Home・End で移る。打った文字・Enter・F2・ダブルクリックで書き始め、Enter で確定、
//! Tab・↑↓ で確定して隣へ、Esc でやめる。Delete で空に、Ctrl+C・Ctrl+V はタブ区切り（表計算と行き来できる）、
//! Insert で前に行を足し、Ctrl+Shift+↑↓ で選んだ行を上下へ動かし、Ctrl+D で選んだ行の写しを下に挿入し、Ctrl+Delete で行を消す。右クリックの欄でも同じ操作ができる。
//! 数のセルは Alt+ドラッグか、セルの右端のつまみのドラッグで値を増減する（Shift で細かく）。Alt+↑↓ は 1 ずつ
//! （Shift で 10、Ctrl で 0.1）。書き換え中の ←→ は文字のカーソル、↑↓ は確定して上下のセルへ。
//!
//! 表の中身は [`Sheet`] で渡す（セルの文字と種類を返し、書き換えを受ける）。選択・書き換え中の文字は [`SheetState`]

use std::hash::Hash;
use std::ops::RangeInclusive;

use egui::{Key, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::i18n::{tr, trf};

/// セルの値の種類（書き方・描き方・ドラッグ）
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Text,
    /// 数（右寄せ、ドラッグで増減。int は整数で 0 以上）
    Num {
        int: bool,
    },
    /// 候補（名前, 説明）から選ぶ。free なら候補に無い文字も書ける
    List {
        items: Vec<(String, String)>,
        free: bool,
    },
    /// true・false（チェックボックス、Space で切り替え）
    Bool,
}

/// 1 つのセル
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cell {
    pub text: String,
    pub kind: Kind,
    /// 変えられない理由（Some なら書き換えない）
    pub ro: Option<String>,
    /// 空のときに薄く出す文字（既定の値など）
    pub hint: String,
    /// ポインタを当てたときの説明
    pub tip: String,
    /// 出す文字が値と違うとき（候補の訳した名前など）
    pub shown: Option<String>,
}

impl Cell {
    fn of(text: impl Into<String>, kind: Kind) -> Cell {
        Cell {
            text: text.into(),
            kind,
            ro: None,
            hint: String::new(),
            tip: String::new(),
            shown: None,
        }
    }
    pub(crate) fn text(t: impl Into<String>) -> Cell {
        Cell::of(t, Kind::Text)
    }
    pub(crate) fn num(t: impl Into<String>) -> Cell {
        Cell::of(t, Kind::Num { int: false })
    }
    pub(crate) fn int(t: impl Into<String>) -> Cell {
        Cell::of(t, Kind::Num { int: true })
    }
    pub(crate) fn list(t: impl Into<String>, items: Vec<(String, String)>, free: bool) -> Cell {
        Cell::of(t, Kind::List { items, free })
    }
    pub(crate) fn boolean(b: bool) -> Cell {
        Cell::of(if b { "true" } else { "false" }, Kind::Bool)
    }
    /// 何も無いセル（その行に無い列）
    pub(crate) fn empty() -> Cell {
        Cell {
            ro: Some(String::new()),
            ..Cell::text("")
        }
    }
    pub(crate) fn hint(mut self, h: impl Into<String>) -> Cell {
        self.hint = h.into();
        self
    }
    pub(crate) fn tip(mut self, t: impl Into<String>) -> Cell {
        self.tip = t.into();
        self
    }
    pub(crate) fn shown(mut self, t: impl Into<String>) -> Cell {
        self.shown = Some(t.into());
        self
    }
    pub(crate) fn ro(mut self, why: Option<String>) -> Cell {
        self.ro = why;
        self
    }
    fn numeric(&self) -> bool {
        matches!(self.kind, Kind::Num { .. })
    }
}

/// 表の中身
pub(crate) trait Sheet {
    fn rows(&self) -> usize;
    fn cols(&self) -> usize;
    /// 列の見出し（`cur` は今の行。行の種類で見出しが変わる表がある）
    fn header(&self, col: usize, cur: Option<usize>) -> String;
    fn header_tip(&self, _col: usize, _cur: Option<usize>) -> String {
        String::new()
    }
    /// 列の初めの幅
    fn width(&self, _col: usize) -> f32 {
        70.0
    }
    fn cell(&self, row: usize, col: usize) -> Cell;
    /// セルを書き換える（決められなければ理由）
    fn set(&mut self, row: usize, col: usize, text: &str) -> Result<(), String>;
    /// 空の行を at（0..=rows）に足す。足せない表は false
    fn insert(&mut self, _at: usize) -> bool {
        false
    }
    fn remove(&mut self, _rows: RangeInclusive<usize>) {}
    /// 行 a と a+1 を入れ替える（行の順を変えられない表は false）
    fn swap(&mut self, _a: usize) -> bool {
        false
    }
}

/// 書き換え中のセル
#[derive(Debug, Clone, Default)]
struct Edit {
    row: usize,
    col: usize,
    text: String,
    started: bool,
    select_all: bool,
    error: Option<String>,
    /// 候補の一覧で選んでいる行・打って絞ったか・↑↓ で選んだか
    pick: usize,
    filtered: bool,
    moved: bool,
    list_hover: bool,
    /// 候補の一覧を一度出した（開いた直後だけ今の値を選んで見せる）
    listed: bool,
}

/// ドラッグで数を変えている途中
#[derive(Debug, Clone, Copy)]
struct Drag {
    row: usize,
    col: usize,
    start: f64,
    dx: f32,
}

/// 表の選択と書き換え中の文字（パネルが持つ）
#[derive(Debug, Clone, Default)]
pub(crate) struct SheetState {
    /// 今のセル（行, 列）
    cur: Option<(usize, usize)>,
    /// Shift で広げた行の範囲の端
    anchor: Option<usize>,
    edit: Option<Edit>,
    /// 表をクリックした後（キーを受ける）
    active: bool,
    drag: Option<Drag>,
    /// 表の下に出す一言（変えられない理由・貼り付けの誤り）
    note: Option<String>,
}

impl SheetState {
    /// 選んでいる行の範囲
    fn rows_sel(&self) -> Option<RangeInclusive<usize>> {
        let r = self.cur?.0;
        let a = self.anchor.unwrap_or(r);
        Some(a.min(r)..=a.max(r))
    }
    #[cfg(test)]
    pub(crate) fn cur(&self) -> Option<(usize, usize)> {
        self.cur
    }
    #[cfg(test)]
    pub(crate) fn editing(&self) -> bool {
        self.edit.is_some()
    }
}

/// このフレームで決めたこと（描き終えてから当てる）
#[derive(Default)]
struct Out {
    clicked: bool,
    start: Option<(usize, usize, Option<String>)>,
    commit: Option<(usize, usize, String)>,
    after: Option<(i32, i32)>,
    cancel: bool,
    /// そのまま書く値（チェックボックス・ドラッグ・空にする）
    set: Vec<(usize, usize, String)>,
    insert: Option<usize>,
    /// 行の範囲を上（-1）・下（+1）へ 1 行動かす
    shift: Option<(RangeInclusive<usize>, i32)>,
    /// 写しを下に挿入する行の範囲
    duplicate: Option<RangeInclusive<usize>>,
    remove: Option<RangeInclusive<usize>>,
    copy: bool,
}

const ROW_H: f32 = 20.0;
/// 数のセルの右端のつまみの幅
const GRIP_W: f32 = 6.0;

/// 表を描く。何か書き換えたら true
pub(crate) fn show(ui: &mut Ui, id: impl Hash, st: &mut SheetState, sh: &mut dyn Sheet) -> bool {
    let id = egui::Id::new(id);
    let (n, m) = (sh.rows(), sh.cols());
    // 行が減ったら選択を中へ
    if n == 0 || m == 0 {
        st.cur = None;
        st.anchor = None;
        st.edit = None;
    } else if let Some((r, c)) = st.cur {
        st.cur = Some((r.min(n - 1), c.min(m - 1)));
        st.anchor = st.anchor.map(|a| a.min(n - 1));
    }
    if st.edit.as_ref().is_some_and(|e| e.row >= n || e.col >= m) {
        st.edit = None;
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        st.drag = None;
    }
    let mut out = Out::default();
    // フォーカスは描く前に見る（Tab を押すと egui が描いている間に表の中のボタンへフォーカスを移す）
    let free = ui.memory(|mm| mm.focused().is_none());
    let inner = ui.scope(|ui| table(ui, id, st, sh, &mut out));
    let rect = inner.response.rect;
    if out.clicked {
        st.active = true;
        st.note = None;
    } else if ui.input(|i| i.pointer.any_click())
        && !ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| rect.contains(p))
    {
        st.active = false;
    }
    if st.active && st.edit.is_none() && free && keys(ui, st, sh, &mut out) {
        // Tab はセルを移すのに使った（ボタンへ移ったフォーカスを外す）
        if let Some(f) = ui.memory(|mm| mm.focused()) {
            ui.memory_mut(|mm| mm.surrender_focus(f));
        }
    }
    let changed = apply(ui, st, sh, out);
    if let Some(t) = &st.note {
        ui.label(
            RichText::new(format!("⚠ {t}"))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
    changed
}

fn table(ui: &mut Ui, id: egui::Id, st: &mut SheetState, sh: &dyn Sheet, out: &mut Out) {
    let (n, m) = (sh.rows(), sh.cols());
    let cur_row = st.cur.map(|x| x.0);
    let sel = st.rows_sel();
    let weak = crate::theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let shift = ui.input(|i| i.modifiers.shift);
    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            let mut tb = TableBuilder::new(ui)
                .id_salt(id.with("t"))
                .vscroll(false)
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(26.0));
            for c in 0..m {
                tb = tb.column(Column::initial(sh.width(c)).at_least(24.0).clip(true));
            }
            tb = tb.column(Column::exact(20.0));
            tb.header(ROW_H, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new("#").small().color(weak));
                });
                for c in 0..m {
                    h.col(|ui| {
                        let t = sh.header(c, cur_row);
                        let r = ui.add(
                            egui::Label::new(RichText::new(t).small().strong().color(weak))
                                .truncate(),
                        );
                        let tip = sh.header_tip(c, cur_row);
                        if !tip.is_empty() {
                            r.on_hover_text(tip);
                        }
                    });
                }
                h.col(|_| {});
            })
            .body(|body| {
                body.rows(ROW_H, n, |mut row| {
                    let r = row.index();
                    row.set_selected(
                        sel.as_ref()
                            .is_some_and(|s| s.start() != s.end() && s.contains(&r)),
                    );
                    row.col(|ui| {
                        let resp =
                            ui.interact(ui.max_rect(), ui.id().with("sheet_num"), Sense::click());
                        ui.label(RichText::new(format!("{}", r + 1)).monospace().color(weak));
                        if resp.clicked() || resp.secondary_clicked() {
                            out.clicked = true;
                            click_to(st, r, st.cur.map_or(0, |x| x.1), shift, false);
                        }
                        resp.context_menu(|ui| row_menu(ui, st, sh, out, r));
                    });
                    for c in 0..m {
                        row.col(|ui| cell_ui(ui, st, sh, out, r, c, weak, accent, shift));
                    }
                    row.col(|ui| {
                        if ui.small_button("✖").on_hover_text(tr("Delete")).clicked() {
                            out.clicked = true;
                            out.remove = Some(r..=r);
                        }
                    });
                });
            });
        });
}

/// 今のセルを (r, c) に。Shift なら行の範囲を広げる。keep は範囲を残す（範囲の中の右クリック）
fn click_to(st: &mut SheetState, r: usize, c: usize, shift: bool, keep: bool) {
    if keep {
        return;
    }
    if shift {
        if st.anchor.is_none() {
            st.anchor = st.cur.map(|x| x.0);
        }
    } else {
        st.anchor = None;
    }
    st.cur = Some((r, c));
}

#[allow(clippy::too_many_arguments)]
fn cell_ui(
    ui: &mut Ui,
    st: &mut SheetState,
    sh: &dyn Sheet,
    out: &mut Out,
    r: usize,
    c: usize,
    weak: egui::Color32,
    accent: egui::Color32,
    shift: bool,
) {
    let cell = sh.cell(r, c);
    if st.edit.as_ref().is_some_and(|e| e.row == r && e.col == c) {
        edit_ui(ui, st, &cell, out);
        return;
    }
    let rect = ui.max_rect();
    let is_cur = st.cur == Some((r, c));
    let drag_ok = cell.numeric() && cell.ro.is_none();
    let resp = ui.interact(
        rect,
        ui.id().with("sheet_cell"),
        if drag_ok {
            Sense::click_and_drag()
        } else {
            Sense::click()
        },
    );
    // 名前は外で作る（widget_info の関数は Context を握ったまま呼ばれる）
    let name = trf("Row {} {}", &[&(r + 1), &sh.header(c, Some(r))]);
    let (enabled, value) = (cell.ro.is_none(), cell.text.clone());
    resp.widget_info(move || {
        let mut w = egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name.clone());
        w.current_text_value = Some(value.clone());
        w
    });
    let hover = resp.hovered();
    let alt = ui.input(|i| i.modifiers.alt);
    let grip = egui::Rect::from_min_max(egui::pos2(rect.right() - GRIP_W, rect.top()), rect.max);
    let on_grip = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| grip.contains(p));
    match &cell.kind {
        Kind::Bool if cell.ro.is_none() => {
            let mut b = cell.text == "true";
            if ui.checkbox(&mut b, "").changed() {
                out.clicked = true;
                click_to(st, r, c, false, false);
                out.set.push((r, c, b.to_string()));
            }
        }
        Kind::Bool => {
            ui.label(RichText::new(if cell.text == "true" { "✔" } else { "" }).color(weak));
        }
        _ => {
            let (t, color) = if cell.text.is_empty() {
                (cell.hint.as_str(), weak)
            } else {
                (
                    cell.shown.as_deref().unwrap_or(&cell.text),
                    if cell.ro.is_some() {
                        weak
                    } else {
                        ui.visuals().text_color()
                    },
                )
            };
            let mut rt = RichText::new(t).color(color);
            if cell.numeric() {
                rt = rt.monospace();
            }
            let layout = if cell.numeric() {
                egui::Layout::right_to_left(egui::Align::Center)
            } else {
                egui::Layout::left_to_right(egui::Align::Center)
            };
            ui.with_layout(layout, |ui| {
                if cell.numeric() {
                    ui.add_space(GRIP_W - 2.0);
                }
                ui.add(egui::Label::new(rt).truncate());
            });
        }
    }
    // 数のつまみ（今のセル・ポインタの下）と、Alt・つまみの上は左右の矢印のポインタ
    if drag_ok && (is_cur || hover) {
        let x = grip.center().x;
        for dx in [-1.5, 1.5] {
            ui.painter().vline(
                x + dx,
                grip.shrink2(egui::vec2(0.0, 5.0)).y_range(),
                egui::Stroke::new(1.0_f32, weak),
            );
        }
    }
    if drag_ok && (hover && (alt || on_grip) || st.drag.is_some_and(|d| d.row == r && d.col == c)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if is_cur {
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.5_f32, accent),
            egui::StrokeKind::Inside,
        );
    }
    if drag_ok && resp.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin());
        let from_grip = press.is_some_and(|p| grip.contains(p));
        if alt || from_grip {
            let start = cell.text.trim().parse::<f64>().unwrap_or(0.0);
            st.drag = Some(Drag {
                row: r,
                col: c,
                start,
                dx: 0.0,
            });
            out.clicked = true;
            click_to(st, r, c, false, false);
        }
    }
    if let Some(d) = st.drag.as_mut().filter(|d| d.row == r && d.col == c) {
        if resp.dragged() {
            let dx = resp.drag_delta().x * if shift { 0.1 } else { 1.0 };
            if dx != 0.0 {
                d.dx += dx;
                let int = matches!(cell.kind, Kind::Num { int: true });
                let v = drag_value(d.start, d.dx, int);
                out.set.push((r, c, crate::json_view::fmt_num(v)));
            }
        }
    }
    let resp = if let Some(why) = cell.ro.as_ref().filter(|w| !w.is_empty()) {
        resp.on_hover_text(why)
    } else if !cell.tip.is_empty() {
        resp.on_hover_text(&cell.tip)
    } else {
        resp
    };
    if resp.clicked() || resp.double_clicked() || resp.secondary_clicked() {
        out.clicked = true;
        let keep = resp.secondary_clicked()
            && st
                .rows_sel()
                .is_some_and(|s| s.start() != s.end() && s.contains(&r));
        click_to(st, r, c, shift && !resp.secondary_clicked(), keep);
    }
    if resp.double_clicked() {
        out.start = Some((r, c, None));
    }
    resp.context_menu(|ui| {
        if cell.ro.is_none() {
            if ui.button(tr("Edit")).clicked() {
                out.start = Some((r, c, None));
                ui.close_menu();
            }
            if ui.button(tr("Clear")).clicked() {
                out.set.push((r, c, String::new()));
                ui.close_menu();
            }
            ui.separator();
        }
        row_menu(ui, st, sh, out, r);
    });
}

/// 行の右クリックの欄（行の番号・セルで同じ）
fn row_menu(ui: &mut Ui, st: &SheetState, _sh: &dyn Sheet, out: &mut Out, r: usize) {
    out.clicked = true;
    let sel = st.rows_sel().filter(|s| s.contains(&r)).unwrap_or(r..=r);
    if ui.button(tr("Copy")).on_hover_text("Ctrl+C").clicked() {
        out.copy = true;
        ui.close_menu();
    }
    if ui
        .button(tr("Insert row above"))
        .on_hover_text("Insert")
        .clicked()
    {
        out.insert = Some(r);
        ui.close_menu();
    }
    if ui.button(tr("Insert row below")).clicked() {
        out.insert = Some(r + 1);
        ui.close_menu();
    }
    if ui
        .add_enabled(*sel.start() > 0, egui::Button::new(tr("Move up")))
        .on_hover_text("Ctrl+Shift+↑")
        .clicked()
    {
        out.shift = Some((sel.clone(), -1));
        ui.close_menu();
    }
    if ui
        .button(tr("Move down"))
        .on_hover_text("Ctrl+Shift+↓")
        .clicked()
    {
        out.shift = Some((sel.clone(), 1));
        ui.close_menu();
    }
    if ui
        .button(tr("Duplicate row"))
        .on_hover_text("Ctrl+D")
        .clicked()
    {
        out.duplicate = Some(sel.clone());
        ui.close_menu();
    }
    let count = sel.end() - sel.start() + 1;
    let label = if count > 1 {
        trf("Delete {} rows", &[&count])
    } else {
        tr("Delete row").to_string()
    };
    if ui.button(label).on_hover_text("Ctrl+Delete").clicked() {
        out.remove = Some(sel);
        ui.close_menu();
    }
}

/// 書き換え中のセル（入力欄。候補のある欄は下に一覧）
fn edit_ui(ui: &mut Ui, st: &mut SheetState, cell: &Cell, out: &mut Out) {
    let Some(e) = st.edit.as_mut() else { return };
    let (r, c) = (e.row, e.col);
    let (resp, keys) = super::cell::input(
        ui,
        &mut e.text,
        &mut e.started,
        e.select_all,
        cell.numeric(),
        e.error.as_deref(),
    );
    if resp.changed() {
        e.error = None;
    }
    let Kind::List { items, free } = &cell.kind else {
        if keys.esc {
            out.cancel = true;
        } else if let Some(mv) = keys.nav(true) {
            out.commit = Some((r, c, e.text.clone()));
            out.after = Some(mv);
        } else if keys.enter || resp.lost_focus() {
            out.commit = Some((r, c, e.text.clone()));
        }
        if out.commit.is_some() && out.after.is_some() {
            resp.surrender_focus();
        }
        return;
    };
    if resp.changed() {
        e.filtered = true;
        e.moved = false;
        e.pick = 0;
    }
    let shown: Vec<(String, String)> = if e.filtered {
        filter_items(items, &e.text)
    } else {
        items.clone()
    };
    let mut scroll = None;
    if !e.filtered && !e.moved && !e.listed {
        e.pick = shown
            .iter()
            .position(|x| x.0.eq_ignore_ascii_case(e.text.trim()))
            .unwrap_or(0);
        scroll = Some(e.pick);
    }
    if e.pick >= shown.len() {
        e.pick = 0;
    }
    if keys.up && e.pick > 0 {
        e.pick -= 1;
        e.moved = true;
        scroll = Some(e.pick);
    }
    if keys.down && e.pick + 1 < shown.len() {
        e.pick += 1;
        e.moved = true;
        scroll = Some(e.pick);
    }
    let res = super::choices::Choices {
        scroll_to: scroll,
        min_width: 160.0,
        ..super::choices::Choices::new(&shown, e.pick)
    }
    .show(ui, resp.id.with("choices"), resp.rect);
    let picked = shown.get(e.pick).map(|x| x.0.clone());
    // 決める文字: ↑↓ で選んだか、打って絞った候補がある（候補に無い文字を書ける欄では打った文字が候補と同じとき）か、打った文字
    let choose = |e: &Edit| -> String {
        let exact = shown
            .iter()
            .find(|x| x.0.eq_ignore_ascii_case(e.text.trim()))
            .map(|x| x.0.clone());
        if e.moved {
            picked.clone().unwrap_or_else(|| e.text.clone())
        } else if let Some(x) = exact {
            x
        } else if e.filtered && !*free {
            picked.clone().unwrap_or_else(|| e.text.clone())
        } else {
            e.text.clone()
        }
    };
    if keys.esc {
        out.cancel = true;
    } else if let Some(k) = res.clicked {
        out.commit = Some((r, c, shown[k].0.clone()));
    } else if keys.tab {
        out.commit = Some((r, c, choose(e)));
        out.after = keys.nav(false);
        resp.surrender_focus();
    } else if keys.enter || (resp.lost_focus() && !res.hovered && !e.list_hover) {
        out.commit = Some((r, c, choose(e)));
    }
    e.list_hover = res.hovered;
    e.listed = true;
}

/// 打った文字で絞った候補（前方一致を先に）
fn filter_items(items: &[(String, String)], q: &str) -> Vec<(String, String)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    let mut head = Vec::new();
    let mut rest = Vec::new();
    for it in items {
        match super::match_rank(&it.0.to_lowercase(), &it.1.to_lowercase(), &q) {
            Some(true) => head.push(it.clone()),
            Some(false) => rest.push(it.clone()),
            None => {}
        }
    }
    head.extend(rest);
    head
}

/// ドラッグした距離 dx [点] での値。1 点で今の値の桁の 1/100（0 は 0.01）、整数は 10 点で 1
pub(crate) fn drag_value(start: f64, dx: f32, int: bool) -> f64 {
    if int {
        return (start + (dx as f64) * 0.1).round().max(0.0);
    }
    let a = start.abs();
    let step = if a > 0.0 {
        10f64.powf(a.log10().floor() - 2.0)
    } else {
        0.01
    };
    let v = start + dx as f64 * step;
    // 刻みの桁で丸める（浮動小数の端数を残さない）
    let d = (-step.log10().round()).max(0.0) as usize;
    format!("{v:.d$}").parse().unwrap_or(v)
}

/// Alt+↑↓ の値（d を足し、浮動小数の端数を丸める。整数は 0 以上）
pub(crate) fn nudge(x: f64, d: f64, int: bool) -> f64 {
    if int {
        return (x + d).round().max(0.0);
    }
    ((x + d) * 1e9).round() / 1e9
}

/// キー（表をクリックした後、書き換え中でないとき）。Tab を使ったら true
fn keys(ui: &mut Ui, st: &mut SheetState, sh: &dyn Sheet, out: &mut Out) -> bool {
    let tab = keys_inner(ui, st, sh, out);
    tab && st.cur.is_some()
}

fn keys_inner(ui: &mut Ui, st: &mut SheetState, sh: &dyn Sheet, out: &mut Out) -> bool {
    let (n, m) = (sh.rows(), sh.cols());
    let ev = ui.input(|i| {
        let mods = i.modifiers;
        let typed = if mods.command || mods.ctrl || mods.alt {
            None
        } else {
            i.events.iter().find_map(|e| match e {
                egui::Event::Text(t) if !t.is_empty() && t != " " => Some(t.clone()),
                _ => None,
            })
        };
        let copy = i
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut));
        let paste = i.events.iter().find_map(|e| match e {
            egui::Event::Paste(t) => Some(t.clone()),
            _ => None,
        });
        let insert = i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { key: Key::Insert, pressed: true, modifiers, .. } if !modifiers.shift)
        });
        Keys {
            up: i.key_pressed(Key::ArrowUp),
            down: i.key_pressed(Key::ArrowDown),
            left: i.key_pressed(Key::ArrowLeft),
            right: i.key_pressed(Key::ArrowRight),
            tab: i.key_pressed(Key::Tab),
            home: i.key_pressed(Key::Home),
            end: i.key_pressed(Key::End),
            enter: i.key_pressed(Key::Enter),
            f2: i.key_pressed(Key::F2),
            del: i.key_pressed(Key::Delete),
            space: i.key_pressed(Key::Space),
            dup: mods.command && i.key_pressed(Key::D),
            esc: i.key_pressed(Key::Escape),
            shift: mods.shift,
            command: mods.command,
            alt: mods.alt,
            ctrl: mods.ctrl,
            typed,
            copy,
            paste,
            insert,
        }
    });
    out.copy |= ev.copy;
    if ev.esc {
        st.anchor = None;
        st.active = false;
        return ev.tab;
    }
    if ev.insert {
        out.insert = Some(st.cur.map_or(n, |x| x.0));
        return ev.tab;
    }
    if n == 0 || m == 0 {
        return ev.tab;
    }
    let (r, c) = st.cur.unwrap_or((0, 0));
    if st.cur.is_none() {
        if ev.up || ev.down || ev.left || ev.right || ev.tab {
            st.cur = Some((0, 0));
        }
        return ev.tab;
    }
    // Ctrl+Shift+↑↓: 選んだ行を上下へ動かす
    if ev.command && ev.shift && (ev.up || ev.down) {
        out.shift = st.rows_sel().map(|s| (s, if ev.up { -1 } else { 1 }));
        return ev.tab;
    }
    if ev.dup {
        out.duplicate = st.rows_sel();
        return ev.tab;
    }
    if ev.del && ev.command {
        out.remove = st.rows_sel();
        return ev.tab;
    }
    // Alt+↑↓: 数を 1 ずつ増減（Shift で 10、Ctrl で 0.1。多重構成の表と同じ）
    if ev.alt && (ev.up || ev.down) {
        let cell = sh.cell(r, c);
        if let (Kind::Num { int }, None) = (&cell.kind, &cell.ro) {
            let d = if ev.shift {
                10.0
            } else if ev.ctrl && !*int {
                0.1
            } else {
                1.0
            };
            let x = cell.text.trim().parse::<f64>().unwrap_or(0.0);
            let v = nudge(x, if ev.up { d } else { -d }, *int);
            out.set.push((r, c, crate::json_view::fmt_num(v)));
        }
        return ev.tab;
    }
    let mut to = None;
    if ev.up && r > 0 {
        to = Some((r - 1, c));
    } else if ev.down && r + 1 < n {
        to = Some((r + 1, c));
    } else if ev.left && c > 0 {
        to = Some((r, c - 1));
    } else if ev.right && c + 1 < m {
        to = Some((r, c + 1));
    } else if ev.tab {
        to = Some(step(r, c, if ev.shift { -1 } else { 1 }, n, m));
    } else if ev.home {
        to = Some((if ev.command { 0 } else { r }, 0));
    } else if ev.end {
        to = Some((if ev.command { n - 1 } else { r }, m - 1));
    }
    if let Some(t) = to {
        if ev.shift && (ev.up || ev.down) {
            if st.anchor.is_none() {
                st.anchor = Some(r);
            }
        } else {
            st.anchor = None;
        }
        st.cur = Some(t);
        return ev.tab;
    }
    if let Some(t) = ev.paste {
        paste(st, sh, out, &t);
        return ev.tab;
    }
    let cell = sh.cell(r, c);
    if ev.del {
        let rows = st.rows_sel().unwrap_or(r..=r);
        if rows.start() != rows.end() {
            for rr in rows {
                for cc in 0..m {
                    if sh.cell(rr, cc).ro.is_none() {
                        out.set.push((rr, cc, String::new()));
                    }
                }
            }
        } else if cell.ro.is_none() {
            out.set.push((r, c, String::new()));
        }
        return ev.tab;
    }
    if ev.space && cell.kind == Kind::Bool && cell.ro.is_none() {
        out.set.push((r, c, (cell.text != "true").to_string()));
        return ev.tab;
    }
    if ev.enter || ev.f2 || ev.typed.is_some() || ev.space {
        out.start = Some((r, c, ev.typed));
    }
    ev.tab
}

struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    tab: bool,
    home: bool,
    end: bool,
    enter: bool,
    f2: bool,
    del: bool,
    space: bool,
    dup: bool,
    esc: bool,
    shift: bool,
    command: bool,
    alt: bool,
    ctrl: bool,
    typed: Option<String>,
    copy: bool,
    paste: Option<String>,
    insert: bool,
}

/// Tab の行き先（行の端で次の行・前の行へ）
fn step(r: usize, c: usize, d: i32, n: usize, m: usize) -> (usize, usize) {
    let k = (r * m + c) as i64 + d as i64;
    let k = k.clamp(0, (n * m - 1) as i64) as usize;
    (k / m, k % m)
}

/// 表の文字（タブ区切り）。行の範囲を選んでいれば全部の列、でなければ今のセル
fn copy_text(st: &SheetState, sh: &dyn Sheet) -> Option<String> {
    let (r, c) = st.cur?;
    let rows = st.rows_sel().unwrap_or(r..=r);
    if rows.start() == rows.end() && st.anchor.is_none() {
        return Some(sh.cell(r, c).text);
    }
    Some(
        rows.map(|rr| {
            (0..sh.cols())
                .map(|cc| sh.cell(rr, cc).text)
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n"),
    )
}

/// 貼り付け: 今のセル（行の範囲を選んでいればその最初の行）から右下へ。足りない行は足す
fn paste(st: &mut SheetState, sh: &dyn Sheet, out: &mut Out, text: &str) {
    let Some((r, c)) = st.cur else { return };
    let r0 = st.rows_sel().map_or(r, |s| *s.start());
    let c0 = if st.anchor.is_some() { 0 } else { c };
    let lines: Vec<&str> = text.trim_end_matches(['\n', '\r']).lines().collect();
    let n = sh.rows();
    for (i, line) in lines.iter().enumerate() {
        let rr = r0 + i;
        for (j, f) in line.split('\t').enumerate() {
            let cc = c0 + j;
            if cc >= sh.cols() {
                break;
            }
            if rr >= n || sh.cell(rr, cc).ro.is_none() {
                out.set.push((rr, cc, f.trim().to_string()));
            }
        }
    }
}

/// rows の写しを rows の下に挿入する（空の行を足して、変えられるセルを順に書く。足せない表は false）
fn duplicate(sh: &mut dyn Sheet, rows: RangeInclusive<usize>) -> bool {
    if *rows.end() >= sh.rows() {
        return false;
    }
    let src: Vec<Vec<(String, bool)>> = rows
        .clone()
        .map(|r| {
            (0..sh.cols())
                .map(|c| {
                    let cell = sh.cell(r, c);
                    (cell.text, cell.ro.is_none())
                })
                .collect()
        })
        .collect();
    let at = rows.end() + 1;
    for (i, row) in src.iter().enumerate() {
        if !sh.insert(at + i) {
            return i > 0;
        }
        // 左の列から（種類の列を先に書くと、その種類の欄が後の列に出る）
        for (c, (t, editable)) in row.iter().enumerate() {
            if *editable && sh.cell(at + i, c).ro.is_none() {
                let _ = sh.set(at + i, c, t);
            }
        }
    }
    true
}

/// 決めたことを当てる
fn apply(ui: &Ui, st: &mut SheetState, sh: &mut dyn Sheet, out: Out) -> bool {
    let mut changed = false;
    if out.copy {
        if let Some(t) = copy_text(st, sh) {
            ui.ctx().copy_text(t);
        }
    }
    if out.cancel {
        st.edit = None;
        st.active = true;
    }
    if let Some((r, c, t)) = out.commit {
        match sh.set(r, c, &t) {
            Ok(()) => {
                changed = true;
                st.edit = None;
                st.active = true;
                st.note = None;
                if let Some((dr, dc)) = out.after {
                    let (n, m) = (sh.rows(), sh.cols());
                    st.cur = Some(if dr == 0 {
                        step(r, c, dc, n, m)
                    } else {
                        ((r as i64 + dr as i64).clamp(0, n as i64 - 1) as usize, c)
                    });
                    st.anchor = None;
                }
            }
            Err(e) => {
                if let Some(ed) = st.edit.as_mut() {
                    ed.error = Some(e);
                    // 欄にフォーカスを戻す（打った文字は残す）
                    ed.started = false;
                    ed.select_all = false;
                }
            }
        }
    }
    let mut errs = Vec::new();
    for (r, c, t) in out.set {
        while r >= sh.rows() {
            if !sh.insert(sh.rows()) {
                break;
            }
        }
        if r < sh.rows() {
            match sh.set(r, c, &t) {
                Ok(()) => changed = true,
                Err(e) if !e.is_empty() => errs.push(e),
                Err(_) => {}
            }
        }
    }
    if let Some(e) = errs.into_iter().next() {
        st.note = Some(e);
    }
    if let Some((r, c, typed)) = out.start {
        let cell = sh.cell(r, c);
        match cell.ro {
            Some(why) => {
                if !why.is_empty() {
                    st.note = Some(why);
                }
            }
            None => {
                st.cur = Some((r, c));
                st.anchor = None;
                let select_all = typed.is_none();
                st.edit = Some(Edit {
                    row: r,
                    col: c,
                    text: typed.unwrap_or(cell.text),
                    select_all,
                    filtered: !select_all,
                    ..Default::default()
                });
            }
        }
    }
    if let Some((rows, d)) = out.shift {
        let (lo, hi) = (*rows.start(), *rows.end());
        let ok = if d < 0 {
            lo > 0 && (lo..=hi).all(|i| sh.swap(i - 1))
        } else {
            hi + 1 < sh.rows() && (lo..=hi).rev().all(|i| sh.swap(i))
        };
        if ok {
            changed = true;
            let mv = |x: usize| (x as i64 + d as i64) as usize;
            st.cur = st.cur.map(|(r, c)| (mv(r), c));
            st.anchor = st.anchor.map(mv);
            st.edit = None;
            st.active = true;
        }
    }
    if let Some(rows) = out.duplicate {
        if duplicate(sh, rows.clone()) {
            changed = true;
            // 写しの最初の行へ（範囲は写しの全部）
            let n = rows.end() - rows.start() + 1;
            let c = st.cur.map_or(0, |x| x.1);
            st.anchor = (n > 1).then_some(rows.end() + 1);
            st.cur = Some((rows.end() + n, c));
            st.edit = None;
            st.active = true;
        }
    }
    if let Some(at) = out.insert {
        if sh.insert(at.min(sh.rows())) {
            changed = true;
            let c = st.cur.map_or(0, |x| x.1);
            st.cur = Some((at.min(sh.rows() - 1), c));
            st.anchor = None;
            st.active = true;
        }
    }
    if let Some(rows) = out.remove {
        if *rows.end() < sh.rows() {
            let lo = *rows.start();
            sh.remove(rows);
            changed = true;
            st.edit = None;
            st.anchor = None;
            st.cur = if sh.rows() == 0 {
                None
            } else {
                st.cur.map(|(_, c)| (lo.min(sh.rows() - 1), c))
            };
            st.active = true;
        }
    }
    changed
}

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
