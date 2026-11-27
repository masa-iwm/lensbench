//! 物の場面（非順次の追跡の段 1）: 順次の系・プロジェクトから、物（玉・鏡・絞り・検出器・機構部品）を大域の座標に
//! 1 回だけ置いた並びを作る。
//!
//! - 順次の系の面は、大域の座標で同じ所・同じ形の面（往復で 2 回通る面、BS の別の経路の同じ面）を 1 つの面にまとめる。
//!   同じかどうかは、頂点・軸（向きは逆でもよい）と、面の上の点が相手の面の上に乗ることで見る（[`SAME_TOL`]）
//! - 玉は、ガラスを挟む 2 面ごとの区間（[`Region`]）でつながった面の集まり。区間の縁は OpticStudio の玉の形と同じ:
//!   面の形はクリア半径＋チップゾーン（[`System::flat_from`]）まで、その外は玉の半径（[`System::lens_radius`]）まで
//!   平らな輪、側面は玉の半径の円柱。2 面の軸がそろわない区間（偏心した玉）と、円でない開口の面（BS のキューブ）の
//!   区間は縁を作らない（[`Object::closed`] が false）
//! - 空気中の鏡は鏡、空気中で開口を持つ面と絞りは絞り、像面は検出器、理想レンズなど空気中の働きのある面は面にする
//! - 機構部品（系の `mechanics`）は、部品の座標の輪ごとの内壁・外壁の円柱と端の輪
//!
//! 面の名前は、展開の面の出どころ（[`SurfOrigin::face`]）の [`FaceId`]。まとめた面はどの名前・どの系の面から来たかを
//! 全部持つ（[`Face::refs`]）。面の両側の媒質は、面の局所座標の −z 側（[`Face::minus`]）と +z 側（[`Face::plus`]）。

use crate::{expand, paths, Env, ExpandOpts, Expansion, FaceId, Project, SurfOrigin};
use lensbench_core::material::Material;
use lensbench_core::math::{add, col, dot, mtv, mv, norm, sub, V3};
use lensbench_core::surface::{Aperture, Surface};
use lensbench_core::system::Frame;
use lensbench_core::{invalid, Result, System};

/// 同じ面とみなす位置のずれ [mm]（頂点・面の上の点）と軸の向きのずれ（1 − |cos|）
pub const SAME_TOL: f64 = 1e-9;

/// 物の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// ガラスの立体（貼り合わせ・プリズム・BS のキューブを含む）
    Lens,
    /// 空気中の鏡
    Mirror,
    /// 空気中の開口（絞り・遮光板）。開口の外は遮る
    Stop,
    /// 空気中の働きのある面（理想レンズ・Jones 面など）
    Plane,
    /// 検出器（像面）
    Detector,
    /// 機構部品（壁で光を遮る）
    Mech,
}

/// 面の形（面の局所座標）
#[derive(Debug, Clone)]
pub enum Shape {
    /// 順次の系の面の形（矢高・開口は面のまま。形は `extent` の半径まで）
    Optical {
        surface: Box<Surface>,
        /// 面の形の及ぶ半径（玉の面はクリア半径＋チップゾーン、ほかは半直径）
        extent: f64,
    },
    /// z = 一定の平らな輪（内側の半径〜外側の半径）
    Annulus { r0: f64, r1: f64, z: f64 },
    /// z 軸のまわりの円柱（半径、z の範囲）
    Cylinder { r: f64, z0: f64, z1: f64 },
}

/// 面がどの系の何番の面から来たか
#[derive(Debug, Clone, PartialEq)]
pub struct SurfRef {
    /// 系の番号（[`Scene::systems`]）
    pub system: usize,
    /// 系の面の番号
    pub surf: usize,
    /// その面の局所座標の z が場面の面の z と逆向き
    pub flipped: bool,
    /// プロジェクトの面の名前
    pub face: Option<FaceId>,
}

/// 場面の面
#[derive(Debug, Clone)]
pub struct Face {
    /// 物の番号（[`Scene::objects`]）
    pub object: usize,
    /// 局所座標（大域の原点と、列が軸の回転）
    pub frame: Frame,
    pub shape: Shape,
    /// 局所の −z 側・+z 側の媒質
    pub minus: Material,
    pub plus: Material,
    /// 膜の名前（無ければ空）
    pub coat: String,
    /// 来た順次の系の面（縁の面は空）
    pub refs: Vec<SurfRef>,
}

impl Face {
    /// 局所座標の点を大域へ
    pub fn to_global(&self, p: V3) -> V3 {
        add(self.frame.o, mv(&self.frame.r, p))
    }
    /// 大域の点を局所座標へ
    pub fn to_local(&self, p: V3) -> V3 {
        mtv(&self.frame.r, sub(p, self.frame.o))
    }
    /// 局所の z 軸（大域）
    pub fn axis(&self) -> V3 {
        col(&self.frame.r, 2)
    }
    /// 面の名前（来た面の名前のうち最初のもの）
    pub fn name(&self) -> Option<&FaceId> {
        self.refs.iter().find_map(|r| r.face.as_ref())
    }
}

/// 物
#[derive(Debug, Clone)]
pub struct Object {
    pub name: String,
    pub kind: ObjectKind,
    /// 面の番号（[`Scene::faces`]）
    pub faces: Vec<usize>,
    /// 縁まで閉じた立体か（玉で縁を作れなかった区間があれば false。玉以外は true）
    pub closed: bool,
}

/// 玉の中のガラスの区間: 2 つの光学面の間の媒質
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// 前・後の面（[`Scene::faces`]、順次の系で通った順）
    pub a: usize,
    pub b: usize,
    pub material: Material,
}

/// 物の場面
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub objects: Vec<Object>,
    pub faces: Vec<Face>,
    pub regions: Vec<Region>,
    /// 足した順次の系の名前（[`SurfRef::system`] の番号の順）
    pub systems: Vec<String>,
    /// 縁を作れなかった区間などの注意
    pub notes: Vec<String>,
    /// 面ごとの玉の縁の値（光学面だけ）: (クリア半径＋チップゾーン, 玉の半径)
    edge: Vec<Option<(f64, f64)>>,
    /// 足した機構部品（同じ部品を 2 回足さない）
    mech_keys: Vec<Vec<i64>>,
    finished: bool,
}

/// 面 j の媒質の前後（順次の系の進む向きで）
fn media(s: &System) -> Vec<(Material, Material)> {
    let mut before = Material::Air;
    s.surfaces
        .iter()
        .map(|d| {
            let after = match &d.mat {
                Material::Mirror | Material::Inherit => before.clone(),
                m => m.clone(),
            };
            let r = (before.clone(), after.clone());
            before = after;
            r
        })
        .collect()
}

fn same_mat(a: &Material, b: &Material) -> bool {
    match (a, b) {
        (Material::Glass(x), Material::Glass(y)) => x.eq_ignore_ascii_case(y),
        _ => a == b,
    }
}

/// 面が光学面として場面に入るか（物体面・無視・座標ブレークを除く）
fn counts(s: &System, j: usize) -> bool {
    j >= 1 && !s.surfaces[j].ignore && !s.surfaces[j].is_coord_break()
}

/// 場面を作るプロジェクトの展開と名前: 光束があれば光束ごと（`beam 名前`）、無ければ BS の経路ごと（`path TR`、BS が
/// 無ければ `system` の 1 つ）
pub fn expansions(p: &Project, env: &Env) -> Result<Vec<(String, Expansion)>> {
    let names = p.beam_names();
    let one = |o: ExpandOpts| expand(p, env, &o).map_err(|e| e.into_error());
    if !names.is_empty() {
        return names
            .into_iter()
            .map(|n| {
                Ok((
                    format!("beam {n}"),
                    one(ExpandOpts {
                        state: Some(n),
                        ..Default::default()
                    })?,
                ))
            })
            .collect();
    }
    let mut qs = paths(p, env)?;
    if qs.is_empty() {
        qs.push(String::new());
    }
    qs.into_iter()
        .map(|q| {
            let x = one(ExpandOpts {
                path: (!q.is_empty()).then_some(q.as_str()),
                ..Default::default()
            })?;
            let label = if q.is_empty() {
                "system".to_string()
            } else {
                format!("path {q}")
            };
            Ok((label, x))
        })
        .collect()
}

impl Scene {
    pub fn new() -> Scene {
        Scene::default()
    }

    /// プロジェクトの場面: [`expansions`] の展開を全部足したもの
    pub fn from_project(p: &Project, env: &Env) -> Result<Scene> {
        let mut sc = Scene::new();
        for (label, x) in expansions(p, env)? {
            sc.add_system(&label, &x.system, Some(&x.map))?;
        }
        sc.finish()?;
        Ok(sc)
    }

    /// 順次の系 s（名前 label）の物を足す。map は展開の面の出どころ（無ければ面の名前は None）。足し終えたら
    /// [`Self::finish`] で玉の縁を作る
    pub fn add_system(
        &mut self,
        label: &str,
        s: &System,
        map: Option<&[SurfOrigin]>,
    ) -> Result<()> {
        if self.finished {
            invalid!("scene: a system can't be added after the scene is finished");
        }
        if let Some(m) = map {
            if m.len() != s.surfaces.len() {
                invalid!(
                    "the surface origins ({}) don't match the system ({} surfaces)",
                    m.len(),
                    s.surfaces.len()
                );
            }
        }
        let sys = self.systems.len();
        self.systems.push(label.to_string());
        let fr = s.frames();
        let md = media(s);
        let img = s.image();
        // 進む向き（面の局所の +z へ進むか）: 反射の面を通るたびに逆になる
        let mut fwd = true;
        // 系の面 j → 場面の面
        let mut at: Vec<Option<usize>> = vec![None; s.surfaces.len()];
        for j in 1..=img {
            if !counts(s, j) {
                continue;
            }
            let d = &s.surfaces[j];
            let Some(f) = fr.surf[j] else { continue };
            let (before, mut after) = md[j].clone();
            // 境界での部分反射（光束の turn・BS の反射）: 向こうの媒質は反射の欄に書いてある
            let partial = d.is_mirror() && d.reflect.is_some();
            if let Some(rf) = d.reflect.as_ref().filter(|_| partial) {
                after = if rf.mat.eq_ignore_ascii_case("AIR") || rf.mat.is_empty() {
                    Material::Air
                } else {
                    Material::Glass(rf.mat.clone())
                };
            }
            let reflects = d.is_mirror();
            let in_glass = !before.is_air() || !after.is_air();
            let kind = if j == img {
                ObjectKind::Detector
            } else if in_glass {
                ObjectKind::Lens
            } else if reflects && !partial {
                ObjectKind::Mirror
            } else if j == s.stop() || !matches!(d.ap, Aperture::None | Aperture::Floating { .. }) {
                ObjectKind::Stop
            } else if !matches!(d.action(), lensbench_core::action::Action::Refract) && !partial {
                ObjectKind::Plane
            } else {
                // 空気中のダミー面（空気どうしの部分反射も）: 物ではない
                if reflects {
                    fwd = !fwd;
                }
                continue;
            };
            let lens = kind == ObjectKind::Lens;
            let extent = if lens { s.flat_from(j) } else { d.semi };
            let edge = lens.then(|| (s.flat_from(j), s.lens_radius(j).max(s.flat_from(j))));
            // 局所の −z 側・+z 側（部分反射の面は、入った側と向こうの媒質）
            let (minus, plus) = if fwd {
                (before.clone(), after.clone())
            } else {
                (after.clone(), before.clone())
            };
            let face = map.and_then(|m| m[j].face.clone());
            let k = match self.find_same(kind, &f, d, extent) {
                Some((k, flipped)) => {
                    self.merge(k, d.semi, extent, edge);
                    // 同じ面の両側の媒質が系の間で合うか
                    let (m2, p2) = if flipped {
                        (&plus, &minus)
                    } else {
                        (&minus, &plus)
                    };
                    let g = &self.faces[k];
                    if !same_mat(&g.minus, m2) || !same_mat(&g.plus, p2) {
                        let (a, b) = (m2.name(), p2.name());
                        let (c, e) = (g.minus.name(), g.plus.name());
                        self.notes.push(format!(
                            "scene: {label} surface {j} has the media {a} | {b} but the same face has {c} | {e}"
                        ));
                    }
                    self.faces[k].refs.push(SurfRef {
                        system: sys,
                        surf: j,
                        flipped,
                        face,
                    });
                    k
                }
                None => {
                    let name = map
                        .and_then(|m| m[j].blocks.first().cloned())
                        .unwrap_or_default();
                    let o = self.objects.len();
                    self.objects.push(Object {
                        name,
                        kind,
                        faces: Vec::new(),
                        closed: true,
                    });
                    self.faces.push(Face {
                        object: o,
                        frame: f,
                        shape: Shape::Optical {
                            surface: Box::new(d.clone()),
                            extent,
                        },
                        minus,
                        plus,
                        coat: d.coat.clone(),
                        refs: vec![SurfRef {
                            system: sys,
                            surf: j,
                            flipped: false,
                            face,
                        }],
                    });
                    self.edge.push(edge);
                    let k = self.faces.len() - 1;
                    self.objects[o].faces.push(k);
                    k
                }
            };
            at[j] = Some(k);
            if reflects {
                fwd = !fwd;
            }
        }
        // ガラスの区間: 媒質がガラスの、続く 2 つの光学面
        let js: Vec<usize> = (1..=img).filter(|&j| at[j].is_some()).collect();
        for w in js.windows(2) {
            let (ja, jb) = (w[0], w[1]);
            let m = md[ja].1.clone();
            if m.is_air() || !m.is_medium() {
                continue;
            }
            let (a, b) = (at[ja].unwrap_or(0), at[jb].unwrap_or(0));
            if a == b {
                continue;
            }
            let dup = self
                .regions
                .iter()
                .any(|r| (r.a == a && r.b == b) || (r.a == b && r.b == a));
            if !dup {
                self.regions.push(Region { a, b, material: m });
            }
            // 検出器（像面）はガラスに接していても玉にまとめない（種類が消えないように）
            let (oa, ob) = (self.faces[a].object, self.faces[b].object);
            if self.objects[oa].kind != ObjectKind::Detector
                && self.objects[ob].kind != ObjectKind::Detector
            {
                self.join(oa, ob);
            }
        }
        // 機構部品
        for part in &s.mechanics {
            let (o, r) = part.frame_in(fr)?;
            self.add_mech(&part.name, Frame { o, r }, &part.profile(), part.square);
        }
        Ok(())
    }

    /// 足し終えた場面の玉の縁（平らな輪と側面の円柱）を作る
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let regions = self.regions.clone();
        for r in &regions {
            self.add_edge(r);
        }
        // 空になった物（まとめた物）を除いて番号を詰める
        let mut new = vec![usize::MAX; self.objects.len()];
        let mut objs = Vec::new();
        for (i, o) in self.objects.iter().enumerate() {
            if !o.faces.is_empty() {
                new[i] = objs.len();
                objs.push(o.clone());
            }
        }
        for f in &mut self.faces {
            f.object = new[f.object];
        }
        self.objects = objs;
        Ok(())
    }

    /// 系 system の面 surf がまとめられた場面の面
    pub fn face_of(&self, system: usize, surf: usize) -> Option<usize> {
        self.faces
            .iter()
            .position(|f| f.refs.iter().any(|r| r.system == system && r.surf == surf))
    }

    /// 名前 id の面（場面の面）
    pub fn face_named(&self, id: &FaceId) -> Option<usize> {
        self.faces
            .iter()
            .position(|f| f.refs.iter().any(|r| r.face.as_ref() == Some(id)))
    }

    /// 頂点 f・面 d と同じ所・同じ形の光学面: (場面の面, 局所の z が逆向きか)
    fn find_same(
        &self,
        kind: ObjectKind,
        f: &Frame,
        d: &Surface,
        extent: f64,
    ) -> Option<(usize, bool)> {
        let ax = col(&f.r, 2);
        let scale = 1.0 + norm(f.o);
        let sh = d.shape();
        self.faces.iter().enumerate().find_map(|(k, g)| {
            let Shape::Optical {
                surface,
                extent: e2,
            } = &g.shape
            else {
                return None;
            };
            if self.objects[g.object].kind != kind {
                return None;
            }
            if norm(sub(g.frame.o, f.o)) > SAME_TOL * scale {
                return None;
            }
            let c = dot(ax, g.axis());
            if 1.0 - c.abs() > SAME_TOL {
                return None;
            }
            // 面の上の点が相手の面に乗る（小さい方の半径の中の点）
            let r = extent.min(*e2);
            let sh2 = surface.shape();
            let pts = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-0.7, 0.7), (0.5, -0.5)];
            let ok = pts.iter().all(|&(u, v)| {
                let (x, y) = (u * r, v * r);
                let z = sh.sag(x, y).0;
                let p = add(f.o, mv(&f.r, [x, y, z]));
                let q = g.to_local(p);
                let z2 = sh2.sag(q[0], q[1]).0;
                (z.is_nan() && z2.is_nan()) || (q[2] - z2).abs() <= SAME_TOL * scale
            });
            ok.then_some((k, c < 0.0))
        })
    }

    /// まとめた面の半径を広げる（経路で自動の半直径が違うとき、大きい方）
    fn merge(&mut self, k: usize, semi: f64, extent: f64, edge: Option<(f64, f64)>) {
        if let Shape::Optical { surface, extent: e } = &mut self.faces[k].shape {
            *e = e.max(extent);
            surface.semi = surface.semi.max(semi);
        }
        if let (Some(Some((a, b))), Some((c, d))) = (self.edge.get_mut(k), edge) {
            *a = a.max(c);
            *b = b.max(d);
        }
    }

    /// 物 b を物 a にまとめる（面を移す）
    fn join(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        let fs = std::mem::take(&mut self.objects[b].faces);
        for &k in &fs {
            self.faces[k].object = a;
        }
        self.objects[a].faces.extend(fs);
        self.objects[a].kind = ObjectKind::Lens;
        if self.objects[a].name.is_empty() {
            self.objects[a].name = std::mem::take(&mut self.objects[b].name);
        }
    }

    /// ガラスの区間の縁: 端の面の平らな輪と、側面の円柱
    fn add_edge(&mut self, r: &Region) {
        let (fa, fb) = (&self.faces[r.a], &self.faces[r.b]);
        let o = fa.object;
        let (Some((ea, ra)), Some((eb, rb))) = (self.edge[r.a], self.edge[r.b]) else {
            return;
        };
        let circ = |f: &Face| match &f.shape {
            Shape::Optical { surface, .. } => {
                matches!(surface.ap, Aperture::None | Aperture::Floating { .. })
                    || matches!(surface.ap, Aperture::Circular { min, dx, dy, .. } if min == 0.0 && dx == 0.0 && dy == 0.0)
            }
            _ => false,
        };
        // 軸がそろい、頂点の差が軸に沿う
        let ax = fa.axis();
        let d = sub(fb.frame.o, fa.frame.o);
        let along = dot(d, ax);
        let off = norm(sub(d, [ax[0] * along, ax[1] * along, ax[2] * along]));
        let c = dot(ax, fb.axis());
        let scale = 1.0 + norm(d);
        if !circ(fa) || !circ(fb) || 1.0 - c.abs() > SAME_TOL || off > SAME_TOL * scale {
            self.objects[o].closed = false;
            let name = self.objects[o].name.clone();
            let name = if name.is_empty() {
                "a lens".to_string()
            } else {
                name
            };
            let (a, b) = (r.a, r.b);
            self.notes.push(format!(
                "scene: the edge of {name} between faces {a} and {b} isn't made (not coaxial or not a round aperture)"
            ));
            return;
        }
        let rim = ra.max(rb);
        let sag = |f: &Face, rr: f64| match &f.shape {
            Shape::Optical { surface, .. } => surface.shape().sag(rr, 0.0).0,
            _ => 0.0,
        };
        // 面の縁（形の終わる半径）の点（大域）
        let (za, zb_own) = (sag(fa, ea), sag(fb, eb));
        let (pa, pb) = (fa.to_global([ea, 0.0, za]), fb.to_global([eb, 0.0, zb_own]));
        // a の座標での b の縁の z、b の座標での a の縁の z
        let zb = fa.to_local(pb)[2];
        let za_in_b = fb.to_local(pa)[2];
        let outside = |f: &Face, inside: &Material| -> Material {
            if same_mat(&f.minus, inside) {
                f.plus.clone()
            } else {
                f.minus.clone()
            }
        };
        let (out_a, out_b) = (outside(fa, &r.material), outside(fb, &r.material));
        let (frame_a, frame_b) = (fa.frame, fb.frame);
        let mut new = Vec::new();
        // 端の輪（ガラスは区間の内側、向こうは面の向こうの媒質）。toward: 局所の +z 側に区間がある
        for (frame, e, z, toward, out) in [
            (frame_a, ea, za, zb > za, out_a.clone()),
            (frame_b, eb, zb_own, za_in_b > zb_own, out_b),
        ] {
            if e < rim {
                let (minus, plus) = if toward {
                    (out, r.material.clone())
                } else {
                    (r.material.clone(), out)
                };
                new.push(Face {
                    object: o,
                    frame,
                    shape: Shape::Annulus { r0: e, r1: rim, z },
                    minus,
                    plus,
                    coat: String::new(),
                    refs: Vec::new(),
                });
            }
        }
        if zb != za {
            // 円柱は −z 側の約束を内側に読み替える: minus = 内（ガラス）、plus = 外（a の外の媒質）
            new.push(Face {
                object: o,
                frame: frame_a,
                shape: Shape::Cylinder {
                    r: rim,
                    z0: za.min(zb),
                    z1: za.max(zb),
                },
                minus: r.material.clone(),
                plus: out_a,
                coat: String::new(),
                refs: Vec::new(),
            });
        }
        for f in new {
            self.faces.push(f);
            self.edge.push(None);
            let k = self.faces.len() - 1;
            self.objects[o].faces.push(k);
        }
    }

    /// 機構部品の壁（同じ所・同じ形の部品は 1 つ）
    fn add_mech(&mut self, name: &str, frame: Frame, rings: &[[f64; 4]], square: bool) {
        let q = |v: f64| (v * 1e6).round() as i64;
        let mut key: Vec<i64> = frame.o.iter().map(|&v| q(v)).collect();
        // 軸まわりの回転も見る（角の部品）
        key.extend(frame.r.iter().flatten().map(|&v| q(v)));
        key.extend(rings.iter().flatten().map(|&v| q(v)));
        key.push(square as i64);
        if rings.is_empty() || self.mech_keys.contains(&key) {
            return;
        }
        self.mech_keys.push(key);
        let o = self.objects.len();
        self.objects.push(Object {
            name: name.to_string(),
            kind: ObjectKind::Mech,
            faces: Vec::new(),
            closed: !square,
        });
        let add = |sc: &mut Scene, shape: Shape| {
            sc.faces.push(Face {
                object: o,
                frame,
                shape,
                minus: Material::Air,
                plus: Material::Air,
                coat: String::new(),
                refs: Vec::new(),
            });
            sc.edge.push(None);
            let k = sc.faces.len() - 1;
            sc.objects[o].faces.push(k);
        };
        for q in rings {
            let [z0, z1, ri, ro] = *q;
            if ri > 0.0 {
                add(self, Shape::Cylinder { r: ri, z0, z1 });
            }
            add(self, Shape::Cylinder { r: ro, z0, z1 });
            for z in [z0, z1] {
                add(self, Shape::Annulus { r0: ri, r1: ro, z });
            }
        }
    }
}
