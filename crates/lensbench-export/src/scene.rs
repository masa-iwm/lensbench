//! 書き出しの元になる場面: 玉（ガラスの立体）・鏡・面（像面など）と光線の折れ線を大域座標で持つ。
//!
//! - ガラスの立体は、ガラスを挟む隣り合った 2 面ごとに 1 つ（貼り合わせは 2 つ）。外径は 2 面の半径の大きい方で、
//!   小さい方の面は外径まで平らに延ばす（OpticStudio のレイアウトと同じ描き方）。
//!   ダブルパスなどで同じ形の立体を 2 回通るときは 1 つにまとめる。
//! - 空気中の鏡は、光の来ない側に厚さ（半径の 1/10）を付けた立体にする。
//! - 像面・有限の物体面・理想レンズは厚さのない面（[`Plate`]）にする。
//! - 機構部品（系の `mechanics`）は部品の軸のまわりの輪（[`MechShape`]、内側の半径〜外側の半径）にする。
//! - 光線は面ごとの交点を結ぶ。けられた光線はけられた面まで、エラーの光線は止まった面の手前まで描く。

use lensbench_core::elements::air_after;
use lensbench_core::freeform;
use lensbench_core::math::*;
use lensbench_core::surface::{Aperture, SurfType, Surface};
use lensbench_core::system::Frame;
use lensbench_core::trace::{sag, sag_toroidal};
use lensbench_core::{Error, FieldSel, Result, System};

/// 光線を並べる瞳の上の形
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fan {
    /// y 方向の扇（OpticStudio のレイアウトの既定）
    Y,
    /// x 方向の扇
    X,
    /// x と y の十字
    Cross,
    /// 正方格子（円の外は除く）
    Grid,
    /// 主光線と、瞳の縁の円周（3D の既定）
    Ring,
}

impl std::str::FromStr for Fan {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Fan, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "y" => Fan::Y,
            "x" => Fan::X,
            "cross" | "xy" => Fan::Cross,
            "grid" => Fan::Grid,
            "ring" => Fan::Ring,
            _ => {
                return Err(format!(
                    "光線の並び {s} は y・x・cross・grid・ring のどれか"
                ))
            }
        })
    }
}

/// 瞳座標の並び（n は扇・十字の片方向の本数、格子の一辺、円周の本数。円周には主光線も付ける）
pub fn pupil(fan: Fan, n: usize) -> (Vec<f64>, Vec<f64>) {
    let n = n.max(1);
    let line = |n: usize| -> Vec<f64> {
        if n == 1 {
            vec![0.0]
        } else {
            (0..n)
                .map(|i| -1.0 + 2.0 * i as f64 / (n - 1) as f64)
                .collect()
        }
    };
    match fan {
        Fan::Y => (vec![0.0; n], line(n)),
        Fan::X => (line(n), vec![0.0; n]),
        Fan::Cross => {
            let l = line(n);
            let mut px = l.clone();
            let mut py = vec![0.0; n];
            for &v in &l {
                if v != 0.0 {
                    px.push(0.0);
                    py.push(v);
                }
            }
            (px, py)
        }
        Fan::Grid => {
            let l = line(n);
            let (mut px, mut py) = (Vec::new(), Vec::new());
            for &y in &l {
                for &x in &l {
                    if x * x + y * y <= 1.0 + 1e-12 {
                        px.push(x);
                        py.push(y);
                    }
                }
            }
            (px, py)
        }
        Fan::Ring => std::iter::once((0.0, 0.0))
            .chain((0..n).map(|i| {
                let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                (a.cos(), a.sin())
            }))
            .unzip(),
    }
}

/// 場面の作り方
#[derive(Debug, Clone)]
pub struct Options {
    /// 視野（0 始まり、None で全視野）
    pub fields: Option<Vec<usize>>,
    /// 波長（0 始まり、None で主波長だけ）
    pub waves: Option<Vec<usize>>,
    pub fan: Fan,
    /// 光線の本数（[`pupil`] の n）
    pub rays: usize,
    /// 描き始める面（None で自動: 有限の物体が近ければ物体面、ほかは面 1）
    pub first: Option<usize>,
    /// 描き終える面（None で像面）
    pub last: Option<usize>,
    /// 光路に沿って展開する（鏡・BS の反射で折れた後の区間を、折れる前の光軸の延長に並べ直す）。
    /// 反射ごとに鏡の頂点の接平面で鏡映するので、展開した区間は左右が裏返る（2D の断面図向け）
    pub unfold: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            fields: None,
            waves: None,
            fan: Fan::Y,
            rays: 7,
            first: None,
            last: None,
            unfold: false,
        }
    }
}

/// 展開の写像 p ↦ a·p + b（a は直交行列。反射の数が奇数なら行列式が −1）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unfold {
    pub a: M3,
    pub b: V3,
}

impl Unfold {
    pub const ID: Unfold = Unfold { a: I3, b: [0.0; 3] };

    pub fn point(&self, p: V3) -> V3 {
        add(mv(&self.a, p), self.b)
    }

    pub fn frame(&self, f: Frame) -> Frame {
        Frame {
            o: self.point(f.o),
            r: mm(&self.a, &f.r),
        }
    }

    /// self の後に、点 o・法線 n の平面での鏡映を先にかけたもの（self ∘ 鏡映）
    fn then_mirror(&self, o: V3, n: V3) -> Unfold {
        let n = normalize(n);
        // 鏡映 q = (I − 2nnᵀ) p + 2(o·n) n
        let mut r = I3;
        for (i, ri) in r.iter_mut().enumerate() {
            for (j, rij) in ri.iter_mut().enumerate() {
                *rij -= 2.0 * n[i] * n[j];
            }
        }
        let t = scale(n, 2.0 * dot(o, n));
        Unfold {
            a: mm(&self.a, &r),
            b: self.point(t),
        }
    }
}

/// 面ごとの展開の写像（面へ来る光の区間、面から出る光の区間）。鏡（BS の反射を含む）の面で、
/// それより後ろを鏡の頂点の接平面で鏡映する。反射のない面は 2 つが同じ
pub fn unfold_maps(s: &System) -> Vec<(Unfold, Unfold)> {
    let fr = s.frames();
    let mut t = Unfold::ID;
    let mut out = Vec::with_capacity(s.surfaces.len());
    for (j, d) in s.surfaces.iter().enumerate() {
        let before = t;
        if j > 0 && d.mirror && !d.ignore && d.typ != SurfType::CoordBrk {
            if let Some(f) = fr.surf[j] {
                t = t.then_mirror(f.o, col(&f.r, 2));
            }
        }
        out.push((before, t));
    }
    out
}

/// 回転対称な面（局所座標で z = sag(r)、半径 semi まで。その外は外径 rim まで平ら）
#[derive(Debug, Clone)]
pub struct Face {
    pub frame: Frame,
    pub c: f64,
    pub k: f64,
    pub asph: Vec<f64>,
    /// x・y に依る面（トロイダル面: c・k・asph は y の曲線、cx は x の回転の曲率。
    /// Zernike・Grid Sag の面: `free` に面のデータ）
    pub toroidal: bool,
    /// Zernike・Grid Sag の面（形は [`lensbench_core::freeform::sag_xy`]）
    pub free: Option<Box<Surface>>,
    pub cx: f64,
    pub semi: f64,
    /// 面番号
    pub surf: usize,
}

impl Face {
    fn from_surface(d: &Surface, frame: Frame, semi: f64, surf: usize) -> Face {
        let free = freeform::is_freeform(d.typ);
        // Cylinder Fresnel の基板は y だけの関数（回転半径が無限のトロイダル面と同じ形）。
        // Fresnel は平面に描く（屈折の傾きだけを持つ）
        let toroidal = matches!(d.typ, SurfType::Toroidal | SurfType::CylFresnel) || free;
        let curved = free
            || matches!(
                d.typ,
                SurfType::Standard
                    | SurfType::EvenAsph
                    | SurfType::Toroidal
                    | SurfType::CylFresnel
                    | SurfType::Binary2
                    | SurfType::Gradium
                    | SurfType::Gradient9
            );
        let mut semi = semi;
        // 半球を超える面は、サグが決まる半径までにする
        if curved && d.c != 0.0 && 1.0 + d.k > 0.0 {
            let rmax = 1.0 / (d.c.abs() * (1.0 + d.k).sqrt());
            semi = semi.min(rmax * (1.0 - 1e-9));
        }
        Face {
            frame,
            c: if curved { d.c } else { 0.0 },
            k: if curved { d.k } else { 0.0 },
            asph: if free
                || matches!(
                    d.typ,
                    SurfType::EvenAsph
                        | SurfType::Toroidal
                        | SurfType::CylFresnel
                        | SurfType::Binary2
                ) {
                d.asph.clone()
            } else {
                Vec::new()
            },
            toroidal,
            cx: if d.typ == SurfType::Toroidal {
                d.cx
            } else {
                0.0
            },
            free: free.then(|| Box::new(d.clone())),
            semi,
            surf,
        }
    }

    /// 局所の半径 r でのサグ（semi の外は semi でのサグ）
    pub fn sag(&self, r: f64) -> f64 {
        let r = r.abs().min(self.semi);
        sag(self.c, self.k, &self.asph, r * r).0
    }

    /// トロイダル面のサグと傾き（semi の外は semi の円の上の値）
    fn sag_tor(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let r = x.hypot(y);
        let (x, y) = if r > self.semi {
            (x * self.semi / r, y * self.semi / r)
        } else {
            (x, y)
        };
        if let Some(d) = &self.free {
            let v = freeform::sag_xy(d, x, y);
            if v.0.is_finite() && v.1.is_finite() && v.2.is_finite() {
                return v;
            }
            // Grid Sag の格子の外は基準の面で描く
            let (z, dz) = sag(self.c, self.k, &self.asph, x * x + y * y);
            return (z, 2.0 * dz * x, 2.0 * dz * y);
        }
        sag_toroidal(self.c, self.k, &self.asph, self.cx, x, y)
    }

    /// 局所の (x, y) の点の大域座標（semi の外は平ら）
    pub fn point(&self, x: f64, y: f64) -> V3 {
        let z = if self.toroidal {
            self.sag_tor(x, y).0
        } else {
            self.sag(x.hypot(y))
        };
        self.to_global([x, y, z])
    }

    /// 局所の (x, y) での単位法線（大域、局所の +z 側を向く）
    pub fn normal(&self, x: f64, y: f64) -> V3 {
        let r = x.hypot(y);
        let n = if self.toroidal && r < self.semi {
            let (_, zx, zy) = self.sag_tor(x, y);
            normalize([-zx, -zy, 1.0])
        } else if r >= self.semi || r == 0.0 {
            [0.0, 0.0, 1.0]
        } else {
            let (_, dz) = sag(self.c, self.k, &self.asph, r * r);
            normalize([-2.0 * dz * x, -2.0 * dz * y, 1.0])
        };
        mv(&self.frame.r, n)
    }

    pub fn to_global(&self, l: V3) -> V3 {
        add(self.frame.o, mv(&self.frame.r, l))
    }

    pub fn axis(&self) -> V3 {
        col(&self.frame.r, 2)
    }

    /// 球面（または平面）か: 円錐係数・非球面係数がない
    pub fn is_spherical(&self) -> bool {
        !self.toroidal && self.k == 0.0 && self.asph.iter().all(|&a| a == 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidKind {
    /// ガラスの立体（材料あり）
    Glass,
    /// 空気中の鏡（裏に厚さを付けたもの）
    Mirror,
}

/// 2 面で挟まれた立体（外径 rim の円柱の側面で閉じる）
#[derive(Debug, Clone)]
pub struct Solid {
    pub kind: SolidKind,
    pub front: Face,
    pub back: Face,
    pub rim: f64,
    /// 材料（鏡は "MIRROR"）
    pub material: String,
    /// 表示用の名前（例: "L1 S2-S3 N-BK7"）
    pub name: String,
    /// 四角い開口の平面 2 枚で挟む立体（ビームスプリッターのキューブの半分の直角プリズムなど）は、
    /// 2 つの長方形の角を囲む凸多面体で描く（外径まで延ばした円板では形が違う）
    pub poly: Option<Poly>,
}

/// 凸多面体（大域座標の頂点と、外から見て反時計回りの頂点の番号で並べた面と外向きの法線）
#[derive(Debug, Clone)]
pub struct Poly {
    pub pts: Vec<V3>,
    pub faces: Vec<(V3, Vec<usize>)>,
}

impl Poly {
    /// 点を囲む凸多面体（点は 4 つ以上で同じ平面にないこと。重なった点はまとめる）
    pub fn hull(points: &[V3]) -> Option<Poly> {
        let scale_len = points
            .iter()
            .flat_map(|p| p.iter())
            .fold(1.0f64, |m, v| m.max(v.abs()));
        let eps = 1e-9 * scale_len;
        let mut pts: Vec<V3> = Vec::new();
        for &p in points {
            if !pts.iter().any(|&q| norm(sub(p, q)) <= eps) {
                pts.push(p);
            }
        }
        let n = pts.len();
        if n < 4 {
            return None;
        }
        let mut planes: Vec<(V3, f64)> = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let c = cross(sub(pts[j], pts[i]), sub(pts[k], pts[i]));
                    if norm(c) <= eps * scale_len {
                        continue;
                    }
                    let mut nv = normalize(c);
                    let mut d = dot(nv, pts[i]);
                    let side: Vec<f64> = pts.iter().map(|&q| dot(nv, q) - d).collect();
                    if side.iter().all(|&v| v <= eps) {
                    } else if side.iter().all(|&v| v >= -eps) {
                        nv = scale(nv, -1.0);
                        d = -d;
                    } else {
                        continue;
                    }
                    if !planes
                        .iter()
                        .any(|(m, e)| norm(sub(*m, nv)) < 1e-9 && (e - d).abs() <= eps)
                    {
                        planes.push((nv, d));
                    }
                }
            }
        }
        if planes.len() < 4 {
            return None;
        }
        let mut faces = Vec::new();
        for (nv, d) in planes {
            let on: Vec<usize> = (0..n)
                .filter(|&i| (dot(nv, pts[i]) - d).abs() <= eps)
                .collect();
            let c = scale(
                on.iter().fold([0.0; 3], |a, &i| add(a, pts[i])),
                1.0 / on.len() as f64,
            );
            let u = normalize(sub(pts[on[0]], c));
            let w = cross(nv, u);
            let mut ord = on.clone();
            ord.sort_by(|&a, &b| {
                let ang = |i: usize| {
                    let v = sub(pts[i], c);
                    dot(v, w).atan2(dot(v, u))
                };
                ang(a).total_cmp(&ang(b))
            });
            faces.push((nv, ord));
        }
        Some(Poly { pts, faces })
    }

    /// 大域の 2 つの座標軸（0 = x, 1 = y, 2 = z）へ投影した外形（凸包、頂点は大域座標のまま）
    pub fn outline(&self, a: usize, b: usize) -> Vec<V3> {
        let mut idx: Vec<usize> = (0..self.pts.len()).collect();
        let q = |i: usize| (self.pts[i][a], self.pts[i][b]);
        idx.sort_by(|&i, &j| {
            let (x, y) = (q(i), q(j));
            x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1))
        });
        let cr = |o: usize, p: usize, r: usize| {
            let (o, p, r) = (q(o), q(p), q(r));
            (p.0 - o.0) * (r.1 - o.1) - (p.1 - o.1) * (r.0 - o.0)
        };
        let mut lower: Vec<usize> = Vec::new();
        for &i in &idx {
            while lower.len() >= 2 && cr(lower[lower.len() - 2], lower[lower.len() - 1], i) <= 0.0 {
                lower.pop();
            }
            lower.push(i);
        }
        let mut upper: Vec<usize> = Vec::new();
        for &i in idx.iter().rev() {
            while upper.len() >= 2 && cr(upper[upper.len() - 2], upper[upper.len() - 1], i) <= 0.0 {
                upper.pop();
            }
            upper.push(i);
        }
        lower.pop();
        upper.pop();
        lower.extend(upper);
        lower.into_iter().map(|i| self.pts[i]).collect()
    }
}

/// 四角い開口の平面の角（大域座標）。平面でないか開口が四角でなければ None
fn rect_corners(d: &Surface, f: &Face) -> Option<Vec<V3>> {
    let Aperture::Rect { hx, hy, dx, dy } = d.ap else {
        return None;
    };
    if f.c != 0.0 || f.toroidal || f.free.is_some() || f.asph.iter().any(|&a| a != 0.0) {
        return None;
    }
    Some(
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(sx, sy)| f.to_global([dx + sx * hx, dy + sy * hy, 0.0]))
            .collect(),
    )
}

impl Solid {
    /// 前の面の局所 z で、後ろの面がある側の符号（+1 か -1）
    pub fn direction(&self) -> f64 {
        let v = sub(self.back.point(0.0, 0.0), self.front.point(0.0, 0.0));
        let d = dot(v, self.front.axis());
        if d < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    /// 後ろの面の周の向きの基準（大域の x・y の単位ベクトル）。軸を前の面の軸と同じ側へ向け、前の面の x を後ろの面に
    /// 投げたものと軸 × x。前の面の局所 x・y の角度 t の縁の点と、後ろの面のこの基準の角度 t の縁の点を側面で結ぶ
    /// （面どうしが軸のまわりに回っていても側面がねじれない。STEP の側面・3D の網・2D の断面で同じ）
    pub fn back_basis(&self) -> (V3, V3) {
        let fx = col(&self.front.frame.r, 0);
        let mut ab = self.back.axis();
        if dot(ab, self.front.axis()) < 0.0 {
            ab = scale(ab, -1.0);
        }
        let bx = normalize(sub(fx, scale(ab, dot(ab, fx))));
        (bx, cross(ab, bx))
    }

    /// 後ろの面の、[`Self::back_basis`] の向きの (c, s) の点（局所 x・y で c·bx + s·by）
    pub fn back_point(&self, c: f64, s: f64) -> V3 {
        let (bx, by) = self.back_basis();
        let l = mtv(&self.back.frame.r, add(scale(bx, c), scale(by, s)));
        self.back.point(l[0], l[1])
    }

    /// 2 面が同じ軸の上にあるか（軸が平行で、後ろの面の頂点が前の面の軸の上）
    pub fn coaxial(&self) -> bool {
        let (a, b) = (self.front.axis(), self.back.axis());
        let v = sub(self.back.frame.o, self.front.frame.o);
        let off = sub(v, scale(a, dot(v, a)));
        let scale_len = self.rim.max(norm(v)).max(1.0);
        norm(cross(a, b)) < 1e-9 && norm(off) < 1e-9 * scale_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateKind {
    Image,
    Object,
    /// 理想レンズ
    Paraxial,
}

/// 厚さのない面
#[derive(Debug, Clone)]
pub struct Plate {
    pub kind: PlateKind,
    pub face: Face,
    pub name: String,
}

/// 光線の折れ線（大域座標）
#[derive(Debug, Clone)]
pub struct RayPath {
    /// 視野・波長（0 始まり）
    pub field: usize,
    pub wave: usize,
    /// 瞳座標
    pub px: f64,
    pub py: f64,
    pub points: Vec<V3>,
    /// けられた面（0 はけられていない）
    pub vig: usize,
    /// 止まった面（0 は最後まで）
    pub err: usize,
    /// 重ね描きの構成（0 始まり、[`overlay`] の並びの番号。1 つの系の場面は 0）
    pub cfg: usize,
}

/// 機構部品の形: 部品の座標系（原点・列が軸）と、軸のまわりの輪 [z0, z1, 内側の半径, 外側の半径]
#[derive(Debug, Clone)]
pub struct MechShape {
    pub name: String,
    pub frame: Frame,
    pub rings: Vec<[f64; 4]>,
}

impl MechShape {
    pub fn to_global(&self, p: V3) -> V3 {
        add(self.frame.o, mv(&self.frame.r, p))
    }

    pub fn axis(&self) -> V3 {
        col(&self.frame.r, 2)
    }

    /// 部品の軸と局所の y 軸（`xz` なら x 軸）を含む断面の、輪ごとの壁の四角（大域座標、片側ずつ 2 つ）
    pub fn section(&self, xz: bool) -> Vec<Vec<V3>> {
        let mut v = Vec::new();
        for q in &self.rings {
            for side in [1.0, -1.0] {
                let p = |r: f64, z: f64| {
                    let l = if xz {
                        [side * r, 0.0, z]
                    } else {
                        [0.0, side * r, z]
                    };
                    self.to_global(l)
                };
                v.push(vec![
                    p(q[2], q[0]),
                    p(q[3], q[0]),
                    p(q[3], q[1]),
                    p(q[2], q[1]),
                ]);
            }
        }
        v
    }

    /// 同じ位置・形の部品を見分ける鍵
    pub fn key(&self) -> Vec<i64> {
        let q = |v: f64| (v * 1e6).round() as i64;
        let mut k: Vec<i64> = self.frame.o.iter().map(|&v| q(v)).collect();
        k.extend(self.axis().iter().map(|&v| q(v)));
        k.extend(self.rings.iter().flatten().map(|&v| q(v)));
        k
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub name: String,
    pub solids: Vec<Solid>,
    pub plates: Vec<Plate>,
    /// 機構部品
    pub mechs: Vec<MechShape>,
    pub rays: Vec<RayPath>,
    pub fields: usize,
    pub waves: Vec<f64>,
    /// 重ね描きした構成の名前（[`overlay`]。1 つの系の場面は空）
    pub configs: Vec<String>,
}

impl Scene {
    /// 全部の点の箱（最小・最大）
    pub fn bounds(&self) -> (V3, V3) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut put = |p: V3| {
            if p.iter().all(|v| v.is_finite()) {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
        };
        for s in &self.solids {
            if let Some(p) = &s.poly {
                p.pts.iter().for_each(|&q| put(q));
                continue;
            }
            for f in [&s.front, &s.back] {
                for a in 0..8 {
                    let t = a as f64 * std::f64::consts::FRAC_PI_4;
                    put(f.point(s.rim * t.cos(), s.rim * t.sin()));
                }
                put(f.point(0.0, 0.0));
            }
        }
        for p in &self.plates {
            for a in 0..8 {
                let t = a as f64 * std::f64::consts::FRAC_PI_4;
                put(p.face.point(p.face.semi * t.cos(), p.face.semi * t.sin()));
            }
        }
        for m in &self.mechs {
            for q in &m.rings {
                for a in 0..8 {
                    let t = a as f64 * std::f64::consts::FRAC_PI_4;
                    for z in [q[0], q[1]] {
                        put(m.to_global([q[3] * t.cos(), q[3] * t.sin(), z]));
                    }
                }
            }
        }
        for r in &self.rays {
            r.points.iter().for_each(|&p| put(p));
        }
        if !lo[0].is_finite() {
            return ([0.0; 3], [0.0; 3]);
        }
        (lo, hi)
    }

    /// 箱の対角の長さ
    pub fn size(&self) -> f64 {
        let (lo, hi) = self.bounds();
        norm(sub(hi, lo)).max(1e-6)
    }
}

fn surface_semi(d: &Surface) -> f64 {
    let mut s = d.semi;
    if s <= 0.0 {
        if let Aperture::Circular { max, .. } = d.ap {
            s = max;
        }
    }
    s
}

/// 描き始める面の既定: 有限の物体が近ければ（物体までが面 1〜像面の長さの 10 倍以内）物体面 0、ほかは面 1
pub fn auto_first(s: &System) -> usize {
    let img = s.image();
    let track: f64 = s.surfaces[1..img]
        .iter()
        .filter(|d| !d.ignore)
        .map(|d| d.t.abs())
        .sum::<f64>()
        .max(1.0);
    if !s.infinite() && s.surfaces[0].t.abs() <= 10.0 * track {
        0
    } else {
        1
    }
}

/// 系から場面を作る
pub fn build(s: &System, opt: &Options) -> Result<Scene> {
    let nf = s.fields.len();
    let fields: Vec<usize> = match &opt.fields {
        None => (0..nf).collect(),
        Some(v) => v.clone(),
    };
    if let Some(&i) = fields.iter().find(|&&i| i >= nf) {
        return Err(Error::Invalid(format!(
            "視野 {} はありません（1〜{nf}）",
            i + 1
        )));
    }
    let nw = s.wavelengths.len();
    let waves: Vec<usize> = match &opt.waves {
        None => vec![s.primary],
        Some(v) => v.clone(),
    };
    if let Some(&i) = waves.iter().find(|&&i| i >= nw) {
        return Err(Error::Invalid(format!(
            "波長 {} はありません（1〜{nw}）",
            i + 1
        )));
    }
    let img = s.image();
    let last = opt.last.unwrap_or(img).min(img);
    let fr = s.frames();
    // 展開: 面へ来る区間・出る区間の写像。展開しなければ恒等
    let maps = if opt.unfold {
        unfold_maps(s)
    } else {
        vec![(Unfold::ID, Unfold::ID); s.surfaces.len()]
    };
    let f_in = |j: usize| fr.surf[j].map(|f| maps[j].0.frame(f));
    let f_out = |j: usize| fr.surf[j].map(|f| maps[j].1.frame(f));
    // 描き始める面
    let first = match opt.first {
        Some(f) => f.min(last),
        None => auto_first(s),
    };
    if first == 0 && s.infinite() {
        return Err(Error::Invalid(
            "無限遠の物体は物体面から描けない（面 1 から）".into(),
        ));
    }

    // 光線（半径の決まっていない面の大きさにも使う）
    let (px, py) = pupil(opt.fan, opt.rays);
    let mut rays = Vec::new();
    let mut reach = vec![0.0f64; s.surfaces.len()];
    let mut trace_into = |fi: usize, w: usize, px: &[f64], py: &[f64], keep: bool| -> Result<()> {
        let t = s.try_trace(FieldSel::Index(fi), px, py, Some(w), Some(last), true)?;
        for i in 0..px.len() {
            let r = &t.rays[i];
            let mut pts = Vec::new();
            if first == 0 {
                pts.push(t.p0[i]);
            }
            for rs in &t.rec {
                let (j, e) = (&rs.surf, rs.entry(i));
                let stop_here = r.err != 0 && *j >= r.err;
                if stop_here || !e.p.iter().all(|v| v.is_finite()) {
                    break;
                }
                if r.vig != 0 && *j > r.vig {
                    break;
                }
                reach[*j] = reach[*j].max(e.xyz[0].hypot(e.xyz[1]));
                // 展開した図では座標ブレークの点を飛ばす（光線の延長の上の点で、戻って見える）
                let cb = s.surfaces[*j].typ == SurfType::CoordBrk;
                if *j >= first.max(1) && !(opt.unfold && cb) {
                    pts.push(maps[*j].0.point(e.p));
                }
            }
            if keep {
                rays.push(RayPath {
                    field: fi,
                    wave: w,
                    px: px[i],
                    py: py[i],
                    points: pts,
                    vig: r.vig,
                    err: r.err,
                    cfg: 0,
                });
            }
        }
        Ok(())
    };
    for &fi in &fields {
        for &w in &waves {
            trace_into(fi, w, &px, &py, true)?;
        }
    }
    // 半径の決まっていない面のため、全視野の瞳の縁も追う（描かない）
    let (rx, ry) = pupil(Fan::Ring, 16);
    for fi in 0..nf {
        trace_into(fi, s.primary, &rx, &ry, false)?;
    }

    let semi_of = |j: usize| -> f64 {
        let v = surface_semi(&s.surfaces[j]);
        if v > 0.0 {
            v
        } else {
            reach[j]
        }
    };

    // 立体
    let air = air_after(s);
    let active: Vec<usize> = (first.max(1)..=last)
        .filter(|&j| {
            let d = &s.surfaces[j];
            !d.ignore && d.typ != SurfType::CoordBrk && fr.surf[j].is_some()
        })
        .collect();
    let mut solids: Vec<Solid> = Vec::new();
    let mut keys: Vec<Vec<i64>> = Vec::new();
    let mut nl = 0;
    let key_of = face_key;
    for w in active.windows(2) {
        let (a, b) = (w[0], w[1]);
        if air[a] {
            continue;
        }
        let (da, db) = (&s.surfaces[a], &s.surfaces[b]);
        let thin = |t: SurfType| matches!(t, SurfType::Paraxial | SurfType::User);
        if thin(da.typ) || thin(db.typ) {
            continue;
        }
        let (sa, sb) = (semi_of(a), semi_of(b));
        // ガラスの立体は前の面から出る区間で描く（BS の斜面で反射した後のキューブなど）
        let fa = Face::from_surface(da, f_out(a).unwrap(), sa, a);
        let fb = Face::from_surface(db, f_in(b).unwrap(), sb, b);
        let mut ka = key_of(&fa);
        let mut kb = key_of(&fb);
        if ka > kb {
            std::mem::swap(&mut ka, &mut kb);
        }
        ka.extend(kb);
        if keys.contains(&ka) {
            continue;
        }
        keys.push(ka);
        let rim = sa.max(sb);
        if rim <= 0.0 {
            continue;
        }
        nl += 1;
        let mat = medium_name(s, a);
        let poly = match (rect_corners(da, &fa), rect_corners(db, &fb)) {
            (Some(mut p), Some(q)) => {
                p.extend(q);
                Poly::hull(&p)
            }
            _ => None,
        };
        solids.push(Solid {
            kind: SolidKind::Glass,
            name: format!("L{nl} S{a}-S{b} {mat}"),
            material: mat,
            front: fa,
            back: fb,
            rim,
            poly,
        });
    }
    fill_split_halves(s, &mut solids);
    // 空気中の鏡
    let mut nm = 0;
    for (i, &j) in active.iter().enumerate() {
        let d = &s.surfaces[j];
        if !d.mirror || !air[j] || j == img {
            continue;
        }
        let semi = semi_of(j);
        if semi <= 0.0 {
            continue;
        }
        let f = Face::from_surface(d, f_in(j).unwrap(), semi, j);
        let k = key_of(&f);
        if keys.contains(&k) {
            continue;
        }
        keys.push(k);
        // 光の来た側: 直前の面の頂点から見た向き
        let from = if i > 0 {
            f_out(active[i - 1]).unwrap().o
        } else {
            sub(f.frame.o, f.axis())
        };
        let v = sub(f.frame.o, from);
        let sgn = if dot(v, f.axis()) < 0.0 { -1.0 } else { 1.0 };
        let th = (semi / 10.0).max(1e-3);
        // 裏の面は、鏡の縁のサグから厚さだけ離した平面
        let zr = f.sag(semi);
        let zb = if sgn > 0.0 {
            zr.max(0.0) + th
        } else {
            zr.min(0.0) - th
        };
        let back = Face {
            frame: Frame {
                o: f.to_global([0.0, 0.0, zb]),
                r: f.frame.r,
            },
            c: 0.0,
            k: 0.0,
            asph: Vec::new(),
            toroidal: false,
            free: None,
            cx: 0.0,
            semi,
            surf: j,
        };
        nm += 1;
        solids.push(Solid {
            kind: SolidKind::Mirror,
            name: format!("M{nm} S{j}"),
            material: "MIRROR".into(),
            front: f,
            back,
            rim: semi,
            poly: None,
        });
    }
    // 厚さのない面
    let mut plates = Vec::new();
    if first == 0 {
        if let Some(f0) = fr.surf[0] {
            let semi = obj_semi(s);
            if semi > 0.0 {
                plates.push(Plate {
                    kind: PlateKind::Object,
                    face: Face::from_surface(&s.surfaces[0], f0, semi, 0),
                    name: "OBJ".into(),
                });
            }
        }
    }
    for &j in &active {
        let d = &s.surfaces[j];
        let kind = if j == img {
            PlateKind::Image
        } else if matches!(d.typ, SurfType::Paraxial | SurfType::User) {
            PlateKind::Paraxial
        } else {
            continue;
        };
        let semi = semi_of(j);
        if semi <= 0.0 {
            continue;
        }
        plates.push(Plate {
            kind,
            face: Face::from_surface(d, f_in(j).unwrap(), semi, j),
            name: if kind == PlateKind::Image {
                "IMA".into()
            } else {
                format!("P S{j}")
            },
        });
    }
    // 機構部品: 置いた面から出る区間の写像で（展開では反射の後の部品も軸の延長に並ぶ）
    let mut mechs = Vec::new();
    for p in &s.mechanics {
        let (o, r) = p.frame(s)?;
        let j = p.surface.min(maps.len() - 1);
        mechs.push(MechShape {
            name: p.name.clone(),
            frame: maps[j].1.frame(Frame { o, r }),
            rings: p.profile(),
        });
    }
    Ok(Scene {
        name: s.name.clone(),
        solids,
        plates,
        mechs,
        rays,
        fields: nf,
        waves: s.wavelengths.clone(),
        configs: Vec::new(),
    })
}

/// BS のキューブの反射側の口だけを通ると、入る面〜分ける面と分ける面〜反射側の出る面の立体は
/// どちらもキューブの同じ半分（直角プリズム）になり、透過側の半分は面の並びに無い。続く 2 つの多面体の立体が
/// ガラスの中の鏡の面（分ける面）を挟んで同じ形なら、後ろの方をその面で折り返した半分に替える
/// （折り返した半分がもう描かれていれば替えない）。
fn fill_split_halves(s: &System, solids: &mut [Solid]) {
    let same = |a: &Poly, b: &Poly| {
        let eps = 1e-9
            * a.pts
                .iter()
                .flat_map(|p| p.iter())
                .fold(1.0f64, |m, v| m.max(v.abs()));
        a.pts.len() == b.pts.len()
            && a.pts
                .iter()
                .all(|&p| b.pts.iter().any(|&q| norm(sub(p, q)) <= eps))
    };
    for i in 1..solids.len() {
        let (prev, cur) = (&solids[i - 1], &solids[i]);
        let j = cur.front.surf;
        if prev.back.surf != j
            || prev.kind != SolidKind::Glass
            || cur.kind != SolidKind::Glass
            || !s.surfaces[j].mirror
            || cur.front.c != 0.0
        {
            continue;
        }
        let (Some(pa), Some(pb)) = (&prev.poly, &cur.poly) else {
            continue;
        };
        if !same(pa, pb) {
            continue;
        }
        // 分ける面の平面で折り返す
        let (o, n) = (cur.front.frame.o, cur.front.axis());
        let refl = |p: V3| sub(p, scale(n, 2.0 * dot(sub(p, o), n)));
        let pts: Vec<V3> = pb.pts.iter().map(|&p| refl(p)).collect();
        let Some(m) = Poly::hull(&pts) else {
            continue;
        };
        if solids
            .iter()
            .any(|sd| sd.poly.as_ref().is_some_and(|q| same(q, &m)))
        {
            continue;
        }
        solids[i].poly = Some(m);
    }
}

/// 立体・面の重なりを見分ける鍵（頂点・軸の向き・曲率を 1e-6 で丸めたもの）
fn face_key(f: &Face) -> Vec<i64> {
    let q = |v: f64| (v * 1e6).round() as i64;
    let mut k: Vec<i64> = f.frame.o.iter().map(|&v| q(v)).collect();
    k.extend(f.axis().iter().map(|&v| q(v)));
    k.push(q(f.c * 1e3));
    k.push(q(f.cx * 1e3));
    k
}

impl Face {
    /// 大域座標で d だけ平行に動かす
    pub fn shift(&mut self, d: V3) {
        self.frame.o = add(self.frame.o, d);
    }
}

impl Solid {
    /// 同じ位置・形の立体を見分ける鍵（前後の面を入れ替えても同じ）
    pub fn key(&self) -> Vec<i64> {
        let mut a = face_key(&self.front);
        let mut b = face_key(&self.back);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        a.extend(b);
        a.push((self.rim * 1e6).round() as i64);
        a
    }
}

/// 複数の構成（.zmx の多重構成・プロジェクトの状態・BS の経路）の場面を 1 つに重ねる。
///
/// 構成 k（0 始まり）は大域座標で k·offset だけずらす（OpticStudio のレイアウトの構成ごとのオフセットと同じ）。
/// 同じ位置・形の立体と面は 1 つだけ残す（ずらせば構成ごとに別の位置なので、玉も構成ごとに描く）。
/// 光線の [`RayPath::cfg`] は構成の番号、立体・面の名前の頭に構成の名前を付ける（構成が 2 つ以上のとき）
pub fn overlay(parts: Vec<(String, Scene)>, offset: V3) -> Result<Scene> {
    if parts.is_empty() {
        return Err(Error::Invalid("重ねる構成がない".into()));
    }
    let many = parts.len() > 1;
    let mut out = Scene {
        name: parts[0].1.name.clone(),
        solids: Vec::new(),
        plates: Vec::new(),
        mechs: Vec::new(),
        rays: Vec::new(),
        fields: 0,
        waves: parts[0].1.waves.clone(),
        configs: parts.iter().map(|p| p.0.clone()).collect(),
    };
    let mut skeys: Vec<Vec<i64>> = Vec::new();
    let mut pkeys: Vec<Vec<i64>> = Vec::new();
    for (k, (label, sc)) in parts.into_iter().enumerate() {
        let d = scale(offset, k as f64);
        let tag = |n: &str| {
            if many {
                format!("{label} {n}")
            } else {
                n.to_string()
            }
        };
        for mut sd in sc.solids {
            sd.front.shift(d);
            sd.back.shift(d);
            if let Some(p) = &mut sd.poly {
                p.pts.iter_mut().for_each(|q| *q = add(*q, d));
            }
            let key = sd.key();
            if skeys.contains(&key) {
                continue;
            }
            skeys.push(key);
            sd.name = tag(&sd.name);
            out.solids.push(sd);
        }
        for mut p in sc.plates {
            p.face.shift(d);
            let mut key = face_key(&p.face);
            key.push((p.face.semi * 1e6).round() as i64);
            if pkeys.contains(&key) {
                continue;
            }
            pkeys.push(key);
            p.name = tag(&p.name);
            out.plates.push(p);
        }
        for mut m in sc.mechs {
            m.frame.o = add(m.frame.o, d);
            if out.mechs.iter().any(|x| x.key() == m.key()) {
                continue;
            }
            m.name = tag(&m.name);
            out.mechs.push(m);
        }
        for mut r in sc.rays {
            r.points.iter_mut().for_each(|p| *p = add(*p, d));
            r.cfg = k;
            out.rays.push(r);
        }
        out.fields = out.fields.max(sc.fields);
    }
    Ok(out)
}

fn medium_name(s: &System, j: usize) -> String {
    // 鏡・座標ブレーク・無視する面は前の媒質を引き継ぐ
    let mut k = j;
    loop {
        let d = &s.surfaces[k];
        if k == 0 || !(d.mirror || d.typ == SurfType::CoordBrk || d.ignore) {
            return d.mat.clone();
        }
        k -= 1;
    }
}

/// 物体面の大きさ: 視野の最大の物体高（物体高の視野でなければ 0 にせず、主光線の開始点から）
fn obj_semi(s: &System) -> f64 {
    let mut m: f64 = 0.0;
    for i in 0..s.fields.len() {
        let (hx, hy) = s.hxhy(i);
        if let Ok(lensbench_core::system::ObjSpec::Point(p)) = s.object_spec(hx, hy) {
            m = m.max(p[0].hypot(p[1]));
        }
    }
    m
}

/// 2D の断面: 立体の外形（局所の yz 面で切った閉じた折れ線）を大域座標で返す。
/// 凸多面体の立体（[`Solid::poly`]）は大域の yz 面へ投影した外形
pub fn solid_section(sd: &Solid, n: usize) -> Vec<V3> {
    if let Some(p) = &sd.poly {
        return p.outline(2, 1);
    }
    let mut pts = Vec::with_capacity(2 * n + 4);
    let r = sd.rim;
    let f = &sd.front;
    let ys = |n: usize| -> Vec<f64> {
        (0..=n)
            .map(|i| -r + 2.0 * r * i as f64 / n as f64)
            .collect()
    };
    for y in ys(n) {
        pts.push(f.point(0.0, y));
    }
    for y in ys(n).into_iter().rev() {
        pts.push(sd.back_point(0.0, y));
    }
    pts
}

/// 2D の断面: 面の線（局所の yz 面）
pub fn face_section(f: &Face, n: usize) -> Vec<V3> {
    let r = f.semi;
    (0..=n)
        .map(|i| f.point(0.0, -r + 2.0 * r * i as f64 / n as f64))
        .collect()
}

/// [`solid_section`] の局所の xz 面の版（多面体は大域の z・x へ投影した外形）
pub fn solid_section_xz(sd: &Solid, n: usize) -> Vec<V3> {
    if let Some(p) = &sd.poly {
        return p.outline(2, 0);
    }
    let r = sd.rim;
    let xs: Vec<f64> = (0..=n)
        .map(|i| -r + 2.0 * r * i as f64 / n as f64)
        .collect();
    let mut pts: Vec<V3> = xs.iter().map(|&x| sd.front.point(x, 0.0)).collect();
    pts.extend(xs.iter().rev().map(|&x| sd.back_point(x, 0.0)));
    pts
}

/// [`face_section`] の局所の xz 面の版
pub fn face_section_xz(f: &Face, n: usize) -> Vec<V3> {
    let r = f.semi;
    (0..=n)
        .map(|i| f.point(-r + 2.0 * r * i as f64 / n as f64, 0.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 鏡映: 鏡の面の点は動かず、法線の向きは裏返り、距離は変わらない。2 回の鏡映は回転（行列式 +1）
    #[test]
    fn unfold_mirror() {
        let det = |a: &M3| {
            a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
                - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
                + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
        };
        let o = [0.0, 1.0, 10.0];
        let n = [0.0, -1.0, 1.0];
        let m = Unfold::ID.then_mirror(o, n);
        let q = m.point(add(o, [3.0, 1.0, 1.0]));
        assert!(norm(sub(q, add(o, [3.0, 1.0, 1.0]))) < 1e-12);
        // 鏡で 90° 折れて +y へ進む光は、展開すると +z へ進む
        let p = m.point(add(o, [0.0, 5.0, 0.0]));
        assert!(norm(sub(p, add(o, [0.0, 0.0, 5.0]))) < 1e-12, "{p:?}");
        assert!((det(&m.a) + 1.0).abs() < 1e-12);
        let m2 = m.then_mirror([0.0, 6.0, 10.0], [0.0, 1.0, 1.0]);
        assert!((det(&m2.a) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn pupil_patterns() {
        let (x, y) = pupil(Fan::Y, 5);
        assert_eq!(x, vec![0.0; 5]);
        assert_eq!(y, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        let (x, _) = pupil(Fan::Cross, 3);
        assert_eq!(x.len(), 5);
        let (x, y) = pupil(Fan::Grid, 3);
        // 角の 4 点は円の外
        assert_eq!(x.len(), 5);
        assert!(x.iter().zip(&y).all(|(a, b)| a * a + b * b <= 1.0));
        let (x, y) = pupil(Fan::Ring, 8);
        assert_eq!(x.len(), 9);
        assert_eq!((x[0], y[0]), (0.0, 0.0));
    }
}
