//! MTF の子午・球欠の向き

use super::pupil_jacobian;
use crate::error::Result;
use crate::system::{FieldSel, FieldType, System};

/// MTF の子午・球欠の向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum MtfAxes {
    /// 既定。子午は視野の向き（視野の座標 (x, y)、角度なら (tan θx, tan θy) の向き。回転対称な系の像面の上では
    /// 主光線の像の動径の向き）の周波数、球欠は
    /// それに直交する向き。x・y の両方に角度がある視野・x だけの視野でも、子午は物理的な子午面の向き
    /// （軸上と y だけの視野は y の向きで、[`MtfAxes::Pupil`] と同じ値）
    #[default]
    Field,
    /// 子午は瞳の py、球欠は px の向き（OpticStudio の FFT MTF と同じ。x の角度がある視野では子午面の向きでない）
    Pupil,
}

/// MTF を求める瞳の向きの組: 球欠の向き s と子午の向き t（正規化瞳の単位ベクトル、t = 視野の向き、s ⟂ t）。
/// 恒等（s = x、t = y）なら、どの計算も向きを足す前と同じビット
#[derive(Debug, Clone, Copy)]
pub(crate) struct MtfFrame {
    s: [f64; 2],
    t: [f64; 2],
    id: bool,
}

impl MtfFrame {
    pub(crate) const IDENTITY: MtfFrame = MtfFrame {
        s: [1.0, 0.0],
        t: [0.0, 1.0],
        id: true,
    };

    /// 視野 field の向きの組。[`MtfAxes::Pupil`]・軸上・x = 0 の視野は恒等
    pub(crate) fn new(s: &System, field: FieldSel, w: usize, axes: MtfAxes) -> Result<MtfFrame> {
        if axes == MtfAxes::Pupil {
            return Ok(MtfFrame::IDENTITY);
        }
        let (hx, hy) = s.field_sel_norm(field, w)?;
        let (nx, ny) = s.field_norm_values();
        let (x, y) = (hx * nx, hy * ny);
        // 角度の視野の向きは (tan θx, tan θy)（物体の側の主光線の向き。方位 45° 以外では (θx, θy) の向きと違う）
        let (x, y) = if s.field_type == FieldType::Angle {
            (x.to_radians().tan(), y.to_radians().tan())
        } else {
            (x, y)
        };
        let r = x.hypot(y);
        if x == 0.0 || !(r > 0.0) || !r.is_finite() {
            return Ok(MtfFrame::IDENTITY);
        }
        // 子午の向きは視野の向き（MTF は向きの符号によらないので y ≥ 0 の側にそろえる）
        let sg = if y < 0.0 || (y == 0.0 && x < 0.0) {
            -1.0
        } else {
            1.0
        };
        let t = [sg * x / r, sg * y / r];
        Ok(MtfFrame {
            s: [t[1], -t[0]],
            t,
            id: false,
        })
    }

    pub(crate) fn is_identity(&self) -> bool {
        self.id
    }

    /// 子午の向き t（恒等なら None）
    pub(crate) fn tangential(&self) -> Option<[f64; 2]> {
        (!self.id).then_some(self.t)
    }

    /// 向き dir（0 球欠、1 子午）の線に沿う座標 a・線の位置 v の点（正規化瞳の座標）。dir 0 は s の向きの線、
    /// 1 は t の向きの線
    #[inline]
    pub(crate) fn xy(&self, dir: usize, a: f64, v: f64) -> (f64, f64) {
        if self.id {
            return if dir == 0 { (a, v) } else { (v, a) };
        }
        let (e, f) = if dir == 0 {
            (self.s, self.t)
        } else {
            (self.t, self.s)
        };
        (a * e[0] + v * f[0], a * e[1] + v * f[1])
    }

    /// 組の座標 (u, v)（u は s、v は t の向き）を正規化瞳の座標に
    #[inline]
    pub(crate) fn pupil_xy(&self, u: f64, v: f64) -> (f64, f64) {
        if self.id {
            return (u, v);
        }
        (u * self.s[0] + v * self.t[0], u * self.s[1] + v * self.t[1])
    }

    /// 組の向きの J' = ∂(L', M')/∂(u, v)（行が L' = (L, M)·s・M' = (L, M)·t、列が s・t の向きの瞳の座標）。
    /// 恒等なら [`pupil_jacobian`]。ほかは瞳の ±s・±t の縁の光線の差（届かなければ J を組へ回したもの）
    pub(crate) fn jacobian(&self, s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
        let j = pupil_jacobian(s, field, w)?;
        if self.id {
            return Ok(j);
        }
        let (e, f) = (self.s, self.t);
        let r = if s.afocal {
            None
        } else {
            let px = [e[0], -e[0], f[0], -f[0]];
            let py = [e[1], -e[1], f[1], -f[1]];
            let r = s.try_trace(field, &px, &py, Some(w), None, false)?;
            (0..4).all(|i| r.traced(i)).then_some(r)
        };
        let jp = match r {
            Some(r) => {
                let d = |i: usize, k: usize, c: [f64; 2]| {
                    let (a, b) = (r.rays[i].lmn, r.rays[k].lmn);
                    0.5 * ((a[0] - b[0]) * c[0] + (a[1] - b[1]) * c[1])
                };
                [[d(0, 1, e), d(2, 3, e)], [d(0, 1, f), d(2, 3, f)]]
            }
            // J' = Rᵀ J R（R の列が s・t）
            None => {
                let jv = |c: [f64; 2]| {
                    [
                        j[0][0] * c[0] + j[0][1] * c[1],
                        j[1][0] * c[0] + j[1][1] * c[1],
                    ]
                };
                let (je, jf) = (jv(e), jv(f));
                let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
                [[dot(e, je), dot(e, jf)], [dot(f, je), dot(f, jf)]]
            }
        };
        if !(jp[0][0].abs() > crate::tol::JACOBIAN_MIN && jp[1][1].abs() > crate::tol::JACOBIAN_MIN)
        {
            crate::notimpl!("MTF: the image space aperture is 0 (afocal?)");
        }
        Ok(jp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_rotated_points() {
        let id = MtfFrame::IDENTITY;
        assert_eq!(id.xy(0, 0.3, -0.2), (0.3, -0.2));
        assert_eq!(id.xy(1, 0.3, -0.2), (-0.2, 0.3));
        let t = [0.6, 0.8];
        let f = MtfFrame {
            s: [t[1], -t[0]],
            t,
            id: false,
        };
        // 子午の線（t の向き）に沿う 1 は t
        let (x, y) = f.xy(1, 1.0, 0.0);
        assert!((x - 0.6).abs() < 1e-15 && (y - 0.8).abs() < 1e-15);
        // 球欠の線に沿う 1 は s、線の位置は t の向き
        let (x, y) = f.xy(0, 1.0, 0.5);
        assert!((x - (0.8 + 0.3)).abs() < 1e-15 && (y - (-0.6 + 0.4)).abs() < 1e-15);
        assert_eq!(f.pupil_xy(0.0, 1.0), (0.6, 0.8));
    }
}
