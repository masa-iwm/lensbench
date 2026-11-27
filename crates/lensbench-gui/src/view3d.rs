//! レイアウトの 3D 表示: serve の `layout`（`plane: "3d"`）の網（書き出しの scene と同じ立体・面）と
//! 光線を、正射影で回して見る。
//!
//! 描画は egui の三角形で（GPU の深度は使わず、遠い順に並べて塗る）。ガラスは半透明、鏡は灰色、
//! 選んだ組合せの要素はアクセント色。操作: ドラッグで回す、Shift・右・中ドラッグで動かす、ホイールで拡大、
//! ダブルクリックで初めの向き、クリックで要素を選ぶ。キーは ← → ↑ ↓ で回す、+ − で拡大、Home で初めの向き。

use egui::{Color32, Key, Pos2, Stroke, Ui, Vec2};
use serde_json::Value;

use crate::i18n::tr;
use crate::theme;

/// 3D の場面（serve の結果を読んだもの）
#[derive(Debug, Clone, Default)]
pub(crate) struct Scene3 {
    pub meshes: Vec<Mesh3>,
    /// 光線の色の番号（1 始まり。視野、構成を重ねたときは構成。機構部品に切られた後ろの部分は
    /// [`crate::layout_rays::FAINT`] の印付き）と点
    pub rays: Vec<(usize, Vec<[f32; 3]>)>,
    /// 場面の真ん中と、それを囲む球の半径
    pub center: [f32; 3],
    pub radius: f32,
    /// 範囲の箱の半分の大きさ（初めの拡大を、見えている広がりに合わせる）
    pub half: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    Glass,
    Mirror,
    /// 機構部品
    Mech,
    /// 像面・物体面・理想レンズ（"image"・"object"・"paraxial"）
    Plate(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Mesh3 {
    pub kind: Kind,
    /// 網の名前（機構部品なら部品の名前）
    pub name: String,
    pub surfaces: [usize; 2],
    pub pos: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
}

impl Scene3 {
    pub(crate) fn parse(v: &Value, by_cfg: bool) -> Scene3 {
        let arr = |x: &Value| x.as_array().cloned().unwrap_or_default();
        let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
        let meshes: Vec<Mesh3> = arr(&v["meshes"])
            .iter()
            .map(|m| {
                let f: Vec<f32> = arr(&m["pos"])
                    .iter()
                    .map(|x| x.as_f64().unwrap_or(0.0) as f32)
                    .collect();
                let t: Vec<u32> = arr(&m["tris"])
                    .iter()
                    .map(|x| x.as_u64().unwrap_or(0) as u32)
                    .collect();
                let n = f.len() / 3;
                Mesh3 {
                    kind: match m["kind"].as_str().unwrap_or_default() {
                        "glass" => Kind::Glass,
                        "mirror" => Kind::Mirror,
                        "mech" => Kind::Mech,
                        k => Kind::Plate(k.to_string()),
                    },
                    name: m["name"].as_str().unwrap_or_default().to_string(),
                    surfaces: [us(&m["surfaces"][0]), us(&m["surfaces"][1])],
                    pos: f
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|c| [c[0], c[1], c[2]])
                        .collect(),
                    tris: t
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|c| [c[0], c[1], c[2]])
                        .filter(|c| c.iter().all(|&i| (i as usize) < n))
                        .collect(),
                }
            })
            .collect();
        let rays: Vec<(usize, Vec<[f32; 3]>)> = arr(&v["rays"])
            .iter()
            .flat_map(|r| {
                let c = crate::layout_rays::ray_class(
                    r,
                    v["color"].as_str().unwrap_or_default(),
                    by_cfg,
                );
                let pts = arr(&r["pts"])
                    .iter()
                    .filter_map(|p| {
                        Some([
                            p[0].as_f64()? as f32,
                            p[1].as_f64()? as f32,
                            p[2].as_f64()? as f32,
                        ])
                    })
                    .collect();
                crate::layout_rays::split_faint(r, c, pts)
            })
            .collect();
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in meshes
            .iter()
            .flat_map(|m| m.pos.iter())
            .chain(rays.iter().flat_map(|r| r.1.iter()))
        {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let half = if lo[0].is_finite() {
            [0, 1, 2].map(|i| ((hi[i] - lo[i]) / 2.0).max(1e-3))
        } else {
            [1.0; 3]
        };
        let (center, radius) = if lo[0].is_finite() {
            let c = [0, 1, 2].map(|i| (lo[i] + hi[i]) / 2.0);
            let r = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f32>().sqrt() / 2.0;
            (c, r.max(1e-3))
        } else {
            ([0.0; 3], 1.0)
        };
        Scene3 {
            meshes,
            rays,
            center,
            radius,
            half,
        }
    }
}

/// 見る向きと拡大（egui の記憶に図ごとに持つ）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cam {
    /// 縦軸まわり・横軸まわりの回転 [rad]
    pub yaw: f32,
    pub pitch: f32,
    /// 全体が入る大きさに対する倍率
    pub zoom: f32,
    /// 画面の上でずらす量（場面の半径を 1 とした量）
    pub pan: Vec2,
    /// 全体表示のときの向きで場面の箱を画面へ写した広がり（右・上の半分、場面の半径を 1 とした量）。
    /// 全体表示（初め・ダブルクリック・Home）のときだけ決め、回す間は変えない（回しても縮尺が変わらないように）。
    /// None なら次に描くときに今の向きで決める
    pub fit: Option<[f32; 2]>,
    /// `fit` を決めたときの場面の箱（系を直して箱が変わったら決め直す）
    pub fit_half: [f32; 3],
}

impl Default for Cam {
    /// 初めの向き: 2D の YZ 面の図（光軸が右、y が上）を斜め上手前から見る
    fn default() -> Self {
        Cam {
            yaw: -0.5,
            pitch: 0.35,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: None,
            fit_half: [0.0; 3],
        }
    }
}

impl Cam {
    /// 大域座標（真ん中からの差）を見る座標 [右, 上, 手前] に
    pub(crate) fn view(&self, p: [f32; 3]) -> [f32; 3] {
        // 回す前: 右 = z、上 = y、手前 = 右 × 上 = −x
        let (x, y, z) = (p[2], p[1], -p[0]);
        let (sy, cy) = self.yaw.sin_cos();
        let (x1, z1) = (x * cy + z * sy, -x * sy + z * cy);
        let (sp, cp) = self.pitch.sin_cos();
        let (y2, z2) = (y * cp - z1 * sp, y * sp + z1 * cp);
        [x1, y2, z2]
    }

    /// 場面の箱（真ん中からの半分の大きさ `half`）を今の向きで画面へ写した広がり（右・上の半分）を、
    /// 場面の半径 `radius` を 1 とした量で返す
    pub(crate) fn extent(&self, half: [f32; 3], radius: f32) -> [f32; 2] {
        let (mut ex, mut ey) = (1e-6f32, 1e-6f32);
        for k in 0..8 {
            let c = [0, 1, 2].map(|i| if k >> i & 1 == 1 { half[i] } else { -half[i] });
            let v = self.view(c);
            ex = ex.max(v[0].abs());
            ey = ey.max(v[1].abs());
        }
        let r = radius.max(1e-6);
        [ex / r, ey / r]
    }

    /// 全体表示の後（`fit` が無いとき）、今の向きで全体が入る広がりを決めて覚える
    pub(crate) fn refit(&mut self, sc: &Scene3) {
        if self.fit.is_none() {
            self.fit = Some(self.extent(sc.half, sc.radius));
            self.fit_half = sc.half;
        }
    }

    /// 画面の 1 単位（場面の座標の 1）あたりの点の数。`fit` が無ければ今の向きで決める
    pub(crate) fn scale(&self, inner: egui::Vec2, half: [f32; 3], radius: f32) -> f32 {
        let [fx, fy] = self.fit.unwrap_or_else(|| self.extent(half, radius));
        let r = radius.max(1e-6);
        let base = (inner.x / 2.0 / (fx * r)).min(inner.y / 2.0 / (fy * r));
        base * self.zoom
    }
}

/// 3D 表示でクリックしたもの
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Click3 {
    /// 何も無い所
    Empty,
    /// 要素（`owner` が面の範囲から決める番号）
    El(usize),
    /// 機構部品（名前）
    Mech(String),
}

/// 図の中の 3D 表示。クリックしたもの（要素か機構部品か何も無い所）を返す。mech_sel は選んでいる機構部品の名前。
/// `pick_mech` が false（組合せでない図）なら機構部品は選べず、その網は面の範囲を持つ要素のものにする
/// `full` はパネルいっぱいの図（素のホイールで拡大縮小。[`crate::widgets::NavOpts::full`]）
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut Ui,
    resp: &egui::Response,
    full: bool,
    sc: &Scene3,
    configs: &[String],
    owner: &dyn Fn([usize; 2]) -> Option<usize>,
    sel: Option<usize>,
    mech_sel: Option<&str>,
    pick_mech: bool,
    accent: Color32,
) -> Option<Click3> {
    let rect = resp.rect;
    let vis = ui.visuals().clone();
    let weak = theme::weak_text(&vis);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
    let id = resp.id.with("lensbench-view3d");
    let mut cam: Cam = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // 操作
    let pan_mode = ui.input(|i| i.modifiers.shift) || resp.dragged_by(egui::PointerButton::Middle);
    let inner = rect.shrink(16.0);
    // 全体が入る倍率: 全体表示のときの向きで箱の 8 つの角を画面へ写した広がりで決め、回す間は変えない
    // （系を直して場面の箱が変わったときも決め直す）
    if cam.fit_half != sc.half {
        cam.fit = None;
    }
    cam.refit(sc);
    let base = cam.scale(inner.size(), sc.half, sc.radius) / cam.zoom;
    // 拡大・移動・全体表示・戻るはほかの図と同じ操作（widgets::pan_zoom）。左ドラッグは回す（Shift+左は移動）、
    // 矢印は回す。右下の＋・－・囲んで拡大（Ctrl＋左ドラッグでも）は図の中心で拡大する
    let opts = crate::widgets::NavOpts {
        full,
        arrows: false,
        ..crate::widgets::NavOpts::FIGURE
    };
    let pz = crate::widgets::pan_zoom(ui, resp, opts);
    // 戻る・進むはこのフレームのほかの操作を当てない。回すドラッグも戻れるように始めに覚える
    let swapped = crate::widgets::fig_nav::history(ui, resp, &pz, &mut cam);
    if !swapped && resp.drag_started() && !pz.changes() {
        crate::widgets::fig_nav::history_push(ui, resp, &cam);
    }
    if swapped {
    } else if resp.dragged() && !crate::widgets::box_zooming(ui, resp) {
        let d = resp.drag_delta();
        if pan_mode || pz.pan != egui::Vec2::ZERO {
            cam.pan += d / (base * cam.zoom * sc.radius);
        } else {
            cam.yaw += d.x * 0.01;
            cam.pitch = (cam.pitch + d.y * 0.01).clamp(-1.55, 1.55);
        }
    } else if pz.pan != egui::Vec2::ZERO {
        // 囲んで拡大: 囲んだ所の中心を図の中心へ
        cam.pan += pz.pan / (base * cam.zoom * sc.radius);
    }
    if pz.at.is_some() && !swapped {
        cam.zoom = (cam.zoom * pz.zoom).clamp(0.05, 200.0);
    }
    if pz.reset && !swapped {
        cam = Cam::default();
    }
    let keys = (resp.hovered() || resp.has_focus())
        && ui.memory(|m| m.focused().is_none_or(|f| f == resp.id));
    if keys {
        ui.input(|i| {
            let st = 0.15;
            if !i.modifiers.is_none() {
                return;
            }
            if i.key_pressed(Key::ArrowLeft) {
                cam.yaw -= st;
            }
            if i.key_pressed(Key::ArrowRight) {
                cam.yaw += st;
            }
            if i.key_pressed(Key::ArrowUp) {
                cam.pitch = (cam.pitch - st).clamp(-1.55, 1.55);
            }
            if i.key_pressed(Key::ArrowDown) {
                cam.pitch = (cam.pitch + st).clamp(-1.55, 1.55);
            }
        });
    }
    cam.refit(sc);
    ui.data_mut(|d| d.insert_temp(id, cam));

    let s = cam.scale(inner.size(), sc.half, sc.radius);
    let c0 = inner.center() + cam.pan * s * sc.radius;
    let to_view = |p: [f32; 3]| {
        cam.view([
            p[0] - sc.center[0],
            p[1] - sc.center[1],
            p[2] - sc.center[2],
        ])
    };
    let to_screen = |v: [f32; 3]| Pos2::new(c0.x + v[0] * s, c0.y - v[1] * s);

    // 塗るもの: 三角形と光線の短い線分を、遠い順に
    enum Prim {
        /// 三角形・色・持ち主（要素の番号か機構部品の網の番号）
        Tri([Pos2; 3], Color32, Owner3),
        Seg([Pos2; 2], Color32),
    }
    let glass = theme::glass_3d(&vis);
    let mirror = theme::mirror_3d(&vis);
    let plate = vis.strong_text_color();
    let light = {
        let l = [0.35f32, 0.55, 1.0];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|x| x / n)
    };
    let mut prims: Vec<(f32, Prim)> = Vec::new();
    for (mi, m) in sc.meshes.iter().enumerate() {
        let k = owner(m.surfaces);
        let selected = k.is_some() && k == sel;
        let mech_selected = m.kind == Kind::Mech && mech_sel == Some(m.name.as_str());
        let who = match (&m.kind, k) {
            (Kind::Mech, _) if pick_mech && !m.name.is_empty() => Owner3::Mech(mi),
            (Kind::Mech, _) if pick_mech => Owner3::None,
            (_, Some(k)) => Owner3::El(k),
            _ => Owner3::None,
        };
        let (col, alpha) = match &m.kind {
            Kind::Glass if selected => (accent, 0.85),
            Kind::Glass => (glass, 0.5),
            Kind::Mirror => (if selected { accent } else { mirror }, 0.9),
            Kind::Plate(_) => (plate, 0.35),
            Kind::Mech if mech_selected => (accent, 0.7),
            Kind::Mech => (theme::mech_3d(&vis), 0.45),
        };
        let vs: Vec<[f32; 3]> = m.pos.iter().map(|&p| to_view(p)).collect();
        for t in &m.tris {
            let [a, b, c] = t.map(|i| vs[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len <= 0.0 || !len.is_finite() {
                continue;
            }
            // 両面とも照らす（網の向きは書き出しの約束どおりとは限らないので絶対値）
            let lam = ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len).abs();
            let f = 0.45 + 0.55 * lam;
            let rgb = [col.r(), col.g(), col.b()].map(|x| (x as f32 * f).min(255.0) as u8);
            let fill =
                Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8);
            let depth = (a[2] + b[2] + c[2]) / 3.0;
            prims.push((
                depth,
                Prim::Tri([to_screen(a), to_screen(b), to_screen(c)], fill, who),
            ));
        }
    }
    // 光線は玉と前後が入れ替わるよう、短く切って並べる
    let piece = sc.radius / 40.0;
    for (f, pts) in &sc.rays {
        let (f, faint) = crate::layout_rays::class_of(*f);
        let col =
            theme::series(f.saturating_sub(1)).gamma_multiply(if faint { 0.25 } else { 0.85 });
        for w in pts.windows(2) {
            let (a, b) = (to_view(w[0]), to_view(w[1]));
            let len =
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
            let n = ((len / piece).ceil() as usize).clamp(1, 200);
            for i in 0..n {
                let t0 = i as f32 / n as f32;
                let t1 = (i + 1) as f32 / n as f32;
                let p = |t: f32| [0, 1, 2].map(|j| a[j] + (b[j] - a[j]) * t);
                let (p0, p1) = (p(t0), p(t1));
                prims.push((
                    (p0[2] + p1[2]) / 2.0,
                    Prim::Seg([to_screen(p0), to_screen(p1)], col),
                ));
            }
        }
    }
    prims.sort_by(|a, b| a.0.total_cmp(&b.0));
    // 続く三角形は 1 つの網にまとめて描く
    let mut mesh = egui::Mesh::default();
    let flush = |mesh: &mut egui::Mesh| {
        if !mesh.is_empty() {
            painter.add(egui::Shape::mesh(std::mem::take(mesh)));
        }
    };
    for (_, p) in &prims {
        match p {
            Prim::Tri(q, c, _) => {
                let i0 = mesh.vertices.len() as u32;
                for &v in q {
                    mesh.colored_vertex(v, *c);
                }
                mesh.add_triangle(i0, i0 + 1, i0 + 2);
            }
            Prim::Seg(q, c) => {
                flush(&mut mesh);
                painter.line_segment(*q, Stroke::new(1.0_f32, *c));
            }
        }
    }
    flush(&mut mesh);

    // 向きの印（左下）: x 赤・y 緑・z 青
    let o = Pos2::new(rect.left() + 34.0, rect.bottom() - 34.0);
    for (i, (name, c)) in [
        ("x", theme::AXES[0]),
        ("y", theme::AXES[1]),
        ("z", theme::AXES[2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut e = [0.0f32; 3];
        e[i] = 1.0;
        let v = cam.view(e);
        let tip = o + Vec2::new(v[0], -v[1]) * 22.0;
        painter.line_segment([o, tip], Stroke::new(1.5_f32, c));
        painter.text(
            tip + Vec2::new(v[0], -v[1]) * 6.0,
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(10.0),
            c,
        );
    }
    // 構成を重ねたときは構成の名前と色（左上）
    if configs.len() > 1 {
        let font = egui::FontId::proportional(11.0);
        for (k, name) in configs.iter().enumerate() {
            let p = rect.left_top() + egui::vec2(10.0, 10.0 + 15.0 * k as f32);
            let c = theme::series(k);
            painter.line_segment(
                [p + egui::vec2(0.0, 6.0), p + egui::vec2(14.0, 6.0)],
                (2.0, c),
            );
            painter.text(
                p + egui::vec2(18.0, 0.0),
                egui::Align2::LEFT_TOP,
                name,
                font.clone(),
                weak,
            );
        }
    }
    // 縮尺（正射影なので図のどこでも同じ。OpticStudio と同じく左下）
    crate::widgets::scale_bar::draw(
        &painter,
        rect.left_bottom() + Vec2::new(12.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        s as f64,
        (rect.width() / 5.0).clamp(40.0, 160.0),
        weak,
    );
    painter.text(
        rect.right_bottom() - Vec2::new(10.0, 8.0),
        egui::Align2::RIGHT_BOTTOM,
        tr(if full {
            "Drag to rotate, Shift- or middle-drag to pan, wheel or pinch to zoom, double-click to reset, right-click for more"
        } else {
            "Drag to rotate, Shift- or middle-drag to pan, Ctrl (⌘ on Mac) + wheel or pinch to zoom, double-click to reset, right-click for more"
        }),
        egui::FontId::proportional(10.5),
        weak,
    );

    crate::widgets::paint_zoom_tools(ui, resp);

    // 要素を指す: 手前から、点を含む三角形
    let pointer = resp.hover_pos();
    let hit = pointer.and_then(|q| {
        prims.iter().rev().find_map(|(_, p)| match p {
            Prim::Tri(t, _, w) if in_tri(t, q) => Some(*w),
            _ => None,
        })
    });
    let hit = match hit.unwrap_or(Owner3::None) {
        Owner3::El(k) => Click3::El(k),
        Owner3::Mech(i) => Click3::Mech(sc.meshes[i].name.clone()),
        Owner3::None => Click3::Empty,
    };
    if hit != Click3::Empty {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        return Some(hit);
    }
    None
}

/// 3D の三角形の持ち主
#[derive(Debug, Clone, Copy)]
enum Owner3 {
    None,
    El(usize),
    /// 機構部品の網（[`Scene3::meshes`] の番号）
    Mech(usize),
}

fn in_tri(t: &[Pos2; 3], p: Pos2) -> bool {
    let s = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let (d1, d2, d3) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_view_matches_yz_plot() {
        // 向きが 0 なら 2D の YZ 面の図と同じ（z が右、y が上）
        let c = Cam {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: None,
            fit_half: [0.0; 3],
        };
        assert_eq!(c.view([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        assert_eq!(c.view([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]);
        // 回しても長さは同じ
        let c = Cam::default();
        let v = c.view([1.0, 2.0, 3.0]);
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((n - 14f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn rotating_keeps_scale() {
        // 細長い箱（光軸 z に長い）: 向きを変えても全体表示で決めた倍率は変わらない
        let half = [5.0, 5.0, 100.0];
        let radius = (25.0f32 + 25.0 + 10000.0).sqrt();
        let inner = egui::vec2(800.0, 600.0);
        let mut cam = Cam::default();
        cam.fit = Some(cam.extent(half, radius));
        let s0 = cam.scale(inner, half, radius);
        for (yaw, pitch) in [(0.0, 0.0), (1.57, 0.0), (0.3, 1.2), (-2.0, -0.7)] {
            cam.yaw = yaw;
            cam.pitch = pitch;
            assert_eq!(cam.scale(inner, half, radius), s0);
        }
        // 全体表示し直す（fit を捨てる）と今の向きでぴったり入る倍率になる（光軸の方向から見ると大きい）
        cam.yaw = 1.57;
        cam.pitch = 0.0;
        cam.fit = None;
        assert!(cam.scale(inner, half, radius) > 5.0 * s0);
        // 拡大はそのまま掛かる
        cam.fit = Some(cam.extent(half, radius));
        let s1 = cam.scale(inner, half, radius);
        cam.zoom = 2.0;
        assert!((cam.scale(inner, half, radius) - 2.0 * s1).abs() < 1e-3);
    }

    #[test]
    fn parse_scene() {
        let v = json!({
            "meshes": [{ "kind": "glass", "surfaces": [1, 2],
                "pos": [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9], "tris": [0, 1, 2, 0, 1, 7] },
                { "kind": "image", "surfaces": [3, 3], "pos": [], "tris": [] }],
            "rays": [{ "field": 2, "cfg": 1, "pts": [[0, 0, 0], [0, 0, 10]] }],
        });
        let s = Scene3::parse(&v, false);
        assert_eq!(s.meshes.len(), 2);
        // 範囲外の番号の三角形は捨てる
        assert_eq!(s.meshes[0].tris.len(), 1);
        assert_eq!(s.meshes[1].kind, Kind::Plate("image".into()));
        assert_eq!(s.rays[0].0, 2);
        assert_eq!(Scene3::parse(&v, true).rays[0].0, 2);
        assert!((s.center[2] - 5.0).abs() < 1e-6);
        assert!(in_tri(
            &[
                Pos2::new(0.0, 0.0),
                Pos2::new(10.0, 0.0),
                Pos2::new(0.0, 10.0)
            ],
            Pos2::new(2.0, 2.0)
        ));
    }

    /// 機構部品の網をクリックすると部品の名前を返し、選んだ部品は網の色で分かる。何も無い所は Empty
    #[test]
    fn click_picks_mechanical_part() {
        // 部品 tube の箱（一辺 20）
        let cube = |name: &str, kind: &str| {
            let mut pos = Vec::new();
            for i in 0..8 {
                pos.extend(
                    [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64]
                        .map(|x| 20.0 * x - 10.0),
                );
            }
            let tris = [
                0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0,
                6, 4, 1, 5, 7, 1, 7, 3,
            ];
            json!({ "name": name, "kind": kind, "surfaces": [0, 0], "pos": pos, "tris": tris.to_vec() })
        };
        let sc = Scene3::parse(
            &json!({ "meshes": [cube("tube", "mech")], "rays": [] }),
            false,
        );
        let ctx = egui::Context::default();
        let size = egui::vec2(400.0, 300.0);
        let run_with = |events: Vec<egui::Event>, pick_mech: bool| {
            let mut out = None;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let (rect, resp) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                    let _ = rect;
                    out = show(
                        ui,
                        &resp,
                        true,
                        &sc,
                        &[],
                        &|_| Some(0),
                        None,
                        Some("tube"),
                        pick_mech,
                        Color32::RED,
                    );
                });
            });
            out
        };
        let run = |events: Vec<egui::Event>| run_with(events, true);
        let press = |at: Pos2, button: egui::PointerButton| {
            let ev = |pressed| egui::Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Default::default(),
            };
            vec![egui::Event::PointerMoved(at), ev(true), ev(false)]
        };
        let click = |at: Pos2| press(at, egui::PointerButton::Primary);
        run(Vec::new());
        let mid = Pos2::new(size.x / 2.0, size.y / 2.0);
        assert_eq!(run(click(mid)), Some(Click3::Mech("tube".into())));
        // 右クリックはメニュー: 部品も要素も選ばない
        assert_eq!(run(press(mid, egui::PointerButton::Secondary)), None);
        run(vec![egui::Event::PointerMoved(Pos2::new(20.0, 20.0))]);
        assert_eq!(run(click(Pos2::new(20.0, 20.0))), Some(Click3::Empty));
        // 組合せでない図: 部品は選べず、網は面の範囲を持つ要素のもの（要素の選択が外れない）
        run_with(vec![egui::Event::PointerMoved(mid)], false);
        assert_eq!(run_with(click(mid), false), Some(Click3::El(0)));
    }
}
