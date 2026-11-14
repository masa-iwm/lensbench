//! 最後の面での光線の着地点・方向の、系のパラメータによる偏微分 d(x, y, l, m)/dpと、
//! 検出面を最後の面から dz ずらした面に置く追跡の後処理。
//!
//! 着地点の微分は、始点（物点、無限遠物体では始点の面の上の点）と開始方向を固定したもの（光線を同じに打ち出し、
//! 系だけを変える）。dOPL/dp（[`crate::fermat`]、端点を固定）とは固定するものが違う。座標は物体に固定した
//! 座標系で扱い、結果は最後の面（dz を与えたらずらした面）の局所座標の x・y と方向余弦 l・m。
//!
//! 方法: 1 回の追跡の記録から、面ごとの線形の写像（光線の変化 (dX, dK) → 次の面の後の (dX, dK)）と、
//! パラメータが面に与える変化（座標系の剛体運動・面の形・媒質の屈折率）からの写像を作る。最後の面から
//! 逆向きに随伴 W_k = dy/dσ_k を掛けて溜め、パラメータごとには作用する面の分だけを足す
//! （剛体運動は opl_grad と同じく、同じ運動が続く範囲を累積和でまとめる）。光線 1 本あたり O(面 + パラメータ)。
//!
//! 対象のパラメータは dOPL/dp と同じ。理想レンズの面を通る追跡と、トロイダル面の形のパラメータは未対応。

use crate::error::Result;
use crate::fermat::{dsag, raw_frames_d, PrepParam};
use crate::math::*;
use crate::par::scoped;
use crate::param::{Kind, Param};
use crate::surface::SurfType;
use crate::system::Trace;
use crate::trace::{sag, sag_toroidal};
use crate::System;
use crate::{invalid, notimpl};
use rayon::prelude::*;

/// 出力の数（x, y, l, m）
const NY: usize = 4;

/// 回転の微分 Ω（反対称、Ω X = ω × X）の ω
fn omega_vec(om: &M3) -> V3 {
    [
        0.5 * (om[2][1] - om[1][2]),
        0.5 * (om[0][2] - om[2][0]),
        0.5 * (om[1][0] - om[0][1]),
    ]
}

/// 回転対称のサグ z(u)（u = r²）の dz/du を、形のパラメータで微分したもの
fn dsag_u(c: f64, k: f64, kind: Kind, u: f64) -> f64 {
    let root = (1.0 - (1.0 + k) * c * c * u).max(0.0).sqrt();
    match kind {
        Kind::Curv | Kind::Radius => {
            let v = 0.5 / root + c * c * (1.0 + k) * u / (2.0 * root * root * root);
            if kind == Kind::Radius {
                -c * c * v
            } else {
                v
            }
        }
        Kind::Conic => c * c * c * u / (4.0 * root * root * root),
        Kind::Asph(i) => (i as f64 + 1.0) * u.powi(i as i32),
        _ => 0.0,
    }
}

/// 回転対称のサグの d²z/du²
fn sag_uu(c: f64, k: f64, asph: &[f64], u: f64) -> f64 {
    let a = (1.0 + k) * c * c;
    let root = (1.0 - a * u).max(0.0).sqrt();
    let mut v = if c != 0.0 {
        c * a / (4.0 * root * root * root)
    } else {
        0.0
    };
    for (i, &ai) in asph.iter().enumerate() {
        let p = (i + 1) as i32;
        if ai != 0.0 && p >= 2 {
            v += ai * (p * (p - 1)) as f64 * u.powi(p - 2);
        }
    }
    v
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Refract,
    Mirror,
    Pass,
}

/// 1 本の光線の、1 つの面での幾何（局所座標）
#[derive(Clone, Copy)]
struct Geo {
    /// 面の座標系の回転（物体に固定した座標系）
    r: M3,
    /// 交点（局所）
    q: V3,
    kin: V3,
    kout: V3,
    /// 前の点から交点までの道のり
    s: f64,
    /// 面の勾配 (−z_x, −z_y, 1) と、サグの 2 階微分 (z_xx, z_xy, z_yy)
    g: V3,
    h: [f64; 3],
    mode: Mode,
    nin: f64,
    nout: f64,
}

/// [`local_map`] の、入力によらない量（面ごと・光線ごとに 1 回だけ求める）
#[derive(Clone, Copy)]
struct LocalPre {
    gk: f64,
    gn: f64,
    nv: V3,
    cosi: f64,
    mu: f64,
    one_c2: f64,
    r: f64,
    sg: f64,
    gam: f64,
}

impl LocalPre {
    fn new(g: &Geo) -> LocalPre {
        let gk = dot(g.g, g.kin);
        let gn = norm(g.g);
        let nv = scale(g.g, 1.0 / gn);
        let cosi = dot(g.kin, nv);
        let mu = g.nin / g.nout;
        let one_c2 = 1.0 - cosi * cosi;
        let disc = 1.0 - mu * mu * one_c2;
        let r = disc.max(0.0).sqrt();
        let sg = cosi.signum();
        let gam = sg * r - mu * cosi;
        LocalPre {
            gk,
            gn,
            nv,
            cosi,
            mu,
            one_c2,
            r,
            sg,
            gam,
        }
    }
}

/// 局所座標の線形の写像: 入る光線の変化 (dx, dk)、面の形の変化 (dz, dz_x, dz_y)、屈折率の変化
/// (dn_in, dn_out) → 交点の変化 dq と出ていく方向の変化 dk'。p は [`LocalPre::new`]`(g)`
fn local_map(g: &Geo, p: &LocalPre, dx: V3, dk: V3, sh: [f64; 3], dn: [f64; 2]) -> (V3, V3) {
    let ds = -(dot(g.g, add(dx, scale(dk, g.s))) - sh[0]) / p.gk;
    let dq = add(add(dx, scale(dk, g.s)), scale(g.kin, ds));
    if g.mode == Mode::Pass {
        return (dq, dk);
    }
    let dg = [
        -(g.h[0] * dq[0] + g.h[1] * dq[1] + sh[1]),
        -(g.h[1] * dq[0] + g.h[2] * dq[1] + sh[2]),
        0.0,
    ];
    let (gn, nv, cosi) = (p.gn, p.nv, p.cosi);
    let dnv = scale(sub(dg, scale(nv, dot(nv, dg))), 1.0 / gn);
    let dcosi = dot(dk, nv) + dot(g.kin, dnv);
    let dk2 = match g.mode {
        Mode::Mirror => sub(dk, scale(add(scale(nv, dcosi), scale(dnv, cosi)), 2.0)),
        _ => {
            let mu = p.mu;
            let dmu = (dn[0] - mu * dn[1]) / g.nout;
            let r = p.r;
            let dr = (-mu * dmu * p.one_c2 + mu * mu * cosi * dcosi) / r;
            let gam = p.gam;
            let dgam = p.sg * dr - dmu * cosi - mu * dcosi;
            add(
                add(scale(g.kin, dmu), scale(dk, mu)),
                add(scale(nv, dgam), scale(dnv, gam)),
            )
        }
    };
    (dq, dk2)
}

/// 列優先 [入力][出力]、列の長さは出力の数（途中の面 6、最後の面 NY）
struct SurfMap {
    /// dσ_out/dσ_in（最後の面では dy/dσ_in）
    j: [f64; 36],
    /// 剛体運動 (a, ω) から
    rig: [f64; 36],
    /// 形 (dz, dz_x, dz_y) から
    shape: [f64; 18],
    /// 屈折率 (dn_in, dn_out) から
    dn: [f64; 12],
}

impl System {
    /// 面ごとの記録つきの追跡 t（全面の記録、波長 w）から、光線ごとの最後の面での d(x, y, l, m)/dp を求める。
    /// `dz` を与えると、最後の面の局所座標の z = dz の平面での値（[`System::shift_end`] と同じ面）。
    /// 結果は行優先 [光線][パラメータ][x, y, l, m]。止まった光線は NaN。
    /// ピックアップで一緒に動く面の分も足す（[`System::grad_with_pickups`]）
    pub fn land_grad(
        &self,
        t: &Trace,
        w: usize,
        params: &[Param],
        dz: Option<f64>,
    ) -> Result<Vec<f64>> {
        let n = t.rays.len();
        // grad_with_pickups は [行][パラメータ] の形を扱うので、[光線][成分][パラメータ] に並べ替えて渡す
        let to_rows = |g: Vec<f64>, np: usize| -> Vec<f64> {
            let mut r = vec![0.0; g.len()];
            for i in 0..n {
                for q in 0..np {
                    for c in 0..NY {
                        r[(i * NY + c) * np + q] = g[(i * np + q) * NY + c];
                    }
                }
            }
            r
        };
        let rows = self.grad_with_pickups(params, n * NY, |p| {
            Ok(to_rows(self.land_grad_raw(t, w, p, dz)?, p.len()))
        })?;
        let np = params.len();
        let mut out = vec![0.0; rows.len()];
        for i in 0..n {
            for q in 0..np {
                for c in 0..NY {
                    out[(i * np + q) * NY + c] = rows[(i * NY + c) * np + q];
                }
            }
        }
        Ok(out)
    }

    fn land_grad_raw(
        &self,
        t: &Trace,
        w: usize,
        params: &[Param],
        dz: Option<f64>,
    ) -> Result<Vec<f64>> {
        let n = t.rays.len();
        let np = params.len();
        let sf = &self.surfaces;
        let all = (1..=t.last).filter(|&j| !sf[j].ignore).count();
        if n > 0 && t.rec.len() != all {
            invalid!("着地点の微分には全面の記録（record）つきの追跡が要る");
        }
        for j in 1..=t.last {
            let d = &sf[j];
            if !d.ignore && matches!(d.typ, SurfType::Paraxial | SurfType::User) {
                notimpl!("理想レンズ・利用者定義の面（面 {j}）を通る光線の着地点の微分");
            }
            if !d.ignore
                && matches!(
                    d.typ,
                    SurfType::Fresnel
                        | SurfType::CylFresnel
                        | SurfType::Binary2
                        | SurfType::Gradium
                        | SurfType::Gradient9
                )
            {
                notimpl!("面 {j}（{:?}）を通る光線の着地点の微分", d.typ);
            }
        }
        for p in params {
            if sf.get(p.surf).is_some_and(|d| d.typ == SurfType::Toroidal)
                && matches!(
                    p.kind,
                    Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_)
                )
            {
                notimpl!(
                    "トロイダル面（面 {}）の形のパラメータの着地点の微分",
                    p.surf
                );
            }
        }
        let owner = self.medium_owner();
        let pre: Vec<PrepParam> = params
            .iter()
            .map(|&p| self.prep_param(p, w, &owner))
            .collect::<Result<_>>()?;
        let ns = t.rec.len();
        if n == 0 || np == 0 {
            return Ok(vec![0.0; n * np * NY]);
        }
        if ns == 0 {
            invalid!("着地点の微分: 追跡する面がない");
        }
        let fr = self.frames();
        let base = raw_frames_d(self, Param::new(0, Kind::Dn));
        let br: Vec<(V3, M3)> = t
            .rec
            .iter()
            .map(|rs| {
                let f = base[rs.surf].expect("記録した面の座標系");
                (f.o, f.r)
            })
            .collect();
        let mut n_in = vec![0.0; ns];
        let mut n_out = vec![0.0; ns];
        let mut own_in = vec![0usize; ns];
        for (k, rs) in t.rec.iter().enumerate() {
            let j = rs.surf;
            n_in[k] = self.index_before(j, w);
            let d = &sf[j];
            n_out[k] = if d.mirror || d.typ == SurfType::CoordBrk {
                n_in[k]
            } else {
                d.n[w]
            };
            own_in[k] = (0..j)
                .rev()
                .find(|&i| i == 0 || (!sf[i].ignore && sf[i].typ != SurfType::CoordBrk))
                .unwrap_or(0);
        }
        // パラメータごとの剛体運動の範囲 [s, e)（a, ω）
        let same = |p: &(V3, V3), q: &(V3, V3)| {
            (0..3).all(|i| (p.0[i] - q.0[i]).abs() <= 1e-12 * (1.0 + p.0[i].abs()))
                && (0..3).all(|i| (p.1[i] - q.1[i]).abs() <= 1e-14 * (1.0 + p.1[i].abs()))
        };
        let runs: Vec<Vec<(usize, usize, V3, V3)>> = pre
            .iter()
            .map(|pp| {
                let mut out: Vec<(usize, usize, V3, V3)> = Vec::new();
                for (k, rs) in t.rec.iter().enumerate() {
                    let Some((a, om)) = pp.glob[rs.surf] else {
                        continue;
                    };
                    let m = (a, omega_vec(&om));
                    if let Some(r) = out.last_mut() {
                        if r.1 == k && same(&(r.2, r.3), &m) {
                            r.1 = k + 1;
                            continue;
                        }
                    }
                    out.push((k, k + 1, m.0, m.1));
                }
                out
            })
            .collect();
        // 屈折率の変化がかかる面（dn_in, dn_out）
        let dn_ks: Vec<Vec<(usize, [f64; 2])>> = pre
            .iter()
            .map(|pp| {
                (0..ns)
                    .filter_map(|k| {
                        let j = t.rec[k].surf;
                        let a = pp.dn[own_in[k]];
                        let b = if sf[j].mirror || sf[j].typ == SurfType::CoordBrk {
                            a
                        } else {
                            pp.dn[j]
                        };
                        (a != 0.0 || b != 0.0).then_some((k, [a, b]))
                    })
                    .collect()
            })
            .collect();
        // 形のパラメータがかかる面
        let sag_k: Vec<Option<usize>> = pre
            .iter()
            .map(|pp| {
                let shape =
                    matches!(
                        pp.kind,
                        Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_) | Kind::Zern(_)
                    ) && (matches!(sf[pp.surf].typ, SurfType::Standard | SurfType::EvenAsph)
                        || crate::freeform::is_freeform(sf[pp.surf].typ));
                if shape {
                    t.rec.iter().position(|rs| rs.surf == pp.surf)
                } else {
                    None
                }
            })
            .collect();
        let j0 = t.rec[0].surf;
        let r_first_glrs = fr.surf[j0].expect("面の座標系").r;
        // 形・屈折率の写像が要る面（要らない面では求めない。値には効かない）
        let mut need_shape = vec![false; ns];
        for k in sag_k.iter().flatten() {
            need_shape[*k] = true;
        }
        let mut need_dn = vec![false; ns];
        for v in &dn_ks {
            for &(k, _) in v {
                need_dn[k] = true;
            }
        }
        // どのパラメータも効かない手前の面は、写像も随伴も要らない（そこより前の値は使わない）
        let k_min = (0..np)
            .map(|q| {
                let mut m = ns;
                if pre[q].d0 != [0.0; 3] {
                    m = 0;
                }
                if let Some(r) = runs[q].first() {
                    m = m.min(r.0);
                }
                if let Some(&(k, _)) = dn_ks[q].first() {
                    m = m.min(k);
                }
                if let Some(k) = sag_k[q] {
                    m = m.min(k);
                }
                m
            })
            .min()
            .unwrap_or(ns);

        let one = |i: usize| -> Vec<f64> {
            if t.rays[i].err != 0 {
                return vec![f64::NAN; np * NY];
            }
            // 面ごとの幾何
            let mut geo: Vec<Geo> = Vec::with_capacity(ns);
            let mut kprev_g = mv(&br[0].1, mtv(&r_first_glrs, t.k0[i]));
            let mut opl_prev = 0.0;
            let mut xg: Vec<V3> = Vec::with_capacity(ns);
            let mut kg: Vec<V3> = Vec::with_capacity(ns);
            for (k, rs) in t.rec.iter().enumerate() {
                let d = &sf[rs.surf];
                let (o, r) = br[k];
                let q = rs.xyz[i];
                let kout = rs.lmn[i];
                let kin = mtv(&r, kprev_g);
                let s = (rs.opl[i] - opl_prev) / n_in[k];
                opl_prev = rs.opl[i];
                let plane = matches!(d.typ, SurfType::CoordBrk | SurfType::Jones);
                let (g, h) = if plane {
                    ([0.0, 0.0, 1.0], [0.0; 3])
                } else if crate::freeform::is_freeform(d.typ) {
                    let (_, zx, zy) = crate::freeform::sag_xy(d, q[0], q[1]);
                    // 2 階微分は勾配の中心差分（Grid Sag の升目の境では片側に寄らない平均になる）
                    let hh = 1e-5 * (1.0 + q[0].abs().max(q[1].abs()));
                    let gx = |x: f64, y: f64| {
                        let (_, a, b) = crate::freeform::sag_xy(d, x, y);
                        (a, b)
                    };
                    let (ax1, bx1) = gx(q[0] + hh, q[1]);
                    let (ax0, bx0) = gx(q[0] - hh, q[1]);
                    let (_, by1) = gx(q[0], q[1] + hh);
                    let (_, by0) = gx(q[0], q[1] - hh);
                    let zxx = (ax1 - ax0) / (2.0 * hh);
                    let zxy = (bx1 - bx0) / (2.0 * hh);
                    let zyy = (by1 - by0) / (2.0 * hh);
                    ([-zx, -zy, 1.0], [zxx, zxy, zyy])
                } else if d.typ == SurfType::Toroidal {
                    let (_, zx, zy) = sag_toroidal(d.c, d.k, &d.asph, d.cx, q[0], q[1]);
                    // 2 階微分は勾配の中心差分
                    let hh = 1e-5 * (1.0 + q[0].abs().max(q[1].abs()));
                    let gx = |x: f64, y: f64| {
                        let (_, a, b) = sag_toroidal(d.c, d.k, &d.asph, d.cx, x, y);
                        (a, b)
                    };
                    let (ax1, bx1) = gx(q[0] + hh, q[1]);
                    let (ax0, bx0) = gx(q[0] - hh, q[1]);
                    let (_, by1) = gx(q[0], q[1] + hh);
                    let (_, by0) = gx(q[0], q[1] - hh);
                    let zxx = (ax1 - ax0) / (2.0 * hh);
                    let zxy = (bx1 - bx0) / (2.0 * hh);
                    let zyy = (by1 - by0) / (2.0 * hh);
                    ([-zx, -zy, 1.0], [zxx, zxy, zyy])
                } else {
                    let u = q[0] * q[0] + q[1] * q[1];
                    let (_, z1) = sag(d.c, d.k, &d.asph, u);
                    let z2 = sag_uu(d.c, d.k, &d.asph, u);
                    (
                        [-2.0 * z1 * q[0], -2.0 * z1 * q[1], 1.0],
                        [
                            2.0 * z1 + 4.0 * z2 * q[0] * q[0],
                            4.0 * z2 * q[0] * q[1],
                            2.0 * z1 + 4.0 * z2 * q[1] * q[1],
                        ],
                    )
                };
                let mode = if d.mirror {
                    Mode::Mirror
                } else if matches!(d.typ, SurfType::CoordBrk | SurfType::Jones) {
                    Mode::Pass
                } else {
                    Mode::Refract
                };
                geo.push(Geo {
                    r,
                    q,
                    kin,
                    kout,
                    s,
                    g,
                    h,
                    mode,
                    nin: n_in[k],
                    nout: n_out[k],
                });
                let x = add(o, mv(&r, q));
                let kk = mv(&r, kout);
                xg.push(x);
                kg.push(kk);
                kprev_g = kk;
            }
            // 面ごとの写像を作る（σ_in = (dX, dK) の前の点・方向は X_{k−1}, K_{k−1}）。
            // 配列は固定の大きさ（光線ごと・面ごとにメモリを確保しない）
            // 随伴: W（NY × 6、行優先）= dy/dσ_in(k)。Q_k = W_{k の出力} · (源の写像)。
            // 面の写像は後ろの面から作りながら掛ける（写像を溜めない）。
            // 列優先 [入力 c][出力 o] の写像 M に、W（NY × nout）を左から掛ける
            let mul = |w: &[f64], m: &[f64], nin: usize, nout: usize| -> [f64; NY * 6] {
                let mut r = [0.0; NY * 6];
                for y in 0..NY {
                    for c in 0..nin {
                        let mut v = 0.0;
                        for o in 0..nout {
                            v += w[y * nout + o] * m[c * nout + o];
                        }
                        r[y * nin + c] = v;
                    }
                }
                r
            };
            let mut q_shape = vec![[0.0; NY * 3]; ns];
            let mut q_dn = vec![[0.0; NY * 2]; ns];
            // 剛体運動の Q の後ろからの累積和（cum[k] = Σ_{j≥k} Q_j）
            let mut cum = vec![[0.0; NY * 6]; ns + 1];
            // 最後の面の出力そのものは y（W = 単位）
            let mut id = [0.0; NY * 6];
            for y in 0..NY {
                id[y * NY + y] = 1.0;
            }
            let mut w_cur = [0.0; NY * 6];
            for k in (k_min..ns).rev() {
                let gk = &geo[k];
                let xprev = if k == 0 {
                    sub(xg[0], scale(mv(&gk.r, gk.kin), gk.s))
                } else {
                    xg[k - 1]
                };
                let kprev = mv(&gk.r, gk.kin);
                let last = k + 1 == ns;
                let nout = if last { NY } else { 6 };
                let lp = LocalPre::new(gk);
                // 入力 (σ 6, 剛体 6, 形 3, 屈折率 2) → 出力（最後の面は先頭の NY 個）
                let apply =
                    |dxg: V3, dkg: V3, a: V3, om: V3, sh: [f64; 3], dn: [f64; 2]| -> [f64; 6] {
                        let u = sub(sub(dxg, a), cross(om, xprev));
                        let v = sub(dkg, cross(om, kprev));
                        let (dq, dk2) = local_map(gk, &lp, mtv(&gk.r, u), mtv(&gk.r, v), sh, dn);
                        if last {
                            let (ex, ey) = match dz {
                                None => (dq[0], dq[1]),
                                Some(z) => {
                                    let tt = (z - gk.q[2]) / gk.kout[2];
                                    let dt = -(dq[2] + tt * dk2[2]) / gk.kout[2];
                                    (
                                        dq[0] + dt * gk.kout[0] + tt * dk2[0],
                                        dq[1] + dt * gk.kout[1] + tt * dk2[1],
                                    )
                                }
                            };
                            [ex, ey, dk2[0], dk2[1], 0.0, 0.0]
                        } else {
                            let dx_out = add(add(a, cross(om, xg[k])), mv(&gk.r, dq));
                            let dk_out = add(cross(om, kg[k]), mv(&gk.r, dk2));
                            [
                                dx_out[0], dx_out[1], dx_out[2], dk_out[0], dk_out[1], dk_out[2],
                            ]
                        }
                    };
                let z3 = [0.0; 3];
                let e = |c: usize| {
                    let mut v = [0.0; 3];
                    v[c] = 1.0;
                    v
                };
                // 列優先 [入力][出力]（列の長さは nout）
                let put = |m: &mut [f64], c: usize, col: [f64; 6]| {
                    m[c * nout..(c + 1) * nout].copy_from_slice(&col[..nout]);
                };
                let mut sm = SurfMap {
                    j: [0.0; 36],
                    rig: [0.0; 36],
                    shape: [0.0; 18],
                    dn: [0.0; 12],
                };
                for c in 0..6 {
                    let col = if c < 3 {
                        apply(e(c), z3, z3, z3, [0.0; 3], [0.0; 2])
                    } else {
                        apply(z3, e(c - 3), z3, z3, [0.0; 3], [0.0; 2])
                    };
                    put(&mut sm.j, c, col);
                }
                // 剛体運動の列は写像が線形なので J の列から作る（local_map を呼び直さない）:
                // 平行移動 a は σ_in = (−a, 0) と出口の +a、回転 ω は σ_in = (−ω×X_{k−1}, −ω×K_{k−1}) と出口の (ω×X_k, ω×K_k)
                for c in 0..3 {
                    let mut col = [0.0; 6];
                    for o in 0..nout {
                        col[o] = -sm.j[c * nout + o];
                    }
                    if !last {
                        col[c] += 1.0;
                    }
                    put(&mut sm.rig, c, col);
                }
                for c in 0..3 {
                    let om = e(c);
                    let u = cross(om, xprev);
                    let v = cross(om, kprev);
                    let sig = [-u[0], -u[1], -u[2], -v[0], -v[1], -v[2]];
                    let mut col = [0.0; 6];
                    for (i, &si) in sig.iter().enumerate() {
                        if si != 0.0 {
                            for o in 0..nout {
                                col[o] += sm.j[i * nout + o] * si;
                            }
                        }
                    }
                    if !last {
                        let (a, b) = (cross(om, xg[k]), cross(om, kg[k]));
                        for i in 0..3 {
                            col[i] += a[i];
                            col[3 + i] += b[i];
                        }
                    }
                    put(&mut sm.rig, 3 + c, col);
                }
                if need_shape[k] {
                    for c in 0..3 {
                        let mut sh = [0.0; 3];
                        sh[c] = 1.0;
                        put(&mut sm.shape, c, apply(z3, z3, z3, z3, sh, [0.0; 2]));
                    }
                }
                if need_dn[k] {
                    for c in 0..2 {
                        let mut dn = [0.0; 2];
                        dn[c] = 1.0;
                        put(&mut sm.dn, c, apply(z3, z3, z3, z3, [0.0; 3], dn));
                    }
                }
                let m = &sm;
                let (wk, nout) = if last { (id, NY) } else { (w_cur, 6) };
                let q_rig = mul(&wk, &m.rig, 6, nout);
                for c in 0..NY * 6 {
                    cum[k][c] = cum[k + 1][c] + q_rig[c];
                }
                if need_shape[k] {
                    q_shape[k].copy_from_slice(&mul(&wk, &m.shape, 3, nout)[..NY * 3]);
                }
                if need_dn[k] {
                    q_dn[k].copy_from_slice(&mul(&wk, &m.dn, 2, nout)[..NY * 2]);
                }
                w_cur = mul(&wk, &m.j, 6, nout);
            }
            let mut out = vec![0.0; np * NY];
            for q in 0..np {
                let o = &mut out[q * NY..(q + 1) * NY];
                for &(s, e, a, om) in &runs[q] {
                    let src = [a[0], a[1], a[2], om[0], om[1], om[2]];
                    for y in 0..NY {
                        for c in 0..6 {
                            o[y] += (cum[s][y * 6 + c] - cum[e][y * 6 + c]) * src[c];
                        }
                    }
                }
                for &(k, dn) in &dn_ks[q] {
                    for y in 0..NY {
                        o[y] += q_dn[k][y * 2] * dn[0] + q_dn[k][y * 2 + 1] * dn[1];
                    }
                }
                if let Some(k) = sag_k[q] {
                    let d = &sf[t.rec[k].surf];
                    let x = geo[k].q;
                    let sh = match (pre[q].kind, &d.zern) {
                        (Kind::Zern(j), Some(z)) => {
                            let (a, ax, ay) = z.term(j as usize, x[0], x[1]);
                            [a, ax, ay]
                        }
                        _ => {
                            let u = x[0] * x[0] + x[1] * x[1];
                            let dzp = dsag(d.c, d.k, pre[q].kind, u);
                            let dz1 = dsag_u(d.c, d.k, pre[q].kind, u);
                            [dzp, 2.0 * dz1 * x[0], 2.0 * dz1 * x[1]]
                        }
                    };
                    for y in 0..NY {
                        o[y] += (0..3).map(|c| q_shape[k][y * 3 + c] * sh[c]).sum::<f64>();
                    }
                }
                // 物点の動き（開始方向は固定）
                let d0 = pre[q].d0;
                if d0 != [0.0; 3] {
                    for y in 0..NY {
                        o[y] += (0..3).map(|c| w_cur[y * 6 + c] * d0[c]).sum::<f64>();
                    }
                }
            }
            out
        };
        let rows: Vec<Vec<f64>> = if n >= 64 && !crate::par::sequential() {
            (0..n)
                .into_par_iter()
                .with_min_len(16)
                .map(scoped(one))
                .collect()
        } else {
            (0..n).map(one).collect()
        };
        Ok(rows.concat())
    }

    /// 最後の面（t.last）の局所座標の z = dz の平面まで、追跡の結果 t を進める。位置 xyz・大域の p・
    /// 光路長 opl（と OpticStudio の生の光路長）を書き換える。方向は変えない。記録（rec）は変えない。
    /// 平面はその面の座標系に固定（面の傾き・偏心に従い、その面の厚さには依らない）。
    /// 返り値は光線ごとの進んだ道のり（止まった光線は NaN）
    pub fn shift_end(&self, t: &mut Trace, w: usize, dz: f64) -> Result<Vec<f64>> {
        if !dz.is_finite() {
            invalid!("dz が有限でない（{dz}）");
        }
        let j = t.last;
        let d = &self.surfaces[j];
        let nout = if d.mirror || d.typ == SurfType::CoordBrk {
            self.index_before(j, w)
        } else {
            d.n[w]
        };
        let Some(f) = self.frames().surf[j] else {
            invalid!("面 {j} は無視する面");
        };
        let mut ts = Vec::with_capacity(t.rays.len());
        for (i, r) in t.rays.iter_mut().enumerate() {
            let tt = (dz - r.xyz[2]) / r.lmn[2];
            let e = add(r.xyz, scale(r.lmn, tt));
            r.xyz = [e[0], e[1], dz];
            if r.err != 0 {
                r.xyz = [f64::NAN; 3];
            }
            r.p = add(f.o, mv(&f.r, r.xyz));
            r.opl += nout * tt;
            if let Some(o) = t.opl_os.get_mut(i) {
                *o += nout * tt;
            }
            ts.push(tt);
        }
        Ok(ts)
    }

    /// dOPL/dp（[`System::opl_grad`]、最後の面の x・y を固定）を、最後の面から dz ずらした平面の x・y を固定した
    /// 微分に直す。g は opl_grad の結果（[光線][パラメータ]）。t は全面の記録つきの、ずらす前の追跡。
    /// 平面は最後の面の座標系に固定なので、剛体運動の寄与は打ち消し合い、最後の面の形と後の媒質の屈折率の分だけが残る。
    /// ピックアップで一緒に動く面の分も足す
    pub fn opl_grad_shift(
        &self,
        t: &Trace,
        w: usize,
        params: &[Param],
        dz: f64,
        g: &mut [f64],
    ) -> Result<()> {
        let corr = self.grad_with_pickups(params, t.rays.len(), |p| {
            let mut c = vec![0.0; t.rays.len() * p.len()];
            self.opl_grad_shift_raw(t, w, p, dz, &mut c)?;
            Ok(c)
        })?;
        for (a, b) in g.iter_mut().zip(corr) {
            *a += b;
        }
        Ok(())
    }

    fn opl_grad_shift_raw(
        &self,
        t: &Trace,
        w: usize,
        params: &[Param],
        dz: f64,
        g: &mut [f64],
    ) -> Result<()> {
        let np = params.len();
        let Some(rl) = t.rec.last() else {
            return Ok(());
        };
        let j = t.last;
        if rl.surf != j {
            invalid!("dz つきの dOPL/dp には最後の面までの全面の記録が要る");
        }
        let owner = self.medium_owner();
        let d = &self.surfaces[j];
        let mirror_like = d.mirror || d.typ == SurfType::CoordBrk;
        let nout = if mirror_like {
            self.index_before(j, w)
        } else {
            d.n[w]
        };
        let own_in = (0..j)
            .rev()
            .find(|&i| {
                i == 0 || (!self.surfaces[i].ignore && self.surfaces[i].typ != SurfType::CoordBrk)
            })
            .unwrap_or(0);
        for (q, &p) in params.iter().enumerate() {
            let pp = self.prep_param(p, w, &owner)?;
            let dn = if mirror_like { pp.dn[own_in] } else { pp.dn[j] };
            let shape = p.surf == j
                && matches!(
                    p.kind,
                    Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_) | Kind::Zern(_)
                )
                && (matches!(d.typ, SurfType::Standard | SurfType::EvenAsph)
                    || crate::freeform::is_freeform(d.typ));
            if dn == 0.0 && !shape {
                continue;
            }
            for i in 0..t.rays.len() {
                let x = rl.xyz[i];
                let k = rl.lmn[i];
                let tt = (dz - x[2]) / k[2];
                let mut v = dn * tt;
                if shape {
                    v -= nout * k[2] * crate::fermat::shape_dz(d, p.kind, x[0], x[1]);
                }
                g[i * np + q] += v;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sag_derivatives_match_differences() {
        let (c, k) = (0.02, -0.7);
        let asph = [1e-5, -2e-8, 3e-11];
        let u = 30.0;
        let h = 1e-4;
        let z1 = |c: f64, k: f64, a: &[f64], u: f64| sag(c, k, a, u).1;
        let num = (z1(c, k, &asph, u + h) - z1(c, k, &asph, u - h)) / (2.0 * h);
        let a = sag_uu(c, k, &asph, u);
        assert!((a - num).abs() < 1e-7 * num.abs(), "{a} {num}");
        let hc = 1e-7;
        let num = (z1(c + hc, k, &asph, u) - z1(c - hc, k, &asph, u)) / (2.0 * hc);
        assert!((dsag_u(c, k, Kind::Curv, u) - num).abs() < 1e-6 * num.abs());
        let num = (z1(c, k + hc, &asph, u) - z1(c, k - hc, &asph, u)) / (2.0 * hc);
        assert!((dsag_u(c, k, Kind::Conic, u) - num).abs() < 1e-6 * num.abs().max(1e-9));
        let r = 1.0 / c;
        let num = (z1(1.0 / (r + 1e-4), k, &asph, u) - z1(1.0 / (r - 1e-4), k, &asph, u)) / 2e-4;
        assert!((dsag_u(c, k, Kind::Radius, u) - num).abs() < 1e-6 * num.abs());
        let mut a2 = asph;
        a2[1] += 1e-9;
        let num = (z1(c, k, &a2, u) - z1(c, k, &asph, u)) / 1e-9;
        assert!((dsag_u(c, k, Kind::Asph(1), u) - num).abs() < 1e-5 * num.abs());
    }
}
