//! パラメータを変えたあとに残してよいキャッシュ（[`keep_for`]）

use super::Kind;
use super::Param;
use crate::system::Keep;
use crate::units::UnitKind;

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
        // ソルブの条件は厚さ・曲率を解き直す
        p.kind == Kind::Thick
            || p.kind == Kind::Unit(UnitKind::Shift)
            || p.kind.is_solve_value()
            || tilt(p.kind)
    });
    let only_tilt = changes.iter().all(|(p, _)| tilt(p.kind));
    Keep {
        frames: !geom,
        paraxial: only_tilt,
    }
}
