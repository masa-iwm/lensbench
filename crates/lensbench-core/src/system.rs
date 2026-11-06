//! 系: 面と系のデータ（波長、視野、絞り、光線エイミング）、近軸量、光線の生成。
//!
//! 流儀は OpticStudio に合わせる（レンズ単位 mm、波長 µm、角度 deg）。
//! - 近軸量: 主波長の y-nu 追跡（展開した軸に沿う。傾き・偏心・座標ブレークは入らず、鏡で向きが反転）
//! - 絞り: 入射瞳径、像側 F 値、物体側 NA、絞り径、近軸作動 F 値、物体側コーン角、物体側テレセントリック
//! - 視野: 角度（方向正接）、物体高、近軸像高、実像高。正規化は半径か矩形。定義済み視野のケラレ係数
//! - 瞳座標: エイミングなしは近軸入射瞳面、エイミングありは絞り面の (px, py)×絞り半径
//! - 無限遠物体の光線は面 1 の頂点を通り視野方向に垂直な面から出る（光路長はそこから数える）

use crate::coating::{self, Coatings, C64};
use crate::error::{Error, Result};
use crate::glass::Glass;
use crate::math::*;
use crate::surface::{After, SurfType, Surface, Tilt};
use crate::trace::{self, PolKind, PolRay, PolSurf, RayOut, RecEntry, CV3};
use crate::{invalid, notimpl};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const ENPP_MAX: f64 = 1e10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Angle,
    ObjH,
    ParH,
    RealH,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldNorm {
    Radial,
    Rect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApertureKind {
    Epd,
    FNum,
    Na,
    Float,
    Pwfn,
    Cone,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aiming {
    Off,
    Paraxial,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub vdx: f64,
    pub vdy: f64,
    pub vcx: f64,
    pub vcy: f64,
    pub van: f64,
}

impl Field {
    pub fn xy(x: f64, y: f64) -> Field {
        Field {
            x,
            y,
            w: 1.0,
            vdx: 0.0,
            vdy: 0.0,
            vcx: 0.0,
            vcy: 0.0,
            van: 0.0,
        }
    }
}

/// 面の座標系（原点、列が軸の回転行列）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub o: V3,
    pub r: M3,
}

/// 局所座標の間の変換 q = r · p + t。`shift` は r が単位行列で t = (0, 0, shift) のとき（共軸の面の間）
#[derive(Debug, Clone, Copy)]
pub struct Rel {
    pub r: M3,
    pub t: V3,
    pub shift: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Frames {
    /// 面ごと（無視する面と、無限遠の物体面は None）
    pub surf: Vec<Option<Frame>>,
    /// 面ごとの、直前の有効な面の局所座標からこの面の局所座標への変換（追跡の内側で大域座標を経ずに進むため）
    pub rel: Vec<Option<Rel>>,
    /// 面 1 の変換の前の座標系（入射瞳を測る物体空間の軸）
    pub obj: Frame,
}

#[derive(Debug, Clone)]
pub struct Paraxial {
    pub enpp: f64,
    pub epd: f64,
    pub efl: f64,
    pub expp: f64,
    pub expd: f64,
    pub stop: usize,
    pub stop_radius: f64,
    pub u_obj: Option<f64>,
    pub marginal: (Vec<f64>, Vec<f64>),
    pub chief: (Vec<f64>, Vec<f64>),
    pub chief_image_per_unit: f64,
    pub wfno: f64,
    pub isfn: f64,
    pub pmag: f64,
}

/// 物体: 無限遠なら物体空間の単位方向、有限なら物体面上の点 (x, y)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjSpec {
    Dir(V3),
    Point([f64; 2]),
}

/// 視野の指定: 定義済み視野の番号か、正規化視野 (hx, hy)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldSel {
    Index(usize),
    Norm(f64, f64),
}
impl From<usize> for FieldSel {
    fn from(i: usize) -> Self {
        FieldSel::Index(i)
    }
}
impl From<(f64, f64)> for FieldSel {
    fn from(h: (f64, f64)) -> Self {
        FieldSel::Norm(h.0, h.1)
    }
}

/// 光線の束の追跡結果
#[derive(Debug, Clone)]
pub struct Trace {
    pub rays: Vec<RayOut>,
    /// (-1)^(通過した鏡の数)
    pub parity: f64,
    /// 開始位置・方向（大域）
    pub p0: Vec<V3>,
    pub k0: Vec<V3>,
    /// OpticStudio の生の光路長（無限遠物体では面 1 の物体側座標系の z = 0 面から）
    pub opl_os: Vec<f64>,
    /// 面ごとの記録（record のとき）: 面番号 → 光線ごと
    pub rec: Vec<(usize, Vec<RecEntry>, f64)>,
    pub last: usize,
    pub hx_hy: (f64, f64),
}

/// 偏光追跡の結果
#[derive(Debug, Clone)]
pub struct PolTrace {
    pub trace: Trace,
    /// 光線ごと・入力の偏光ごとの電場（最後の面の局所座標、伝搬の位相込み）。追跡が止まった光線は NaN
    pub e: Vec<Vec<CV3>>,
}

impl Trace {
    pub fn ok(&self, i: usize) -> bool {
        self.rays[i].err == 0 && self.rays[i].vig == 0
    }
}

#[derive(Debug, Default)]
struct Cache {
    frames: OnceLock<Frames>,
    par: OnceLock<Paraxial>,
    expp: OnceLock<f64>,
    expd: OnceLock<f64>,
    rs: OnceLock<(f64, f64)>,
    realh: Mutex<HashMap<(u64, u64), ObjSpec>>,
    conj: Mutex<HashMap<usize, f64>>,
}

/// これ以上の本数で並列に追う
const PAR_MIN_RAYS: usize = 512;
/// 並列のときの 1 タスクあたりの最少本数
const PAR_CHUNK: usize = 256;

#[derive(Debug)]
pub struct System {
    pub surfaces: Vec<Surface>,
    pub wavelengths: Vec<f64>,
    pub weights: Vec<f64>,
    pub primary: usize,
    pub fields: Vec<Field>,
    pub field_type: FieldType,
    pub field_norm: FieldNorm,
    pub aperture: (ApertureKind, f64),
    pub telecentric: bool,
    pub aiming: Aiming,
    pub glasses: HashMap<String, Glass>,
    pub temperature: f64,
    pub pressure: f64,
    pub glrs: usize,
    pub ropd: i64,
    pub coating_file: Option<PathBuf>,
    pub name: String,
    pub adjust_index: bool,
    cache: Cache,
}

impl Clone for System {
    fn clone(&self) -> Self {
        System {
            surfaces: self.surfaces.clone(),
            wavelengths: self.wavelengths.clone(),
            weights: self.weights.clone(),
            primary: self.primary,
            fields: self.fields.clone(),
            field_type: self.field_type,
            field_norm: self.field_norm,
            aperture: self.aperture,
            telecentric: self.telecentric,
            aiming: self.aiming,
            glasses: self.glasses.clone(),
            temperature: self.temperature,
            pressure: self.pressure,
            glrs: self.glrs,
            ropd: self.ropd,
            coating_file: self.coating_file.clone(),
            name: self.name.clone(),
            adjust_index: self.adjust_index,
            cache: Cache::default(),
        }
    }
}

impl System {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        surfaces: Vec<Surface>,
        wavelengths: Vec<f64>,
        weights: Vec<f64>,
        primary: usize,
        fields: Vec<Field>,
        field_type: FieldType,
        field_norm: FieldNorm,
        aperture: (ApertureKind, f64),
        telecentric: bool,
        aiming: Aiming,
        glasses: HashMap<String, Glass>,
    ) -> Result<System> {
        let s = System {
            surfaces,
            wavelengths,
            weights,
            primary,
            fields: if fields.is_empty() {
                vec![Field::xy(0.0, 0.0)]
            } else {
                fields
            },
            field_type,
            field_norm,
            aperture,
            telecentric,
            aiming,
            glasses,
            temperature: 20.0,
            pressure: 1.0,
            glrs: 1,
            ropd: 2,
            coating_file: None,
            name: String::new(),
            adjust_index: true,
            cache: Cache::default(),
        };
        s.check()?;
        Ok(s)
    }

    pub fn check(&self) -> Result<()> {
        let sf = &self.surfaces;
        if sf.len() < 3 {
            invalid!("面が少なすぎる（{}）", sf.len());
        }
        if self.telecentric && !sf[0].t.is_finite() {
            notimpl!("無限遠物体で物体側テレセントリック");
        }
        if self.telecentric && self.aiming != Aiming::Off {
            notimpl!("物体側テレセントリックで光線エイミング");
        }
        if !sf[0].t.is_finite() && self.field_type == FieldType::ObjH {
            invalid!("無限遠物体で物体高の視野");
        }
        if self.telecentric && !matches!(self.aperture.0, ApertureKind::Na | ApertureKind::Cone) {
            invalid!("物体側テレセントリックには物体側 NA かコーン角の絞りが要る");
        }
        let st = self.stop();
        if st == 0 || st >= sf.len() - 1 {
            invalid!("絞り面 {st}");
        }
        Ok(())
    }

    /// 面や系のデータを書き換えたあとに呼ぶ（座標系・近軸量などのキャッシュを捨てる）
    pub fn changed(&mut self) -> Result<()> {
        self.cache = Cache::default();
        self.check()
    }

    pub fn stop(&self) -> usize {
        self.surfaces.iter().position(|d| d.stop).unwrap_or(0)
    }
    pub fn image(&self) -> usize {
        self.surfaces.len() - 1
    }
    pub fn infinite(&self) -> bool {
        !self.surfaces[0].t.is_finite()
    }

    pub fn frames(&self) -> &Frames {
        self.cache
            .frames
            .get_or_init(|| frames(&self.surfaces, self.glrs))
    }

    /// 面 j の直前の媒質の屈折率
    pub fn index_before(&self, j: usize, w: usize) -> f64 {
        let mut n = self.surfaces[0].n[w];
        for d in &self.surfaces[1..j.max(1)] {
            if !d.ignore && d.typ != SurfType::CoordBrk {
                n = d.n[w];
            }
        }
        n
    }

    pub fn image_index(&self, w: usize) -> f64 {
        self.index_before(self.image(), w)
    }

    // ---------------------------------------------------------------------------------- 近軸
    /// 面 1 の頂点面で高さ y1、物体空間の傾き u0 の近軸光線 → 各面の頂点面での高さ y と面の後の傾き u（展開した向き）
    pub fn ptrace(&self, y1: f64, u0: f64, w: usize, upto: Option<usize>) -> (Vec<f64>, Vec<f64>) {
        let sf = &self.surfaces;
        let upto = upto.unwrap_or(sf.len() - 1);
        let mut y = vec![f64::NAN; sf.len()];
        let mut u = vec![f64::NAN; sf.len()];
        let mut n = sf[0].n[w];
        let (mut uu, mut yy, mut sgn) = (u0, y1, 1.0);
        y[0] = yy - uu * if sf[0].t.is_finite() { sf[0].t } else { 0.0 };
        u[0] = uu;
        for j in 1..=upto {
            let d = &sf[j];
            if d.ignore {
                continue;
            }
            y[j] = yy;
            match d.typ {
                SurfType::Standard | SurfType::EvenAsph => {
                    let a1 = d.asph.first().copied().unwrap_or(0.0);
                    let c = sgn * (d.c + 2.0 * a1);
                    if d.mirror {
                        uu += 2.0 * c * yy;
                        sgn = -sgn;
                    } else {
                        let n2 = d.n[w];
                        uu = (n * uu - yy * c * (n2 - n)) / n2;
                        n = n2;
                    }
                }
                SurfType::Paraxial => uu -= yy / d.focal,
                SurfType::Jones => n = d.n[w],
                SurfType::CoordBrk => {}
            }
            u[j] = uu;
            if j < upto {
                yy += sgn * d.t * uu;
            }
        }
        (y, u)
    }

    fn stop_lin(&self) -> (f64, f64) {
        let s = self.stop();
        let a = self.ptrace(1.0, 0.0, self.primary, Some(s)).0[s];
        let b = self.ptrace(0.0, 1.0, self.primary, Some(s)).0[s];
        (a, b)
    }

    /// 主波長の近軸量
    pub fn paraxial(&self) -> &Paraxial {
        self.cache.par.get_or_init(|| self.compute_paraxial())
    }

    fn compute_paraxial(&self) -> Paraxial {
        let sf = &self.surfaces;
        let s = self.stop();
        let t0 = sf[0].t;
        let pw = self.primary;
        let (a, b) = self.stop_lin();
        let enpp = if self.telecentric || a.abs() < 1e-300 {
            ENPP_MAX
        } else {
            let e = b / a;
            if e.abs() >= ENPP_MAX {
                ENPP_MAX
            } else {
                e
            }
        };
        let img = self.image();
        let (_, uk) = self.ptrace(1.0, 0.0, pw, None);
        let efl = if uk[img - 1] != 0.0 {
            -1.0 / uk[img - 1]
        } else {
            f64::INFINITY
        };
        let (kind, val) = self.aperture;
        let mut u_obj: Option<f64> = None;
        let n_i = self.image_index(pw);
        let epd = match kind {
            ApertureKind::Epd => val,
            ApertureKind::FNum => efl.abs() / val,
            ApertureKind::Float => {
                let semi = sf[s].semi;
                if self.infinite() {
                    2.0 * semi / a.abs()
                } else {
                    let u = semi / (b + a * t0).abs();
                    u_obj = Some(u);
                    2.0 * u * (enpp + t0).abs()
                }
            }
            ApertureKind::Na | ApertureKind::Cone => {
                let n0 = sf[0].n[pw];
                let th = if kind == ApertureKind::Na {
                    (val / n0).asin()
                } else {
                    val.to_radians()
                };
                let u = th.tan();
                u_obj = Some(u);
                2.0 * u * (enpp + t0).abs()
            }
            ApertureKind::Pwfn => {
                if self.infinite() {
                    let (_, um) = self.ptrace(1.0, 0.0, pw, None);
                    2.0 * 1.0 / (2.0 * val * um[img - 1].abs() * n_i)
                } else {
                    let (_, um) = self.ptrace(t0, 1.0, pw, None);
                    let u = 1.0 / (2.0 * val * um[img - 1].abs() * n_i);
                    u_obj = Some(u);
                    2.0 * u * (enpp + t0).abs()
                }
            }
        };
        let (ym, um) = if self.infinite() {
            self.ptrace(epd / 2.0, 0.0, pw, None)
        } else {
            let u = *u_obj.get_or_insert(epd / 2.0 / (enpp + t0));
            self.ptrace(u * t0, u, pw, None)
        };
        let (yc, uc) = if self.infinite() {
            self.ptrace(-enpp, 1.0, pw, None)
        } else if enpp >= ENPP_MAX {
            self.ptrace(1.0, 0.0, pw, None)
        } else {
            let u0 = -1.0 / (enpp + t0);
            self.ptrace(1.0 + u0 * t0, u0, pw, None)
        };
        let (yci, uci) = (yc[img], uc[img - 1]);
        let (ymi, umi) = (ym[img], um[img - 1]);
        let mut expp = if uci != 0.0 {
            -yci / uci
        } else {
            f64::INFINITY
        };
        let nmir = sf[1..img].iter().filter(|d| d.mirror && !d.ignore).count();
        if nmir % 2 == 1 {
            expp = -expp;
        }
        let expd = if expp.is_finite() {
            2.0 * (ymi + umi * expp).abs()
        } else {
            f64::INFINITY
        };
        Paraxial {
            enpp,
            epd,
            efl,
            expp,
            expd,
            stop: s,
            stop_radius: ym[s].abs(),
            u_obj,
            chief_image_per_unit: yci,
            wfno: if umi != 0.0 {
                1.0 / (2.0 * n_i * umi.abs())
            } else {
                f64::INFINITY
            },
            isfn: efl.abs() / epd,
            pmag: if !self.infinite() && umi != 0.0 {
                sf[0].n[pw] * um[0] / (n_i * umi)
            } else {
                0.0
            },
            marginal: (ym, um),
            chief: (yc, uc),
        }
    }

    /// 座標ブレークや面の傾き偏心が面を軸から動かしていなければ true
    pub fn symmetric(&self) -> bool {
        for d in &self.surfaces {
            if d.ignore {
                continue;
            }
            if d.typ == SurfType::CoordBrk && d.cb.iter().any(|&v| v != 0.0) {
                return false;
            }
            if d.before.is_some() || matches!(d.after, After::Tilt(_)) {
                return false;
            }
        }
        true
    }

    /// 射出瞳の位置（像面からその局所 z に沿って。OPD の基準）。回転対称なら近軸、そうでなければ
    /// OpticStudio と同じく小さい y 視野の実主光線が軸上主光線と交わる位置（2 視野で外挿）
    pub fn exit_pupil(&self) -> f64 {
        *self.cache.expp.get_or_init(|| {
            let mut e = self.paraxial().expp;
            if !self.symmetric() {
                let r0 = self.trace_novig(0.0, 0.0, &[0.0], &[0.0]);
                let (p0, k0) = (r0.rays[0].xyz, r0.rays[0].lmn);
                let mut z = [0.0; 2];
                for (i, h) in [2e-4, 1e-4].iter().enumerate() {
                    let r1 = self.trace_novig(0.0, *h, &[0.0], &[0.0]);
                    let (p1, k1) = (r1.rays[0].xyz, r1.rays[0].lmn);
                    let w0 = sub(p0, p1);
                    let (a, b, c) = (dot(k0, k0), dot(k0, k1), dot(k1, k1));
                    let (d, f) = (dot(k0, w0), dot(k1, w0));
                    z[i] = add(p0, scale(k0, (b * f - c * d) / (a * c - b * b)))[2];
                }
                e = (4.0 * z[1] - z[0]) / 3.0;
            }
            e
        })
    }

    /// 瞳の算出用: ケラレ係数を掛けずに主波長で追う（瞳は系の性質で、視野のケラレ係数とは無関係）
    fn trace_novig(&self, hx: f64, hy: f64, px: &[f64], py: &[f64]) -> Trace {
        let spec = self.object_spec(hx, hy).expect("瞳の算出の物体指定");
        let mut t = self.trace_spec(
            spec,
            px,
            py,
            self.primary,
            Some([0.0; 5]),
            None,
            false,
            Some((hx, hy)),
        );
        t.hx_hy = (hx, hy);
        t
    }

    /// 射出瞳径（OpticStudio の EXPD）
    pub fn exit_pupil_diameter(&self) -> f64 {
        *self.cache.expd.get_or_init(|| {
            let e = self.exit_pupil();
            if (self.aiming == Aiming::Off && self.symmetric()) || !e.is_finite() {
                self.paraxial().expd
            } else {
                let dl = 1e-4;
                let r = self.trace_novig(0.0, 0.0, &[0.0, 0.0, 0.0], &[0.0, dl, -dl]);
                let p: Vec<V3> = r.rays.iter().map(|x| x.xyz).collect();
                let k: Vec<V3> = r.rays.iter().map(|x| x.lmn).collect();
                let c = k[0];
                let cc = add(p[0], scale(c, (e - p[0][2]) / c[2]));
                let q: Vec<V3> = (0..3)
                    .map(|i| add(p[i], scale(k[i], dot(sub(cc, p[i]), c) / dot(k[i], c))))
                    .collect();
                norm(sub(q[1], q[2])) / dl
            }
        })
    }

    /// 理想レンズ j の近軸の物体距離（局所 z、進行方向に。無限大は平行光）
    pub fn lens_conjugate(&self, j: usize) -> f64 {
        if let Some(v) = self.cache.conj.lock().unwrap().get(&j) {
            return *v;
        }
        let (ym, um) = &self.paraxial().marginal;
        let prev = (0..j)
            .rev()
            .find(|&i| !self.surfaces[i].ignore)
            .unwrap_or(0);
        let u_in = um[prev];
        let v = if u_in.abs() > 1e-15 * ym[j].abs().max(1.0) {
            -ym[j] / u_in
        } else {
            f64::INFINITY
        };
        self.cache.conj.lock().unwrap().insert(j, v);
        v
    }

    // ---------------------------------------------------------------------------------- 視野
    pub fn field_norm_values(&self) -> (f64, f64) {
        let or1 = |v: f64| if v == 0.0 { 1.0 } else { v };
        match self.field_norm {
            FieldNorm::Rect => (
                or1(self.fields.iter().map(|f| f.x.abs()).fold(0.0, f64::max)),
                or1(self.fields.iter().map(|f| f.y.abs()).fold(0.0, f64::max)),
            ),
            FieldNorm::Radial => {
                let r = or1(self
                    .fields
                    .iter()
                    .map(|f| f.x.hypot(f.y))
                    .fold(0.0, f64::max));
                (r, r)
            }
        }
    }

    /// 視野 i の正規化視野
    pub fn hxhy(&self, i: usize) -> (f64, f64) {
        let (nx, ny) = self.field_norm_values();
        (self.fields[i].x / nx, self.fields[i].y / ny)
    }

    fn vig_factors(&self, hx: f64, hy: f64) -> Result<[f64; 5]> {
        let (fx, fy) = self.field_norm_values();
        for f in &self.fields {
            if (f.x / fx - hx).abs() < 1e-12 && (f.y / fy - hy).abs() < 1e-12 {
                return Ok([f.vdx, f.vdy, f.vcx, f.vcy, f.van]);
            }
        }
        if self.fields.iter().any(|f| {
            [f.vdx, f.vdy, f.vcx, f.vcy, f.van]
                .iter()
                .any(|&v| v != 0.0)
        }) {
            notimpl!("定義済み視野以外の点でのケラレ係数");
        }
        Ok([0.0; 5])
    }

    /// 物体の指定（無限遠: 方向、有限: 物体面の点）
    pub fn object_spec(&self, hx: f64, hy: f64) -> Result<ObjSpec> {
        let (nx, ny) = self.field_norm_values();
        let (x, y) = (hx * nx, hy * ny);
        let par = self.paraxial();
        let (tx, ty) = match self.field_type {
            FieldType::RealH => return self.real_height_spec(x, y),
            FieldType::Angle => (x.to_radians().tan(), y.to_radians().tan()),
            FieldType::ParH => {
                let h1 = par.chief_image_per_unit;
                if self.infinite() {
                    (x / h1, y / h1)
                } else {
                    return Ok(ObjSpec::Point([x / h1, y / h1]));
                }
            }
            FieldType::ObjH => return Ok(ObjSpec::Point([x, y])),
        };
        if self.infinite() {
            return Ok(ObjSpec::Dir(normalize([tx, ty, 1.0])));
        }
        let l = self.surfaces[0].t + par.enpp;
        Ok(ObjSpec::Point([-l * tx, -l * ty]))
    }

    fn real_height_spec(&self, x: f64, y: f64) -> Result<ObjSpec> {
        let key = (x.to_bits(), y.to_bits());
        if let Some(v) = self.cache.realh.lock().unwrap().get(&key) {
            return Ok(*v);
        }
        let h1 = self.paraxial().chief_image_per_unit;
        let mut v = [x / h1, y / h1];
        let spec = |v: [f64; 2]| {
            if self.infinite() {
                ObjSpec::Dir(normalize([v[0], v[1], 1.0]))
            } else {
                ObjSpec::Point(v)
            }
        };
        let img = |v: [f64; 2]| -> [f64; 2] {
            let r = self.trace_spec(
                spec(v),
                &[0.0],
                &[0.0],
                self.primary,
                Some([0.0; 5]),
                None,
                false,
                None,
            );
            [r.rays[0].xyz[0], r.rays[0].xyz[1]]
        };
        for _ in 0..40 {
            let q = img(v);
            let e = [q[0] - x, q[1] - y];
            if e[0].abs().max(e[1].abs()) < 1e-13 {
                break;
            }
            let h = 1e-7 * v[0].abs().max(v[1].abs()).max(1.0);
            let qx = img([v[0] + h, v[1]]);
            let qy = img([v[0], v[1] + h]);
            let (a, b, c, d) = (
                (qx[0] - q[0]) / h,
                (qy[0] - q[0]) / h,
                (qx[1] - q[1]) / h,
                (qy[1] - q[1]) / h,
            );
            let det = a * d - b * c;
            v = [
                v[0] - (d * e[0] - b * e[1]) / det,
                v[1] - (-c * e[0] + a * e[1]) / det,
            ];
        }
        let q = img(v);
        let e = (q[0] - x).abs().max((q[1] - y).abs());
        if !(e < 1e-9 * x.abs().max(y.abs()).max(1.0)) {
            invalid!("実像高の視野 ({x}, {y}): 届く実主光線がない（{e} ずれる）");
        }
        let s = spec(v);
        self.cache.realh.lock().unwrap().insert(key, s);
        Ok(s)
    }

    // ---------------------------------------------------------------------------------- 光線
    /// 入射瞳面の点 (ex, ey) [mm] を通る光線の開始位置・方向（大域）
    pub fn start_ray(&self, spec: ObjSpec, ex: f64, ey: f64) -> (V3, V3) {
        let fr = self.frames();
        let (o1, r0) = (fr.obj.o, fr.obj.r);
        let par = self.paraxial();
        let e = add(o1, mv(&r0, [ex, ey, par.enpp.min(ENPP_MAX)]));
        match spec {
            ObjSpec::Dir(d) => {
                let d = mv(&r0, d);
                let p = sub(e, scale(d, dot(sub(e, o1), d)));
                (p, d)
            }
            ObjSpec::Point(pt) => {
                let f0 = fr.surf[0].expect("有限物体の座標系");
                let p = add(f0.o, mv(&f0.r, [pt[0], pt[1], 0.0]));
                let mut k = sub(e, p);
                if dot(k, col(&r0, 2)) < 0.0 {
                    k = scale(k, -1.0);
                }
                (p, normalize(k))
            }
        }
    }

    /// 瞳座標 1 が狙う絞りの半径 (rx, ry)
    pub fn stop_radius(&self) -> (f64, f64) {
        *self.cache.rs.get_or_init(|| {
            if self.aperture.0 == ApertureKind::Float {
                let s = self.surfaces[self.stop()].semi;
                (s, s)
            } else if self.aiming == Aiming::Real {
                let spec = if self.infinite() {
                    ObjSpec::Dir([0.0, 0.0, 1.0])
                } else {
                    ObjSpec::Point([0.0, 0.0])
                };
                let epd = self.paraxial().epd;
                let st = self.stop();
                let fr = self.frames();
                let go = |ex: f64, ey: f64| {
                    let (p, k) = self.start_ray(spec, ex, ey);
                    trace::trace_ray(self, fr, self.primary, p, k, 0.0, 1, st, false, None)
                };
                (go(epd / 2.0, 0.0).xyz[0], go(0.0, epd / 2.0).xyz[1])
            } else {
                let r = self.paraxial().stop_radius;
                (r, r)
            }
        })
    }

    /// 物体の指定と瞳座標の光線を追跡する
    #[allow(clippy::too_many_arguments)]
    pub fn trace_spec(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        vig: Option<[f64; 5]>,
        last: Option<usize>,
        record: bool,
        hxy: Option<(f64, f64)>,
    ) -> Trace {
        self.trace_spec_inner(spec, px, py, w, vig, last, record, hxy, None)
            .0
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn trace_spec_inner(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        vig: Option<[f64; 5]>,
        last: Option<usize>,
        record: bool,
        hxy: Option<(f64, f64)>,
        pol: Option<(&[[C64; 2]], &[PolSurf])>,
    ) -> (Trace, Vec<Vec<CV3>>, Option<Error>) {
        let n = px.len();
        let [vdx, vdy, vcx, vcy, van] = match vig {
            Some(v) => v,
            None => self
                .vig_factors(hxy.unwrap().0, hxy.unwrap().1)
                .unwrap_or([0.0; 5]),
        };
        let (mut qx, mut qy) = (px.to_vec(), py.to_vec());
        if vdx != 0.0 || vdy != 0.0 || vcx != 0.0 || vcy != 0.0 || van != 0.0 {
            let (s, c) = van.to_radians().sin_cos();
            for i in 0..n {
                let (a, b) = (vdx + px[i] * (1.0 - vcx), vdy + py[i] * (1.0 - vcy));
                qx[i] = a * c - b * s;
                qy[i] = a * s + b * c;
            }
        }
        let epd = self.paraxial().epd;
        let mut ex: Vec<f64> = qx.iter().map(|v| v * epd / 2.0).collect();
        let mut ey: Vec<f64> = qy.iter().map(|v| v * epd / 2.0).collect();
        let mut fail = vec![false; n];
        if self.aiming != Aiming::Off {
            self.aim(spec, &qx, &qy, &mut ex, &mut ey, &mut fail, w);
        }
        let fr = self.frames();
        let last = last.unwrap_or(self.image());
        let st = self.stop();
        let kw = 2.0 * std::f64::consts::PI / (self.wavelengths[w] * 1e-3);
        let one = |i: usize| {
            let (p, k) = self.start_ray(spec, ex[i], ey[i]);
            let mut rec = if record { Some(Vec::new()) } else { None };
            // 偏光: 物体の座標系の (Ex, Ey, 0) を光線に垂直な面へ射影する（規格化しない、OpticStudio と同じ）
            let mut pr = pol.map(|(inp, surf)| PolRay {
                e: inp
                    .iter()
                    .map(|a| {
                        let j: CV3 =
                            [0, 1, 2].map(|c| a[0] * fr.obj.r[c][0] + a[1] * fr.obj.r[c][1]);
                        let jk = j[0] * k[0] + j[1] * k[1] + j[2] * k[2];
                        [0, 1, 2].map(|c| j[c] - jk * k[c])
                    })
                    .collect(),
                surf,
                err: None,
            });
            let mut r = match pr.as_mut() {
                Some(q) => {
                    trace::trace_ray_pol(self, fr, w, p, k, 0.0, 1, last, false, rec.as_mut(), q)
                }
                None => trace::trace_ray(self, fr, w, p, k, 0.0, 1, last, false, rec.as_mut()),
            };
            if fail[i] {
                r.err = st;
            }
            // 最後の面の局所座標の電場に伝搬の位相 exp(−i k OPL) をかける
            // 偏光なしの追跡で結果を大きくしないよう箱に入れる
            let pe = pr.map(|mut pr| {
                let ph = C64::new(0.0, -kw * r.opl).exp();
                pr.e.iter_mut().for_each(|x| *x = x.map(|c| c * ph));
                Box::new((pr.e, pr.err))
            });
            (r, p, k, rec, pe)
        };
        // 光線は互いに独立なので、本数が多いときは並列に追う（結果の順序は入力のまま）
        let out: Vec<_> = if n >= PAR_MIN_RAYS {
            (0..n)
                .into_par_iter()
                .with_min_len(PAR_CHUNK)
                .map(one)
                .collect()
        } else {
            (0..n).map(one).collect()
        };
        let mut rays = Vec::with_capacity(n);
        let mut p0 = Vec::with_capacity(n);
        let mut k0 = Vec::with_capacity(n);
        let mut recs: Vec<Vec<RecEntry>> = Vec::new();
        let mut es = Vec::new();
        let mut first_err = None;
        for (r, p, k, rec, pe) in out {
            let through = r.err == 0 && r.vig == 0;
            rays.push(r);
            p0.push(p);
            k0.push(k);
            if let Some(rc) = rec {
                recs.push(rc);
            }
            if let Some(b) = pe {
                let (e, perr) = *b;
                es.push(e);
                // 束を止めるのは、最後まで通った（けられず、エイミングにも失敗していない）光線のエラーだけ。
                // ほかの光線は電場が NaN になるだけ
                if first_err.is_none() && through {
                    first_err = perr;
                }
            }
        }
        let mut parity = 1.0;
        let mut rec_out = Vec::new();
        for j in 1..=last {
            let d = &self.surfaces[j];
            if d.ignore {
                continue;
            }
            if d.mirror {
                parity = -parity;
            }
            if record {
                let q = recs
                    .iter()
                    .map(|rc| rc.iter().find(|e| e.surf == j).copied().unwrap())
                    .collect();
                rec_out.push((j, q, parity));
            }
        }
        let opl_os = match spec {
            ObjSpec::Dir(_) => {
                // 面 1 が無視（IGNR）なら、その次の有効な面（大域座標の基準と同じ）
                let j1 = (1..fr.surf.len())
                    .find(|&j| fr.surf[j].is_some())
                    .unwrap_or(1);
                let ob = fr.surf[j1].unwrap();
                let ax = col(&ob.r, 2);
                let nb = self.index_before(j1, w);
                (0..n)
                    .map(|i| rays[i].opl - dot(sub(ob.o, p0[i]), ax) / dot(k0[i], ax) * nb)
                    .collect()
            }
            ObjSpec::Point(_) => rays.iter().map(|r| r.opl).collect(),
        };
        (
            Trace {
                rays,
                parity,
                p0,
                k0,
                opl_os,
                rec: rec_out,
                last,
                hx_hy: hxy.unwrap_or((0.0, 0.0)),
            },
            es,
            first_err,
        )
    }

    /// 偏光追跡での面ごとの処理を波長 w について用意する（境界の係数、Jones 行列、媒質の吸収）
    pub fn pol_surfaces(&self, w: usize, coat: &Coatings) -> Result<Vec<PolSurf>> {
        let (keys, before) = coating::media_keys(self);
        let wl = self.wavelengths[w];
        let mut out = Vec::with_capacity(self.surfaces.len());
        for (j, d) in self.surfaces.iter().enumerate() {
            let it = self
                .glasses
                .get(&before[j])
                .and_then(|g| coating::internal_t(g, wl));
            let kind = if d.ignore || j == 0 {
                PolKind::None
            } else {
                match d.typ {
                    SurfType::Jones => {
                        let a = d.jones;
                        PolKind::Jones([
                            C64::new(a[0], a[1]),
                            C64::new(a[2], a[3]),
                            C64::new(a[4], a[5]),
                            C64::new(a[6], a[7]),
                        ])
                    }
                    SurfType::CoordBrk | SurfType::Paraxial => PolKind::None,
                    _ => match &keys[j] {
                        Some(k) => PolKind::Iface(coat.prepare(
                            self,
                            k,
                            wl,
                            self.index_before(j, w),
                            d.n[w],
                        )?),
                        None => PolKind::None,
                    },
                }
            };
            out.push(PolSurf { kind, it });
        }
        Ok(out)
    }

    /// 偏光追跡: 入力の偏光 pols（物体の座標系の (Ex, Ey)）ごとの電場を、最後の面の局所座標で
    /// 伝搬の位相 exp(−i k OPL) 込みで返す（OpticStudio の偏光追跡と同じ約束）
    pub fn try_trace_pol(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        pols: &[[C64; 2]],
        coat: &Coatings,
    ) -> Result<PolTrace> {
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = match field {
            FieldSel::Index(i) => self.hxhy(i),
            FieldSel::Norm(a, b) => (a, b),
        };
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        let ps = self.pol_surfaces(w, coat)?;
        let (mut t, e, err) = self.trace_spec_inner(
            spec,
            px,
            py,
            w,
            Some(vig),
            None,
            false,
            Some((hx, hy)),
            Some((pols, &ps)),
        );
        if let Some(e) = err {
            return Err(e);
        }
        t.hx_hy = (hx, hy);
        Ok(PolTrace { trace: t, e })
    }

    #[allow(clippy::too_many_arguments)]
    fn aim(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        ex: &mut [f64],
        ey: &mut [f64],
        fail: &mut [bool],
        w: usize,
    ) {
        let (rx, ry) = self.stop_radius();
        let s = self.stop();
        let fr = self.frames();
        let tol = 1e-12;
        let fail_tol = 1e-9;
        let scale_ = ex
            .iter()
            .chain(ey.iter())
            .fold(1.0f64, |m, v| m.max(v.abs()));
        let h = 1e-7 * scale_;
        let lim = tol * rx.max(ry).max(1.0);
        let at_stop = |v: [f64; 2]| -> [f64; 2] {
            let (p, k) = self.start_ray(spec, v[0], v[1]);
            let r = trace::trace_ray(self, fr, w, p, k, 0.0, 1, s, false, None);
            [r.xyz[0], r.xyz[1]]
        };
        for i in 0..px.len() {
            let tgt = [px[i] * rx, py[i] * ry];
            let mut v = [ex[i], ey[i]];
            let mut q = at_stop(v);
            for _ in 0..40 {
                let e = [q[0] - tgt[0], q[1] - tgt[1]];
                let ne = e[0].abs().max(e[1].abs());
                if !(ne > lim) {
                    break;
                }
                let qx = at_stop([v[0] + h, v[1]]);
                let qy = at_stop([v[0], v[1] + h]);
                let (a, b, c, d) = (
                    (qx[0] - q[0]) / h,
                    (qx[1] - q[1]) / h,
                    (qy[0] - q[0]) / h,
                    (qy[1] - q[1]) / h,
                );
                let det = a * d - b * c;
                let dv = [(d * e[0] - c * e[1]) / det, (-b * e[0] + a * e[1]) / det];
                if dv[0].is_finite() && dv[1].is_finite() {
                    v = [v[0] - dv[0], v[1] - dv[1]];
                }
                q = at_stop(v);
            }
            let e = [q[0] - tgt[0], q[1] - tgt[1]];
            let f = e[0].is_finite()
                && e[1].is_finite()
                && e[0].abs().max(e[1].abs()) > fail_tol * rx.max(ry).max(1.0);
            fail[i] = f;
            ex[i] = if f { 0.0 } else { v[0] };
            ey[i] = if f { 0.0 } else { v[1] };
        }
    }

    /// 視野の光線を瞳座標 (px, py) で波長 w（既定は主波長）、面 last（既定は像面）まで追跡する
    pub fn trace(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        record: bool,
    ) -> Trace {
        self.try_trace(field, px, py, w, last, record)
            .expect("trace")
    }

    pub fn try_trace(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        record: bool,
    ) -> Result<Trace> {
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = match field {
            FieldSel::Index(i) => self.hxhy(i),
            FieldSel::Norm(a, b) => (a, b),
        };
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        let mut t = self.trace_spec(spec, px, py, w, Some(vig), last, record, Some((hx, hy)));
        t.hx_hy = (hx, hy);
        Ok(t)
    }

    /// 追跡の前に、未対応の項目がないかを確かめる（理想レンズの媒質など）
    pub fn validate(&self) -> Result<()> {
        for (j, d) in self.surfaces.iter().enumerate() {
            if d.typ == SurfType::Paraxial && !d.ignore {
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), d.n[w]);
                if nb != 1.0 || na != 1.0 {
                    notimpl!("空気以外の中の理想レンズ（面 {j}: {nb} → {na}）");
                }
                let z = self.lens_conjugate(j);
                if z.is_finite() && z.abs() <= 1e12 && 1.0 / d.focal + 1.0 / z == 0.0 {
                    notimpl!("理想レンズが物体を無限遠に結像する（OPD）");
                }
            }
        }
        if self.surfaces[0].before.is_some() || self.surfaces[0].after != After::None {
            notimpl!("物体面の傾き偏心");
        }
        Ok(())
    }
}

/// 座標ブレーク（dx dy tx ty tz）を座標系に適用する
pub fn apply_cb(o: V3, r: M3, t: &Tilt) -> (V3, M3) {
    let [dx, dy, tx, ty, tz] = t.d;
    let (rx, ry, rz) = (rot(0, tx), rot(1, ty), rot(2, tz));
    if t.order == 0 {
        let o = add(o, mv(&r, [dx, dy, 0.0]));
        (o, mm(&mm(&mm(&r, &rx), &ry), &rz))
    } else {
        let r = mm(&mm(&mm(&r, &rz), &ry), &rx);
        (add(o, mv(&r, [dx, dy, 0.0])), r)
    }
}

/// 面ごとの座標系（大域座標 = 面 glrs の座標系）
pub fn frames(surfaces: &[Surface], glrs: usize) -> Frames {
    let mut o: V3 = [0.0; 3];
    let mut r: M3 = I3;
    let mut out: Vec<Option<Frame>> = vec![None; surfaces.len()];
    let t0 = surfaces[0].t;
    for (j, d) in surfaces.iter().enumerate() {
        if j == 0 {
            out[0] = if t0.is_finite() {
                Some(Frame {
                    o: [0.0, 0.0, -t0],
                    r: I3,
                })
            } else {
                None
            };
            continue;
        }
        if d.ignore {
            continue;
        }
        if d.typ == SurfType::CoordBrk {
            let (o2, r2) = apply_cb(
                o,
                r,
                &Tilt {
                    d: d.cb,
                    order: d.cb_order,
                },
            );
            o = o2;
            r = r2;
            out[j] = Some(Frame { o, r });
        } else {
            let (o_pre, r_pre) = (o, r);
            if let Some(b) = &d.before {
                (o, r) = apply_cb(o, r, b);
            }
            out[j] = Some(Frame { o, r });
            match &d.after {
                After::Reverse => {
                    o = o_pre;
                    r = r_pre;
                }
                After::Tilt(a) => (o, r) = apply_cb(o, r, a),
                After::None => {}
            }
        }
        o = add(o, scale(col(&r, 2), d.t));
    }
    let g = if glrs < out.len() && out[glrs].is_some() {
        out[glrs].unwrap()
    } else {
        // 面 1 が無視（IGNR）なら、その次の有効な面を基準にする
        out.iter()
            .skip(1)
            .flatten()
            .next()
            .copied()
            .unwrap_or(Frame {
                o: [0.0; 3],
                r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            })
    };
    let gt = transpose(&g.r);
    let res = out
        .iter()
        .map(|f| {
            f.map(|f| Frame {
                o: mtv(&g.r, sub(f.o, g.o)),
                r: mm(&gt, &f.r),
            })
        })
        .collect::<Vec<Option<Frame>>>();
    let mut rel = vec![None; res.len()];
    let mut prev: Option<Frame> = None;
    for (j, f) in res.iter().enumerate() {
        if let Some(f) = f {
            if let Some(q) = prev {
                let ft = transpose(&f.r);
                let r = mm(&ft, &q.r);
                let t = mv(&ft, sub(q.o, f.o));
                let ident = r == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
                let shift = (ident && t[0] == 0.0 && t[1] == 0.0).then_some(t[2]);
                rel[j] = Some(Rel { r, t, shift });
            }
            prev = Some(*f);
        }
    }
    Frames {
        surf: res,
        rel,
        obj: Frame {
            o: mv(&gt, scale(g.o, -1.0)),
            r: gt,
        },
    }
}
