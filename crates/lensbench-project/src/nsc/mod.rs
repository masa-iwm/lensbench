//! 非順次の追跡（非順次の段 2）: 物の場面（[`Scene`]）の上で、光線が次に当たる面を交点の距離で決め、境界ごとに
//! 反射と透過へ分けて追う。結果は検出器の照度（インコヒーレントにパワーを足す）と、光線が通った出来事の列（経路）の表。
//!
//! - 交点: 場面の全部の面のうち最も近いもの。面の境界の箱の木（BVH、平らな配列）で絞る。空気中の絞りは開口の外が
//!   遮る板（形の続く所まで）なので木に入れず、いつも調べる
//! - 境界: 屈折・反射・全反射。分かれ方は膜込みの Fresnel（偏光追跡と同じ境界の係数
//!   [`lensbench_core::coating::Coatings::prepare`]: 透過は `膜|入射側|反対側`、反射は `膜|入射側|反対側|R`、鏡は
//!   `膜|入射側|MIRROR`）。係数はパワーで規格化した s・p の振幅（|r|² + |t|² = 1 − 膜の吸収）
//! - 偏光: 光線ごとに大域の電場のベクトル（複素）を持ち、|E|² を光線のパワー [W] とする。偏光なしの光源は直交する
//!   2 つの状態に半分ずつ（[`StartRay::unpolarized`]）
//! - 媒質: 硝材の内部透過率（カタログの IT、1 mm あたり）で、通った長さの分を吸収する（振幅は 1/2 乗。偏光追跡と同じ）
//! - 分け方（[`Split`]）: 決まった分岐は R と T の両方を追い、パワーが光源の光線の [`NscOpts::threshold`] 倍より
//!   小さい枝を捨てる。モンテカルロは R か T をパワーの比の確率で選び（パワーは両方の和のまま）、閾値より小さく
//!   なった光線をロシアンルーレット（半分の確率で 2 倍にして残す）で減らす。乱数は光線の番号と出来事の回数から
//!   決まるカウンタ式（[`lensbench_core::rng::CounterRng`]）
//! - 物ごとの働き: 玉の面・縁は境界、空気中の鏡は両側で反射（残りは吸収）、絞りは開口の外で吸収、機構部品の壁は
//!   吸収、検出器（像面）は矩形の中に順次の系の光が来る側から当たったパワーを画素に数える（膜があれば反射の分は
//!   先へ進む。裏から来た光は素通りし、物の陰を作らない）。
//!   ガラスの中の働きの無い面（同じ媒質の間で膜も無い）は素通りし、出来事にしない
//! - 足し方: 光線を決まった数ずつの組に分けて組ごとに数え、組の順に足す（スレッド数に依らず同じ値）
//!
//! エネルギーの収支: 光源 = 検出 + 吸収（膜・媒質・鏡・遮るもの）+ 外へ出た + 閾値で捨てた + 出来事の数の上限で
//! 止めた（[`Budget`]）。決まった分岐では丸めの誤差の範囲で等しい。
//!
//! 扱わないもの（誤りにする）: 理想レンズ・Jones 面・回折・Fresnel 面・複屈折材・利用者定義の面。散乱は段 3。

mod bvh;
mod geom;
mod scatter;
mod source;

use scatter::{basis, branch_key, ScatterCx};
pub use scatter::{face_finishes, FaceFinish};
pub use source::{back_off, disc_source, system_source};
mod lines;
pub mod seq;

pub use lines::{spread, Polyline};

use crate::raypath::EventKind;
use crate::scene::{ObjectKind, Scene, Shape};
use geom::{Bound, LocalHit};
use lensbench_core::coating::{Coatings, Iface, C64};
use lensbench_core::math::{cross, dot, norm, normalize, scale, V3};
use lensbench_core::rng::CounterRng;
use lensbench_core::scatter::{Bsdf, FinishAt, Scatter, ScatterSide};
use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, ObjSpec};
use lensbench_core::{invalid, Result, System};
use std::collections::HashMap;

/// 複素の電場のベクトル（大域）
pub type CV3 = [C64; 3];

/// 分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// 決まった分岐（R と T の両方を追う）
    Deterministic,
    /// モンテカルロ（R か T を確率で選ぶ）。seed は乱数の種
    MonteCarlo { seed: u64 },
}

/// 検出器の矩形と画素
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorSpec {
    /// 半幅 (x, y) [mm]（像面の局所座標。None なら像面の半直径の正方形。像面の半直径が 0 ならその系の面の半直径の
    /// 最大）
    pub half: Option<[f64; 2]>,
    /// 画素の数 (x, y)
    pub pixels: [usize; 2],
}

impl Default for DetectorSpec {
    fn default() -> Self {
        DetectorSpec {
            half: None,
            pixels: [100, 100],
        }
    }
}

/// 経路の条件（検出器の図を絞る）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathFilter {
    /// 反射（全反射を含む）の回数の範囲（含む）
    pub reflections: Option<(usize, usize)>,
    /// 通る出来事: (場面の面, 種類（None は何でも）) を全部含む
    pub via: Vec<(usize, Option<EventKind>)>,
}

impl PathFilter {
    fn accepts(&self, hits: &[Hit]) -> bool {
        if let Some((lo, hi)) = self.reflections {
            let n = hits.iter().filter(|h| h.kind.reflects()).count();
            if n < lo || n > hi {
                return false;
            }
        }
        self.via.iter().all(|&(f, k)| {
            hits.iter()
                .any(|h| h.face == f && k.is_none_or(|k| k == h.kind))
        })
    }
}

/// 追跡の設定
#[derive(Debug, Clone, PartialEq)]
pub struct NscOpts {
    /// 波長の番号（None は主波長）
    pub wave: Option<usize>,
    pub split: Split,
    /// 捨てる・ルーレットにかける枝のパワー（光源の光線のパワーに対する比）
    pub threshold: f64,
    /// 1 本の枝の出来事の数の上限
    pub max_events: usize,
    pub detector: DetectorSpec,
    /// 検出器の図に数える経路の条件（経路の表・収支には効かない）
    pub filter: Option<PathFilter>,
    /// 決まった分岐で、散乱する面 1 回あたりに作る散乱の光線の数（散乱の割合のパワーを等しく分ける）。
    /// モンテカルロでは使わない（散乱か鏡面かを確率で選ぶ）
    pub scatter_rays: usize,
    /// 決まった分岐の散乱の方向の乱数の種（モンテカルロは [`Split::MonteCarlo`] の種）
    pub seed: u64,
}

impl Default for NscOpts {
    fn default() -> Self {
        NscOpts {
            wave: None,
            split: Split::Deterministic,
            threshold: 1e-6,
            max_events: 256,
            detector: DetectorSpec::default(),
            filter: None,
            scatter_rays: 1,
            seed: 1,
        }
    }
}

/// 光源の光線: 始点・方向（大域）と電場（|E|² がパワー [W]、方向に垂直）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartRay {
    pub p: V3,
    pub k: V3,
    pub e: CV3,
}

impl StartRay {
    /// 偏光 (Ex, Ey)（座標系 r の x・y の成分）を方向に垂直な面へ射影し、パワー power に規格化した光線
    pub fn polarized(p: V3, k: V3, jones: [C64; 2], r: &[[f64; 3]; 3], power: f64) -> StartRay {
        let k = normalize(k);
        let j: CV3 = [0, 1, 2].map(|c| jones[0] * r[c][0] + jones[1] * r[c][1]);
        let jk = j[0] * k[0] + j[1] * k[1] + j[2] * k[2];
        let e: CV3 = [0, 1, 2].map(|c| j[c] - jk * k[c]);
        let n2: f64 = e.iter().map(|z| z.norm_sqr()).sum();
        let s = if n2 > 0.0 { (power / n2).sqrt() } else { 0.0 };
        StartRay {
            p,
            k,
            e: e.map(|z| z * s),
        }
    }

    /// 偏光なしの光線: 座標系 r の x を方向に垂直な面へ射影した向きと、それに直交する向きの 2 本（パワーは半分ずつ）
    pub fn unpolarized(p: V3, k: V3, r: &[[f64; 3]; 3], power: f64) -> [StartRay; 2] {
        let k = normalize(k);
        let x = [r[0][0], r[1][0], r[2][0]];
        let mut a = lensbench_core::math::sub(x, scale(k, dot(x, k)));
        if norm(a) < 1e-6 {
            let y = [r[0][1], r[1][1], r[2][1]];
            a = lensbench_core::math::sub(y, scale(k, dot(y, k)));
        }
        let a = normalize(a);
        let b = cross(k, a);
        let s = (0.5 * power).sqrt();
        let c = |v: V3| v.map(|x| C64::new(x * s, 0.0));
        [StartRay { p, k, e: c(a) }, StartRay { p, k, e: c(b) }]
    }

    /// パワー（|E|²）
    pub fn power(&self) -> f64 {
        power_of(&self.e)
    }
}

fn power_of(e: &CV3) -> f64 {
    e.iter().map(|z| z.norm_sqr()).sum()
}

/// 出来事（交点つき）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// 場面の面（[`Scene::faces`]）
    pub face: usize,
    pub kind: EventKind,
    /// 交点（大域）
    pub p: V3,
    /// 出来事の後のこの枝のパワー [W]
    pub power: f64,
}

/// 枝の終わり方
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum End {
    /// 検出器（[`Tracer::detectors`] の番号）の局所座標 (x, y) に届いた
    Detected { detector: usize, x: f64, y: f64 },
    /// 吸収された（絞り・機構部品・鏡の裏）
    Absorbed,
    /// どの面にも当たらずに外へ出た
    Escaped,
    /// パワーが閾値より小さくて捨てた（ルーレットで消えたものも）
    Discarded,
    /// 出来事の数の上限で止めた
    Truncated,
}

/// 追跡の終わった枝
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub hits: Vec<Hit>,
    pub end: End,
    /// 終わった所の位置・方向・電場（大域）とパワー（検出器では数えたパワー）
    pub p: V3,
    pub k: V3,
    pub e: CV3,
    pub power: f64,
}

/// エネルギーの収支 [W]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Budget {
    pub source: f64,
    pub detected: f64,
    /// 膜・媒質・鏡で吸収された分と、遮るもの（絞り・機構部品・鏡の裏）に当たった分
    pub absorbed: f64,
    pub escaped: f64,
    pub discarded: f64,
    pub truncated: f64,
}

impl Budget {
    fn add(&mut self, o: &Budget) {
        self.source += o.source;
        self.detected += o.detected;
        self.absorbed += o.absorbed;
        self.escaped += o.escaped;
        self.discarded += o.discarded;
        self.truncated += o.truncated;
    }
    /// 光源 − 行き先の和
    pub fn residual(&self) -> f64 {
        self.source
            - (self.detected + self.absorbed + self.escaped + self.discarded + self.truncated)
    }
}

/// 検出器の結果
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorImage {
    /// 場面の面
    pub face: usize,
    pub label: String,
    /// 半幅 [mm] と画素の数
    pub half: [f64; 2],
    pub pixels: [usize; 2],
    /// 画素ごとのパワー [W]（行 = y、y・x とも小さい順。画素 (i, j) は下の端を含み上の端を含まない）
    pub power: Vec<f64>,
    /// 画素ごとの光線（枝）の数
    pub hits: Vec<u64>,
}

impl DetectorImage {
    /// 画素の面積 [mm²]
    pub fn pixel_area(&self) -> f64 {
        4.0 * self.half[0] * self.half[1] / (self.pixels[0] * self.pixels[1]) as f64
    }
    /// 画素ごとの照度 [W/mm²]
    pub fn irradiance(&self) -> Vec<f64> {
        let a = self.pixel_area();
        self.power.iter().map(|p| p / a).collect()
    }
    /// 全パワー [W]
    pub fn total(&self) -> f64 {
        self.power.iter().sum()
    }
    /// 画素の中心の座標 (x の列, y の列) [mm]
    pub fn centers(&self) -> (Vec<f64>, Vec<f64>) {
        let c = |h: f64, n: usize| -> Vec<f64> {
            let d = 2.0 * h / n as f64;
            (0..n).map(|i| -h + (i as f64 + 0.5) * d).collect()
        };
        (
            c(self.half[0], self.pixels[0]),
            c(self.half[1], self.pixels[1]),
        )
    }
}

/// 経路の表の 1 行: 検出器に届いた、同じ出来事の列の枝の和
#[derive(Debug, Clone, PartialEq)]
pub struct PathRow {
    pub detector: usize,
    /// (場面の面, 種類) の列（最後は検出）
    pub events: Vec<(usize, EventKind)>,
    pub power: f64,
    pub rays: u64,
    /// この経路だけの検出器の画素ごとのパワー [W]（画素の番号は [`DetectorImage::power`] と同じ、番号の順、0 は省く）。
    /// 反射の回数の絞り込み（[`NscOpts::filter`]）に依らない
    pub pixels: Vec<(usize, f64)>,
}

impl PathRow {
    /// 反射（全反射を含む）の回数
    pub fn reflections(&self) -> usize {
        self.events.iter().filter(|e| e.1.reflects()).count()
    }
}

/// 追跡の結果
#[derive(Debug, Clone, PartialEq)]
pub struct NscResult {
    pub detectors: Vec<DetectorImage>,
    /// 経路の表（パワーの大きい順、同じなら出来事の列の順）
    pub paths: Vec<PathRow>,
    pub budget: Budget,
    /// 光源の光線の数
    pub launched: usize,
    /// 波長 [µm]
    pub wavelength: f64,
}

/// 面の働き
#[derive(Debug, Clone)]
enum Role {
    /// 媒質の境界（屈折・反射・全反射）
    Boundary,
    /// 鏡（両側で反射）
    Mirror,
    /// 空気中の開口（開口の外で吸収）
    Stop,
    /// 検出器（[`Tracer::detectors`] の番号と、数える側: 0 = −z 側から、1 = +z 側から）
    Detector(usize, usize),
    /// 光を通さない面（機構部品の壁・塗った縁）。反射率 [`FaceData::wall`] の分を反射し、残りは吸収
    Wall,
    /// 働きの無い面（素通り）
    Pass,
}

/// 面ごとの用意
#[derive(Debug, Clone)]
struct FaceData {
    role: Role,
    bound: Bound,
    /// −z 側・+z 側の屈折率
    n: [f64; 2],
    /// −z 側・+z 側の媒質の振幅の吸収 0.5 ln(内部透過率) [1/mm]
    la: [f64; 2],
    /// 側（0 = −z 側から、1 = +z 側から）ごとの透過・反射の係数
    t_if: [Option<Iface>; 2],
    r_if: [Option<Iface>; 2],
    /// 光を通さない所（壁・絞りの板）の反射率
    wall: f64,
    /// 散乱
    /// 散乱の分布・散乱する側・偏光を失うか
    scatter: Option<(Bsdf, ScatterSide, bool)>,
}

/// 用意した追跡（場面・波長ごと）
pub struct Tracer<'a> {
    scene: &'a Scene,
    faces: Vec<FaceData>,
    bvh: bvh::Bvh,
    /// 木に入らない面（範囲の無い絞り）
    loose: Vec<usize>,
    /// 検出器の面
    detectors: Vec<usize>,
    labels: Vec<String>,
    opts: NscOpts,
    wl: f64,
}

/// 面の名前（物の名前か種類と番号、面は系の面の番号か縁・壁の番号）
pub fn face_labels(sc: &Scene) -> Vec<String> {
    let mut seen: HashMap<usize, usize> = HashMap::new();
    sc.faces
        .iter()
        .map(|f| {
            let o = &sc.objects[f.object];
            let obj = if o.name.is_empty() {
                let kind = match o.kind {
                    ObjectKind::Lens => "lens",
                    ObjectKind::Mirror => "mirror",
                    ObjectKind::Stop => "stop",
                    ObjectKind::Plane => "plane",
                    ObjectKind::Detector => "detector",
                    ObjectKind::Mech => "mech",
                };
                let i = sc.objects[..f.object]
                    .iter()
                    .filter(|x| x.kind == o.kind && x.name.is_empty())
                    .count();
                format!("{kind}{}", i + 1)
            } else {
                o.name.clone()
            };
            let part = match (&f.shape, f.refs.first()) {
                (Shape::Optical { .. }, Some(r)) => format!("S{}", r.surf),
                _ => {
                    let n = seen.entry(f.object).or_insert(0);
                    *n += 1;
                    // 機構部品は壁、玉は縁
                    if o.kind == ObjectKind::Mech {
                        format!("wall{n}")
                    } else {
                        format!("edge{n}")
                    }
                }
            };
            format!("{obj}.{part}")
        })
        .collect()
}

/// 系の媒質の名前ごとの屈折率（波長 w）
fn indices(systems: &[&System], w: usize, into: &mut HashMap<String, f64>) {
    for s in systems {
        into.entry("AIR".into()).or_insert(1.0);
        let n0 = s.n(0)[w];
        into.entry(s.surfaces[0].mat.label()).or_insert(n0);
        for (j, d) in s.surfaces.iter().enumerate().skip(1) {
            if d.ignore || d.is_coord_break() {
                continue;
            }
            if let Some(r) = &d.reflect {
                if let Some(&n) = r.n.get(w) {
                    let lab = if r.mat.is_empty() {
                        "AIR".to_string()
                    } else {
                        lensbench_core::material::Material::parse(&r.mat).label()
                    };
                    into.entry(lab).or_insert(n);
                }
            }
            if !d.is_mirror() && d.mat.is_medium() {
                into.entry(d.mat.label()).or_insert(s.n(j)[w]);
            }
        }
    }
}

impl<'a> Tracer<'a> {
    /// 場面 sc を、場面を作った系（[`Scene::systems`] の順）と膜で用意する
    pub fn new(
        sc: &'a Scene,
        systems: &[&System],
        coat: &Coatings,
        opts: &NscOpts,
    ) -> Result<Tracer<'a>> {
        if systems.is_empty() || systems.len() != sc.systems.len() {
            invalid!(
                "nsc: {} systems for a scene made of {}",
                systems.len(),
                sc.systems.len()
            );
        }
        let s0 = systems[0];
        let w = opts.wave.unwrap_or(s0.primary);
        if w >= s0.wavelengths.len() {
            let (n, nw) = (w + 1, s0.wavelengths.len());
            invalid!("no wavelength {n} (1 to {nw})");
        }
        if !(opts.threshold >= 0.0 && opts.threshold < 1.0) {
            invalid!("nsc: threshold {} (0 to below 1)", opts.threshold);
        }
        if opts.max_events == 0 {
            invalid!("nsc: max_events must be 1 or more");
        }
        let [px, py] = opts.detector.pixels;
        if px == 0 || py == 0 || px * py > 16_000_000 {
            invalid!("nsc: detector pixels {px} × {py}");
        }
        let wl = s0.wavelengths[w];
        let mut nmap = HashMap::new();
        indices(systems, w, &mut nmap);
        let labels = face_labels(sc);
        let finishes = face_finishes(sc, systems)?;
        let mut faces = Vec::with_capacity(sc.faces.len());
        let mut detectors = Vec::new();
        for (i, f) in sc.faces.iter().enumerate() {
            let obj = &sc.objects[f.object];
            let lab = &labels[i];
            let surf = match &f.shape {
                Shape::Optical { surface, .. } => Some(surface.as_ref()),
                _ => None,
            };
            if let Some(d) = surf {
                use lensbench_core::action::Action;
                match d.action() {
                    Action::Refract | Action::Reflect { .. } => {}
                    _ => {
                        let what = format!("{:?}", d.typ());
                        invalid!("nsc: {lab} is a {what} surface, which the non-sequential trace doesn't handle")
                    }
                }
            }
            let mirror = surf.is_some_and(|d| d.is_mirror() && d.reflect.is_none());
            let mlab = [f.minus.label(), f.plus.label()];
            let n_of = |m: &str| -> Result<f64> {
                nmap.get(m).copied().ok_or_else(|| {
                    lensbench_core::Error::Invalid(format!(
                        "nsc: no index of {m} at {wl} µm ({lab})"
                    ))
                })
            };
            let n = [n_of(&mlab[0])?, n_of(&mlab[1])?];
            let la = [0, 1].map(|s| {
                systems
                    .iter()
                    .find_map(|sy| sy.glasses.get(&mlab[s]))
                    .and_then(|g| lensbench_core::coating::internal_t(g, wl))
                    .map_or(0.0, |t| 0.5 * t.ln())
            });
            let (role, bound) = match obj.kind {
                ObjectKind::Detector => {
                    // 像面の半直径（決まっていなければ、その系の面の半直径の最大）
                    let sy = f
                        .refs
                        .first()
                        .map_or(s0, |r| systems.get(r.system).copied().unwrap_or(s0));
                    let semi = surf
                        .map(|d| d.semi)
                        .filter(|&v| v > 0.0)
                        .unwrap_or_else(|| sy.surfaces.iter().map(|d| d.semi).fold(0.0, f64::max));
                    let [hx, hy] = opts.detector.half.unwrap_or([semi, semi]);
                    if !(hx > 0.0 && hy > 0.0 && hx.is_finite() && hy.is_finite()) {
                        invalid!("nsc: detector half widths {hx} × {hy} mm ({lab})");
                    }
                    detectors.push(i);
                    // 順次の系の光が来る側: 鏡で折り返した回数が偶数なら局所の −z 側から
                    let front = f.refs.first().map_or(0, |r| {
                        let sy = systems.get(r.system).copied().unwrap_or(s0);
                        let m = sy.surfaces[1..r.surf.min(sy.surfaces.len())]
                            .iter()
                            .filter(|d| !d.ignore && d.is_mirror())
                            .count();
                        (m % 2) ^ usize::from(r.flipped)
                    });
                    (Role::Detector(detectors.len() - 1, front), Bound::Rect { hx, hy })
                }
                ObjectKind::Stop => {
                    // 遮る板の外径: 絞りの面の固定した機械半径、無ければ来た系の面の半直径・機械半径の最大
                    // （板はシステムで最も大きい玉ほど。その外を通る光（BS のほかの腕など）は遮らない）
                    let sy = f
                        .refs
                        .first()
                        .map_or(s0, |r| systems.get(r.system).copied().unwrap_or(s0));
                    let big = sy
                        .surfaces
                        .iter()
                        .filter(|d| !d.ignore)
                        .map(|d| d.semi.max(d.mech_semi.unwrap_or(0.0)))
                        .fold(0.0, f64::max);
                    let semi = surf.map_or(0.0, |d| d.semi);
                    let r = surf.and_then(|d| d.mech_semi).unwrap_or(big).max(semi);
                    (Role::Stop, Bound::Disc { r })
                }
                ObjectKind::Mech => (Role::Wall, Bound::Extent),
                ObjectKind::Plane => invalid!(
                    "nsc: {lab} is an ideal lens or a Jones surface, which the non-sequential trace doesn't handle"
                ),
                _ if finishes[i].as_ref().is_some_and(|f| f.edge && f.reflect.is_some()) => {
                    (Role::Wall, Bound::Extent)
                }
                _ if mirror => (Role::Mirror, Bound::Extent),
                _ if mlab[0] == mlab[1] && f.coat.is_empty() => (Role::Pass, Bound::Extent),
                _ => (Role::Boundary, Bound::Extent),
            };
            // 膜の係数を用意する系（来た系、縁は最初の系）
            let sy = f
                .refs
                .first()
                .map_or(s0, |r| systems.get(r.system).copied().unwrap_or(s0));
            let c = if f.coat.is_empty() { "-" } else { &f.coat };
            let prep = |key: String, n1: f64, n2: f64| -> Result<Iface> {
                coat.prepare(sy, &key, wl, n1, n2)
                    .map_err(|e| e.at(lab.clone()))
            };
            let mut t_if = [None, None];
            let mut r_if = [None, None];
            let boundary = match role {
                Role::Boundary => true,
                Role::Detector(..) => !f.coat.is_empty(),
                _ => false,
            };
            for s in 0..2 {
                let o = 1 - s;
                if boundary {
                    t_if[s] = Some(prep(format!("{c}|{}|{}", mlab[s], mlab[o]), n[s], n[o])?);
                    r_if[s] = Some(prep(format!("{c}|{}|{}|R", mlab[s], mlab[o]), n[s], n[o])?);
                } else if matches!(role, Role::Mirror) {
                    r_if[s] = Some(prep(format!("{c}|{}|MIRROR", mlab[s]), n[s], n[s])?);
                }
            }
            let fin = finishes[i].clone().unwrap_or_default();
            let opaque = matches!(role, Role::Wall | Role::Stop);
            if fin.reflect.is_some() && !opaque {
                invalid!("nsc: {lab}: reflect is only for a mechanical part, a stop plate or a painted lens edge (an optical surface reflects by Fresnel with its coating)");
            }
            if fin.scatter.is_some() && matches!(role, Role::Detector(..) | Role::Pass) {
                invalid!("nsc: {lab}: a detector or a surface without an action can't scatter");
            }
            let scatter = match &fin.scatter {
                Some(sc) => Some((
                    sc.prepare().map_err(|e| e.at(lab.clone()))?,
                    sc.on,
                    sc.depolarizes(),
                )),
                None => None,
            };
            faces.push(FaceData {
                role,
                bound,
                n,
                la,
                t_if,
                r_if,
                wall: fin.reflect.unwrap_or(0.0),
                scatter,
            });
        }
        let mut boxes = Vec::new();
        let mut loose = Vec::new();
        for (i, f) in sc.faces.iter().enumerate() {
            match geom::bbox(f, faces[i].bound) {
                Some(b) => boxes.push((i, b)),
                None => loose.push(i),
            }
        }
        Ok(Tracer {
            scene: sc,
            faces,
            bvh: bvh::Bvh::build(&boxes),
            loose,
            detectors,
            labels,
            opts: opts.clone(),
            wl,
        })
    }

    /// 検出器の面（場面の面の番号）
    pub fn detectors(&self) -> &[usize] {
        &self.detectors
    }

    /// 面の名前（[`face_labels`]）
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// 波長 [µm]
    pub fn wavelength(&self) -> f64 {
        self.wl
    }

    /// 最も近い交点: (面, 交点)。skip の面は距離 [`geom::SELF_EPS`] 以下を除く
    fn nearest(&self, p: V3, k: V3, skip: &[usize]) -> Option<(usize, LocalHit)> {
        let mut best: Option<(usize, LocalHit)> = None;
        let mut test = |i: usize| -> f64 {
            let tmin = if skip.contains(&i) {
                geom::SELF_EPS
            } else {
                geom::MIN_T
            };
            if let Some(h) = geom::intersect(&self.scene.faces[i], self.faces[i].bound, p, k, tmin)
            {
                if best.is_none_or(|b| h.t < b.1.t) {
                    best = Some((i, h));
                }
            }
            best.map_or(f64::INFINITY, |b| b.1.t)
        };
        let mut tmax = f64::INFINITY;
        for &i in &self.loose {
            tmax = test(i);
        }
        let inv = k.map(|x| 1.0 / x);
        self.bvh.visit(p, inv, tmax, &mut test);
        best
    }

    /// 光源の光線 1 本（番号 id）を追い、終わった枝を out に足す（record_all でなければ検出器に届いた枝だけ）。
    /// 返り値はこの光線の収支
    pub fn trace(
        &self,
        start: &StartRay,
        id: u64,
        record_all: bool,
        out: &mut Vec<Branch>,
    ) -> Budget {
        let mut bud = Budget::default();
        self.trace_with(
            start,
            id,
            &mut |b: Branch, bud: &mut Budget| {
                match b.end {
                    End::Detected { .. } => bud.detected += b.power,
                    End::Absorbed => bud.absorbed += b.power,
                    End::Escaped => bud.escaped += b.power,
                    End::Discarded => bud.discarded += b.power,
                    End::Truncated => bud.truncated += b.power,
                }
                if record_all || matches!(b.end, End::Detected { .. }) {
                    out.push(b);
                }
            },
            &mut bud,
        );
        bud
    }

    fn trace_with(
        &self,
        start: &StartRay,
        id: u64,
        sink: &mut dyn FnMut(Branch, &mut Budget),
        bud: &mut Budget,
    ) {
        let p0 = start.power();
        bud.source += p0;
        let keep = self.opts.threshold * p0;
        let rng = match self.opts.split {
            Split::MonteCarlo { seed } => Some(CounterRng::new(seed)),
            Split::Deterministic => None,
        };
        let srng = match self.opts.split {
            Split::MonteCarlo { seed } => CounterRng::new(seed),
            Split::Deterministic => CounterRng::new(self.opts.seed),
        };
        let mut sx = ScatterCx {
            rng: srng,
            id,
            keep,
        };
        let mut stack = vec![Live {
            p: start.p,
            k: normalize(start.k),
            e: start.e,
            hits: Vec::new(),
            skip: Vec::new(),
            passes: 0,
            key: 0,
        }];
        while let Some(mut s) = stack.pop() {
            let mut pw = power_of(&s.e);
            if s.hits.len() >= self.opts.max_events {
                finish(s, End::Truncated, pw, sink, bud);
                continue;
            }
            // モンテカルロのルーレット
            if let Some(r) = &rng {
                if pw < keep && pw > 0.0 {
                    let u = r.uniform(id, s.hits.len() as u32, 1);
                    if u < 0.5 {
                        finish(s, End::Discarded, pw, sink, bud);
                        continue;
                    }
                    // 半分の確率で残す光線は 2 倍（期待値を保つ。増えた分は収支の外）
                    s.e = s.e.map(|z| z * std::f64::consts::SQRT_2);
                    pw = power_of(&s.e);
                }
            }
            let Some((fi, h)) = self.nearest(s.p, s.k, &s.skip) else {
                finish(s, End::Escaped, pw, sink, bud);
                continue;
            };
            let face = &self.scene.faces[fi];
            let fd = &self.faces[fi];
            let q = geom::along(s.p, s.k, h.t);
            let ng = geom::normal_global(face, h.n);
            let cosk = dot(s.k, ng);
            // 入ってくる側: +z 側から（k が法線と逆向き）なら 1
            let side = usize::from(cosk < 0.0);
            // 媒質の吸収
            if fd.la[side] != 0.0 {
                let a = (fd.la[side] * h.t).exp();
                s.e = s.e.map(|z| z * a);
                let p2 = power_of(&s.e);
                bud.absorbed += pw - p2;
                pw = p2;
            }
            s.p = q;
            // 素通りする面（働きの無い面・開いた絞り・検出器の裏）は、同じ点で通った面を全部除く（同じ所に重なった
            // 面の間を行き来しない）。ほかの出来事の後はその面だけ
            let passing = match fd.role {
                Role::Pass => true,
                Role::Stop => self.stop_open(face, &h),
                Role::Detector(_, front) => side != front,
                _ => false,
            };
            if passing && h.t <= geom::SELF_EPS {
                s.skip.push(fi);
            } else {
                s.skip.clear();
                s.skip.push(fi);
            }
            if passing {
                s.passes += 1;
                if s.passes > 4 * self.opts.max_events {
                    finish(s, End::Truncated, pw, sink, bud);
                    continue;
                }
            }
            let ev = |kind: EventKind, power: f64| Hit {
                face: fi,
                kind,
                p: q,
                power,
            };
            match fd.role {
                Role::Pass => {
                    stack.push(s);
                }
                Role::Stop => {
                    if passing {
                        stack.push(s);
                    } else {
                        self.wall(s, fi, ng, pw, &mut sx, &mut stack, sink, bud);
                    }
                }
                Role::Wall => {
                    self.wall(s, fi, ng, pw, &mut sx, &mut stack, sink, bud);
                }
                Role::Mirror => {
                    let r = fd.r_if[side].as_ref().expect("mirror coefficients");
                    let kr = reflect(s.k, ng);
                    let er = coef(r, s.e, s.k, kr, ng, cosk.abs(), true);
                    let pr = power_of(&er);
                    bud.absorbed += pw - pr;
                    s.e = er;
                    s.k = kr;
                    self.go_scatter(
                        s,
                        ev(EventKind::Reflect, pr),
                        true,
                        ng,
                        &mut sx,
                        &mut stack,
                        sink,
                        bud,
                    );
                }
                Role::Detector(_, front) if side != front => {
                    // 裏から来た光は素通り（検出器は物の陰を作らない）
                    stack.push(s);
                }
                Role::Detector(di, _) => {
                    let (et, er) = match (&fd.t_if[side], &fd.r_if[side]) {
                        (Some(t), Some(r)) => {
                            let kr = reflect(s.k, ng);
                            (
                                coef(t, s.e, s.k, s.k, ng, cosk.abs(), false),
                                Some((coef(r, s.e, s.k, kr, ng, cosk.abs(), true), kr)),
                            )
                        }
                        _ => (s.e, None),
                    };
                    let mut pt = power_of(&et);
                    let pr = er.as_ref().map_or(0.0, |x| power_of(&x.0));
                    bud.absorbed += pw - pt - pr;
                    let mut et = et;
                    let mut er = er;
                    // モンテカルロ: 境界と同じく反射か検出かをパワーの比で選び、選んだ方に両方のパワーを持たせる
                    if let (Some(rg), Some((e_r, k_r))) = (&rng, er.take()) {
                        let sum = pr + pt;
                        if sum.is_nan() || sum <= 0.0 {
                            s.e = et;
                            finish(s, End::Discarded, 0.0, sink, bud);
                            continue;
                        }
                        let u = rg.uniform(id, s.hits.len() as u32, 0);
                        if u * sum < pr {
                            s.e = e_r.map(|z| z * (sum / pr).sqrt());
                            s.k = k_r;
                            let p2 = power_of(&s.e);
                            bud.absorbed += sum - p2;
                            s.hits.push(ev(EventKind::Reflect, p2));
                            stack.push(s);
                            continue;
                        }
                        et = et.map(|z| z * (sum / pt).sqrt());
                        let p2 = power_of(&et);
                        bud.absorbed += sum - p2;
                        pt = p2;
                    }
                    if let Some((er, kr)) = er {
                        let b = Live {
                            p: q,
                            k: kr,
                            e: er,
                            hits: s.hits.clone(),
                            skip: vec![fi],
                            passes: s.passes,
                            key: branch_key(s.key, 1),
                        };
                        self.go_on(
                            b,
                            ev(EventKind::Reflect, pr),
                            pr,
                            keep,
                            &mut stack,
                            sink,
                            bud,
                        );
                    }
                    s.hits.push(ev(EventKind::Detect, pt));
                    s.e = et;
                    finish(
                        s,
                        End::Detected {
                            detector: di,
                            x: h.p[0],
                            y: h.p[1],
                        },
                        pt,
                        sink,
                        bud,
                    );
                }
                Role::Boundary => {
                    let o = 1 - side;
                    let (n1, n2) = (fd.n[side], fd.n[o]);
                    let kr = reflect(s.k, ng);
                    let r = fd.r_if[side].as_ref().expect("boundary coefficients");
                    match refract(s.k, ng, n1, n2) {
                        None => {
                            // 全反射
                            let er = coef(r, s.e, s.k, kr, ng, cosk.abs(), true);
                            let er = if er.iter().all(|z| z.re.is_finite() && z.im.is_finite()) {
                                er
                            } else {
                                s.e
                            };
                            let pr = power_of(&er);
                            bud.absorbed += pw - pr;
                            s.e = er;
                            s.k = kr;
                            self.go_scatter(
                                s,
                                ev(EventKind::Tir, pr),
                                true,
                                ng,
                                &mut sx,
                                &mut stack,
                                sink,
                                bud,
                            );
                        }
                        Some(kt) => {
                            let t = fd.t_if[side].as_ref().expect("boundary coefficients");
                            let er = coef(r, s.e, s.k, kr, ng, cosk.abs(), true);
                            let et = coef(t, s.e, s.k, kt, ng, cosk.abs(), false);
                            let (pr, pt) = (power_of(&er), power_of(&et));
                            bud.absorbed += pw - pr - pt;
                            match &rng {
                                None => {
                                    let b = Live {
                                        p: q,
                                        k: kr,
                                        e: er,
                                        hits: s.hits.clone(),
                                        skip: vec![fi],
                                        passes: s.passes,
                                        key: branch_key(s.key, 1),
                                    };
                                    s.k = kt;
                                    s.e = et;
                                    // 透過を先に積むと反射の枝から先に追う（どちらでも結果は同じ）
                                    self.go_scatter(
                                        s,
                                        ev(EventKind::Transmit, pt),
                                        false,
                                        ng,
                                        &mut sx,
                                        &mut stack,
                                        sink,
                                        bud,
                                    );
                                    self.go_scatter(
                                        b,
                                        ev(EventKind::Reflect, pr),
                                        true,
                                        ng,
                                        &mut sx,
                                        &mut stack,
                                        sink,
                                        bud,
                                    );
                                }
                                Some(rg) => {
                                    let sum = pr + pt;
                                    if sum.is_nan() || sum <= 0.0 {
                                        s.e = et;
                                        finish(s, End::Discarded, 0.0, sink, bud);
                                        continue;
                                    }
                                    let u = rg.uniform(id, s.hits.len() as u32, 0);
                                    let (kind, e, k, pk) = if u * sum < pr {
                                        (EventKind::Reflect, er, kr, pr)
                                    } else {
                                        (EventKind::Transmit, et, kt, pt)
                                    };
                                    // 選んだ枝に両方のパワーを持たせる
                                    let a = (sum / pk).sqrt();
                                    s.e = e.map(|z| z * a);
                                    s.k = k;
                                    let p2 = power_of(&s.e);
                                    // 丸めの差は吸収に入れる（収支を閉じる）
                                    bud.absorbed += sum - p2;
                                    if fd.scatter.is_some() {
                                        self.go_scatter(
                                            s,
                                            ev(kind, p2),
                                            kind == EventKind::Reflect,
                                            ng,
                                            &mut sx,
                                            &mut stack,
                                            sink,
                                            bud,
                                        );
                                    } else {
                                        s.hits.push(ev(kind, p2));
                                        stack.push(s);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 絞りの面の交点 h が開口の中か
    fn stop_open(&self, face: &crate::scene::Face, h: &LocalHit) -> bool {
        match &face.shape {
            Shape::Optical { surface: d, .. } => match d.ap {
                Aperture::None | Aperture::Floating { .. } => {
                    h.p[0].hypot(h.p[1]) <= d.semi * (1.0 + 1e-12)
                }
                ap => !lensbench_core::trace::aperture_out(ap, h.p),
            },
            _ => false,
        }
    }

    /// 出来事 h を足した枝を積む。決まった分岐ではパワーが閾値より小さい枝を捨てる
    #[allow(clippy::too_many_arguments)]
    fn go_on(
        &self,
        mut s: Live,
        h: Hit,
        power: f64,
        keep: f64,
        stack: &mut Vec<Live>,
        sink: &mut dyn FnMut(Branch, &mut Budget),
        bud: &mut Budget,
    ) {
        s.hits.push(h);
        if self.opts.split == Split::Deterministic && power < keep {
            finish(s, End::Discarded, power, sink, bud);
        } else {
            stack.push(s);
        }
    }
}

/// 追っている枝
struct Live {
    p: V3,
    k: V3,
    e: CV3,
    hits: Vec<Hit>,
    /// 直前の面（自分の面に戻る交点を除く）
    skip: Vec<usize>,
    /// 素通りした面の数（重なった面の間の行き来を止める）
    passes: usize,
    /// 枝の印（決まった分岐で分かれた枝ごとに違う散乱の乱数を引く）
    key: u64,
}

fn finish(
    s: Live,
    end: End,
    power: f64,
    sink: &mut dyn FnMut(Branch, &mut Budget),
    bud: &mut Budget,
) {
    sink(
        Branch {
            hits: s.hits,
            end,
            p: s.p,
            k: s.k,
            e: s.e,
            power,
        },
        bud,
    )
}

/// 鏡面反射の方向
fn reflect(k: V3, n: V3) -> V3 {
    let c = 2.0 * dot(k, n);
    [k[0] - c * n[0], k[1] - c * n[1], k[2] - c * n[2]]
}

/// 屈折の方向（n1 の側から n2 の側へ）。全反射なら None
fn refract(k: V3, n: V3, n1: f64, n2: f64) -> Option<V3> {
    // 入射側を向く法線
    let c0 = dot(k, n);
    let nn = if c0 > 0.0 { scale(n, -1.0) } else { n };
    let ci = c0.abs();
    let eta = n1 / n2;
    let s2 = eta * eta * (1.0 - ci * ci);
    if s2 > 1.0 {
        return None;
    }
    let ct = (1.0 - s2).sqrt();
    let a = eta * ci - ct;
    Some(normalize([
        eta * k[0] + a * nn[0],
        eta * k[1] + a * nn[1],
        eta * k[2] + a * nn[2],
    ]))
}

/// 境界の係数 f（入射角の cos が cosi）を電場 e にかける（偏光追跡の面の処理と同じ s・p の基底: s = k × n、
/// 鏡・反射は p の向きを反転）
fn coef(f: &Iface, e: CV3, kin: V3, kout: V3, nv: V3, cosi: f64, mirror: bool) -> CV3 {
    let mut sv = cross(kin, nv);
    if norm(sv).is_nan() || norm(sv) <= lensbench_core::tol::PARALLEL_SIN {
        sv = cross(kin, [1.0, 0.0, 0.0]);
    }
    let sv = normalize(sv);
    let pin = cross(sv, kin);
    let mut pout = cross(sv, kout);
    if mirror {
        pout = scale(pout, -1.0);
    }
    let tc = match f.eval_cos(cosi.clamp(0.0, 1.0)) {
        Ok(t) => t,
        Err(_) => return [C64::new(f64::NAN, 0.0); 3],
    };
    let cd = |x: &CV3, v: V3| x[0] * v[0] + x[1] * v[1] + x[2] * v[2];
    let (es, ep) = (cd(&e, sv), cd(&e, pin));
    let (a, b) = (tc[0] * es, tc[1] * ep);
    [0, 1, 2].map(|i| a * sv[i] + b * pout[i])
}

/// 組ごとの集計
struct Acc {
    power: Vec<Vec<f64>>,
    hits: Vec<Vec<u64>>,
    paths: HashMap<Vec<(usize, EventKind)>, PathAcc>,
    budget: Budget,
}

/// 経路ごとの集計: 検出器・パワー・本数・画素ごとのパワー
#[derive(Default)]
struct PathAcc {
    detector: usize,
    power: f64,
    rays: u64,
    pixels: HashMap<usize, f64>,
}

/// 1 つの組の光線の数（スレッド数に依らない足し方の単位）
const BLOCK: usize = 64;
/// 一度に並列に数える組の数
const GROUP: usize = 256;

impl Tracer<'_> {
    fn acc(&self) -> Acc {
        let n = self.opts.detector.pixels[0] * self.opts.detector.pixels[1];
        Acc {
            power: vec![vec![0.0; n]; self.detectors.len()],
            hits: vec![vec![0; n]; self.detectors.len()],
            paths: HashMap::new(),
            budget: Budget::default(),
        }
    }

    /// 検出器 di の局所座標 (x, y) の画素
    fn pixel(&self, di: usize, x: f64, y: f64) -> Option<usize> {
        let Bound::Rect { hx, hy } = self.faces[self.detectors[di]].bound else {
            return None;
        };
        let [nx, ny] = self.opts.detector.pixels;
        let u = (x + hx) / (2.0 * hx) * nx as f64;
        let v = (y + hy) / (2.0 * hy) * ny as f64;
        // 上の端（|x| = hx ちょうど）は最後の画素に入れる
        let i = (u.max(0.0) as usize).min(nx - 1);
        let j = (v.max(0.0) as usize).min(ny - 1);
        (u >= 0.0 && v >= 0.0).then_some(j * nx + i)
    }

    fn block(&self, starts: &[StartRay], first: usize) -> Acc {
        let mut a = self.acc();
        let mut out = Vec::new();
        for (i, st) in starts.iter().enumerate() {
            out.clear();
            let b = self.trace(st, (first + i) as u64, false, &mut out);
            a.budget.add(&b);
            for br in &out {
                let End::Detected { detector, x, y } = br.end else {
                    continue;
                };
                let key: Vec<(usize, EventKind)> =
                    br.hits.iter().map(|h| (h.face, h.kind)).collect();
                let q = self.pixel(detector, x, y);
                let e = a.paths.entry(key).or_insert_with(|| PathAcc {
                    detector,
                    ..Default::default()
                });
                e.power += br.power;
                e.rays += 1;
                if let Some(q) = q {
                    *e.pixels.entry(q).or_insert(0.0) += br.power;
                }
                if self
                    .opts
                    .filter
                    .as_ref()
                    .is_some_and(|f| !f.accepts(&br.hits))
                {
                    continue;
                }
                if let Some(q) = q {
                    a.power[detector][q] += br.power;
                    a.hits[detector][q] += 1;
                }
            }
        }
        a
    }

    /// 光源の光線を全部追って、検出器・経路の表・収支を数える（光線の番号は並びの順）
    pub fn run(&self, starts: &[StartRay]) -> Result<NscResult> {
        let mut tot = self.acc();
        let blocks: Vec<usize> = (0..starts.len()).step_by(BLOCK).collect();
        for g in blocks.chunks(GROUP) {
            let parts = lensbench_core::par::map(g, |&b0| {
                let b1 = (b0 + BLOCK).min(starts.len());
                self.block(&starts[b0..b1], b0)
            });
            for a in parts {
                for (d, p) in a.power.iter().enumerate() {
                    for (x, y) in tot.power[d].iter_mut().zip(p) {
                        *x += y;
                    }
                    for (x, y) in tot.hits[d].iter_mut().zip(&a.hits[d]) {
                        *x += y;
                    }
                }
                tot.budget.add(&a.budget);
                for (k, v) in a.paths {
                    let e = tot.paths.entry(k).or_insert_with(|| PathAcc {
                        detector: v.detector,
                        ..Default::default()
                    });
                    e.power += v.power;
                    e.rays += v.rays;
                    for (q, p) in v.pixels {
                        *e.pixels.entry(q).or_insert(0.0) += p;
                    }
                }
            }
            lensbench_core::par::check_cancel()?;
        }
        let b = &tot.budget;
        if ![
            b.source,
            b.detected,
            b.absorbed,
            b.escaped,
            b.discarded,
            b.truncated,
        ]
        .iter()
        .all(|x| x.is_finite())
        {
            invalid!("nsc: the coating coefficients can't be evaluated for some rays (the power isn't finite)");
        }
        let mut paths: Vec<PathRow> = tot
            .paths
            .into_iter()
            .map(|(events, a)| {
                let mut pixels: Vec<(usize, f64)> =
                    a.pixels.into_iter().filter(|x| x.1 != 0.0).collect();
                pixels.sort_by_key(|x| x.0);
                PathRow {
                    detector: a.detector,
                    events,
                    power: a.power,
                    rays: a.rays,
                    pixels,
                }
            })
            .collect();
        paths.sort_by(|a, b| {
            b.power
                .total_cmp(&a.power)
                .then_with(|| a.detector.cmp(&b.detector))
                .then_with(|| a.events.cmp(&b.events))
        });
        let detectors = self
            .detectors
            .iter()
            .enumerate()
            .map(|(d, &fi)| {
                let Bound::Rect { hx, hy } = self.faces[fi].bound else {
                    unreachable!("a detector has a rectangle")
                };
                DetectorImage {
                    face: fi,
                    label: self.labels[fi].clone(),
                    half: [hx, hy],
                    pixels: self.opts.detector.pixels,
                    power: std::mem::take(&mut tot.power[d]),
                    hits: std::mem::take(&mut tot.hits[d]),
                }
            })
            .collect();
        Ok(NscResult {
            detectors,
            paths,
            budget: tot.budget,
            launched: starts.len(),
            wavelength: self.wl,
        })
    }
}
