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
pub struct Scene3 {
    pub meshes: Vec<Mesh3>,
    /// 光線の色の番号（1 始まり。視野、構成を重ねたときは構成）と点
    pub rays: Vec<(usize, Vec<[f32; 3]>)>,
    /// 場面の真ん中と、それを囲む球の半径
    pub center: [f32; 3],
    pub radius: f32,
    /// 範囲の箱の半分の大きさ（初めの拡大を、見えている広がりに合わせる）
    pub half: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Glass,
    Mirror,
    /// 機構部品
    Mech,
    /// 像面・物体面・理想レンズ（"image"・"object"・"paraxial"）
    Plate(String),
}

#[derive(Debug, Clone)]
pub struct Mesh3 {
    pub kind: Kind,
    pub surfaces: [usize; 2],
    pub pos: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
}

impl Scene3 {
    pub fn parse(v: &Value, by_cfg: bool) -> Scene3 {
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
                    surfaces: [us(&m["surfaces"][0]), us(&m["surfaces"][1])],
                    pos: f.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
                    tris: t
                        .chunks_exact(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .filter(|c| c.iter().all(|&i| (i as usize) < n))
                        .collect(),
                }
            })
            .collect();
        let rays: Vec<(usize, Vec<[f32; 3]>)> = arr(&v["rays"])
            .iter()
            .map(|r| {
                let c = if by_cfg {
                    us(&r["cfg"]) + 1
                } else {
                    us(&r["field"])
                };
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
                (c, pts)
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
pub struct Cam {
    /// 縦軸まわり・横軸まわりの回転 [rad]
    pub yaw: f32,
    pub pitch: f32,
    /// 全体が入る大きさに対する倍率
    pub zoom: f32,
    /// 画面の上でずらす量（場面の半径を 1 とした量）
    pub pan: Vec2,
}

impl Default for Cam {
    /// 初めの向き: 2D の YZ 面の図（光軸が右、y が上）を斜め上手前から見る
    fn default() -> Self {
        Cam {
            yaw: -0.5,
            pitch: 0.35,
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl Cam {
    /// 大域座標（真ん中からの差）を見る座標 [右, 上, 手前] に
    pub fn view(&self, p: [f32; 3]) -> [f32; 3] {
        // 回す前: 右 = z、上 = y、手前 = 右 × 上 = −x
        let (x, y, z) = (p[2], p[1], -p[0]);
        let (sy, cy) = self.yaw.sin_cos();
        let (x1, z1) = (x * cy + z * sy, -x * sy + z * cy);
        let (sp, cp) = self.pitch.sin_cos();
        let (y2, z2) = (y * cp - z1 * sp, y * sp + z1 * cp);
        [x1, y2, z2]
    }
}

/// 図の中の 3D 表示。クリックで選んだ要素（`owner` が面の範囲から決める番号）を返す（何もない所なら Some(None)）
pub fn show(
    ui: &mut Ui,
    resp: &egui::Response,
    sc: &Scene3,
    configs: &[String],
    owner: &dyn Fn([usize; 2]) -> Option<usize>,
    sel: Option<usize>,
    accent: Color32,
) -> Option<Option<usize>> {
    let rect = resp.rect;
    let vis = ui.visuals().clone();
    let weak = theme::weak_text(&vis);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
    let id = resp.id.with("lensbench-view3d");
    let mut cam: Cam = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // 操作
    let pan_mode = ui.input(|i| i.modifiers.shift)
        || resp.dragged_by(egui::PointerButton::Secondary)
        || resp.dragged_by(egui::PointerButton::Middle);
    let inner = rect.shrink(16.0);
    // 全体が入る倍率: 箱の 8 つの角を今の向きで画面へ写した広がり（回すと少し変わる）
    let (mut ex, mut ey) = (1e-6f32, 1e-6f32);
    for k in 0..8 {
        let c = [0, 1, 2].map(|i| {
            if k >> i & 1 == 1 {
                sc.half[i]
            } else {
                -sc.half[i]
            }
        });
        let v = cam.view(c);
        ex = ex.max(v[0].abs());
        ey = ey.max(v[1].abs());
    }
    let base = (inner.width() / 2.0 / ex).min(inner.height() / 2.0 / ey);
    if resp.dragged() {
        let d = resp.drag_delta();
        if pan_mode {
            cam.pan += d / (base * cam.zoom * sc.radius);
        } else {
            cam.yaw += d.x * 0.01;
            cam.pitch = (cam.pitch + d.y * 0.01).clamp(-1.55, 1.55);
        }
    }
    if resp.hovered() {
        let z = ui.input(|i| i.smooth_scroll_delta.y);
        if z != 0.0 {
            cam.zoom = (cam.zoom * (z * 0.002).exp()).clamp(0.05, 200.0);
        }
    }
    if resp.double_clicked() {
        cam = Cam::default();
    }
    if resp.has_focus() {
        ui.input(|i| {
            let st = 0.15;
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
            if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                cam.zoom *= 1.25;
            }
            if i.key_pressed(Key::Minus) {
                cam.zoom /= 1.25;
            }
            if i.key_pressed(Key::Home) {
                cam = Cam::default();
            }
        });
    }
    ui.data_mut(|d| d.insert_temp(id, cam));

    let s = base * cam.zoom;
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
        Tri([Pos2; 3], Color32, Option<usize>),
        Seg([Pos2; 2], Color32),
    }
    let glass = if vis.dark_mode {
        Color32::from_rgb(0x4A, 0x7A, 0xA6)
    } else {
        Color32::from_rgb(0x9C, 0xC6, 0xEC)
    };
    let mirror = Color32::from_rgb(0xA8, 0xAF, 0xB8);
    let plate = vis.strong_text_color();
    let light = {
        let l = [0.35f32, 0.55, 1.0];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|x| x / n)
    };
    let mut prims: Vec<(f32, Prim)> = Vec::new();
    for m in &sc.meshes {
        let k = owner(m.surfaces);
        let selected = k.is_some() && k == sel;
        let (col, alpha) = match &m.kind {
            Kind::Glass if selected => (accent, 0.85),
            Kind::Glass => (glass, 0.5),
            Kind::Mirror => (if selected { accent } else { mirror }, 0.9),
            Kind::Plate(_) => (plate, 0.35),
            Kind::Mech => (Color32::from_rgb(0x9A, 0x8C, 0x7A), 0.45),
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
                Prim::Tri([to_screen(a), to_screen(b), to_screen(c)], fill, k),
            ));
        }
    }
    // 光線は玉と前後が入れ替わるよう、短く切って並べる
    let piece = sc.radius / 40.0;
    for (f, pts) in &sc.rays {
        let col = theme::series(f.saturating_sub(1)).gamma_multiply(0.85);
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
        ("x", Color32::from_rgb(0xD9, 0x53, 0x4F)),
        ("y", Color32::from_rgb(0x4C, 0xAF, 0x50)),
        ("z", Color32::from_rgb(0x42, 0x8B, 0xCA)),
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
    painter.text(
        rect.right_bottom() - Vec2::new(10.0, 8.0),
        egui::Align2::RIGHT_BOTTOM,
        tr("ドラッグで回す・Shift か右ドラッグで動かす・ホイールで拡大・ダブルクリックで戻す"),
        egui::FontId::proportional(10.5),
        weak,
    );

    // 要素を指す: 手前から、点を含む三角形
    let pointer = resp.hover_pos();
    let hit = pointer.and_then(|q| {
        prims.iter().rev().find_map(|(_, p)| match p {
            Prim::Tri(t, _, k) if in_tri(t, q) => Some(*k),
            _ => None,
        })
    });
    if hit.flatten().is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        return Some(hit.flatten());
    }
    None
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
}
