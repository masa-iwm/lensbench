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
//! ピックアップ（[`Pickup`]、.zmx の曲率・厚さ・円錐係数・パラメータのピックアップ）は OpticStudio と同じく追従する:
//! 元の面のパラメータを変えると、ピックアップ先も「倍率 × 元の値 + オフセット」に解決し直す。
//! ピックアップ先のパラメータそのものは変えられない（エラー。元の面を変える）。
//! 玉の並びを組み直した系（[`crate::elements::assemble`]）はピックアップを引き継がない。
//!
//! 玉・群の単位のパラメータ（[`crate::units`]）も同じ形で指す: `lens2.tx`（玉モードの 2 番目の玉の傾き）、
//! `G1.shift`（群 G1 の移動）。[`Param::surf`] は玉・群の番号（[`System::units`]）、種類は [`Kind::Unit`]。
//! 名前が面の名前（コメント）と玉・群の名前の両方にあるときは、`unit:名前.種類` `surf:名前.種類` で区別する。

use crate::error::Result;
use crate::surface::{After, SurfType, Surface, Tilt};
use crate::system::{Keep, System};
use crate::units::UnitKind;
use crate::{invalid, notimpl};

/// パラメータの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// （光源の波長のずれの公差用。最適化の変数・多重構成の行には使えない）
    Wave(u8),
    /// 玉・群のパラメータ（[`Param::surf`] は玉・群の番号）
    Unit(UnitKind),
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
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
            _ => {
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
                invalid!("パラメータの種類 {s}（c r k a2〜a16 z1〜z231 t dx dy tx ty tz dn disp objx objy wave1〜）")
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
            Kind::Unit(u) => u.name().into(),
        }
    }

    /// 傾き偏心の値の位置（dx dy tx ty tz）
    pub(crate) fn tilt_index(&self) -> Option<usize> {
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
        self.is_object() || self.is_wave()
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

/// ピックアップする量
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PickKind {
    /// 曲率 c（倍率・オフセットは曲率にかかる）
    Curv,
    /// 次の面までの厚さ
    Thick,
    /// 円錐係数
    Conic,
    /// 面タイプのパラメータ（.zmx の PARM の番号、1 から）
    Parm(u8),
    /// 媒質の屈折率のずれ dn（材料のピックアップで、元の面の材料と一緒に追従する）
    Dn,
    /// 媒質の分散の相対変化 disp（材料のピックアップ）
    Disp,
    /// 曲率半径 R = 1/c（平面は無限大）。別の列からのピックアップの元としてだけ使う（先は [`PickKind::Curv`]）
    Radius,
    /// 半直径（semi-diameter）
    Semi,
    /// 面の前の傾き偏心の値 i（0〜4 = dx dy tx ty tz）。ピックアップの元にも先にもなる（座標ブレークの面は [`PickKind::Parm`]）
    Before(u8),
    /// 面の後の傾き偏心の値 i（.zmx の SCBD 2 のピックアップ。元は [`PickKind::Before`]、反転は倍率 −1）
    After(u8),
    /// 膜の名前（倍率・オフセットは使わない。OpticStudio に無いので .zmx には書かない）
    Coating,
}

impl PickKind {
    /// .zmx のピックアップの列のコード（0 = 同じ列、1 = 半径、2 = 厚さ、3 = 円錐、4 = 半直径、5 + k = パラメータ k。
    /// OpticStudio 2026 R1.01 で実測）から、元の量。先と同じ列なら None
    pub fn from_column(code: i64, dst: PickKind) -> Option<Option<PickKind>> {
        let k = match code {
            0 => return Some(None),
            1 => PickKind::Radius,
            2 => PickKind::Thick,
            3 => PickKind::Conic,
            4 => PickKind::Semi,
            c if (5..=5 + 255).contains(&c) => PickKind::Parm((c - 5) as u8),
            _ => return None,
        };
        let same = k == dst || (k == PickKind::Radius && dst == PickKind::Curv);
        Some(if same { None } else { Some(k) })
    }

    /// .zmx のピックアップの列のコード（[`PickKind::from_column`] の逆）。列にならない量は None
    pub fn column(self) -> Option<i64> {
        Some(match self {
            PickKind::Curv | PickKind::Radius => 1,
            PickKind::Thick => 2,
            PickKind::Conic => 3,
            PickKind::Semi => 4,
            PickKind::Parm(k) => 5 + k as i64,
            _ => return None,
        })
    }
}

/// ソルブの種類（.zmx の書き方は OpticStudio 2026 R1.01 で実測）。値は [`ThickSolve::height`]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SolveKind {
    /// 厚さ: 次の面での主波長の近軸マージナル光線の高さ = 値（MAZH 高さ 0）
    #[default]
    MarginalHeight,
    /// 厚さ: 次の面での実光線（軸上、瞳 (0, 帯)、主波長）の高さ = 値（MAZH 高さ 帯、帯 ≠ 0）
    MarginalHeightZone(f64),
    /// 厚さ: 次の面での近軸主光線の高さ = 値（CHZH 高さ 0）
    ChiefHeight,
    /// 厚さ: 軸上の視野・主波長の実光線（瞳 (0, 帯)）の像面での OPD [波] = 値（OPDZ OPD 帯）。OPD は
    /// [`crate::analysis::opd`]（OpticStudio の光線の OPD、OPDC と同じ）
    Opd(f64),
    /// 厚さ: 次の面を近軸の瞳（その空間で近軸主光線が軸を切る位置）に置く（TPUP）
    PupilPosition,
    /// 厚さ: 半径方向の高さ r（0 は面の半直径）でのコバ厚 = 値（EDGE コバ厚 r）
    Edge(f64),
    /// 厚さ: 面 s から次の面までの距離（厚さの和 Σ t[s..=この面]）= 値（TOLE s 長さ）。s が後ろなら、この面から
    /// 面 s までの距離 Σ t[この面..s] = 値
    Position(usize),
    /// 厚さ: 面 s の厚さ + この面の厚さ = 値（TCOM s 和）
    Compensator(usize),
    /// 厚さ: 次の面を面 s の曲率中心に置く（TCOC s 0。平面は R = 1e10）
    CenterOfCurvature(usize),
    /// 曲率: 面の後の近軸マージナル光線の傾き = 値（CURV c 2 値）
    MarginalAngle,
    /// 曲率: 面の後の近軸主光線の傾き = 値（CURV c 3 値）
    ChiefAngle,
    /// 曲率: 近軸マージナル光線が面に垂直に入る（CURV c 5）
    MarginalNormal,
    /// 曲率: 近軸主光線が面に垂直に入る（CURV c 6）
    ChiefNormal,
    /// 曲率: 近軸マージナル光線で不遊（u′/n′ = u/n、CURV c 7）
    Aplanatic,
    /// 曲率: この面を含む玉（後ろがガラスなら次の面と、そうでなければ前の面と）のパワー = 値（CURV c 8 値）
    ElementPower,
    /// 曲率: 曲率中心を面 s の頂点に置く（CURV c 9 s）
    ConcentricSurface(usize),
    /// 曲率: 曲率中心を面 s の曲率中心に置く（CURV c 10 s。平面は R = 1e10）
    ConcentricRadius(usize),
    /// 曲率: 面の後の近軸マージナル光線の傾き = −1/(2 F)（CURV c 11 F）
    FNumber,
    /// 曲率: ZPL のマクロ（[`System::macros`] の番号）の SOLVERETURN の値（CURV c 12 … "名前"）
    MacroCurv(u16),
    /// 厚さ: ZPL のマクロ（[`System::macros`] の番号）の SOLVERETURN の値
    MacroThick(u16),
}

impl SolveKind {
    /// 曲率のソルブ（ほかは厚さ）
    pub fn is_curv(&self) -> bool {
        matches!(
            self,
            SolveKind::MarginalAngle
                | SolveKind::ChiefAngle
                | SolveKind::MarginalNormal
                | SolveKind::ChiefNormal
                | SolveKind::Aplanatic
                | SolveKind::ElementPower
                | SolveKind::ConcentricSurface(_)
                | SolveKind::ConcentricRadius(_)
                | SolveKind::FNumber
                | SolveKind::MacroCurv(_)
        )
    }

    /// 決めるパラメータの種類
    pub fn param_kind(&self) -> Kind {
        if self.is_curv() {
            Kind::Curv
        } else {
            Kind::Thick
        }
    }

    /// 参照する別の面（あれば）
    pub fn other_surface(&self) -> Option<usize> {
        match *self {
            SolveKind::Position(s)
            | SolveKind::Compensator(s)
            | SolveKind::CenterOfCurvature(s)
            | SolveKind::ConcentricSurface(s)
            | SolveKind::ConcentricRadius(s) => Some(s),
            _ => None,
        }
    }

    /// 参照する別の面を付け替えたもの
    pub fn with_other_surface(self, s: usize) -> SolveKind {
        match self {
            SolveKind::Position(_) => SolveKind::Position(s),
            SolveKind::Compensator(_) => SolveKind::Compensator(s),
            SolveKind::CenterOfCurvature(_) => SolveKind::CenterOfCurvature(s),
            SolveKind::ConcentricSurface(_) => SolveKind::ConcentricSurface(s),
            SolveKind::ConcentricRadius(_) => SolveKind::ConcentricRadius(s),
            k => k,
        }
    }
}

/// ソルブ: 面 surf の厚さか曲率（[`SolveKind::is_curv`]）を、種類 kind の条件で決める。height はその値
/// （高さ・角度・コバ厚・長さ・和・パワー・F/#。使わない種類では 0）。名前は厚さのソルブだけだった頃のもの
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ThickSolve {
    pub surf: usize,
    pub height: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub kind: SolveKind,
}

/// ピックアップ: 面 dst の量 what = scale × 面 src の量 from（None なら what）+ offset。
///
/// 曲率は OpticStudio と同じく特別で（2026 R1.01 で実測）、同じ列（曲率・半径）からなら c = scale × c元、
/// 別の列の値 v からなら元の値を半径とみなして c = scale / v（オフセットなし）。ほかの量が半径の列から取るときは
/// 半径の値 R を使う（v = scale × R + offset）
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Pickup {
    pub dst: usize,
    pub src: usize,
    pub what: PickKind,
    pub scale: f64,
    pub offset: f64,
    /// 元の量（別の列からのピックアップ）。None は what と同じ
    #[cfg_attr(feature = "serde", serde(default))]
    pub from: Option<PickKind>,
}

impl Pickup {
    /// 同じ量どうしのピックアップ
    pub fn new(dst: usize, src: usize, what: PickKind, scale: f64, offset: f64) -> Pickup {
        Pickup {
            dst,
            src,
            what,
            scale,
            offset,
            from: None,
        }
    }

    /// 元の量
    pub fn source(&self) -> PickKind {
        self.from.unwrap_or(self.what)
    }

    /// 元の面 a から求めた、先の量の値（元の面にその量がなければ None）
    pub fn value(&self, a: &Surface) -> Option<f64> {
        let from = self.source();
        let v = pick_get(a, from)?;
        if self.what == PickKind::Curv && !matches!(from, PickKind::Curv | PickKind::Radius) {
            return Some(if v == 0.0 || !v.is_finite() {
                0.0
            } else {
                self.scale / v
            });
        }
        if self.what == PickKind::Curv && from == PickKind::Radius {
            return Some(self.scale * a.c);
        }
        Some(self.scale * v + self.offset)
    }

    /// 元の量（[`pick_kind`] の言い方: 半径は曲率）が x のとき、d(先の量)/dx
    fn slope(&self, a: &Surface) -> f64 {
        let from = self.source();
        if self.what == PickKind::Curv && !matches!(from, PickKind::Curv | PickKind::Radius) {
            let v = pick_get(a, from).unwrap_or(0.0);
            return if v == 0.0 { 0.0 } else { -self.scale / (v * v) };
        }
        if from == PickKind::Radius && self.what != PickKind::Curv {
            // R = 1/c: dR/dc = −1/c²
            return if a.c == 0.0 {
                0.0
            } else {
                -self.scale / (a.c * a.c)
            };
        }
        self.scale
    }

    /// 系 s でこのピックアップの関係がいまの値で成り立っているか（相対 1e-12。面がないか量がなければ false）
    pub fn holds(&self, s: &System) -> bool {
        let (Some(a), Some(b)) = (s.surfaces.get(self.src), s.surfaces.get(self.dst)) else {
            return false;
        };
        match (self.value(a), pick_get(b, self.what)) {
            (Some(v), Some(y)) => v == y || (v - y).abs() <= 1e-12 * (1.0 + v.abs().max(y.abs())),
            _ => false,
        }
    }
}

/// 面タイプのパラメータ i（.zmx の PARM の番号）の値。この面タイプで値を持たない番号は None
pub fn surf_parm(d: &Surface, i: u8) -> Option<f64> {
    let i = i as usize;
    let at =
        |v: &[f64], j: usize| -> Option<f64> { (j < 8).then(|| v.get(j).copied().unwrap_or(0.0)) };
    match d.typ {
        SurfType::EvenAsph | SurfType::Fresnel | SurfType::CylFresnel | SurfType::Binary2
            if (1..=8).contains(&i) =>
        {
            at(&d.asph, i - 1)
        }
        SurfType::Toroidal if i == 1 => Some(if d.cx == 0.0 { 0.0 } else { 1.0 / d.cx }),
        SurfType::Toroidal if (2..=8).contains(&i) => at(&d.asph, i - 2),
        SurfType::CoordBrk if (1..=5).contains(&i) => Some(d.cb[i - 1]),
        SurfType::CoordBrk if i == 6 => Some(d.cb_order as f64),
        SurfType::Paraxial if i == 1 => Some(d.focal),
        SurfType::Paraxial if i == 2 => Some(d.opd_mode as f64),
        SurfType::Jones if (1..=8).contains(&i) => Some(d.jones[i - 1]),
        SurfType::Gradium | SurfType::Gradient9 => {
            use crate::grin::GrinKind;
            let g = d.grin.as_deref()?;
            match (&g.kind, i) {
                (_, 1) => Some(g.dt),
                (GrinKind::Gradium { prof, .. }, 0) => Some(prof.zmax),
                (GrinKind::Gradium { dz, .. }, 2) => Some(*dz),
                (GrinKind::Gradium { .. }, 3) => g.ref_index(),
                (GrinKind::Gradium { dec, .. }, 4..=7) => Some(dec[i - 4]),
                (GrinKind::Gradium { cap, .. }, 8) => Some(*cap as f64),
                (GrinKind::Grad9 { tan, .. }, 2 | 3) => Some(tan[i - 2]),
                _ => None,
            }
        }
        _ => None,
    }
}

/// 面タイプのパラメータ i を書く。値を持たない番号なら false
pub(crate) fn set_surf_parm(d: &mut Surface, i: u8, v: f64) -> bool {
    let i = i as usize;
    let put = |a: &mut Vec<f64>, j: usize, n: usize| {
        if a.len() < n {
            a.resize(n, 0.0);
        }
        a[j] = v;
    };
    match d.typ {
        SurfType::EvenAsph | SurfType::Fresnel | SurfType::CylFresnel | SurfType::Binary2
            if (1..=8).contains(&i) =>
        {
            put(&mut d.asph, i - 1, 8)
        }
        SurfType::Toroidal if i == 1 => d.cx = if v == 0.0 { 0.0 } else { 1.0 / v },
        SurfType::Toroidal if (2..=8).contains(&i) => put(&mut d.asph, i - 2, 7),
        SurfType::CoordBrk if (1..=5).contains(&i) => d.cb[i - 1] = v,
        SurfType::CoordBrk if i == 6 => d.cb_order = v as u8,
        SurfType::Paraxial if i == 1 => d.focal = v,
        SurfType::Paraxial if i == 2 => d.opd_mode = v as i32,
        SurfType::Jones if (1..=8).contains(&i) => {
            d.jones[i - 1] = v;
            d.retarder = None;
        }
        // GRIN: Δt と Gradient 9 の形の傾きだけ（Δz などは頂点の屈折率 n も変わるので未対応）
        SurfType::Gradium | SurfType::Gradient9 => {
            use crate::grin::GrinKind;
            let Some(g) = d.grin.as_mut() else {
                return false;
            };
            let g = std::sync::Arc::make_mut(g);
            match (&mut g.kind, i) {
                (_, 1) => g.dt = v,
                (GrinKind::Grad9 { tan, .. }, 2 | 3) => tan[i - 2] = v,
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn pick_get(d: &Surface, what: PickKind) -> Option<f64> {
    match what {
        PickKind::Curv => Some(d.c),
        PickKind::Thick => Some(d.t),
        PickKind::Conic => Some(d.k),
        PickKind::Parm(i) => surf_parm(d, i),
        PickKind::Dn => Some(d.dn),
        PickKind::Disp => Some(d.disp),
        // 平面の半径は 1e10（OpticStudio が平面を R = 1e10 として扱うのに合わせる）
        PickKind::Radius => Some(if d.c == 0.0 { 1e10 } else { 1.0 / d.c }),
        PickKind::Semi => Some(d.semi),
        // 膜は名前を写すだけ（[`System::pick_names`]）。数の値は無いので一定の 0
        PickKind::Coating => Some(0.0),
        PickKind::Before(i) => Some(d.before.map(|t| t.d[i as usize]).unwrap_or(0.0)),
        PickKind::After(i) => match d.after {
            After::Tilt(t) => Some(t.d[i as usize]),
            // 面の後の変換がない = 値が 0
            After::None => Some(0.0),
            After::Reverse => None,
        },
    }
}

pub(crate) fn pick_set(d: &mut Surface, what: PickKind, v: f64) -> bool {
    match what {
        PickKind::Curv => d.c = v,
        PickKind::Thick => d.t = v,
        PickKind::Conic => d.k = v,
        PickKind::Parm(i) => return set_surf_parm(d, i, v),
        PickKind::Dn => d.dn = v,
        PickKind::Disp => d.disp = v,
        PickKind::Semi => {
            d.semi = v;
            if let crate::surface::Aperture::Floating { semi } = &mut d.ap {
                *semi = v;
            }
        }
        PickKind::After(i) => match &mut d.after {
            After::Tilt(t) => t.d[i as usize] = v,
            // 元の面に前の傾き偏心がない（値 0）。後で元に値が入ったら作る（反転なら順序も逆。前の傾き偏心を
            // パラメータで作ると順序 0 なので、反転は 1）
            After::None if v == 0.0 => {}
            After::None => {
                let mut t = Tilt {
                    d: [0.0; 5],
                    order: 0,
                };
                t.d[i as usize] = v;
                d.after = After::Tilt(t);
            }
            After::Reverse => return false,
        },
        PickKind::Before(i) => {
            if d.typ == SurfType::CoordBrk {
                return false;
            }
            match &mut d.before {
                Some(t) => t.d[i as usize] = v,
                // 元の値が 0 のうちは作らない（後で元に値が入ったら作る）
                None if v == 0.0 => {}
                // パラメータで値を入れるとき（System::set_params）と同じく、前の傾き偏心を作り、
                // 面の後がなければ「前を戻す」にする（その面だけが動く）
                None => {
                    let mut t = Tilt {
                        d: [0.0; 5],
                        order: 0,
                    };
                    t.d[i as usize] = v;
                    d.before = Some(t);
                    if d.after == After::None {
                        d.after = After::Reverse;
                    }
                }
            }
        }
        PickKind::Radius => return false,
        PickKind::Coating => {}
    }
    true
}

/// パラメータ p が動かす、ピックアップの量（なければ None）
pub(crate) fn pick_kind(d: &Surface, kind: Kind) -> Option<PickKind> {
    match kind {
        Kind::Curv | Kind::Radius => Some(PickKind::Curv),
        Kind::Thick => Some(PickKind::Thick),
        Kind::Conic => Some(PickKind::Conic),
        Kind::Dn => Some(PickKind::Dn),
        Kind::Disp => Some(PickKind::Disp),
        Kind::Asph(i) if d.typ == SurfType::EvenAsph => Some(PickKind::Parm(i + 1)),
        k if d.typ == SurfType::CoordBrk => k.tilt_index().map(|i| PickKind::Parm(i as u8 + 1)),
        k => k.tilt_index().map(|i| PickKind::Before(i as u8)),
    }
}

/// ピックアップの量を、面 d のパラメータの種類に戻す（Param で表せなければ None）
fn kind_of_pick(d: &Surface, what: PickKind) -> Option<Kind> {
    match what {
        PickKind::Curv => Some(Kind::Curv),
        PickKind::Thick => Some(Kind::Thick),
        PickKind::Conic => Some(Kind::Conic),
        PickKind::Dn => Some(Kind::Dn),
        PickKind::Disp => Some(Kind::Disp),
        PickKind::Radius | PickKind::Semi | PickKind::After(_) | PickKind::Coating => None,
        PickKind::Before(i) => [Kind::Dx, Kind::Dy, Kind::Tx, Kind::Ty, Kind::Tz]
            .get(i as usize)
            .copied(),
        PickKind::Parm(i) => match d.typ {
            SurfType::EvenAsph if (1..=8).contains(&i) => Some(Kind::Asph(i - 1)),
            SurfType::CoordBrk => [Kind::Dx, Kind::Dy, Kind::Tx, Kind::Ty, Kind::Tz]
                .get((i as usize).wrapping_sub(1))
                .copied(),
            _ => None,
        },
    }
}

/// 面 surf のパラメータ kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Param {
    pub surf: usize,
    pub kind: Kind,
}

impl Param {
    pub fn new(surf: usize, kind: Kind) -> Param {
        Param { surf, kind }
    }
    pub fn obj_x() -> Param {
        Param::new(0, Kind::ObjX)
    }
    pub fn obj_y() -> Param {
        Param::new(0, Kind::ObjY)
    }
    /// 玉・群 u のパラメータ
    pub fn unit(u: usize, kind: UnitKind) -> Param {
        Param::new(u, Kind::Unit(kind))
    }
    pub fn name(&self) -> String {
        if self.kind.is_global() {
            self.kind.name()
        } else if self.kind.is_unit() {
            format!("unit{}.{}", self.surf, self.kind.name())
        } else {
            format!("{}.{}", self.surf, self.kind.name())
        }
    }
}

impl System {
    /// 面を番号か名前（コメント、前後の空白と大文字小文字は区別しない）で探す。名前が複数の面にあれば不正
    pub fn find_surface(&self, target: &str) -> Result<usize> {
        let t = target.trim();
        if let Ok(j) = t.parse::<usize>() {
            if j >= self.surfaces.len() {
                invalid!("面 {j}（面は 0〜{}）", self.surfaces.len() - 1);
            }
            return Ok(j);
        }
        let hits: Vec<usize> = (0..self.surfaces.len())
            .filter(|&j| self.surfaces[j].comment.trim().eq_ignore_ascii_case(t))
            .collect();
        match hits.as_slice() {
            [j] => Ok(*j),
            [] => invalid!("名前 {t} の面がない"),
            _ => invalid!("名前 {t} の面が複数ある（{hits:?}）。番号で指定する"),
        }
    }

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
            invalid!("パラメータ {s}（面.種類 の形で、例 3.t）");
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
            invalid!("名前 {} の玉・群がない", t.trim());
        }
        if let Some(u) = unit {
            if force.is_none() && self.find_surface(t).is_ok() {
                invalid!(
                    "{} は面の名前でも玉・群の名前でもある（unit:{0}.{k} か surf:{0}.{k} で指す）",
                    t.trim()
                );
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
            Kind::Unit(k) => match self.units.get(p.surf) {
                Some(u) => format!("{}.{}", u.name, k.name()),
                None => p.name(),
            },
            _ => p.name(),
        }
    }

    /// パラメータがこの系で使えるか
    pub fn check_param(&self, p: Param) -> Result<()> {
        if p.kind.is_object() {
            return Ok(());
        }
        if let Kind::Wave(i) = p.kind {
            if i as usize >= self.wavelengths.len() {
                invalid!("{}（波長は {} 個）", p.kind.name(), self.wavelengths.len());
            }
            return Ok(());
        }
        if let Kind::Unit(k) = p.kind {
            let Some(u) = self.units.get(p.surf) else {
                invalid!("玉・群 {}（{} 個）", p.surf, self.units.len());
            };
            if matches!(k, UnitKind::Dn | UnitKind::Disp) && self.unit_media(p.surf).is_empty() {
                invalid!(
                    "玉・群 {} の中に媒質（ガラス）がない（{}）",
                    u.name,
                    k.name()
                );
            }
            return Ok(());
        }
        let j = p.surf;
        if j >= self.surfaces.len() {
            invalid!("面 {j}（面は 0〜{}）", self.surfaces.len() - 1);
        }
        let d = &self.surfaces[j];
        let curved = matches!(d.typ, SurfType::Standard | SurfType::EvenAsph)
            || crate::freeform::is_freeform(d.typ);
        match p.kind {
            Kind::Curv | Kind::Radius | Kind::Conic if j == 0 || !curved => {
                invalid!("面 {j}（{:?}）の {}", d.typ, p.kind.name());
            }
            Kind::Asph(i) => {
                if d.typ != SurfType::EvenAsph && !crate::freeform::is_freeform(d.typ) {
                    invalid!("面 {j} は偶数次非球面でない（{}）", p.kind.name());
                }
                if i >= 8 {
                    invalid!("非球面の係数 {}", p.kind.name());
                }
            }
            Kind::Zern(n) => match &d.zern {
                Some(z) if (n as usize) <= crate::freeform::ZernSag::max_terms(z.fringe) => {}
                Some(_) => invalid!("面 {j} の Zernike の項 {n} は上限を超える"),
                None => invalid!("面 {j} は Zernike Sag の面でない（{}）", p.kind.name()),
            },
            Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz if j == 0 => {
                notimpl!("物体面の傾き偏心");
            }
            Kind::Dn | Kind::Disp if d.mirror || d.typ == SurfType::CoordBrk => {
                invalid!(
                    "面 {j} は鏡か座標ブレーク（媒質は前の面の {}）",
                    p.kind.name()
                );
            }
            Kind::Thick if j == self.image() => invalid!("像面の厚さ"),
            _ => {}
        }
        Ok(())
    }

    /// パラメータの今の値
    pub fn get_param(&self, p: Param) -> Result<f64> {
        self.check_param(p)?;
        if p.kind.is_object() {
            invalid!("物点 {} は系のデータでない（追跡の入力）", p.kind.name());
        }
        if let Kind::Wave(i) = p.kind {
            return Ok(self.wavelengths[i as usize]);
        }
        if let Kind::Unit(k) = p.kind {
            let u = &self.units[p.surf];
            if let Some(i) = k.motion_index() {
                return Ok(u.motion[i]);
            }
            return Ok(match k {
                UnitKind::Shift => u.shift,
                _ => {
                    let v: Vec<f64> = self
                        .unit_media(p.surf)
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
                        invalid!(
                            "玉・群 {} の媒質の {} が面ごとに違う（{v:?}）",
                            u.name,
                            k.name()
                        );
                    }
                    v[0]
                }
            });
        }
        let d = &self.surfaces[p.surf];
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
            Kind::Asph(i) => d.asph.get(i as usize).copied().unwrap_or(0.0),
            Kind::Zern(n) => d
                .zern
                .as_ref()
                .and_then(|z| z.coef.get(n as usize - 1).copied())
                .unwrap_or(0.0),
            Kind::Thick => d.t,
            Kind::Dn => d.dn,
            Kind::Disp => d.disp,
            k => {
                let i = k.tilt_index().unwrap();
                if d.typ == SurfType::CoordBrk {
                    d.cb[i]
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
    pub(crate) fn with_params_fast(&self, changes: &[(Param, f64)]) -> Result<System> {
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
            let x = &mut s.units[u];
            match k.motion_index() {
                Some(i) => x.motion[i] = v,
                None => x.shift = v,
            }
        }
        // ピックアップで決まる先は、展開した値と合っているか確かめる（倍率・オフセットが玉・群の動きと合わない）
        for &(p, v) in &picked {
            let now = s.get_param(p)?;
            if (now - v).abs() > 1e-12 * (1.0 + v.abs()) {
                invalid!(
                    "{} はピックアップで {now} になり、玉・群の変更の {v} と合わない",
                    p.name()
                );
            }
        }
        // 傾き・偏心だけの変更で近軸量を残せて、ソルブが近軸量と面の形・厚さだけで決まり既に解けていれば、
        // 解き直さない（解き直すと座標系などのキャッシュを捨てる）
        let any = !s.solves.is_empty() || !s.macro_parms.is_empty();
        if solve && any && !(keep.paraxial && s.macro_parms.is_empty() && s.solves_still_hold()) {
            s.solve_thickness()?;
        }
        if flat.iter().any(|(p, _)| p.surf == 0) {
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
                    let d = v - self.units[p.surf].shift;
                    for (q, f) in self.unit_terms(p, false)? {
                        match shifted.iter_mut().find(|x| x.0 == q.surf) {
                            Some(x) => x.1 += f * d,
                            None => shifted.push((q.surf, f * d)),
                        }
                    }
                    shift_at.get_or_insert(flat.len());
                    motions.push((p.surf, k, v));
                }
                UnitKind::Dn | UnitKind::Disp => {
                    for (q, _) in self.unit_terms(p, false)? {
                        flat.push((q, v, true));
                    }
                }
                _ => motions.push((p.surf, k, v)),
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
                        q.surf == pk.src
                            && pick_kind(&self.surfaces[q.surf], q.kind) == Some(pk.what)
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
        let u = &self.units[p.surf];
        let mut out: Vec<(Param, f64)> = Vec::new();
        match k {
            UnitKind::Shift => {
                let mut add = |j: usize, f: f64| match out.iter_mut().find(|x| x.0.surf == j) {
                    Some(x) => x.1 += f,
                    None => out.push((Param::new(j, Kind::Thick), f)),
                };
                for s in &u.spans {
                    add(s[0] - 1, 1.0);
                    add(s[1], -1.0);
                }
                out.retain(|(q, f)| {
                    *f != 0.0
                        && self.surfaces[q.surf].t.is_finite()
                        && !self.solves.iter().any(|x| x.surf == q.surf)
                });
                if drop_picked {
                    let all: Vec<usize> = out.iter().map(|x| x.0.surf).collect();
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
                for j in self.unit_media(p.surf) {
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
                invalid!("物点 {} は系のデータでない（追跡の入力）", p.kind.name());
            }
            if !v.is_finite() && !(p.kind == Kind::Thick && p.surf == 0) {
                invalid!("{} = {v}", p.name());
            }
            if p.kind.is_wave() {
                if !(v > 0.0) {
                    invalid!("{} = {v} µm", p.name());
                }
                continue;
            }
            let solved = |k: Kind| {
                self.solves
                    .iter()
                    .find(|q| q.surf == p.surf && q.kind.param_kind() == k)
            };
            let q = match p.kind {
                Kind::Thick => solved(Kind::Thick),
                Kind::Curv | Kind::Radius => solved(Kind::Curv),
                _ => None,
            };
            if let Some(q) = q {
                invalid!("{} はソルブ（{:?}）で決まる", p.name(), q.kind);
            }
            if let Some(pk) = self.pickup_of(p) {
                invalid!(
                    "{} は面 {} からのピックアップ（倍率 {}、オフセット {}）。元の面を変える",
                    p.name(),
                    pk.src,
                    pk.scale,
                    pk.offset
                );
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
            if p.kind.is_wave() {
                continue;
            }
            let d = &mut self.surfaces[p.surf];
            match p.kind {
                Kind::Curv => d.c = v,
                Kind::Radius => d.c = if v == 0.0 { 0.0 } else { 1.0 / v },
                Kind::Conic => d.k = v,
                Kind::Asph(i) => {
                    let i = i as usize;
                    if d.asph.len() <= i {
                        d.asph.resize(8, 0.0);
                    }
                    d.asph[i] = v;
                }
                Kind::Zern(n) => {
                    let z = std::sync::Arc::make_mut(d.zern.as_mut().unwrap());
                    let i = n as usize - 1;
                    if z.coef.len() <= i {
                        z.coef.resize(i + 1, 0.0);
                    }
                    z.coef[i] = v;
                    z.rebuild();
                }
                Kind::Thick => d.t = v,
                Kind::Dn => {
                    d.dn = v;
                    index = true;
                }
                Kind::Disp => {
                    d.disp = v;
                    index = true;
                }
                k => {
                    let i = k.tilt_index().unwrap();
                    if d.typ == SurfType::CoordBrk {
                        d.cb[i] = v;
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
                        d.before.as_mut().unwrap().d[i] = v;
                    }
                }
            }
        }
        if !self.pickups.is_empty() {
            self.resolve_pickups()?;
        }
        if index {
            self.update_indices();
        }
        Ok(keep_for(changes))
    }

    /// パラメータ p がピックアップ先ならそのピックアップ
    pub fn pickup_of(&self, p: Param) -> Option<Pickup> {
        if p.kind.is_global() || p.kind.is_unit() || p.surf >= self.surfaces.len() {
            return None;
        }
        let what = pick_kind(&self.surfaces[p.surf], p.kind)?;
        self.pickups
            .iter()
            .find(|q| q.dst == p.surf && q.what == what)
            .copied()
    }

    /// 名前のピックアップを写す: 材料（[`PickKind::Dn`] のピックアップは元の面の材料・鏡も一緒に追従する。
    /// OpticStudio の材料のピックアップ）と膜（[`PickKind::Coating`]）。連なっていてもよい。屈折率は後で材料から作る
    pub(crate) fn pick_names(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for _ in 0..=ns {
            let mut changed = false;
            for i in 0..self.pickups.len() {
                let q = self.pickups[i];
                if q.src >= ns || q.dst >= ns {
                    invalid!("ピックアップの面（元 {}、先 {}）がない", q.src, q.dst);
                }
                match q.what {
                    PickKind::Dn => {
                        let a = &self.surfaces[q.src];
                        let (mat, mirror) = (a.mat.clone(), a.mirror);
                        let d = &mut self.surfaces[q.dst];
                        if d.typ == SurfType::CoordBrk
                            || (mirror && q.dst == 0)
                            || (d.mat == mat && d.mirror == mirror)
                        {
                            continue;
                        }
                        d.mat = mat;
                        d.mirror = mirror;
                        d.n.clear();
                        d.n_base.clear();
                        changed = true;
                    }
                    PickKind::Coating => {
                        let coat = self.surfaces[q.src].coat.clone();
                        let d = &mut self.surfaces[q.dst];
                        if d.coat != coat {
                            d.coat = coat;
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
            if !changed {
                return Ok(());
            }
        }
        invalid!("材料・膜のピックアップが収束しない（循環？）")
    }

    /// ピックアップを解決する（元の面が後ろにあっても、連なっていてもよい）。循環して収まらなければエラー
    pub fn resolve_pickups(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for q in &self.pickups {
            if q.src >= ns || q.dst >= ns {
                invalid!("ピックアップの面（元 {}、先 {}）がない", q.src, q.dst);
            }
        }
        for _ in 0..=ns {
            let mut changed = false;
            for i in 0..self.pickups.len() {
                let q = self.pickups[i];
                let Some(v) = q.value(&self.surfaces[q.src]) else {
                    invalid!("面 {} の {:?} はピックアップできない", q.src, q.source());
                };
                let d = &mut self.surfaces[q.dst];
                let before = pick_get(d, q.what);
                if before != Some(v) {
                    let created = matches!(q.what, PickKind::After(_)) && d.after == After::None;
                    let made_before = matches!(q.what, PickKind::Before(_)) && d.before.is_none();
                    let after_was = d.after;
                    let ok = pick_set(d, q.what, v);
                    // 前の傾き偏心を作ったとき、同じ面の後もピックアップの先なら「前を戻す」にしない（後はそのピックアップで決まる）
                    if made_before
                        && after_was == After::None
                        && self
                            .pickups
                            .iter()
                            .any(|k| k.dst == q.dst && matches!(k.what, PickKind::After(_)))
                    {
                        self.surfaces[q.dst].after = After::None;
                    }
                    if created {
                        // 作ったばかりの面の後の変換の順序は元の面の前のもの（反転なら逆）
                        let o = self.surfaces[q.src].before.map(|b| b.order).unwrap_or(0);
                        if let After::Tilt(t) = &mut self.surfaces[q.dst].after {
                            t.order = if q.scale < 0.0 { 1 - o.min(1) } else { o };
                        }
                    }
                    let d = &mut self.surfaces[q.dst];
                    if !ok {
                        invalid!("面 {} の {:?} にピックアップできない", q.dst, q.what);
                    }
                    // 書くと丸まる量（トロイダルの回転半径、整数の値）もあるので、書いた後の値で比べる
                    changed |= pick_get(d, q.what) != before;
                }
            }
            if !changed {
                return Ok(());
            }
        }
        invalid!("ピックアップが収束しない（循環？）")
    }

    /// パラメータ p を動かしたときに一緒に動くパラメータと、その倍率 d(先)/d(p)（p 自身を含む）。
    /// ピックアップ先の量が Param で表せない（トロイダルのパラメータなど）なら未対応
    pub(crate) fn pickup_chain(&self, p: Param) -> Result<Vec<(Param, f64)>> {
        if p.kind.is_unit() && !p.kind.is_unit_motion() {
            // 面のパラメータの組合せ（それぞれのピックアップの先も）
            let mut out = Vec::new();
            for (q, f) in self.unit_linear(p)? {
                for (r, g) in self.pickup_chain(q)? {
                    out.push((r, f * g));
                }
            }
            return Ok(out);
        }
        let mut out = vec![(p, 1.0)];
        if p.kind.is_global() || p.kind.is_unit() || self.pickups.is_empty() {
            return Ok(out);
        }
        // 半径を変えたときの曲率の変化 dc/dR = −c²
        let first = if p.kind == Kind::Radius {
            let c = self.surfaces[p.surf].c;
            (Param::new(p.surf, Kind::Curv), -c * c)
        } else {
            (p, 1.0)
        };
        let mut stack = vec![first];
        let mut depth = 0;
        while let Some((q, f)) = stack.pop() {
            depth += 1;
            if depth > 64 * self.surfaces.len() {
                invalid!("ピックアップが循環している");
            }
            let Some(what) = pick_kind(&self.surfaces[q.surf], q.kind) else {
                continue;
            };
            for pk in self.pickups.iter().filter(|k| {
                let from = match k.source() {
                    PickKind::Radius => PickKind::Curv,
                    w => w,
                };
                k.src == q.surf && from == what
            }) {
                let d = &self.surfaces[pk.dst];
                let Some(kind) = kind_of_pick(d, pk.what) else {
                    notimpl!("面 {} の {:?}（ピックアップ先）の微分", pk.dst, pk.what);
                };
                let r = (
                    Param::new(pk.dst, kind),
                    f * pk.slope(&self.surfaces[pk.src]),
                );
                out.push(r);
                stack.push(r);
            }
        }
        Ok(out)
    }

    /// 屈折率のずれ（dn・disp）を n に反映する。鏡・座標ブレークの面は前の媒質を引き継ぐ
    pub fn update_indices(&mut self) {
        let wp = self.primary;
        let mut nprev: Option<Vec<f64>> = None;
        // 直前の媒質を今回書き換えたか（その後の鏡・座標ブレークにだけ写す）
        let mut touched = false;
        for j in 0..self.surfaces.len() {
            let d = &mut self.surfaces[j];
            if j > 0 && (d.mirror || d.typ == SurfType::CoordBrk) {
                if touched {
                    if let Some(n) = &nprev {
                        d.n = n.clone();
                    }
                }
            } else {
                let has = d.dn != 0.0 || d.disp != 0.0;
                if has || !d.n_base.is_empty() {
                    if d.n_base.is_empty() {
                        d.n_base = d.n.clone();
                    }
                    let b = &d.n_base;
                    d.n = if has {
                        let bp = b[wp];
                        b.iter().map(|&x| x + d.dn + d.disp * (x - bp)).collect()
                    } else {
                        b.clone()
                    };
                    if !has {
                        d.n_base.clear();
                    }
                    if !d.ignore {
                        touched = true;
                    }
                } else if !d.ignore {
                    touched = false;
                }
            }
            if !d.ignore {
                nprev = Some(d.n.clone());
            }
        }
    }

    /// 屈折率のずれをかける前の媒質の屈折率（面 j の後、波長 w）
    pub fn base_index(&self, j: usize, w: usize) -> f64 {
        let d = &self.surfaces[j];
        if d.n_base.is_empty() {
            d.n[w]
        } else {
            d.n_base[w]
        }
    }
}

/// 変えるパラメータから、残してよいキャッシュを決める
pub(crate) fn keep_for(changes: &[(Param, f64)]) -> Keep {
    // 玉・群の剛体の動きは傾き偏心と同じ（座標系だけ）。移動は厚さ、dn・disp は屈折率
    let tilt = |k: Kind| k.tilt_index().is_some() || k.is_unit_motion();
    let geom = changes.iter().any(|(p, _)| {
        p.kind == Kind::Thick || p.kind == Kind::Unit(UnitKind::Shift) || tilt(p.kind)
    });
    let only_tilt = changes.iter().all(|(p, _)| tilt(p.kind));
    Keep {
        frames: !geom,
        paraxial: only_tilt,
    }
}

impl System {
    /// ソルブ（[`ThickSolve`]: 厚さ・曲率）を解き直す。読込と、パラメータを変えた後に呼ぶ。
    /// 近軸の量は主波長で、無視した面は飛ばす（近軸の追跡と同じ）。面の順に解き、どれかを変えると前のソルブの
    /// 条件も変わりうる（入射瞳・F/# の絞りなど）ので、全部が収まるまで繰り返す。ピックアップは途中で解決し直す。
    /// 既に解けている値（相対 1e-14）は変えない（読み戻し・関係ないパラメータの変更で同じ系になるように）
    pub fn solve_thickness(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for q in &self.solves {
            let what = if q.kind.is_curv() {
                PickKind::Curv
            } else {
                PickKind::Thick
            };
            if q.surf == 0 || q.surf + 1 >= ns {
                invalid!("面 {}: 物体面・像面（またはない面）のソルブ", q.surf);
            }
            if let Some(o) = q.kind.other_surface() {
                if o >= ns {
                    invalid!("面 {}: ソルブの基準の面 {o} がない", q.surf);
                }
            }
            if self
                .pickups
                .iter()
                .any(|p| p.dst == q.surf && p.what == what)
            {
                invalid!("面 {}: ソルブとピックアップの両方（{:?}）", q.surf, what);
            }
            if q.kind.is_curv() && self.surfaces[q.surf].typ == SurfType::CoordBrk {
                invalid!("面 {}: 座標ブレークの曲率のソルブ", q.surf);
            }
            if q.kind == SolveKind::Position(0) {
                invalid!("面 {}: 位置ソルブの基準が物体面", q.surf);
            }
        }
        let solves: Vec<ThickSolve> = self
            .solves
            .iter()
            .filter(|q| !self.surfaces[q.surf].ignore)
            .copied()
            .collect();
        if solves.is_empty() && self.macro_parms.is_empty() {
            return Ok(());
        }
        for _ in 0..100 {
            let mut moved = false;
            for q in &solves {
                moved |= self.solve_one(q)?;
            }
            if !self.macro_parms.is_empty() {
                moved |= self.solve_macro_parms()?;
            }
            if !moved {
                return Ok(());
            }
        }
        invalid!("ソルブが収束しない")
    }

    /// 解くソルブ（無視する面のものを除く、解く順）
    pub(crate) fn active_solves(&self) -> Vec<ThickSolve> {
        self.solves
            .iter()
            .filter(|q| !self.surfaces[q.surf].ignore)
            .copied()
            .collect()
    }

    /// 傾き・偏心だけを変えた系（近軸量は同じ）で、ソルブがすべて解けたままか。実光線を使うもの・軸上の距離を
    /// 使うもの（傾き偏心をまたぐと解けない）は false。閉じた形のものはビットまで同じ値、光線の高さのものは
    /// 解きの許容差の中
    fn solves_still_hold(&self) -> bool {
        self.active_solves().iter().all(|q| {
            if matches!(
                q.kind,
                SolveKind::MarginalHeightZone(_)
                    | SolveKind::Opd(_)
                    | SolveKind::CenterOfCurvature(_)
                    | SolveKind::ConcentricSurface(_)
                    | SolveKind::ConcentricRadius(_)
                    | SolveKind::MacroCurv(_)
                    | SolveKind::MacroThick(_)
            ) {
                return false;
            }
            let v = self.solved_value(q);
            match q.kind {
                SolveKind::MarginalHeight | SolveKind::ChiefHeight | SolveKind::PupilPosition => {
                    self.paraxial_height_residual(q)
                        .is_ok_and(|(r, du)| (r / du).abs() <= 1e-13 * (1.0 + v.abs()))
                }
                _ => self.closed_value(q).is_ok_and(|c| c == v),
            }
        })
    }

    /// ソルブで決まる量（[`System::active_solves`] の順）の、パラメータ ps それぞれへの微分 ds/dp
    /// （[パラメータ][ソルブ]）。陰関数の式 ds/dp = −(∂R/∂s)⁻¹ ∂R/∂p で求める（R はソルブの条件の残差
    /// [`System::solve_residual`]）。偏微分は、ソルブを解き直さない系の R の中心差分（R は滑らかなので、
    /// 解き直した系の差分と違ってソルブの許容差の分の誤差が乗らない。解き直しの反復もない）。
    /// ps はソルブで決まる量・ピックアップの先・玉や群・物点でない面のパラメータ
    pub(crate) fn solve_sensitivity(&self, ps: &[Param]) -> Result<Vec<Vec<f64>>> {
        let qs = self.active_solves();
        let ns = qs.len();
        if ns == 0 {
            return Ok(vec![Vec::new(); ps.len()]);
        }
        let resid = |s: &System| -> Result<Vec<f64>> {
            let r: Vec<f64> = qs
                .iter()
                .map(|q| s.solve_residual(q))
                .collect::<Result<_>>()?;
            if r.iter().any(|x| !x.is_finite()) {
                invalid!("ソルブの残差が有限でない");
            }
            Ok(r)
        };
        let step = |v: f64| 1e-6 * (1.0 + v.abs());
        // ∂R/∂s（列 j がソルブ j の量への微分）。ソルブの量を直接書き換えた系で
        let mut js = vec![0.0; ns * ns];
        for (j, q) in qs.iter().enumerate() {
            let v = self.solved_value(q);
            let h = step(v);
            let mut r = [Vec::new(), Vec::new()];
            for (i, x) in [v + h, v - h].into_iter().enumerate() {
                let mut s = self.clone_keep(Keep {
                    frames: false,
                    paraxial: false,
                });
                s.put_solved(q.surf, q.kind.is_curv(), x)?;
                r[i] = resid(&s)?;
            }
            for i in 0..ns {
                js[i * ns + j] = (r[0][i] - r[1][i]) / (2.0 * h);
            }
        }
        let lu = Lu::new(js, ns).ok_or_else(|| {
            crate::error::Error::Invalid(
                "ソルブの条件がソルブの量で変わらない（微分が求まらない）".into(),
            )
        })?;
        ps.iter()
            .map(|&p| {
                let v = self.get_param(p)?;
                let h = step(v);
                let a = resid(&self.with_params_solve(&[(p, v + h)], false)?)?;
                let b = resid(&self.with_params_solve(&[(p, v - h)], false)?)?;
                let jp: Vec<f64> = a
                    .iter()
                    .zip(&b)
                    .map(|(x, y)| -(x - y) / (2.0 * h))
                    .collect();
                Ok(lu.solve(&jp))
            })
            .collect()
    }

    /// 近軸主光線（[`crate::system::Paraxial::chief`]、物体空間の傾き 1 か物体の高さ 1）を最大視野（正規化視野 1）
    /// の主光線にする倍率
    fn chief_scale(&self) -> Result<f64> {
        let (_, ny) = self.field_norm_values();
        Ok(match self.field_type {
            crate::system::FieldType::Angle => ny.to_radians().tan(),
            crate::system::FieldType::ObjH => ny,
            crate::system::FieldType::ParH => ny / self.paraxial().chief_parimage_per_unit,
            t => notimpl!("視野の種類 {t:?} での主光線のソルブ"),
        })
    }

    /// 値 v を面 k の厚さか曲率に入れて系を更新する（ピックアップも）
    fn put_solved(&mut self, k: usize, curv: bool, v: f64) -> Result<()> {
        if curv {
            self.surfaces[k].c = v;
        } else {
            self.surfaces[k].t = v;
        }
        if !self.pickups.is_empty() {
            self.resolve_pickups()?;
        }
        self.changed()
    }

    /// 面 k の前の、無視しない面の番号（なければ 0）
    fn prev_live(&self, k: usize) -> usize {
        (1..k)
            .rev()
            .find(|&j| !self.surfaces[j].ignore)
            .unwrap_or(0)
    }

    /// 面 a から面 b（a ≤ b）の頂点までの軸上の距離（無視しない面の厚さの和。鏡・傾き偏心をまたぐものは未対応）
    fn axial(&self, a: usize, b: usize) -> Result<f64> {
        let mut z = 0.0;
        for j in a..b {
            let d = &self.surfaces[j];
            if d.ignore {
                continue;
            }
            let tilted = (d.typ == SurfType::CoordBrk && d.cb.iter().any(|&v| v != 0.0))
                || d.before.is_some()
                || matches!(d.after, After::Tilt(_));
            if d.mirror || tilted {
                notimpl!("鏡・傾き偏心（面 {j}）をまたぐソルブの距離");
            }
            z += d.t;
        }
        Ok(z)
    }

    /// ソルブ 1 つを解く。値を変えたら true
    fn solve_one(&mut self, q: &ThickSolve) -> Result<bool> {
        let k = q.surf;
        let curv = q.kind.is_curv();
        match q.kind {
            SolveKind::MarginalHeight | SolveKind::ChiefHeight | SolveKind::PupilPosition => {
                return self.solve_paraxial_height(q);
            }
            SolveKind::MarginalHeightZone(z) => return self.solve_real_height(k, q.height, z),
            SolveKind::Opd(z) => return self.solve_opd(k, q.height, z),
            _ => {}
        }
        let old = self.solved_value(q);
        let tol = |v: f64| 1e-14 * (1.0 + v.abs());
        let v = self.closed_value(q)?;
        // 前の値に関わらず同じ値にする（パラメータを変えて戻した系が、読み直した系とビット一致するように）
        if v != old {
            self.put_solved(k, curv, v)?;
        }
        Ok((v - old).abs() > tol(v))
    }

    /// ソルブ q で決まる量（厚さか曲率）の今の値
    fn solved_value(&self, q: &ThickSolve) -> f64 {
        let d = &self.surfaces[q.surf];
        if q.kind.is_curv() {
            d.c
        } else {
            d.t
        }
    }

    /// ソルブ q の条件の残差（解けていれば 0）。閉じた形のソルブは「今の値 − 解いた値」、光線の高さのソルブは
    /// 「次の面での高さ − 目標」。どちらも系を書き換えずに求める（[`System::solve_sensitivity`] で使う）
    pub(crate) fn solve_residual(&self, q: &ThickSolve) -> Result<f64> {
        Ok(match q.kind {
            SolveKind::MarginalHeight | SolveKind::ChiefHeight | SolveKind::PupilPosition => {
                self.paraxial_height_residual(q)?.0
            }
            SolveKind::MarginalHeightZone(z) => self.real_height_residual(q.surf, q.height, z)?,
            SolveKind::Opd(z) => self.opd_residual(q.height, z)?,
            _ => self.solved_value(q) - self.closed_value(q)?,
        })
    }

    /// 閉じた形で決まるソルブの値（光線の高さのソルブ以外）。系の今の値から求め、系は書き換えない
    fn closed_value(&self, q: &ThickSolve) -> Result<f64> {
        let k = q.surf;
        let h = q.height;
        let radius = |c: f64| if c == 0.0 { 1e10 } else { 1.0 / c };
        let closed: Option<f64> = match q.kind {
            SolveKind::MarginalHeight
            | SolveKind::ChiefHeight
            | SolveKind::PupilPosition
            | SolveKind::MarginalHeightZone(_)
            | SolveKind::Opd(_) => unreachable!(),
            SolveKind::Edge(r) => {
                let d = &self.surfaces[k];
                let n = &self.surfaces[k + 1];
                for (j, x) in [(k, d), (k + 1, n)] {
                    if !matches!(
                        x.typ,
                        SurfType::Standard | SurfType::EvenAsph | SurfType::OddAsph
                    ) {
                        notimpl!("面 {j}（{:?}）のコバ厚のソルブ", x.typ);
                    }
                }
                let r = if r == 0.0 { d.semi } else { r };
                let r2 = r * r;
                // Odd Asphere は回転対称なので y 軸の上の矢高で測る
                let sag_r = |x: &crate::surface::Surface| match x.typ {
                    SurfType::OddAsph => crate::freeform::sag_xy(x, 0.0, r).0,
                    _ => crate::trace::sag(x.c, x.k, &x.asph, r2).0,
                };
                let z1 = sag_r(d);
                let z2 = sag_r(n);
                Some(h + z1 - z2)
            }
            SolveKind::Position(f) => {
                // 基準の面からこの面の手前までの厚さの和（基準の面が後ろなら、次の面から基準の面の手前までの和。
                // ピックアップで決まる厚さも含む）
                let rest = if f <= k { f..k } else { k + 1..f };
                let t = h - self.surfaces[rest].iter().map(|d| d.t).sum::<f64>();
                if !t.is_finite() {
                    invalid!("面 {k}: 位置ソルブの厚さが有限でない（面 {f} からの和）");
                }
                Some(t)
            }
            SolveKind::Compensator(s0) => {
                if s0 == k {
                    invalid!("面 {k}: 自分を基準にした補償のソルブ");
                }
                Some(h - self.surfaces[s0].t)
            }
            SolveKind::CenterOfCurvature(s0) => {
                if s0 > k {
                    notimpl!("面 {k}: 後ろの面 {s0} の曲率中心のソルブ");
                }
                Some(radius(self.surfaces[s0].c) - self.axial(s0, k)?)
            }
            SolveKind::ConcentricSurface(j) => {
                let z = if j >= k {
                    self.axial(k, j)?
                } else {
                    -self.axial(j, k)?
                };
                Some(if z == 0.0 { 0.0 } else { 1.0 / z })
            }
            SolveKind::ConcentricRadius(j) => {
                let z = if j >= k {
                    self.axial(k, j)?
                } else {
                    -self.axial(j, k)?
                };
                // 平面の基準は R = −1e10（OpticStudio 2026 R1.01 の値 c = −1e-10 に合わせる。曲率中心のソルブは +1e10）
                let cj = self.surfaces[j].c;
                let r = z + if cj == 0.0 { -1e10 } else { 1.0 / cj };
                Some(if r == 0.0 { 0.0 } else { 1.0 / r })
            }
            SolveKind::ElementPower => Some(self.element_power_curv(k, h)?),
            SolveKind::MacroCurv(i) | SolveKind::MacroThick(i) => {
                let Some(m) = self.macros.get(i as usize) else {
                    invalid!("面 {k}: マクロ {i} がない（全 {}）", self.macros.len());
                };
                // 値を入れずに止まったマクロ（OpticStudio は値を変えない）は今の値のまま
                Some(self.run_macro(m, k)?.unwrap_or(self.solved_value(q)))
            }
            _ => Some(self.paraxial_curv(q)?),
        };
        let v = closed.unwrap();
        if !v.is_finite() {
            invalid!("面 {k}: ソルブ {:?} が解けない（{v}）", q.kind);
        }
        Ok(v)
    }

    /// 近軸光線の傾き・垂直・不遊・F/# の曲率のソルブ
    fn paraxial_curv(&self, q: &ThickSolve) -> Result<f64> {
        let k = q.surf;
        let d = &self.surfaces[k];
        if d.mirror || self.surfaces[1..k].iter().any(|x| x.mirror && !x.ignore) {
            notimpl!("面 {k}: 鏡・鏡の後の面の光線の曲率のソルブ");
        }
        if !matches!(
            d.typ,
            SurfType::Standard | SurfType::EvenAsph | SurfType::Toroidal
        ) {
            notimpl!("面 {k}（{:?}）の曲率のソルブ", d.typ);
        }
        let pw = self.primary;
        let par = self.paraxial();
        let chief = matches!(q.kind, SolveKind::ChiefAngle | SolveKind::ChiefNormal);
        let (y, u) = if chief { &par.chief } else { &par.marginal };
        // 主光線は最大視野（正規化視野 1）のもの（OpticStudio と同じ）
        let f = if chief { self.chief_scale()? } else { 1.0 };
        let (y, u) = (f * y[k], f * u[self.prev_live(k)]);
        let n = self.index_before(k, pw);
        let n2 = d.n[pw];
        // 非球面の 2 次の項（r^2）は近軸の曲率に入る（近軸の追跡と同じ）
        let a1 = if d.typ == SurfType::EvenAsph {
            2.0 * d.asph.first().copied().unwrap_or(0.0)
        } else {
            0.0
        };
        if y == 0.0 {
            invalid!("面 {k}: 光線の高さが 0 で曲率のソルブが解けない");
        }
        let ce = match q.kind {
            // n′u′ = n u − y c (n′ − n)
            SolveKind::MarginalAngle | SolveKind::ChiefAngle | SolveKind::FNumber => {
                if n2 == n {
                    invalid!("面 {k}: 前後の屈折率が同じで光線の傾きのソルブが解けない");
                }
                let a = if q.kind == SolveKind::FNumber {
                    if q.height == 0.0 {
                        invalid!("面 {k}: F/# 0 のソルブ");
                    }
                    -1.0 / (2.0 * q.height)
                } else {
                    q.height
                };
                (n * u - n2 * a) / (y * (n2 - n))
            }
            SolveKind::MarginalNormal | SolveKind::ChiefNormal => -u / y,
            SolveKind::Aplanatic => -u * (n2 + n) / (n * y),
            _ => unreachable!(),
        };
        Ok(ce - a1)
    }

    /// 玉のパワーの曲率のソルブ: 前の面と面 k の組（厚肉レンズ）のパワーを p にする（OpticStudio 2026 R1.01 で実測:
    /// 面 k の後ろがガラスでも前の面と組む）
    fn element_power_curv(&self, k: usize, p: f64) -> Result<f64> {
        let pw = self.primary;
        let (a, b) = (self.prev_live(k), k);
        if a == 0 {
            invalid!("面 {k}: 玉のパワーのソルブの組（面 {a}・{b}）");
        }
        for j in [a, b] {
            let d = &self.surfaces[j];
            if d.mirror || !matches!(d.typ, SurfType::Standard | SurfType::EvenAsph) {
                notimpl!("面 {j}（{:?}）を含む玉のパワーのソルブ", d.typ);
            }
        }
        let n0 = self.index_before(a, pw);
        let ng = self.surfaces[a].n[pw];
        let n1 = self.surfaces[b].n[pw];
        let t = self.axial(a, b)?;
        let ce = |j: usize| {
            let d = &self.surfaces[j];
            let a1 = if d.typ == SurfType::EvenAsph {
                2.0 * d.asph.first().copied().unwrap_or(0.0)
            } else {
                0.0
            };
            (d.c + a1, a1)
        };
        // φ = φa + φb − (t/ng) φa φb、φa = ca (ng − n0)、φb = cb (n1 − ng)
        let (fa, fb) = (ng - n0, n1 - ng);
        let c = if k == a {
            let (cb, _) = ce(b);
            let pb = cb * fb;
            let den = fa * (1.0 - t / ng * pb);
            (p - pb) / den
        } else {
            let (ca, _) = ce(a);
            let pa = ca * fa;
            let den = fb * (1.0 - t / ng * pa);
            (p - pa) / den
        };
        Ok(c - ce(k).1)
    }

    /// 近軸マージナル光線・主光線の高さ（瞳の位置は主光線の高さ 0）の厚さのソルブ。ソルブの面より後ろに絞りがあると
    /// 入射瞳が動いて光線もその厚さで変わるので、割線法で解く
    fn solve_paraxial_height(&mut self, q: &ThickSolve) -> Result<bool> {
        let k = q.surf;
        let chief = q.kind != SolveKind::MarginalHeight;
        if q.kind == SolveKind::PupilPosition && k < self.stop() {
            // OpticStudio は絞りより前の面では厚さを 0 にする（意味のある位置がない）
            notimpl!("面 {k}: 絞りより前の面の瞳の位置のソルブ");
        }
        // 面 k の厚さを t にした系で、次の面での光線の高さ − 目標と、高さの t への傾き（t を固定した近似）
        let eval = |s: &mut System, t: f64| -> Result<(f64, f64)> {
            s.put_solved(k, false, t)?;
            s.paraxial_height_residual(q)
        };
        let what = if chief {
            "主光線"
        } else {
            "マージナル光線"
        };
        // 前の値に関わらず厚さ 0 から解く（解きの値が前の値の最後の桁で変わらないように）
        let old = self.surfaces[k].t;
        let (f0, du) = eval(self, 0.0)?;
        if !(du.abs() > 0.0) || !f0.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("面 {k}: {what}が光軸に平行で厚さのソルブが解けない");
        }
        let t = self.secant(k, 0.0, f0, -f0 / du, |s, t| Ok(eval(s, t)?.0))?;
        self.finish_thick(k, old, t)
    }

    /// 近軸光線の高さ・瞳の位置の厚さのソルブの、今の厚さでの次の面での高さ − 目標と、高さの厚さへの傾き
    /// （面 k での高さと傾きを固定した値）
    fn paraxial_height_residual(&self, q: &ThickSolve) -> Result<(f64, f64)> {
        let k = q.surf;
        let (chief, h) = match q.kind {
            SolveKind::MarginalHeight => (false, q.height),
            SolveKind::ChiefHeight => (true, q.height),
            _ => (true, 0.0),
        };
        let t = self.surfaces[k].t;
        let par = self.paraxial();
        let (y, u) = if chief { &par.chief } else { &par.marginal };
        let (y, u) = (y[k], u[k]);
        // 主光線は最大視野（正規化視野 1）のもの
        let f = if chief && h != 0.0 {
            self.chief_scale()?
        } else {
            1.0
        };
        // 鏡のあとは展開した向き（近軸の追跡と同じく、厚さは sgn·t で進む）
        let nmir = self.surfaces[1..=k]
            .iter()
            .filter(|d| d.mirror && !d.ignore)
            .count();
        let du = f * if nmir % 2 == 1 { -u } else { u };
        Ok((f * y + du * t - h, du))
    }

    /// 実光線（軸上、瞳 (0, 帯)）の次の面での高さの厚さのソルブ
    fn solve_real_height(&mut self, k: usize, h: f64, zone: f64) -> Result<bool> {
        let (d, n) = (&self.surfaces[k], &self.surfaces[k + 1]);
        if d.mirror
            || d.typ == SurfType::CoordBrk
            || d.after != After::None
            || n.before.is_some()
            || n.typ == SurfType::CoordBrk
            || n.ignore
        {
            notimpl!(
                "面 {k}: 鏡・傾き偏心・座標ブレーク・無視する面の隣の実光線の高さの厚さのソルブ"
            );
        }
        let eval = |s: &mut System, t: f64| -> Result<f64> {
            s.put_solved(k, false, t)?;
            s.real_height_residual(k, h, zone)
        };
        // 前の値に関わらず厚さ 0 から解く（高さは面 k の後の光線を延ばして測るので、次の面の位置によらない）
        let old = self.surfaces[k].t;
        let t0 = 0.0;
        let f0 = eval(self, t0)?;
        if !f0.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("面 {k}: 実光線（瞳の帯 {zone}）が通らず厚さのソルブが解けない");
        }
        // 初めの傾きは差分で
        let dt = 1e-6;
        let f1 = eval(self, t0 + dt)?;
        let slope = (f1 - f0) / dt;
        if !(slope.abs() > 0.0) {
            self.put_solved(k, false, old)?;
            invalid!("面 {k}: 実光線の高さが厚さで変わらず厚さのソルブが解けない");
        }
        let t = self.secant(k, t0, f0, t0 - f0 / slope, eval)?;
        self.finish_thick(k, old, t)
    }

    /// 実光線の高さの厚さのソルブの、今の厚さでの次の面での高さ − 目標（光線が通らなければ NaN）
    fn real_height_residual(&self, k: usize, h: f64, zone: f64) -> Result<f64> {
        let t = self.surfaces[k].t;
        // 高さは次の面の頂点の接平面で測る（OpticStudio 2026 R1.01 の値に合う）: 面 k の後の光線を延ばす
        let tr = self.try_trace(
            crate::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[zone],
            None,
            Some(k),
            false,
        )?;
        let r = &tr.rays[0];
        if r.err != 0 || r.lmn[2] == 0.0 {
            return Ok(f64::NAN);
        }
        Ok(r.xyz[1] + r.lmn[1] / r.lmn[2] * (t - r.xyz[2]) - h)
    }

    /// OPD の厚さのソルブ: 像面での OPD [波]（軸上の視野、主波長、瞳 (0, 帯)）を opd にする。OPD は厚さに
    /// 対して単調とは限らない（根が複数ありうる）ので、今の厚さから割線法で解く。
    ///
    /// ほかのソルブ（像面の焦点の MAZH など）は厚さを変えるたびに解き直してから OPD を測る（全部のソルブを同時に
    /// 満たす値。ほかのソルブを止めたままでは解がないことがある: 焦点ソルブの前の面の OPD のソルブ）。
    /// OPD のソルブどうしは外の繰り返し（[`System::solve_thickness`]）で揃える
    fn solve_opd(&mut self, k: usize, opd: f64, zone: f64) -> Result<bool> {
        if self.surfaces[k].mirror || self.surfaces[k].typ == SurfType::CoordBrk {
            notimpl!("面 {k}: 鏡・座標ブレークの OPD の厚さのソルブ");
        }
        if k + 1 >= self.surfaces.len() {
            invalid!("面 {k}: 像面の OPD の厚さのソルブ");
        }
        let others: Vec<ThickSolve> = self
            .active_solves()
            .into_iter()
            .filter(|q| !matches!(q.kind, SolveKind::Opd(_)))
            .collect();
        let eval = |s: &mut System, t: f64| -> Result<f64> {
            s.put_solved(k, false, t)?;
            if !others.is_empty() || !s.macro_parms.is_empty() {
                let mut done = false;
                for _ in 0..100 {
                    let mut moved = false;
                    for q in &others {
                        moved |= s.solve_one(q)?;
                    }
                    if !s.macro_parms.is_empty() {
                        moved |= s.solve_macro_parms()?;
                    }
                    if !moved {
                        done = true;
                        break;
                    }
                }
                if !done {
                    invalid!("面 {k}: OPD の厚さのソルブの中でほかのソルブが収束しない");
                }
            }
            s.opd_residual(opd, zone)
        };
        let old = self.surfaces[k].t;
        let f0 = eval(self, old)?;
        if !f0.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("面 {k}: 実光線（瞳の帯 {zone}）が通らず OPD の厚さのソルブが解けない");
        }
        // OPD は光路長の差なので丸めの揺れ（1e-11 波ほど）がある。揺れの中なら解けているとして動かさない
        // （動かすと外の繰り返しが揺れで止まらない）
        if f0.abs() <= OPD_SOLVE_TOL {
            return Ok(false);
        }
        let dt = 1e-6 * (1.0 + old.abs());
        let f1 = eval(self, old + dt)?;
        let slope = (f1 - f0) / dt;
        if !(slope.abs() > 0.0) || !slope.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("面 {k}: OPD が厚さで変わらず OPD の厚さのソルブが解けない");
        }
        let t = self.secant(k, old, f0, old - f0 / slope, eval)?;
        self.finish_thick(k, old, t)
    }

    /// OPD の厚さのソルブの、今の系での像面の OPD [波] − 目標（光線が通らなければ NaN）
    fn opd_residual(&self, opd: f64, zone: f64) -> Result<f64> {
        let o = crate::analysis::opd(
            self,
            crate::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[zone],
            None,
            true,
        )?;
        if !o.ok(0) {
            return Ok(f64::NAN);
        }
        Ok(o.opd[0] - opd)
    }

    /// 解いた厚さ t を入れる。前の値 old から許容より大きく変わったら true
    fn finish_thick(&mut self, k: usize, old: f64, t: f64) -> Result<bool> {
        self.put_solved(k, false, t)?;
        Ok((t - old).abs() > 1e-14 * (1.0 + t.abs()))
    }

    /// 厚さの割線法（t0 で f0、次の候補 t1）。根を返す（系の厚さは途中の値のまま）
    fn secant(
        &mut self,
        k: usize,
        t0: f64,
        f0: f64,
        t1: f64,
        mut eval: impl FnMut(&mut System, f64) -> Result<f64>,
    ) -> Result<f64> {
        let tol = |t: f64| 1e-14 * (1.0 + t.abs());
        let (mut t0, mut f0, mut t1) = (t0, f0, t1);
        let mut done = false;
        for _ in 0..100 {
            if (t1 - t0).abs() <= tol(t1) {
                done = true;
                break;
            }
            let f1 = eval(self, t1)?;
            if f1 == 0.0 || f1 == f0 {
                done = f1 == 0.0 || (t1 - t0).abs() <= 1e-9 * (1.0 + t1.abs());
                break;
            }
            let t2 = t1 - f1 * (t1 - t0) / (f1 - f0);
            (t0, f0, t1) = (t1, f1, t2);
            if !t1.is_finite() {
                break;
            }
        }
        if !done {
            invalid!("面 {k}: 厚さのソルブが収束しない");
        }
        Ok(t1)
    }
}

/// OPD の厚さのソルブの許容 [波]（解けているとみなす OPD の残り）
const OPD_SOLVE_TOL: f64 = 1e-9;

/// 小さな正方行列の LU 分解（部分ピボット、行優先）
struct Lu {
    a: Vec<f64>,
    piv: Vec<usize>,
    n: usize,
}

impl Lu {
    fn new(mut a: Vec<f64>, n: usize) -> Option<Lu> {
        let scale = a.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        if !(scale > 0.0) || !scale.is_finite() {
            return None;
        }
        let mut piv: Vec<usize> = (0..n).collect();
        for k in 0..n {
            let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
            if !(a[p * n + k].abs() > 1e-14 * scale) {
                return None;
            }
            if p != k {
                for c in 0..n {
                    a.swap(k * n + c, p * n + c);
                }
                piv.swap(k, p);
            }
            for i in k + 1..n {
                let f = a[i * n + k] / a[k * n + k];
                a[i * n + k] = f;
                for c in k + 1..n {
                    a[i * n + c] -= f * a[k * n + c];
                }
            }
        }
        Some(Lu { a, piv, n })
    }

    fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = self.n;
        let mut x: Vec<f64> = self.piv.iter().map(|&i| b[i]).collect();
        for i in 0..n {
            for c in 0..i {
                x[i] -= self.a[i * n + c] * x[c];
            }
        }
        for i in (0..n).rev() {
            for c in i + 1..n {
                x[i] -= self.a[i * n + c] * x[c];
            }
            x[i] /= self.a[i * n + i];
        }
        x
    }
}
