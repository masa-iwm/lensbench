//! レイアウト図（2D）を描き、クリック・ドラッグを扱う。段に分ける:
//!
//! 1. [`xform`]：図の範囲から画面への写し方（拡大縮小・移動を含む）
//! 2. [`scene`]：画面の上の形（BS・玉・機構部品・開口・要素の名前・寸法線）を求める。描かず、ポインタも見ない
//! 3. [`hits`]：ポインタの下にある要素・寸法線・寸法の数字・玉の面の範囲・機構部品
//! 4. [`paint`]：描く（指した機構部品の名前だけ 3 の結果を使う）
//! 5. [`interact`]：クリック・ドラッグ、[`dim_edit`]：寸法の数字のその場の入力欄
//!
//! 3D の図は [`crate::view3d::show`] が描き、要素の選択だけを返す。

use super::*;
use egui::{Align2, FontId, Galley, Painter};
use std::sync::Arc;

fn panning(ui: &Ui, resp: &egui::Response) -> bool {
    crate::widgets::pan_zoom_pans(ui, resp, false)
}

/// 2D の図の拡大縮小・移動・全体表示。操作はほかの図と同じ [`crate::widgets::pan_zoom`]:
/// ホイール・ピンチでカーソルの所を中心に拡大縮小、中ボタン・右ボタン・Space＋左のドラッグで移動、右上の
/// ＋・－・囲んで拡大（Ctrl＋左ドラッグでも）、Home キー・何も無い所のダブルクリック（[`interact`]）・「全体」で
/// 全体表示。inner は図を描く所（画面。全体表示のときの中心はその中心）
fn navigate(ui: &mut Ui, inner: Rect, resp: &egui::Response, view: &mut View) {
    let fit_center = inner.center();
    let mut pz = crate::widgets::pan_zoom(ui, resp, false);
    // 囲んで拡大は、余白を除いた描く所いっぱいに広げる（図の場所全体で決めると囲んだ所の端が切れる）
    if let Some(z) = pz.area.and_then(|b| crate::widgets::zoom_to_box(inner, b)) {
        pz = z;
    }
    view.pan += pz.pan;
    if let Some(c) = pz.at {
        view.zoom_about(fit_center, c, pz.zoom);
    }
    // ダブルクリックは要素・寸法線の上では別の役なので、ここでは Home と「全体」だけ（何も無い所は interact）
    if pz.reset && !resp.double_clicked() {
        view.zoom_log = 0.0;
        view.pan = Vec2::ZERO;
    }
}

/// 図を描き、クリック・ドラッグを扱う
pub(super) fn figure(
    ui: &mut Ui,
    rect: Rect,
    resp: &egui::Response,
    mut combo: Option<&mut Combo>,
    d: &LayoutData,
    view: &mut View,
    accent: Color32,
) {
    // 開いた系・カタログのプレビュー（combo なし）は描くだけ。読むのは空の組合せ（毎フレーム作らない）
    static EMPTY: std::sync::LazyLock<Combo> = std::sync::LazyLock::new(|| Combo {
        picks: Vec::new(),
        ..Default::default()
    });
    view.d3 = d.d3.is_some();
    view.fixed = d.unfold || view.d3;
    let ro: &Combo = combo.as_deref().unwrap_or(&EMPTY);
    if let Some(sc) = &d.d3 {
        // 3D: 要素の選択だけ（寸法線・ドラッグでの移動はなし）
        let spans = d.spans_for(&ro.ids(), ro.lead_stop());
        let owner = |sf: [usize; 2]| {
            spans
                .iter()
                .position(|sp| sp.is_some_and(|sp| sp[0] <= sf[0] && sf[0] <= sp[1]))
        };
        view.axes.clear();
        view.mids.clear();
        if let Some(k) = crate::view3d::show(ui, resp, sc, &d.configs, &owner, view.sel, accent) {
            view.set_sel(k);
            view.dim = None;
        }
        return;
    }
    let vis = ui.visuals().clone();
    let weak = theme::weak_text(&vis);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
    let Some((xf, inner)) = xform(ui, rect, resp, ro, d, view) else {
        return;
    };
    view.surf_pos = d.vertices.iter().map(|v| v.map(|p| xf.to(p))).collect();
    view.xz = d.xz;
    let sc = scene(&painter, rect, inner, xf, ro, d, view, accent, weak);
    view.axes = sc.axes.clone();
    view.mids = sc.mids.clone();
    let h = hits(&sc, resp.interact_pointer_pos().or(resp.hover_pos()));
    if h.surfs.is_some() {
        view.hover_surfs = h.surfs;
    }
    // 選んだ機構部品（組合せの図だけ。欄の品番でも選べる）と、指した部品の名前・品番
    let mech_sel = combo
        .is_some()
        .then(|| super::mech::selected(ui.ctx()))
        .flatten();
    let pick = |n: &str| ro.mechanics.iter().find(|m| m.name == n);
    let mech_hover = h.mech.as_deref().map(|n| match pick(n) {
        Some(m) => format!("{n}  {}", m.item),
        None => n.to_string(),
    });
    let ms = (mech_sel.as_deref(), mech_hover.as_deref());
    paint(&painter, &vis, rect, &sc, &h, d, xf, accent, weak, ms);
    if let Some(m) = mech_sel.as_deref().and_then(pick) {
        mech_card(&painter, &vis, rect, &m.card(), accent);
    }
    // 縮尺（レイアウトのパネルの図。OpticStudio と同じく左下）
    if view.nav {
        crate::widgets::scale_bar::draw(
            &painter,
            rect.left_bottom() + egui::vec2(12.0, -6.0),
            Align2::LEFT_BOTTOM,
            xf.s,
            (rect.width() / 5.0).clamp(40.0, 160.0),
            weak,
        );
    }
    if view.nav {
        crate::widgets::paint_zoom_tools(ui, resp);
    }
    // 組合せを変える操作（寸法の書き換え・ドラッグで動かす）は組合せの図だけ
    interact(ui, resp, combo.as_deref_mut(), view, &h, xf.s);
    let editable = combo.is_some();
    if let Some(combo) = combo {
        dim_edit(ui, resp, combo, view, &sc.dims);
    }
    if h.el.is_some() || h.dim.is_some() || (h.lead && editable) || (h.mech.is_some() && editable) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

/// 図の座標 [z, y] から画面へ
#[derive(Debug, Clone, Copy)]
pub(super) struct Xform {
    center: Pos2,
    cz: f64,
    cy: f64,
    /// 図の 1 mm の画面の長さ [pt]
    pub(super) s: f64,
}

impl Xform {
    pub(super) fn to(&self, p: [f64; 2]) -> Pos2 {
        Pos2::new(
            self.center.x + ((p[0] - self.cz) * self.s) as f32,
            self.center.y - ((p[1] - self.cy) * self.s) as f32,
        )
    }
}

/// 図の範囲（寸法線の分、下に余白）から画面への写し方と、図を収める範囲。描くものが無ければ None。
/// レイアウトのパネルの図では、ここで拡大縮小・移動の操作も受ける
fn xform(
    ui: &mut Ui,
    rect: Rect,
    resp: &egui::Response,
    combo: &Combo,
    d: &LayoutData,
    view: &mut View,
) -> Option<(Xform, Rect)> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut grow = |p: &[f64; 2]| {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    };
    d.solids
        .iter()
        .flat_map(|s| s.pts.iter())
        .for_each(&mut grow);
    d.plates.iter().flat_map(|p| p.2.iter()).for_each(&mut grow);
    d.mechs
        .iter()
        .flat_map(|m| m.1.iter().flatten())
        .for_each(&mut grow);
    d.rays.iter().flat_map(|r| r.1.iter()).for_each(&mut grow);
    if !lo[0].is_finite() {
        return None;
    }
    let inner = rect.shrink2(egui::vec2(24.0, 16.0));
    // 寸法線の分（要素が無ければ要らない）
    let dim_room = if combo.is_empty() { 0.0 } else { 44.0 };
    let inner = Rect::from_min_max(inner.min, inner.max - egui::vec2(0.0, dim_room));
    let w = (hi[0] - lo[0]).max(1e-6);
    let h = (hi[1] - lo[1]).max(1e-6);
    let fit = (inner.width() as f64 / w).min(inner.height() as f64 / h);
    if view.nav {
        navigate(ui, inner, resp, view);
    }
    let xf = Xform {
        center: inner.center() + view.pan,
        cz: (lo[0] + hi[0]) / 2.0,
        cy: (lo[1] + hi[1]) / 2.0,
        s: fit * (view.zoom_log as f64).exp(),
    };
    Some((xf, inner))
}

/// BS（経路に沿った要素 k）の正方形と、分ける面
pub(super) struct Cube {
    pub(super) k: usize,
    pub(super) sq: Vec<Pos2>,
    diag: [Pos2; 2],
}

/// 玉の断面（BS のものは [`Cube`]）
pub(super) struct Body {
    /// 持ち主の要素（経路に沿った番号）
    pub(super) k: Option<usize>,
    pub(super) poly: Vec<Pos2>,
    mirror: bool,
    pub(super) surfaces: [usize; 2],
}

/// 要素の印: 名前と、開口・絞りの当たり
pub(super) struct Mark {
    pub(super) k: usize,
    kind: ElemKind,
    /// 要素の真ん中
    pub(super) pos: Pos2,
    /// 画面の上の光軸の向きと、名前を出す側（光軸に垂直）
    u: Vec2,
    n: Vec2,
    /// 名前を出す半径 [pt]
    r: f32,
    /// 開口・遮蔽が光を遮る区間（pos から n の向きの [pt]）
    ap: Vec<(f32, f32)>,
    /// 名前の文字と、その範囲
    pub(super) label: Rect,
    galley: Arc<Galley>,
    sel: bool,
}

/// 寸法線（要素 g の後の空気、[`STOP_GAP`] は絞りから最初の要素）
pub(super) struct Dim {
    pub(super) g: usize,
    sel: bool,
    /// 頂点から寸法線への細い線 2 本
    thin: [[Pos2; 2]; 2],
    /// 寸法線と、その始まりの向き（矢印は長さが 12 pt を超えるときだけ）
    pub(super) seg: [Pos2; 2],
    dir: Vec2,
    arrows: bool,
    /// 数字と、その範囲
    pub(super) text: Rect,
    galley: Arc<Galley>,
}

/// 画面の上の形
pub(super) struct Scene {
    pub(super) cubes: Vec<Cube>,
    pub(super) bodies: Vec<Body>,
    /// 面の表で選んだ面を持つ玉の縁（閉じた線）と、その面の頂点
    hl: Vec<Vec<Pos2>>,
    hl_vertex: Option<(Pos2, usize)>,
    /// 機構部品の名前と壁の断面
    pub(super) mechs: Vec<(String, Vec<Vec<Pos2>>)>,
    /// 絞りの面の中心・光軸に垂直な向き・半径 [pt]
    stop: Option<(Pos2, Vec2, f32)>,
    /// 絞りが先頭に自動で置く絞り（要素ではない。クリックで [`View::select_lead`]）と、それを選んでいるか
    lead: bool,
    lead_sel: bool,
    pub(super) marks: Vec<Mark>,
    pub(super) dims: Vec<Dim>,
    /// 選んだ要素（縁を太く）
    sel: Option<usize>,
    /// 要素ごとの画面の上の光軸の向き・真ん中（[`View::axes`]・[`View::mids`] へ）
    axes: Vec<Vec2>,
    mids: Vec<Option<Pos2>>,
}

/// 画面の上の形を求める（painter は文字の大きさを測るのにだけ使う）
#[allow(clippy::too_many_arguments)]
fn scene(
    painter: &Painter,
    rect: Rect,
    inner: Rect,
    xf: Xform,
    combo: &Combo,
    d: &LayoutData,
    view: &View,
    accent: Color32,
    weak: Color32,
) -> Scene {
    let to = |p: [f64; 2]| xf.to(p);
    let s = xf.s;
    // 名前・寸法を寄せる、図の見える範囲
    let view_area = rect.shrink(6.0);
    // 要素（今の経路に沿った並び）と玉の対応
    let ids = combo.span_ids();
    let crate::combo_tree::ChainView {
        steps: chain,
        picks: els,
    } = combo.chain_view();
    let spans = d.spans_for(&ids, combo.lead_stop());
    // 要素ごとの、画面の上での光軸の向き（矢印キー・ドラッグを画面の向きに合わせる）
    let vtx = |j: usize| d.vertices.get(j).copied().flatten().map(to);
    // 面 j へ入ってくる向き（前の面の頂点から）
    let incoming = |j: usize| {
        let a = vtx(j)?;
        (0..j).rev().find_map(|i| {
            let u = a - vtx(i)?;
            (u.length() > 1.0).then(|| u.normalized())
        })
    };
    let axes: Vec<Vec2> = spans
        .iter()
        .zip(&els)
        .map(|(sp, p)| {
            let sp = (*sp)?;
            // BS は入ってくる向き（反射側へ進むと、前後の面の差は光軸の向きにならない）
            if p.kind == ElemKind::Splitter {
                return incoming(sp[0]);
            }
            let (a, b) = (vtx(sp[0])?, vtx(sp[1])?);
            let u = b - a;
            if u.length() > 1.0 {
                return Some(u.normalized());
            }
            // 面が 1 枚（鏡・絞り）: 入ってくる向き
            incoming(sp[0])
        })
        .map(|u| u.unwrap_or(Vec2::X))
        .collect();
    let owner = |sd: &Solid| {
        spans
            .iter()
            .position(|sp| sp.is_some_and(|sp| sp[0] <= sd.surfaces[0] && sd.surfaces[0] <= sp[1]))
    };
    // BS: serve の断面（キューブの半分の直角プリズム = 直角三角形）から正方形と分ける面を作る。
    // 反射側へ進むと、通るのは片方の半分だけなので、もう半分は直角の頂点を斜辺の中点で折り返して補う
    let is_bs = |k: Option<usize>| k.is_some_and(|k| els[k].kind == ElemKind::Splitter);
    let mut cubes = Vec::new();
    let mut bs_center: Vec<Option<Pos2>> = vec![None; els.len()];
    for (k, p) in els.iter().enumerate() {
        if p.kind != ElemKind::Splitter {
            continue;
        }
        let Some([sq, diag]) = d
            .solids
            .iter()
            .filter(|sd| owner(sd) == Some(k))
            .find_map(|sd| cube_of_prism(&sd.pts))
        else {
            continue;
        };
        let sq: Vec<Pos2> = sq.iter().map(|&q| to(q)).collect();
        let diag = [to(diag[0]), to(diag[1])];
        bs_center[k] = Some(diag[0] + (diag[1] - diag[0]) / 2.0);
        cubes.push(Cube { k, sq, diag });
    }
    let bodies = d
        .solids
        .iter()
        .filter_map(|sd| {
            let k = owner(sd);
            (!is_bs(k)).then(|| Body {
                k,
                poly: sd.pts.iter().map(|&q| to(q)).collect(),
                mirror: sd.mirror,
                surfaces: sd.surfaces,
            })
        })
        .collect();
    let closed = |pts: &[[f64; 2]]| {
        let mut v: Vec<Pos2> = pts.iter().map(|&q| to(q)).collect();
        if let Some(f) = v.first().copied() {
            v.push(f);
        }
        v
    };
    let (hl, hl_vertex) = match view.hl_surf {
        Some(j) => (
            d.solids
                .iter()
                .filter(|sd| sd.surfaces[0] <= j && j <= sd.surfaces[1])
                .map(|sd| closed(&sd.pts))
                .collect(),
            d.vertices.get(j).copied().flatten().map(|v| (to(v), j)),
        ),
        None => (Vec::new(), None),
    };
    let mechs = d
        .mechs
        .iter()
        .map(|(name, quads)| {
            let polys = quads
                .iter()
                .map(|q| q.iter().map(|&p| to(p)).collect())
                .collect();
            (name.clone(), polys)
        })
        .collect();
    // 絞り（面の上下に短い線）。絞りの要素なら、その光軸に垂直に
    let stop = d.stop.and_then(|j| {
        let c = vtx(j)?;
        let r = (d.stop_semi.unwrap_or(combo.epd / 2.0) * s) as f32;
        let u = spans
            .iter()
            .position(|sp| sp.is_some_and(|sp| sp[0] <= j && j <= sp[1]))
            .and_then(|k| axes.get(k).copied())
            .unwrap_or(Vec2::X);
        Some((c, Vec2::new(u.y, -u.x), r))
    });
    // 要素の名前（鏡・絞り・BS など玉で描かれない要素は頂点の位置に）
    let mut mids = vec![None; els.len()];
    let mut marks = Vec::new();
    for (k, p) in els.iter().enumerate() {
        let Some(sp) = spans[k] else { continue };
        let Some(front) = vtx(sp[0]) else {
            continue;
        };
        let u = axes.get(k).copied().unwrap_or(Vec2::X);
        // 名前は要素の真ん中から
        let pos = if let Some(c) = bs_center[k] {
            // BS は立方体の中心（反射側へ進むと、出る面は光軸の先にない）
            c
        } else if p.kind == ElemKind::Splitter {
            front + u * (p.bs().size * s) as f32 / 2.0
        } else {
            vtx(sp[1]).map_or(front, |b| front + (b - front) / 2.0)
        };
        mids[k] = Some(pos);
        let sel = view.sel == Some(k);
        let r = (if p.kind == ElemKind::Aperture {
            p.ap_extent() + 14.0 / s
        } else {
            p.semi
        }
        .max(1.0)
            * s) as f32;
        // 名前は光軸に垂直に、要素の外へ（折り曲げなしなら上。鏡は折れた先と重ならないよう反対側）
        let n = Vec2::new(u.y, -u.x)
            * if p.kind == ElemKind::Mirror {
                -1.0
            } else {
                1.0
            };
        let ap = if p.kind == ElemKind::Aperture {
            let iv = if d.xz { 0 } else { 1 };
            ap_segments(p, iv, s)
                .iter()
                .map(|(a, b)| ((a * s) as f32, (b * s) as f32))
                .collect()
        } else {
            Vec::new()
        };
        let across_y = n.y.abs() >= n.x.abs();
        let (label_pos, align) = if across_y {
            let y = if n.y < 0.0 {
                pos.y - r - 10.0
            } else {
                pos.y + r + 10.0
            };
            (
                Pos2::new(pos.x, y),
                if n.y < 0.0 {
                    Align2::CENTER_BOTTOM
                } else {
                    Align2::CENTER_TOP
                },
            )
        } else if n.x < 0.0 {
            (Pos2::new(pos.x - r - 10.0, pos.y), Align2::RIGHT_CENTER)
        } else {
            (Pos2::new(pos.x + r + 10.0, pos.y), Align2::LEFT_CENTER)
        };
        let text = if p.locked {
            format!("{} [{}]", p.label(), tr("Lock"))
        } else {
            p.label()
        };
        let galley = painter.layout_no_wrap(
            text,
            FontId::proportional(11.0),
            if sel { accent } else { weak },
        );
        // 要素が少しでも見えていれば、光軸に垂直な向きに図の中へ寄せる（要素がはみ出ても名前は見える。
        // 光軸の向きは寄せない）。要素が全部図の外なら寄せない
        let label = align.anchor_size(label_pos, galley.size());
        let label = if Rect::from_center_size(pos, Vec2::splat(2.0 * r)).intersects(rect) {
            keep_in(label, view_area, if across_y { 1 } else { 0 })
        } else {
            label
        };
        marks.push(Mark {
            k,
            kind: p.kind,
            pos,
            u,
            n,
            r,
            ap,
            label,
            galley,
            sel,
        });
    }
    // 寸法線（要素の後の空気。最後は像面まで）
    let mut gaps: Vec<(usize, Pos2, Pos2)> = Vec::new();
    if combo.lead_stop() {
        if let (Some(j), Some(sp)) = (d.stop, spans.first().copied().flatten()) {
            if let (Some(a), Some(b)) = (vtx(j), vtx(sp[0])) {
                gaps.push((STOP_GAP, a, b));
            }
        }
    }
    for k in 0..els.len() {
        let Some(sp) = spans[k] else { continue };
        // 後の間隔の無い要素（往路を戻る鏡）・後の要素の前の間隔が別のもの（戻って口 D へ）は描かない
        if chain[k].after.is_none() || chain.get(k + 1).is_some_and(|n| n.before != chain[k].after)
        {
            continue;
        }
        let a = d.vertices.get(sp[1]).copied().flatten();
        let b = if k + 1 < els.len() {
            spans[k + 1].and_then(|n| d.vertices.get(n[0]).copied().flatten())
        } else {
            d.image_vertex()
        };
        if let (Some(a), Some(b)) = (a, b) {
            gaps.push((k, to(a), to(b)));
        }
    }
    let dim_y = inner.bottom() + 26.0;
    // 縦の寸法線を光軸からどれだけ離すか（要素の半径の最大 + 余白）
    let vr = (els.iter().map(|p| p.semi).fold(combo.epd / 2.0, f64::max) * s) as f32 + 24.0;
    // 寸法線は、間隔の両側の要素（光軸の区間を要素の半径の最大だけ広げた所）が少しでも見えている間だけ描く
    let dims = gaps
        .iter()
        .filter(|&&(_, a, b)| Rect::from_two_pos(a, b).expand(vr).intersects(rect))
        .map(|&(g, a, b)| {
            // ソルブ・ピックアップで決まる寸法は系の今の値に「S」・「P」（レンズデータの表の印と同じ）
            let solved = super::SolvedGap::find(&view.solved, g);
            let val = solved.map_or_else(|| gap_get(combo, g).unwrap_or(0.0), |x| x.value);
            let sel = view.dim == Some(g);
            let col = if sel { accent } else { weak };
            let num = fmt_num((val * 1e3).round() / 1e3);
            let galley = painter.layout_no_wrap(
                match solved {
                    Some(x) => format!("{num} {}", x.mark),
                    None => num,
                },
                FontId::monospace(11.0),
                col,
            );
            let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
            let (thin, seg, dir, arrows, at, align) = if dy < 1.0 || (dx > 1.0 && dy / dx < 0.02) {
                // 横の間隔: 図の下にそろえる
                let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
                (
                    [
                        [a, Pos2::new(a.x, dim_y + 4.0)],
                        [b, Pos2::new(b.x, dim_y + 4.0)],
                    ],
                    [Pos2::new(x0, dim_y), Pos2::new(x1, dim_y)],
                    Vec2::X,
                    x1 - x0 > 12.0,
                    Pos2::new(
                        along_in(x0, x1, view_area.x_range(), galley.size().x),
                        dim_y - 3.0,
                    ),
                    Align2::CENTER_BOTTOM,
                )
            } else if dx < 1.0 || dx / dy < 0.02 {
                // 縦の間隔（折り曲げの後）: 光軸の右に
                // 横の寸法線を図の下にそろえるのと同じく、光軸がはみ出ても寸法線は図の中へ寄せる
                let x = (a.x.max(b.x) + vr)
                    .min(view_area.right() - galley.size().x - 5.0)
                    .max(view_area.left());
                let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
                (
                    [[a, Pos2::new(x + 4.0, a.y)], [b, Pos2::new(x + 4.0, b.y)]],
                    [Pos2::new(x, y0), Pos2::new(x, y1)],
                    Vec2::Y,
                    y1 - y0 > 12.0,
                    Pos2::new(
                        x + 5.0,
                        along_in(y0, y1, view_area.y_range(), galley.size().y),
                    ),
                    Align2::LEFT_CENTER,
                )
            } else {
                // 斜めの間隔: 光軸に平行に、右側へずらす（横・縦への射影ではなく、光軸に沿った長さ）
                let u = (b - a).normalized();
                let mut n = Vec2::new(-u.y, u.x);
                if n.x < 0.0 {
                    n = -n;
                }
                // 縦の間隔と同じく、ずらす量を図の中に入るよう詰める（文字の分は右を空ける）
                let mut area = view_area;
                area.max.x -= galley.size().x + 5.0;
                let t = fit_along(a + (b - a) / 2.0, n, vr, area);
                let (a2, b2) = (a + n * t, b + n * t);
                (
                    [[a, a2 + n * 4.0], [b, b2 + n * 4.0]],
                    [a2, b2],
                    u,
                    (b2 - a2).length() > 12.0,
                    a2 + (b2 - a2) / 2.0 + n * 5.0,
                    Align2::LEFT_CENTER,
                )
            };
            Dim {
                g,
                sel,
                thin,
                seg,
                dir,
                arrows,
                text: align.anchor_size(at, galley.size()),
                galley,
            }
        })
        .collect();
    Scene {
        cubes,
        bodies,
        hl,
        hl_vertex,
        mechs,
        stop,
        lead: combo.lead_stop(),
        lead_sel: combo.lead_stop() && view.lead(),
        marks,
        dims,
        sel: view.sel,
        axes,
        mids,
    }
}

/// 矩形 r を、軸 axis（0 = 横、1 = 縦）の向きだけ area の中へずらす（area より大きければ area の始まりにそろえる）
pub(super) fn keep_in(r: Rect, area: Rect, axis: usize) -> Rect {
    let d = (area.max[axis] - r.max[axis]).min(0.0) + (area.min[axis] - r.min[axis]).max(0.0);
    let d = if r.max[axis] - r.min[axis] > area.max[axis] - area.min[axis] {
        area.min[axis] - r.min[axis]
    } else {
        d
    };
    let mut v = Vec2::ZERO;
    v[axis] = d;
    r.translate(v)
}

/// 寸法の数字の位置（区間 [lo, hi] の真ん中）を、見える範囲 vis に入る区間の部分へ寄せる。
/// w は数字のその向きの大きさ。区間が見えなければ真ん中のまま
pub(super) fn along_in(lo: f32, hi: f32, vis: egui::Rangef, w: f32) -> f32 {
    let c = (lo + hi) / 2.0;
    let (a, b) = (lo.max(vis.min + w / 2.0), hi.min(vis.max - w / 2.0));
    if a <= b {
        c.clamp(a, b)
    } else {
        c
    }
}

/// 点 m から向き n へ t だけ離した所が area に入るよう t を詰める。入る t が無ければそのまま
pub(super) fn fit_along(m: Pos2, n: Vec2, t: f32, area: Rect) -> f32 {
    let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
    for ax in 0..2 {
        if n[ax].abs() < 1e-6 {
            if m[ax] < area.min[ax] || m[ax] > area.max[ax] {
                return t;
            }
            continue;
        }
        let (p, q) = (
            (area.min[ax] - m[ax]) / n[ax],
            (area.max[ax] - m[ax]) / n[ax],
        );
        lo = lo.max(p.min(q));
        hi = hi.min(p.max(q));
    }
    if lo <= hi {
        t.clamp(lo, hi)
    } else {
        t
    }
}

/// 開口・遮蔽が断面（縦 iv: 0 = x, 1 = y）で光を遮る区間 [mm]（要素の真ん中から縦の向き）。
/// 開口は縁から外へ 14 pt（s は図の 1 mm の画面の長さ）
fn ap_segments(p: &Pick, iv: usize, s: f64) -> Vec<(f64, f64)> {
    let dy = p.decenter[iv];
    match p.ap().shape {
        ApShape::Obscuration => {
            let (lo, hi) = (p.ap().min.max(0.0), p.semi);
            if lo <= 0.0 {
                vec![(dy - hi, dy + hi)]
            } else {
                vec![(dy + lo, dy + hi), (dy - hi, dy - lo)]
            }
        }
        ApShape::Spider => {
            // 中心の腕の幅と、断面の向きに沿う腕（半径 semi まで）
            let hw = p.ap().spider_width.max(0.0) / 2.0;
            let mut v = vec![(dy - hw, dy + hw)];
            let n = p.ap().spider_arms.max(1);
            for k in 0..n {
                let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
                let (c, s) = (a.cos(), a.sin());
                let (along, across) = if iv == 0 { (c, s) } else { (s, c) };
                if across.abs() < 1e-9 {
                    v.push(if along > 0.0 {
                        (dy, dy + p.semi)
                    } else {
                        (dy - p.semi, dy)
                    });
                }
            }
            v
        }
        _ => {
            let h = match p.ap().shape {
                ApShape::Circle => p.semi,
                _ => p.ap().half[iv],
            };
            let edge = 14.0 / s;
            let mut v = vec![(dy + h, dy + h + edge), (dy - h - edge, dy - h)];
            // 輪帯（内径）は真ん中を遮る
            if p.ap().shape == ApShape::Circle && p.ap().min > 0.0 {
                v.push((dy - p.ap().min, dy + p.ap().min));
            }
            v
        }
    }
}

/// ポインタの下にあるもの
#[derive(Debug, Default, PartialEq)]
pub(super) struct Hits {
    /// 要素（後に描くものほど上: BS → 玉 → 開口・絞り・名前）
    pub(super) el: Option<usize>,
    /// 先頭に自動で置く絞り（要素ではない）の面の線の近く
    pub(super) lead: bool,
    /// 寸法線（数字の近くか、線から 5 pt 以内）
    pub(super) dim: Option<usize>,
    /// 寸法の数字の上
    pub(super) num: Option<usize>,
    /// 玉の面の範囲（開いた系の図のクリックで面の表の行を選ぶ）
    pub(super) surfs: Option<[usize; 2]>,
    /// 指した機構部品の壁（[`Scene::mechs`] の番号と壁の番号）。名前を出す
    pub(super) mechs: Vec<(usize, usize)>,
    /// 一番上の指した機構部品の名前（クリックで選ぶ）
    pub(super) mech: Option<String>,
}

/// ポインタ q の下にあるものを、描く順に後のものを上として決める
pub(super) fn hits(sc: &Scene, q: Option<Pos2>) -> Hits {
    let mut h = Hits::default();
    let Some(q) = q else {
        return h;
    };
    for c in &sc.cubes {
        if inside(&c.sq, q) {
            h.el = Some(c.k);
        }
    }
    for b in &sc.bodies {
        if inside(&b.poly, q) {
            h.surfs = Some(b.surfaces);
            if let Some(k) = b.k {
                h.el = Some(k);
            }
        }
    }
    // 先頭に自動で置く絞り: 要素の絞りと同じく、面の線（光軸に垂直）の近く（厚さが無いので玉の中の判定では選べない）
    if let (true, Some((c, n, r))) = (sc.lead, sc.stop) {
        let w = q - c;
        let u = Vec2::new(-n.y, n.x);
        if w.dot(u).abs() < 6.0 && w.dot(n).abs() < r + 14.0 {
            h.lead = true;
            // 面の線の上（3 pt 以内）なら、重なる BS・玉より絞り（間隔 0 で最初の要素の平らな前面に重なるとき）
            if w.dot(u).abs() < 3.0 {
                h.el = None;
            }
        }
    }
    for (i, polys) in sc.mechs.iter().map(|m| &m.1).enumerate() {
        for (j, poly) in polys.iter().enumerate() {
            if inside(poly, q) {
                h.mechs.push((i, j));
                h.mech = Some(sc.mechs[i].0.clone());
            }
        }
    }
    for m in &sc.marks {
        let w = q - m.pos;
        // 開口・遮蔽: 光を遮る太線の近く
        if m.kind == ElemKind::Aperture {
            let t = w.dot(m.n);
            if w.dot(m.u).abs() < 6.0 && m.ap.iter().any(|(a, b)| t >= a - 4.0 && t <= b + 4.0) {
                h.el = Some(m.k);
            }
        }
        // 絞りは面の線（光軸に垂直）の近くでも選べる
        if m.kind == ElemKind::Stop && w.dot(m.u).abs() < 6.0 && w.dot(m.n).abs() < m.r + 14.0 {
            h.el = Some(m.k);
        }
        if m.label.expand(3.0).contains(q) {
            h.el = Some(m.k);
        }
    }
    for dm in &sc.dims {
        if dm.text.expand(4.0).contains(q) {
            h.num = Some(dm.g);
        }
        // 数字の近くか、寸法線から 5 pt 以内
        let (p0, p1) = (dm.seg[0], dm.seg[1]);
        let w = p1 - p0;
        let t = ((q - p0).dot(w) / w.length_sq().max(1e-6)).clamp(0.0, 1.0);
        if dm.text.expand(3.0).contains(q) || (q - (p0 + w * t)).length() <= 5.0 {
            h.dim = Some(dm.g);
        }
    }
    h
}

/// 描く
#[allow(clippy::too_many_arguments)]
fn paint(
    painter: &Painter,
    vis: &egui::Visuals,
    rect: Rect,
    sc: &Scene,
    h: &Hits,
    d: &LayoutData,
    xf: Xform,
    accent: Color32,
    weak: Color32,
    (mech_sel, mech_hover): (Option<&str>, Option<&str>),
) {
    // 光軸
    let axis_y = xf.to([0.0, 0.0]).y;
    if rect.y_range().contains(axis_y) {
        painter.add(egui::Shape::dashed_line(
            &[
                Pos2::new(rect.left() + 8.0, axis_y),
                Pos2::new(rect.right() - 8.0, axis_y),
            ],
            Stroke::new(1.0_f32, vis.widgets.noninteractive.bg_stroke.color),
            6.0,
            4.0,
        ));
    }
    // 光線（視野ごとの色）
    for (f, p) in &d.rays {
        let line: Vec<Pos2> = p.iter().map(|&q| xf.to(q)).collect();
        // 機構部品に切られた後ろ（部品で切らない設定のとき）は薄く
        let (f, faint) = crate::layout_rays::class_of(*f);
        let a = if faint { 0.25 } else { 0.85 };
        painter.add(egui::Shape::line(
            line,
            Stroke::new(
                1.0_f32,
                theme::series(f.saturating_sub(1)).gamma_multiply(a),
            ),
        ));
    }
    // 構成を重ねたときは構成の名前と色（左上）
    if d.configs.len() > 1 {
        let font = FontId::proportional(11.0);
        for (k, name) in d.configs.iter().enumerate() {
            let p = rect.left_top() + egui::vec2(10.0, 10.0 + 15.0 * k as f32);
            let c = theme::series(k);
            painter.line_segment(
                [p + egui::vec2(0.0, 6.0), p + egui::vec2(14.0, 6.0)],
                (2.0, c),
            );
            painter.text(
                p + egui::vec2(18.0, 0.0),
                Align2::LEFT_TOP,
                name,
                font.clone(),
                weak,
            );
        }
    }
    let glass_fill = theme::glass_fill(vis);
    let mirror_fill = theme::mirror_fill(vis);
    let outline = |selected: bool| {
        if selected {
            Stroke::new(2.2_f32, accent)
        } else {
            Stroke::new(1.0_f32, accent.gamma_multiply(0.75))
        }
    };
    let sel = |k: Option<usize>| k.is_some() && k == sc.sel;
    for c in &sc.cubes {
        painter.add(egui::Shape::convex_polygon(
            c.sq.clone(),
            glass_fill,
            outline(sel(Some(c.k))),
        ));
        painter.line_segment(c.diag, Stroke::new(1.5_f32, mirror_fill));
    }
    for b in &sc.bodies {
        // 凹面のある玉（両凹・メニスカス）の断面は凸でないので、三角形に分けて塗る
        painter.add(poly_fill::filled(
            &b.poly,
            if b.mirror { mirror_fill } else { glass_fill },
        ));
        let mut closed = b.poly.clone();
        if let Some(f) = b.poly.first() {
            closed.push(*f);
        }
        painter.add(egui::Shape::line(closed, outline(sel(b.k))));
    }
    // 面の表で選んだ面: その面を持つ玉の縁と、面の頂点の印
    let hl = theme::highlight(vis);
    for line in &sc.hl {
        painter.add(egui::Shape::line(line.clone(), Stroke::new(2.0_f32, hl)));
    }
    if let Some((c, j)) = sc.hl_vertex {
        painter.circle_stroke(c, 4.0, Stroke::new(2.0_f32, hl));
        painter.text(
            c + egui::vec2(5.0, -5.0),
            Align2::LEFT_BOTTOM,
            format!("S{j}"),
            FontId::proportional(11.0),
            hl,
        );
    }
    // 機構部品: 壁の断面。選んだ部品は輪郭を目立たせ、指した壁には名前と品番
    let mech_fill = theme::mech_fill(vis);
    let mech_line = theme::mech_line(vis);
    for (i, (name, polys)) in sc.mechs.iter().enumerate() {
        let stroke = if mech_sel == Some(name.as_str()) {
            Stroke::new(2.2_f32, accent)
        } else {
            Stroke::new(1.0_f32, mech_line)
        };
        for (j, poly) in polys.iter().enumerate() {
            painter.add(egui::Shape::convex_polygon(poly.clone(), mech_fill, stroke));
            if h.mechs.last() == Some(&(i, j)) {
                if let Some(c) = poly.first() {
                    painter.text(
                        *c,
                        Align2::LEFT_BOTTOM,
                        mech_hover.unwrap_or(name),
                        FontId::proportional(11.0),
                        mech_line,
                    );
                }
            }
        }
    }
    // 像面・物体面
    for (kind, _, p) in &d.plates {
        let line: Vec<Pos2> = p.iter().map(|&q| xf.to(q)).collect();
        let c = if kind == "image" {
            vis.strong_text_color()
        } else {
            weak
        };
        painter.add(egui::Shape::line(line, Stroke::new(1.5_f32, c)));
    }
    // 絞り（面の上下に短い線）
    if let Some((c, n, r)) = sc.stop {
        let (w, col) = if sc.lead_sel {
            (3.0_f32, accent)
        } else {
            (2.0, weak)
        };
        for sgn in [-1.0f32, 1.0] {
            painter.line_segment(
                [c + n * sgn * r, c + n * sgn * (r + 14.0)],
                Stroke::new(w, col),
            );
        }
        if sc.lead_sel {
            painter.circle_stroke(c, 6.0, Stroke::new(2.0_f32, accent));
        }
    }
    // 要素: 開口・遮蔽の光を遮る所の太線、名前、選んだ鏡・絞り・開口の丸
    for m in &sc.marks {
        let col = if m.sel { accent } else { weak };
        for (a, b) in &m.ap {
            painter.line_segment(
                [m.pos + m.n * *a, m.pos + m.n * *b],
                Stroke::new(if m.sel { 3.0_f32 } else { 2.5 }, col),
            );
        }
        painter.galley(m.label.min, m.galley.clone(), col);
        if matches!(
            m.kind,
            ElemKind::Stop | ElemKind::Mirror | ElemKind::Aperture
        ) && m.sel
        {
            painter.circle_stroke(m.pos, 6.0, Stroke::new(2.0_f32, accent));
        }
    }
    // 寸法線
    for dm in &sc.dims {
        let col = if dm.sel { accent } else { weak };
        let st = Stroke::new(if dm.sel { 1.5_f32 } else { 1.0 }, col);
        let thin = Stroke::new(0.5_f32, col.gamma_multiply(0.5));
        let arrow = |tip: Pos2, dir: Vec2| {
            let side = Vec2::new(-dir.y, dir.x) * 3.0;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip + dir * 6.0 + side, tip + dir * 6.0 - side],
                col,
                Stroke::NONE,
            ));
        };
        painter.line_segment(dm.thin[0], thin);
        painter.line_segment(dm.thin[1], thin);
        painter.line_segment(dm.seg, st);
        if dm.arrows {
            arrow(dm.seg[0], dm.dir);
            arrow(dm.seg[1], -dm.dir);
        }
        painter.galley(dm.text.min, dm.galley.clone(), col);
    }
}

/// クリック・ドラッグ（要素を選ぶ・開く・光軸に沿って動かす・Shift で偏心、寸法を選ぶ・書き換えを始める）。
/// s は図の 1 mm の画面の長さ。combo が無ければ（描くだけの図）選ぶ・開く・全体表示だけ
fn interact(
    ui: &mut Ui,
    resp: &egui::Response,
    mut combo: Option<&mut Combo>,
    view: &mut View,
    h: &Hits,
    s: f64,
) {
    if resp.double_clicked() {
        if let (Some(g), Some(combo)) = (h.dim, combo.as_deref_mut()) {
            // 寸法線のダブルクリックでも数字を書き換える
            view.start_edit(combo, g);
        } else if let Some(k) = h.el {
            // 要素のダブルクリックは、その要素の面を面の表で
            view.select(k);
            view.open = Some(k);
        } else if view.nav {
            // 何も無い所のダブルクリックは全体表示
            view.zoom_log = 0.0;
            view.pan = Vec2::ZERO;
        }
    } else if resp.clicked() {
        if let (Some(g), Some(combo)) = (h.num, combo.as_deref_mut()) {
            // 数字のクリックはその場で書き換える（今の数字は全選択で、打てば置き換わる）
            view.start_edit(combo, g);
        } else if let Some(g) = h.dim {
            // 寸法線（数字の外）のクリックは選ぶだけ（スライダー・← →）
            view.dim = Some(g);
            view.set_sel(None);
        } else if h.el.is_none() && h.lead && combo.is_some() {
            view.select_lead();
        } else if let (None, Some(n), true) = (h.el, h.mech.as_deref(), combo.is_some()) {
            // 要素の無い所の機構部品の壁: その部品を選ぶ（説明を図の右下に、欄のその行まで送る）
            super::mech::select(ui.ctx(), Some(n));
            ui.data_mut(|d| d.insert_temp(super::mech::scroll_id(), true));
            view.set_sel(None);
            view.dim = None;
        } else {
            view.set_sel(h.el);
            view.dim = None;
        }
        if h.mech.is_none() || h.el.is_some() {
            super::mech::select(ui.ctx(), None);
        }
    }
    // 移動・囲んで拡大のドラッグは要素を選ぶ・動かす役をしない
    let pan_drag = view.nav && (panning(ui, resp) || crate::widgets::box_zooming(ui, resp));
    // 寸法の数字（Alt なら寸法線も）のドラッグはその間隔を増減する（表のセルの Alt+ドラッグと同じ量）
    let dd = egui::Id::unique("lensbench-dim-drag");
    let dim_hit = h.num.or(h.dim.filter(|_| ui.input(|i| i.modifiers.alt)));
    if let (true, false, Some(g), Some(c)) = (
        resp.drag_started_by(egui::PointerButton::Primary),
        pan_drag,
        dim_hit,
        combo.as_deref(),
    ) {
        if super::SolvedGap::find(&view.solved, g).is_some() {
            // ソルブで決まる寸法はドラッグで変えない（選んで理由を出す）
            view.dim = Some(g);
            view.set_sel(None);
        } else if let Some(v) = super::gap_get(c, g) {
            ui.data_mut(|d| d.insert_temp(dd, (g, v, 0.0_f32)));
            view.dim = Some(g);
            view.set_sel(None);
        }
    }
    if let (Some((g, v0, dx)), Some(c)) = (
        ui.data(|d| d.get_temp::<(usize, f64, f32)>(dd)),
        combo.as_deref_mut(),
    ) {
        if resp.drag_stopped() || !resp.dragged() {
            ui.data_mut(|d| d.remove::<(usize, f64, f32)>(dd));
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            let m = resp.drag_delta();
            let dx = dx
                + (m.x - m.y)
                    * if ui.input(|i| i.modifiers.shift) {
                        0.1
                    } else {
                        1.0
                    };
            ui.data_mut(|d| d.insert_temp(dd, (g, v0, dx)));
            if let Some(x) = super::gap_mut(c, &view.solved, g) {
                *x = crate::widgets::num::drag_value(v0, dx, false).max(0.0);
            }
        }
        return;
    }
    if resp.drag_started_by(egui::PointerButton::Primary) && !pan_drag {
        if let Some(k) = h.el {
            view.set_sel(Some(k));
            view.dim = None;
        }
        if let (Some(p), Some(combo)) = (resp.interact_pointer_pos(), combo.as_deref()) {
            view.drag0 = Some((p, combo.clone()));
        }
    }
    let Some(combo) = combo else {
        return;
    };
    if resp.dragged_by(egui::PointerButton::Primary) && !pan_drag {
        if let (Some(k), Some((p0, c0)), Some(p)) = (
            view.sel
                .filter(|&k| combo.el(k).is_some_and(|p| !p.fixed())),
            view.drag0.as_ref(),
            resp.interact_pointer_pos(),
        ) {
            // 画面の動きを、要素の光軸に沿う量と垂直な量に
            let (a, n) = view.along(k, p - *p0);
            let dz = (a as f64 / s * 100.0).round() / 100.0;
            let dy = (n as f64 / s * 100.0).round() / 100.0;
            let mut c = c0.clone();
            if ui.input(|i| i.modifiers.shift) {
                // Shift: 縦は偏心
                let st = c.active_state().map(str::to_string);
                if let Some(p) = c
                    .el_mut(k)
                    .filter(|p| !matches!(p.kind, ElemKind::Mirror | ElemKind::Stop))
                {
                    view.nudge(p, st.as_deref(), dy, 0.0);
                }
            } else if !view.before_solved(k) {
                c.shift(k, dz);
            }
            *combo = c;
        }
    }
    if resp.drag_stopped() {
        view.drag0 = None;
    }
}

/// 寸法の数字のその場の入力欄（Enter・Tab・外をクリックで決める、Esc でやめる）
fn dim_edit(ui: &mut Ui, resp: &egui::Response, combo: &mut Combo, view: &mut View, dims: &[Dim]) {
    let mut done = None;
    if let Some((g, text, focused)) = view.edit.as_mut() {
        if let Some(r) = dims.iter().find(|dm| dm.g == *g).map(|dm| dm.text) {
            let r =
                Rect::from_center_size(r.center(), egui::vec2(r.width().max(40.0) + 36.0, 20.0));
            let te_id = egui::Id::unique(("lensbench-dim-edit", *g));
            let te = ui.put(
                r,
                egui::TextEdit::singleline(text)
                    .id(te_id)
                    .font(FontId::monospace(12.0))
                    .align(egui::Align2::CENTER_TOP)
                    .margin(egui::Margin::symmetric(4, 2)),
            );
            if !*focused || te.gained_focus() {
                // 全選択（打てば今の数字が置き換わる。表のセルと同じ）。打ち込みで始めたら後ろにカーソル
                let typed = ui.data_mut(|d| d.remove_temp::<bool>(te_id.with("typed")));
                crate::widgets::cell::start(ui, &te, text, typed != Some(true));
                *focused = true;
            } else if te.lost_focus() {
                let (esc, enter) =
                    ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
                let v = text
                    .trim()
                    .replace(',', ".")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                let err_id = te_id.with("err");
                if !esc && v.is_none() {
                    // 数でない・負の値は黙って捨てず、欄を残して赤枠にする（Esc でやめる）
                    ui.data_mut(|d| d.insert_temp(err_id, true));
                    *focused = false;
                } else {
                    ui.data_mut(|d| d.remove::<bool>(err_id));
                    // Enter・Esc で終えたら図にフォーカスを戻す（寸法は選んだまま、← → が効く）
                    if esc || enter {
                        resp.request_focus();
                    }
                    done = Some((*g, if esc { None } else { v }));
                }
            }
            if ui
                .data(|d| d.get_temp::<bool>(te_id.with("err")))
                .unwrap_or(false)
            {
                ui.painter().rect_stroke(
                    te.rect.expand(1.0),
                    2.0,
                    egui::Stroke::new(1.5_f32, ui.visuals().error_fg_color),
                    egui::StrokeKind::Outside,
                );
                te.on_hover_text(tr("A number ≥ 0 (Esc to cancel)"));
            }
        } else {
            done = Some((*g, None));
        }
    }
    if let Some((g, v)) = done {
        view.edit = None;
        if let Some(v) = v.filter(|v| v.is_finite() && *v >= 0.0) {
            if let Some(x) = gap_mut(combo, &view.solved, g) {
                *x = v;
            }
        }
    }
}

/// キューブの半分（直角三角形の断面 [z, y]）から、キューブの正方形（4 隅）と分ける面（斜辺）
pub(super) fn cube_of_prism(pts: &[[f64; 2]]) -> Option<[Vec<[f64; 2]>; 2]> {
    // 重なった点（断面の端で同じ角が 2 度出ることがある）は 1 つに
    let mut q: Vec<[f64; 2]> = Vec::new();
    for p in pts {
        let tol = 1e-6 * (1.0 + p[0].abs() + p[1].abs());
        if !q
            .iter()
            .any(|r| (r[0] - p[0]).abs() < tol && (r[1] - p[1]).abs() < tol)
        {
            q.push(*p);
        }
    }
    let pts = &q[..];
    if pts.len() != 3 {
        return None;
    }
    let sub = |a: [f64; 2], b: [f64; 2]| [a[0] - b[0], a[1] - b[1]];
    let len = |v: [f64; 2]| v[0].hypot(v[1]);
    // 直角の頂点: 残りの 2 点への向きの内積がいちばん小さい
    let (i, cosv) = (0..3)
        .map(|i| {
            let (c, a, b) = (pts[i], pts[(i + 1) % 3], pts[(i + 2) % 3]);
            let (u, v) = (sub(a, c), sub(b, c));
            (
                i,
                (u[0] * v[0] + u[1] * v[1]) / (len(u) * len(v)).max(1e-300),
            )
        })
        .min_by(|x, y| x.1.abs().total_cmp(&y.1.abs()))?;
    if cosv.abs() > 1e-3 {
        return None;
    }
    let (c, a, b) = (pts[i], pts[(i + 1) % 3], pts[(i + 2) % 3]);
    let o = [a[0] + b[0] - c[0], a[1] + b[1] - c[1]];
    Some([vec![a, c, b, o], vec![a, b]])
}

/// 選んだ機構部品の説明（図の右下の札。1 行目は名前と品番）
fn mech_card(
    painter: &Painter,
    vis: &egui::Visuals,
    rect: Rect,
    lines: &[String],
    accent: Color32,
) {
    let wrap = (rect.width() * 0.5).clamp(160.0, 420.0);
    let galleys: Vec<Arc<Galley>> = lines
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (font, col) = if i == 0 {
                (FontId::proportional(13.0), vis.strong_text_color())
            } else {
                (FontId::proportional(11.5), vis.text_color())
            };
            painter.layout(t.clone(), font, col, wrap)
        })
        .collect();
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let h: f32 = galleys.iter().map(|g| g.size().y + 2.0).sum();
    let pad = 8.0;
    let r = Rect::from_min_size(
        rect.right_bottom() - egui::vec2(w + 2.0 * pad + 10.0, h + 2.0 * pad + 10.0),
        egui::vec2(w + 2.0 * pad, h + 2.0 * pad),
    );
    painter.rect(
        r,
        6.0,
        vis.window_fill.gamma_multiply(0.95),
        Stroke::new(1.0_f32, accent),
        egui::StrokeKind::Inside,
    );
    let mut y = r.top() + pad;
    for g in galleys {
        let gh = g.size().y;
        painter.galley(egui::pos2(r.left() + pad, y), g, vis.text_color());
        y += gh + 2.0;
    }
}

/// 点が多角形の中か
pub(super) fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
    }
    c
}

#[cfg(test)]
#[path = "figure_tests.rs"]
mod tests;

#[path = "poly_fill.rs"]
mod poly_fill;

/// 寸法を選んでいるときに数を打てば、その字から書き換えを始める（表のセルと同じ。後ろにカーソル）
pub(super) fn type_to_edit(ui: &mut Ui, combo: &mut Combo, view: &mut View) {
    if let (Some(g), None) = (view.dim, view.edit.as_ref()) {
        let t: String = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || ".,".contains(c)) {
            view.start_edit(combo, g);
            if let Some(e) = view.edit.as_mut() {
                e.1 = t;
            }
            let id = egui::Id::unique(("lensbench-dim-edit", g)).with("typed");
            ui.data_mut(|d| d.insert_temp(id, true));
        }
    }
}
