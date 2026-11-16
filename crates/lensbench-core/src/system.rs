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
use crate::interp2d::interp_weights;
use crate::math::*;
use crate::surface::{After, Bire, SurfType, Surface};
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
    kinds: OnceLock<Vec<crate::trace::SurfKind>>,
    /// 面ごとの空間の進む向き（[`crate::trace::space_signs`]、追跡の内側で光線ごとに作らないため）。面の並びで決まる
    signs: OnceLock<Vec<f64>>,
    /// 追跡の面ごとの準備（[`crate::batch::Plan`]。鍵は (波長, 最後の面, Floating 開口でけるか)。系の座標系で追う分だけ）
    plans: crate::batch::PlanCache,
    /// 像面だけをずらした系の追跡の共有の表（[`crate::focus`]。changed() で外れる）
    pub(crate) focus: Option<crate::focus::FocusShift>,
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
    // 系で使えるガラス（読んだカタログ全体）。系の写し（with_params など）で共有し、変えるときは Arc::make_mut で写す
    pub glasses: Arc<HashMap<String, Glass>>,
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
    /// 定義した視野以外の点のケラレ係数の求め方（既定は連続、OpticStudio と照合するときは OpticStudio）
    #[cfg_attr(feature = "serde", serde(default))]
    pub vig_interp: VigInterp,
    /// 浮動開口（半直径を固定した面・.zmx の FLAP）の外の光線をけるか（既定はける。OpticStudio は半直径を固定した面を
    /// 浮動開口にしてける）。光線エイミングの追跡は開口を見ない
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::edit::yes",
            skip_serializing_if = "crate::edit::is_yes"
        )
    )]
    pub floating_clip: bool,
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
    /// ファイルから読んだときの注意（.zmx の読まずに飛ばした行・既定と違う設定など）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub import_notes: Vec<String>,
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
            vig_interp: self.vig_interp,
            floating_clip: self.floating_clip,
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
            import_notes: self.import_notes.clone(),
            units: self.units.clone(),
            mechanics: self.mechanics.clone(),
            configs: self.configs.clone(),
            cache,
        }
    }
}

/// 視野が軸上だけの系で、射出瞳を求める補助の主光線に使う最大視野の代わりの値（視野の単位）
const ON_AXIS_PUPIL_FIELD: f64 = 10.0;

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
                        "surface {k} material {} ({}): wavelength {l} µm is outside the dispersion formula range {lo} to {hi} µm (the formula is extended)",
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
        glasses: impl Into<Arc<HashMap<String, Glass>>>,
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
            glasses: glasses.into(),
            temperature: 20.0,
            pressure: 1.0,
            glrs: 1,
            ropd: 2,
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
            cache: Cache::default(),
        };
        s.check()?;
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
        let st = self.stop();
        if st == 0 || st >= sf.len() - 1 {
            invalid!("stop surface {st}");
        }
        // 非球面の項を使わない面タイプに項が残っていれば誤り（交点・法線・近軸で解釈が食い違う）
        for (j, d) in sf.iter().enumerate() {
            if matches!(
                d.typ,
                SurfType::Standard | SurfType::Gradium | SurfType::Gradient9
            ) && d.has_asph()
            {
                let typ = d.typ;
                invalid!("surface {j} ({typ:?}) has asphere coefficients (use a type such as even asphere, or remove them)");
            }
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
                    let typ = e.typ;
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
            return format!("surface {j}");
        };
        let mut what = crate::edit::surf_type_name(d.typ).to_string();
        if !d.mat.is_empty() && d.mat != "-" {
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

    /// 面ごとの交点・法線の求め方（[`crate::trace::surf_kind`]）
    pub(crate) fn surf_kinds(&self) -> &[crate::trace::SurfKind] {
        self.cache
            .kinds
            .get_or_init(|| self.surfaces.iter().map(crate::trace::surf_kind).collect())
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

    /// 追跡の前に、未対応の項目がないかを確かめる（理想レンズの媒質など）
    pub fn validate(&self) -> Result<()> {
        for (j, d) in self.surfaces.iter().enumerate() {
            if d.typ == SurfType::User && !d.ignore {
                if d.user.is_none() {
                    invalid!("user-defined surface (surface {j}) has no model (user)");
                }
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), d.n[w]);
                if nb != 1.0 || na != 1.0 || d.mirror {
                    notimpl!("user-defined surface in a medium other than air or on a mirror (surface {j}: {nb} → {na})");
                }
            }
            if d.typ == SurfType::Paraxial && !d.ignore {
                let w = self.primary;
                let (nb, na) = (self.index_before(j, w), d.n[w]);
                if nb != 1.0 || na != 1.0 {
                    notimpl!("paraxial lens in a medium other than air (surface {j}: {nb} → {na})");
                }
                let z = self.lens_conjugate(j);
                if z.is_finite() && z.abs() <= 1e12 && 1.0 / d.focal + 1.0 / z == 0.0 {
                    notimpl!("the paraxial lens images the object to infinity (OPD)");
                }
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
