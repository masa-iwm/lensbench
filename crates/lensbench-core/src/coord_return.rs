//! 座標の戻り（OpticStudio の Coordinate Return）: 座標ブレークの傾き・偏心（XYZ なら厚さも）を、前の面の座標系に戻るように
//! 求める。
//!
//! OpticStudio 2026 R1.01 の実測（.zmx の `SCBD 3 モード 戻り先の面`）:
//! - 座標ブレーク面だけに効く（標準面に書いても使わない）。順序は「傾きの後に偏心」（PARM 6 = 1）にする
//! - Orientation Only（1）: 向きだけを戻り先の面の座標系にそろえ、偏心は 0。Orientation XY（2）: 加えて偏心で x・y の位置をそろえる。
//!   Orientation XYZ（3）: 加えて厚さで次の面を戻り先の面の頂点に置く（Only・XY の厚さは書いた値を戻した向きの z に当てる）
//! - 戻り先の面の座標系は、その面が置かれる座標系（座標ブレークなら傾き偏心を当てる前、前の傾きのある面なら当てた後）。面 0 は
//!   物体面（物体の厚さが有限のときだけ位置を戻せる）
//!
//! 角度は 傾き z・y・x の順の回転 R = Rz·Ry·Rx（[`crate::system::apply_cb`] の順序 1）に分ける。玉・群の剛体の動きは入れない

use crate::error::Result;
use crate::invalid;
use crate::math::*;
use crate::param::{Kind, Param};
use crate::system::{step_frame, Prescription, System};

/// 戻し方（OpticStudio の番号 1〜3）
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReturnMode {
    /// 戻さない（多重構成の行で構成ごとに切るとき）
    Off,
    /// 向きだけ
    Orientation,
    /// 向きと x・y
    OrientationXy,
    /// 向きと x・y・z（厚さも求める）
    OrientationXyz,
}

impl ReturnMode {
    /// OpticStudio の番号（1〜3）
    pub fn os_code(self) -> u8 {
        match self {
            ReturnMode::Off => 0,
            ReturnMode::Orientation => 1,
            ReturnMode::OrientationXy => 2,
            ReturnMode::OrientationXyz => 3,
        }
    }

    /// OpticStudio の番号から
    pub fn from_os_code(n: i64) -> Option<ReturnMode> {
        match n {
            0 => Some(ReturnMode::Off),
            1 => Some(ReturnMode::Orientation),
            2 => Some(ReturnMode::OrientationXy),
            3 => Some(ReturnMode::OrientationXyz),
            _ => None,
        }
    }
}

/// 座標の戻り（[`crate::surface::Surface::coord_return`]）
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct CoordReturn {
    pub mode: ReturnMode,
    /// 戻り先の面（この面より前）
    pub surf: usize,
}

/// R = Rz(c)·Ry(b)·Rx(a) の (a, b, c) [deg]
fn angles_zyx(m: &M3) -> [f64; 3] {
    let b = (-m[2][0]).clamp(-1.0, 1.0).asin();
    let (a, c) = if b.cos().abs() > 1e-12 {
        (m[2][1].atan2(m[2][2]), m[1][0].atan2(m[0][0]))
    } else {
        // ジンバルロック: c = 0 として a を決める
        ((-m[1][2]).atan2(m[1][1]), 0.0)
    };
    [a.to_degrees(), b.to_degrees(), c.to_degrees()]
}

/// 座標の戻りのある座標ブレークの傾き・偏心（と厚さ）を求め直す
pub(crate) fn resolve(rx: &mut Prescription) -> Result<()> {
    let n = rx.surfaces.len();
    let t0 = rx.surfaces[0].t;
    // 面の置かれる座標系（面 0 は物体面、厚さが無限なら向きだけ）
    let mut placed: Vec<Option<(V3, M3)>> = vec![None; n];
    placed[0] = Some(([0.0, 0.0, if t0.is_finite() { -t0 } else { f64::NAN }], I3));
    let (mut o, mut r): (V3, M3) = ([0.0; 3], I3);
    for j in 1..n {
        if let (Some(cr), true) = (
            rx.surfaces[j]
                .coord_return
                .filter(|c| c.mode != ReturnMode::Off),
            rx.surfaces[j].is_coord_break() && !rx.surfaces[j].ignore,
        ) {
            if cr.surf >= j {
                invalid!(
                    "surface {j}: the coordinate return goes to surface {} (an earlier surface)",
                    cr.surf
                );
            }
            let (ot, rt) = placed[cr.surf].expect("an earlier surface is placed");
            let [tx, ty, tz] = angles_zyx(&mm(&transpose(&r), &rt));
            // 戻り先の座標系で見た今の原点
            let p = mtv(&rt, sub(o, ot));
            let xy = cr.mode != ReturnMode::Orientation;
            if xy && !(p[0].is_finite() && p[1].is_finite()) {
                invalid!(
                    "surface {j}: the coordinate return to the object surface at infinity can only return the orientation"
                );
            }
            let d = &mut rx.surfaces[j];
            if let Some((cb, order)) = d.cb_mut() {
                *cb = [
                    if xy { -p[0] } else { 0.0 },
                    if xy { -p[1] } else { 0.0 },
                    tx,
                    ty,
                    tz,
                ];
                *order = 1;
            }
            if cr.mode == ReturnMode::OrientationXyz {
                d.t = -p[2];
            }
        }
        let d = &rx.surfaces[j];
        let (frame, next) = step_frame(o, r, d);
        // 座標ブレークは当てる前、ほかの面は前の傾きを当てた後の座標系に置かれる
        placed[j] = Some(if d.is_coord_break() && !d.ignore {
            (o, r)
        } else {
            frame
        });
        (o, r) = next;
    }
    Ok(())
}

/// 座標の戻りがあれば [`resolve`] し、厚さが変わったかを返す（パラメータを変えたあと。厚さが変われば近軸量も変わる）
pub(crate) fn resolve_changed(rx: &mut Prescription) -> Result<bool> {
    if rx.surfaces.iter().all(|d| d.coord_return.is_none()) {
        return Ok(false);
    }
    let before: Vec<f64> = rx.surfaces.iter().map(|d| d.t).collect();
    resolve(rx)?;
    Ok(rx.surfaces.iter().zip(&before).any(|(d, &t)| d.t != t))
}

/// パラメータ [`Kind::CoordRet`]・[`Kind::CoordRetSurf`] に値 v を入れてよいか
pub(crate) fn check(s: &System, p: Param, v: f64) -> Result<()> {
    let j = p.index();
    if !s.surfaces[j].is_coord_break() {
        invalid!("{}: only a coordinate break returns coordinates", p.name());
    }
    if p.kind == Kind::CoordRet && (v.fract() != 0.0 || !(0.0..=3.0).contains(&v)) {
        invalid!("{}: coordinate return {v} (0 to 3)", p.name());
    }
    if p.kind == Kind::CoordRetSurf && (v.fract() != 0.0 || !(0.0..j as f64).contains(&v)) {
        invalid!(
            "{}: coordinate return surface {v} (0 to {})",
            p.name(),
            j.saturating_sub(1)
        );
    }
    Ok(())
}
