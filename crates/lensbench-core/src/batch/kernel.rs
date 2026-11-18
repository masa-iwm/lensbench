//! 追跡のカーネル（組の幅 V についてジェネリック。幅 1 が 1 本ずつの追跡）

use super::lanes::{sel, Lanes, Mask};
use super::pol::{aperture_out_l, pol_surface_l, PolLanes, PolSoa};
use super::{LaneSink, Move, Plan, Sag, Step};
use crate::action::{Act, Action};
use crate::freeform::ZernSag;
use crate::math::*;
use crate::shape::{Isect, Norm, Shape};
use crate::surface::Aperture;
use crate::trace::{self, RayOut, RayStatus, RecEntry};

/// 3 成分の組
pub(super) type VL<V> = [V; 3];

#[inline(always)]
pub(super) fn sel3<V: Lanes>(m: V::B, a: VL<V>, b: VL<V>) -> VL<V> {
    [sel(m, a[0], b[0]), sel(m, a[1], b[1]), sel(m, a[2], b[2])]
}

/// a + b·s（光線ごとの s）
#[inline(always)]
fn axpy<V: Lanes>(a: VL<V>, b: VL<V>, s: V) -> VL<V> {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// M v（[`mv`] と同じ順）
#[inline(always)]
pub(super) fn mv_l<V: Lanes>(m: &M3, v: &VL<V>) -> VL<V> {
    let row = |r: usize| v[0] * m[r][0] + v[1] * m[r][1] + v[2] * m[r][2];
    [row(0), row(1), row(2)]
}

/// Mᵀ v（[`mtv`] と同じ順）
#[inline(always)]
pub(super) fn mtv_l<V: Lanes>(m: &M3, v: &VL<V>) -> VL<V> {
    let col = |r: usize| v[0] * m[0][r] + v[1] * m[1][r] + v[2] * m[2][r];
    [col(0), col(1), col(2)]
}

/// a · b（[`dot`] と同じ順）
#[inline(always)]
pub(super) fn dot_l<V: Lanes>(a: &VL<V>, b: &VL<V>) -> V {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline(always)]
pub(super) fn get<V: Lanes>(v: &VL<V>, i: usize) -> V3 {
    [v[0][i], v[1][i], v[2][i]]
}

#[inline(always)]
fn put<V: Lanes>(v: &mut VL<V>, i: usize, x: V3) {
    for c in 0..3 {
        v[c][i] = x[c];
    }
}

/// [`trace::powi`] の光線ごと版（指数は共通）
#[inline(always)]
fn powi_l<V: Lanes>(mut x: V, n: i32) -> V {
    let mut n = n as u32;
    let mut r = V::splat(1.0);
    loop {
        if n & 1 != 0 {
            r = r * x;
        }
        n /= 2;
        if n == 0 {
            return r;
        }
        x = x * x;
    }
}

/// [`crate::shape::conic_root`] の光線ごと版（3 つの場合をすべて計算して選ぶ。同じ演算なので同じビット）
#[inline(always)]
fn conic_root_l<V: Lanes>(ff: V, gg: V, aa: V, disc: V, sig: f64) -> V {
    let q = disc.sqrt() * sig;
    let near = ff / (gg + q);
    let far = (gg - q) / aa;
    let lin = ff / (gg * 2.0);
    sel((gg * sig).ge(0.0), near, sel(aa.ne(0.0), far, lin))
}

impl Sag {
    /// [`trace::sag`] の光線ごと版
    #[inline(always)]
    fn eval<V: Lanes>(&self, r2: V) -> (V, V) {
        let c = self.c;
        let arg = V::splat(1.0) - r2 * self.kcc;
        let root = if c != 0.0 {
            arg.max0().sqrt()
        } else {
            V::splat(1.0)
        };
        let mut z = r2 * c / (root + 1.0);
        let mut dz = if c != 0.0 {
            sel(arg.gt(0.0), V::splat(c) / (root * 2.0), V::splat(f64::NAN))
        } else {
            V::splat(0.0)
        };
        for &(n, a, na) in &self.terms {
            let p = powi_l(r2, n);
            let q = powi_l(r2, n - 1);
            z += p * a;
            dz += q * na;
        }
        (z, dz)
    }
}

/// Zernike の面の矢高と (∂z/∂x, ∂z/∂y) の光線の組の版（[`Shape::sag`] の Zernike の枝と同じ演算を同じ順に:
/// 偶数次非球面 + Zernike の項）
#[inline(always)]
fn zern_sag_l<V: Lanes>(sag: &Sag, terms: Option<&ZernSag>, x: V, y: V) -> (V, V, V) {
    let (z, dz) = sag.eval(x * x + y * y);
    let (mut z, mut zx, mut zy) = (z, dz * 2.0 * x, dz * 2.0 * y);
    if let Some(zs) = terms {
        let (a, ax, ay) = zs.eval_l(x, y);
        z += a;
        zx += ax;
        zy += ay;
    }
    (z, zx, zy)
}

/// 光線ごとの面の番号（止まった面・けられた面。f64 で持つと比較・選択がまとめてできる）
#[inline(always)]
fn mark<V: Lanes>(x: V, m: V::B, j: f64) -> V {
    sel(m, V::splat(j), x)
}

/// 平らな法線 (0, 0, 1)
#[inline(always)]
fn flat<V: Lanes>() -> VL<V> {
    [V::splat(0.0), V::splat(0.0), V::splat(1.0)]
}

/// GRIN の媒質の中を、媒質の始まりの面の局所座標で面との交点まで進める（光線 i）。
/// 戻り値は (この面の局所座標の位置・方向, 光路長, 交点の屈折率)。届かなければ None
#[allow(clippy::too_many_arguments)]
fn grin_lane<V: Lanes, const N: usize>(
    plan: &Plan,
    st: &Step,
    loc: (V3, V3),
    i: usize,
    k0: &mut [Option<V3>; N],
    pe: Option<&mut PolSoa<V>>,
) -> Option<(V3, V3, f64, f64)> {
    let g = st.grin.expect("GRIN step");
    let sys = plan.sys;
    let d = &sys.surfaces[st.j];
    let gs = &sys.surfaces[g.gj];
    let gm = gs.grin().expect("GRIN data of a GRIN step");
    let lam = plan.lam;
    let (rr, tt) = (g.rr, g.tt);
    let mut loc = loc;
    let mut pe = pe;
    // 直前の面が座標ブレークなら、その局所座標から GRIN の面の局所座標へ戻す
    if let Some((rb, tb)) = g.back {
        loc = (add(mv(&rb, loc.0), tb), mv(&rb, loc.1));
        if let Some(e) = pe.as_deref_mut() {
            for q in 0..e.re.len() {
                let x = trace::cmv(&rb, e.get(i, q));
                e.put(i, q, x);
            }
        }
    }
    let cb = d.is_coord_break();
    // 次の面は光線の後ろにもありうる（厚さ 0 の傾けた座標ブレーク、座標ブレークの後の負の厚さなど。
    // [`crate::grin::propagate_to`]）
    let kz = mv(&rr, loc.1)[2];
    let ex = if cb {
        let fx = |q: V3| add(mv(&rr, q), tt)[2];
        crate::grin::propagate_to(gm, lam, loc.0, loc.1, fx, g.max_len, kz)
    } else {
        crate::grin::next_shape(d).and_then(|sh| {
            let fx = |q: V3| {
                let q2 = add(mv(&rr, q), tt);
                q2[2] - sh.sag(q2[0], q2[1]).0
            };
            crate::grin::propagate_to(gm, lam, loc.0, loc.1, fx, g.max_len, kz)
        })
    };
    let Some(e) = ex else {
        k0[i] = None;
        return None;
    };
    // 媒質に入ったときの方向（GRIN の面の局所座標）
    let kin0 = *k0[i].get_or_insert(loc.1);
    if let (Some(pe), false) = (pe.as_deref_mut(), cb) {
        // 電場は、媒質に入ったときからの方向の変化と同じ最小の回転で回す（媒質の中の偏光の変化は
        // 見ない。座標ブレークで区切っても同じ回転になるよう、次の実在の面でまとめて回す）
        pe.rotate_lane(i, cross(kin0, e.k), dot(kin0, e.k));
    }
    if !cb {
        k0[i] = None;
    }
    // 電場も方向と同じ変換で今の面の局所座標へ
    if let Some(pe) = pe {
        for q in 0..pe.re.len() {
            let x = trace::cmv(&rr, pe.get(i, q));
            pe.put(i, q, x);
        }
    }
    Some((add(mv(&rr, e.p), tt), mv(&rr, e.k), e.opl, e.n))
}

/// N 本の光線（大域の位置 p0・方向 k0、光路長 0）を plan のとおりに追跡する（N は組の幅 V の光線の数）
#[inline(always)]
pub(super) fn run<V: Lanes, const N: usize, R: LaneSink>(
    plan: &Plan,
    p0: &[V3; N],
    k0: &[V3; N],
    mut rec: Option<&mut R>,
    mut pol: Option<&mut PolLanes<'_, N>>,
) -> [RayOut; N] {
    const { assert!(N == V::N) };
    let sys = plan.sys;
    let mut pe: Option<PolSoa<V>> = pol.as_deref().map(PolSoa::new);
    let nan = V::splat(f64::NAN);
    let nan3 = [nan; 3];
    let zero = V::splat(0.0);
    let one = V::splat(1.0);
    let p: VL<V> = [0, 1, 2].map(|c| V::from_fn(|i| p0[i][c]));
    let k: VL<V> = [0, 1, 2].map(|c| V::from_fn(|i| k0[i][c]));
    // 止まった面・けられた面の番号（f64 で持つと比較・選択がまとめてできる）
    let mut err = zero;
    let mut tir = V::B::splat(false);
    let mut vig = zero;
    let mut opl = zero;
    let mut lens_opd = zero;
    // 前の面の後ろの媒質の屈折率（GRIN の媒質では光線ごと）
    let mut nprev = V::splat(plan.n_first);
    // 今いる面の局所座標の位置・方向
    let mut pl = nan3;
    let mut kl = nan3;
    // GRIN の媒質に入ったときの方向（光線ごと、座標ブレークを挟んで進むあいだ持つ）
    let mut grin_k0: [Option<V3>; N] = [None; N];
    for (ri, st) in plan.steps.iter().enumerate() {
        let j = st.j;
        let d = &sys.surfaces[j];
        let jf = j as f64;
        // 追跡できない面（複屈折材の板の Mode 1・3）: 残りの光線もここで止める
        if st.blocked {
            err = mark(err, err.eq(0.0), jf);
        }
        // ここまでに止まった光線（以後は NaN）
        let es: V::B = err.ne(0.0);
        // 複屈折材の板の中の光線があるか（その面の偏光は光線ごとのスカラーで）
        let cry_any = pol
            .as_deref()
            .is_some_and(|pr| pr.cry.iter().any(|c| c.is_some()));
        // GRIN の媒質の中を面との交点まで進めた光線（交点は面の上なので、下の交点の計算は飛ばす）
        let on_surf = st.grin.is_some();
        if let Some(g) = st.grin {
            for i in 0..N {
                if es.at(i) {
                    continue;
                }
                let loc = (get(&pl, i), get(&kl, i));
                match grin_lane(plan, st, loc, i, &mut grin_k0, pe.as_mut()) {
                    Some((pq, kq, dl, n)) => {
                        put(&mut pl, i, pq);
                        put(&mut kl, i, kq);
                        opl[i] += dl;
                        nprev[i] = n;
                    }
                    None => {
                        err[i] = jf;
                        put(&mut pl, i, [f64::NAN; 3]);
                        put(&mut kl, i, [f64::NAN; 3]);
                    }
                }
                // 板の結晶軸も方向と同じ変換で（座標ブレークから戻す変換は軸にかけない）
                if let Some(c) = pol.as_deref_mut().and_then(|pr| pr.cry[i].as_mut()) {
                    c.axis = mv(&g.rr, c.axis);
                }
            }
        } else {
            match st.mv {
                Move::Shift(dz) => {
                    pl[2] = pl[2] + dz;
                }
                Move::Rel(q) => {
                    pl = mv_l(&q.r, &pl);
                    for c in 0..3 {
                        pl[c] = pl[c] + q.t[c];
                    }
                    kl = mv_l(&q.r, &kl);
                }
                Move::First => {
                    let o = st.f.o;
                    let dp = {
                        let f = |c: usize| p[c] - o[c];
                        [f(0), f(1), f(2)]
                    };
                    pl = mtv_l(&st.f.r, &dp);
                    kl = mtv_l(&st.f.r, &k);
                }
            }
            // 電場も方向と同じ変換で今の面の局所座標へ（止まった光線の値は最後に NaN にする）
            if let Some(e) = pe.as_mut() {
                match st.mv {
                    Move::Shift(_) => {}
                    Move::Rel(q) => e.mv(&q.r),
                    Move::First => e.mtv(&st.f.r),
                }
            }
            if cry_any {
                for c in pol
                    .as_deref_mut()
                    .expect("polarization state when tracing crystals")
                    .cry
                    .iter_mut()
                    .flatten()
                {
                    c.axis = match st.mv {
                        Move::Shift(_) => c.axis,
                        Move::Rel(q) => mv(&q.r, c.axis),
                        Move::First => mtv(&st.f.r, c.axis),
                    };
                }
            }
        }
        // 頂点面へ（GRIN の媒質から来た光線はもう面の上なので 0）
        let s = if on_surf { zero } else { -pl[2] / kl[2] };
        err = mark(err, err.eq(0.0) & !s.is_finite(), jf);
        pl = axpy(pl, kl, s);
        opl += nprev * s;
        // この面までに進んだ長さ（偏光の吸収）
        let mut s_tot = s;
        if !on_surf {
            // 光線ごとのスカラーで交点を求める（s は頂点面からの長さ、bad は求まらない）
            let mut lane_isect = |f: &dyn Fn(V3, V3) -> (f64, bool)| {
                for i in 0..N {
                    if err[i] == 0.0 {
                        let (pi, ki) = (get(&pl, i), get(&kl, i));
                        let (s, bad) = f(pi, ki);
                        if bad {
                            err[i] = jf;
                        }
                        put(&mut pl, i, add(pi, scale(ki, s)));
                        opl[i] += nprev[i] * s;
                        s_tot[i] += s;
                    }
                }
            };
            match st.kind.isect {
                Isect::Plane => {}
                // Zernike の面は光線の組のまま（[`Shape::isect`] の Zernike の枝・[`crate::freeform::isect_with`] と
                // 同じ演算を同じ順に。光線ごとの反復の回数も同じ）
                Isect::Newton if matches!(d.shape(), Shape::Zernike { .. }) => {
                    let Shape::Zernike { base, terms } = d.shape() else {
                        unreachable!("matched above")
                    };
                    let m: V::B = err.eq(0.0);
                    // 初期値: 基準の二次曲面との交点（交わらなければ 0）
                    let mut s = zero;
                    if base.c != 0.0 {
                        let c = base.c;
                        let ff = (pl[0] * pl[0] + pl[1] * pl[1]) * c;
                        let gg = kl[2] - (pl[0] * kl[0] + pl[1] * kl[1]) * c;
                        let aa =
                            (kl[0] * kl[0] + kl[1] * kl[1] + kl[2] * (1.0 + base.k) * kl[2]) * c;
                        let disc = gg * gg - aa * ff;
                        let r = conic_root_l(ff, gg, aa, disc, st.sig);
                        s = sel(disc.ge(0.0) & r.is_finite(), r, zero);
                    }
                    let mut act = m;
                    let mut ds = V::splat(f64::NAN);
                    for _ in 0..60 {
                        if !act.any() {
                            break;
                        }
                        let q = axpy(pl, kl, s);
                        let (z, zx, zy) = zern_sag_l(&st.sag, terms, q[0], q[1]);
                        let dn = (q[2] - z) / (kl[2] - zx * kl[0] - zy * kl[1]);
                        ds = sel(act, dn, ds);
                        s = sel(act, s - dn, s);
                        let t = (s.abs() + z.abs()) * (8.0 * f64::EPSILON);
                        let tm = crate::tol::NEWTON_STEP_MIN;
                        let tol = sel(t.gt(tm), t, V::splat(tm));
                        act = act & dn.abs().gt(tol);
                    }
                    // !(|ds| <= 1e-10)（NaN も）は求まらない。位置・光路長は求まらなくても進める（光線ごとの版と同じ）
                    err = mark(err, m & !ds.abs().le(1e-10), jf);
                    pl = sel3(m, axpy(pl, kl, s), pl);
                    opl = sel(m, opl + nprev * s, opl);
                    s_tot = sel(m, s_tot + s, s_tot);
                }
                Isect::Newton => {
                    let sh = d.shape();
                    lane_isect(&|pi, ki| sh.isect(pi, ki, st.sig))
                }
                Isect::Std { asph } => {
                    let m: V::B = err.eq(0.0);
                    let c = d.c;
                    let mut s = zero;
                    if c != 0.0 {
                        let ff = (pl[0] * pl[0] + pl[1] * pl[1]) * c;
                        let gg = kl[2] - (pl[0] * kl[0] + pl[1] * kl[1]) * c;
                        let aa = (kl[0] * kl[0] + kl[1] * kl[1] + kl[2] * st.k1 * kl[2]) * c;
                        let disc = gg * gg - aa * ff;
                        s = conic_root_l(ff, gg, aa, disc.max0(), st.sig);
                        // !(disc >= 0)（NaN も）か s が有限でない
                        err = mark(err, m & !(disc.ge(0.0) & s.is_finite()), jf);
                    }
                    if asph {
                        let mut act: V::B = m & err.eq(0.0);
                        let newton = act;
                        let mut ds = zero;
                        for _ in 0..60 {
                            if !act.any() {
                                break;
                            }
                            let q = axpy(pl, kl, s);
                            let r2 = q[0] * q[0] + q[1] * q[1];
                            let (z, dz) = st.sag.eval(r2);
                            let dn =
                                (q[2] - z) / (kl[2] - dz * 2.0 * (q[0] * kl[0] + q[1] * kl[1]));
                            ds = sel(act, dn, ds);
                            s = sel(act, s - dn, s);
                            // 止める刻みは trace::newton_tol と同じ（NEWTON_STEP_MIN か s・z の丸めの 8 ulp の大きい方）
                            let t = (s.abs() + z.abs()) * (8.0 * f64::EPSILON);
                            let m = crate::tol::NEWTON_STEP_MIN;
                            let tol = sel(t.gt(m), t, V::splat(m));
                            act = act & dn.abs().gt(tol);
                        }
                        // !(|ds| <= 1e-10)（NaN も）
                        err = mark(err, newton & !ds.abs().le(1e-10), jf);
                    }
                    pl = sel3(m, axpy(pl, kl, s), pl);
                    opl = sel(m, opl + nprev * s, opl);
                    s_tot = sel(m, s_tot + s, s_tot);
                }
            }
        }
        // 通ってきた媒質の吸収（振幅は内部透過率の 1/2 乗）
        if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
            for (i, c) in pr.cry.iter_mut().enumerate() {
                if let Some(c) = c {
                    c.len += s_tot[i];
                }
            }
            let ps = &pr.surf[j];
            if ps.it.is_some() {
                let la = ps.la;
                let a = V::from_fn(|i| {
                    if es.at(i) || s_tot[i] == 0.0 {
                        1.0
                    } else {
                        (la * s_tot[i]).exp()
                    }
                });
                e.scale(a);
            }
        }
        // 開口
        let clip = match d.ap {
            Aperture::None => None,
            Aperture::Floating { .. } if !plan.floating_clip => None,
            a => Some(a),
        };
        if let Some(a) = clip {
            let out = aperture_out_l(a, &pl);
            vig = mark(vig, out & vig.eq(0.0) & err.eq(0.0), jf);
        }
        // 面の後ろの屈折率（GRIN の媒質へは交点の値で）
        let mut nn = V::splat(st.n_tab);
        if st.grin_eval {
            let g = d.grin().expect("GRIN data of a GRIN step");
            for i in 0..N {
                if err[i] == 0.0 {
                    match g.eval(plan.lam, get(&pl, i)) {
                        Some((n, _)) => nn[i] = n,
                        None => err[i] = jf,
                    }
                }
            }
        }
        // 偏光を光線ごとのスカラーで扱う面（複屈折材の板・貼り合わせ）
        let pol_scalar = pol.as_deref().is_some_and(|pr| {
            let ps = &pr.surf[j];
            cry_any || ps.cry.is_some() || ps.cement.is_some() || ps.cemented
        });
        match st.kind.act {
            Act::CoordBrk => nn = nprev,
            Act::Paraxial => {
                for i in 0..N {
                    if !es.at(i) {
                        let k1 = get(&kl, i);
                        let (k2, dl) = trace::paraxial(d.focal(), get(&pl, i), k1, st.conj);
                        // 電場は方向の変化と同じ最小の回転で回す（振幅は変えない。OpticStudio との照合は未確認）
                        if let (Some(e), true) = (pe.as_mut(), err[i] == 0.0) {
                            e.rotate_lane(i, cross(k1, k2), dot(k1, k2));
                        }
                        put(&mut kl, i, k2);
                        lens_opd[i] += dl;
                    }
                }
            }
            Act::User => {
                let u = match d.action() {
                    Action::User(u) => u,
                    _ => None,
                };
                for i in 0..N {
                    if es.at(i) {
                        continue;
                    }
                    let k1 = get(&kl, i);
                    let out = u.and_then(|u| {
                        u.instance()
                            .deflect(get(&pl, i), k1, nprev[i], nn[i], plan.lam)
                            .filter(|(k2, dl)| k2.iter().all(|v| v.is_finite()) && dl.is_finite())
                    });
                    match out {
                        Some((k2, dl)) if err[i] == 0.0 => {
                            // 電場は理想レンズと同じく、方向の変化と同じ最小の回転で回す
                            if let Some(e) = pe.as_mut() {
                                e.rotate_lane(i, cross(k1, k2), dot(k1, k2));
                            }
                            put(&mut kl, i, k2);
                            lens_opd[i] += dl;
                        }
                        Some(_) => {}
                        None => {
                            if err[i] == 0.0 {
                                err[i] = jf;
                            }
                        }
                    }
                }
            }
            // 平面で屈折率が変わらない面（鏡でない）は方向が変わらないので、法線を求めない
            // （求めても使わない。結果は同じビット）
            _ if !d.is_mirror()
                && st.kind.norm == Norm::Flat
                && st.kind.act != Act::Binary2
                && !pol_scalar
                && (st.kind.act == Act::Jones || st.n_uniform.is_some_and(|(a, b)| a == b)) =>
            {
                // 偏光は法線 (0, 0, 1) で面の処理をする（法線を求めたときと同じ値）
                if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
                    if !matches!(pr.surf[j].kind, trace::PolKind::None) {
                        let nv: VL<V> = flat();
                        let cosi = dot_l(&kl, &nv);
                        let act = err.eq(0.0);
                        pol_surface_l(sys, pr, e, j, (&kl, &kl, &nv, cosi), false, act);
                    }
                }
            }
            _ => {
                // 光線ごとのスカラーで法線（の傾き）を求める
                let lane_normal = |err: &mut V, f: &dyn Fn(f64, f64) -> V3| -> VL<V> {
                    let mut nv = flat();
                    for i in 0..N {
                        let v = f(pl[0][i], pl[1][i]);
                        if !(v[0].is_finite() && v[1].is_finite()) && err[i] == 0.0 {
                            err[i] = jf;
                        }
                        put(&mut nv, i, v);
                    }
                    nv
                };
                let nvec: VL<V> = match st.kind.norm {
                    Norm::Grad if matches!(d.shape(), Shape::Zernike { .. }) => {
                        let Shape::Zernike { terms, .. } = d.shape() else {
                            unreachable!("matched above")
                        };
                        let (_, zx, zy) = zern_sag_l(&st.sag, terms, pl[0], pl[1]);
                        err = mark(err, !(zx.is_finite() & zy.is_finite()) & err.eq(0.0), jf);
                        [-zx, -zy, one]
                    }
                    Norm::Grad => {
                        let sh = d.shape();
                        lane_normal(&mut err, &|x, y| {
                            let (_, zx, zy) = sh.sag(x, y);
                            [-zx, -zy, 1.0]
                        })
                    }
                    Norm::Fresnel => match d.action() {
                        Action::Fresnel(b) => lane_normal(&mut err, &|x, y| b.normal(x, y)),
                        _ => unreachable!("Fresnel の法線は Fresnel の面だけ"),
                    },
                    Norm::Asph => {
                        let r2 = pl[0] * pl[0] + pl[1] * pl[1];
                        let (_, dz) = st.sag.eval(r2);
                        err = mark(err, !dz.is_finite() & err.eq(0.0), jf);
                        [dz * -2.0 * pl[0], dz * -2.0 * pl[1], one]
                    }
                    Norm::Flat => flat(),
                    Norm::Conic => {
                        let c = d.c;
                        [pl[0] * -c, pl[1] * -c, one - pl[2] * st.ck1]
                    }
                };
                let inv = one / (nvec[0] * nvec[0] + nvec[1] * nvec[1] + nvec[2] * nvec[2]).sqrt();
                let nv: VL<V> = [nvec[0] * inv, nvec[1] * inv, nvec[2] * inv];
                let cosi = kl[0] * nv[0] + kl[1] * nv[1] + kl[2] * nv[2];
                let kin = kl;
                if d.is_mirror() {
                    kl = {
                        let f = |c: usize| kl[c] - nv[c] * (cosi * 2.0);
                        [f(0), f(1), f(2)]
                    };
                    nn = nprev;
                } else if st.kind.act == Act::Jones {
                } else if let Some((a, b)) = st.n_uniform {
                    // 前後の屈折率がどの光線でも同じ（GRIN の媒質が絡まない）: 比は数 1 つ
                    if b != a {
                        let mu = a / b;
                        let mu2 = mu * mu;
                        let disc = one - (one - cosi * cosi) * mu2;
                        let total = !disc.ge(0.0) & err.eq(0.0);
                        err = mark(err, total, jf);
                        tir = tir | total;
                        let t = cosi.signum() * disc.max0().sqrt() - cosi * mu;
                        kl = {
                            let f = |c: usize| kl[c] * mu + nv[c] * t;
                            [f(0), f(1), f(2)]
                        };
                    }
                } else {
                    let refr = nn.ne(nprev);
                    if refr.any() {
                        let mu = nprev / nn;
                        let mu2 = mu * mu;
                        let disc = one - mu2 * (one - cosi * cosi);
                        let total = refr & !disc.ge(0.0) & err.eq(0.0);
                        err = mark(err, total, jf);
                        tir = tir | total;
                        let t = cosi.signum() * disc.max0().sqrt() - mu * cosi;
                        kl = sel3(
                            refr,
                            {
                                let f = |c: usize| kl[c] * mu + nv[c] * t;
                                [f(0), f(1), f(2)]
                            },
                            kl,
                        );
                    }
                }
                if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
                    if pol_scalar {
                        for i in 0..N {
                            if err[i] != 0.0 {
                                continue;
                            }
                            let mut x = e.lane(i);
                            trace::pol_step(
                                sys,
                                pr.surf,
                                j,
                                &mut x,
                                &mut pr.cry[i],
                                &mut pr.err[i],
                                (get(&kin, i), get(&kl, i), get(&nv, i), cosi[i]),
                                d.is_mirror(),
                                plan.lam,
                            );
                            e.set_lane(i, &x);
                        }
                    } else if !matches!(pr.surf[j].kind, trace::PolKind::None) {
                        let act = err.eq(0.0);
                        pol_surface_l(sys, pr, e, j, (&kin, &kl, &nv, cosi), d.is_mirror(), act);
                    }
                }
                // 働きの見方は使う面でだけ作る（どの面でも通る所なので）
                let fres = match st.kind.fresnel.then(|| d.action()) {
                    Some(Action::Fresnel(b)) => Some(b),
                    _ => None,
                };
                if let Some(bend) = fres {
                    for i in 0..N {
                        if err[i] == 0.0 {
                            let a = [pl[0][i], pl[1][i]];
                            match trace::fresnel_opl(
                                &bend,
                                d.is_mirror(),
                                get(&kin, i),
                                nprev[i],
                                nn[i],
                                plan.fresnel_ref(st.j),
                                a,
                            ) {
                                Some(e) => opl[i] += e,
                                None => err[i] = jf,
                            }
                        }
                    }
                }
                let bin2 = match (st.kind.act == Act::Binary2).then(|| d.action()) {
                    Some(Action::Binary2(b)) => b,
                    _ => None,
                };
                if let Some(b) = bin2 {
                    let sc = plan.lam * 1e-3 / (2.0 * std::f64::consts::PI);
                    for i in 0..N {
                        if err[i] != 0.0 {
                            continue;
                        }
                        let (ph, gx, gy) = b.phase(pl[0][i], pl[1][i]);
                        opl[i] += sc * ph;
                        let k1 = get(&kl, i);
                        match trace::diffract(k1, get(&nv, i), nn[i], [sc * gx, sc * gy]) {
                            Some(k2) => {
                                // 電場は方向の変化と同じ最小の回転で回す（振幅は変えない。OpticStudio との照合は未確認）
                                if let Some(e) = pe.as_mut() {
                                    e.rotate_lane(i, cross(k1, k2), dot(k1, k2));
                                }
                                put(&mut kl, i, k2);
                            }
                            None => err[i] = jf,
                        }
                    }
                }
            }
        }
        // 面の中で方向・位置が有限でなくなった光線（理想レンズの kl[2] = 0、屈折率 NaN など）もこの面で止める
        // （最後の面でも「err = 0 なら有限」を守る）
        let fin = pl[0].is_finite()
            & pl[1].is_finite()
            & pl[2].is_finite()
            & kl[0].is_finite()
            & kl[1].is_finite()
            & kl[2].is_finite()
            & opl.is_finite();
        err = mark(err, err.eq(0.0) & !fin, jf);
        let bad: V::B = err.ne(0.0);
        pl = sel3(bad, nan3, pl);
        kl = sel3(bad, nan3, kl);
        opl = sel(bad, nan, opl);
        nprev = nn;
        if let Some(r) = rec.as_deref_mut().filter(|r| r.wants(ri)) {
            let pg = mv_l(&st.f.r, &pl);
            for i in 0..N {
                let e = if es.at(i) {
                    RecEntry {
                        surf: j,
                        ..RecEntry::NAN
                    }
                } else {
                    RecEntry {
                        surf: j,
                        xyz: get(&pl, i),
                        lmn: get(&kl, i),
                        opl: opl[i],
                        p: add(st.f.o, get(&pg, i)),
                    }
                };
                r.put(i, ri, e);
            }
        }
    }
    if let (Some(pr), Some(e)) = (pol, pe) {
        let np = pr.np;
        for i in 0..N {
            let dead = err[i] != 0.0 || pr.err[i].is_some();
            for q in 0..np {
                pr.e[i * np + q] = if dead { trace::CNAN3 } else { e.get(i, q) };
            }
        }
    }
    // 大域の位置・方向は最後の面でだけ求める（追う面がなければ始めのまま）
    let Some(f) = plan.steps.last().map(|s| s.f) else {
        return std::array::from_fn(|i| RayOut {
            xyz: [f64::NAN; 3],
            lmn: [f64::NAN; 3],
            opl: 0.0,
            status: RayStatus::Ok,
            vig: 0,
            p: p0[i],
            k: k0[i],
            lens_opd: 0.0,
        });
    };
    let pg = mv_l(&f.r, &pl);
    let kg = mv_l(&f.r, &kl);
    std::array::from_fn(|i| RayOut {
        xyz: get(&pl, i),
        lmn: get(&kl, i),
        opl: opl[i],
        status: match err[i] as usize {
            0 => RayStatus::Ok,
            surf if tir.at(i) => RayStatus::Tir { surf },
            surf => RayStatus::Missed { surf },
        },
        vig: vig[i] as usize,
        p: add(f.o, get(&pg, i)),
        k: get(&kg, i),
        lens_opd: lens_opd[i],
    })
}
