//! 実光線の追跡の核（1 本ずつ）。
//!
//! 光線が面に当たらない・全反射するとその面で止まり（err = 面番号、以後 NaN）、開口の外に出た光線は
//! 先へ進む（vig = 最初の面）。OpticStudio のバッチ追跡と同じ扱い。

use crate::coating::{Iface, C64};
use crate::math::*;
use crate::surface::{Aperture, SurfType};
use crate::system::{Frame, Frames, Rel, System};
use crate::Error;

/// 複素の 3 次元ベクトル（電場）
pub type CV3 = [C64; 3];

/// 偏光追跡での面ごとの処理（波長ごとに用意する）
#[derive(Debug, Clone)]
pub enum PolKind {
    None,
    /// Jones 行列 [[a, b], [c, d]]
    Jones([C64; 4]),
    /// 境界の s・p 係数
    Iface(Iface),
}

#[derive(Debug, Clone)]
pub struct PolSurf {
    pub kind: PolKind,
    /// 面の前の媒質の内部透過率（強度、1 mm あたり）。吸収がなければ None
    pub it: Option<f64>,
}

/// 1 本の光線の偏光の状態: 電場（偏光の入力ごと、今いる面の局所座標）
pub struct PolRay<'a> {
    pub e: Vec<CV3>,
    pub surf: &'a [PolSurf],
    /// 係数を求められなかったとき（膜の表の範囲外など）
    pub err: Option<Error>,
}

#[inline]
fn cmv(m: &M3, e: CV3) -> CV3 {
    [0, 1, 2].map(|i| e[0] * m[i][0] + e[1] * m[i][1] + e[2] * m[i][2])
}
#[inline]
fn cmtv(m: &M3, e: CV3) -> CV3 {
    [0, 1, 2].map(|i| e[0] * m[0][i] + e[1] * m[1][i] + e[2] * m[2][i])
}
#[inline]
fn cdot(e: CV3, v: V3) -> C64 {
    e[0] * v[0] + e[1] * v[1] + e[2] * v[2]
}
#[inline]
fn comb(a: C64, u: V3, b: C64, v: V3) -> CV3 {
    [0, 1, 2].map(|i| a * u[i] + b * v[i])
}

const CNAN3: CV3 = [C64::new(f64::NAN, f64::NAN); 3];

/// 面での電場の変化（局所座標）。kin・kl は面の前後の方向、nv は法線
fn pol_surface(
    ps: &PolSurf,
    e: &mut [CV3],
    kin: V3,
    kl: V3,
    nv: V3,
    cosi: f64,
    mirror: bool,
) -> Result<(), Error> {
    match &ps.kind {
        PolKind::None => {}
        PolKind::Jones(m) => {
            // OpticStudio は、局所の ±z 軸を光線へ移す最小の回転が x・y を運んだ横の基底で行列をかける
            let sz = if kl[2] > 0.0 {
                1.0
            } else if kl[2] < 0.0 {
                -1.0
            } else {
                0.0
            };
            let h = scale(add(kl, [0.0, 0.0, sz]), 1.0 / (1.0 + kl[2].abs()));
            let u = sub([1.0, 0.0, 0.0], scale(h, kl[0]));
            let v = sub([0.0, 1.0, 0.0], scale(h, kl[1]));
            for x in e.iter_mut() {
                let (eu, ev) = (cdot(*x, u), cdot(*x, v));
                *x = comb(m[0] * eu + m[1] * ev, u, m[2] * eu + m[3] * ev, v);
            }
        }
        PolKind::Iface(f) => {
            let mut sv = cross(kin, nv);
            if !(norm(sv) > 1e-12) {
                sv = cross(kin, [1.0, 0.0, 0.0]);
            }
            let sv = normalize(sv);
            let pin = cross(sv, kin);
            let mut pout = cross(sv, kl);
            if mirror {
                pout = scale(pout, -1.0);
            }
            let th = cosi.abs().clamp(0.0, 1.0).acos();
            let tc = f.eval(th)?;
            for x in e.iter_mut() {
                let (es, ep) = (cdot(*x, sv), cdot(*x, pin));
                *x = comb(tc[0] * es, sv, tc[1] * ep, pout);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct RayOut {
    /// 最後の面の局所座標での位置・方向（方向は物理的な向き）
    pub xyz: V3,
    pub lmn: V3,
    /// 幾何的な光路長（理想レンズの位相を含まない）
    pub opl: f64,
    pub err: usize,
    pub vig: usize,
    /// 大域の位置・方向
    pub p: V3,
    pub k: V3,
    /// 理想レンズの位相（OPD に足す）
    pub lens_opd: f64,
}

/// 面ごとの記録
#[derive(Debug, Clone, Copy)]
pub struct RecEntry {
    pub surf: usize,
    pub xyz: V3,
    pub lmn: V3,
    pub opl: f64,
    pub p: V3,
}

/// 偶数次非球面のサグと d サグ / d(r^2)
pub fn sag(c: f64, k: f64, asph: &[f64], r2: f64) -> (f64, f64) {
    let arg = 1.0 - (1.0 + k) * c * c * r2;
    let root = if c != 0.0 { arg.max(0.0).sqrt() } else { 1.0 };
    let mut z = c * r2 / (1.0 + root);
    let mut dz = if c != 0.0 {
        if arg > 0.0 {
            c / (2.0 * root)
        } else {
            f64::NAN
        }
    } else {
        0.0
    };
    for (i, &a) in asph.iter().enumerate() {
        if a != 0.0 {
            let i = (i + 1) as i32;
            z += a * r2.powi(i);
            dz += i as f64 * a * r2.powi(i - 1);
        }
    }
    (z, dz)
}

const NAN3: V3 = [f64::NAN; 3];

/// 理想レンズ（焦点距離 f の薄肉、z = 0 面）: 新しい方向と OPD 用の位相
fn paraxial(f: f64, pl: V3, kl: V3, z_obj: f64) -> (V3, f64) {
    let sz = kl[2].signum();
    let t = [kl[0] / kl[2].abs(), kl[1] / kl[2].abs()];
    let r = [pl[0], pl[1]];
    let t2 = [t[0] - r[0] / f, t[1] - r[1] / f];
    let nn = (1.0 + t2[0] * t2[0] + t2[1] * t2[1]).sqrt();
    let k2 = [t2[0] / nn, t2[1] / nn, sz / nn];
    let rr = [r[0], r[1], 0.0];
    if !z_obj.is_finite() || z_obj.abs() > 1e12 {
        let ff = [f * t[0], f * t[1], f];
        let kin = scale(kl, sz);
        return (
            k2,
            f.signum() * (norm(ff) - norm(sub(ff, rr)) - dot(kin, rr)),
        );
    }
    let p = [r[0] + t[0] * z_obj, r[1] + t[1] * z_obj, z_obj];
    let zi = 1.0 / (1.0 / f + 1.0 / z_obj);
    let pi = [p[0] * (zi / z_obj), p[1] * (zi / z_obj), zi];
    let s_in = (-z_obj).signum() * norm(sub(rr, p));
    let s_out = zi.signum() * norm(sub(pi, rr));
    let s_in0 = (-z_obj).signum() * norm(p);
    let s_out0 = zi.signum() * norm(pi);
    (k2, (s_in0 + s_out0) - (s_in + s_out))
}

/// 大域の位置 p・方向 k、光路長 opl の光線を、面 first の直前から面 last まで、波長 w で追跡する
#[allow(clippy::too_many_arguments)]
pub fn trace_ray(
    sys: &System,
    fr: &Frames,
    w: usize,
    p: V3,
    k: V3,
    opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    rec: Option<&mut Vec<RecEntry>>,
) -> RayOut {
    trace_core(sys, fr, w, p, k, opl, first, last, floating_clip, rec, None)
}

/// trace_ray に偏光を加えたもの（電場 pol.e を面ごとに更新する）
#[allow(clippy::too_many_arguments)]
pub fn trace_ray_pol(
    sys: &System,
    fr: &Frames,
    w: usize,
    p: V3,
    k: V3,
    opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    rec: Option<&mut Vec<RecEntry>>,
    pol: &mut PolRay,
) -> RayOut {
    trace_core(
        sys,
        fr,
        w,
        p,
        k,
        opl,
        first,
        last,
        floating_clip,
        rec,
        Some(pol),
    )
}

// 偏光なしの追跡に偏光の分岐が残らないよう、呼び出し側ごとに展開する
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn trace_core(
    sys: &System,
    fr: &Frames,
    w: usize,
    mut p: V3,
    mut k: V3,
    mut opl: f64,
    first: usize,
    last: usize,
    floating_clip: bool,
    mut rec: Option<&mut Vec<RecEntry>>,
    mut pol: Option<&mut PolRay>,
) -> RayOut {
    let sf = &sys.surfaces;
    let mut err = 0usize;
    let mut vig = 0usize;
    let mut nprev = sys.index_before(first, w);
    let mut lens_opd = 0.0;
    let mut loc = (NAN3, NAN3);
    // 直前に通った面の番号（その局所座標 loc から次の面へ直接移る）
    let mut prev: Option<usize> = None;
    let mut last_frame: Option<Frame> = None;
    for j in first..=last {
        let d = &sf[j];
        if d.ignore {
            continue;
        }
        let f = fr.surf[j].unwrap();
        let (o, rm) = (f.o, f.r);
        last_frame = Some(f);
        if err != 0 {
            let pl = NAN3;
            loc = (pl, NAN3);
            p = NAN3;
            k = NAN3;
            prev = Some(j);
            if let Some(r) = rec.as_deref_mut() {
                r.push(RecEntry {
                    surf: j,
                    xyz: pl,
                    lmn: NAN3,
                    opl: f64::NAN,
                    p: NAN3,
                });
            }
            continue;
        }
        let (mut pl, mut kl) = match (prev, fr.rel[j]) {
            (
                Some(_),
                Some(Rel {
                    shift: Some(dz), ..
                }),
            ) => ([loc.0[0], loc.0[1], loc.0[2] + dz], loc.1),
            (Some(_), Some(q)) => (add(mv(&q.r, loc.0), q.t), mv(&q.r, loc.1)),
            _ => (mtv(&rm, sub(p, o)), mtv(&rm, k)),
        };
        // 電場も方向と同じ変換で今の面の局所座標へ
        if let Some(pr) = pol.as_deref_mut() {
            match (prev, fr.rel[j]) {
                (Some(_), Some(Rel { shift: Some(_), .. })) => {}
                (Some(_), Some(q)) => pr.e.iter_mut().for_each(|e| *e = cmv(&q.r, *e)),
                _ => pr.e.iter_mut().for_each(|e| *e = cmtv(&rm, *e)),
            }
        }
        prev = Some(j);
        // 頂点面へ
        let s = -pl[2] / kl[2];
        if !s.is_finite() {
            err = j;
        }
        pl = add(pl, scale(kl, s));
        opl += nprev * s;
        let mut s_tot = s;
        let curved = matches!(d.typ, SurfType::Standard | SurfType::EvenAsph)
            && (d.c != 0.0 || !d.asph.is_empty());
        if err == 0 && curved {
            let (c, kc) = (d.c, d.k);
            let mut s = 0.0;
            if c != 0.0 {
                let ff = c * (pl[0] * pl[0] + pl[1] * pl[1]);
                let gg = kl[2] - c * (pl[0] * kl[0] + pl[1] * kl[1]);
                let aa = c * (kl[0] * kl[0] + kl[1] * kl[1] + (1.0 + kc) * kl[2] * kl[2]);
                let disc = gg * gg - aa * ff;
                if !(disc >= 0.0) {
                    err = j;
                }
                s = ff / (gg + gg.signum() * disc.max(0.0).sqrt());
                if !s.is_finite() && err == 0 {
                    err = j;
                }
            }
            if d.has_asph() && err == 0 {
                let mut ds = 0.0;
                for _ in 0..60 {
                    let q = add(pl, scale(kl, s));
                    let r2 = q[0] * q[0] + q[1] * q[1];
                    let (z, dz) = sag(c, kc, &d.asph, r2);
                    ds = (q[2] - z) / (kl[2] - 2.0 * dz * (q[0] * kl[0] + q[1] * kl[1]));
                    s -= ds;
                    if !(ds.abs() > 1e-14) {
                        break;
                    }
                }
                if !(ds.abs() <= 1e-10) {
                    err = j;
                }
            }
            pl = add(pl, scale(kl, s));
            opl += nprev * s;
            s_tot += s;
        }
        // 通ってきた媒質の吸収（振幅は内部透過率の 1/2 乗）
        if let Some(pr) = pol.as_deref_mut() {
            if let Some(t) = pr.surf[j].it {
                let a = t.powf(s_tot / 2.0);
                pr.e.iter_mut().for_each(|e| *e = e.map(|c| c * a));
            }
        }
        // 開口
        let clip = match d.ap {
            Aperture::None => None,
            Aperture::Floating { .. } if !floating_clip => None,
            a => Some(a),
        };
        if let Some(a) = clip {
            let out = match a {
                Aperture::Circular { min, max, dx, dy } => {
                    let r = (pl[0] - dx).hypot(pl[1] - dy);
                    r > max + 1e-9 || r < min - 1e-9
                }
                Aperture::Floating { semi } => (pl[0]).hypot(pl[1]) > semi + 1e-9,
                Aperture::Obsc { min, max, dx, dy } => {
                    let r = (pl[0] - dx).hypot(pl[1] - dy);
                    r <= max + 1e-9 && r >= min - 1e-9
                }
                Aperture::Rect { hx, hy, dx, dy } => {
                    (pl[0] - dx).abs() > hx + 1e-9 || (pl[1] - dy).abs() > hy + 1e-9
                }
                Aperture::None => false,
            };
            if out && vig == 0 && err == 0 {
                vig = j;
            }
        }
        let mut nn = d.n[w];
        match d.typ {
            SurfType::CoordBrk => nn = nprev,
            SurfType::Paraxial => {
                let (k2, dl) = paraxial(d.focal, pl, kl, sys.lens_conjugate(j));
                kl = k2;
                lens_opd += dl;
            }
            _ => {
                let nvec = if d.typ == SurfType::EvenAsph && d.has_asph() {
                    let (_, dz) = sag(d.c, d.k, &d.asph, pl[0] * pl[0] + pl[1] * pl[1]);
                    if !dz.is_finite() && err == 0 {
                        err = j;
                    }
                    [-2.0 * dz * pl[0], -2.0 * dz * pl[1], 1.0]
                } else if d.c == 0.0 {
                    [0.0, 0.0, 1.0]
                } else {
                    [-d.c * pl[0], -d.c * pl[1], 1.0 - d.c * (1.0 + d.k) * pl[2]]
                };
                let nv = normalize(nvec);
                let cosi = dot(kl, nv);
                let kin = kl;
                if d.mirror {
                    kl = sub(kl, scale(nv, 2.0 * cosi));
                    nn = nprev;
                } else if d.typ != SurfType::Jones && nn != nprev {
                    let mu = nprev / nn;
                    let disc = 1.0 - mu * mu * (1.0 - cosi * cosi);
                    if disc < 0.0 && err == 0 {
                        err = j;
                    }
                    kl = add(
                        scale(kl, mu),
                        scale(nv, cosi.signum() * disc.max(0.0).sqrt() - mu * cosi),
                    );
                }
                if let Some(pr) = pol.as_deref_mut() {
                    if err == 0 {
                        if let Err(e) =
                            pol_surface(&pr.surf[j], &mut pr.e, kin, kl, nv, cosi, d.mirror)
                        {
                            pr.err.get_or_insert(e);
                        }
                    }
                }
            }
        }
        if err != 0 {
            pl = NAN3;
            kl = NAN3;
            opl = f64::NAN;
        }
        nprev = nn;
        loc = (pl, kl);
        if let Some(r) = rec.as_deref_mut() {
            p = add(o, mv(&rm, pl));
            r.push(RecEntry {
                surf: j,
                xyz: pl,
                lmn: kl,
                opl,
                p,
            });
        }
    }
    if let Some(pr) = pol {
        if err != 0 || pr.err.is_some() {
            pr.e.iter_mut().for_each(|e| *e = CNAN3);
        }
    }
    // 大域の位置・方向は最後の面でだけ求める
    if let Some(f) = last_frame {
        p = add(f.o, mv(&f.r, loc.0));
        k = mv(&f.r, loc.1);
    }
    RayOut {
        xyz: loc.0,
        lmn: loc.1,
        opl,
        err,
        vig,
        p,
        k,
        lens_opd,
    }
}
