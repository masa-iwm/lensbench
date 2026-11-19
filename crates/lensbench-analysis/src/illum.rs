//! 相対照度と実効 F ナンバー: 像点に届く光束の、像空間の向きの広がり（射影立体角）から求める
//!
//! 像点 x₀（視野のその波長の主光線が像面に当たる点）に届く光線は、瞳の中を通る光線のうち像面で x₀ に着くもの。瞳の縁の各点 (cos a, sin a) について、x₀ に着くように視野 (hx, hy) をニュートン法で解き、その光線の像空間の
//! 向きの、像面の接平面への成分 (L, M) を集める。縁の (L, M) が囲む面積が射影立体角 Ωp で、像面の照度は
//! 物体の輝度が一様なら Ωp に比例する（輝度は光線に沿って保たれる）。けられ係数は使わず、面の開口でけられる向きは
//! 瞳の半径を縮めて縁を探す（縁が中心から見て星形であること）。
//!
//! 1 つの物体点からの光束の向き（像点はばらつく）で測ると、照度は収差の分だけずれる（照合の系で 3e-3）。

use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::math::*;
use lensbench_core::system::{FieldSel, Launch, System};

/// 縁の点の数の既定（面積はリチャードソン外挿で 1/n⁴ の誤差、照合の系で 128 と 512 の差が 1e-7 以下）
pub const RIM_POINTS: usize = 128;

/// 視野 h・正規化瞳座標 p の光線の像面の点 (x, y) と向き（像面の局所座標、けられ係数なし）。止まったか、けられたら None
fn ray(s: &System, h: (f64, f64), p: (f64, f64), w: usize) -> Result<Option<(f64, f64, V3)>> {
    // 視野の範囲の外（ニュートン法の途中）は届かない光線として扱い、呼ぶ側の二分法に回す
    let Ok(spec) = s.object_spec(h.0, h.1) else {
        return Ok(None);
    };
    let l = Launch::Pupil {
        spec,
        signed: false,
        px: &[p.0],
        py: &[p.1],
        vig: Some([0.0; 5]),
        hxy: Some(h),
    };
    let t = s.trace_launch(&l, w, None, false);
    let r = &t.rays[0];
    if r.status.surf().is_some() || r.vig != 0 || !r.xyz[0].is_finite() {
        return Ok(None);
    }
    Ok(Some((r.xyz[0], r.xyz[1], r.lmn)))
}

/// 瞳の点 p を通って像点 x0 に着く光線の向き（視野を h から解く）。解けない・けられたら None
fn ray_to(
    s: &System,
    x0: (f64, f64),
    h: &mut (f64, f64),
    p: (f64, f64),
    w: usize,
) -> Result<Option<V3>> {
    // 正規化視野の刻み（像面の位置の視野での微分）
    const D: f64 = 1e-6;
    for _ in 0..20 {
        let Some(r) = ray(s, *h, p, w)? else {
            return Ok(None);
        };
        let e = (r.0 - x0.0, r.1 - x0.1);
        if e.0.abs().max(e.1.abs()) < 1e-13 * (1.0 + x0.0.abs().max(x0.1.abs())) {
            return Ok(Some(r.2));
        }
        let (Some(rx), Some(ry)) = (ray(s, (h.0 + D, h.1), p, w)?, ray(s, (h.0, h.1 + D), p, w)?)
        else {
            return Ok(None);
        };
        let j = [
            [(rx.0 - r.0) / D, (ry.0 - r.0) / D],
            [(rx.1 - r.1) / D, (ry.1 - r.1) / D],
        ];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det == 0.0 || !det.is_finite() {
            return Ok(None);
        }
        let dh = (
            (j[1][1] * e.0 - j[0][1] * e.1) / det,
            (-j[1][0] * e.0 + j[0][0] * e.1) / det,
        );
        *h = (h.0 - dh.0, h.1 - dh.1);
        if dh.0.abs().max(dh.1.abs()) < 1e-15 {
            return Ok(ray(s, *h, p, w)?.map(|r| r.2));
        }
    }
    Ok(None)
}

/// 像点 x0 に届く光束の、瞳の向き a の縁の光線の向き（けられたら瞳の半径を縮めて境目を探す）
fn rim_dir(s: &System, x0: (f64, f64), h0: (f64, f64), a: f64, w: usize) -> Result<V3> {
    let (c, sn) = (a.cos(), a.sin());
    let mut h = h0;
    if let Some(k) = ray_to(s, x0, &mut h, (c, sn), w)? {
        return Ok(k);
    }
    // 半径の二分法（中心は届く）
    let (mut lo, mut hi) = (0.0, 1.0);
    let mut best = None;
    let mut hl = h0;
    for _ in 0..40 {
        let m = 0.5 * (lo + hi);
        let mut hm = hl;
        match ray_to(s, x0, &mut hm, (m * c, m * sn), w)? {
            Some(k) => {
                lo = m;
                hl = hm;
                best = Some(k);
            }
            None => hi = m,
        }
    }
    match best {
        Some(k) => Ok(k),
        None => invalid!(
            "no ray of the field reaches its image point through the pupil (direction {a:.3} rad)"
        ),
    }
}

/// 視野 field の波長 w の主光線が像面に当たる点に届く光束の射影立体角 Ωp に、像空間の屈折率の 2 乗をかけたもの
/// （光学的な方向余弦 n′L・n′M の面積）。n は縁の点の数（偶数、8 以上）
pub fn exit_cone(s: &System, field: FieldSel, w: usize, n: usize) -> Result<f64> {
    if s.afocal {
        invalid!("the illumination of an afocal image space is not supported");
    }
    if n < 8 || !n.is_multiple_of(2) {
        invalid!("the number of rim points must be even and 8 or more ({n})");
    }
    if w >= s.wavelengths.len() {
        invalid!("wavelength {w} (of {})", s.wavelengths.len());
    }
    let h0 = s.field_sel_norm(field, w)?;
    let Some(c) = ray(s, h0, (0.0, 0.0), w)? else {
        invalid!("the chief ray doesn't reach the image");
    };
    let x0 = (c.0, c.1);
    // 像面の x0 での接平面の正規直交基底 e1（x 寄り）・e2
    let img = &s.surfaces[s.image()];
    let (_, zx, zy) = lensbench_core::freeform::sag_xy(img, x0.0, x0.1);
    let (zx, zy) = if zx.is_finite() && zy.is_finite() {
        (zx, zy)
    } else {
        (0.0, 0.0)
    };
    let nrm = normalize([-zx, -zy, 1.0]);
    let e1 = normalize(sub([1.0, 0.0, 0.0], scale(nrm, nrm[0])));
    let e2 = cross(nrm, e1);
    let mut pts = Vec::with_capacity(n);
    for k in 0..n {
        let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
        let d = rim_dir(s, x0, h0, a, w)?;
        pts.push((dot(d, e1), dot(d, e2)));
    }
    let area = |step: usize| -> f64 {
        let m = n / step;
        let mut a = 0.0;
        for i in 0..m {
            let (x0, y0) = pts[i * step];
            let (x1, y1) = pts[((i + 1) % m) * step];
            a += x0 * y1 - x1 * y0;
        }
        (a / 2.0).abs()
    };
    // 多角形の面積の誤差は滑らかな縁で 1/n² の級数なので、半分の点の面積とでリチャードソン外挿
    let a = (4.0 * area(1) - area(2)) / 3.0;
    let ni = s.image_index(w);
    Ok(a * ni * ni)
}

/// 相対照度: 視野 field の像点の照度の、軸上の視野 (0, 0) の像点に対する比（物体の輝度が一様、けられ係数なし）
pub fn relative_illumination(s: &System, field: FieldSel, w: usize, n: usize) -> Result<f64> {
    let a0 = exit_cone(s, FieldSel::Norm(0.0, 0.0), w, n)?;
    let a = exit_cone(s, field, w, n)?;
    Ok(a / a0)
}

/// 実効 F ナンバー 1 / (2 NA)（NA = √(Ωp n′² / π)、[`exit_cone`]）
pub fn effective_fno(s: &System, field: FieldSel, w: usize, n: usize) -> Result<f64> {
    let a = exit_cone(s, field, w, n)?;
    Ok(1.0 / (2.0 * (a / std::f64::consts::PI).sqrt()))
}
