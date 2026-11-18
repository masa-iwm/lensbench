//! 系: 面と系のデータ（波長、視野、絞り、光線エイミング）、近軸量、光線の生成。
//!
//! 流儀は OpticStudio に合わせる（レンズ単位 mm、波長 µm、角度 deg）。
//! - 近軸量: 主波長の y-nu 追跡（展開した軸に沿う。傾き・偏心・座標ブレークは入らず、鏡で向きが反転）
//! - 絞り: 入射瞳径、像側 F 値、物体側 NA、絞り径、近軸作動 F 値、物体側コーン角、物体側テレセントリック
//! - 視野: 角度（方向正接）、物体高、近軸像高、実像高。正規化は半径か矩形。定義済み視野のケラレ係数
//! - 瞳座標: エイミングなしは近軸入射瞳面、エイミングありは絞り面の (px, py)×絞り半径
//! - 無限遠物体の光線は面 1 の頂点を通り視野方向に垂直な面から出る（光路長はそこから数える）

use crate::aim::{aim_lanes, AimFit, AimSol, AimStart};
use crate::batch;
use crate::coating::{self, Coatings, C64};
use crate::error::{Error, Result};
use crate::glass::Glass;
use crate::interp2d::interp_weights;
use crate::math::*;
use crate::surface::{After, Bire, Material, Surface};
use crate::trace::{self, CryKind, PolKind, PolSurf, RayOut, RayStatus, RecEntry, CV3};
use crate::LockExt;
use crate::{invalid, notimpl, par::scoped};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const ENPP_MAX: f64 = 1e10;

#[derive(Debug, Default)]
struct Cache {
    frames: OnceLock<Frames>,
    par: OnceLock<Paraxial>,
    expp: OnceLock<f64>,
    expd: OnceLock<f64>,
    rs: OnceLock<(f64, f64)>,
    /// 実像高・条件で定める視野の解（鍵は条件・解く波長・目標値。field_cond.rs）
    realh: Mutex<HashMap<(FieldCond, usize, u64, u64), ObjSpec>>,
    /// 理想レンズの物体距離（面ごと。追跡の内側で読むのでロックなし）
    conj: OnceLock<Vec<f64>>,
    /// 面ごとの交点・法線の求め方（追跡の内側の分岐を前計算）。面の形で決まるので、[`Keep`] でも引き継がない
    kinds: OnceLock<Vec<crate::shape::SurfKind>>,
    /// 面ごとの空間の進む向き（[`crate::trace::space_signs`]、追跡の内側で光線ごとに作らないため）。面の並びで決まる
    signs: OnceLock<Vec<f64>>,
    /// 追跡の面ごとの準備（[`crate::batch::Plan`]。鍵は (波長, 最後の面, Floating 開口でけるか)。系の座標系で追う分だけ）
    plans: crate::batch::PlanCache,
    /// 像面だけをずらした系の追跡の共有の表（[`crate::focus`]。changed() で外れる）
    pub(crate) focus: Option<crate::focus::FocusShift>,
    /// 瞳の格子の光線と OPD（[`crate::pupil::PupilFunction::trace`]）
    pupils: crate::pupil::PupilCache,
}

/// これ以上の本数で並列に追う
const PAR_MIN_RAYS: usize = 512;
/// 並列のときの 1 タスクあたりの最少本数
const PAR_CHUNK: usize = 256;
/// 実光線エイミングで当てはめの初期値を使う本数（格子の光線を解く分より得になる目安）
const AIM_FIT_MIN: usize = 1024;
/// 頂点の尖った面がある系の瞳を求める主光線の視野 [°]（[`System::cusp_pupil`]）
const CUSP_FIELD_DEG: f64 = 0.01;

/// 系の文書: 利用者が書く値（面・波長・視野・開口・ピックアップ・ソルブ・多重構成など）。
/// [`System`] はこれを [`Arc`] で持ち、書き換えは [`System::edit`] を通す（直した後に座標系・近軸量などを作り直す）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Prescription {
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
    // 系で使えるガラス（読んだカタログ全体）。系の写し（with_params など）で共有し、変えるときは Arc::make_mut で写す
    pub glasses: Arc<HashMap<String, Glass>>,
    pub temperature: f64,
    pub pressure: f64,
    pub glrs: usize,
    pub ropd: i64,
    /// 瞳のアポダイゼーション（.zmx の GFAC。PSF・MTF の瞳関数の振幅と、スポット・重心・RMS 波面・幾何 MTF・最適化の
    /// RMS の項の光線の重み（強度）。Zernike の当てはめには入れない。[`crate::pupil::Apodization`]）
    #[serde(default, skip_serializing_if = "crate::pupil::Apodization::is_uniform")]
    pub apodization: crate::pupil::Apodization,
    /// 像空間がアフォーカル（.zmx の FTYP の 7 番目）。波面収差・Zernike・OPD を、射出瞳の位置で主光線に
    /// 垂直な平面（平面波）を基準に測る。スポット・MTF・PSF は角度（[`System::image_point`]、単位は
    /// [`System::afocal_unit`]）で出す
    #[serde(default)]
    pub afocal: bool,
    /// アフォーカル像空間の角度の単位（.zmx の UNIT の値の 6 番目）
    #[serde(default)]
    pub afocal_unit: AfocalUnit,
    /// 定義した視野以外の点のケラレ係数の求め方（既定は連続、OpticStudio と照合するときは OpticStudio）
    #[serde(default)]
    pub vig_interp: VigInterp,
    /// 浮動開口（半直径を固定した面・.zmx の FLAP）の外の光線をけるか（既定はける。OpticStudio は半直径を固定した面を
    /// 浮動開口にしてける）。光線エイミングの追跡は開口を見ない
    #[serde(
        default = "crate::configs::yes",
        skip_serializing_if = "crate::configs::is_yes"
    )]
    pub floating_clip: bool,
    /// 実像高・条件で定める視野を解く波長（None で主波長）。単色の RMS の掃引で OpticStudio に合わせるとき
    /// だけ使う（[`crate::sweep::RmsOpts::field_wave`]）
    #[serde(skip)]
    pub field_wave: Option<usize>,
    pub coating_file: Option<PathBuf>,
    /// 膜ファイルの外で定めた TABLE 膜（名前 → 表。プロジェクト・カタログのフィルタの透過特性）。
    /// 膜ファイルの同じ名前より優先する
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub coat_tables: HashMap<String, crate::coating::Table>,
    pub name: String,
    pub adjust_index: bool,
    /// ピックアップ（.zmx の PZUP・PPAR・PKUP・曲率のピックアップ）。パラメータを変えると解決し直す（[`crate::param`]）
    pub pickups: Vec<crate::pickup::Pickup>,
    /// 厚さのソルブ（.zmx の MAZH）。パラメータを変えると解き直す（[`crate::param`]）
    pub solves: Vec<crate::solves::Solve>,
    /// ソルブが呼ぶ ZPL のマクロ（[`crate::solves::SolveKind::MacroCurv`]・`MacroThick` の番号で引く）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<crate::zpl_solve::Macro>,
    /// 面タイプのパラメータの ZPL マクロのソルブ
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_parms: Vec<crate::zpl_solve::MacroParm>,
    /// マクロのソルブを外して保存された値で固定した理由（マクロがない・対応していない命令など）と、
    /// 解き直しでマクロがエラーになって値を変えなかったこと（[`System::macro_warnings`]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_notes: Vec<String>,
    /// ファイルから読んだときの注意（.zmx の読まずに飛ばした行・既定と違う設定など）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_notes: Vec<String>,
    /// 玉・群（[`crate::units`]）。剛体の動きは座標系にかける
    #[serde(default)]
    pub units: Vec<crate::units::Unit>,
    /// 機構部品（[`crate::mech`]）。光線の追跡には効かず、ケラレの確認と描画だけに使う
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanics: Vec<crate::mech::MechPart>,
    /// 多重構成の表（[`crate::edit`]）。None なら構成は 1 つ
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configs: Option<crate::configs::Configs>,
}

/// 系: 文書（[`Prescription`]、[`Arc`] で共有）と、そこから作る追跡の準備（座標系・近軸量・追跡の表などのキャッシュ）。
/// 欄は書き換えられない（読むのは `Deref` で文書の欄、直すのは [`System::edit`]）。複製は文書を共有するので軽い
#[derive(Debug)]
pub struct System {
    rx: Arc<Prescription>,
    /// 文書から求めた媒質の屈折率（[`crate::media::Media`]）
    media: Arc<crate::media::Media>,
    cache: Cache,
    /// まとめた編集（[`System::batch`]）の中か
    pub(crate) batch: Batch,
}

/// まとめた編集（[`System::batch`]）の中か
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Batch {
    /// 外: 編集のたびに複製し、自動の半直径も求め直す
    #[default]
    Off,
    /// 中: 編集は系をそのまま直し（失敗した系は batch が捨てる）、自動の半直径は追跡に効かなければ最後に 1 回
    On,
    /// 中で、自動の半直径の求め直しが残っている
    SemiPending,
}

impl std::ops::Deref for System {
    type Target = Prescription;
    fn deref(&self) -> &Prescription {
        &self.rx
    }
}

impl Clone for System {
    fn clone(&self) -> Self {
        self.clone_keep(Keep::default())
    }
}

impl Prescription {
    /// 面を番号か名前（コメント、前後の空白と大文字小文字は区別しない）で探す。名前が複数の面にあれば不正
    pub fn find_surface(&self, target: &str) -> Result<usize> {
        let t = target.trim();
        if let Ok(j) = t.parse::<usize>() {
            if j >= self.surfaces.len() {
                let last = self.surfaces.len() - 1;
                invalid!("surface {j} (surfaces are 0 to {last})");
            }
            return Ok(j);
        }
        let hits: Vec<usize> = (0..self.surfaces.len())
            .filter(|&j| self.surfaces[j].comment.trim().eq_ignore_ascii_case(t))
            .collect();
        match hits.as_slice() {
            [j] => Ok(*j),
            [] => invalid!("no surface named {t}"),
            _ => invalid!("several surfaces are named {t} ({hits:?}); use the number"),
        }
    }

    /// 分散式の有効範囲（AGF の LD 行）の外の波長で屈折率を求めている材料の警告（面の番号・材料・波長・範囲）。
    /// lensbench は式をそのまま延ばして計算する（OpticStudio は範囲外ではエラーにして屈折率を 1.0 にする）
    pub fn dispersion_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (k, d) in self.surfaces.iter().enumerate() {
            if d.ignore {
                continue;
            }
            let Some(g) = d.mat.glass().and_then(|m| self.glasses.get(m)) else {
                continue;
            };
            let Some([lo, hi]) = g.ld else {
                continue;
            };
            for &l in &self.wavelengths {
                if (l < lo || l > hi) && seen.insert((d.mat.name(), l.to_bits())) {
                    out.push(format!(
                        "surface {k} material {} ({}): wavelength {l} µm is outside the dispersion formula range {lo} to {hi} µm (the formula is extended)",
                        d.mat, g.catalog
                    ));
                }
            }
        }
        out
    }

    pub fn stop(&self) -> usize {
        self.surfaces.iter().position(|d| d.stop).unwrap_or(0)
    }

    /// エラーに添える面の名札: 「面 j(種類, 材料, 膜 名前, 「コメント」)」。空の項は省く
    pub fn surf_label(&self, j: usize) -> String {
        let Some(d) = self.surfaces.get(j) else {
            return format!("surface {j}");
        };
        let mut what = crate::edit::surf_type_name(d.typ()).to_string();
        if !matches!(d.mat, Material::Air | Material::Inherit) {
            what = format!("{what}, {}", d.mat);
        }
        if !d.coat.is_empty() {
            what = format!("{what}, coating {}", d.coat);
        }
        if !d.comment.is_empty() {
            what = format!("{what}, 「{}」", d.comment);
        }
        format!("surface {j}({what})")
    }

    pub fn image(&self) -> usize {
        self.surfaces.len() - 1
    }

    pub fn infinite(&self) -> bool {
        !self.surfaces[0].t.is_finite()
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

    /// 系を作る: 面の値の確かめ、材料からの屈折率、ピックアップの解決、厚さのソルブ、系の確かめ、自動の半直径
    /// （[`System::edit`] と同じ後始末）
    pub fn build(self) -> Result<System> {
        let mut s = System::from_rx(self)?;
        s.refresh()?;
        Ok(s)
    }

    /// 値をそのまま置いて系を作る（ピックアップ・ソルブ・屈折率・自動の半直径は求め直さない）。系は確かめる
    pub fn build_raw(self) -> Result<System> {
        let s = System::from_rx(self)?;
        s.check()?;
        Ok(s)
    }
}

impl System {
    /// 確かめずに作る（読み込みの途中・確かめた系の写しから）
    pub(crate) fn from_rx(rx: Prescription) -> Result<System> {
        let media = Arc::new(crate::media::resolve(&rx)?);
        Ok(System {
            rx: Arc::new(rx),
            media,
            cache: Cache::default(),
            batch: Batch::Off,
        })
    }

    /// 媒質の屈折率を文書から作り直す（材料・波長・温度・dn・disp などを変えた後）
    pub(crate) fn resolve_media(&mut self) -> Result<()> {
        self.media = Arc::new(crate::media::resolve(&self.rx)?);
        Ok(())
    }

    /// 面 j の後の媒質の屈折率（波長ごと、dn・disp 込み。鏡・座標ブレークは前の媒質）
    pub fn n(&self, j: usize) -> &[f64] {
        &self.media.n[j]
    }

    /// 屈折率のずれ（dn・disp）をかける前の媒質の屈折率（面 j の後、波長 w）
    pub fn base_index(&self, j: usize, w: usize) -> f64 {
        self.media.base[j][w]
    }

    /// 面 j の直前の媒質の屈折率
    pub fn index_before(&self, j: usize, w: usize) -> f64 {
        let mut n = self.media.n[0][w];
        for (k, d) in self.surfaces.iter().enumerate().take(j.max(1)).skip(1) {
            if !d.ignore && !d.is_coord_break() {
                n = self.media.n[k][w];
            }
        }
        n
    }

    pub fn image_index(&self, w: usize) -> f64 {
        self.index_before(self.image(), w)
    }
}

impl serde::Serialize for System {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.rx.serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for System {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<System, D::Error> {
        let p = Prescription::deserialize(d)?;
        System::from_rx(p).map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for System {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        Prescription::schema_name()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Prescription::json_schema(g)
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
            rx: self.rx.clone(),
            media: self.media.clone(),
            cache,
            batch: self.batch,
        }
    }

    /// 文書を書き換える（共有していれば写す）。キャッシュは捨てないので、書き換えた後に
    /// `changed` か `refresh` を呼ぶ（[`Keep`] で残す分を選んだときを除く）
    pub(crate) fn rx_mut(&mut self) -> &mut Prescription {
        Arc::make_mut(&mut self.rx)
    }

    /// 文書
    pub fn prescription(&self) -> &Prescription {
        &self.rx
    }

    /// 文書を取り出す（共有していなければ写さない）
    pub fn into_prescription(self) -> Prescription {
        Arc::try_unwrap(self.rx).unwrap_or_else(|a| (*a).clone())
    }
}

/// 視野が軸上だけの系で、射出瞳を求める補助の主光線に使う最大視野の代わりの値（視野の単位）
const ON_AXIS_PUPIL_FIELD: f64 = 10.0;

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
        glasses: impl Into<Arc<HashMap<String, Glass>>>,
    ) -> Result<System> {
        let s = Prescription {
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
            glasses: glasses.into(),
            temperature: 20.0,
            pressure: 1.0,
            glrs: 1,
            ropd: 2,
            apodization: crate::pupil::Apodization::Uniform,
            afocal: false,
            afocal_unit: AfocalUnit::Milliradians,
            vig_interp: VigInterp::Continuous,
            floating_clip: true,
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
            import_notes: Vec::new(),
            units: Vec::new(),
            mechanics: Vec::new(),
            configs: None,
        }
        .build_raw()?;
        Ok(s)
    }

    pub fn check(&self) -> Result<()> {
        let sf = &self.surfaces;
        if sf.len() < 3 {
            invalid!("too few surfaces ({})", sf.len());
        }
        if self.telecentric && !sf[0].t.is_finite() {
            notimpl!("object-space telecentric with an object at infinity");
        }
        if self.telecentric && self.aiming != Aiming::Off {
            notimpl!("ray aiming with object-space telecentric");
        }
        if !sf[0].t.is_finite() && self.field_type == FieldType::ObjH {
            invalid!("object height field with an object at infinity");
        }
        if self.telecentric && !matches!(self.aperture.0, ApertureKind::Na | ApertureKind::Cone) {
            invalid!("object-space telecentric needs an object-space NA or cone angle aperture");
        }
        // Jones の面は頂点の平面だけ（曲率の残った面の交点・法線は決まらない）
        if let Some(j) = sf
            .iter()
            .position(|d| d.typ() == crate::surface::SurfType::Jones && d.c != 0.0)
        {
            notimpl!("Jones matrix surface {j} on a curved surface");
        }
        let st = self.stop();
        if st == 0 || st >= sf.len() - 1 {
            invalid!("stop surface {st}");
        }
        // GRIN の媒質の次の面（座標ブレークは通して、その次）は交点を求められる面タイプに限る
        for (j, d) in sf.iter().enumerate() {
            if d.ignore || d.is_mirror() || d.grin().is_none() {
                continue;
            }
            if let Some((k, e)) = sf
                .iter()
                .enumerate()
                .skip(j + 1)
                .find(|(_, e)| !e.ignore && !e.is_coord_break())
            {
                if !crate::grin::next_ok(e) {
                    let typ = e.typ();
                    notimpl!("surface {k} ({typ:?}) after the GRIN medium of surface {j}");
                }
            }
        }
        Ok(())
    }

    /// キャッシュを捨てる（系のデータの確かめはしない）
    pub(crate) fn invalidate_frames(&mut self) {
        self.cache = Cache::default();
    }

    /// 像面だけをずらした系の追跡に共有の表を使う（[`crate::sweep::defocus_shared`]）
    pub(crate) fn set_focus(&mut self, f: crate::focus::FocusShift) {
        self.cache.focus = Some(f);
        self.cache.pupils.clear();
    }

    /// 瞳の格子の光線と OPD のしまい場所
    pub(crate) fn pupil_cache(&self) -> &crate::pupil::PupilCache {
        &self.cache.pupils
    }

    /// 文書を書き換えたあとに呼ぶ（座標系・近軸量などのキャッシュを捨てて系を確かめる）
    pub(crate) fn changed(&mut self) -> Result<()> {
        self.cache = Cache::default();
        self.resolve_media()?;
        self.check()
    }

    /// 面ごとの交点・法線の求め方（[`crate::shape::surf_kind`]）
    pub(crate) fn surf_kinds(&self) -> &[crate::shape::SurfKind] {
        self.cache
            .kinds
            .get_or_init(|| self.surfaces.iter().map(crate::shape::surf_kind).collect())
    }

    /// 追跡の面ごとの準備のキャッシュ（[`crate::batch::Plan::new`]）
    pub(crate) fn plan_cache(&self) -> &crate::batch::PlanCache {
        &self.cache.plans
    }

    pub(crate) fn space_signs(&self) -> &[f64] {
        self.cache
            .signs
            .get_or_init(|| crate::trace::space_signs(self))
    }

    pub fn frames(&self) -> &Frames {
        self.cache.frames.get_or_init(|| frames(self))
    }

    // ---------------------------------------------------------------------------------- 近軸

    /// 追跡の前に、未対応の項目がないかを確かめる（理想レンズの媒質など）
    pub fn validate(&self) -> Result<()> {
        for (j, d) in self.surfaces.iter().enumerate() {
            let action = d.action();
            if matches!(action, crate::action::Action::User(_)) && !d.ignore {
                if d.user().is_none() {
                    invalid!("user-defined surface (surface {j}) has no model (user)");
                }
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), self.n(j)[w]);
                if nb != 1.0 || na != 1.0 || d.is_mirror() {
                    notimpl!("user-defined surface in a medium other than air or on a mirror (surface {j}: {nb} → {na})");
                }
            }
            if let (crate::action::Action::Paraxial { .. }, false) = (action, d.ignore) {
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), self.n(j)[w]);
                if nb != 1.0 || na != 1.0 {
                    notimpl!("paraxial lens in a medium other than air (surface {j}: {nb} → {na})");
                }
                // 物体を無限遠に結像する（コリメータ）は断らない: 理想レンズの位相は出る光を平行光として求め
                // （trace::paraxial）、OPD の基準は実レンズのコリメータと同じ RefSurface::of で決まる
            }
        }
        if self.surfaces[0].before.is_some() || self.surfaces[0].after != After::None {
            notimpl!("tilt or decenter of the object surface");
        }
        Ok(())
    }
}

mod aiming;
mod field;
mod launch;
mod near;
mod paraxial;
mod pol;
pub use pol::PolIn;
mod raybuf;
mod rays;
mod types;
use launch::StartGeom;
use near::NearGrid;
pub use raybuf::{Cols, RayBuf};
use raybuf::{Done, Dst};
pub use rays::*;
pub use types::*;

mod frame_calc;
pub use frame_calc::{apply_cb, frames, step_frame};

mod field_cond;

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
