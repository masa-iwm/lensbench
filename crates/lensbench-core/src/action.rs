//! 面の働き（[`Action`]）: 光線の方向・光路長・偏光を面でどう変えるか。
//!
//! [`Surface::action`] は今の平たい [`Surface`] の値から働きを借りて作る見方で、形（[`crate::shape`]）と組で
//! 面の種類を表す。座標ブレークは面でないので [`Surface::is_coord_break`] で先に分ける。
//! 面の後ろの屈折率分布（GRIN）は面の働きでなく、面の後ろの媒質に属する（[`Surface::grin`]）

use crate::math::*;
use crate::shape::sag;
use crate::surface::{Binary2, Bire, CylFresnel, Reflect, Retarder, SurfType, Surface};
use crate::usersurf::UserSurf;

/// Jones の面の行列の元
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JonesSrc<'a> {
    /// 波長によらない行列 [Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]
    Matrix(&'a [f64; 8]),
    /// 波長ごとに作る偏光子・波長板
    Retarder(&'a Retarder),
}

impl JonesSrc<'_> {
    /// 波長 `wl` [µm] の行列 [Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]
    pub fn jones8(&self, wl: f64) -> [f64; 8] {
        match self {
            JonesSrc::Matrix(m) => **m,
            JonesSrc::Retarder(r) => r.jones8(wl),
        }
    }
}

/// Fresnel の面の屈折を決める面（交点は形 [`crate::shape::Shape`] で求め、屈折はこの面の傾きで）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FresnelBend<'a> {
    /// Fresnel（FRESNELS）: 偶数次非球面（c・k・r², r⁴, … の係数 a）
    Radial { c: f64, k: f64, a: &'a [f64] },
    /// Cylinder Fresnel（CFRESNEL）: y だけの面（無ければ曲げない）
    Cyl(Option<&'a CylFresnel>),
}

impl FresnelBend<'_> {
    /// 局所座標 (x, y) での屈折の面の法線（規格化前）
    pub fn normal(&self, x: f64, y: f64) -> V3 {
        match *self {
            FresnelBend::Cyl(Some(f)) => {
                let (_, dz) = sag(f.c, f.k, &f.coef, y * y);
                [0.0, -2.0 * dz * y, 1.0]
            }
            FresnelBend::Cyl(None) => [0.0, 0.0, 1.0],
            FresnelBend::Radial { c, k, a } => {
                let (_, dz) = sag(c, k, a, x * x + y * y);
                [-2.0 * dz * x, -2.0 * dz * y, 1.0]
            }
        }
    }

    /// 近軸の曲率（YZ 面）。Cylinder Fresnel の屈折の面が無ければ None（基板の形で）
    pub fn paraxial_curv(&self) -> Option<f64> {
        match *self {
            FresnelBend::Radial { c, a, .. } => Some(c + 2.0 * a.first().copied().unwrap_or(0.0)),
            FresnelBend::Cyl(Some(f)) => Some(f.c + 2.0 * f.coef.first().copied().unwrap_or(0.0)),
            FresnelBend::Cyl(None) => None,
        }
    }
}

/// 面の働き（[`Surface::action`]）。鏡（[`Surface::mirror`]）の Binary 2・Fresnel は Binary2・Fresnel のまま
/// （反射してから位相・傾きを加える）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action<'a> {
    /// 屈折（前後の媒質の屈折率で）
    Refract,
    /// 反射（鏡）。`ghost` は屈折面での反射（ゴーストの経路の解析の写しだけ）の、光が入らない側の媒質
    Reflect { ghost: Option<&'a Reflect> },
    /// 理想レンズ（焦点距離 f、OPD の求め方 opd_mode）
    Paraxial { f: f64, opd_mode: i32 },
    /// Jones 行列（方向は変えず、電場に行列をかける）
    Jones(JonesSrc<'a>),
    /// 屈折・反射の後に位相 Φ で回折する（Binary 2）
    Binary2(Option<&'a Binary2>),
    /// 屈折を別の面の傾きで決める（Fresnel・Cylinder Fresnel）
    Fresnel(FresnelBend<'a>),
    /// 複屈折材の板の入口・出口（屈折は常光で、偏光の位相を板の中の道のりで）
    Birefringent(&'a Bire),
    /// 利用者定義の面（登録のモデル）
    User(Option<&'a UserSurf>),
}

impl Surface {
    /// 面の働き（今の値から作る見方）。座標ブレークは面でないので [`Surface::is_coord_break`] で先に分ける
    /// （ここでは [`Action::Refract`] を返すが、屈折率は前の値を引き継ぐ）
    pub fn action(&self) -> Action<'_> {
        match self.typ() {
            SurfType::Paraxial => Action::Paraxial {
                f: self.focal(),
                opd_mode: self.opd_mode(),
            },
            SurfType::User => Action::User(self.user()),
            SurfType::Jones => Action::Jones(match &self.retarder {
                Some(r) => JonesSrc::Retarder(r),
                None => JonesSrc::Matrix(match &self.form {
                    crate::surface::Form::Jones { jones } => jones,
                    _ => &crate::surface::JONES_IDENTITY,
                }),
            }),
            SurfType::Binary2 => Action::Binary2(self.bin2()),
            SurfType::Fresnel => Action::Fresnel(FresnelBend::Radial {
                c: self.c,
                k: self.k,
                a: &self.asph,
            }),
            SurfType::CylFresnel => Action::Fresnel(FresnelBend::Cyl(self.cylf())),
            SurfType::CoordBrk => Action::Refract,
            _ => match &self.bire {
                Some(b) if !self.is_mirror() => Action::Birefringent(b),
                _ if self.is_mirror() => Action::Reflect {
                    ghost: self.reflect.as_ref(),
                },
                _ => Action::Refract,
            },
        }
    }
}

impl Action<'_> {
    /// 形を持たず、方向を直接変える面（理想レンズ・利用者定義の面）
    pub fn is_ideal(&self) -> bool {
        matches!(self, Action::Paraxial { .. } | Action::User(_))
    }
}

/// 追跡の内側で面ごとに分ける働き（[`crate::shape::SurfKind`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    /// 屈折・反射（Fresnel・複屈折材の板も）
    Bend,
    CoordBrk,
    Paraxial,
    User,
    Jones,
    Binary2,
}

impl Act {
    pub(crate) fn of(d: &Surface, a: &Action) -> Act {
        if d.is_coord_break() {
            return Act::CoordBrk;
        }
        match a {
            Action::Paraxial { .. } => Act::Paraxial,
            Action::User(_) => Act::User,
            Action::Jones(_) => Act::Jones,
            Action::Binary2(_) => Act::Binary2,
            Action::Refract
            | Action::Reflect { .. }
            | Action::Fresnel(_)
            | Action::Birefringent(_) => Act::Bend,
        }
    }
}
