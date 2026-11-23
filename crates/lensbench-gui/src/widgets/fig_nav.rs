//! 図・グラフの操作の続き（[`super::pan_zoom`] から使う）: 図ごとの決め事、キー、右クリックのメニュー、
//! 戻る・進むの履歴、図の画像の写し・保存、スクロールの中の図で素のホイールを回したときの案内

use super::PanZoom;
use crate::i18n::tr;
use egui::{Response, Ui, Vec2};

/// 図ごとの操作の決め事（[`super::pan_zoom`] に渡す）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NavOpts {
    /// 左ドラッグでも移動する（ガラスマップ・グラフなど、左ドラッグにほかの役が無い図）
    pub left_pans: bool,
    /// パネルいっぱいの図（スクロールの中に無い）。素のホイールで拡大縮小する。false の図（スクロールの中）では
    /// 素のホイールはスクロールに残し、拡大の仕方を一瞬出す
    pub full: bool,
    /// 矢印キーで移動する（3D は回すので false）
    pub arrows: bool,
    /// ＋・－・0・F・矢印・戻るのキーを受ける（レイアウト図は要素・寸法を選んでいる間は、そのキーの役に譲って false）
    pub keys: bool,
    /// グラフ（右クリックのメニューに「軸の範囲」と「データを写す」を出す）
    pub graph: bool,
}

impl NavOpts {
    /// スクロールの中の図の既定
    pub(crate) const FIGURE: NavOpts = NavOpts {
        left_pans: false,
        full: false,
        arrows: true,
        keys: true,
        graph: false,
    };
}

/// 右クリックのメニューで選んだ、図を描く側がする操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FigMenu {
    /// 軸の範囲を数で決める
    AxisRange,
    /// 曲線の点を CSV で写す
    CopyData,
}

/// キーを受けるか: ポインタが図の上にあるか図にフォーカスがあり、ほかの欄（入力欄・表）がキーを取っていない
fn keys_active(ui: &Ui, resp: &Response) -> bool {
    (resp.hovered() || resp.has_focus()) && ui.memory(|m| m.focused().is_none_or(|f| f == resp.id))
}

/// ＋・－・0・F・Home・矢印・Backspace・Alt＋←→ を out に足す
pub(super) fn read_keys(ui: &Ui, resp: &Response, opts: NavOpts, out: &mut PanZoom) {
    let active = keys_active(ui, resp);
    // Home は前から（選んでいる間も）全体表示
    if active && ui.input(|i| i.key_pressed(egui::Key::Home)) {
        out.reset = true;
    }
    if !(active && opts.keys) {
        return;
    }
    let rect = resp.rect;
    ui.input(|i| {
        let m = i.modifiers;
        // ＋ は多くの配列で Shift が要るので Shift は見ない
        let plain = !m.command && !m.alt;
        if plain && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
            out.zoom = super::ZOOM_STEP;
            out.zoom_xy = Vec2::splat(out.zoom);
            out.at = Some(rect.center());
        }
        if plain && i.key_pressed(egui::Key::Minus) {
            out.zoom = 1.0 / super::ZOOM_STEP;
            out.zoom_xy = Vec2::splat(out.zoom);
            out.at = Some(rect.center());
        }
        if m.is_none() && (i.key_pressed(egui::Key::Num0) || i.key_pressed(egui::Key::F)) {
            out.reset = true;
        }
        if (m.is_none() && i.key_pressed(egui::Key::Backspace))
            || (m.alt && i.key_pressed(egui::Key::ArrowLeft))
        {
            out.back = true;
        }
        if m.alt && i.key_pressed(egui::Key::ArrowRight) {
            out.forward = true;
        }
        if opts.arrows && m.is_none() {
            // 地図と同じく、矢印の向きの先が見えるように中身を逆へ（見えている幅の 1 割）
            let s = rect.size() * 0.1;
            let mut d = Vec2::ZERO;
            if i.key_pressed(egui::Key::ArrowLeft) {
                d.x += s.x;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                d.x -= s.x;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                d.y += s.y;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                d.y -= s.y;
            }
            out.pan += d;
        }
    });
}

/// 素のホイール（Ctrl・⌘ なし）: パネルいっぱいの図は拡大縮小、スクロールの中の図はスクロールに残して案内を出す
pub(super) fn plain_wheel(ui: &Ui, resp: &Response, opts: NavOpts, out: &mut PanZoom) {
    let Some(p) = resp.hover_pos() else {
        return;
    };
    let (d, m) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers));
    if d == Vec2::ZERO || m.command {
        return;
    }
    if opts.full {
        // egui の Ctrl＋ホイールと同じくらいの効き（1 目盛り 50 点ほどで 1.28 倍）
        let f = ((d.y + d.x) / 200.0).exp();
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        out.zoom = f;
        out.zoom_xy = Vec2::splat(f);
        out.at = Some(p);
    } else {
        let now = ui.input(|i| i.time);
        ui.data_mut(|dd| dd.insert_temp(hint_id(resp), now));
    }
}

fn hint_id(resp: &Response) -> egui::Id {
    resp.id.with("wheel_hint")
}

/// 案内を出す時間 [s]
const HINT_SECS: f64 = 1.5;

/// スクロールの中の図で素のホイールを回したときの案内（図の上の中ほど）
pub(super) fn paint_hint(ui: &Ui, resp: &Response) {
    let Some(t) = ui.data(|d| d.get_temp::<f64>(hint_id(resp))) else {
        return;
    };
    let left = HINT_SECS - (ui.input(|i| i.time) - t);
    if left <= 0.0 || !ui.rect_contains_pointer(resp.rect) {
        return;
    }
    let p = ui.painter_at(resp.rect);
    let font = egui::FontId::proportional(12.0);
    let col = ui.visuals().strong_text_color();
    let g = p.layout_no_wrap(
        tr("Ctrl+wheel (⌘ on Mac) or pinch to zoom").to_owned(),
        font,
        col,
    );
    let c = egui::pos2(
        resp.rect.center().x,
        resp.rect.top() + 12.0 + g.size().y / 2.0,
    );
    let r = egui::Rect::from_center_size(c, g.size() + egui::vec2(16.0, 8.0));
    p.rect_filled(r, 6.0, ui.visuals().extreme_bg_color.gamma_multiply(0.92));
    p.galley(r.center() - g.size() / 2.0, g, col);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(left));
}

/// 右クリックのメニュー。全体・戻る・進む・画像を写す・保存するはここで、軸の範囲とデータは描く側へ返す
pub(super) fn context_menu(resp: &Response, opts: NavOpts, out: &mut PanZoom) {
    resp.context_menu(|ui| {
        if ui.button(tr("Fit all")).clicked() {
            out.reset = true;
            ui.close();
        }
        if ui.button(tr("Back to the previous view")).clicked() {
            out.back = true;
            ui.close();
        }
        if ui.button(tr("Forward")).clicked() {
            out.forward = true;
            ui.close();
        }
        if opts.graph && ui.button(tr("Axis range…")).clicked() {
            out.menu = Some(FigMenu::AxisRange);
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Copy image")).clicked() {
            request_shot(ui.ctx(), resp, true);
            ui.close();
        }
        if ui.button(tr("Save image…")).clicked() {
            request_shot(ui.ctx(), resp, false);
            ui.close();
        }
        if opts.graph && ui.button(tr("Copy data (CSV)")).clicked() {
            out.menu = Some(FigMenu::CopyData);
            ui.close();
        }
    });
}

// ---- 戻る・進む ----

/// 図の表示の履歴（図ごとに egui の一時データ）
#[derive(Clone)]
struct Hist<T> {
    back: Vec<T>,
    fwd: Vec<T>,
    /// 最後に表示が変わった時刻
    last: f64,
}

impl<T> Default for Hist<T> {
    fn default() -> Self {
        Hist {
            back: Vec::new(),
            fwd: Vec::new(),
            last: f64::NEG_INFINITY,
        }
    }
}

/// 戻れる数
const HIST_MAX: usize = 50;
/// 前の操作からこれだけ空いたら、新しい操作として今の表示を覚える [s]（ドラッグやホイールの続きは 1 つに）
const HIST_GAP: f64 = 0.5;

/// 戻る・進む。図の表示 `view` を pz で変える前に呼ぶ。戻る・進むなら view を入れ替えて true を返す（このフレームは
/// pz のほかの操作を当てない）。表示を変える操作が始まる所で、変える前の view を覚える
pub(crate) fn history<T: Clone + Send + Sync + 'static>(
    ui: &Ui,
    resp: &Response,
    pz: &PanZoom,
    view: &mut T,
) -> bool {
    let id = resp.id.with("view_hist");
    let now = ui.input(|i| i.time);
    let mut h: Hist<T> = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let mut swapped = false;
    if pz.back {
        if let Some(v) = h.back.pop() {
            h.fwd.push(std::mem::replace(view, v));
            swapped = true;
        }
    } else if pz.forward {
        if let Some(v) = h.fwd.pop() {
            h.back.push(std::mem::replace(view, v));
            swapped = true;
        }
    } else if pz.changes() {
        if now - h.last > HIST_GAP {
            h.back.push(view.clone());
            if h.back.len() > HIST_MAX {
                h.back.remove(0);
            }
            h.fwd.clear();
        }
        h.last = now;
    }
    ui.data_mut(|d| d.insert_temp(id, h));
    swapped
}

/// pz 以外で表示を変える前（数で決めた軸の範囲など）に、今の表示を覚える
pub(crate) fn history_push<T: Clone + Send + Sync + 'static>(ui: &Ui, resp: &Response, view: &T) {
    let id = resp.id.with("view_hist");
    let mut h: Hist<T> = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    h.back.push(view.clone());
    if h.back.len() > HIST_MAX {
        h.back.remove(0);
    }
    h.fwd.clear();
    h.last = f64::NEG_INFINITY;
    ui.data_mut(|d| d.insert_temp(id, h));
}

// ---- 図の画像 ----

/// 図の画像を写す・保存する頼み（全体で 1 つ）。メニューが閉じた後のフレームで画面の写しを頼み、届いたら
/// アプリ（`App::take_shot`）が図の所を切り出す
#[derive(Debug, Clone)]
pub(crate) struct FigShot {
    /// 図（Response の id）
    pub fig: egui::Id,
    /// 図の場所（画面の点。毎フレーム新しくする）
    pub rect: egui::Rect,
    /// true なら写す、false なら PNG で保存する
    pub copy: bool,
    /// 写しを頼むまで待つフレーム（メニューが消えてから写す）
    wait: u8,
    /// 写しを頼んだ
    pub requested: bool,
}

fn shot_id() -> egui::Id {
    egui::Id::unique("lensbench-fig-shot")
}

fn request_shot(ctx: &egui::Context, resp: &Response, copy: bool) {
    let s = FigShot {
        fig: resp.id,
        rect: resp.rect,
        copy,
        wait: 2,
        requested: false,
    };
    ctx.data_mut(|d| d.insert_temp(shot_id(), s));
    ctx.request_repaint();
}

/// この図の写しの頼みがあれば、待ちを減らし、待ち終わったら画面の写しを頼む。写す間は道具・案内を描かないので true
pub(super) fn tick_shot(ui: &Ui, resp: &Response) -> bool {
    let Some(mut s) = pending_shot(ui.ctx()).filter(|s| s.fig == resp.id) else {
        return false;
    };
    s.rect = resp.rect;
    if !s.requested {
        if s.wait > 0 {
            s.wait -= 1;
            ui.ctx().request_repaint();
        } else {
            s.requested = true;
            crate::image_util::request_screenshot(ui.ctx());
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(shot_id(), s));
    true
}

/// 図の写しの頼み（無ければ None）
pub(crate) fn pending_shot(ctx: &egui::Context) -> Option<FigShot> {
    ctx.data(|d| d.get_temp::<FigShot>(shot_id()))
}

/// 写す図: パネルの頼み `app`（図の場所, 写すか, 保存の名前）が無ければ、画面の写しを頼み終えた図の右クリックの頼み。
/// 最後の値は図の右クリックの頼みを使ったか（[`clear_shot`] に渡す）
pub(crate) fn shot_or(
    ctx: &egui::Context,
    app: Option<(Option<egui::Rect>, bool, String)>,
) -> Option<(Option<egui::Rect>, bool, String, bool)> {
    match app {
        Some((r, copy, name)) => Some((r, copy, name, false)),
        None => pending_shot(ctx)
            .filter(|s| s.requested)
            .map(|s| (Some(s.rect), s.copy, "figure".to_owned(), true)),
    }
}

/// 写しが届いて切り出した後に呼ぶ。図の右クリックの頼みを使ったなら消す。パネルの頼みが同じ写しを使ったなら、
/// 図の頼みは残して次の写しを頼み直す（同じフレームに 2 つの頼みが重なっても片方が消えないように）
pub(crate) fn clear_shot(ctx: &egui::Context, used_fig: bool) {
    if used_fig {
        ctx.data_mut(|d| d.remove::<FigShot>(shot_id()));
    } else if let Some(mut s) = pending_shot(ctx).filter(|s| s.requested) {
        s.requested = false;
        s.wait = 0;
        ctx.data_mut(|d| d.insert_temp(shot_id(), s));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requested(ctx: &egui::Context) {
        let s = FigShot {
            fig: egui::Id::unique("fig"),
            rect: egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 50.0)),
            copy: true,
            wait: 0,
            requested: true,
        };
        ctx.data_mut(|d| d.insert_temp(shot_id(), s));
    }

    #[test]
    fn panel_shot_first_keeps_the_figure_shot_for_the_next_screenshot() {
        let ctx = egui::Context::default();
        requested(&ctx);
        // 同じ写しをパネルの頼みが使う: 図の頼みは残り、写しを頼み直す
        let app = Some((None, false, "layout".to_owned()));
        let got = shot_or(&ctx, app).unwrap();
        assert_eq!((got.2.as_str(), got.3), ("layout", false));
        clear_shot(&ctx, got.3);
        let s = pending_shot(&ctx).expect("figure shot kept");
        assert!(!s.requested);
        // 次の写しが届いたら図の頼みを使って消す
        requested(&ctx);
        let got = shot_or(&ctx, None).unwrap();
        assert_eq!((got.0, got.1, got.3), (Some(s.rect), true, true));
        clear_shot(&ctx, got.3);
        assert!(pending_shot(&ctx).is_none());
    }
}
