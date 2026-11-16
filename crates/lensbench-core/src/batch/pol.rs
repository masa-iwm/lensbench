//! 偏光の電場の組（[`PolSoa`]）と面での変化、開口の判定の組の版

use super::kernel::{dot_l, get, mtv_l, mv_l, sel3, VL};
use super::lanes::{sel, Lanes, Mask};
use crate::coating::C64;
use crate::math::*;
use crate::surface::Aperture;
use crate::system::System;
use crate::trace::{self, CryState, PolKind, PolSurf, CV3};
use crate::Error;

/// N 本の光線の偏光の状態（[`trace::pol_step`] の状態の N 本分）
pub(crate) struct PolLanes<'a, const N: usize> {
    /// 電場（光線 l の偏光の入力 q は `e[l * np + q]`、今いる面の局所座標）
    pub e: Vec<CV3>,
    /// 偏光の入力の数
    pub np: usize,
    pub surf: &'a [PolSurf],
    /// 光線ごとの係数のエラー（膜の表の範囲外など。電場は NaN にする）
    pub err: [Option<Error>; N],
    /// 複屈折材の板の中を進んでいる光線の状態
    pub cry: [Option<CryState>; N],
}

impl<'a, const N: usize> PolLanes<'a, N> {
    pub fn new(e: Vec<CV3>, np: usize, surf: &'a [PolSurf]) -> Self {
        PolLanes {
            e,
            np,
            surf,
            err: std::array::from_fn(|_| None),
            cry: [None; N],
        }
    }
}

/// N 本の電場（偏光の入力ごとに実部・虚部の成分を組で持つ）
pub(super) struct PolSoa<V> {
    pub re: Vec<VL<V>>,
    pub im: Vec<VL<V>>,
}

/// a × b（[`cross`] と同じ式）
#[inline(always)]
fn cross_l<V: Lanes>(a: &VL<V>, b: &VL<V>) -> VL<V> {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl<V: Lanes> PolSoa<V> {
    pub fn new<const N: usize>(pr: &PolLanes<N>) -> PolSoa<V> {
        let np = pr.np;
        let z = [V::splat(0.0); 3];
        let (mut re, mut im) = (vec![z; np], vec![z; np]);
        for l in 0..N {
            for q in 0..np {
                let e = pr.e[l * np + q];
                for c in 0..3 {
                    re[q][c][l] = e[c].re;
                    im[q][c][l] = e[c].im;
                }
            }
        }
        PolSoa { re, im }
    }

    pub fn get(&self, l: usize, q: usize) -> CV3 {
        [0, 1, 2].map(|c| C64::new(self.re[q][c][l], self.im[q][c][l]))
    }

    pub fn put(&mut self, l: usize, q: usize, e: CV3) {
        for c in 0..3 {
            self.re[q][c][l] = e[c].re;
            self.im[q][c][l] = e[c].im;
        }
    }

    /// 光線 l の全入力の電場
    pub fn lane(&self, l: usize) -> Vec<CV3> {
        (0..self.re.len()).map(|q| self.get(l, q)).collect()
    }

    pub fn set_lane(&mut self, l: usize, e: &[CV3]) {
        for (q, x) in e.iter().enumerate() {
            self.put(l, q, *x);
        }
    }

    /// 実の行列 m をかける（[`trace::cmv`] と同じ順）
    #[inline(always)]
    pub fn mv(&mut self, m: &M3) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = mv_l(m, v);
        }
    }

    /// 実の行列 m の転置をかける（[`trace::cmtv`] と同じ順）
    #[inline(always)]
    pub fn mtv(&mut self, m: &M3) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = mtv_l(m, v);
        }
    }

    /// 光線ごとの実数 a をかける
    #[inline(always)]
    pub fn scale(&mut self, a: V) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = [v[0] * a, v[1] * a, v[2] * a];
        }
    }

    /// 光線 l の電場を最小の回転で回す（[`trace::rotate_min`]）
    pub fn rotate_lane(&mut self, l: usize, ax: V3, c: f64) {
        for q in 0..self.re.len() {
            let x = trace::rotate_min(self.get(l, q), ax, c);
            self.put(l, q, x);
        }
    }

    /// 各入力の電場 x を、係数 (a, b)（複素、x を u・w へ射影した値から作る）で a u + b v に置き換える
    /// （[`trace::pol_surface`] の `comb` と同じ順）。f は射影 (x·u, x·w) から (a, b) を作る
    #[inline(always)]
    fn recombine(
        &mut self,
        pu: &VL<V>,
        pw: &VL<V>,
        u: &VL<V>,
        v: &VL<V>,
        f: impl Fn(Cl<V>, Cl<V>) -> (Cl<V>, Cl<V>),
    ) {
        for q in 0..self.re.len() {
            let (xr, xi) = (&mut self.re[q], &mut self.im[q]);
            let eu = (dot_l(xr, pu), dot_l(xi, pu));
            let ew = (dot_l(xr, pw), dot_l(xi, pw));
            let (a, b) = f(eu, ew);
            *xr = {
                let f = |c: usize| a.0 * u[c] + b.0 * v[c];
                [f(0), f(1), f(2)]
            };
            *xi = {
                let f = |c: usize| a.1 * u[c] + b.1 * v[c];
                [f(0), f(1), f(2)]
            };
        }
    }
}

/// 複素数の組（実部, 虚部）
type Cl<V> = (V, V);

/// 複素の数 × 組（[`C64`] の積と同じ式）
#[inline(always)]
fn cmul_s<V: Lanes>(m: C64, x: Cl<V>) -> Cl<V> {
    (x.0 * m.re - x.1 * m.im, x.1 * m.re + x.0 * m.im)
}

/// 組 × 組の複素の積（[`C64`] の積と同じ式）
#[inline(always)]
fn cmul<V: Lanes>(t: Cl<V>, x: Cl<V>) -> Cl<V> {
    (t.0 * x.0 - t.1 * x.1, t.0 * x.1 + t.1 * x.0)
}

#[inline(always)]
fn cadd<V: Lanes>(a: Cl<V>, b: Cl<V>) -> Cl<V> {
    (a.0 + b.0, a.1 + b.1)
}

/// 面での電場の変化（[`trace::pol_surface`] の組の版。複屈折材の板を除く）。g = 面の前後の方向・法線・入射角の
/// cos、act = 処理する光線（ほかの光線の電場は使わない値になる）。係数のエラーは光線ごとに持つ
#[inline(always)]
pub(super) fn pol_surface_l<V: Lanes, const N: usize>(
    sys: &System,
    pr: &mut PolLanes<N>,
    e: &mut PolSoa<V>,
    j: usize,
    g: (&VL<V>, &VL<V>, &VL<V>, V),
    mirror: bool,
    act: V::B,
) {
    let (kin, kl, nv, cosi) = g;
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    match &pr.surf[j].kind {
        PolKind::None => {}
        PolKind::Jones(m) => {
            // u・v は [`trace::pol_surface`] と同じ式（0 の足し算・引き算も同じ）
            let k2 = kl[2];
            let sz = sel(k2.gt(0.0), one, sel(k2.lt(0.0), -one, zero));
            let inv = one / (one + k2.abs());
            let h = [(kl[0] + 0.0) * inv, (kl[1] + 0.0) * inv, (k2 + sz) * inv];
            let u = [one - h[0] * kl[0], zero - h[1] * kl[0], zero - h[2] * kl[0]];
            let v = [zero - h[0] * kl[1], one - h[1] * kl[1], zero - h[2] * kl[1]];
            let m = *m;
            e.recombine(&u, &v, &u, &v, |eu, ev| {
                (
                    cadd(cmul_s(m[0], eu), cmul_s(m[1], ev)),
                    cadd(cmul_s(m[2], eu), cmul_s(m[3], ev)),
                )
            });
        }
        PolKind::Iface(f) => {
            // s = kin × nv（小さければ kin × x̂）を規格化
            let s1 = cross_l(kin, nv);
            let s2 = cross_l(kin, &[one, zero, zero]);
            let ok = dot_l(&s1, &s1).sqrt().gt(1e-12);
            let sv = sel3(ok, s1, s2);
            let inv = one / dot_l(&sv, &sv).sqrt();
            let sv = [sv[0] * inv, sv[1] * inv, sv[2] * inv];
            let pin = cross_l(&sv, kin);
            let mut pout = cross_l(&sv, kl);
            if mirror {
                pout = [pout[0] * -1.0, pout[1] * -1.0, pout[2] * -1.0];
            }
            let (mut t0, mut t1) = ((zero, zero), (zero, zero));
            for i in 0..N {
                if act.at(i) {
                    match f.eval_cos(cosi[i].abs().clamp(0.0, 1.0)) {
                        Ok(t) => {
                            (t0.0[i], t0.1[i]) = (t[0].re, t[0].im);
                            (t1.0[i], t1.1[i]) = (t[1].re, t[1].im);
                        }
                        Err(er) => {
                            pr.err[i].get_or_insert_with(|| er.at(sys.surf_label(j)));
                        }
                    }
                }
            }
            e.recombine(&sv, &pin, &sv, &pout, |es, ep| (cmul(t0, es), cmul(t1, ep)));
        }
    }
}

/// 開口の外か（光線ごと）。円・遮蔽は半径の 2 乗で十分に離れていればそれで決め、
/// 境界の近く（と NaN・無限大）だけ [`trace::aperture_out`]（hypot）で決める
#[inline(always)]
pub(super) fn aperture_out_l<V: Lanes>(a: Aperture, pl: &VL<V>) -> V::B {
    // 半径の 2 乗の丸めの誤差（数 ulp）より十分大きい幅
    const M: f64 = 1e-12;
    #[inline(always)]
    fn ring<V: Lanes>(
        a: Aperture,
        pl: &VL<V>,
        (dx, dy): (f64, f64),
        (lo, hi): (f64, f64),
        obsc: bool,
    ) -> V::B {
        let (u, v) = (pl[0] - dx, pl[1] - dy);
        let r = u * u + v * v;
        let (lo2, hi2) = (lo * lo, hi * hi);
        // inside: lo < r < hi が確か、outside: r < lo か r > hi が確か（どちらでもなければ hypot で）
        let (has_lo, has_hi) = (V::B::splat(lo > 0.0), V::B::splat(hi > 0.0));
        let (hi_in, hi_out, lo_in, lo_out) = (
            hi2 * (1.0 - M),
            hi2 * (1.0 + M),
            lo2 * (1.0 + M),
            lo2 * (1.0 - M),
        );
        let inside = has_hi & r.lt(hi_in) & (!has_lo | r.gt(lo_in));
        let outside = (has_lo & r.lt(lo_out)) | (has_hi & r.gt(hi_out) & r.lt(f64::INFINITY));
        let mut out = if obsc { inside } else { outside };
        let unsure = !(inside | outside);
        if unsure.any() {
            for i in 0..V::N {
                if unsure.at(i) {
                    out.set(i, trace::aperture_out(a, get(pl, i)));
                }
            }
        }
        out
    }
    match a {
        Aperture::Circular { min, max, dx, dy } => {
            ring(a, pl, (dx, dy), (min - 1e-9, max + 1e-9), false)
        }
        Aperture::Obsc { min, max, dx, dy } => {
            ring(a, pl, (dx, dy), (min - 1e-9, max + 1e-9), true)
        }
        // [`trace::aperture_out`] と同じ比較を光線ごとに（NaN は外にならない）
        Aperture::Rect { hx, hy, dx, dy } => {
            let (bx, by) = (hx + 1e-9, hy + 1e-9);
            (pl[0] - dx).abs().gt(bx) | (pl[1] - dy).abs().gt(by)
        }
        _ => V::B::from_fn(|i| trace::aperture_out(a, get(pl, i))),
    }
}

#[cfg(test)]
mod tests {
    use super::super::lanes::Wide;
    use super::*;

    /// まとめた開口の判定は、光線ごとの判定（trace::aperture_out）と同じ（境界のすぐ内外・NaN・無限大も）
    #[test]
    fn lane_aperture_matches_scalar_for_floating_and_circular() {
        let semi = 3.7;
        let edge = semi + 1e-9;
        let rs = [
            0.0,
            1.0,
            edge * (1.0 - 1e-15),
            edge,
            edge * (1.0 + 1e-15),
            edge * (1.0 + 1e-12),
            edge * (1.0 + 1e-9),
            10.0,
            f64::NAN,
            f64::INFINITY,
            -edge,
            edge * 0.999_999_999_999,
        ];
        let aps = [
            Aperture::Floating { semi },
            Aperture::Circular {
                min: 0.0,
                max: semi,
                dx: 0.0,
                dy: 0.0,
            },
        ];
        for a in aps {
            for ang in [0.0f64, 0.3, 1.1, 2.5] {
                let mut pl: VL<Wide> = [Wide::splat(0.0); 3];
                for i in 0..Wide::N {
                    let r = rs[i % rs.len()];
                    let t = ang + i as f64 * 0.7;
                    pl[0][i] = r * t.cos();
                    pl[1][i] = r * t.sin();
                }
                let out = aperture_out_l(a, &pl);
                for i in 0..Wide::N {
                    assert_eq!(out.at(i), trace::aperture_out(a, get(&pl, i)), "{a:?} {i}");
                }
            }
        }
    }
}
