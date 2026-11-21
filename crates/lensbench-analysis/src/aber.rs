//! 収差図の計算: 横収差と OPD の扇（Ray Fan・OPD Fan）、像面湾曲（主光線の近くの焦点）、倍率色収差、色の焦点ずれ。
//!
//! どれも評価関数の演算子と同じ計算から作る（TRAY・TRAX は [`ray_fan`]、OPDC は [`opd_fan`]、FCGT・FCGS は
//! [`field_focus`]、LONA は `lensbench_core::seidel::longitudinal`）。図の値と演算子の値が合う。

use lensbench_core::{invalid, FieldSel, Result, System};

/// 子午・球欠の扇の 1 本（瞳の座標 p ごとの値。けられた・届かなかった光線は NaN）
#[derive(Debug, Clone, PartialEq)]
pub struct Fan {
    /// 正規化瞳座標（子午は Py、球欠は Px）
    pub p: Vec<f64>,
    /// 子午の扇（瞳 (0, p)）の値
    pub tan: Vec<f64>,
    /// 球欠の扇（瞳 (p, 0)）の値
    pub sag: Vec<f64>,
}

/// 主波長の主光線の像面の点（像面の点は [`System::image_point`]、アフォーカル像空間では角度）
fn chief_point(s: &System, field: FieldSel, w: usize) -> Result<(f64, f64)> {
    let t = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
    if !t.traced(0) {
        let r = &t.rays[0];
        invalid!(
            "the chief ray (wavelength {}) doesn't reach the image (surface {})",
            w + 1,
            r.status.code()
        );
    }
    let r = &t.rays[0];
    Ok(s.image_point(r.xyz, r.lmn))
}

/// −1 から 1 までの 2n + 1 点（瞳の扇の座標。OpticStudio の Number of Rays の n）
pub fn fan_points(n: usize) -> Vec<f64> {
    let n = n.max(1);
    (0..=2 * n).map(|k| -1.0 + k as f64 / n as f64).collect()
}

/// 横収差の扇（Ray Fan）: 子午は ey（像面の y の、主波長の主光線との差）を Py に、球欠は ex（x の差）を Px に対して。
/// 演算子 TRAY・TRAX と同じ定義（アフォーカル像空間では向きの角度の差）
pub fn ray_fan(s: &System, field: FieldSel, w: usize, p: &[f64]) -> Result<Fan> {
    let (cx, cy) = chief_point(s, field, s.primary)?;
    let zeros = vec![0.0; p.len()];
    let tan = s.try_trace(field, &zeros, p, Some(w), None, false)?;
    let sag = s.try_trace(field, p, &zeros, Some(w), None, false)?;
    let val = |t: &lensbench_core::system::Trace, i: usize, y: bool| {
        if !t.ok(i) {
            return f64::NAN;
        }
        let r = &t.rays[i];
        let (x, yy) = s.image_point(r.xyz, r.lmn);
        if y {
            yy - cy
        } else {
            x - cx
        }
    };
    Ok(Fan {
        p: p.to_vec(),
        tan: (0..p.len()).map(|i| val(&tan, i, true)).collect(),
        sag: (0..p.len()).map(|i| val(&sag, i, false)).collect(),
    })
}

/// OPD の扇（OPD Fan）[波]: 参照球の中心は主波長の主光線（演算子 OPDC・波面図の ref_primary と同じ）、光路の基準は波長 w
/// 自身の主光線（瞳の中心で 0）
pub fn opd_fan(s: &System, field: FieldSel, w: usize, p: &[f64]) -> Result<Fan> {
    let zeros = vec![0.0; p.len()];
    let one = |px: &[f64], py: &[f64]| -> Result<Vec<f64>> {
        let o = lensbench_core::opd::opd(s, field, px, py, Some(w), true)?;
        Ok((0..p.len())
            .map(|i| if o.ok(i) { o.opd[i] } else { f64::NAN })
            .collect())
    };
    Ok(Fan {
        p: p.to_vec(),
        tan: one(&zeros, p)?,
        sag: one(p, &zeros)?,
    })
}

/// 主光線の近くの光線の焦点の、面 j からの z の距離（像面湾曲。OpticStudio の FCGT（tangential）・FCGS）。正規化瞳座標を y
/// （子午）か x（球欠）の向きに ±0.001 ずらした 2 本の光線が最も近づく z（横の位置の差 Δp と傾きの差 Δs から
/// −Δp·Δs / |Δs|²。回転対称でない系でも決まる）。OpticStudio 26.1 は主光線と +0.001 の光線の片側の差分で求め、ずらす向きが
/// 視野の向きと垂直なら中心差分と 1e-10 で合い、そうでなければ 1e-4 ほど違う（刻みを 0 に近づけた値は中心差分の側）
pub fn field_focus(
    s: &System,
    field: FieldSel,
    w: usize,
    j: usize,
    tangential: bool,
) -> Result<f64> {
    const H: f64 = 1e-3;
    let (px, py) = if tangential {
        ([0.0, 0.0], [-H, H])
    } else {
        ([-H, H], [0.0, 0.0])
    };
    let t = s.try_trace(field, &px, &py, Some(w), Some(j), false)?;
    if let Some(r) = t.rays.iter().find(|r| r.status.surf().is_some()) {
        invalid!(
            "the ray near the chief ray stopped (wavelength {}, surface {})",
            w + 1,
            r.status.code()
        );
    }
    let (a, b) = (&t.rays[0], &t.rays[1]);
    let dp = [b.xyz[0] - a.xyz[0], b.xyz[1] - a.xyz[1]];
    let ds = [
        b.lmn[0] / b.lmn[2] - a.lmn[0] / a.lmn[2],
        b.lmn[1] / b.lmn[2] - a.lmn[1] / a.lmn[2],
    ];
    let z = -(dp[0] * ds[0] + dp[1] * ds[1]) / (ds[0] * ds[0] + ds[1] * ds[1]);
    if !z.is_finite() {
        invalid!("the field curvature is not finite (wavelength {})", w + 1);
    }
    Ok(z)
}

/// 視野 field の波長 w の実主光線の像面の点と、主波長の実主光線の点の差 (dx, dy)（倍率色収差。アフォーカル像空間では角度）
pub fn lateral_color(s: &System, field: FieldSel, w: usize) -> Result<(f64, f64)> {
    let (x0, y0) = chief_point(s, field, s.primary)?;
    let (x, y) = chief_point(s, field, w)?;
    Ok((x - x0, y - y0))
}

/// 波長 lam [µm] の近軸焦点の、主波長の近軸焦点からのずれ [レンズ単位]（色の焦点ずれ。像面の位置に対する近軸の後側焦点の
/// ずれ: 演算子 LONA の帯 0 の差、AXCL と同じ）
pub fn focal_shift(s: &System, lam: f64) -> Result<f64> {
    let base = lensbench_core::seidel::longitudinal(s, s.primary, 0.0)?;
    let (d, w) = s.with_wavelength(lam)?;
    Ok(lensbench_core::seidel::longitudinal(&d, w, 0.0)? - base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_points_are_symmetric() {
        assert_eq!(fan_points(2), vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(fan_points(0), vec![-1.0, 0.0, 1.0]);
    }
}
