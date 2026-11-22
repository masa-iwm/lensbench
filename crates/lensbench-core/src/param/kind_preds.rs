//! パラメータの種類の分類（[`Kind`] の is_* など）

use super::Kind;

impl Kind {
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
                | Kind::MechSemi
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
                | Kind::ModelNd
                | Kind::ModelVd
                | Kind::ModelDpgf
                | Kind::Tce
                | Kind::MechSemi
                | Kind::ChipZone
                | Kind::CoordRet
                | Kind::CoordRetSurf
                | Kind::Material
                | Kind::Coating
        ) || self.is_solve_value()
    }

    /// 座標の戻りの行（[`Kind::CoordRet`]・[`Kind::CoordRetSurf`]）
    pub fn is_coord_return(&self) -> bool {
        matches!(self, Kind::CoordRet | Kind::CoordRetSurf)
    }

    /// ソルブ・ピックアップの条件の値（[`Kind::CurvSolve`]・[`Kind::ThickSolve`]・[`Kind::ParmSolve`]）
    pub fn is_solve_value(&self) -> bool {
        matches!(
            self,
            Kind::CurvSolve(_) | Kind::ThickSolve(_) | Kind::ParmSolve(..)
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
                | Kind::AimPupil(_)
                | Kind::InputPol(_)
                | Kind::FieldX(_)
                | Kind::FieldY(_)
                | Kind::FieldW(_)
                | Kind::FieldVig(..)
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
