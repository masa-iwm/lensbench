//! 光線を [`L`] 本ずつまとめて追跡する（SIMD）。
//!
//! [`crate::trace::trace_core`]（偏光なし）と同じ演算を同じ順に光線ごとに行う。光線の間で値を混ぜず、
//! 分岐は光線ごとの選択に置き換えるだけなので、結果は 1 本ずつの追跡とビット単位で一致する
//! （FMA などの融合はしない。`tests/batch.rs` で確かめる）。光線の組は入力の順に L 本ずつに固定するので、
//! スレッド数にもよらない。
//!
//! 演算は光線の値の組 [`lanes::F`] の演算子で書く。ネイティブでは組は光線の配列で、演算は素直なループ
//! （コンパイラのベクトル化に任せる）。x86_64 では AVX があれば 4 本を 1 命令で計算する版を実行時に選ぶ
//! （AVX は演算の結果を変えない。FMA は有効にしない）。wasm ＋ simd128 では f64x2 の命令を名指しする
//! （LLVM が wasm ではこのループをベクトル化しないため。公開 #99）。
//! トロイダル面・理想レンズ・開口の際どい判定など、まれな・重い処理は光線ごとのスカラーで行う。
//! 偏光の追跡（[`trace_lanes_pol`]）も同じく、電場の計算を 1 本ずつの追跡と同じ演算・同じ順で光線ごとに
//! 行う（膜の係数は光線ごとに求める。複屈折材の板は 1 本ずつの追跡で）。

use crate::coating::C64;
use crate::math::*;
use crate::surface::{Aperture, SurfType, Surface};
use crate::system::{Frame, Frames, Rel, System};
use crate::trace::{self, PolKind, PolSurf, RayOut, RecEntry, CV3};
use crate::Error;
use std::sync::atomic::{AtomicBool, Ordering};

/// 1 回にまとめて追う光線の数（AVX の 4 本 × 4。多いほど面ごとの準備の手間が薄まる）。
/// wasm は 8 本（f64x2 × 4）: 16 本では値の組がレジスタに載らず、読み書きが増えて遅い（公開 #99）
#[cfg(not(target_arch = "wasm32"))]
pub const L: usize = 16;
#[cfg(target_arch = "wasm32")]
pub const L: usize = 8;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// まとめ追跡を使うか（既定は使う）。結果は同じビットなので、切り分け・比較のためだけに使う
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// n 本の追跡をまとめて追うか（少ない本数は 1 本ずつの方が速い）
pub fn use_for(n: usize) -> bool {
    n >= L / 4 && ENABLED.load(Ordering::Relaxed)
}

mod lanes;
use lanes::{sel, B, F};

type VL = [F; 3];

/// 3 成分（配列の map は内側に展開されないことがあるので書き下す）
macro_rules! v3 {
    ($c:ident => $e:expr) => {{
        [
            {
                let $c = 0;
                $e
            },
            {
                let $c = 1;
                $e
            },
            {
                let $c = 2;
                $e
            },
        ]
    }};
}

#[inline(always)]
fn sel3(m: B, a: VL, b: VL) -> VL {
    [sel(m, a[0], b[0]), sel(m, a[1], b[1]), sel(m, a[2], b[2])]
}

/// a + b·s（光線ごとの s）
#[inline(always)]
fn axpy(a: VL, b: VL, s: F) -> VL {
    v3!(c => a[c] + b[c] * s)
}

/// M v（[`mv`] と同じ順）
#[inline(always)]
fn mv_l(m: &M3, v: &VL) -> VL {
    v3!(r => m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

/// Mᵀ v（[`mtv`] と同じ順）
#[inline(always)]
fn mtv_l(m: &M3, v: &VL) -> VL {
    v3!(r => m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

#[inline(always)]
fn get(v: &VL, i: usize) -> V3 {
    [v[0][i], v[1][i], v[2][i]]
}

#[inline(always)]
fn put(v: &mut VL, i: usize, x: V3) {
    for c in 0..3 {
        v[c][i] = x[c];
    }
}

/// [`trace::powi`] の光線ごと版（指数は共通）
#[inline(always)]
fn powi_l(mut x: F, n: i32) -> F {
    let mut n = n as u32;
    let mut r = F::splat(1.0);
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

/// [`trace::conic_root`] の光線ごと版（3 つの場合をすべて計算して選ぶ。同じ演算なので同じビット）
#[inline(always)]
fn conic_root_l(ff: F, gg: F, aa: F, disc: F, sig: f64) -> F {
    let q = sig * disc.sqrt();
    let near = ff / (gg + q);
    let far = (gg - q) / aa;
    let lin = ff / (2.0 * gg);
    sel((gg * sig).ge(0.0), near, sel(aa.ne(0.0), far, lin))
}

/// 前の面からこの面の局所座標への移り方
#[derive(Clone, Copy)]
enum Move {
    /// 大域座標から（最初の面）
    First,
    Shift(f64),
    Rel(Rel),
}

/// 偶数次非球面のサグの係数（[`trace::sag`] と同じ順に足す）
struct Sag {
    c: f64,
    /// (1 + k) c c
    kcc: f64,
    /// (指数, a, 指数·a)。0 の係数は除く
    terms: Vec<(i32, f64, f64)>,
}

impl Sag {
    fn new(d: &Surface) -> Sag {
        Sag {
            c: d.c,
            kcc: (1.0 + d.k) * d.c * d.c,
            terms: d
                .asph
                .iter()
                .enumerate()
                .filter(|(_, &a)| a != 0.0)
                .map(|(i, &a)| {
                    let i = (i + 1) as i32;
                    (i, a, i as f64 * a)
                })
                .collect(),
        }
    }

    /// [`trace::sag`] の光線ごと版
    #[inline(always)]
    fn eval(&self, r2: F) -> (F, F) {
        let c = self.c;
        let arg = 1.0 - self.kcc * r2;
        let root = if c != 0.0 {
            arg.max0().sqrt()
        } else {
            F::splat(1.0)
        };
        let mut z = c * r2 / (1.0 + root);
        let mut dz = if c != 0.0 {
            sel(arg.gt(0.0), c / (2.0 * root), F::splat(f64::NAN))
        } else {
            F::splat(0.0)
        };
        for &(n, a, na) in &self.terms {
            let p = powi_l(r2, n);
            let q = powi_l(r2, n - 1);
            z += a * p;
            dz += na * q;
        }
        (z, dz)
    }
}

/// 面ごとの準備（光線によらない量）
struct Step<'a> {
    j: usize,
    d: &'a Surface,
    f: Frame,
    mv: Move,
    nprev: f64,
    nn: f64,
    curved: bool,
    asph: bool,
    sag: Sag,
    /// (1 + k)（交点の二次方程式）と c (1 + k)（法線）
    k1: f64,
    ck1: f64,
    clip: Option<Aperture>,
    /// 理想レンズの物体側の共役距離
    conj: f64,
    /// この面に向かう空間の進む向き（[`trace::space_signs`]）
    sig: f64,
}

/// 面 first..=last の追跡の準備（波長 w）。光線によらない量を先に求めておく
pub struct Plan<'a> {
    sys: &'a System,
    steps: Vec<Step<'a>>,
}

impl<'a> Plan<'a> {
    pub fn new(
        sys: &'a System,
        fr: &Frames,
        w: usize,
        first: usize,
        last: usize,
        floating_clip: bool,
    ) -> Plan<'a> {
        let mut steps = Vec::new();
        let sigs = sys.space_signs();
        let mut nprev = sys.index_before(first, w);
        for j in first..=last {
            let d = &sys.surfaces[j];
            if d.ignore {
                continue;
            }
            // Fresnel・Cylinder Fresnel・Binary 2・GRIN・利用者定義の面は 1 本ずつの追跡で（空の計画を返す）
            if matches!(
                d.typ,
                SurfType::Fresnel
                    | SurfType::CylFresnel
                    | SurfType::Binary2
                    | SurfType::Gradium
                    | SurfType::Gradient9
                    | SurfType::User
            ) {
                return Plan {
                    sys,
                    steps: Vec::new(),
                };
            }
            let f = fr.surf[j].unwrap();
            let mv = match (steps.is_empty(), fr.rel[j]) {
                (
                    false,
                    Some(Rel {
                        shift: Some(dz), ..
                    }),
                ) => Move::Shift(dz),
                (false, Some(q)) => Move::Rel(q),
                _ => Move::First,
            };
            let nn = if d.typ == SurfType::CoordBrk
                || d.mirror && !matches!(d.typ, SurfType::Paraxial | SurfType::CoordBrk)
            {
                nprev
            } else {
                d.n[w]
            };
            let curved = matches!(
                d.typ,
                SurfType::Standard | SurfType::EvenAsph | SurfType::Toroidal
            ) && (d.c != 0.0 || !d.asph.is_empty() || d.cx != 0.0);
            let clip = match d.ap {
                Aperture::None => None,
                Aperture::Floating { .. } if !floating_clip => None,
                a => Some(a),
            };
            steps.push(Step {
                j,
                d,
                f,
                mv,
                nprev,
                nn,
                curved,
                asph: d.has_asph(),
                sag: Sag::new(d),
                k1: 1.0 + d.k,
                ck1: d.c * (1.0 + d.k),
                clip,
                sig: sigs[j],
                conj: if d.typ == SurfType::Paraxial {
                    sys.lens_conjugate(j)
                } else {
                    0.0
                },
            });
            nprev = nn;
        }
        Plan { sys, steps }
    }

    /// 偏光の追跡（面ごとの処理 surf）をまとめて追えるか（複屈折材の板があれば 1 本ずつ）
    pub fn supports_pol(&self, surf: &[PolSurf]) -> bool {
        !self.is_empty() && self.steps.iter().all(|st| surf[st.j].cry.is_none())
    }

    /// 追う面が 1 つもない
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// 面ごとの記録を光線ごとに受け取る（lane は組の中の番号、k は無視しない面の通し番号）
pub trait LaneSink {
    fn put(&mut self, lane: usize, k: usize, e: RecEntry);
    /// 無視しない面の通し番号 k の記録を受け取るか（受け取らない面は記録の値を作らない。既定は全面）
    #[inline(always)]
    fn wants(&self, _k: usize) -> bool {
        true
    }
}

/// 記録しない
pub struct NoRec;
impl LaneSink for NoRec {
    #[inline(always)]
    fn put(&mut self, _: usize, _: usize, _: RecEntry) {}
}

/// L 本の光線（大域の位置 p・方向 k、光路長 0）を plan のとおりに追跡する。
/// 結果は光線ごとに [`trace::trace_ray`] と同じビット
pub fn trace_lanes<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
) -> [RayOut; L] {
    dispatch(plan, p, k, rec, None)
}

/// L 本の光線の偏光の状態（[`trace::PolRay`] の L 本分）
pub struct PolLanes<'a> {
    /// 電場（光線 l の偏光の入力 q は `e[l * np + q]`、今いる面の局所座標）
    pub e: Vec<CV3>,
    /// 偏光の入力の数
    pub np: usize,
    pub surf: &'a [PolSurf],
    /// 光線ごとの係数のエラー（[`trace::PolRay::err`]）
    pub err: [Option<Error>; L],
}

/// [`trace_lanes`] に偏光を加えたもの（電場 pol.e を面ごとに更新する）。plan は
/// [`Plan::supports_pol`] を満たすこと。結果は光線ごとに [`trace::trace_ray_pol`] と同じビット
pub fn trace_lanes_pol<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: &mut PolLanes,
) -> [RayOut; L] {
    dispatch(plan, p, k, rec, Some(pol))
}

#[inline(always)]
fn dispatch<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: Option<&mut PolLanes>,
) -> [RayOut; L] {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx") {
            // 安全: AVX があることを上で確かめた
            return unsafe { run_avx(plan, p, k, rec, pol) };
        }
    }
    run(plan, p, k, rec, pol)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn run_avx<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: Option<&mut PolLanes>,
) -> [RayOut; L] {
    run(plan, p, k, rec, pol)
}

/// L 本の電場（偏光の入力ごとに実部・虚部の成分を組で持つ）
struct PolSoa {
    re: Vec<VL>,
    im: Vec<VL>,
}

/// a · b（[`dot`] と同じ順）
#[inline(always)]
fn dot_l(a: &VL, b: &VL) -> F {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// a × b（[`cross`] と同じ式）
#[inline(always)]
fn cross_l(a: &VL, b: &VL) -> VL {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl PolSoa {
    fn new(pr: &PolLanes) -> PolSoa {
        let np = pr.np;
        let z = [F::splat(0.0); 3];
        let (mut re, mut im) = (vec![z; np], vec![z; np]);
        for l in 0..L {
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

    fn get(&self, l: usize, q: usize) -> CV3 {
        [0, 1, 2].map(|c| C64::new(self.re[q][c][l], self.im[q][c][l]))
    }

    fn put(&mut self, l: usize, q: usize, e: CV3) {
        for c in 0..3 {
            self.re[q][c][l] = e[c].re;
            self.im[q][c][l] = e[c].im;
        }
    }

    /// 実の行列 m をかける（[`trace::cmv`] と同じ順）
    #[inline(always)]
    fn mv(&mut self, m: &M3) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = mv_l(m, v);
        }
    }

    /// 実の行列 m の転置をかける（[`trace::cmtv`] と同じ順）
    #[inline(always)]
    fn mtv(&mut self, m: &M3) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = mtv_l(m, v);
        }
    }

    /// 光線ごとの実数 a をかける
    #[inline(always)]
    fn scale(&mut self, a: F) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            *v = v3!(c => v[c] * a);
        }
    }

    /// 各入力の電場 x を、係数 (a, b)（複素、x を u・w へ射影した値から作る）で a u + b v に置き換える
    /// （[`trace::pol_surface`] の `comb` と同じ順）。f は射影 (x·u, x·w) から (a, b) を作る
    #[inline(always)]
    fn recombine(&mut self, pu: &VL, pw: &VL, u: &VL, v: &VL, f: impl Fn(Cl, Cl) -> (Cl, Cl)) {
        for q in 0..self.re.len() {
            let (xr, xi) = (&mut self.re[q], &mut self.im[q]);
            let eu = (dot_l(xr, pu), dot_l(xi, pu));
            let ew = (dot_l(xr, pw), dot_l(xi, pw));
            let (a, b) = f(eu, ew);
            *xr = v3!(c => a.0 * u[c] + b.0 * v[c]);
            *xi = v3!(c => a.1 * u[c] + b.1 * v[c]);
        }
    }
}

/// 複素数の組（実部, 虚部）
type Cl = (F, F);

/// 複素の数 × 組（[`C64`] の積と同じ式）
#[inline(always)]
fn cmul_s(m: C64, x: Cl) -> Cl {
    (m.re * x.0 - m.im * x.1, m.re * x.1 + m.im * x.0)
}

/// 組 × 組の複素の積（[`C64`] の積と同じ式）
#[inline(always)]
fn cmul(t: Cl, x: Cl) -> Cl {
    (t.0 * x.0 - t.1 * x.1, t.0 * x.1 + t.1 * x.0)
}

#[inline(always)]
fn cadd(a: Cl, b: Cl) -> Cl {
    (a.0 + b.0, a.1 + b.1)
}

/// 面での電場の変化（[`trace::pol_surface`] の L 本版。複屈折材の板を除く）。g = 面の前後の方向・法線・入射角の
/// cos、act = 処理する光線（ほかの光線の電場は使わない値になる）。係数のエラーは光線ごとに持つ
#[inline(always)]
fn pol_surface_l(
    sys: &System,
    pr: &mut PolLanes,
    e: &mut PolSoa,
    j: usize,
    g: (&VL, &VL, &VL, F),
    mirror: bool,
    act: B,
) {
    let (kin, kl, nv, cosi) = g;
    let (zero, one) = (F::splat(0.0), F::splat(1.0));
    match &pr.surf[j].kind {
        PolKind::None => {}
        PolKind::Jones(m) => {
            // u・v は [`trace::pol_surface`] と同じ式（0 の足し算・引き算も同じ）
            let k2 = kl[2];
            let sz = sel(k2.gt(0.0), one, sel(k2.lt(0.0), -one, zero));
            let inv = 1.0 / (1.0 + k2.abs());
            let h = [(kl[0] + 0.0) * inv, (kl[1] + 0.0) * inv, (k2 + sz) * inv];
            let u = [1.0 - h[0] * kl[0], 0.0 - h[1] * kl[0], 0.0 - h[2] * kl[0]];
            let v = [0.0 - h[0] * kl[1], 1.0 - h[1] * kl[1], 0.0 - h[2] * kl[1]];
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
            let inv = 1.0 / dot_l(&sv, &sv).sqrt();
            let sv = v3!(c => sv[c] * inv);
            let pin = cross_l(&sv, kin);
            let mut pout = cross_l(&sv, kl);
            if mirror {
                pout = v3!(c => pout[c] * -1.0);
            }
            let (mut t0, mut t1) = ((zero, zero), (zero, zero));
            for i in 0..L {
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
fn aperture_out_l(a: Aperture, pl: &VL) -> B {
    // 半径の 2 乗の丸めの誤差（数 ulp）より十分大きい幅
    const M: f64 = 1e-12;
    #[inline(always)]
    fn ring(a: Aperture, pl: &VL, (dx, dy): (f64, f64), (lo, hi): (f64, f64), obsc: bool) -> B {
        let (u, v) = (pl[0] - dx, pl[1] - dy);
        let r = u * u + v * v;
        let (lo2, hi2) = (lo * lo, hi * hi);
        // inside: lo < r < hi が確か、outside: r < lo か r > hi が確か（どちらでもなければ hypot で）
        let (has_lo, has_hi) = (B::splat(lo > 0.0), B::splat(hi > 0.0));
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
            for i in 0..L {
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
        _ => B::from_fn(|i| trace::aperture_out(a, get(pl, i))),
    }
}

#[inline(always)]
fn run<R: LaneSink>(
    plan: &Plan,
    p0: &[V3; L],
    k0: &[V3; L],
    mut rec: Option<&mut R>,
    mut pol: Option<&mut PolLanes>,
) -> [RayOut; L] {
    let mut pe = pol.as_deref().map(PolSoa::new);
    let nan = F::splat(f64::NAN);
    let nan3 = [nan; 3];
    let zero = F::splat(0.0);
    let p: VL = v3!(c => F::from_fn(|i| p0[i][c]));
    let k: VL = v3!(c => F::from_fn(|i| k0[i][c]));
    // 止まった面・けられた面の番号（f64 で持つと比較・選択がまとめてできる）
    let mut err = zero;
    let mut tir = B::splat(false);
    let mut vig = zero;
    let mut opl = zero;
    let mut lens_opd = zero;
    // 今いる面の局所座標の位置・方向
    let mut pl = nan3;
    let mut kl = nan3;
    for (ri, st) in plan.steps.iter().enumerate() {
        let (d, j) = (st.d, st.j);
        let jf = F::splat(j as f64);
        let nprev = st.nprev;
        // ここまでに止まった光線（以後は NaN）
        let es: B = err.ne(0.0);
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
                let dp = v3!(c => p[c] - o[c]);
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
        // 頂点面へ
        let s = -pl[2] / kl[2];
        err = sel(err.eq(0.0) & !s.is_finite(), jf, err);
        pl = axpy(pl, kl, s);
        opl += nprev * s;
        // この面までに進んだ長さ（偏光の吸収）
        let mut s_tot = s;
        if crate::freeform::is_freeform(d.typ) {
            for i in 0..L {
                if err[i] == 0.0 {
                    let (pi, ki) = (get(&pl, i), get(&kl, i));
                    let (s, bad) = crate::freeform::isect(d, pi, ki, st.sig);
                    if bad {
                        err[i] = j as f64;
                    }
                    put(&mut pl, i, add(pi, scale(ki, s)));
                    opl[i] += nprev * s;
                    s_tot[i] += s;
                }
            }
        } else if st.curved && d.typ == SurfType::Toroidal {
            for i in 0..L {
                if err[i] == 0.0 {
                    let (pi, ki) = (get(&pl, i), get(&kl, i));
                    let (s, bad) = trace::toroidal_isect(d, pi, ki, st.sig);
                    if bad {
                        err[i] = j as f64;
                    }
                    put(&mut pl, i, add(pi, scale(ki, s)));
                    opl[i] += nprev * s;
                    s_tot[i] += s;
                }
            }
        } else if st.curved {
            let m: B = err.eq(0.0);
            let c = d.c;
            let mut s = zero;
            if c != 0.0 {
                let ff = c * (pl[0] * pl[0] + pl[1] * pl[1]);
                let gg = kl[2] - c * (pl[0] * kl[0] + pl[1] * kl[1]);
                let aa = c * (kl[0] * kl[0] + kl[1] * kl[1] + st.k1 * kl[2] * kl[2]);
                let disc = gg * gg - aa * ff;
                s = conic_root_l(ff, gg, aa, disc.max0(), st.sig);
                // !(disc >= 0)（NaN も）か s が有限でない
                err = sel(m & !(disc.ge(0.0) & s.is_finite()), jf, err);
            }
            if st.asph {
                let mut act: B = m & err.eq(0.0);
                let newton = act;
                let mut ds = zero;
                for _ in 0..60 {
                    if !act.any() {
                        break;
                    }
                    let q = axpy(pl, kl, s);
                    let r2 = q[0] * q[0] + q[1] * q[1];
                    let (z, dz) = st.sag.eval(r2);
                    let dn = (q[2] - z) / (kl[2] - 2.0 * dz * (q[0] * kl[0] + q[1] * kl[1]));
                    ds = sel(act, dn, ds);
                    s = sel(act, s - dn, s);
                    act = act & dn.abs().gt(1e-14);
                }
                // !(|ds| <= 1e-10)（NaN も）
                err = sel(newton & !ds.abs().le(1e-10), jf, err);
            }
            pl = sel3(m, axpy(pl, kl, s), pl);
            opl = sel(m, opl + nprev * s, opl);
            s_tot = sel(m, s_tot + s, s_tot);
        }
        // 通ってきた媒質の吸収（振幅は内部透過率の 1/2 乗。[`trace::trace_core`] と同じ）
        if let (Some(pr), Some(e)) = (pol.as_deref(), pe.as_mut()) {
            let ps = &pr.surf[j];
            if ps.it.is_some() {
                let la = ps.la;
                let a = F::from_fn(|i| {
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
        if let Some(a) = st.clip {
            let out = aperture_out_l(a, &pl);
            vig = sel(out & vig.eq(0.0) & err.eq(0.0), jf, vig);
        }
        match d.typ {
            SurfType::CoordBrk => {}
            SurfType::Paraxial => {
                for i in 0..L {
                    if !es.at(i) {
                        let (k2, dl) = trace::paraxial(d.focal, get(&pl, i), get(&kl, i), st.conj);
                        // 電場は方向の変化と同じ最小の回転で回す（[`trace::trace_core`] と同じ）
                        if let (Some(e), true) = (pe.as_mut(), err[i] == 0.0) {
                            let k1 = get(&kl, i);
                            let (ax, c) = (cross(k1, k2), dot(k1, k2));
                            for q in 0..e.re.len() {
                                let x = trace::rotate_min(e.get(i, q), ax, c);
                                e.put(i, q, x);
                            }
                        }
                        put(&mut kl, i, k2);
                        lens_opd[i] += dl;
                    }
                }
            }
            // 平面で屈折率が変わらない面（鏡でない）は方向が変わらないので、法線を求めない
            // （求めても使わない。結果は同じビット）
            _ if !d.mirror
                && !crate::freeform::is_freeform(d.typ)
                && d.typ != SurfType::Toroidal
                && !(d.typ == SurfType::EvenAsph && st.asph)
                && d.c == 0.0
                && (d.typ == SurfType::Jones || st.nn == nprev) =>
            {
                // 偏光は法線 (0, 0, 1) で面の処理をする（[`trace::trace_core`] と同じ値）
                if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
                    if !matches!(pr.surf[j].kind, PolKind::None) {
                        let nv: VL = [zero, zero, F::splat(1.0)];
                        let cosi = dot_l(&kl, &nv);
                        let act = err.eq(0.0);
                        pol_surface_l(plan.sys, pr, e, j, (&kl, &kl, &nv, cosi), false, act);
                    }
                }
            }
            _ => {
                let one = F::splat(1.0);
                let nvec: VL = if crate::freeform::is_freeform(d.typ) {
                    let mut nv = [zero, zero, one];
                    for i in 0..L {
                        let (_, zx, zy) = crate::freeform::sag_xy(d, pl[0][i], pl[1][i]);
                        if !(zx.is_finite() && zy.is_finite()) && err[i] == 0.0 {
                            err[i] = j as f64;
                        }
                        nv[0][i] = -zx;
                        nv[1][i] = -zy;
                    }
                    nv
                } else if d.typ == SurfType::Toroidal {
                    let mut nv = [zero, zero, one];
                    for i in 0..L {
                        let (_, zx, zy) =
                            trace::sag_toroidal(d.c, d.k, &d.asph, d.cx, pl[0][i], pl[1][i]);
                        if !(zx.is_finite() && zy.is_finite()) && err[i] == 0.0 {
                            err[i] = j as f64;
                        }
                        nv[0][i] = -zx;
                        nv[1][i] = -zy;
                    }
                    nv
                } else if d.typ == SurfType::EvenAsph && st.asph {
                    let r2 = pl[0] * pl[0] + pl[1] * pl[1];
                    let (_, dz) = st.sag.eval(r2);
                    err = sel(!dz.is_finite() & err.eq(0.0), jf, err);
                    [-2.0 * dz * pl[0], -2.0 * dz * pl[1], one]
                } else if d.c == 0.0 {
                    [zero, zero, one]
                } else {
                    let c = d.c;
                    [-c * pl[0], -c * pl[1], 1.0 - st.ck1 * pl[2]]
                };
                let inv = 1.0 / (nvec[0] * nvec[0] + nvec[1] * nvec[1] + nvec[2] * nvec[2]).sqrt();
                let nv: VL = v3!(c => nvec[c] * inv);
                let cosi = kl[0] * nv[0] + kl[1] * nv[1] + kl[2] * nv[2];
                let kin = kl;
                if d.mirror {
                    kl = v3!(c => kl[c] - nv[c] * (2.0 * cosi));
                } else if d.typ != SurfType::Jones && st.nn != nprev {
                    let mu = nprev / st.nn;
                    let mu2 = mu * mu;
                    let disc = 1.0 - mu2 * (1.0 - cosi * cosi);
                    let total = disc.lt(0.0) & err.eq(0.0);
                    err = sel(total, jf, err);
                    tir = tir | total;
                    let t = cosi.signum() * disc.max0().sqrt() - mu * cosi;
                    kl = v3!(c => kl[c] * mu + nv[c] * t);
                }
                if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
                    if !matches!(pr.surf[j].kind, PolKind::None) {
                        let act = err.eq(0.0);
                        pol_surface_l(plan.sys, pr, e, j, (&kin, &kl, &nv, cosi), d.mirror, act);
                    }
                }
            }
        }
        let bad: B = err.ne(0.0);
        pl = sel3(bad, nan3, pl);
        kl = sel3(bad, nan3, kl);
        opl = sel(bad, nan, opl);
        if let Some(r) = rec.as_deref_mut().filter(|r| r.wants(ri)) {
            let pg = mv_l(&st.f.r, &pl);
            for i in 0..L {
                let e = if es.at(i) {
                    RecEntry {
                        surf: j,
                        xyz: [f64::NAN; 3],
                        lmn: [f64::NAN; 3],
                        opl: f64::NAN,
                        p: [f64::NAN; 3],
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
        for i in 0..L {
            let dead = err[i] != 0.0 || pr.err[i].is_some();
            for q in 0..np {
                pr.e[i * np + q] = if dead { trace::CNAN3 } else { e.get(i, q) };
            }
        }
    }
    let f = plan.steps.last().expect("追う面がない").f;
    let pg = mv_l(&f.r, &pl);
    let kg = mv_l(&f.r, &kl);
    std::array::from_fn(|i| RayOut {
        xyz: get(&pl, i),
        lmn: get(&kl, i),
        opl: opl[i],
        err: err[i] as usize,
        tir: tir.at(i),
        vig: vig[i] as usize,
        p: add(f.o, get(&pg, i)),
        k: get(&kg, i),
        lens_opd: lens_opd[i],
    })
}

#[cfg(test)]
mod tests {
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
                let mut pl: VL = [F::splat(0.0); 3];
                for i in 0..L {
                    let r = rs[i % rs.len()];
                    let t = ang + i as f64 * 0.7;
                    pl[0][i] = r * t.cos();
                    pl[1][i] = r * t.sin();
                }
                let out = aperture_out_l(a, &pl);
                for i in 0..L {
                    assert_eq!(out.at(i), trace::aperture_out(a, get(&pl, i)), "{a:?} {i}");
                }
            }
        }
    }
}
