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

/// 届いた光線: 正規化視野 h、正規化瞳座標 p、入射瞳の座標 v（[`System::pupil_ray`]）
#[derive(Clone, Copy)]
struct Near {
    h: (f64, f64),
    p: [f64; 2],
    v: [f64; 2],
}

/// 視野 h・正規化瞳座標 p の光線の入射瞳の座標を、近くの届いた光線 n から、視野と瞳座標を一緒に少しずつ動かして
/// （継続法、各点は [`System::aim_pupil_near`]）解く。1 歩で解けなければ刻みを半分にし、1/16 より小さくなったら None
fn aim_from(s: &System, n: &Near, h: (f64, f64), p: [f64; 2], w: usize) -> Option<[f64; 2]> {
    let lerp = |a: f64, b: f64, t: f64| if t >= 1.0 { b } else { a + t * (b - a) };
    let (mut from, mut v) = (s.object_spec(n.h.0, n.h.1).ok()?, n.v);
    let (mut t, mut dt) = (0.0f64, 1.0f64);
    while t < 1.0 {
        let tn = if dt >= 1.0 - t { 1.0 } else { t + dt };
        let hn = (lerp(n.h.0, h.0, tn), lerp(n.h.1, h.1, tn));
        let pn = [lerp(n.p[0], p[0], tn), lerp(n.p[1], p[1], tn)];
        let r = s
            .object_spec(hn.0, hn.1)
            .ok()
            .and_then(|spec| s.aim_pupil_near(from, v, spec, pn, w).map(|vn| (spec, vn)));
        match r {
            Some((spec, vn)) => {
                (from, v) = (spec, vn);
                t = tn;
                dt *= 2.0;
            }
            None => {
                dt *= 0.5;
                if dt < 1.0 / 16.0 {
                    return None;
                }
            }
        }
    }
    Some(v)
}

/// 視野 h・正規化瞳座標 p の光線の像面の点 (x, y) と向き（像面の局所座標、けられ係数なし）。止まったか、けられたら None。
///
/// near は続けて追う近くの届いた光線（同じ縁の点のニュートン法・二分法の、1 つ前の届いた光線。初めは主光線）で、
/// そこから視野と瞳座標を少しずつ動かしてエイミングし（[`aim_from`]）、届いたらこの光線にする。超広角の系では
/// 近軸の入射瞳から始めると光線が絞りまで届かず、光線ごとに主光線を軸上から継続法で解き直していた（届かない
/// 探りの点では特に遅く、解けるはずの縁の光線を届かないとすることもあった）。近くの光線からたどれなければ届かない
/// とする。届いた光線は絞りの点 p を通る実光線（収束の幅はエイミングと同じ）。near が無いとき（主光線）は
/// [`System::aim_pupil`] で解く。near は縁の点ごとに持つので、結果はスレッド数によらない
fn ray(
    s: &System,
    h: (f64, f64),
    p: (f64, f64),
    w: usize,
    near: &mut Option<Near>,
) -> Result<Option<(f64, f64, V3)>> {
    // 視野の範囲の外（ニュートン法の途中）は届かない光線として扱い、呼ぶ側の二分法に回す
    let Ok(spec) = s.object_spec(h.0, h.1) else {
        return Ok(None);
    };
    let q = [p.0, p.1];
    let v = match *near {
        Some(n) => aim_from(s, &n, h, q, w),
        None => s.aim_pupil(spec, q, w),
    };
    let Some(v) = v else {
        return Ok(None);
    };
    let (p0, k0) = s.pupil_ray(spec, v);
    let t = s.trace_launch(&Launch::Rays { p: &[p0], k: &[k0] }, w, None, false);
    let r = &t.rays[0];
    if r.status.surf().is_some() || r.vig != 0 || !r.xyz[0].is_finite() {
        return Ok(None);
    }
    *near = Some(Near { h, p: q, v });
    Ok(Some((r.xyz[0], r.xyz[1], r.lmn)))
}

/// 瞳の点 p を通って像点 x0 に着く光線の向き（視野を h から解く）。解けない・けられたら None
///
/// ニュートン法の 1 歩が届かない光線（全反射・開口・視野の外）に入ったら、歩幅を半分にして戻る（届く光線の範囲の縁の
/// 近くでは、解があっても全幅の 1 歩が外へ出る。そこで止めると、届く縁の光線をけられたものと見て縁を内側に取っていた）。
/// 微分の刻みの先が届かなければ逆向きの刻みで差分を取る
fn ray_to(
    s: &System,
    x0: (f64, f64),
    h: &mut (f64, f64),
    p: (f64, f64),
    w: usize,
    near: &mut Option<Near>,
) -> Result<Option<V3>> {
    // 正規化視野の刻み（像面の位置の視野での微分）
    const D: f64 = 1e-6;
    let tol = 1e-13 * (1.0 + x0.0.abs().max(x0.1.abs()));
    let Some(mut r) = ray(s, *h, p, w, near)? else {
        return Ok(None);
    };
    for _ in 0..40 {
        let e = (r.0 - x0.0, r.1 - x0.1);
        if e.0.abs().max(e.1.abs()) < tol {
            return Ok(Some(r.2));
        }
        // 前進差分、届かなければ後退差分
        let mut col = |dh: (f64, f64)| -> Result<Option<(f64, f64)>> {
            for sgn in [1.0, -1.0] {
                if let Some(q) = ray(s, (h.0 + sgn * dh.0, h.1 + sgn * dh.1), p, w, near)? {
                    return Ok(Some(((q.0 - r.0) / (sgn * D), (q.1 - r.1) / (sgn * D))));
                }
            }
            Ok(None)
        };
        let (Some(cx), Some(cy)) = (col((D, 0.0))?, col((0.0, D))?) else {
            return Ok(None);
        };
        let j = [[cx.0, cy.0], [cx.1, cy.1]];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det == 0.0 || !det.is_finite() {
            return Ok(None);
        }
        let dh = (
            (j[1][1] * e.0 - j[0][1] * e.1) / det,
            (-j[1][0] * e.0 + j[0][0] * e.1) / det,
        );
        if dh.0.abs().max(dh.1.abs()) < 1e-15 {
            return Ok(Some(r.2));
        }
        // 届く光線に着くまで歩幅を半分に（像面での誤差が減らなくても、届く所で次の 1 歩を取り直す）
        let mut t = 1.0;
        let next = loop {
            let hn = (h.0 - t * dh.0, h.1 - t * dh.1);
            if let Some(q) = ray(s, hn, p, w, near)? {
                break Some((hn, q));
            }
            t *= 0.5;
            if t < 1.0 / 256.0 {
                break None;
            }
        };
        let Some((hn, q)) = next else {
            return Ok(None);
        };
        *h = hn;
        r = q;
    }
    Ok(None)
}

/// 像点 x0 に届く光束の、瞳の向き a の縁の光線の向き（けられたら瞳の半径を縮めて境目を探す）
fn rim_dir(s: &System, x0: (f64, f64), h0: (f64, f64), c0: Near, a: f64, w: usize) -> Result<V3> {
    let (c, sn) = (a.cos(), a.sin());
    let mut h = h0;
    // 近くの届いた光線（[`ray`]）。初めは主光線、二分法では届いた半径の下端の光線から
    let mut near = Some(c0);
    if let Some(k) = ray_to(s, x0, &mut h, (c, sn), w, &mut near)? {
        return Ok(k);
    }
    // 半径の二分法（中心は届く）
    let (mut lo, mut hi) = (0.0, 1.0);
    let mut best = None;
    let mut hl = h0;
    let mut near_lo = c0;
    for _ in 0..40 {
        let m = 0.5 * (lo + hi);
        let mut hm = hl;
        let mut near = Some(near_lo);
        match ray_to(s, x0, &mut hm, (m * c, m * sn), w, &mut near)? {
            Some(k) => {
                lo = m;
                hl = hm;
                best = Some(k);
                near_lo = near.expect("届いた光線");
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
    let mut v0 = None;
    let Some(c) = ray(s, h0, (0.0, 0.0), w, &mut v0)? else {
        invalid!("the chief ray doesn't reach the image");
    };
    let v0 = v0.expect("届いた光線の入射瞳の座標");
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
    // 縁の点は独立に解く（並列、結果はスレッド数によらない）
    let ks: Vec<usize> = (0..n).collect();
    let pts = lensbench_core::par::try_map(&ks, |&k| -> Result<(f64, f64)> {
        let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
        let d = rim_dir(s, x0, h0, v0, a, w)?;
        Ok((dot(d, e1), dot(d, e2)))
    })?;
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

/// ケラレ係数を面の開口に置き換えた系（OpticStudio の Relative Illumination・Vignetting Plot の既定 Remove Vignetting Factors、
/// 道具 Replace Vignetting With Apertures と同じ）: ケラレ係数を使った今の半直径（自動の値も）を全部の面で先に読んで固定にし、
/// 前後で媒質が変わる面（[`lensbench_core::surface::medium_edges`]）で開口の無い面に、その半直径の浮動開口を付けてから、
/// 全部の視野のケラレ係数を 0 にする（係数の無い系はそのまま）。OpticStudio 2026 R1.01 の A6 で、既定の RI・Vignetting Plot と差 0。
/// 1 面ずつ固定にすると、前の面の開口で後ろの面の自動の値が縮むので合わない
pub fn replace_vignetting(s: &System) -> Result<System> {
    use lensbench_core::surface::{Aperture, SurfType};
    // ケラレ係数の無い系はそのまま（演算子 RELI と同じ値。自動の半直径を固定すると縁の光線がわずかにけられる）
    let no_vig = s
        .fields
        .iter()
        .all(|f| f.vdx == 0.0 && f.vdy == 0.0 && f.vcx == 0.0 && f.vcy == 0.0 && f.van == 0.0);
    if no_vig {
        return Ok(s.clone());
    }
    let semi: Vec<f64> = s.surfaces.iter().map(|d| d.semi).collect();
    let edge = lensbench_core::surface::medium_edges(&s.surfaces);
    // 絞りの大きさで決める系（Float By Stop Size）の絞りの半直径は瞳そのもの
    let float_stop =
        (s.aperture.0 == lensbench_core::system::ApertureKind::Float).then(|| s.stop());
    let mut t = s.clone();
    t.edit_raw(|rx| {
        let n = rx.surfaces.len();
        for (k, d) in rx.surfaces.iter_mut().enumerate() {
            d.semi = semi[k];
            d.semi_auto = false;
            let inner = k > 0 && k + 1 < n;
            let cb = d.typ() == SurfType::CoordBrk;
            // 半直径を書いていない面（0）には付けない（プロジェクトの展開の浮動開口と同じ）
            let has = semi[k] > 0.0;
            if inner && has && edge[k] && !cb && Some(k) != float_stop && d.ap == Aperture::None {
                d.ap = Aperture::Floating { semi: semi[k] };
            }
        }
        for f in rx.fields.iter_mut() {
            (f.vdx, f.vdy, f.vcx, f.vcy, f.van) = (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        rx.floating_clip = true;
        Ok(())
    })?;
    Ok(t)
}
