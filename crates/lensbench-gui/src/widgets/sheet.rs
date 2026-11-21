//! 表の部品（最適化の変数・評価関数の項、公差、差し替え探索、ズームなどで共通）。レンズデータの表と同じ操作にする:
//! クリックでセルを選び、矢印・Tab・Home・End で移る。打った文字・Enter・F2・ダブルクリックで書き始め、Enter で確定、
//! Tab・↑↓ で確定して隣へ、Esc でやめる。Delete で空に、Ctrl+C・Ctrl+V はタブ区切り（表計算と行き来できる）、
//! Insert で前に行を足し、Ctrl+Shift+↑↓ で選んだ行を上下へ動かし、Ctrl+D で選んだ行の写しを下に挿入し、Ctrl+Delete で行を消す。右クリックの欄でも同じ操作ができる。
//! Shift+クリック・Shift+矢印で四角の範囲を選び、行の番号で行全体、列の見出しで列全体、左上の # か Ctrl+A で表全体を選ぶ
//! （コピー・貼り付け・Delete は範囲に効く。1 つの値を貼ると範囲を埋める）。候補のあるセルは打った文字に続く候補を補う
//! （補った部分は選んだ状態で、続けて打てば置き換わり、Backspace で消せば打った文字のまま）。
//! 数のセルは Alt+ドラッグか、セルの右端のつまみのドラッグで値を増減する（Shift で細かく）。Alt+↑↓ は 1 ずつ
//! （Shift で 10、Ctrl で 0.1）。書き換え中の ←→ は文字のカーソル、↑↓ は確定して上下のセルへ。
//! 行の番号・列の見出しをドラッグして行・列を動かす（列は動かせる表だけ）。最後の行の下の空の行の左端に値を書くか
//! 候補から選ぶと、その行を足す（OpticStudio のエディタと同じ。足せる表だけ）。
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
    /// 枠で目立たせる（多重構成の表のその構成で変えた値など）
    pub mark: bool,
    /// 文字を右のセルへはみ出して描く（コメントの行。右のセルは空にしておく）
    pub overflow: bool,
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
            mark: false,
            overflow: false,
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
    pub(crate) fn overflow(mut self) -> Cell {
        self.overflow = true;
        self
    }
    pub(crate) fn mark(mut self, m: bool) -> Cell {
        self.mark = m;
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
    /// 空の行を足せる表か（false なら右クリックの欄に行の挿入・複製を出さない）
    fn can_insert(&self) -> bool {
        true
    }
    /// 列の見出しの応答（クリック・右クリックの欄を表の側で受ける。表の側は操作を貯めて show の後に当てる）
    fn header_ui(&self, _resp: &egui::Response, _col: usize) {}
    /// セルの右クリックの欄の頭に表の側の項目を足す
    fn cell_menu(&self, _ui: &mut Ui, _row: usize, _col: usize) {}
    /// 列を見出しのドラッグで動かせるか
    fn can_move_col(&self, _col: usize) -> bool {
        false
    }
    /// 列 from を列 to の位置へ動かす（動かしたか）
    fn move_col(&mut self, _from: usize, _to: usize) -> bool {
        false
    }
    /// 最後の行の下の空の行の左端のセル（ここに書くと行を足す）。None なら空の行を出さない。
    /// full が false のときは描くだけ（候補は要らない）
    fn new_row(&self, _full: bool) -> Option<Cell> {
        None
    }
    /// 空の行の左端に text を書いて行を足す（既定は最後に空の行を足して左端の列に書く。書けなければ足した行を消す）
    fn add_row(&mut self, text: &str) -> Result<(), String> {
        let at = self.rows();
        if !self.insert(at) {
            return Err(String::new());
        }
        let r = self.set(at, 0, text);
        if r.is_err() {
            self.remove(at..=at);
        }
        r
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
    /// 候補で補う前に打った文字（候補の絞り込みに使う）
    query: Option<String>,
    /// 前のフレームの文字の数（増えたときだけ補う）
    prev_n: usize,
}

/// ドラッグで数を変えている途中
#[derive(Debug, Clone, Copy)]
struct Drag {
    row: usize,
    col: usize,
    start: f64,
    dx: f32,
}

/// 見出しのドラッグで動かしている行・列
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grab {
    /// 行 lo..=hi を、掴んだ行 at が行 to に来るように動かす
    Rows {
        lo: usize,
        hi: usize,
        at: usize,
        to: usize,
    },
    /// 列 at を列 to へ
    Col { at: usize, to: usize },
}

/// 表の選択と書き換え中の文字（パネルが持つ）
#[derive(Debug, Clone, Default)]
pub(crate) struct SheetState {
    /// 今のセル（行, 列）
    cur: Option<(usize, usize)>,
    /// Shift で広げた範囲の端のセル（今のセルとの四角が選んだ範囲）
    anchor: Option<(usize, usize)>,
    edit: Option<Edit>,
    /// 表をクリックした後（キーを受ける）
    active: bool,
    drag: Option<Drag>,
    /// ドラッグで範囲を選んでいる途中
    drag_sel: bool,
    grab: Option<Grab>,
    /// 表の下に出す一言（変えられない理由・貼り付けの誤り）
    note: Option<String>,
}

impl SheetState {
    /// 選んでいる範囲（行, 列）
    fn sel(&self) -> Option<(RangeInclusive<usize>, RangeInclusive<usize>)> {
        let (r, c) = self.cur?;
        let (ar, ac) = self.anchor.unwrap_or((r, c));
        Some((ar.min(r)..=ar.max(r), ac.min(c)..=ac.max(c)))
    }
    /// 行の操作（移動・複製・削除）の行。列だけを選んだとき（全部の行・一部の列）は今の行だけ
    /// （列の見出しを押して Ctrl+Delete で全部の行を消さない）。n・m は表の行と列の数
    fn op_rows(&self, n: usize, m: usize) -> Option<RangeInclusive<usize>> {
        let (rows, cols) = self.sel()?;
        let col_only = self.multi()
            && n > 1
            && *rows.start() == 0
            && *rows.end() + 1 == n
            && !(*cols.start() == 0 && *cols.end() + 1 == m);
        if col_only {
            self.cur.map(|(r, _)| r..=r)
        } else {
            Some(rows)
        }
    }
    /// 2 つ以上のセルを選んでいるか
    fn multi(&self) -> bool {
        self.anchor.is_some() && self.anchor != self.cur
    }
    /// (r, c) が選んだ範囲（2 つ以上）の中か
    fn in_sel(&self, r: usize, c: usize) -> bool {
        self.multi()
            && self
                .sel()
                .is_some_and(|(rs, cs)| rs.contains(&r) && cs.contains(&c))
    }
    /// 行 r0..=r の全部の列を選ぶ（r0 が None なら r だけ）
    fn select_rows(&mut self, r0: Option<usize>, r: usize, m: usize) {
        if m == 0 {
            return;
        }
        self.anchor = Some((r0.unwrap_or(r), m - 1));
        self.cur = Some((r, 0));
    }
    /// 列 c0..=c の全部の行を選ぶ
    fn select_cols(&mut self, c0: Option<usize>, c: usize, n: usize) {
        if n == 0 {
            return;
        }
        self.anchor = Some((n - 1, c0.unwrap_or(c)));
        self.cur = Some((0, c));
    }
    /// 選んだ範囲（試験）
    #[cfg(test)]
    pub(crate) fn selection(&self) -> Option<(RangeInclusive<usize>, RangeInclusive<usize>)> {
        self.sel()
    }
    /// セル (r, c) を選んでキーを受ける（試験）
    #[cfg(test)]
    pub(crate) fn select(&mut self, r: usize, c: usize) {
        self.cur = Some((r, c));
        self.active = true;
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
    /// 行の範囲を上（負）・下（正）へ動かす
    shift: Option<(RangeInclusive<usize>, i32)>,
    /// 列を動かす（元, 先）
    move_col: Option<(usize, usize)>,
    /// 行・列を落とす所の線（横線の y と左端、縦線の x と上端）
    drop_h: Option<(f32, f32)>,
    drop_v: Option<(f32, f32)>,
    /// 写しを下に挿入する行の範囲
    duplicate: Option<RangeInclusive<usize>>,
    remove: Option<RangeInclusive<usize>>,
    copy: bool,
    /// 表の見えている範囲（はみ出す文字を切る）
    view: Option<egui::Rect>,
    /// はみ出す文字（行の縞が後のセルで塗られるので、表を描いた後に描く）
    spill: Vec<(
        egui::Pos2,
        std::sync::Arc<egui::Galley>,
        egui::Rect,
        egui::Color32,
    )>,
}

const ROW_H: f32 = 20.0;
/// 数のセルの右端のつまみの幅
const GRIP_W: f32 = 6.0;
/// 候補のあるセルの右の ▼ の幅
const ARROW_W: f32 = 14.0;

/// 表を描く。何か書き換えたら true
pub(crate) fn show(ui: &mut Ui, id: impl Hash, st: &mut SheetState, sh: &mut dyn Sheet) -> bool {
    let id = egui::Id::new(id);
    let (n, m) = (sh.rows(), sh.cols());
    // 最後の行の下の空の行（行 n。左端の列だけ）
    let ghost = sh.new_row(false);
    let nv = n + usize::from(ghost.is_some());
    // 行が減ったら選択を中へ
    if nv == 0 || m == 0 {
        st.cur = None;
        st.anchor = None;
        st.edit = None;
    } else if let Some((r, c)) = st.cur {
        let r = r.min(nv - 1);
        st.cur = Some((r, if r >= n { 0 } else { c.min(m - 1) }));
        st.anchor = st
            .anchor
            .filter(|_| r < n)
            .map(|(a, b)| (a.min(n - 1), b.min(m - 1)));
    }
    if st
        .edit
        .as_ref()
        .is_some_and(|e| e.col >= m || e.row > n || (e.row == n && (ghost.is_none() || e.col > 0)))
    {
        st.edit = None;
    }
    let mut out = Out::default();
    if !ui.input(|i| i.pointer.primary_down()) {
        st.drag = None;
        st.drag_sel = false;
        // 見出しのドラッグを放した: 行・列を動かす
        match st.grab.take() {
            Some(Grab::Rows { lo, hi, at, to }) if to != at && hi < n => {
                let d = (to as i64 - at as i64).clamp(-(lo as i64), (n - 1 - hi) as i64);
                if d != 0 {
                    out.shift = Some((lo..=hi, d as i32));
                }
            }
            Some(Grab::Col { at, to }) if at != to && to < m => out.move_col = Some((at, to)),
            _ => {}
        }
    }
    if st.grab.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    // フォーカスは描く前に見る（Tab を押すと egui が描いている間に表の中のボタンへフォーカスを移す）
    let free = ui.memory(|mm| mm.focused().is_none());
    let inner = ui.scope(|ui| table(ui, id, st, sh, ghost.as_ref(), &mut out));
    // セルの painter はセルで切られるので、層の painter で描く
    let lp = ui.ctx().layer_painter(ui.layer_id());
    for (pos, g, clip, color) in out.spill.drain(..) {
        lp.with_clip_rect(clip).galley(pos, g, color);
    }
    let rect = inner.response.rect;
    let line = egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color);
    let lp = lp.with_clip_rect(out.view.unwrap_or(rect));
    if let Some((y, x0)) = out.drop_h {
        lp.hline(x0..=rect.right(), y, line);
    }
    if let Some((x, y0)) = out.drop_v {
        lp.vline(x, y0..=rect.bottom(), line);
    }
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

/// 書いた数（空は def）。lo 未満・hi 超は誤り
pub(crate) fn num_in(t: &str, what: &str, def: f64, lo: f64, hi: f64) -> Result<f64, String> {
    let x = crate::widgets::parse_num(t, what)?.unwrap_or(def);
    if x < lo || x > hi {
        return Err(trf(
            "{} must be from {} to {}",
            &[
                &what,
                &crate::json_view::fmt_num(lo),
                &crate::json_view::fmt_num(hi),
            ],
        ));
    }
    Ok(x)
}

/// n 行から消す行の範囲（全部の行なら最初の行は残す）。消す行が無ければ None
pub(crate) fn keep_range(n: usize, rows: RangeInclusive<usize>) -> Option<(usize, usize)> {
    let hi = (*rows.end()).min(n.checked_sub(1)?);
    let lo = match *rows.start() {
        0 if hi + 1 == n => 1,
        lo => lo,
    };
    (lo <= hi).then_some((lo, hi))
}

/// 表の状態を egui の一時の記憶に置いて描く（状態を持つ所の無いフォームの表）。(書き換えたか, 数をドラッグ中か)
pub(crate) fn show_temp(ui: &mut Ui, id: impl Hash, sh: &mut dyn Sheet) -> (bool, bool) {
    let id = egui::Id::new(id);
    let mut st: SheetState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let changed = show(ui, id, &mut st, sh);
    let busy = st.drag.is_some();
    ui.data_mut(|d| d.insert_temp(id, st));
    (changed, busy)
}

fn table(
    ui: &mut Ui,
    id: egui::Id,
    st: &mut SheetState,
    sh: &dyn Sheet,
    ghost: Option<&Cell>,
    out: &mut Out,
) {
    let (n, m) = (sh.rows(), sh.cols());
    let nv = n + usize::from(ghost.is_some());
    let cur_row = st.cur.map(|x| x.0).filter(|&r| r < n);
    // 全部の列を選んだ行（行の番号を目立たせる）
    let full_rows = st
        .sel()
        .filter(|(_, cs)| st.multi() && *cs.start() == 0 && *cs.end() + 1 == m)
        .map(|x| x.0);
    let weak = crate::theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let shift = ui.input(|i| i.modifiers.shift);
    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            out.view = Some(ui.clip_rect());
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
                    // 左上の # で表全体を選ぶ
                    let r = ui
                        .add(
                            egui::Label::new(RichText::new("#").small().color(weak))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(tr("Select all (Ctrl+A)"));
                    if r.clicked() && n > 0 && m > 0 {
                        out.clicked = true;
                        st.anchor = Some((n - 1, m - 1));
                        st.cur = Some((0, 0));
                    }
                });
                for c in 0..m {
                    h.col(|ui| {
                        // クリックは見出しのセル全体で受ける（文字の上だけでなく）
                        let t = sh.header(c, cur_row);
                        let r = ui.interact(
                            ui.max_rect(),
                            ui.id().with("sheet_head"),
                            Sense::click_and_drag(),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(t).small().strong().color(weak))
                                .truncate(),
                        );
                        let tip = sh.header_tip(c, cur_row);
                        let r = if tip.is_empty() {
                            r
                        } else {
                            r.on_hover_text(tip)
                        };
                        // 見出しのクリックで列全体を選ぶ（Shift で前に選んだ列から）
                        if r.clicked() {
                            out.clicked = true;
                            let c0 = shift
                                .then(|| st.anchor.map(|a| a.1).or(st.cur.map(|x| x.1)))
                                .flatten();
                            st.select_cols(c0, c, n);
                        }
                        sh.header_ui(&r, c);
                        // 見出しのドラッグで列を動かす
                        if r.drag_started() && sh.can_move_col(c) {
                            out.clicked = true;
                            st.grab = Some(Grab::Col { at: c, to: c });
                        }
                        col_target(ui, st, sh, out, c);
                    });
                }
                h.col(|_| {});
            })
            .body(|body| {
                body.rows(ROW_H, nv, |mut row| {
                    let r = row.index();
                    if r >= n {
                        // 最後の空の行（左端に書くと行を足す）
                        row.col(|ui| {
                            ui.label(RichText::new("+").monospace().color(weak));
                            row_target(ui, st, out, r, n);
                        });
                        if let Some(cell) = ghost {
                            row.col(|ui| ghost_ui(ui, st, sh, cell, out, n, weak, accent));
                        }
                        for _ in 1..=m {
                            row.col(|_| {});
                        }
                        return;
                    }
                    row.set_selected(full_rows.as_ref().is_some_and(|s| s.contains(&r)));
                    row.col(|ui| {
                        let resp = ui.interact(
                            ui.max_rect(),
                            ui.id().with("sheet_num"),
                            Sense::click_and_drag(),
                        );
                        ui.label(RichText::new(format!("{}", r + 1)).monospace().color(weak));
                        // 行の番号のドラッグで行（選んだ行の範囲）を動かす
                        if resp.drag_started() {
                            out.clicked = true;
                            let (lo, hi) = match full_rows.as_ref().filter(|s| s.contains(&r)) {
                                Some(s) => (*s.start(), *s.end()),
                                None => {
                                    st.select_rows(None, r, m);
                                    (r, r)
                                }
                            };
                            st.grab = Some(Grab::Rows {
                                lo,
                                hi,
                                at: r,
                                to: r,
                            });
                        }
                        row_target(ui, st, out, r, n);
                        // 行の番号で行全体を選ぶ（Shift で前に選んだ行から。範囲の中の右クリックは範囲を残す）
                        let keep = resp.secondary_clicked()
                            && full_rows.as_ref().is_some_and(|s| s.contains(&r));
                        if (resp.clicked() || resp.secondary_clicked()) && !keep {
                            out.clicked = true;
                            let r0 = (shift && resp.clicked())
                                .then(|| st.anchor.map(|a| a.0).or(st.cur.map(|x| x.0)))
                                .flatten();
                            st.select_rows(r0, r, m);
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
            super::scroll_gap(ui);
        });
}

/// 今のセルを (r, c) に。Shift なら行の範囲を広げる。keep は範囲を残す（範囲の中の右クリック）
fn click_to(st: &mut SheetState, r: usize, c: usize, shift: bool, keep: bool) {
    if keep {
        return;
    }
    if shift {
        if st.anchor.is_none() {
            st.anchor = st.cur;
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
    if st.in_sel(r, c) {
        ui.painter().rect_filled(
            rect,
            0.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.35),
        );
    }
    let drag_ok = cell.numeric() && cell.ro.is_none();
    // ドラッグは数の増減（Alt・右端のつまみ）か範囲の選択
    let resp = ui.interact(rect, ui.id().with("sheet_cell"), Sense::click_and_drag());
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
    // 候補のあるセルの右の ▼（押すと候補の一覧を出して選ぶ）
    let pick = matches!(cell.kind, Kind::List { .. }) && cell.ro.is_none();
    let arrow = egui::Rect::from_min_max(egui::pos2(rect.right() - ARROW_W, rect.top()), rect.max);
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
            if cell.overflow {
                // 右の空のセルへはみ出して描く（表の見えている範囲で切る）
                let font = egui::TextStyle::Body.resolve(ui.style());
                let g = ui.painter().layout_no_wrap(t.to_string(), font, color);
                let pos = egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0);
                let view = out.view.unwrap_or(rect);
                let clip =
                    egui::Rect::from_min_max(rect.min, egui::pos2(view.right(), rect.bottom()))
                        .intersect(view);
                out.spill.push((pos, g, clip, color));
            } else {
                ui.with_layout(layout, |ui| {
                    if cell.numeric() {
                        ui.add_space(GRIP_W - 2.0);
                    }
                    if pick {
                        ui.set_max_width((rect.width() - ARROW_W).max(0.0));
                    }
                    ui.add(egui::Label::new(rt).truncate());
                });
            }
            if pick {
                let on = hover
                    && ui
                        .input(|i| i.pointer.hover_pos())
                        .is_some_and(|p| arrow.contains(p));
                ui.painter().text(
                    arrow.center(),
                    egui::Align2::CENTER_CENTER,
                    "▼",
                    egui::FontId::proportional(9.0),
                    if on { ui.visuals().text_color() } else { weak },
                );
            }
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
    } else if cell.mark {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
    }
    if resp.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin());
        let from_grip = press.is_some_and(|p| grip.contains(p));
        if !(drag_ok && (alt || from_grip)) {
            // 範囲の選択を始める（Shift なら今の範囲から）
            out.clicked = true;
            if !shift || st.cur.is_none() {
                st.anchor = Some((r, c));
            } else if st.anchor.is_none() {
                st.anchor = st.cur;
            }
            st.cur = Some((r, c));
            st.drag_sel = true;
        } else {
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
    // ドラッグで選んでいる途中: ポインタの下のセルまで広げる（ドラッグ中は他のセルに hover が来ない）
    if st.drag_sel
        && st.cur != Some((r, c))
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| rect.contains(p))
    {
        st.cur = Some((r, c));
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
        let keep = resp.secondary_clicked() && st.in_sel(r, c);
        click_to(st, r, c, shift && !resp.secondary_clicked(), keep);
    }
    let on_arrow = pick
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| arrow.contains(p));
    if resp.double_clicked() || (resp.clicked() && on_arrow) {
        out.start = Some((r, c, None));
    }
    resp.context_menu(|ui| {
        sh.cell_menu(ui, r, c);
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
fn row_menu(ui: &mut Ui, st: &SheetState, sh: &dyn Sheet, out: &mut Out, r: usize) {
    out.clicked = true;
    let sel = st
        .op_rows(sh.rows(), sh.cols())
        .filter(|s| s.contains(&r))
        .unwrap_or(r..=r);
    if ui.button(tr("Copy")).on_hover_text("Ctrl+C").clicked() {
        out.copy = true;
        ui.close_menu();
    }
    let ins = sh.can_insert();
    if ins
        && ui
            .button(tr("Insert row above"))
            .on_hover_text("Insert")
            .clicked()
    {
        out.insert = Some(r);
        ui.close_menu();
    }
    if ins && ui.button(tr("Insert row below")).clicked() {
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
        .add_enabled(
            *sel.end() + 1 < sh.rows(),
            egui::Button::new(tr("Move down")),
        )
        .on_hover_text("Ctrl+Shift+↓")
        .clicked()
    {
        out.shift = Some((sel.clone(), 1));
        ui.close_menu();
    }
    if ins
        && ui
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
    // Ctrl+A は取る（どのパネルでも効く「次の状態」に渡さない）
    // （Ctrl+Shift+A は「前の状態」なので取らない）
    let all =
        ui.input_mut(|i| !i.modifiers.shift && i.consume_key(egui::Modifiers::COMMAND, Key::A));
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
            all: false,
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
    let ev = Keys { all, ..ev };
    out.copy |= ev.copy;
    if ev.esc {
        st.anchor = None;
        st.active = false;
        return ev.tab;
    }
    if ev.insert {
        out.insert = Some(st.cur.map_or(n, |x| x.0.min(n)));
        return ev.tab;
    }
    let ghost = sh.new_row(false).is_some();
    if let Some((r, _)) = st.cur.filter(|x| x.0 >= n) {
        // 最後の空の行: ↑ で表へ、打つ・Enter・F2 で書き始める（書いた値の行を足す）
        if ev.up && n > 0 {
            st.cur = Some((n - 1, 0));
        } else if ev.enter || ev.f2 || ev.typed.is_some() || ev.space {
            out.start = Some((r, 0, ev.typed));
        }
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
    // 最後の行から ↓ で空の行へ
    if ev.down && !ev.shift && !ev.alt && !ev.command && r + 1 == n && ghost {
        st.anchor = None;
        st.cur = Some((n, 0));
        return ev.tab;
    }
    if ev.all {
        st.anchor = Some((n - 1, m - 1));
        st.cur = Some((0, 0));
        return ev.tab;
    }
    // Ctrl+Shift+↑↓: 選んだ行を上下へ動かす
    if ev.command && ev.shift && (ev.up || ev.down) {
        out.shift = st.op_rows(n, m).map(|s| (s, if ev.up { -1 } else { 1 }));
        return ev.tab;
    }
    if ev.dup {
        out.duplicate = st.op_rows(n, m);
        return ev.tab;
    }
    if ev.del && ev.command {
        out.remove = st.op_rows(n, m);
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
        // Shift+矢印で範囲を広げる（Shift+Tab は前のセルへ移るだけ）
        if ev.shift && (ev.up || ev.down || ev.left || ev.right) {
            if st.anchor.is_none() {
                st.anchor = Some((r, c));
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
        if let Some((rows, cols)) = st.sel().filter(|_| st.multi()) {
            for rr in rows {
                for cc in cols.clone() {
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
    all: bool,
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

/// 表の文字（タブ区切り）。範囲を選んでいればその範囲、でなければ今のセル
fn copy_text(st: &SheetState, sh: &dyn Sheet) -> Option<String> {
    let (r, c) = st.cur.filter(|x| x.0 < sh.rows())?;
    if !st.multi() {
        return Some(sh.cell(r, c).text);
    }
    let (rows, cols) = st.sel()?;
    Some(
        rows.map(|rr| {
            cols.clone()
                .map(|cc| sh.cell(rr, cc).text)
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n"),
    )
}

/// 貼り付け: 選んだ範囲の左上から右下へ。足りない行は足す。1 つの値を範囲に貼ると範囲を埋める
fn paste(st: &mut SheetState, sh: &dyn Sheet, out: &mut Out, text: &str) {
    let Some((rows, cols)) = st.sel() else { return };
    let (r0, c0) = (*rows.start(), *cols.start());
    let one = text.trim_end_matches(['\n', '\r']);
    if st.multi() && !one.contains(['\t', '\n']) {
        for rr in rows {
            for cc in cols.clone() {
                if sh.cell(rr, cc).ro.is_none() {
                    out.set.push((rr, cc, one.trim().to_string()));
                }
            }
        }
        return;
    }
    let lines: Vec<&str> = one.lines().collect();
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

/// rows の写しを rows の下に挿入する（空の行を足して、変えられるセルを順に書く。足せない表は false）。
/// 写せなかったセルの最初の理由を err に
fn duplicate(sh: &mut dyn Sheet, rows: RangeInclusive<usize>, err: &mut Option<String>) -> bool {
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
                if let Err(e) = sh.set(at + i, c, t) {
                    if err.is_none() && !e.is_empty() {
                        *err = Some(e);
                    }
                }
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
        // 最後の空の行に書いたら行を足す（空ならやめる）
        let new = r >= sh.rows();
        let res = if !new {
            sh.set(r, c, &t)
        } else if t.trim().is_empty() {
            Ok(())
        } else {
            sh.add_row(&t)
        };
        match res {
            Ok(()) => {
                changed |= !new || !t.trim().is_empty();
                st.edit = None;
                st.active = true;
                st.note = None;
                // 行の無い表（最後の空の行に何も書かずに確定した）は動かない
                if let Some((dr, dc)) = out.after.filter(|_| sh.rows() > 0 && sh.cols() > 0) {
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
        let cell = if r < sh.rows() {
            sh.cell(r, c)
        } else {
            match sh.new_row(false) {
                Some(cell) => cell,
                None => Cell::empty().ro(Some(String::new())),
            }
        };
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
        // 1 行ずつ入れ替えて d 行動かす（端で止める）
        let (mut lo, mut hi) = (*rows.start(), *rows.end());
        let mut moved = 0i64;
        for _ in 0..d.unsigned_abs() {
            let ok = if d < 0 {
                lo > 0 && (lo..=hi).all(|i| sh.swap(i - 1))
            } else {
                hi + 1 < sh.rows() && (lo..=hi).rev().all(|i| sh.swap(i))
            };
            if !ok {
                break;
            }
            moved += i64::from(d.signum());
            (lo, hi) = (
                (lo as i64 + i64::from(d.signum())) as usize,
                (hi as i64 + i64::from(d.signum())) as usize,
            );
        }
        if moved != 0 {
            changed = true;
            let mv = |x: usize| (x as i64 + moved) as usize;
            st.cur = st.cur.map(|(r, c)| (mv(r), c));
            st.anchor = st.anchor.map(|(r, c)| (mv(r), c));
            st.edit = None;
            st.active = true;
        }
    }
    if let Some((from, to)) = out.move_col {
        if sh.move_col(from, to) {
            changed = true;
            st.edit = None;
            st.active = true;
            st.select_cols(None, to, sh.rows());
        }
    }
    if let Some(rows) = out.duplicate {
        let mut err = None;
        if duplicate(sh, rows.clone(), &mut err) {
            changed = true;
            st.note = err;
            // 写しを選ぶ（列の範囲はそのまま）
            let n = rows.end() - rows.start() + 1;
            let (c, ac) = match (st.cur, st.anchor) {
                (Some((_, c)), Some((_, a))) => (c, a),
                (Some((_, c)), None) => (c, c),
                _ => (0, 0),
            };
            st.anchor = Some((rows.end() + 1, ac));
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

#[path = "sheet_edit.rs"]
mod edit;
#[cfg(test)]
use edit::completion;
#[cfg(test)]
pub(crate) use edit::filter_items;
use edit::{col_target, edit_ui, ghost_ui, row_target};

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
