//! 系を作り直さずにパラメータを変える。
//!
//! 面は番号か名前（コメント）で選ぶ。変えたパラメータが影響しないキャッシュ（面の座標系・近軸量）は残す。
//! 変えた後の系は、同じ値で .zmx から読み直した系と同じデータになり、追跡はビット一致する
//! （屈折率のずれ dn・disp は .zmx に書けないので除く）。
//!
//! パラメータの意味:
//! - `c`（曲率）・`r`（曲率半径、0 は平面）・`k`（円錐係数）・`a2`〜`a16`（偶数次非球面の r^2〜r^16 の係数）
//! - `t`: 次の面までの厚さ（面 0 は物体距離）
//! - `dx` `dy` `tx` `ty` `tz`: 座標ブレークならその値。ほかの面では面の前の傾き偏心（OpticStudio の
//!   Tilt/Decenter の「前」）。前の傾き偏心がない面に与えると、面の後は「前を戻す」になる（その面だけが動く）。
//!   既にある面では面の後の扱いを変えない
//! - `dn`: 面の後の媒質の屈折率に全波長で足すずれ
//! - `disp`: 同じ媒質の分散の相対変化。n = n0(λ) + dn + disp·(n0(λ) − n0(主波長))
//!
//! 媒質は鏡・座標ブレークを越えて続くので、dn・disp はその後に続く鏡・座標ブレークの面にも反映する。
//! 物点の移動（`objx` `objy`）は系のデータではなく追跡の入力なので、ここでは変えない（微分 [`crate::fermat`] で使う）。
//!
//! ピックアップ（[`crate::pickup::Pickup`]、.zmx の曲率・厚さ・円錐係数・パラメータのピックアップ）は OpticStudio と同じく追従する:
//! 元の面のパラメータを変えると、ピックアップ先も「倍率 × 元の値 + オフセット」に解決し直す。
//! ピックアップ先のパラメータそのものは変えられない（エラー。元の面を変える）。
//! 玉の並びを組み直した系（[`crate::elements::assemble`]）はピックアップを引き継がない。
//!
//! 玉・群の単位のパラメータ（[`crate::units`]）も同じ形で指す: `lens2.tx`（玉モードの 2 番目の玉の傾き）、
//! `G1.shift`（群 G1 の移動）。[`Param::surf`] は玉・群の番号（[`System::units`]）、種類は [`Kind::Unit`]。
//! 名前が面の名前（コメント）と玉・群の名前の両方にあるときは、`unit:名前.種類` `surf:名前.種類` で区別する。

use crate::error::Result;
use crate::pickup::pick_kind;
use crate::pupil::Apodization;
use crate::solves::solve_kind_info;
use crate::surface::{After, Aperture, Tilt};
use crate::system::{Aiming, ApertureKind, Keep, System};
use crate::units::UnitKind;
use crate::{invalid, notimpl};

/// パラメータの種類
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub enum Kind {
    /// 曲率 c
    Curv,
    /// 曲率半径 R = 1/c（0 は平面）
    Radius,
    /// 円錐係数
    Conic,
    /// 偶数次非球面の係数（0 が r^2、7 が r^16）
    Asph(u8),
    /// Zernike Sag の面の項 j の係数（1 から）
    Zern(u16),
    /// 次の面までの厚さ
    Thick,
    Dx,
    Dy,
    /// 傾き [deg]
    Tx,
    Ty,
    Tz,
    /// 媒質の屈折率のずれ
    Dn,
    /// 媒質の分散の相対変化
    Disp,
    /// 物点の x・y（有限物体。面の番号は使わない）
    ObjX,
    ObjY,
    /// 系の波長 i（0 始まり、名前は wave1 から）[µm]。面の番号は使わない。変えると全媒質の屈折率がその波長の値になる
    /// （光源の波長のずれの公差・多重構成の行（.zmx の WAVE）用。最適化の変数には使えない）
    Wave(u8),
    /// 系の波長 i の重み（0 始まり、名前は wwt1 から）。多重構成の行（.zmx の WLWT）。最適化の変数には使えない
    WaveW(u8),
    /// 主波長の番号（1 から。値は整数）。多重構成の行（.zmx の PRWV）。最適化の変数には使えない
    Primary,
    /// 開口の値（開口の種類の単位: 入射瞳径 mm・F 値・NA など）。多重構成の行（.zmx の APER）。最適化の変数には使えない
    Aper,
    /// 開口の種類（OpticStudio の番号 0〜5: 入射瞳径・像側 F 値・物体側 NA・絞り径・近軸作動 F 値・物体側の開き角）。
    /// 多重構成の行（.zmx の SATP）。最適化の変数には使えない
    ApertureType,
    /// 絞りの面の番号（値は整数）。多重構成の行（.zmx の STPS）。最適化の変数には使えない
    Stop,
    /// 視野の種類（OpticStudio の番号 0〜3: 角度・物体高・近軸像高・実像高）。多重構成の行（.zmx の FLTP）。
    /// 最適化の変数には使えない
    FieldType,
    /// 系の温度 [°C]・気圧 [atm]（変えると全媒質の屈折率を作り直す）。多重構成の行（.zmx の TEMP・PRES）。最適化の変数には使えない
    Temp,
    Pres,
    /// アフォーカルの像空間か（1 でアフォーカル）。多重構成の行（.zmx の AFOC）。最適化の変数には使えない
    Afocal,
    /// アポダイゼーションの種類（OpticStudio の番号 0〜2: 一様・ガウス・cos³）と、ガウスの係数。多重構成の行
    /// （.zmx の APDT・APDF）。最適化の変数には使えない
    ApodType,
    Apod,
    /// 光線エイミング（0〜2: なし・近軸・実光線）。多重構成の行（.zmx の RAAM）。最適化の変数には使えない
    Aiming,
    /// OPD の基準（OpticStudio の番号 0〜3、.zmx の ROPD と同じ）。多重構成の行（.zmx の OPDR）。最適化の変数には使えない
    Ropd,
    /// 視野 i の x・y・重み（0 始まり、名前は fx1・fy1・fwt1 から）。多重構成の行（.zmx の XFIE・YFIE・FLWT）。
    /// 最適化の変数には使えない
    FieldX(u8),
    FieldY(u8),
    FieldW(u8),
    /// 玉・群のパラメータ（付く先は [`Target::Unit`]）
    Unit(UnitKind),
    /// 半直径（semi-diameter）。ピックアップ・マクロのソルブの量と、多重構成の行（.zmx の SDIA。固定の半直径にし、
    /// 浮動開口の面はその半直径でける）。最適化の変数には使えない
    Semi,
    /// 面を無視するか（1 で無視、0 で使う）。多重構成の行（.zmx の IGNR）。最適化の変数には使えない
    Ignore,
    /// 円形の開口・遮蔽の最大の半径。多重構成の行（.zmx の APMX）。最適化の変数には使えない
    ApMax,
    /// 円形の開口・遮蔽の最小の半径。多重構成の行（.zmx の APMN）。最適化の変数には使えない
    ApMin,
    /// 開口・遮蔽の偏心 x・y。多重構成の行（.zmx の APDX・APDY）。最適化の変数には使えない
    ApDx,
    ApDy,
    /// 開口の種類（OpticStudio の番号: 0 なし・1 円形の開口・2 円形の遮蔽・3 スパイダー・4 矩形・6 楕円・10 浮動。
    /// 変えられるのは 0〜2 の間で、半径は今の円形の開口・遮蔽の値）。多重構成の行（.zmx の APTP）。最適化の変数には使えない
    ApType,
    /// 面の前・後の傾き偏心の順序（0 偏心から、1 傾きから）。多重構成の行（.zmx の CBOR・CAOR）。最適化の変数には使えない
    BeforeOrder,
    AfterOrder,
    /// 面の材料。多重構成の行（.zmx の GLSS）だけで、値は行の材料の名前の番号（[`crate::configs::ConfigRow::names`]）。
    /// 最適化の変数には使えない
    Material,
    /// 面タイプのパラメータ（.zmx の PARM の番号、1 から）。ピックアップ・マクロのソルブの量（パラメータは
    /// [`Kind::Asph`] などの意味のある名前で）
    Parm(u8),
    /// 面の前の傾き偏心の値 i（0〜4 = dx dy tx ty tz）。ピックアップの量（座標ブレークの面は [`Kind::Parm`]、
    /// パラメータは [`Kind::Dx`] など）
    Before(u8),
    /// 面の後の傾き偏心の値 i（.zmx の SCBD 2 のピックアップ。元は [`Kind::Before`]、反転は倍率 −1）
    After(u8),
    /// 膜の名前（ピックアップだけ。倍率・オフセットは使わない。OpticStudio に無いので .zmx には書かない）
    Coating,
    /// 利用者定義の形の面の設定の数の項目（名前、[`crate::usershape`]）。名前は `u_<項目>`（面の番号と `.` で区切るので、項目の名前に `.` は使えない）
    User(crate::usershape::UserKey),
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        if let Some(n) = s
            .trim()
            .strip_prefix("u_")
            .filter(|n| !n.is_empty() && !n.contains('.'))
        {
            return match crate::usershape::UserKey::get(n) {
                Some(k) => Ok(Kind::User(k)),
                None => invalid!("no user-defined shape has the number setting {n}"),
            };
        }
        let l = s.trim().to_ascii_lowercase();
        Ok(match l.as_str() {
            "c" | "curv" => Kind::Curv,
            "r" | "radius" => Kind::Radius,
            "k" | "conic" => Kind::Conic,
            "t" | "thick" | "thickness" => Kind::Thick,
            "dx" => Kind::Dx,
            "dy" => Kind::Dy,
            "tx" => Kind::Tx,
            "ty" => Kind::Ty,
            "tz" => Kind::Tz,
            "dn" => Kind::Dn,
            "disp" => Kind::Disp,
            "objx" => Kind::ObjX,
            "objy" => Kind::ObjY,
            "semi" => Kind::Semi,
            "ignore" => Kind::Ignore,
            "apmax" => Kind::ApMax,
            "material" => Kind::Material,
            "apmin" => Kind::ApMin,
            "apdx" => Kind::ApDx,
            "apdy" => Kind::ApDy,
            "aptype" => Kind::ApType,
            "beforeorder" => Kind::BeforeOrder,
            "afterorder" => Kind::AfterOrder,
            "primary" => Kind::Primary,
            "aper" => Kind::Aper,
            "atype" => Kind::ApertureType,
            "stop" => Kind::Stop,
            "ftype" => Kind::FieldType,
            "temp" => Kind::Temp,
            "pres" => Kind::Pres,
            "afocal" => Kind::Afocal,
            "apodtype" => Kind::ApodType,
            "apod" => Kind::Apod,
            "aiming" => Kind::Aiming,
            "ropd" => Kind::Ropd,
            "coating" | "coat" => Kind::Coating,
            _ => {
                let num = |p: &str| l.strip_prefix(p).and_then(|n| n.parse::<u8>().ok());
                if let Some(i) = num("before").filter(|&i| i < 5) {
                    return Ok(Kind::Before(i));
                }
                if let Some(i) = num("after").filter(|&i| i < 5) {
                    return Ok(Kind::After(i));
                }
                if let Some(i) = num("parm").or_else(|| num("p")) {
                    return Ok(Kind::Parm(i));
                }
                for (pre, f) in [
                    ("wwt", Kind::WaveW as fn(u8) -> Kind),
                    ("fx", Kind::FieldX),
                    ("fy", Kind::FieldY),
                    ("fwt", Kind::FieldW),
                ] {
                    if let Some(n) = l.strip_prefix(pre).and_then(|v| v.parse::<u16>().ok()) {
                        if (1..=256).contains(&n) {
                            return Ok(f((n - 1) as u8));
                        }
                    }
                }
                if let Some(n) = l.strip_prefix("wave").and_then(|v| v.parse::<u16>().ok()) {
                    if (1..=256).contains(&n) {
                        return Ok(Kind::Wave((n - 1) as u8));
                    }
                }
                if let Some(n) = l.strip_prefix('z').and_then(|v| v.parse::<u16>().ok()) {
                    if (1..=231).contains(&n) {
                        return Ok(Kind::Zern(n));
                    }
                }
                if let Some(n) = l.strip_prefix('a').and_then(|v| v.parse::<u8>().ok()) {
                    if (2..=16).contains(&n) && n % 2 == 0 {
                        return Ok(Kind::Asph(n / 2 - 1));
                    }
                }
                invalid!("parameter type {s} (c r k a2..a16 z1..z231 t dx dy tx ty tz dn disp objx objy wave1.. wwt1.. primary aper fx1.. fy1.. fwt1.. u_<setting>, for pickups also semi p1.. before0..4 after0..4 coating)")
            }
        })
    }

    pub fn name(&self) -> String {
        match self {
            Kind::Curv => "c".into(),
            Kind::Radius => "r".into(),
            Kind::Conic => "k".into(),
            Kind::Asph(i) => format!("a{}", 2 * (*i as usize + 1)),
            Kind::Zern(j) => format!("z{j}"),
            Kind::Thick => "t".into(),
            Kind::Dx => "dx".into(),
            Kind::Dy => "dy".into(),
            Kind::Tx => "tx".into(),
            Kind::Ty => "ty".into(),
            Kind::Tz => "tz".into(),
            Kind::Dn => "dn".into(),
            Kind::Disp => "disp".into(),
            Kind::ObjX => "objx".into(),
            Kind::ObjY => "objy".into(),
            Kind::Wave(i) => format!("wave{}", *i as usize + 1),
            Kind::WaveW(i) => format!("wwt{}", *i as usize + 1),
            Kind::Primary => "primary".into(),
            Kind::Aper => "aper".into(),
            Kind::ApertureType => "atype".into(),
            Kind::Stop => "stop".into(),
            Kind::FieldType => "ftype".into(),
            Kind::Temp => "temp".into(),
            Kind::Pres => "pres".into(),
            Kind::Afocal => "afocal".into(),
            Kind::ApodType => "apodtype".into(),
            Kind::Apod => "apod".into(),
            Kind::Aiming => "aiming".into(),
            Kind::Ropd => "ropd".into(),
            Kind::FieldX(i) => format!("fx{}", *i as usize + 1),
            Kind::FieldY(i) => format!("fy{}", *i as usize + 1),
            Kind::FieldW(i) => format!("fwt{}", *i as usize + 1),
            Kind::Unit(u) => u.name().into(),
            Kind::Semi => "semi".into(),
            Kind::Ignore => "ignore".into(),
            Kind::ApMax => "apmax".into(),
            Kind::Material => "material".into(),
            Kind::ApMin => "apmin".into(),
            Kind::ApDx => "apdx".into(),
            Kind::ApDy => "apdy".into(),
            Kind::ApType => "aptype".into(),
            Kind::BeforeOrder => "beforeorder".into(),
            Kind::AfterOrder => "afterorder".into(),
            Kind::Parm(i) => format!("p{i}"),
            Kind::Before(i) => format!("before{i}"),
            Kind::After(i) => format!("after{i}"),
            Kind::Coating => "coating".into(),
            Kind::User(k) => format!("u_{}", k.name()),
        }
    }

    /// ピックアップの量（[`crate::pickup::Pickup::what`] と元）になる種類
    pub fn is_pick(&self) -> bool {
        matches!(
            self,
            Kind::Curv
                | Kind::Radius
                | Kind::Conic
                | Kind::Thick
                | Kind::Dn
                | Kind::Disp
                | Kind::Semi
                | Kind::Parm(_)
                | Kind::Before(_)
                | Kind::After(_)
                | Kind::Coating
        )
    }

    /// ピックアップ・マクロのソルブだけの量（パラメータには使えない）
    pub fn is_pick_only(&self) -> bool {
        matches!(
            self,
            Kind::Semi | Kind::Parm(_) | Kind::Before(_) | Kind::After(_) | Kind::Coating
        )
    }

    /// 多重構成の表の行にだけ使う面の値（半直径・面の無視・開口・面の前後の傾き偏心の値と順序）。最適化の変数・微分には
    /// 使えない
    pub fn is_table_only(&self) -> bool {
        matches!(
            self,
            Kind::Semi
                | Kind::Ignore
                | Kind::ApMax
                | Kind::ApMin
                | Kind::ApDx
                | Kind::ApDy
                | Kind::ApType
                | Kind::Before(_)
                | Kind::After(_)
                | Kind::BeforeOrder
                | Kind::AfterOrder
                | Kind::Material
        )
    }

    /// 傾き偏心の値の位置（dx dy tx ty tz）
    pub fn tilt_index(&self) -> Option<usize> {
        match self {
            Kind::Dx => Some(0),
            Kind::Dy => Some(1),
            Kind::Tx => Some(2),
            Kind::Ty => Some(3),
            Kind::Tz => Some(4),
            _ => None,
        }
    }

    /// 物点の移動（面のパラメータではない）
    pub fn is_object(&self) -> bool {
        matches!(self, Kind::ObjX | Kind::ObjY)
    }

    /// 系の波長
    pub fn is_wave(&self) -> bool {
        matches!(self, Kind::Wave(_))
    }

    /// 面・玉・群に付かない系全体のパラメータ（物点の移動・波長）。[`Param::surf`] は使わない
    pub fn is_global(&self) -> bool {
        self.is_object() || self.is_system_value()
    }

    /// 系の値（波長・重み・主波長・開口・開口の種類・絞りの面・視野）。公差・多重構成の行に使い、最適化の変数・微分には使えない
    pub fn is_system_value(&self) -> bool {
        matches!(
            self,
            Kind::Wave(_)
                | Kind::WaveW(_)
                | Kind::Primary
                | Kind::Aper
                | Kind::ApertureType
                | Kind::Stop
                | Kind::FieldType
                | Kind::Temp
                | Kind::Pres
                | Kind::Afocal
                | Kind::ApodType
                | Kind::Apod
                | Kind::Aiming
                | Kind::Ropd
                | Kind::FieldX(_)
                | Kind::FieldY(_)
                | Kind::FieldW(_)
        )
    }

    /// 玉・群のパラメータ
    pub fn is_unit(&self) -> bool {
        matches!(self, Kind::Unit(_))
    }

    /// 玉・群の剛体の動き（座標系を動かす。面のパラメータの組合せにならない）
    pub fn is_unit_motion(&self) -> bool {
        matches!(self, Kind::Unit(u) if u.motion_index().is_some())
    }
}

/// パラメータが付く先。面・玉/群の番号の意味は種類で変わらない
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// 面の番号
    Surf(usize),
    /// 玉・群の番号（[`System::units`]）
    Unit(usize),
    /// 面・玉・群に付かない系全体（物点の移動・波長）
    System,
}

/// パラメータ: 付く先と種類。付く先は種類から決まる（[`Kind::is_global`] は系全体、[`Kind::Unit`] は玉・群、
/// ほかは面）ので、[`Param::new`]・[`Param::unit`]・[`Param::system`] で作る
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub struct Param {
    pub target: Target,
    pub kind: Kind,
}

impl Param {
    /// 面 surf のパラメータ kind（kind が系全体・玉/群の種類ならそちら。玉・群は surf が玉・群の番号）
    pub fn new(surf: usize, kind: Kind) -> Param {
        let target = if kind.is_global() {
            Target::System
        } else if kind.is_unit() {
            Target::Unit(surf)
        } else {
            Target::Surf(surf)
        };
        Param { target, kind }
    }
    /// 系全体のパラメータ（物点の移動・波長）
    pub fn system(kind: Kind) -> Param {
        debug_assert!(kind.is_global());
        Param {
            target: Target::System,
            kind,
        }
    }
    pub fn obj_x() -> Param {
        Param::system(Kind::ObjX)
    }
    pub fn obj_y() -> Param {
        Param::system(Kind::ObjY)
    }
    /// 玉・群 u のパラメータ
    pub fn unit(u: usize, kind: UnitKind) -> Param {
        Param {
            target: Target::Unit(u),
            kind: Kind::Unit(kind),
        }
    }
    /// 面の番号（面のパラメータでなければ None）
    pub fn surface(&self) -> Option<usize> {
        match self.target {
            Target::Surf(j) => Some(j),
            _ => None,
        }
    }
    /// 玉・群の番号（玉・群のパラメータでなければ None）
    pub fn unit_index(&self) -> Option<usize> {
        match self.target {
            Target::Unit(u) => Some(u),
            _ => None,
        }
    }
    /// 面・玉/群の番号（系全体は 0）。番号の意味は [`Self::target`] で決まる
    pub fn index(&self) -> usize {
        match self.target {
            Target::Surf(j) | Target::Unit(j) => j,
            Target::System => 0,
        }
    }
    pub fn name(&self) -> String {
        match self.target {
            Target::System => self.kind.name(),
            Target::Unit(u) => format!("unit{u}.{}", self.kind.name()),
            Target::Surf(j) => format!("{j}.{}", self.kind.name()),
        }
    }
}

impl System {
    /// パラメータを名前で: "3.t"、"L1 front.c"（面の名前.種類）、"objx"、"lens2.tx"（玉・群の名前.種類）。
    /// `unit:` `surf:` を前に付けると玉・群か面かを決める
    pub fn param(&self, spec: &str) -> Result<Param> {
        let s = spec.trim();
        if let Ok(k) = Kind::parse(s) {
            if k.is_global() {
                let p = Param::new(0, k);
                self.check_param(p)?;
                return Ok(p);
            }
        }
        let Some((t, k)) = s.rsplit_once('.') else {
            invalid!("parameter {s} (write surface.type, e.g. 3.t)");
        };
        let (t, force) = if let Some(x) = t.strip_prefix("unit:") {
            (x, Some(true))
        } else if let Some(x) = t.strip_prefix("surf:") {
            (x, Some(false))
        } else {
            (t, None)
        };
        let unit = if force == Some(false) || t.trim().parse::<usize>().is_ok() {
            None
        } else {
            self.find_unit(t)
        };
        if force == Some(true) && unit.is_none() {
            invalid!("no lens or group named {}", t.trim());
        }
        if let Some(u) = unit {
            if force.is_none() && self.find_surface(t).is_ok() {
                let t = t.trim();
                invalid!("{t} is the name of a surface and of a lens or group (write unit:{t}.{k} or surf:{t}.{k})");
            }
            let p = Param::unit(u, UnitKind::parse(k)?);
            self.check_param(p)?;
            return Ok(p);
        }
        let kind = Kind::parse(k)?;
        let surf = self.find_surface(t)?;
        let p = Param::new(surf, kind);
        self.check_param(p)?;
        Ok(p)
    }

    /// パラメータの名前（玉・群は名前で）
    pub fn param_name(&self, p: Param) -> String {
        match p.kind {
            Kind::Unit(k) => match self.units.get(p.index()) {
                Some(u) => format!("{}.{}", u.name, k.name()),
                None => p.name(),
            },
            _ => p.name(),
        }
    }

    /// パラメータがこの系で使えるか
    pub fn check_param(&self, p: Param) -> Result<()> {
        // 付く先は種類から決まる（読んだ JSON の食い違いを断る）
        if Param::new(p.index(), p.kind) != p {
            invalid!("parameter {}: the target doesn't match the kind", p.name());
        }
        if p.kind.is_pick_only() && !p.kind.is_table_only() {
            invalid!("{} is a pickup quantity, not a parameter", p.kind.name());
        }
        if p.kind.is_object() {
            return Ok(());
        }
        if let Kind::Wave(i) | Kind::WaveW(i) = p.kind {
            if i as usize >= self.wavelengths.len() {
                let (kind, nw) = (p.kind.name(), self.wavelengths.len());
                invalid!("{kind} (there are {nw} wavelengths)");
            }
            return Ok(());
        }
        if let Kind::FieldX(i) | Kind::FieldY(i) | Kind::FieldW(i) = p.kind {
            if i as usize >= self.fields.len() {
                let (kind, nf) = (p.kind.name(), self.fields.len());
                invalid!("{kind} (there are {nf} field points)");
            }
            return Ok(());
        }
        if p.kind.is_system_value() {
            return Ok(());
        }
        if let Kind::Unit(k) = p.kind {
            let Some(u) = self.units.get(p.index()) else {
                invalid!("lens or group {} ({} of them)", p.index(), self.units.len());
            };
            if matches!(k, UnitKind::Dn | UnitKind::Disp) && self.unit_media(p.index()).is_empty() {
                let (unit, medium) = (&u.name, k.name());
                invalid!("lens or group {unit} has no medium (glass) inside ({medium})");
            }
            return Ok(());
        }
        let Target::Surf(j) = p.target else {
            invalid!("parameter {} is not on a surface", p.name());
        };
        if j >= self.surfaces.len() {
            let last = self.surfaces.len() - 1;
            invalid!("surface {j} (surfaces are 0 to {last})");
        }
        let d = &self.surfaces[j];
        let curved = d.shape_params();
        match p.kind {
            Kind::Curv | Kind::Radius | Kind::Conic if j == 0 || !curved => {
                invalid!("surface {j} ({:?}) {}", d.typ(), p.kind.name());
            }
            Kind::Asph(_) if !crate::pickup::takes_asph(d) => {
                invalid!("surface {j} is not an even asphere ({})", p.kind.name());
            }
            Kind::Asph(i) if i >= 8 => invalid!("asphere coefficient {}", p.kind.name()),
            Kind::Zern(n) => match &d.zern() {
                Some(z) if (n as usize) <= crate::freeform::ZernSag::max_terms(z.fringe) => {}
                Some(_) => invalid!("surface {j} Zernike term {n} is over the limit"),
                None => invalid!(
                    "surface {j} is not a Zernike Sag surface ({})",
                    p.kind.name()
                ),
            },
            _ => {}
        }
        // 形に効かない値（Tilted の c・k、Tilted の 3 番目からの asph など）は
        // 変数・微分にしない
        if let Kind::User(k) = p.kind {
            if d.user_shape().and_then(|u| u.value(k.name())).is_none() {
                let kind = p.kind.name();
                invalid!("surface {j} is not a user-defined shape with the number setting {kind}");
            }
        }
        if matches!(
            p.kind,
            Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_) | Kind::Zern(_) | Kind::User(_)
        ) && !d.takes_shape_param(p.kind)
        {
            let (typ, kind) = (d.typ(), p.kind.name());
            invalid!("surface {j} ({typ:?}): {kind} doesn't change the shape of this surface type");
        }
        match p.kind {
            Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz if j == 0 => {
                notimpl!("tilt or decenter of the object surface");
            }
            Kind::Dn | Kind::Disp if d.is_mirror() || d.is_coord_break() => {
                let kind = p.kind.name();
                invalid!("surface {j} is a mirror or a coordinate break (the medium is surface {kind} before it)");
            }
            Kind::Thick if j == self.image() => invalid!("thickness of the image surface"),
            _ => {}
        }
        Ok(())
    }

    /// パラメータの今の値
    pub fn get_param(&self, p: Param) -> Result<f64> {
        self.check_param(p)?;
        if p.kind.is_object() {
            let kind = p.kind.name();
            invalid!("field point {kind} is not system data (it is a trace input)");
        }
        match p.kind {
            Kind::Wave(i) => return Ok(self.wavelengths[i as usize]),
            Kind::WaveW(i) => return Ok(self.weights[i as usize]),
            Kind::Primary => return Ok(self.primary as f64 + 1.0),
            Kind::Aper => return Ok(self.aperture.1),
            Kind::ApertureType => return Ok(self.aperture.0.os_code() as f64),
            Kind::Stop => return Ok(self.stop() as f64),
            Kind::FieldType => match self.field_type.os_code() {
                Some(n) => return Ok(n as f64),
                None => invalid!("a solved field type has no field type number"),
            },
            Kind::Temp => return Ok(self.temperature),
            Kind::Pres => return Ok(self.pressure),
            Kind::Afocal => return Ok(self.afocal as u8 as f64),
            Kind::ApodType => return Ok(self.apodization.os_code() as f64),
            Kind::Apod => return Ok(self.apodization.factor()),
            Kind::Aiming => return Ok(self.aiming.os_code() as f64),
            Kind::Ropd => return Ok(self.ropd as f64),
            Kind::FieldX(i) => return Ok(self.fields[i as usize].x),
            Kind::FieldY(i) => return Ok(self.fields[i as usize].y),
            Kind::FieldW(i) => return Ok(self.fields[i as usize].w),
            _ => {}
        }
        if let Kind::Unit(k) = p.kind {
            let u = &self.units[p.index()];
            if let Some(i) = k.motion_index() {
                return Ok(u.motion[i]);
            }
            return Ok(match k {
                UnitKind::Shift => u.shift,
                _ => {
                    let v: Vec<f64> = self
                        .unit_media(p.index())
                        .iter()
                        .map(|&j| {
                            let d = &self.surfaces[j];
                            if k == UnitKind::Dn {
                                d.dn
                            } else {
                                d.disp
                            }
                        })
                        .collect();
                    if v.iter().any(|&x| x != v[0]) {
                        let (unit, medium) = (&u.name, k.name());
                        invalid!("lens or group {unit}: medium {medium} differs between surfaces ({v:?})");
                    }
                    v[0]
                }
            });
        }
        let d = &self.surfaces[p.index()];
        Ok(match p.kind {
            Kind::Curv => d.c,
            Kind::Radius => {
                if d.c == 0.0 {
                    0.0
                } else {
                    1.0 / d.c
                }
            }
            Kind::Conic => d.k,
            Kind::Asph(i) => d.asph().get(i as usize).copied().unwrap_or(0.0),
            Kind::Zern(n) => d
                .zern()
                .as_ref()
                .and_then(|z| z.coef.get(n as usize - 1).copied())
                .unwrap_or(0.0),
            Kind::User(k) => d
                .user_shape()
                .and_then(|u| u.value(k.name()))
                .unwrap_or(0.0),
            Kind::Thick => d.t,
            Kind::Semi => d.semi,
            Kind::Ignore => f64::from(u8::from(d.ignore)),
            Kind::Material => invalid!("{}: a material is a name, not a number", p.name()),
            Kind::ApMax => match d.ap {
                Aperture::Circular { max, .. } | Aperture::Obsc { max, .. } => max,
                _ => invalid!(
                    "{}: surface has no circular aperture or obscuration",
                    p.name()
                ),
            },
            Kind::ApMin => match d.ap {
                Aperture::Circular { min, .. } | Aperture::Obsc { min, .. } => min,
                _ => invalid!(
                    "{}: surface has no circular aperture or obscuration",
                    p.name()
                ),
            },
            Kind::ApDx | Kind::ApDy => match aperture_decenter(&d.ap) {
                Some((dx, dy)) => {
                    if p.kind == Kind::ApDx {
                        dx
                    } else {
                        dy
                    }
                }
                None => invalid!("{}: surface has no aperture", p.name()),
            },
            Kind::ApType => aperture_os_code(&d.ap) as f64,
            Kind::Before(i) => d.before.map_or(0.0, |t| t.d[i as usize]),
            Kind::BeforeOrder => d.before.map_or(0.0, |t| f64::from(t.order)),
            Kind::After(i) => match d.after {
                After::Tilt(t) => t.d[i as usize],
                After::None => 0.0,
                After::Reverse => invalid!(
                    "{}: the tilt and decenter after the surface reverses the one before",
                    p.name()
                ),
            },
            Kind::AfterOrder => match d.after {
                After::Tilt(t) => f64::from(t.order),
                After::None => 0.0,
                After::Reverse => invalid!(
                    "{}: the tilt and decenter after the surface reverses the one before",
                    p.name()
                ),
            },
            Kind::Dn => d.dn,
            Kind::Disp => d.disp,
            k => {
                let i = k.tilt_index().expect("tilt kind");
                if d.is_coord_break() {
                    d.cb()[i]
                } else {
                    d.before.map(|b| b.d[i]).unwrap_or(0.0)
                }
            }
        })
    }

    /// パラメータを変える。影響しないキャッシュ（座標系・近軸量）は残す
    pub fn set_param(&mut self, p: Param, v: f64) -> Result<()> {
        self.set_params(&[(p, v)])
    }

    /// まとめて変える（キャッシュの捨て方も 1 回で決める）。エラーのときは系を変えない
    pub fn set_params(&mut self, changes: &[(Param, f64)]) -> Result<()> {
        *self = self.with_params(changes)?;
        Ok(())
    }

    /// 複製してパラメータを変える（元の系のキャッシュのうち影響しないものを引き継ぐ）
    /// 自動の半直径も求め直す
    pub fn with_params(&self, changes: &[(Param, f64)]) -> Result<System> {
        let mut s = self.with_params_solve(changes, true)?;
        s.update_semi();
        Ok(s)
    }

    /// [`System::with_params`] で、自動の半直径を求め直さない（最適化・公差の差分など、半直径を使わない繰り返し用。
    /// 半直径は光線の追跡に効かない）
    pub fn with_params_fast(&self, changes: &[(Param, f64)]) -> Result<System> {
        let mut s = self.with_params_solve(changes, true)?;
        if s.semi_affects_trace() {
            s.update_semi();
        }
        Ok(s)
    }

    /// [`System::with_params`]。solve が false ならソルブを解き直さない（ソルブの微分の差分用）
    pub(crate) fn with_params_solve(
        &self,
        changes: &[(Param, f64)],
        solve: bool,
    ) -> Result<System> {
        let (flat, motions, picked) = self.unit_changes(changes)?;
        let mut keep = keep_for(&flat);
        if !motions.is_empty() {
            keep.frames = false;
        }
        let mut s = self.clone_keep(keep);
        s.apply(&flat)?;
        for &(u, k, v) in &motions {
            let x = &mut s.rx_mut().units[u];
            match k.motion_index() {
                Some(i) => x.motion[i] = v,
                None => x.shift = v,
            }
        }
        // ピックアップで決まる先は、展開した値と合っているか確かめる（倍率・オフセットが玉・群の動きと合わない）
        for &(p, v) in &picked {
            let now = s.get_param(p)?;
            if (now - v).abs() > 1e-12 * (1.0 + v.abs()) {
                let name = p.name();
                invalid!("{name} becomes {now} by a pickup and doesn't match {v} of the lens or group change");
            }
        }
        // 傾き・偏心だけの変更で近軸量を残せて、ソルブが近軸量と面の形・厚さだけで決まり既に解けていれば、
        // 解き直さない（解き直すと座標系などのキャッシュを捨てる）
        let any = !s.solves.is_empty() || !s.macro_parms.is_empty();
        if solve && any && !(keep.paraxial && s.macro_parms.is_empty() && s.solves_still_hold()) {
            s.solve_thickness()?;
        }
        if flat.iter().any(|(p, _)| p.index() == 0) {
            s.check()?;
        }
        Ok(s)
    }

    /// 玉・群のパラメータの変更を、面のパラメータの変更と、玉・群の動き・移動の値の更新に分ける。
    /// 3 つ目は、ピックアップ先なので変更から除いた面のパラメータと、なるべき値
    #[allow(clippy::type_complexity)]
    fn unit_changes(
        &self,
        changes: &[(Param, f64)],
    ) -> Result<(
        Vec<(Param, f64)>,
        Vec<(usize, UnitKind, f64)>,
        Vec<(Param, f64)>,
    )> {
        if !changes.iter().any(|(p, _)| p.kind.is_unit()) {
            return Ok((changes.to_vec(), Vec::new(), Vec::new()));
        }
        // 3 つ目は玉・群から展開した変更か
        let mut flat: Vec<(Param, f64, bool)> = Vec::new();
        let mut motions = Vec::new();
        // 移動の厚さの変化は、同じ面に重なれば足す（隣り合う範囲の間の間隔など）
        let mut shifted: Vec<(usize, f64)> = Vec::new();
        let mut shift_at: Option<usize> = None;
        for &(p, v) in changes {
            let Kind::Unit(k) = p.kind else {
                flat.push((p, v, false));
                continue;
            };
            self.check_param(p)?;
            if !v.is_finite() {
                invalid!("{} = {v}", self.param_name(p));
            }
            match k {
                UnitKind::Shift => {
                    let d = v - self.units[p.index()].shift;
                    for (q, f) in self.unit_terms(p, false)? {
                        match shifted.iter_mut().find(|x| x.0 == q.index()) {
                            Some(x) => x.1 += f * d,
                            None => shifted.push((q.index(), f * d)),
                        }
                    }
                    shift_at.get_or_insert(flat.len());
                    motions.push((p.index(), k, v));
                }
                UnitKind::Dn | UnitKind::Disp => {
                    for (q, _) in self.unit_terms(p, false)? {
                        flat.push((q, v, true));
                    }
                }
                _ => motions.push((p.index(), k, v)),
            }
        }
        if let Some(at) = shift_at {
            let th: Vec<(Param, f64, bool)> = shifted
                .iter()
                .filter(|x| x.1 != 0.0)
                .map(|&(j, d)| (Param::new(j, Kind::Thick), self.surfaces[j].t + d, true))
                .collect();
            flat.splice(at..at, th);
        }
        // ピックアップ先で、元も変える面は除く（ピックアップで追従する）
        let mut picked = Vec::new();
        let srcs: Vec<Param> = flat.iter().map(|x| x.0).collect();
        flat.retain(|&(p, v, gen)| match self.pickup_of(p) {
            Some(pk)
                if gen
                    && srcs.iter().any(|q| {
                        q.index() == pk.src
                            && pick_kind(&self.surfaces[q.index()], q.kind) == Some(pk.what)
                    }) =>
            {
                picked.push((p, v));
                false
            }
            _ => true,
        });
        Ok((
            flat.into_iter().map(|(p, v, _)| (p, v)).collect(),
            motions,
            picked,
        ))
    }

    /// 玉・群の shift・dn・disp を、面のパラメータの組合せ（パラメータ, 係数）にする。
    /// shift: 範囲ごとに前の間隔 +1・後の間隔 −1（無限遠の物体距離・厚さのソルブの面・ピックアップ先で元も
    /// 組合せに入る面は除く）。dn・disp: 範囲の中の媒質ごとに +1
    pub(crate) fn unit_linear(&self, p: Param) -> Result<Vec<(Param, f64)>> {
        self.unit_terms(p, true)
    }

    /// unit_linear の中身。drop_picked が false ならピックアップ先も残す（変更では後で確かめる）
    fn unit_terms(&self, p: Param, drop_picked: bool) -> Result<Vec<(Param, f64)>> {
        let Kind::Unit(k) = p.kind else {
            return Ok(vec![(p, 1.0)]);
        };
        let u = &self.units[p.index()];
        let mut out: Vec<(Param, f64)> = Vec::new();
        match k {
            UnitKind::Shift => {
                let mut add = |j: usize, f: f64| match out.iter_mut().find(|x| x.0.index() == j) {
                    Some(x) => x.1 += f,
                    None => out.push((Param::new(j, Kind::Thick), f)),
                };
                for s in &u.spans {
                    add(s[0] - 1, 1.0);
                    add(s[1], -1.0);
                }
                out.retain(|(q, f)| {
                    *f != 0.0
                        && self.surfaces[q.index()].t.is_finite()
                        && !self.solves.iter().any(|x| x.surf == q.index())
                });
                if drop_picked {
                    let all: Vec<usize> = out.iter().map(|x| x.0.index()).collect();
                    out.retain(|(q, _)| match self.pickup_of(*q) {
                        Some(pk) => !all.contains(&pk.src),
                        None => true,
                    });
                }
            }
            UnitKind::Dn | UnitKind::Disp => {
                let kind = if k == UnitKind::Dn {
                    Kind::Dn
                } else {
                    Kind::Disp
                };
                for j in self.unit_media(p.index()) {
                    out.push((Param::new(j, kind), 1.0));
                }
            }
            _ => out.push((p, 1.0)),
        }
        Ok(out)
    }

    /// 値を書き換える（キャッシュはそのまま）。残してよいキャッシュを返す
    fn apply(&mut self, changes: &[(Param, f64)]) -> Result<Keep> {
        // 書き換える前にすべて確かめる（途中で止まって半端に変わった系を残さない）
        for &(p, v) in changes {
            self.check_param(p)?;
            if p.kind.is_object() {
                let kind = p.kind.name();
                invalid!("field point {kind} is not system data (it is a trace input)");
            }
            if !v.is_finite() && !(p.kind == Kind::Thick && p.index() == 0) {
                invalid!("{} = {v}", p.name());
            }
            if p.kind.is_wave() {
                if !(v > 0.0) {
                    invalid!("{} = {v} µm", p.name());
                }
                continue;
            }
            match p.kind {
                Kind::Primary
                    if v.fract() != 0.0 || v < 1.0 || v > self.wavelengths.len() as f64 =>
                {
                    invalid!("primary wavelength {v} (1 to {})", self.wavelengths.len());
                }
                Kind::WaveW(_) | Kind::FieldW(_) if v < 0.0 => invalid!("{} = {v}", p.name()),
                Kind::Aper if !(v > 0.0) => invalid!("aperture value {v}"),
                Kind::ApertureType
                    if v.fract() != 0.0 || ApertureKind::from_os_code(v as i64).is_none() =>
                {
                    invalid!("aperture type {v} (0 to 5)");
                }
                Kind::Semi if v < 0.0 => invalid!("{} = {v}", p.name()),
                Kind::Ignore if v != 0.0 && v != 1.0 => invalid!("{} = {v} (0 or 1)", p.name()),
                Kind::ApMax if v < 0.0 => invalid!("{} = {v}", p.name()),
                Kind::Material => invalid!("{}: a material is a name, not a number", p.name()),
                Kind::ApMin if v < 0.0 => invalid!("{} = {v}", p.name()),
                Kind::BeforeOrder | Kind::AfterOrder if v != 0.0 && v != 1.0 => {
                    invalid!("{} = {v} (0 or 1)", p.name())
                }
                Kind::ApType if v.fract() != 0.0 || !(0.0..=2.0).contains(&v) => {
                    invalid!("{}: aperture type {v} (0 to 2 can be set)", p.name())
                }
                Kind::ApType
                    if v != 0.0
                        && !matches!(
                            self.surfaces[p.index()].ap,
                            Aperture::Circular { .. } | Aperture::Obsc { .. }
                        ) =>
                {
                    invalid!(
                        "{}: surface has no circular aperture or obscuration (its radii are kept)",
                        p.name()
                    )
                }
                Kind::ApMin
                    if !matches!(
                        self.surfaces[p.index()].ap,
                        Aperture::Circular { .. } | Aperture::Obsc { .. }
                    ) =>
                {
                    invalid!(
                        "{}: surface has no circular aperture or obscuration",
                        p.name()
                    )
                }
                Kind::ApDx | Kind::ApDy
                    if aperture_decenter(&self.surfaces[p.index()].ap).is_none() =>
                {
                    invalid!("{}: surface has no aperture", p.name())
                }
                Kind::Before(_) | Kind::BeforeOrder | Kind::After(_) | Kind::AfterOrder
                    if self.surfaces[p.index()].is_coord_break() =>
                {
                    invalid!(
                        "{}: a coordinate break has no surface tilt or decenter",
                        p.name()
                    )
                }
                Kind::After(_) | Kind::AfterOrder
                    if self.surfaces[p.index()].after == After::Reverse =>
                {
                    invalid!(
                        "{}: the tilt and decenter after the surface reverses the one before",
                        p.name()
                    )
                }
                Kind::ApMax
                    if !matches!(
                        self.surfaces[p.index()].ap,
                        Aperture::Circular { .. } | Aperture::Obsc { .. }
                    ) =>
                {
                    invalid!(
                        "{}: surface has no circular aperture or obscuration",
                        p.name()
                    )
                }
                Kind::Ignore if p.index() == 0 || p.index() + 1 >= self.surfaces.len() => {
                    invalid!(
                        "{}: the object and image surfaces can't be ignored",
                        p.name()
                    );
                }
                Kind::FieldType
                    if v.fract() != 0.0
                        || crate::system::FieldType::from_os_code(v as i64).is_none() =>
                {
                    invalid!("field type {v} (0 to 3)");
                }
                Kind::Pres if v < 0.0 => invalid!("pressure {v} atm"),
                Kind::Temp if v < -273.15 => invalid!("temperature {v} °C"),
                Kind::Afocal if v != 0.0 && v != 1.0 => invalid!("{} = {v} (0 or 1)", p.name()),
                Kind::ApodType if v.fract() != 0.0 || !(0.0..=2.0).contains(&v) => {
                    invalid!("apodization type {v} (0 to 2)");
                }
                Kind::Apod if v < 0.0 => invalid!("apodization factor {v}"),
                Kind::Aiming if v.fract() != 0.0 || !(0.0..=2.0).contains(&v) => {
                    invalid!("ray aiming {v} (0 to 2)");
                }
                Kind::Ropd if v.fract() != 0.0 || !(0.0..=3.0).contains(&v) => {
                    invalid!("OPD reference {v} (0 to 3)");
                }
                Kind::Stop
                    if v.fract() != 0.0 || v < 1.0 || v > self.surfaces.len() as f64 - 2.0 =>
                {
                    let last = self.surfaces.len() - 2;
                    invalid!("stop surface {v} (1 to {last})");
                }
                _ => {}
            }
            if p.kind.is_system_value() {
                continue;
            }
            let solved = |k: Kind| {
                self.solves
                    .iter()
                    .find(|q| q.surf == p.index() && q.kind.param_kind() == k)
            };
            let q = match p.kind {
                Kind::Thick => solved(Kind::Thick),
                Kind::Curv | Kind::Radius => solved(Kind::Curv),
                _ => None,
            };
            if let Some(q) = q {
                invalid!(
                    "{} is set by a solve ({})",
                    p.name(),
                    solve_kind_info(q.kind).0
                );
            }
            if let Some(pk) = self.pickup_of(p) {
                let (name, src, scale, offset) = (p.name(), pk.src, pk.scale, pk.offset);
                invalid!("{name} is a pickup from surface {src} (scale {scale}, offset {offset}); change the source surface");
            }
        }
        let mut index = false;
        // 波長は先に変える（屈折率は元の系の値の作り方に合わせて作り直す）
        let waves: Vec<(usize, f64)> = changes
            .iter()
            .filter_map(|&(p, v)| match p.kind {
                Kind::Wave(i) => Some((i as usize, v)),
                _ => None,
            })
            .collect();
        if !waves.is_empty() {
            self.set_wavelengths(&waves)?;
            index = true;
        }
        for &(p, v) in changes {
            let rx = self.rx_mut();
            match p.kind {
                Kind::Wave(_) => continue,
                Kind::WaveW(i) => rx.weights[i as usize] = v,
                Kind::Primary => rx.primary = v as usize - 1,
                Kind::Aper => rx.aperture.1 = v,
                Kind::ApertureType => {
                    // 番号は上で確かめた
                    if let Some(k) = ApertureKind::from_os_code(v as i64) {
                        rx.aperture.0 = k;
                    }
                }
                Kind::FieldType => {
                    // 番号は上で確かめた
                    if let Some(t) = crate::system::FieldType::from_os_code(v as i64) {
                        rx.field_type = t;
                    }
                }
                Kind::Stop => {
                    for (j, d) in rx.surfaces.iter_mut().enumerate() {
                        d.stop = j == v as usize;
                    }
                }
                Kind::Temp => {
                    rx.temperature = v;
                    index = true;
                }
                Kind::Pres => {
                    rx.pressure = v;
                    index = true;
                }
                Kind::Afocal => rx.afocal = v != 0.0,
                // 種類を変えてから係数（同じ変更の中の順によらない）
                Kind::ApodType => {
                    let g = changes
                        .iter()
                        .find(|(q, _)| q.kind == Kind::Apod)
                        .map_or(rx.apodization.factor(), |c| c.1);
                    rx.apodization = Apodization::from_os_code(v as i64, g);
                }
                Kind::Apod => {
                    if let Apodization::Gaussian(g) = &mut rx.apodization {
                        *g = v;
                    }
                }
                Kind::Aiming => rx.aiming = Aiming::from_os_code(v as i64),
                Kind::Ropd => rx.ropd = v as i64,
                Kind::FieldX(i) => rx.fields[i as usize].x = v,
                Kind::FieldY(i) => rx.fields[i as usize].y = v,
                Kind::FieldW(i) => rx.fields[i as usize].w = v,
                _ => {}
            }
            if p.kind.is_system_value() {
                continue;
            }
            let d = &mut self.rx_mut().surfaces[p.index()];
            match p.kind {
                Kind::Curv => d.c = v,
                Kind::Radius => d.c = if v == 0.0 { 0.0 } else { 1.0 / v },
                Kind::Conic => d.k = v,
                Kind::Asph(i) => {
                    let i = i as usize;
                    if let Some(a) = d.asph_mut().filter(|a| a.len() <= i) {
                        a.resize(8, 0.0);
                    }
                    d.set_asph_at(i, v);
                }
                Kind::Zern(n) => {
                    let z = std::sync::Arc::make_mut(
                        d.zern_mut().and_then(|z| z.as_mut()).expect("Zernike data"),
                    );
                    let i = n as usize - 1;
                    if z.coef.len() <= i {
                        z.coef.resize(i + 1, 0.0);
                    }
                    z.coef[i] = v;
                    z.rebuild();
                }
                Kind::User(k) => {
                    let slot = d.user_shape_mut().expect("user-defined shape");
                    let u = slot
                        .as_ref()
                        .expect("user-defined shape")
                        .with_value(k.name(), v)?;
                    *slot = Some(u);
                }
                Kind::Thick => d.t = v,
                Kind::Semi => {
                    d.semi = v;
                    d.semi_auto = false;
                    // 浮動開口は面の半直径でける
                    if let Aperture::Floating { semi } = &mut d.ap {
                        *semi = v;
                    }
                }
                Kind::Ignore => d.ignore = v != 0.0,
                // 上で断った
                Kind::Material => {}
                Kind::ApMax => {
                    if let Aperture::Circular { max, .. } | Aperture::Obsc { max, .. } = &mut d.ap {
                        *max = v;
                    }
                }
                Kind::ApMin
                | Kind::ApDx
                | Kind::ApDy
                | Kind::ApType
                | Kind::Before(_)
                | Kind::BeforeOrder
                | Kind::After(_)
                | Kind::AfterOrder => {
                    set_surface_table_value(d, p.kind, v);
                }
                Kind::Dn => {
                    d.dn = v;
                    index = true;
                }
                Kind::Disp => {
                    d.disp = v;
                    index = true;
                }
                k => {
                    let i = k.tilt_index().expect("tilt kind");
                    if let Some((cb, _)) = d.cb_mut() {
                        cb[i] = v;
                    } else {
                        // 前の傾き偏心がなければ作り、面の後は「前を戻す」にする（その面だけが動く）。
                        // 既にあれば面の後はそのまま（後ろの面も一緒に動く系もある）
                        if d.before.is_none() {
                            d.before = Some(Tilt {
                                d: [0.0; 5],
                                order: 0,
                            });
                            if d.after == After::None {
                                d.after = After::Reverse;
                            }
                        }
                        d.before.as_mut().expect("coordinate break created above").d[i] = v;
                    }
                }
            }
        }
        if !self.pickups.is_empty() {
            self.resolve_pickups()?;
        }
        if index {
            self.resolve_media()?;
        }
        Ok(keep_for(changes))
    }
}

/// 変えるパラメータから、残してよいキャッシュを決める
pub(crate) fn keep_for(changes: &[(Param, f64)]) -> Keep {
    // 玉・群の剛体の動きは傾き偏心と同じ（座標系だけ）。移動は厚さ、dn・disp は屈折率
    let tilt = |k: Kind| {
        k.tilt_index().is_some()
            || k.is_unit_motion()
            || matches!(
                k,
                Kind::Before(_) | Kind::After(_) | Kind::BeforeOrder | Kind::AfterOrder
            )
    };
    let geom = changes.iter().any(|(p, _)| {
        p.kind == Kind::Thick || p.kind == Kind::Unit(UnitKind::Shift) || tilt(p.kind)
    });
    let only_tilt = changes.iter().all(|(p, _)| tilt(p.kind));
    Keep {
        frames: !geom,
        paraxial: only_tilt,
    }
}

/// 開口・遮蔽の偏心（偏心を持たない開口は None）
fn aperture_decenter(a: &Aperture) -> Option<(f64, f64)> {
    match *a {
        Aperture::Circular { dx, dy, .. }
        | Aperture::Rect { dx, dy, .. }
        | Aperture::Ellipse { dx, dy, .. }
        | Aperture::Obsc { dx, dy, .. }
        | Aperture::Spider { dx, dy, .. } => Some((dx, dy)),
        _ => None,
    }
}

fn set_aperture_decenter(a: &mut Aperture, y: bool, v: f64) {
    if let Aperture::Circular { dx, dy, .. }
    | Aperture::Rect { dx, dy, .. }
    | Aperture::Ellipse { dx, dy, .. }
    | Aperture::Obsc { dx, dy, .. }
    | Aperture::Spider { dx, dy, .. } = a
    {
        *if y { dy } else { dx } = v;
    }
}

/// 開口の種類の OpticStudio の番号（SurfaceApertureTypes の順）
fn aperture_os_code(a: &Aperture) -> i64 {
    match a {
        Aperture::None => 0,
        Aperture::Circular { .. } => 1,
        Aperture::Obsc { .. } => 2,
        Aperture::Spider { .. } => 3,
        Aperture::Rect { .. } => 4,
        Aperture::Ellipse { .. } => 6,
        Aperture::Floating { .. } => 10,
    }
}

/// 開口の種類を番号 n（0〜2）にする。半径・偏心は今の円形の開口・遮蔽の値（なしにしたら捨てる）
fn set_aperture_os_code(a: &mut Aperture, n: i64) {
    let (min, max, dx, dy) = match *a {
        Aperture::Circular { min, max, dx, dy } | Aperture::Obsc { min, max, dx, dy } => {
            (min, max, dx, dy)
        }
        _ => {
            if n == 0 {
                *a = Aperture::None;
            }
            return;
        }
    };
    *a = match n {
        1 => Aperture::Circular { min, max, dx, dy },
        2 => Aperture::Obsc { min, max, dx, dy },
        _ => Aperture::None,
    };
}

/// 面の多重構成の表だけの値（開口の最小の半径・偏心・種類、面の前後の傾き偏心の値と順序）を面 d に入れる（値は確かめない）。
/// 面の前後の傾き偏心を表の値で作るときは、面の後を変えない（OpticStudio と同じく、後は値 0 のまま）。無いものに 0 を入れるときは
/// 作らない。ほかの種類は何もせず false
pub fn set_surface_table_value(d: &mut crate::surface::Surface, k: Kind, v: f64) -> bool {
    match k {
        Kind::ApMin => {
            if let Aperture::Circular { min, .. } | Aperture::Obsc { min, .. } = &mut d.ap {
                *min = v;
            }
        }
        Kind::ApDx | Kind::ApDy => set_aperture_decenter(&mut d.ap, k == Kind::ApDy, v),
        Kind::ApType => set_aperture_os_code(&mut d.ap, v as i64),
        Kind::Before(_) | Kind::BeforeOrder if d.before.is_none() && v == 0.0 => {}
        Kind::After(_) | Kind::AfterOrder if d.after == After::None && v == 0.0 => {}
        Kind::Before(i) => {
            d.before
                .get_or_insert(Tilt {
                    d: [0.0; 5],
                    order: 0,
                })
                .d[i as usize] = v
        }
        Kind::BeforeOrder => {
            d.before
                .get_or_insert(Tilt {
                    d: [0.0; 5],
                    order: 0,
                })
                .order = v as u8
        }
        Kind::After(_) | Kind::AfterOrder => {
            if d.after == After::None {
                d.after = After::Tilt(Tilt {
                    d: [0.0; 5],
                    order: 0,
                });
            }
            if let After::Tilt(t) = &mut d.after {
                match k {
                    Kind::After(i) => t.d[i as usize] = v,
                    _ => t.order = v as u8,
                }
            }
        }
        _ => return false,
    }
    true
}
