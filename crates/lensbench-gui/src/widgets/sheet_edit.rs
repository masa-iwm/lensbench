//! 表の部品（[`super`]）の書き換え中のセル・候補の補い・最後の空の行・見出しのドラッグ

use egui::{RichText, Sense, Ui};

use super::{Cell, Edit, Grab, Kind, Out, Sheet, SheetState, ARROW_W};
use crate::i18n::tr;

/// 行の番号のドラッグの行き先（ポインタの高さの行）と、落とす所の線
pub(super) fn row_target(ui: &Ui, st: &mut SheetState, out: &mut Out, r: usize, n: usize) {
    let Some(Grab::Rows { at, to, .. }) = st.grab.as_mut() else {
        return;
    };
    let rect = ui.max_rect();
    if n > 0
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| p.y >= rect.top() && p.y < rect.bottom())
    {
        *to = r.min(n - 1);
    }
    if *to == r && *to != *at {
        let y = if *to > *at { rect.bottom() } else { rect.top() };
        out.drop_h = Some((y, rect.left()));
    }
}

/// 列の見出しのドラッグの行き先（ポインタの横の、動かせる列）と、落とす所の線
pub(super) fn col_target(ui: &Ui, st: &mut SheetState, sh: &dyn Sheet, out: &mut Out, c: usize) {
    let Some(Grab::Col { at, to }) = st.grab.as_mut() else {
        return;
    };
    let rect = ui.max_rect();
    if sh.can_move_col(c)
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| p.x >= rect.left() && p.x < rect.right())
    {
        *to = c;
    }
    if *to == c && *to != *at {
        let x = if *to > *at { rect.right() } else { rect.left() };
        out.drop_v = Some((x, rect.top()));
    }
}

/// 最後の空の行の左端のセル（クリックで選び、打つかダブルクリック・▼で書き始める）
#[allow(clippy::too_many_arguments)]
pub(super) fn ghost_ui(
    ui: &mut Ui,
    st: &mut SheetState,
    sh: &dyn Sheet,
    cell: &Cell,
    out: &mut Out,
    n: usize,
    weak: egui::Color32,
    accent: egui::Color32,
) {
    if st.edit.as_ref().is_some_and(|e| e.row == n) {
        let full = sh.new_row(true).unwrap_or_else(|| cell.clone());
        edit_ui(ui, st, &full, out);
        return;
    }
    let rect = ui.max_rect();
    let resp = ui.interact(rect, ui.id().with("sheet_new"), Sense::click());
    let name = tr("New row").to_string();
    resp.widget_info(move || {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone())
    });
    let hint = if cell.hint.is_empty() {
        tr("Add a row")
    } else {
        cell.hint.as_str()
    };
    let pick = matches!(cell.kind, Kind::List { .. });
    ui.scope(|ui| {
        if pick {
            ui.set_max_width((rect.width() - ARROW_W).max(0.0));
        }
        ui.add(egui::Label::new(RichText::new(hint).italics().color(weak)).truncate());
    });
    let arrow = egui::Rect::from_min_max(egui::pos2(rect.right() - ARROW_W, rect.top()), rect.max);
    if pick {
        ui.painter().text(
            arrow.center(),
            egui::Align2::CENTER_CENTER,
            "▼",
            egui::FontId::proportional(9.0),
            weak,
        );
    }
    if st.cur == Some((n, 0)) {
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.5_f32, accent),
            egui::StrokeKind::Inside,
        );
    }
    let resp = resp.on_hover_text(tr("Type or pick a value here to add a row"));
    if resp.clicked() || resp.double_clicked() || resp.secondary_clicked() {
        out.clicked = true;
        st.cur = Some((n, 0));
        st.anchor = None;
    }
    let on_arrow = pick
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| arrow.contains(p));
    if resp.double_clicked() || (resp.clicked() && on_arrow) {
        out.start = Some((n, 0, None));
    }
}

/// 書き換え中のセル（入力欄。候補のある欄は下に一覧）
pub(super) fn edit_ui(ui: &mut Ui, st: &mut SheetState, cell: &Cell, out: &mut Out) {
    let Some(e) = st.edit.as_mut() else { return };
    let (r, c) = (e.row, e.col);
    let first = !e.started;
    let (resp, keys) = crate::widgets::cell::input(
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
        e.query = None;
    }
    // 打った文字が増えたら（打ち始めも）続く候補で補う
    let n_chars = e.text.chars().count();
    if (resp.changed() && n_chars > e.prev_n) || (first && e.filtered) {
        if let Some(full) = completion(items, &e.text) {
            let k = e.text.chars().count();
            e.query = Some(std::mem::replace(&mut e.text, full));
            if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(k),
                    egui::text::CCursor::new(e.text.chars().count()),
                )));
                s.store(ui.ctx(), resp.id);
            }
        }
    }
    // 補った後は打った文字の数（補った部分を消しても補い直さず、次の 1 文字で補う）
    e.prev_n = e.query.as_deref().unwrap_or(&e.text).chars().count();
    let shown: Vec<(String, String)> = if e.filtered {
        filter_items(items, e.query.as_deref().unwrap_or(&e.text))
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
    let res = crate::widgets::choices::Choices {
        scroll_to: scroll,
        min_width: 160.0,
        ..crate::widgets::choices::Choices::new(&shown, e.pick)
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

/// 打った文字 q に続く候補（大小を問わない前方一致で q より長い最初の候補）。q と同じ名前の候補があれば補わない
pub(super) fn completion(items: &[(String, String)], q: &str) -> Option<String> {
    let ql = q.to_lowercase();
    if ql.trim().is_empty() || items.iter().any(|x| x.0.to_lowercase() == ql) {
        return None;
    }
    items
        .iter()
        .map(|x| &x.0)
        .find(|x| x.chars().count() > q.chars().count() && x.to_lowercase().starts_with(&ql))
        .cloned()
}

/// 打った文字で絞った候補（前方一致を先に）
pub(super) fn filter_items(items: &[(String, String)], q: &str) -> Vec<(String, String)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    let mut head = Vec::new();
    let mut rest = Vec::new();
    for it in items {
        match crate::widgets::match_rank(&it.0.to_lowercase(), &it.1.to_lowercase(), &q) {
            Some(true) => head.push(it.clone()),
            Some(false) => rest.push(it.clone()),
            None => {}
        }
    }
    head.extend(rest);
    head
}
