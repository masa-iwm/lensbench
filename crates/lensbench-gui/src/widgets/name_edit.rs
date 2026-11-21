//! 名前の欄の補完（ガラス・膜・群など）: 候補を打った文字で絞って出し（名前の完全一致・前方一致・部分一致の順）、
//! ↑↓ で選び、Tab・クリック（↑↓ で選んだ後は Enter も）で入れる。候補に無い名前を受けない欄（ガラス）は、欄の横に ⚠ を出す
//! （計算の前に打ち間違いに気づける）。
//!
//! ガラスの候補は App が画面ごとに egui の記憶に置き（[`set_glasses`]）、欄はそこから読む（要素の欄まで
//! ガラスの一覧を引数で渡さない）。まだ読んでいなければ、欄に入ったときに頼み（[`take_glass_request`]）、App が読む

use std::sync::Arc;

use egui::{Id, Key, Response, RichText, Ui};

use crate::i18n::tr;
use crate::lens_data::GlassChoice;

/// 候補（名前と説明）
pub(crate) trait Named {
    fn name(&self) -> &str;
    fn desc(&self) -> &str {
        ""
    }
}

impl Named for String {
    fn name(&self) -> &str {
        self
    }
}

impl Named for GlassChoice {
    fn name(&self) -> &str {
        &self.name
    }
    fn desc(&self) -> &str {
        &self.desc
    }
}

/// 打っている欄の補完の様子
#[derive(Clone, Default)]
struct Comp {
    /// 絞った文字と候補の数（変わったら絞り直す）
    key: Option<(String, usize)>,
    list: Arc<Vec<(String, String)>>,
    pick: usize,
    /// ↑↓ で候補を選んだ（Enter で入れるのはこのときだけ。新しい名前も書ける欄で、打った名前を候補に置き換えない）
    moved: bool,
    refocus: bool,
    /// 前のフレームでポインタが候補の一覧の上にあった。実際のマウスでは押したフレームで欄がフォーカスを失い、
    /// 離したフレーム（クリックが立つ）では has_focus も lost_focus も立たないので、その間も一覧を出し続ける
    over: bool,
}

/// 候補を打った文字で絞る（完全一致、前方一致、部分一致の順。説明にも当てる。大文字・小文字は区別しない）
pub(crate) fn filter<T: Named>(items: &[T], text: &str) -> Vec<(String, String)> {
    let q = text.trim().to_uppercase();
    let (mut exact, mut head, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    for it in items {
        let u = it.name().to_uppercase();
        let pair = || (it.name().to_string(), it.desc().to_string());
        match crate::widgets::match_rank(&u, &it.desc().to_uppercase(), &q) {
            Some(true) if u == q => exact.push(pair()),
            Some(true) => head.push(pair()),
            Some(false) => rest.push(pair()),
            None => {}
        }
    }
    exact.extend(head);
    exact.extend(rest);
    exact
}

/// 補完つきの名前の欄。known が false を返す名前（空でない）には欄の横に ⚠ を出し、unknown をホバーで見せる。
/// 候補を選んで値を変えたときも changed
pub(crate) fn name_edit<T: Named>(
    ui: &mut Ui,
    s: &mut String,
    width: f32,
    hint: &str,
    items: &[T],
    known: impl Fn(&str) -> bool,
    unknown: &str,
) -> Response {
    let mut resp = ui.add(
        egui::TextEdit::singleline(s)
            .hint_text(hint)
            .desired_width(width),
    );
    crate::widgets::name_field(&resp, hint, s);
    let sid = resp.id.with("name_comp");
    let mut st: Comp = ui.data(|d| d.get_temp(sid)).unwrap_or_default();
    if std::mem::take(&mut st.refocus) {
        resp.request_focus();
    }
    let enter = resp.lost_focus() && ui.input(|x| x.key_pressed(Key::Enter));
    // Tab・候補のクリックでは欄が先にフォーカスを失うので、失ったフレームと、ポインタが一覧の上にある間も見る
    let over = std::mem::take(&mut st.over);
    if resp.has_focus() || resp.lost_focus() || over {
        let key = (s.clone(), items.len());
        if st.key.as_ref() != Some(&key) {
            st.list = Arc::new(filter(items, s));
            st.key = Some(key);
            st.pick = 0;
            st.moved = false;
        }
        let m = st.list.clone();
        let exact = m.iter().any(|x| x.0.eq_ignore_ascii_case(s.trim()));
        let (up, down, tab) = ui.input(|x| {
            (
                x.key_pressed(Key::ArrowUp),
                x.key_pressed(Key::ArrowDown),
                x.key_pressed(Key::Tab),
            )
        });
        if up && st.pick > 0 {
            st.pick -= 1;
            st.moved = true;
        }
        if down && st.pick + 1 < m.len() {
            st.pick += 1;
            st.moved = true;
        }
        if !m.is_empty() && !(exact && m.len() == 1) {
            let shown = crate::widgets::choices::Choices {
                scroll_to: (up || down).then_some(st.pick),
                max_height: 240.0,
                ..crate::widgets::choices::Choices::new(&m, st.pick)
            }
            .show(ui, resp.id.with("comp"), resp.rect);
            // クリックで入れたら一覧を閉じる（欄のフォーカスは戻さない）
            st.over = shown.hovered && shown.clicked.is_none();
            let set = if let Some(k) = shown.clicked {
                m.get(k)
            } else if tab || (enter && st.moved) {
                st.refocus = tab;
                m.get(st.pick)
            } else {
                None
            };
            if let Some(n) = set.filter(|n| n.0 != *s) {
                *s = n.0.clone();
                resp.mark_changed();
            }
        }
    } else {
        st.key = None;
    }
    ui.data_mut(|d| d.insert_temp(sid, st));
    if !s.trim().is_empty() && !known(s.trim()) {
        let c = ui.visuals().warn_fg_color;
        ui.label(RichText::new("⚠").color(c)).on_hover_text(unknown);
    }
    resp
}

fn glass_id() -> Id {
    Id::new("lensbench-glass-names")
}

fn want_id() -> Id {
    Id::new("lensbench-glass-wanted")
}

/// 画面ごとに App が置くガラスの候補
pub(crate) fn set_glasses(ctx: &egui::Context, glasses: Arc<Vec<GlassChoice>>) {
    ctx.data_mut(|d| d.insert_temp(glass_id(), glasses));
}

/// ガラスの欄が候補を頼んだか（頼みは取り消す）
pub(crate) fn take_glass_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(want_id()))
        .unwrap_or(false)
}

/// ガラスの名前（硝材）の欄。候補はガラスカタログ。モデルガラス nd,νd も受け、medium なら屈折率の数も受ける
/// （間隔の媒質）。カタログに無い名前は ⚠
pub(crate) fn glass_edit(
    ui: &mut Ui,
    s: &mut String,
    width: f32,
    hint: &str,
    medium: bool,
) -> Response {
    let glasses: Arc<Vec<GlassChoice>> = ui.data(|d| d.get_temp(glass_id())).unwrap_or_default();
    let known = |t: &str| glass_known(t, &glasses, medium);
    let unknown = if medium {
        tr("Not in the glass catalogs (write a glass name, a refractive index, or a model glass as nd,νd like 1.5,60)")
    } else {
        tr("Not in the glass catalogs (write a model glass as nd,νd like 1.5,60; add catalogs in File > Catalog folders…)")
    };
    let r = name_edit(ui, s, width, hint, &glasses, known, unknown);
    // 候補が無ければ、欄に入ったときに頼む（欄を出しただけではカタログを読まない。表の硝材の欄と同じ）
    if r.has_focus() && glasses.is_empty() {
        ui.data_mut(|d| d.insert_temp(want_id(), true));
    }
    r
}

/// ガラスの欄の名前を受けるか: 空・AIR・MIRROR、カタログのガラス、モデルガラス nd,νd、medium なら屈折率（1 以上の数）。
/// カタログをまだ読んでいなければ受ける
pub(crate) fn glass_known(t: &str, glasses: &[GlassChoice], medium: bool) -> bool {
    crate::lens_data::material_ok(t, glasses, &[]).is_ok()
        || (medium && t.trim().parse::<f64>().is_ok_and(|n| n >= 1.0))
}
