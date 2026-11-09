//! 系: 面と系のデータ（波長、視野、絞り、光線エイミング）、近軸量、光線の生成。
//!
//! 流儀は OpticStudio に合わせる（レンズ単位 mm、波長 µm、角度 deg）。
//! - 近軸量: 主波長の y-nu 追跡（展開した軸に沿う。傾き・偏心・座標ブレークは入らず、鏡で向きが反転）
//! - 絞り: 入射瞳径、像側 F 値、物体側 NA、絞り径、近軸作動 F 値、物体側コーン角、物体側テレセントリック
//! - 視野: 角度（方向正接）、物体高、近軸像高、実像高。正規化は半径か矩形。定義済み視野のケラレ係数
//! - 瞳座標: エイミングなしは近軸入射瞳面、エイミングありは絞り面の (px, py)×絞り半径
//! - 無限遠物体の光線は面 1 の頂点を通り視野方向に垂直な面から出る（光路長はそこから数える）

use crate::aim::{AimFit, AimLane};
use crate::batch;
use crate::coating::{self, Coatings, C64};
use crate::error::{Error, Result};
use crate::glass::Glass;
use crate::math::*;
use crate::surface::{After, SurfType, Surface, Tilt};
use crate::trace::{self, PolKind, PolRay, PolSurf, RayOut, RecEntry, CV3};
use crate::{invalid, notimpl};
use rayon::prelude::*;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const ENPP_MAX: f64 = 1e10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FieldType {
    Angle,
    ObjH,
    ParH,
    RealH,
    /// 条件で定める視野: 視野の (x, y) が条件の目標値になる物体（有限なら物点、無限遠なら方向）を、
    /// 主波長の主光線で解く。実像高は `Solve(FieldCond::Position { surf: 像面 })` と同じ
    Solve(FieldCond),
}

/// 条件で定める視野の条件（主光線、局所座標）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FieldCond {
    /// 面 surf での位置 (x, y) = 視野の値
    Position { surf: usize },
    /// 面 s1 と s2 での方向余弦 (l, m) の和 = 視野の値（法線入射で戻る主光線なら 0）
    DirSum { s1: usize, s2: usize },
    /// 面 s1 と s2 での方向余弦 (l, m) の差（s1 − s2）= 視野の値
    DirDiff { s1: usize, s2: usize },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FieldNorm {
    Radial,
    Rect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ApertureKind {
    Epd,
    FNum,
    Na,
    Float,
    Pwfn,
    Cone,
}
/// アフォーカル像空間の角度の単位（.zmx の UNIT の値の 6 番目、OpticStudio の Afocal Mode Units）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AfocalUnit {
    Microradians,
    #[default]
    Milliradians,
    Radians,
    ArcSeconds,
    ArcMinutes,
    Degrees,
}

impl AfocalUnit {
    /// 1 単位あたりのラジアン
    pub fn radians(self) -> f64 {
        match self {
            AfocalUnit::Microradians => 1e-6,
            AfocalUnit::Milliradians => 1e-3,
            AfocalUnit::Radians => 1.0,
            AfocalUnit::ArcSeconds => std::f64::consts::PI / 648_000.0,
            AfocalUnit::ArcMinutes => std::f64::consts::PI / 10_800.0,
            AfocalUnit::Degrees => std::f64::consts::PI / 180.0,
        }
    }
    /// .zmx の UNIT 行の記号（UR・MR・RR・AS・AM・DG）
    pub fn code(self) -> &'static str {
        match self {
            AfocalUnit::Microradians => "UR",
            AfocalUnit::Milliradians => "MR",
            AfocalUnit::Radians => "RR",
            AfocalUnit::ArcSeconds => "AS",
            AfocalUnit::ArcMinutes => "AM",
            AfocalUnit::Degrees => "DG",
        }
    }
    pub fn from_code(c: &str) -> Option<AfocalUnit> {
        [
            AfocalUnit::Microradians,
            AfocalUnit::Milliradians,
            AfocalUnit::Radians,
            AfocalUnit::ArcSeconds,
            AfocalUnit::ArcMinutes,
            AfocalUnit::Degrees,
        ]
        .into_iter()
        .find(|u| u.code().eq_ignore_ascii_case(c))
    }
    /// 表示の名前（urad・mrad・rad・arcsec・arcmin・deg）
    pub fn label(self) -> &'static str {
        match self {
            AfocalUnit::Microradians => "urad",
            AfocalUnit::Milliradians => "mrad",
            AfocalUnit::Radians => "rad",
            AfocalUnit::ArcSeconds => "arcsec",
            AfocalUnit::ArcMinutes => "arcmin",
            AfocalUnit::Degrees => "deg",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Aiming {
    Off,
    Paraxial,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Fresnel・Cylinder Fresnel の面の光路長の基準点（面ごとの局所座標 (x, y)。空か範囲外なら頂点）。
    /// OpticStudio と同じく、同じ視野・波長の主光線の交点にする（[`System::trace_launch`] が入れる）
    pub fres_ref: Vec<[f64; 2]>,
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
    /// 主光線の近軸像面（マージナル光線の高さが 0 の面）での高さ。近軸像高の視野はこの面で決める（OpticStudio と同じ）
    pub chief_parimage_per_unit: f64,
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

/// 光線の始め方
#[derive(Debug, Clone, Copy)]
pub enum Launch<'a> {
    /// 物体と正規化瞳座標（ケラレ係数 vig、None なら視野 hxy のもの。エイミングは系の設定どおり）。
    /// signed なら、入射瞳が物体より前（ENPP + 物体距離 < 0）の有限物体系で瞳座標の符号を反転して与える
    /// （px = 1 が絞りの同じ側を通り、入射瞳が無限遠をまたいでも向きが連続。既定の false は OpticStudio と同じ）
    Pupil {
        spec: ObjSpec,
        signed: bool,
        px: &'a [f64],
        py: &'a [f64],
        vig: Option<[f64; 5]>,
        hxy: Option<(f64, f64)>,
    },
    /// 物体と、絞り面の局所座標の点 (x, y) [mm]。その点を通る実光線を光線ごとに解く
    /// （系のエイミングの設定・入射瞳の位置によらない）。解けない光線は絞りで止まった扱い
    Stop {
        spec: ObjSpec,
        x: &'a [f64],
        y: &'a [f64],
    },
    /// 開始位置と方向（大域座標。方向は規格化する）。光路長は開始位置から測る
    Rays { p: &'a [V3], k: &'a [V3] },
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
    /// 面ごとの記録（record のとき）。無視しない面 1..=last（[`RecSel::Surfs`] ならそのうち指定の面）の順
    pub rec: Vec<RecSurf>,
    pub last: usize,
    pub hx_hy: (f64, f64),
}

/// 偏光追跡の結果
#[derive(Debug, Clone)]
pub struct PolTrace {
    pub trace: Trace,
    /// 光線ごと・入力の偏光ごとの電場（最後の面の局所座標、既定は伝搬の位相込み）。追跡が止まった光線は NaN
    pub e: Vec<Vec<CV3>>,
}

impl Trace {
    pub fn ok(&self, i: usize) -> bool {
        self.rays[i].err == 0 && self.rays[i].vig == 0
    }
    /// 開口を無視して像面まで届いたか（けられても追跡は続く）。主光線などの基準の光線はこちらで見る
    /// （OpticStudio と同じく、基準の光線は開口・遮蔽を無視する。中心遮蔽の系など）
    pub fn traced(&self, i: usize) -> bool {
        self.rays[i].err == 0
    }
    /// 面 j の記録（無視する面と、record でない追跡では None）
    pub fn rec_at(&self, j: usize) -> Option<&RecSurf> {
        self.rec
            .binary_search_by_key(&j, |r| r.surf)
            .ok()
            .map(|k| &self.rec[k])
    }
}

/// 面ごとの記録（record）をどの面について残すか
#[derive(Debug, Clone, Copy)]
pub enum RecSel<'a> {
    /// 記録しない
    Off,
    /// 無視しない面 1..=last のすべて
    All,
    /// 指定した面だけ（面の番号。無視する面・last より後の面は除かれる）
    Surfs(&'a [usize]),
}

impl From<bool> for RecSel<'_> {
    fn from(b: bool) -> Self {
        if b {
            RecSel::All
        } else {
            RecSel::Off
        }
    }
}

/// 記録の配列（面ごと）の先頭。追跡の並列化で、光線 i は各配列の i 番目だけに書く
struct SoaPtrs {
    /// 記録の番号（無視しない面の順）→ 出力の面の番号（記録しない面は usize::MAX）
    slot: Vec<usize>,
    xyz: Vec<*mut V3>,
    lmn: Vec<*mut V3>,
    opl: Vec<*mut f64>,
    p: Vec<*mut V3>,
    n: usize,
}
// 安全: 書く位置は光線ごとに別（SoaRow::put を参照）で、配列は追跡の間は動かさない
unsafe impl Sync for SoaPtrs {}

impl SoaPtrs {
    fn new(r: &mut [RecSurf], slot: Vec<usize>, n: usize) -> SoaPtrs {
        SoaPtrs {
            slot,
            xyz: r.iter_mut().map(|x| x.xyz.as_mut_ptr()).collect(),
            lmn: r.iter_mut().map(|x| x.lmn.as_mut_ptr()).collect(),
            opl: r.iter_mut().map(|x| x.opl.as_mut_ptr()).collect(),
            p: r.iter_mut().map(|x| x.p.as_mut_ptr()).collect(),
            n,
        }
    }
}

struct SoaRow<'a> {
    out: &'a SoaPtrs,
    i: usize,
}

impl trace::RecSink for SoaRow<'_> {
    #[inline(always)]
    fn put(&mut self, k: usize, e: RecEntry) {
        let o = self.out;
        let k = match o.slot.get(k) {
            Some(&s) if s != usize::MAX => s,
            _ => return,
        };
        assert!(k < o.xyz.len() && self.i < o.n);
        // 安全: 範囲は上で確かめた。光線 i は SoaRow を 1 つだけ持ち、i 番目には他の光線が書かない
        unsafe {
            *o.xyz[k].add(self.i) = e.xyz;
            *o.lmn[k].add(self.i) = e.lmn;
            *o.opl[k].add(self.i) = e.opl;
            *o.p[k].add(self.i) = e.p;
        }
    }
}

/// まとめて追う光線の組の記録（組の l 番目は光線 base + l。端の組の余りは書かない）
struct SoaLanes<'a> {
    out: &'a SoaPtrs,
    base: usize,
}

impl batch::LaneSink for SoaLanes<'_> {
    #[inline(always)]
    fn put(&mut self, lane: usize, k: usize, e: RecEntry) {
        let i = self.base + lane;
        if i < self.out.n {
            trace::RecSink::put(&mut SoaRow { out: self.out, i }, k, e);
        }
    }
}

/// 1 つの面での全光線の記録（量ごとに光線の順に並べる）。止まった光線は NaN
#[derive(Debug, Clone)]
pub struct RecSurf {
    pub surf: usize,
    /// (-1)^(この面までに通った鏡の数)
    pub parity: f64,
    /// 面の局所座標での交点
    pub xyz: Vec<V3>,
    /// 面の後の方向（局所座標、物理的な向き）
    pub lmn: Vec<V3>,
    /// 面までの光路長
    pub opl: Vec<f64>,
    /// 大域の交点
    pub p: Vec<V3>,
}

impl RecSurf {
    pub fn entry(&self, i: usize) -> RecEntry {
        RecEntry {
            surf: self.surf,
            xyz: self.xyz[i],
            lmn: self.lmn[i],
            opl: self.opl[i],
            p: self.p[i],
        }
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
    /// 理想レンズの物体距離（面ごと。追跡の内側で読むのでロックなし）
    conj: OnceLock<Vec<f64>>,
    /// 面ごとの交点・法線の求め方（追跡の内側の分岐を前計算）。面の形で決まるので、[`Keep`] でも引き継がない
    kinds: OnceLock<Vec<crate::trace::SurfKind>>,
}

/// これ以上の本数で並列に追う
const PAR_MIN_RAYS: usize = 512;
/// 並列のときの 1 タスクあたりの最少本数
const PAR_CHUNK: usize = 256;
/// 実光線エイミングで当てはめの初期値を使う本数（格子の光線を解く分より得になる目安）
const AIM_FIT_MIN: usize = 1024;
/// 頂点の尖った面がある系の瞳を求める主光線の視野 [°]（[`System::cusp_pupil`]）
const CUSP_FIELD_DEG: f64 = 0.01;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// 像空間がアフォーカル（.zmx の FTYP の 7 番目）。波面収差・Zernike・OPD を、射出瞳の位置で主光線に
    /// 垂直な平面（平面波）を基準に測る。スポット・MTF・PSF は角度（[`System::image_point`]、単位は
    /// [`System::afocal_unit`]）で出す
    #[cfg_attr(feature = "serde", serde(default))]
    pub afocal: bool,
    /// アフォーカル像空間の角度の単位（.zmx の UNIT の値の 6 番目）
    #[cfg_attr(feature = "serde", serde(default))]
    pub afocal_unit: AfocalUnit,
    /// 実像高・条件で定める視野を解く波長（None で主波長）。単色の RMS の掃引で OpticStudio に合わせるとき
    /// だけ使う（[`crate::sweep::RmsOpts::field_wave`]）。変えたら [`System::changed`] を呼ぶ
    #[cfg_attr(feature = "serde", serde(skip))]
    pub field_wave: Option<usize>,
    pub coating_file: Option<PathBuf>,
    /// 膜ファイルの外で定めた TABLE 膜（名前 → 表。プロジェクト・カタログのフィルタの透過特性）。
    /// 膜ファイルの同じ名前より優先する
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub coat_tables: HashMap<String, crate::coating::Table>,
    pub name: String,
    pub adjust_index: bool,
    /// ピックアップ（.zmx の PZUP・PPAR・PKUP・曲率のピックアップ）。パラメータを変えると解決し直す（[`crate::param`]）
    pub pickups: Vec<crate::param::Pickup>,
    /// 厚さのソルブ（.zmx の MAZH）。パラメータを変えると解き直す（[`crate::param`]）
    pub solves: Vec<crate::param::ThickSolve>,
    /// ソルブが呼ぶ ZPL のマクロ（[`crate::param::SolveKind::MacroCurv`]・`MacroThick` の番号で引く）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub macros: Vec<crate::zpl_solve::Macro>,
    /// 面タイプのパラメータの ZPL マクロのソルブ
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub macro_parms: Vec<crate::zpl_solve::MacroParm>,
    /// マクロのソルブを外して保存された値で固定した理由（マクロがない・対応していない命令など）と、
    /// 解き直しでマクロがエラーになって値を変えなかったこと（[`System::macro_warnings`]）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub macro_notes: Vec<String>,
    /// 玉・群（[`crate::units`]）。剛体の動きは座標系にかける
    #[cfg_attr(feature = "serde", serde(default))]
    pub units: Vec<crate::units::Unit>,
    /// 機構部品（[`crate::mech`]）。光線の追跡には効かず、ケラレの確認と描画だけに使う
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub mechanics: Vec<crate::mech::MechPart>,
    /// 多重構成の表（[`crate::edit`]）。None なら構成は 1 つ
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub configs: Option<crate::edit::Configs>,
    #[cfg_attr(feature = "serde", serde(skip))]
    cache: Cache,
}

impl Clone for System {
    fn clone(&self) -> Self {
        self.clone_keep(Keep::default())
    }
}

/// 複製・変更のときに残してよいキャッシュ（変えたパラメータが影響しないもの）
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Keep {
    /// 面の座標系（厚さ・傾き・偏心・座標ブレーク・無視を変えていない）
    pub frames: bool,
    /// 近軸量（傾き・偏心・座標ブレークだけを変えた）
    pub paraxial: bool,
}

/// 光線ごとの開始位置・方向（大域）。瞳座標の光線は入射瞳の上の点だけを持ち、使うときに
/// [`System::start_ray`] で求める（大きな配列を作らない。値は先に求めるのと同じビット）
enum Starts {
    Done(Vec<(V3, V3)>),
    Pupil {
        geom: StartGeom,
        ex: Vec<f64>,
        ey: Vec<f64>,
    },
}

/// [`System::start_ray`] のうち瞳の点によらない量（物体の座標系、入射瞳の位置、平行光の方向・物点）。
/// 光線ごとに座標系・近軸量を引き直さないよう、1 回の追跡で 1 度だけ求める。値は同じ式・同じ順なので同じビット
#[derive(Clone, Copy)]
struct StartGeom {
    o1: V3,
    r0: M3,
    z: f64,
    kind: StartKind,
}

#[derive(Clone, Copy)]
enum StartKind {
    /// 大域の方向
    Dir(V3),
    /// 大域の物点
    Point(V3),
}

impl StartGeom {
    fn new(s: &System, spec: ObjSpec) -> StartGeom {
        let fr = s.frames();
        let (o1, r0) = (fr.obj.o, fr.obj.r);
        let z = s.paraxial().enpp.min(ENPP_MAX);
        let kind = match spec {
            ObjSpec::Dir(d) => StartKind::Dir(mv(&r0, d)),
            ObjSpec::Point(pt) => {
                let f0 = fr.surf[0].expect("有限物体の座標系");
                // 物体面が平面でない形（Tilted など）なら物点はその面の上（OpticStudio と同じ）
                let d0 = &s.surfaces[0];
                let z0 = if crate::freeform::is_freeform(d0.typ) {
                    crate::freeform::sag_xy(d0, pt[0], pt[1]).0
                } else {
                    0.0
                };
                StartKind::Point(add(f0.o, mv(&f0.r, [pt[0], pt[1], z0])))
            }
        };
        StartGeom { o1, r0, z, kind }
    }

    #[inline]
    fn ray(&self, ex: f64, ey: f64) -> (V3, V3) {
        let (o1, r0) = (self.o1, &self.r0);
        let e = add(o1, mv(r0, [ex, ey, self.z]));
        match self.kind {
            StartKind::Dir(d) => {
                let p = sub(e, scale(d, dot(sub(e, o1), d)));
                (p, d)
            }
            StartKind::Point(p) => {
                let mut k = sub(e, p);
                if dot(k, col(r0, 2)) < 0.0 {
                    k = scale(k, -1.0);
                }
                (p, normalize(k))
            }
        }
    }
}

impl Starts {
    fn len(&self) -> usize {
        match self {
            Starts::Done(v) => v.len(),
            Starts::Pupil { ex, .. } => ex.len(),
        }
    }

    #[inline]
    fn get(&self, i: usize) -> (V3, V3) {
        match self {
            Starts::Done(v) => v[i],
            Starts::Pupil { geom, ex, ey } => geom.ray(ex[i], ey[i]),
        }
    }
}

impl System {
    /// キャッシュのうち keep の分を引き継いで複製する
    pub(crate) fn clone_keep(&self, keep: Keep) -> System {
        let cache = Cache::default();
        if keep.frames {
            if let Some(f) = self.cache.frames.get() {
                let _ = cache.frames.set(f.clone());
            }
        }
        if keep.paraxial {
            if let Some(p) = self.cache.par.get() {
                let _ = cache.par.set(p.clone());
            }
            if let Some(c) = self.cache.conj.get() {
                let _ = cache.conj.set(c.clone());
            }
        }
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
            afocal: self.afocal,
            afocal_unit: self.afocal_unit,
            field_wave: self.field_wave,
            coating_file: self.coating_file.clone(),
            coat_tables: self.coat_tables.clone(),
            name: self.name.clone(),
            adjust_index: self.adjust_index,
            pickups: self.pickups.clone(),
            solves: self.solves.clone(),
            macros: self.macros.clone(),
            macro_parms: self.macro_parms.clone(),
            macro_notes: self.macro_notes.clone(),
            units: self.units.clone(),
            mechanics: self.mechanics.clone(),
            configs: self.configs.clone(),
            cache,
        }
    }
}

impl System {
    /// 分散式の有効範囲（AGF の LD 行）の外の波長で屈折率を求めている材料の警告（面の番号・材料・波長・範囲）。
    /// lensbench は式をそのまま延ばして計算する（OpticStudio は範囲外ではエラーにして屈折率を 1.0 にする）
    pub fn dispersion_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (k, d) in self.surfaces.iter().enumerate() {
            if d.ignore || d.mat.is_empty() {
                continue;
            }
            let Some(g) = self.glasses.get(&d.mat) else {
                continue;
            };
            let Some([lo, hi]) = g.ld else {
                continue;
            };
            for &l in &self.wavelengths {
                if (l < lo || l > hi) && seen.insert((d.mat.clone(), l.to_bits())) {
                    out.push(format!(
                        "面 {k} の材料 {}（{}）: 波長 {l} µm が分散式の範囲 {lo}〜{hi} µm の外（式を延ばして計算）",
                        d.mat, g.catalog
                    ));
                }
            }
        }
        out
    }

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
            afocal: false,
            afocal_unit: AfocalUnit::Milliradians,
            field_wave: None,
            coating_file: None,
            coat_tables: HashMap::new(),
            name: String::new(),
            adjust_index: true,
            pickups: Vec::new(),
            solves: Vec::new(),
            macros: Vec::new(),
            macro_parms: Vec::new(),
            macro_notes: Vec::new(),
            units: Vec::new(),
            mechanics: Vec::new(),
            configs: None,
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
        // GRIN の媒質の次の面（座標ブレークは通して、その次）は交点を求められる面タイプに限る
        for (j, d) in sf.iter().enumerate() {
            if d.ignore || d.mirror || d.grin.is_none() {
                continue;
            }
            if let Some((k, e)) = sf
                .iter()
                .enumerate()
                .skip(j + 1)
                .find(|(_, e)| !e.ignore && e.typ != SurfType::CoordBrk)
            {
                if !crate::grin::next_ok(e.typ) {
                    notimpl!("面 {j} の GRIN の媒質の次の面 {k}（{:?}）", e.typ);
                }
            }
        }
        Ok(())
    }

    /// キャッシュを捨てる（系のデータの確かめはしない）
    pub(crate) fn invalidate_frames(&mut self) {
        self.cache = Cache::default();
    }

    /// 面や系のデータを書き換えたあとに呼ぶ（座標系・近軸量などのキャッシュを捨てる）
    pub fn changed(&mut self) -> Result<()> {
        self.cache = Cache::default();
        self.check()
    }

    pub fn stop(&self) -> usize {
        self.surfaces.iter().position(|d| d.stop).unwrap_or(0)
    }
    /// エラーに添える面の名札: 「面 j(種類, 材料, 膜 名前, 「コメント」)」。空の項は省く
    pub fn surf_label(&self, j: usize) -> String {
        let Some(d) = self.surfaces.get(j) else {
            return format!("面 {j}");
        };
        let mut parts = vec![crate::edit::surf_type_name(d.typ).to_string()];
        if !d.mat.is_empty() && d.mat != "-" {
            parts.push(d.mat.clone());
        }
        if !d.coat.is_empty() {
            parts.push(format!("膜 {}", d.coat));
        }
        if !d.comment.is_empty() {
            parts.push(format!("「{}」", d.comment));
        }
        format!("面 {j}({})", parts.join(", "))
    }

    pub fn image(&self) -> usize {
        self.surfaces.len() - 1
    }
    pub fn infinite(&self) -> bool {
        !self.surfaces[0].t.is_finite()
    }

    /// 面ごとの交点・法線の求め方（[`crate::trace::surf_kind`]）
    pub(crate) fn surf_kinds(&self) -> &[crate::trace::SurfKind] {
        self.cache
            .kinds
            .get_or_init(|| self.surfaces.iter().map(crate::trace::surf_kind).collect())
    }

    pub fn frames(&self) -> &Frames {
        self.cache.frames.get_or_init(|| frames(self))
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

    /// 像面での光線の点。ふつうは像面の局所座標 (x, y) [mm]。アフォーカル像空間では向きの角度
    /// (atan(l/n), atan(m/n)) を [`System::afocal_unit`] の単位で（OpticStudio のスポットと同じ定義）
    pub fn image_point(&self, xyz: [f64; 3], lmn: [f64; 3]) -> (f64, f64) {
        if self.afocal {
            let u = self.afocal_unit.radians();
            ((lmn[0] / lmn[2]).atan() / u, (lmn[1] / lmn[2]).atan() / u)
        } else {
            (xyz[0], xyz[1])
        }
    }

    /// 像面の点・周波数の単位の名前: ふつうは mm、アフォーカル像空間では角度の単位
    pub fn image_unit(&self) -> &'static str {
        if self.afocal {
            self.afocal_unit.label()
        } else {
            "mm"
        }
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
        let mut gmed: Option<(&crate::grin::Grin, f64)> = None;
        y[0] = yy - uu * if sf[0].t.is_finite() { sf[0].t } else { 0.0 };
        u[0] = uu;
        for j in 1..=upto {
            let d = &sf[j];
            if d.ignore {
                continue;
            }
            y[j] = yy;
            match d.typ {
                // トロイダル面は YZ 面の近軸（OpticStudio と同じ）
                SurfType::Standard
                | SurfType::EvenAsph
                | SurfType::Toroidal
                | SurfType::ZernikeStd
                | SurfType::ZernikeFringe
                | SurfType::GridSag
                | SurfType::Fresnel
                | SurfType::CylFresnel
                | SurfType::Binary2
                | SurfType::Gradium
                | SurfType::Gradient9
                | SurfType::OddAsph
                | SurfType::TiltSurf
                | SurfType::Irregular => {
                    let a1 = d.asph.first().copied().unwrap_or(0.0);
                    let c = sgn
                        * if crate::freeform::is_freeform(d.typ) {
                            crate::freeform::paraxial_curv(d)
                        } else if let Some(f) =
                            d.cylf.as_ref().filter(|_| d.typ == SurfType::CylFresnel)
                        {
                            // 屈折は Fresnel の面（YZ 面）で決まる
                            f.c + 2.0 * f.coef.first().copied().unwrap_or(0.0)
                        } else {
                            d.c + 2.0 * a1
                        };
                    // Binary 2 の位相の 2 次の項: n′u′ に (λ/2π) ∂Φ/∂y を足す（λ はレンズの単位 mm）
                    let dphi = match d.bin2.as_ref().filter(|_| d.typ == SurfType::Binary2) {
                        Some(b) => {
                            let a = b.coef.first().copied().unwrap_or(0.0);
                            self.wavelengths[w] * 1e-3 / (2.0 * std::f64::consts::PI)
                                * b.order
                                * 2.0
                                * a
                                / (b.norm * b.norm)
                                * yy
                        }
                        None => 0.0,
                    };
                    if d.mirror {
                        uu += 2.0 * c * yy - sgn * dphi / n;
                        sgn = -sgn;
                    } else {
                        // GRIN の媒質へは頂点の屈折率で
                        let n2 = d
                            .grin
                            .as_deref()
                            .and_then(|g| g.vertex_index(self.wavelengths[w]))
                            .unwrap_or(d.n[w]);
                        uu = (n * uu - yy * c * (n2 - n) + sgn * dphi) / n2;
                        n = n2;
                    }
                }
                SurfType::Paraxial => uu -= yy / d.focal,
                SurfType::User => {
                    if let Some(u) = &d.user {
                        uu -= yy * u.instance().paraxial_power();
                    }
                }
                SurfType::Jones => n = d.n[w],
                SurfType::CoordBrk => {}
            }
            u[j] = uu;
            // GRIN の媒質（座標ブレークの後ろも続く）と、その面の局所 z の今の位置
            gmed = match (d.grin.as_deref().filter(|_| !d.mirror), d.typ) {
                (Some(g), _) => Some((g, 0.0)),
                (None, SurfType::CoordBrk) => gmed,
                _ => None,
            };
            if j < upto {
                match gmed {
                    // GRIN の媒質の中は分布の近軸の受け渡し（出口の屈折率が次の面の前の屈折率）
                    Some((g, z0)) => {
                        match g.paraxial(self.wavelengths[w], yy, uu, z0, d.t, sgn) {
                            Some((y2, u2, n2)) => {
                                yy = y2;
                                uu = u2;
                                n = n2;
                            }
                            None => {
                                yy = f64::NAN;
                                uu = f64::NAN;
                            }
                        }
                        gmed = Some((g, z0 + d.t));
                    }
                    None => yy += sgn * d.t * uu,
                }
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
        let n_i = self.image_index(pw);
        // 空気中の値（像空間の屈折率をかけた角度で割る）。像面がガラスの中でも OpticStudio の EFFL は
        // R/(n−1) の形（2026 R1.01 で実測）
        let efl = if uk[img - 1] != 0.0 {
            -1.0 / (n_i * uk[img - 1])
        } else {
            f64::INFINITY
        };
        let (kind, val) = self.aperture;
        let mut u_obj: Option<f64> = None;
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
        // 像面から近軸像面までずらした主光線の高さ（像面が近軸像面にあれば yci と同じ）
        let ycp = if umi != 0.0 {
            yci - uci * ymi / umi
        } else {
            yci
        };
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
            chief_parimage_per_unit: ycp,
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
        self.symmetric_opts(true)
    }

    /// [`System::symmetric`]。freeform が false なら Zernike・Grid Sag の面を見ない（OpticStudio の自動の
    /// 半直径の光線の取り方は、これらの面があっても回転対称の系と同じ）
    pub(crate) fn symmetric_opts(&self, freeform: bool) -> bool {
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
            // Zernike・Grid Sag・Odd Asphere・Tilted・Irregular の面は回転対称とみない（瞳は実光線で求める。OpticStudio と
            // 同じ。自作の系で、近軸の瞳にすると EXPD・OPDC の OpticStudio との差が大きくなった）
            if freeform && crate::freeform::is_freeform(d.typ) {
                return false;
            }
            // GRIN の公差の項・Gradient 9 の形の傾き
            if d.grin.as_deref().is_some_and(|g| !g.symmetric()) {
                return false;
            }
        }
        !self.units.iter().any(|u| u.moved())
    }

    /// 射出瞳の位置（像面からその局所 z に沿って。OPD の基準）。回転対称なら近軸、そうでなければ
    /// OpticStudio と同じく小さい y 視野の実主光線が軸上主光線と交わる位置（2 視野で外挿）
    pub fn exit_pupil(&self) -> f64 {
        *self.cache.expp.get_or_init(|| {
            let mut e = self.paraxial().expp;
            if !self.symmetric() {
                let r0 = self.trace_novig(0.0, 0.0, &[0.0], &[0.0]);
                let (p0, k0) = (r0.rays[0].xyz, r0.rays[0].lmn);
                // GRIN の分布・形を傾けた面も同じ（G8B で OpticStudio と OPDC 5e-7 波）
                let free = self.surfaces.iter().any(|d| {
                    !d.ignore
                        && (crate::freeform::is_freeform(d.typ)
                            || d.grin.as_deref().is_some_and(|g| !g.symmetric()))
                });
                let cross_z = |p1: V3, k1: V3| -> f64 {
                    if free {
                        // Zernike・Grid Sag・傾けた GRIN の面を含む系は y-z 面で交わる点（3 次元の最近点だと、
                        // 面の傾きの項で主光線が x にも振れる系でずれる。OpticStudio 26.1 の EXPP と、鏡 1 枚の
                        // Z2 で 3e-12、G3 で 8e-7 一致。非公開側の照合データ）
                        let det = k1[1] * k0[2] - k0[1] * k1[2];
                        let (ry, rz) = (p1[1] - p0[1], p1[2] - p0[2]);
                        let t = (rz * k1[1] - ry * k1[2]) / det;
                        p0[2] + k0[2] * t
                    } else {
                        // 傾き・偏心の系は 3 次元の最近点（C0・C1 で OpticStudio と一致）
                        let w0 = sub(p0, p1);
                        let (a, b, c) = (dot(k0, k0), dot(k0, k1), dot(k1, k1));
                        let (d, f) = (dot(k0, w0), dot(k1, w0));
                        add(p0, scale(k0, (b * f - c * d) / (a * c - b * b)))[2]
                    }
                };
                if self.cusp_pupil() {
                    // 頂点の尖った面がある系: OpticStudio と同じく視野 0.01° の実主光線 1 本（外挿しない。外挿すると
                    // 尖りの傾きの分が消える）。その光線の高さが絞りの後のどこかの面で近軸の主光線と逆の符号になれば
                    // （尖りで曲がって軸を横切る）、視野を 1/100 にした光線で求め直す
                    let yc = &self.paraxial().chief.0;
                    let (s, img) = (self.stop(), self.image());
                    let chief = |deg: f64| {
                        self.trace_spec(
                            ObjSpec::Dir(normalize([0.0, deg.to_radians().tan(), 1.0])),
                            &[0.0],
                            &[0.0],
                            self.primary,
                            Some([0.0; 5]),
                            None,
                            true,
                            None,
                        )
                    };
                    let mut r1 = chief(CUSP_FIELD_DEG);
                    let flipped = r1
                        .rec
                        .iter()
                        .any(|q| q.surf > s && q.surf < img && q.xyz[0][1] * yc[q.surf] < 0.0);
                    if flipped {
                        r1 = chief(CUSP_FIELD_DEG / 100.0);
                    }
                    e = cross_z(r1.rays[0].xyz, r1.rays[0].lmn);
                } else {
                    let mut z = [0.0; 2];
                    for (i, h) in [2e-4, 1e-4].iter().enumerate() {
                        let r1 = self.trace_novig(0.0, *h, &[0.0], &[0.0]);
                        z[i] = cross_z(r1.rays[0].xyz, r1.rays[0].lmn);
                    }
                    e = (4.0 * z[1] - z[0]) / 3.0;
                }
            }
            e
        })
    }

    /// r の 1 次の項（β1、頂点が尖る）がある Odd Asphere の面を含む、無限遠物体・角度の視野の系なら true。
    /// OpticStudio 2026 R1.01 はこの系の瞳を、視野 0.01° の実主光線 1 本と、軸上の視野の +y 側だけの瞳座標 1e-3 の
    /// 光線 1 本で求める（尖りの傾き β1 がそのまま瞳の位置・径に効く。β1 = ±1e-3・1e-5、入射瞳径 10・20、
    /// 最大視野 2・5・10° の自作の系で EXPP・EXPD が 1e-5 mm 一致。非公開側の照合データ）。
    /// 外挿や ±両側の光線で求める極限の値とは違う
    pub(crate) fn cusp_pupil(&self) -> bool {
        self.infinite()
            && self.field_type == FieldType::Angle
            && self.surfaces.iter().any(|d| {
                !d.ignore && d.typ == SurfType::OddAsph && d.asph.first().is_some_and(|&b| b != 0.0)
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
                // 頂点の尖った面がある系は OpticStudio と同じく +y 側の瞳座標 1e-3 の光線だけ（[`System::cusp_pupil`]）
                let cusp = self.cusp_pupil();
                let dl = if cusp { 1e-3 } else { 1e-4 };
                let r = self.trace_novig(0.0, 0.0, &[0.0, 0.0, 0.0], &[0.0, dl, -dl]);
                let p: Vec<V3> = r.rays.iter().map(|x| x.xyz).collect();
                let k: Vec<V3> = r.rays.iter().map(|x| x.lmn).collect();
                let c = k[0];
                let cc = add(p[0], scale(c, (e - p[0][2]) / c[2]));
                let q: Vec<V3> = (0..3)
                    .map(|i| add(p[i], scale(k[i], dot(sub(cc, p[i]), c) / dot(k[i], c))))
                    .collect();
                if cusp {
                    2.0 * norm(sub(q[1], q[0])) / dl
                } else {
                    norm(sub(q[1], q[2])) / dl
                }
            }
        })
    }

    /// 理想レンズ j の近軸の物体距離（局所 z、進行方向に。無限大は平行光）
    pub fn lens_conjugate(&self, j: usize) -> f64 {
        self.cache.conj.get_or_init(|| {
            let (ym, um) = &self.paraxial().marginal;
            (0..self.surfaces.len())
                .map(|j| {
                    let prev = (0..j)
                        .rev()
                        .find(|&i| !self.surfaces[i].ignore)
                        .unwrap_or(0);
                    let u_in = um[prev];
                    if u_in.abs() > 1e-15 * ym[j].abs().max(1.0) {
                        -ym[j] / u_in
                    } else {
                        f64::INFINITY
                    }
                })
                .collect()
        })[j]
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

    /// 視野 (hx, hy) のケラレ係数 [VDX, VDY, VCX, VCY, VAN]（視野の補間込み）
    pub fn vig_factors(&self, hx: f64, hy: f64) -> Result<[f64; 5]> {
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
        // 視野がすべて 0（軸上だけ）の系では、正規化視野は軸上のまま（OpticStudio 2026 R1 と同じ。正規化の値の
        // 1 は割り算のためだけ）
        let all0 = self.fields.iter().all(|f| f.x == 0.0 && f.y == 0.0);
        let (nx, ny) = if all0 {
            (0.0, 0.0)
        } else {
            self.field_norm_values()
        };
        let (x, y) = (hx * nx, hy * ny);
        let par = self.paraxial();
        let (tx, ty) = match self.field_type {
            FieldType::RealH => {
                return self.cond_spec(FieldCond::Position { surf: self.image() }, x, y)
            }
            FieldType::Solve(c) => return self.cond_spec(c, x, y),
            FieldType::Angle => (x.to_radians().tan(), y.to_radians().tan()),
            FieldType::ParH => {
                let h1 = par.chief_parimage_per_unit;
                if self.infinite() {
                    (x / h1, y / h1)
                } else {
                    return Ok(ObjSpec::Point([x / h1, y / h1]));
                }
            }
            FieldType::ObjH => return Ok(ObjSpec::Point([x, y])),
        };
        if self.infinite() {
            // 角度の視野が 90° を超えると光は物体側の後ろから来る（方向余弦 n < 0。OpticStudio と同じ）。
            // tan のままでは向きが反対になるので符号を戻す。ちょうど 90° は向きが決まらない
            if self.field_type == FieldType::Angle {
                let c = x.to_radians().cos() * y.to_radians().cos();
                if c == 0.0 || !c.is_finite() {
                    notimpl!("角度の視野 ({x}°, {y}°): ちょうど 90° の視野");
                }
                if c < 0.0 {
                    return Ok(ObjSpec::Dir(normalize([-tx, -ty, -1.0])));
                }
            }
            return Ok(ObjSpec::Dir(normalize([tx, ty, 1.0])));
        }
        let l = self.surfaces[0].t + par.enpp;
        Ok(ObjSpec::Point([-l * tx, -l * ty]))
    }

    /// 条件 c の目標値 (x, y) を満たす主光線の物体を解く（実像高と条件で定める視野）。結果は系にキャッシュする
    fn cond_spec(&self, c: FieldCond, x: f64, y: f64) -> Result<ObjSpec> {
        let img_s = self.image();
        let surfs = match c {
            FieldCond::Position { surf } => vec![surf],
            FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => vec![s1, s2],
        };
        for &j in &surfs {
            if j == 0 || j > img_s || self.surfaces[j].ignore {
                invalid!("視野の条件の面 {j}（無視しない面 1〜{img_s}）");
            }
        }
        let key = (x.to_bits(), y.to_bits());
        if let Some(v) = self.cache.realh.lock().unwrap().get(&key) {
            return Ok(*v);
        }
        // 初期値: 像面の位置なら近軸の像高から、ほかは軸上から
        let mut v = match c {
            FieldCond::Position { surf } if surf == img_s => {
                let h1 = self.paraxial().chief_image_per_unit;
                [x / h1, y / h1]
            }
            _ => [0.0, 0.0],
        };
        let spec = |v: [f64; 2]| {
            if self.infinite() {
                ObjSpec::Dir(normalize([v[0], v[1], 1.0]))
            } else {
                ObjSpec::Point(v)
            }
        };
        let img = |v: [f64; 2]| -> [f64; 2] {
            match c {
                FieldCond::Position { surf } => {
                    let r = self.trace_spec(
                        spec(v),
                        &[0.0],
                        &[0.0],
                        self.field_wave.unwrap_or(self.primary),
                        Some([0.0; 5]),
                        Some(surf),
                        false,
                        None,
                    );
                    [r.rays[0].xyz[0], r.rays[0].xyz[1]]
                }
                FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => {
                    let r = self.trace_spec(
                        spec(v),
                        &[0.0],
                        &[0.0],
                        self.field_wave.unwrap_or(self.primary),
                        Some([0.0; 5]),
                        Some(s1.max(s2)),
                        true,
                        None,
                    );
                    if r.rays[0].err != 0 {
                        return [f64::NAN; 2];
                    }
                    let (Some(a), Some(b)) = (r.rec_at(s1), r.rec_at(s2)) else {
                        return [f64::NAN; 2];
                    };
                    let (a, b) = (a.lmn[0], b.lmn[0]);
                    if matches!(c, FieldCond::DirSum { .. }) {
                        [a[0] + b[0], a[1] + b[1]]
                    } else {
                        [a[0] - b[0], a[1] - b[1]]
                    }
                }
            }
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
            match c {
                FieldCond::Position { surf } if surf == img_s => {
                    invalid!("実像高の視野 ({x}, {y}): 届く実主光線がない（{e} ずれる）")
                }
                _ => invalid!("条件 {c:?} の視野 ({x}, {y}): 解けない（{e} ずれる）"),
            }
        }
        let s = spec(v);
        self.cache.realh.lock().unwrap().insert(key, s);
        Ok(s)
    }

    // ---------------------------------------------------------------------------------- 光線
    /// 入射瞳面の点 (ex, ey) [mm] を通る光線の開始位置・方向（大域）
    pub fn start_ray(&self, spec: ObjSpec, ex: f64, ey: f64) -> (V3, V3) {
        StartGeom::new(self, spec).ray(ex, ey)
    }

    /// 瞳座標 1 が狙う絞りの半径 (rx, ry)。符号つき: 入射瞳の上端を通る近軸光線が絞りで負の側に来る系
    /// （入射瞳が物体より前の有限物体系など）では負（OpticStudio と同じく、px = 1 の光線が絞りの −x 側を通る）
    pub fn stop_radius(&self) -> (f64, f64) {
        *self.cache.rs.get_or_init(|| {
            let par = self.paraxial();
            let st = self.stop();
            // 入射瞳の +x の点を通る近軸光線（有限物体は物点から、入射瞳が物体より前なら遠ざかる向き）の絞りでの高さの符号
            let h = if self.infinite() {
                self.ptrace(1.0, 0.0, self.primary, Some(st)).0[st]
            } else {
                let u = 1.0 / (par.enpp.min(ENPP_MAX) + self.surfaces[0].t);
                self.ptrace(u * self.surfaces[0].t, u, self.primary, Some(st))
                    .0[st]
            };
            let sgn = if h < 0.0 { -1.0 } else { 1.0 };
            if self.aperture.0 == ApertureKind::Float {
                let s = sgn * self.surfaces[self.stop()].semi;
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
                let r = sgn * par.stop_radius;
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
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
    ) -> (Trace, Vec<Vec<CV3>>, Option<Error>) {
        let l = Launch::Pupil {
            spec,
            signed: false,
            px,
            py,
            vig,
            hxy,
        };
        self.trace_launch_inner(&l, w, last, RecSel::from(record), pol)
    }

    /// 光線の始め方 l から、光線ごとの開始位置・方向（大域）と、解けなかった光線の印を求める
    fn launch_rays(&self, l: &Launch, w: usize) -> (Vec<(V3, V3)>, Vec<bool>) {
        let (st, fail) = self.launch_starts(l, w);
        let st = match st {
            Starts::Done(v) => v,
            Starts::Pupil { .. } => {
                let start = |i: usize| st.get(i);
                let n = st.len();
                if n >= PAR_MIN_RAYS && !crate::par::sequential() {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(start)
                        .collect()
                } else {
                    (0..n).map(start).collect()
                }
            }
        };
        (st, fail)
    }

    /// [`Self::launch_rays`] で、瞳座標の光線は開始位置・方向をまだ求めない（追跡の組の中で求める）
    fn launch_starts(&self, l: &Launch, w: usize) -> (Starts, Vec<bool>) {
        match *l {
            Launch::Pupil {
                spec,
                signed,
                px,
                py,
                vig,
                hxy,
            } => {
                let n = px.len();
                // 瞳座標（符号の反転・けられ係数をかけたもの）。作業の配列はこの 2 本だけ
                let sg = if signed && self.pupil_reversed() {
                    -1.0
                } else {
                    1.0
                };
                let [vdx, vdy, vcx, vcy, van] = match vig {
                    Some(v) => v,
                    None => {
                        let (hx, hy) = hxy.unwrap_or((0.0, 0.0));
                        self.vig_factors(hx, hy).unwrap_or([0.0; 5])
                    }
                };
                let (qx, qy): (Vec<f64>, Vec<f64>) =
                    if vdx != 0.0 || vdy != 0.0 || vcx != 0.0 || vcy != 0.0 || van != 0.0 {
                        let (s, c) = van.to_radians().sin_cos();
                        (0..n)
                            .map(|i| {
                                let (px, py) = (sg * px[i], sg * py[i]);
                                let (a, b) = (vdx + px * (1.0 - vcx), vdy + py * (1.0 - vcy));
                                (a * c - b * s, a * s + b * c)
                            })
                            .unzip()
                    } else if sg < 0.0 {
                        (
                            px.iter().map(|v| -v).collect(),
                            py.iter().map(|v| -v).collect(),
                        )
                    } else {
                        (px.to_vec(), py.to_vec())
                    };
                let epd = self.paraxial().epd;
                let mut ex: Vec<f64> = qx.iter().map(|v| v * epd / 2.0).collect();
                let mut ey: Vec<f64> = qy.iter().map(|v| v * epd / 2.0).collect();
                let mut fail = vec![false; n];
                if self.aiming != Aiming::Off {
                    self.aim(spec, &qx, &qy, &mut ex, &mut ey, &mut fail, w);
                }
                (
                    Starts::Pupil {
                        geom: StartGeom::new(self, spec),
                        ex,
                        ey,
                    },
                    fail,
                )
            }
            Launch::Stop { spec, x, y } => {
                let (st, fail) = self.aim_stop(spec, x, y, w);
                (Starts::Done(st), fail)
            }
            Launch::Rays { p, k } => {
                let st = p
                    .iter()
                    .zip(k)
                    .map(|(&p, &k)| {
                        let nk = norm(k);
                        (
                            p,
                            if nk > 0.0 {
                                scale(k, 1.0 / nk)
                            } else {
                                [f64::NAN; 3]
                            },
                        )
                    })
                    .collect();
                (Starts::Done(st), vec![false; p.len()])
            }
        }
    }

    /// 有限物体で入射瞳が物体より前にあるか（ENPP + 物体距離 < 0）。このとき OpticStudio の瞳座標の光線は
    /// 入射瞳の点から遠ざかる向きになり、px = 1 は絞りの反対側を通る（[`Launch::Pupil`] の signed）
    pub fn pupil_reversed(&self) -> bool {
        !self.infinite() && self.paraxial().enpp + self.surfaces[0].t < 0.0
    }

    /// 物体の座標系（有限物体は物体面、無限遠物体は面 1 の変換の前の座標系）の原点と回転（大域）。
    /// [`Launch::Rays`] に物体の座標系で与えた光線を大域に直すのに使う
    pub fn object_frame(&self) -> Frame {
        let fr = self.frames();
        fr.surf[0].unwrap_or(fr.obj)
    }

    /// 光線の始め方 l で追跡する（波長 w、面 last まで、record なら面ごとの記録つき）
    pub fn trace_launch(&self, l: &Launch, w: usize, last: Option<usize>, record: bool) -> Trace {
        self.trace_launch_inner(l, w, last, RecSel::from(record), None)
            .0
    }

    /// [`Self::trace_launch`] で、記録する面を選ぶ（[`RecSel`]）。面の多い系で一部の面だけ記録すると速い
    pub fn trace_launch_rec(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: RecSel<'_>,
    ) -> Trace {
        self.trace_launch_inner(l, w, last, rec, None).0
    }

    /// 光線の始め方 l での偏光追跡（[`Self::trace_pol_spec`] と同じ約束）
    pub fn trace_pol_launch(
        &self,
        l: &Launch,
        w: usize,
        pols: &[[C64; 2]],
        coat: &Coatings,
        phase: bool,
    ) -> Result<PolTrace> {
        let ps = self.pol_surfaces(w, coat)?;
        let (t, e, err) =
            self.trace_launch_inner(l, w, None, RecSel::Off, Some((pols, &ps, phase)));
        if let Some(e) = err {
            return Err(e);
        }
        Ok(PolTrace { trace: t, e })
    }

    /// Fresnel・Cylinder Fresnel の面（面 last まで）があれば、同じ物体・視野・波長の主光線（瞳座標 0、絞りの中心）の
    /// 交点をその面の光路長の基準点にした座標系（[`Frames::fres_ref`]）。OpticStudio と同じ約束。
    /// 光線を直接与えたとき（主光線がない）と主光線が通らないときは頂点
    fn fresnel_frames(&self, l: &Launch, w: usize, last: usize) -> Option<Frames> {
        let is_f = |j: usize| {
            let d = &self.surfaces[j];
            !d.ignore && matches!(d.typ, SurfType::Fresnel | SurfType::CylFresnel)
        };
        if !(1..=last).any(is_f) {
            return None;
        }
        let zero = [0.0];
        let chief = match *l {
            Launch::Pupil {
                spec,
                signed,
                vig,
                hxy,
                ..
            } => Launch::Pupil {
                spec,
                signed,
                px: &zero,
                py: &zero,
                vig,
                hxy,
            },
            Launch::Stop { spec, .. } => Launch::Stop {
                spec,
                x: &zero,
                y: &zero,
            },
            Launch::Rays { .. } => return None,
        };
        let (st, fail) = self.launch_rays(&chief, w);
        let fr = self.frames();
        let mut f = fr.clone();
        f.fres_ref = vec![[0.0; 2]; self.surfaces.len()];
        if fail[0] {
            return Some(f);
        }
        let nrec = (1..=last).filter(|&j| !self.surfaces[j].ignore).count();
        let mut rec = vec![trace::RecEntry::NAN; nrec];
        let (p, k) = st[0];
        let r = trace::trace_ray(
            self,
            fr,
            w,
            p,
            k,
            0.0,
            1,
            last,
            trace::floating_clip(),
            Some(&mut rec),
        );
        if r.err == 0 {
            for e in &rec {
                if is_f(e.surf) {
                    f.fres_ref[e.surf] = [e.xyz[0], e.xyz[1]];
                }
            }
        }
        Some(f)
    }

    #[allow(clippy::type_complexity)]
    fn trace_launch_inner(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: RecSel<'_>,
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
    ) -> (Trace, Vec<Vec<CV3>>, Option<Error>) {
        let (starts, fail) = self.launch_starts(l, w);
        let n = starts.len();
        let (spec, hxy) = match *l {
            Launch::Pupil { spec, hxy, .. } => (Some(spec), hxy),
            Launch::Stop { spec, .. } => (Some(spec), None),
            Launch::Rays { .. } => (None, None),
        };
        let last = last.unwrap_or(self.image());
        let fres_fr = self.fresnel_frames(l, w, last);
        let fr = fres_fr.as_ref().unwrap_or(self.frames());
        let clip = trace::floating_clip();
        let st = self.stop();
        let kw = 2.0 * std::f64::consts::PI / (self.wavelengths[w] * 1e-3);
        // 記録する面（無視しない面）の配列を先に用意し、光線ごとにその位置へ直接書く
        let mut rec_out: Vec<RecSurf> = Vec::new();
        let mut slot = Vec::new();
        let record = !matches!(rec, RecSel::Off);
        if record {
            let mut heads = Vec::new();
            let mut parity = 1.0;
            for j in 1..=last {
                let d = &self.surfaces[j];
                if d.ignore {
                    continue;
                }
                if d.mirror {
                    parity = -parity;
                }
                let want = match rec {
                    RecSel::Surfs(v) => v.contains(&j),
                    _ => true,
                };
                if want {
                    slot.push(heads.len());
                    heads.push((j, parity));
                } else {
                    slot.push(usize::MAX);
                }
            }
            // 配列の確保と NaN の書き込み（新しいページの割り当て）が追跡より重くなるので、面ごとに並列に
            let mk = |&(surf, parity): &(usize, f64)| RecSurf {
                surf,
                parity,
                xyz: vec![[f64::NAN; 3]; n],
                lmn: vec![[f64::NAN; 3]; n],
                opl: vec![f64::NAN; n],
                p: vec![[f64::NAN; 3]; n],
            };
            rec_out = if n * heads.len() >= 1 << 16 && !crate::par::sequential() {
                heads.par_iter().map(mk).collect()
            } else {
                heads.iter().map(mk).collect()
            };
        }
        let ptrs = SoaPtrs::new(&mut rec_out, slot, n);
        let one = |i: usize| {
            let (p, k) = starts.get(i);
            let mut row = SoaRow { out: &ptrs, i };
            let rec = if record { Some(&mut row) } else { None };
            // 偏光: 物体の座標系の (Ex, Ey, 0) を光線に垂直な面へ射影する（規格化しない、OpticStudio と同じ）
            let mut pr = pol.map(|(inp, surf, _)| PolRay {
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
                Some(q) => trace::trace_core(self, fr, w, p, k, 0.0, 1, last, clip, rec, Some(q)),
                None => trace::trace_core(self, fr, w, p, k, 0.0, 1, last, clip, rec, None),
            };
            if fail[i] {
                r.err = st;
            }
            // 最後の面の局所座標の電場に伝搬の位相 exp(−i k OPL) をかける（phase のとき）
            // 偏光なしの追跡で結果を大きくしないよう箱に入れる
            let phase = pol.is_some_and(|p| p.2);
            let pe = pr.map(|mut pr| {
                if phase {
                    let ph = C64::new(0.0, -kw * r.opl).exp();
                    pr.e.iter_mut().for_each(|x| *x = x.map(|c| c * ph));
                }
                Box::new((pr.e, pr.err))
            });
            (r, p, k, pe)
        };
        // 光線は互いに独立なので、本数が多いときは並列に追う（結果の順序は入力のまま、
        // スレッド数によらず同じビット）。with_threads(Some(1), ..) の中では逐次
        let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
        // 偏光なしは L 本ずつまとめて追う（1 本ずつと同じビット。組は入力の順に固定）
        let plan = if pol.is_none() && batch::use_for(n) {
            Some(batch::Plan::new(self, fr, w, 1, last, clip)).filter(|p| !p.is_empty())
        } else {
            None
        };
        let mut rays: Vec<RayOut>;
        let mut p0: Vec<V3>;
        let mut k0: Vec<V3>;
        let mut es = Vec::new();
        let mut first_err = None;
        if let Some(plan) = &plan {
            // 結果の配列へ組ごとに直接書く（中間の配列を作らない）
            // 結果の配列は初期化せずに確保し、組ごとに（並列に）全要素を書いてから長さを決める
            // （大きな配列の初期化を 1 スレッドでしないため）
            rays = Vec::with_capacity(n);
            p0 = Vec::with_capacity(n);
            k0 = Vec::with_capacity(n);
            const L: usize = batch::L;
            type Out<'a, T> = &'a mut [MaybeUninit<T>];
            let group = |(b, ((ro, po), ko)): (usize, ((Out<RayOut>, Out<V3>), Out<V3>))| {
                // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
                let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
                let start = idx.map(|i| starts.get(i));
                let (p, k) = (start.map(|s| s.0), start.map(|s| s.1));
                let mut sink = SoaLanes {
                    out: &ptrs,
                    base: b * L,
                };
                let r = batch::trace_lanes(plan, &p, &k, record.then_some(&mut sink));
                for l in 0..ro.len() {
                    let mut x = r[l];
                    if fail[idx[l]] {
                        x.err = st;
                    }
                    ro[l].write(x);
                    po[l].write(p[l]);
                    ko[l].write(k[l]);
                }
            };
            let (ru, pu, ku) = (
                &mut rays.spare_capacity_mut()[..n],
                &mut p0.spare_capacity_mut()[..n],
                &mut k0.spare_capacity_mut()[..n],
            );
            if par {
                ru.par_chunks_mut(L)
                    .zip(pu.par_chunks_mut(L))
                    .zip(ku.par_chunks_mut(L))
                    .enumerate()
                    .with_min_len(PAR_CHUNK / L)
                    .for_each(group);
            } else {
                ru.chunks_mut(L)
                    .zip(pu.chunks_mut(L))
                    .zip(ku.chunks_mut(L))
                    .enumerate()
                    .for_each(group);
            }
            // 安全: 上で 0..n の全要素を書いた（組 b は b·L から L 本、端の組は残りの本数を書く）
            unsafe {
                rays.set_len(n);
                p0.set_len(n);
                k0.set_len(n);
            }
            drop(ptrs);
        } else {
            let out: Vec<_> = if par {
                (0..n)
                    .into_par_iter()
                    .with_min_len(PAR_CHUNK)
                    .map(one)
                    .collect()
            } else {
                (0..n).map(one).collect()
            };
            drop(ptrs);
            rays = Vec::with_capacity(n);
            p0 = Vec::with_capacity(n);
            k0 = Vec::with_capacity(n);
            for (r, p, k, pe) in out {
                let through = r.err == 0 && r.vig == 0;
                rays.push(r);
                p0.push(p);
                k0.push(k);
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
        }
        let mut parity = 1.0;
        for j in 1..=last {
            let d = &self.surfaces[j];
            if !d.ignore && d.mirror {
                parity = -parity;
            }
        }
        let opl_os = match spec {
            Some(ObjSpec::Dir(_)) => {
                // 面 1 が無視（IGNR）なら、その次の有効な面（大域座標の基準と同じ）
                let j1 = (1..fr.surf.len())
                    .find(|&j| fr.surf[j].is_some())
                    .unwrap_or(1);
                let ob = fr.surf[j1].unwrap();
                let ax = col(&ob.r, 2);
                let nb = self.index_before(j1, w);
                let f = |i: usize| rays[i].opl - dot(sub(ob.o, p0[i]), ax) / dot(k0[i], ax) * nb;
                if par {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(f)
                        .collect()
                } else {
                    (0..n).map(f).collect()
                }
            }
            _ => rays.iter().map(|r| r.opl).collect(),
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
                        let a = match &d.retarder {
                            Some(r) => r.jones8(wl),
                            None => d.jones,
                        };
                        PolKind::Jones([
                            C64::new(a[0], a[1]),
                            C64::new(a[2], a[3]),
                            C64::new(a[4], a[5]),
                            C64::new(a[6], a[7]),
                        ])
                    }
                    SurfType::CoordBrk | SurfType::Paraxial | SurfType::User => PolKind::None,
                    _ => match &keys[j] {
                        Some(k) => PolKind::Iface(
                            coat.prepare(
                                self,
                                k,
                                wl,
                                self.index_before(j, w),
                                match &d.reflect {
                                    Some(r) if d.mirror => r.n[w],
                                    _ => d.n[w],
                                },
                            )
                            .map_err(|e| e.at(self.surf_label(j)))?,
                        ),
                        None => PolKind::None,
                    },
                }
            };
            out.push(PolSurf::new(kind, it));
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
        self.try_trace_pol_opts(field, px, py, w, pols, coat, true)
    }

    /// [`Self::try_trace_pol`] で、伝搬の位相 exp(−i k OPL) をかけるかを選ぶ（phase = false なら膜・境界・
    /// Jones 面の振幅と位相だけ。光路長は `trace.rays[i].opl` にある）
    #[allow(clippy::too_many_arguments)]
    pub fn try_trace_pol_opts(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        pols: &[[C64; 2]],
        coat: &Coatings,
        phase: bool,
    ) -> Result<PolTrace> {
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = match field {
            FieldSel::Index(i) => self.hxhy(i),
            FieldSel::Norm(a, b) => (a, b),
        };
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        self.trace_pol_spec(
            spec,
            px,
            py,
            w,
            Some(vig),
            Some((hx, hy)),
            pols,
            coat,
            phase,
        )
    }

    /// 物体の指定（物点・方向）と瞳座標の偏光追跡（[`Self::try_trace_pol_opts`] の物体を直接与える形）。
    /// vig はケラレ係数（None なら視野 hxy のもの）
    #[allow(clippy::too_many_arguments)]
    pub fn trace_pol_spec(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        vig: Option<[f64; 5]>,
        hxy: Option<(f64, f64)>,
        pols: &[[C64; 2]],
        coat: &Coatings,
        phase: bool,
    ) -> Result<PolTrace> {
        let ps = self.pol_surfaces(w, coat)?;
        let (mut t, e, err) = self.trace_spec_inner(
            spec,
            px,
            py,
            w,
            vig,
            None,
            false,
            hxy,
            Some((pols, &ps, phase)),
        );
        if let Some(e) = err {
            return Err(e);
        }
        t.hx_hy = hxy.unwrap_or((0.0, 0.0));
        Ok(PolTrace { trace: t, e })
    }

    /// 物体 spec から絞り面の局所座標 (x, y) を通る実光線を光線ごとに解く（[`Launch::Stop`]）。
    /// 未知数は、有限物体なら物点からの方向の傾き (l/n, m/n)（面 1 の変換の前の座標系）、無限遠物体なら
    /// 入射瞳面での位置。入射瞳が無限遠に近い系でも未知数の大きさが変わらない
    fn aim_stop(
        &self,
        spec: ObjSpec,
        x: &[f64],
        y: &[f64],
        w: usize,
    ) -> (Vec<(V3, V3)>, Vec<bool>) {
        let s = self.stop();
        let fr = self.frames();
        let r0 = fr.obj.r;
        let geom = StartGeom::new(self, spec);
        let start = |v: [f64; 2]| -> (V3, V3) {
            match spec {
                ObjSpec::Dir(_) => geom.ray(v[0], v[1]),
                ObjSpec::Point(pt) => {
                    let f0 = fr.surf[0].expect("有限物体の座標系");
                    let p = add(f0.o, mv(&f0.r, [pt[0], pt[1], 0.0]));
                    (p, normalize(mv(&r0, [v[0], v[1], 1.0])))
                }
            }
        };
        let at_stop = |v: [f64; 2]| -> [f64; 2] {
            let (p, k) = start(v);
            let r = trace::trace_ray(self, fr, w, p, k, 0.0, 1, s, false, None);
            if r.err != 0 {
                return [f64::NAN; 2];
            }
            [r.xyz[0], r.xyz[1]]
        };
        let h = match spec {
            ObjSpec::Point(_) => 1e-7,
            ObjSpec::Dir(_) => 1e-7 * (self.paraxial().epd / 2.0).max(1.0),
        };
        let jac = |v: [f64; 2], q: [f64; 2]| -> [f64; 4] {
            let qx = at_stop([v[0] + h, v[1]]);
            let qy = at_stop([v[0], v[1] + h]);
            [
                (qx[0] - q[0]) / h,
                (qy[0] - q[0]) / h,
                (qx[1] - q[1]) / h,
                (qy[1] - q[1]) / h,
            ]
        };
        // J v = e を解く（J は行優先 [a b; c d]）
        let solve2 = |j: [f64; 4], e: [f64; 2]| -> [f64; 2] {
            let det = j[0] * j[3] - j[1] * j[2];
            [
                (j[3] * e[0] - j[1] * e[1]) / det,
                (-j[2] * e[0] + j[0] * e[1]) / det,
            ]
        };
        let newton = |mut v: [f64; 2], tgt: [f64; 2]| -> ([f64; 2], f64) {
            let lim = 1e-12 * tgt[0].abs().max(tgt[1].abs()).max(1.0);
            let mut q = at_stop(v);
            for _ in 0..40 {
                let e = [q[0] - tgt[0], q[1] - tgt[1]];
                if !(e[0].abs().max(e[1].abs()) > lim) {
                    break;
                }
                let dv = solve2(jac(v, q), e);
                if !(dv[0].is_finite() && dv[1].is_finite()) {
                    break;
                }
                v = [v[0] - dv[0], v[1] - dv[1]];
                q = at_stop(v);
            }
            let e = (q[0] - tgt[0]).abs().max((q[1] - tgt[1]).abs());
            (v, if e.is_finite() { e } else { f64::INFINITY })
        };
        // 絞りの中心を通る光線を解き、その点のヤコビアンで光線ごとの初期値を作る
        let v0 = match spec {
            ObjSpec::Dir(_) => [0.0, 0.0],
            ObjSpec::Point(_) => {
                let (_, k) = self.start_ray(spec, 0.0, 0.0);
                let kk = mtv(&r0, k);
                [kk[0] / kk[2], kk[1] / kk[2]]
            }
        };
        let (vc, _) = newton(v0, [0.0, 0.0]);
        let jc = jac(vc, at_stop(vc));
        let one = |i: usize| -> ((V3, V3), bool) {
            let tgt = [x[i], y[i]];
            let d = solve2(jc, tgt);
            let g = if d[0].is_finite() && d[1].is_finite() {
                [vc[0] + d[0], vc[1] + d[1]]
            } else {
                vc
            };
            let (v, e) = newton(g, tgt);
            let fail = !(tgt[0].is_finite() && tgt[1].is_finite())
                || !(e <= 1e-9 * tgt[0].abs().max(tgt[1].abs()).max(1.0));
            (start(v), fail)
        };
        let n = x.len().min(y.len());
        let out: Vec<_> = if n >= PAR_MIN_RAYS && !crate::par::sequential() {
            (0..n)
                .into_par_iter()
                .with_min_len(PAR_CHUNK)
                .map(one)
                .collect()
        } else {
            (0..n).map(one).collect()
        };
        out.into_iter().unzip()
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
        let fail_tol = 1e-9 * rx.max(ry).max(1.0);
        let scale_ = ex
            .iter()
            .chain(ey.iter())
            .fold(1.0f64, |m, v| m.max(v.abs()));
        let h = 1e-7 * scale_;
        let lim = tol * rx.max(ry).max(1.0);
        let geom = StartGeom::new(self, spec);
        let at_stop = |v: [f64; 2]| -> [f64; 2] {
            let (p, k) = geom.ray(v[0], v[1]);
            let r = trace::trace_ray(self, fr, w, p, k, 0.0, 1, s, false, None);
            [r.xyz[0], r.xyz[1]]
        };
        // 1 本ずつの反復
        let one = |mut a: AimLane| -> AimLane {
            a.q = at_stop(a.v);
            while let Some(j) = a.check(lim) {
                let v = a.v;
                let jac = j.then(|| (at_stop([v[0] + h, v[1]]), at_stop([v[0], v[1] + h])));
                a.update(h, jac);
                a.q = at_stop(a.v);
            }
            a
        };
        // L 本ずつまとめて同じ反復をする（光線ごとの手順・結果は上の one と同じビット。
        // 収束した光線はそこで止め、残りの光線の反復だけを続ける）
        const L: usize = batch::L;
        let plan = batch::Plan::new(self, fr, w, 1, s, false);
        let at_stop_l = |v: &[[f64; 2]; L]| -> [[f64; 2]; L] {
            let st = v.map(|v| geom.ray(v[0], v[1]));
            let r =
                batch::trace_lanes::<batch::NoRec>(&plan, &st.map(|s| s.0), &st.map(|s| s.1), None);
            std::array::from_fn(|l| [r[l].xyz[0], r[l].xyz[1]])
        };
        let group = |a: &mut [AimLane; L]| {
            let q = at_stop_l(&std::array::from_fn(|l| a[l].v));
            for l in 0..L {
                a[l].q = q[l];
            }
            let mut act = [true; L];
            loop {
                let mut need = [false; L];
                for l in 0..L {
                    if act[l] {
                        match a[l].check(lim) {
                            None => act[l] = false,
                            Some(j) => need[l] = j,
                        }
                    }
                }
                if !act.iter().any(|&x| x) {
                    break;
                }
                let jac = if need.iter().any(|&x| x) {
                    let qx = at_stop_l(&std::array::from_fn(|l| [a[l].v[0] + h, a[l].v[1]]));
                    let qy = at_stop_l(&std::array::from_fn(|l| [a[l].v[0], a[l].v[1] + h]));
                    Some((qx, qy))
                } else {
                    None
                };
                for l in 0..L {
                    if act[l] {
                        a[l].update(h, jac.filter(|_| need[l]).map(|(qx, qy)| (qx[l], qy[l])));
                    }
                }
                let qn = at_stop_l(&std::array::from_fn(|l| a[l].v));
                for l in 0..L {
                    if act[l] {
                        a[l].q = qn[l];
                    }
                }
            }
        };
        // 光線は互いに独立なので、本数が多ければ並列に（組は入力の順に固定、スレッド数によらず同じビット）
        // 光線 i の初期状態 init(i) から解き、(v, 解けなかったか) を返す
        let res = |a: &AimLane| {
            (
                a.v,
                a.failed(fail_tol),
                !(a.q[0].is_finite() && a.q[1].is_finite()),
            )
        };
        let solve = |n: usize,
                     init: &(dyn Fn(usize) -> AimLane + Sync)|
         -> Vec<([f64; 2], bool, bool)> {
            let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
            if plan.is_empty() || !batch::use_for(n) {
                let f = |i: usize| res(&one(init(i)));
                if par {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(f)
                        .collect()
                } else {
                    (0..n).map(f).collect()
                }
            } else {
                // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
                let nb = n.div_ceil(L);
                let grp = |b: usize| {
                    let mut a: [AimLane; L] = std::array::from_fn(|l| init((b * L + l).min(n - 1)));
                    group(&mut a);
                    std::array::from_fn::<_, L, _>(|l| res(&a[l]))
                };
                let g: Vec<_> = if par {
                    (0..nb)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK / L)
                        .map(grp)
                        .collect()
                } else {
                    (0..nb).map(grp).collect()
                };
                let mut out = g.into_iter().flatten().collect::<Vec<_>>();
                out.truncate(n);
                out
            }
        };
        let n = px.len();
        let tgt = |q: [f64; 2]| [q[0] * rx, q[1] * ry];
        // 近軸の入射瞳から始めると光線が絞りまで届かない系（超広角など）: 主光線を視野の継続法で解き、
        // 初期値をその入射瞳上の点だけずらす（届く系はずらさない。結果は今までと同じビット）
        // 90° を超える視野（物体側の後ろから来る光）は、届いても光線を逆向きに進めた別の解のことがあるので必ず継続法で
        let behind = matches!(spec, ObjSpec::Dir(d) if d[2] < 0.0);
        let shift = if !behind && at_stop([0.0, 0.0]).iter().all(|v| v.is_finite()) {
            None
        } else {
            self.aim_chief_continuation(spec, w)
        };
        if let Some(c) = shift {
            ex.iter_mut().for_each(|v| *v += c[0]);
            ey.iter_mut().for_each(|v| *v += c[1]);
        }
        let sh = shift.unwrap_or([0.0, 0.0]);
        // 本数が多いときは、格子の光線を先に解いて q → v を多項式で当てはめ、初期値と固定の逆ヤコビアンに使う
        let fit = if n >= AIM_FIT_MIN {
            AimFit::grid(px, py).and_then(|(c, hw, pts)| {
                let epd = self.paraxial().epd;
                let init = |i: usize| {
                    let p = pts[i];
                    let v = [p[0] * epd / 2.0 + sh[0], p[1] * epd / 2.0 + sh[1]];
                    AimLane::new(tgt(p), v, None, v)
                };
                let v: Vec<_> = solve(pts.len(), &init)
                    .into_iter()
                    .map(|(v, f, nan)| (!f && !nan).then_some(v))
                    .collect();
                AimFit::new(c, hw, &pts, &v)
            })
        } else {
            None
        };
        let init = |i: usize| {
            let q = [px[i], py[i]];
            let v0 = [ex[i], ey[i]];
            if let Some(f) = &fit {
                let (v, d) = f.eval(q);
                let m = [d[0] / rx, d[1] / ry, d[2] / rx, d[3] / ry];
                if v.iter().chain(&m).all(|x| x.is_finite()) {
                    return AimLane::new(tgt(q), v, Some(m), v0);
                }
            }
            AimLane::new(tgt(q), v0, None, v0)
        };
        let mut out = solve(n, &init);
        // 解けなかった光線（絞りまで届かなかったものも）は、解けた光線のうち瞳座標の最も近いものから、目標を少しずつ
        // 動かす継続法で解き直す（近軸の初期値の光線が頂点面に平行になる折り曲げの系など）
        let bad: Vec<usize> = (0..n).filter(|&i| out[i].1 || out[i].2).collect();
        let good: Vec<usize> = (0..n).filter(|&i| !out[i].1 && !out[i].2).collect();
        if !bad.is_empty() && !good.is_empty() {
            let d2 = |i: usize, j: usize| (px[i] - px[j]).powi(2) + (py[i] - py[j]).powi(2);
            let redo = |&i: &usize| {
                let j = good
                    .iter()
                    .copied()
                    .min_by(|&a, &b| d2(i, a).total_cmp(&d2(i, b)))?;
                self.aim_follow(spec, w, out[j].0, tgt([px[j], py[j]]), tgt([px[i], py[i]]))
            };
            // 継続法は 1 本で数百〜数千回の追跡になる（瞳の外の届かない光線は刻みを最小まで縮めてから諦める）ので、
            // 少ない本数でも並列に。光線ごとに独立なので結果はスレッド数によらず同じビット
            let re: Vec<Option<[f64; 2]>> = if bad.len() > 1 && !crate::par::sequential() {
                bad.par_iter().with_min_len(1).map(redo).collect()
            } else {
                bad.iter().map(redo).collect()
            };
            for (&i, r) in bad.iter().zip(re) {
                if let Some(v) = r {
                    out[i] = (v, false, false);
                }
            }
        }
        for (i, (v, f, _)) in out.into_iter().enumerate() {
            fail[i] = f;
            (ex[i], ey[i]) = if f { (0.0, 0.0) } else { (v[0], v[1]) };
        }
    }

    /// 主光線（絞りの中心を通る光線）の入射瞳の座標を、物体（視野の角度・物点）を軸上から少しずつ動かす継続法で解く。
    /// 近軸の入射瞳から始めると絞りに届かない超広角の系の光線エイミングの初期値に使う。解けなければ None
    fn aim_chief_continuation(&self, spec: ObjSpec, w: usize) -> Option<[f64; 2]> {
        let s = self.stop();
        let fr = self.frames();
        let (o1, r0) = (fr.obj.o, fr.obj.r);
        // 無限遠物体の未知数は、面 1 の原点を通り光線に垂直な面の上の位置（入射瞳の面の座標は視野 90° で決まらなく
        // なるので、90° を超える視野へもたどれるように）。光線はその面から十分前に戻した点から出し、各面まで光線の
        // 向きに進む（逆向きに進んで交わる解を採らない）ものだけを認める
        let n_path = match spec {
            ObjSpec::Dir(d) if d[0] != 0.0 || d[1] != 0.0 => {
                let ph = d[1].atan2(d[0]);
                [-ph.sin(), ph.cos(), 0.0]
            }
            _ => [1.0, 0.0, 0.0],
        };
        let basis = |d: V3| -> (V3, V3) {
            let u1 = normalize(sub(n_path, scale(d, dot(n_path, d))));
            (u1, cross(d, u1))
        };
        let back = 1e3
            + (1..self.image())
                .map(|j| self.surfaces[j].t.abs())
                .filter(|t| t.is_finite())
                .sum::<f64>();
        let nrec = (1..=s).filter(|&j| !self.surfaces[j].ignore).count();
        let at = |sp: ObjSpec, v: [f64; 2]| -> [f64; 2] {
            let (p, k) = match sp {
                ObjSpec::Dir(d) => {
                    let (u1, u2) = basis(d);
                    let q = add(scale(u1, v[0]), scale(u2, v[1]));
                    (add(o1, mv(&r0, sub(q, scale(d, back)))), mv(&r0, d))
                }
                ObjSpec::Point(_) => self.start_ray(sp, v[0], v[1]),
            };
            let mut rec = vec![trace::RecEntry::NAN; nrec];
            let r = trace::trace_ray(self, fr, w, p, k, 0.0, 1, s, false, Some(&mut rec));
            if r.err != 0 {
                return [f64::NAN; 2];
            }
            // 光路長が面ごとに減らない（光線の向きに進む）
            let mut prev: f64 = 0.0;
            for e in &rec {
                if e.opl < prev - 1e-9 * prev.abs().max(1.0) {
                    return [f64::NAN; 2];
                }
                prev = e.opl;
            }
            [r.xyz[0], r.xyz[1]]
        };
        let (rx, ry) = self.stop_radius();
        let lim = 1e-12 * rx.abs().max(ry.abs()).max(1.0);
        let h = 1e-7 * (self.paraxial().epd / 2.0).max(1.0);
        let newton = |sp: ObjSpec, mut v: [f64; 2]| -> Option<[f64; 2]> {
            let mut q = at(sp, v);
            for _ in 0..60 {
                if !(q[0].is_finite() && q[1].is_finite()) {
                    return None;
                }
                if q[0].abs().max(q[1].abs()) <= lim {
                    return Some(v);
                }
                let qx = at(sp, [v[0] + h, v[1]]);
                let qy = at(sp, [v[0], v[1] + h]);
                let j = [
                    (qx[0] - q[0]) / h,
                    (qy[0] - q[0]) / h,
                    (qx[1] - q[1]) / h,
                    (qy[1] - q[1]) / h,
                ];
                let det = j[0] * j[3] - j[1] * j[2];
                let dv = [
                    (j[3] * q[0] - j[1] * q[1]) / det,
                    (-j[2] * q[0] + j[0] * q[1]) / det,
                ];
                if !(dv[0].is_finite() && dv[1].is_finite()) {
                    return None;
                }
                // 光線が面から外れたら刻みを半分に
                let mut t = 1.0;
                loop {
                    let vn = [v[0] - t * dv[0], v[1] - t * dv[1]];
                    let qn = at(sp, vn);
                    if qn[0].is_finite() && qn[1].is_finite() {
                        v = vn;
                        q = qn;
                        break;
                    }
                    t *= 0.5;
                    if t < 1e-6 {
                        return None;
                    }
                }
            }
            (q[0].abs().max(q[1].abs()) <= lim).then_some(v)
        };
        // 軸上（主光線は入射瞳の中心）から物体を動かす
        let step = |t: f64| -> ObjSpec {
            match spec {
                ObjSpec::Dir(d) => {
                    let th = d[2].clamp(-1.0, 1.0).acos() * t;
                    let ph = d[1].atan2(d[0]);
                    ObjSpec::Dir([th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()])
                }
                ObjSpec::Point(p) => ObjSpec::Point([p[0] * t, p[1] * t]),
            }
        };
        // 走査の範囲: 面 1 から絞りまでの面の半径の最大の 2 倍（なければ全長）
        let span = (1..=s)
            .map(|j| self.surfaces[j].semi)
            .filter(|v| v.is_finite())
            .fold(0.0f64, f64::max)
            .max(1.0)
            * 2.0;
        let scan = |sp: ObjSpec| -> Option<[f64; 2]> {
            let m = 400;
            let mut best: Option<(f64, [f64; 2])> = None;
            for i in 0..=m {
                let v = [0.0, -span + 2.0 * span * i as f64 / m as f64];
                let q = at(sp, v);
                let e = q[0].hypot(q[1]);
                if e.is_finite() && best.map_or(true, |b| e < b.0) {
                    best = Some((e, v));
                }
            }
            newton(sp, best?.1)
        };
        let mut v = [0.0, 0.0];
        let mut t: f64 = 0.0;
        let mut dt: f64 = 1.0 / 16.0;
        while t < 1.0 {
            let tn = (t + dt).min(1.0);
            match newton(step(tn), v) {
                Some(vn) => {
                    v = vn;
                    t = tn;
                    dt = (dt * 1.5).min(0.25);
                }
                None => {
                    dt *= 0.5;
                    if dt < 1e-4 {
                        // 解の枝が途切れた（視野 90° を越えると、光線が面 1 に当たる所が飛ぶ）: 少し先の視野で、
                        // 継続法の面の中の直線（v1 = 0）を走査して初期値を探し直す
                        let tn = (t + 1.0 / 64.0).min(1.0);
                        v = scan(step(tn))?;
                        t = tn;
                        dt = 1.0 / 64.0;
                    }
                }
            }
        }
        match spec {
            // 入射瞳の面（物体の座標系の z = 近軸の入射瞳の位置）での座標に直す
            ObjSpec::Dir(d) => {
                let (u1, u2) = basis(d);
                let q = add(scale(u1, v[0]), scale(u2, v[1]));
                let zp = self.paraxial().enpp.min(ENPP_MAX);
                let t = (zp - q[2]) / d[2];
                let e = [q[0] + t * d[0], q[1] + t * d[1]];
                (e[0].is_finite() && e[1].is_finite()).then_some(e)
            }
            ObjSpec::Point(_) => Some(v),
        }
    }

    /// 入射瞳の座標 v0 で絞りの点 t0 を通る光線から、目標を t1 まで少しずつ動かして（継続法、各点はニュートン法）
    /// t1 を通る光線の入射瞳の座標を解く。解けなければ None
    fn aim_follow(
        &self,
        spec: ObjSpec,
        w: usize,
        v0: [f64; 2],
        t0: [f64; 2],
        t1: [f64; 2],
    ) -> Option<[f64; 2]> {
        let s = self.stop();
        let fr = self.frames();
        let at = |v: [f64; 2], tg: [f64; 2]| -> [f64; 2] {
            let (p, k) = self.start_ray(spec, v[0], v[1]);
            let r = trace::trace_ray(self, fr, w, p, k, 0.0, 1, s, false, None);
            if r.err != 0 {
                return [f64::NAN; 2];
            }
            [r.xyz[0] - tg[0], r.xyz[1] - tg[1]]
        };
        let (rx, ry) = self.stop_radius();
        let lim = 1e-12 * rx.abs().max(ry.abs()).max(1.0);
        let h = 1e-7 * (self.paraxial().epd / 2.0).abs().max(1.0);
        let newton = |mut v: [f64; 2], tg: [f64; 2]| -> Option<[f64; 2]> {
            let mut q = at(v, tg);
            for _ in 0..60 {
                if !(q[0].is_finite() && q[1].is_finite()) {
                    return None;
                }
                if q[0].abs().max(q[1].abs()) <= lim {
                    return Some(v);
                }
                let qx = at([v[0] + h, v[1]], tg);
                let qy = at([v[0], v[1] + h], tg);
                let j = [
                    (qx[0] - q[0]) / h,
                    (qy[0] - q[0]) / h,
                    (qx[1] - q[1]) / h,
                    (qy[1] - q[1]) / h,
                ];
                let det = j[0] * j[3] - j[1] * j[2];
                let dv = [
                    (j[3] * q[0] - j[1] * q[1]) / det,
                    (-j[2] * q[0] + j[0] * q[1]) / det,
                ];
                if !(dv[0].is_finite() && dv[1].is_finite()) {
                    return None;
                }
                let mut t = 1.0;
                loop {
                    let vn = [v[0] - t * dv[0], v[1] - t * dv[1]];
                    let qn = at(vn, tg);
                    if qn[0].is_finite() && qn[1].is_finite() {
                        v = vn;
                        q = qn;
                        break;
                    }
                    t *= 0.5;
                    if t < 1e-6 {
                        return None;
                    }
                }
            }
            (q[0].abs().max(q[1].abs()) <= lim).then_some(v)
        };
        let mut v = v0;
        let mut t: f64 = 0.0;
        let mut dt: f64 = 1.0 / 8.0;
        while t < 1.0 {
            let tn = (t + dt).min(1.0);
            let tg = [t0[0] + tn * (t1[0] - t0[0]), t0[1] + tn * (t1[1] - t0[1])];
            match newton(v, tg) {
                Some(vn) => {
                    v = vn;
                    t = tn;
                    dt = (dt * 1.5).min(0.25);
                }
                None => {
                    dt *= 0.5;
                    if dt < 1e-4 {
                        return None;
                    }
                }
            }
        }
        Some(v)
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

    /// [`Self::try_trace`] で、記録する面を選ぶ（[`RecSel`]）
    pub fn try_trace_rec(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        rec: RecSel<'_>,
    ) -> Result<Trace> {
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = match field {
            FieldSel::Index(i) => self.hxhy(i),
            FieldSel::Norm(a, b) => (a, b),
        };
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        let l = Launch::Pupil {
            spec,
            signed: false,
            px,
            py,
            vig: Some(vig),
            hxy: Some((hx, hy)),
        };
        let mut t = self.trace_launch_rec(&l, w, last, rec);
        t.hx_hy = (hx, hy);
        Ok(t)
    }

    /// 追跡の前に、未対応の項目がないかを確かめる（理想レンズの媒質など）
    pub fn validate(&self) -> Result<()> {
        for (j, d) in self.surfaces.iter().enumerate() {
            if d.typ == SurfType::User && !d.ignore {
                if d.user.is_none() {
                    invalid!("利用者定義の面（面 {j}）にモデル（user）がない");
                }
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), d.n[w]);
                if nb != 1.0 || na != 1.0 || d.mirror {
                    notimpl!("空気以外の中・鏡の利用者定義の面（面 {j}: {nb} → {na}）");
                }
            }
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

/// 座標系を 1 面ぶん進める（[`frames`] と同じ）。面の座標系と、次の面へ渡す座標系を返す
pub fn step_frame(o: V3, r: M3, d: &Surface) -> ((V3, M3), (V3, M3)) {
    if d.ignore {
        return ((o, r), (o, r));
    }
    let (mut o, mut r) = (o, r);
    let frame;
    if d.typ == SurfType::CoordBrk {
        (o, r) = apply_cb(
            o,
            r,
            &Tilt {
                d: d.cb,
                order: d.cb_order,
            },
        );
        frame = (o, r);
    } else {
        let (o_pre, r_pre) = (o, r);
        if let Some(b) = &d.before {
            (o, r) = apply_cb(o, r, b);
        }
        frame = (o, r);
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
    (frame, (o, r))
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

/// 面ごとの座標系（大域座標 = 面 glrs の座標系）。玉・群の剛体の動き（[`crate::units`]）も入れる
pub fn frames(s: &System) -> Frames {
    let glrs = s.glrs;
    let raw = crate::units::raw_frames(s);
    let mut out = raw.surf;
    if s.units.iter().any(|u| u.moved()) {
        crate::units::apply_frames(&s.units, &raw.entry, &mut out);
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
        fres_ref: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 理想レンズ f 50（絞り）の後ろに、近軸像面から外した像面を置いた系。視野は近軸像高 (0, 5)
    fn defocused(obj: &str, aperture: &str, image_t: f64) -> System {
        let t = format!(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\n{aperture}\nFTYP 2 0 1 1 0 0 0\n\
             XFLN 0\nYFLN 5\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
             SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ {obj}\n\
             SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ {image_t}\n\
             SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        let raw = crate::zmx::parse_text(&t).unwrap();
        crate::zmx::build(&raw, &Default::default(), &[], None).unwrap()
    }

    #[test]
    fn paraxial_image_height_is_taken_at_the_paraxial_image_plane() {
        // 無限遠の物体: 像面がどこでも、近軸像面での高さ 5 は tanθ = 5/50
        for t in [50.0, 40.0, 60.0] {
            let s = defocused("INFINITY", "ENPD 10", t);
            match s.object_spec(0.0, 1.0).unwrap() {
                ObjSpec::Dir(d) => assert!(((d[1] / d[2]).abs() - 0.1).abs() < 1e-14, "{t}: {d:?}"),
                o => panic!("{o:?}"),
            }
        }
        // 有限の物体（距離 100、倍率 −1）: 物体高 5
        for t in [100.0, 80.0, 120.0] {
            let s = defocused("100", "OBNA 0.05 0", t);
            match s.object_spec(0.0, 1.0).unwrap() {
                ObjSpec::Point(p) => assert!((p[1].abs() - 5.0).abs() < 1e-12, "{t}: {p:?}"),
                o => panic!("{o:?}"),
            }
        }
    }
}
