//! 光線を [`L`] 本ずつまとめて追跡する（SIMD）。
//!
//! [`crate::trace::trace_core`]（偏光なし）と同じ演算を同じ順に光線ごとに行う。光線の間で値を混ぜず、
//! 分岐は光線ごとの選択に置き換えるだけなので、結果は 1 本ずつの追跡とビット単位で一致する
//! （FMA などの融合はしない。`tests/batch.rs` で確かめる）。光線の組は入力の順に L 本ずつに固定するので、
//! スレッド数にもよらない。
//!
//! 演算は光線の配列への素直なループで書き、コンパイラのベクトル化に任せる。x86_64 では AVX があれば
//! 4 本を 1 命令で計算する版を実行時に選ぶ（AVX は演算の結果を変えない。FMA は有効にしない）。
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

/// 1 回にまとめて追う光線の数（AVX の 4 本 × 4。多いほど面ごとの準備の手間が薄まる）
pub const L: usize = 16;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// まとめ追跡を使うか（既定は使う）。結果は同じビットなので、切り分け・比較のためだけに使う
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// n 本の追跡をまとめて追うか（少ない本数は 1 本ずつの方が速い）
pub fn use_for(n: usize) -> bool {
    n >= L / 4 && ENABLED.load(Ordering::Relaxed)
}

type F = [f64; L];
type B = [bool; L];
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

macro_rules! lanes {
    ($i:ident => $e:expr) => {{
        let mut o = [0.0f64; L];
        for $i in 0..L {
            o[$i] = $e;
        }
        o
    }};
}

#[inline(always)]
fn any(m: B) -> bool {
    m.iter().any(|&b| b)
}

#[inline(always)]
fn sel(m: B, a: F, b: F) -> F {
    lanes!(i => if m[i] { a[i] } else { b[i] })
}

#[inline(always)]
fn sel3(m: B, a: VL, b: VL) -> VL {
    [sel(m, a[0], b[0]), sel(m, a[1], b[1]), sel(m, a[2], b[2])]
}

/// a + b·s（光線ごとの s）
#[inline(always)]
fn axpy(a: VL, b: VL, s: F) -> VL {
    v3!(c => lanes!(i => a[c][i] + b[c][i] * s[i]))
}

/// M v（[`mv`] と同じ順）
#[inline(always)]
fn mv_l(m: &M3, v: &VL) -> VL {
    v3!(r => lanes!(i => m[r][0] * v[0][i] + m[r][1] * v[1][i] + m[r][2] * v[2][i]))
}

/// Mᵀ v（[`mtv`] と同じ順）
#[inline(always)]
fn mtv_l(m: &M3, v: &VL) -> VL {
    v3!(r => lanes!(i => m[0][r] * v[0][i] + m[1][r] * v[1][i] + m[2][r] * v[2][i]))
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
    let mut r = [1.0; L];
    loop {
        if n & 1 != 0 {
            r = lanes!(i => r[i] * x[i]);
        }
        n /= 2;
        if n == 0 {
            return r;
        }
        x = lanes!(i => x[i] * x[i]);
    }
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
        let arg = lanes!(i => 1.0 - self.kcc * r2[i]);
        let root = if c != 0.0 {
            lanes!(i => arg[i].max(0.0).sqrt())
        } else {
            [1.0; L]
        };
        let mut z = lanes!(i => c * r2[i] / (1.0 + root[i]));
        let mut dz = if c != 0.0 {
            lanes!(i => if arg[i] > 0.0 { c / (2.0 * root[i]) } else { f64::NAN })
        } else {
            [0.0; L]
        };
        for &(n, a, na) in &self.terms {
            let p = powi_l(r2, n);
            let q = powi_l(r2, n - 1);
            z = lanes!(i => z[i] + a * p[i]);
            dz = lanes!(i => dz[i] + na * q[i]);
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

/// L 本の電場（偏光の入力ごとに実部・虚部の成分を光線の配列で持つ）
struct PolSoa {
    re: Vec<VL>,
    im: Vec<VL>,
}

impl PolSoa {
    fn new(pr: &PolLanes) -> PolSoa {
        let np = pr.np;
        let mut re = vec![[[0.0; L]; 3]; np];
        let mut im = vec![[[0.0; L]; 3]; np];
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
    fn scale(&mut self, a: &F) {
        for v in self.re.iter_mut().chain(self.im.iter_mut()) {
            for c in 0..3 {
                v[c] = lanes!(i => v[c][i] * a[i]);
            }
        }
    }
}

/// 面での電場の変化（[`trace::pol_surface`] の L 本版。複屈折材の板を除く）。g = 面の前後の方向・法線・入射角の
/// cos、act = 処理する光線（ほかの光線の電場は使わない値になる）。係数のエラーは光線ごとに持つ
#[inline(always)]
fn pol_surface_l(
    sys: &System,
    pr: &mut PolLanes,
    e: &mut PolSoa,
    j: usize,
    g: (&VL, &VL, &VL, &F),
    mirror: bool,
    act: B,
) {
    let (kin, kl, nv, cosi) = g;
    match &pr.surf[j].kind {
        PolKind::None => {}
        PolKind::Jones(m) => {
            // u・v は [`trace::pol_surface`] と同じ式（+0.0 の足し算も同じ）
            let mut u: VL = [[0.0; L]; 3];
            let mut v: VL = [[0.0; L]; 3];
            for i in 0..L {
                let k2 = kl[2][i];
                let sz = if k2 > 0.0 {
                    1.0
                } else if k2 < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                let h = scale(add(get(kl, i), [0.0, 0.0, sz]), 1.0 / (1.0 + k2.abs()));
                put(&mut u, i, sub([1.0, 0.0, 0.0], scale(h, kl[0][i])));
                put(&mut v, i, sub([0.0, 1.0, 0.0], scale(h, kl[1][i])));
            }
            for q in 0..e.re.len() {
                let (xr, xi) = (&mut e.re[q], &mut e.im[q]);
                let d = |x: &VL, w: &VL| lanes!(i => x[0][i] * w[0][i] + x[1][i] * w[1][i] + x[2][i] * w[2][i]);
                let (eur, eui, evr, evi) = (d(xr, &u), d(xi, &u), d(xr, &v), d(xi, &v));
                let (m0, m1, m2, m3) = (m[0], m[1], m[2], m[3]);
                let ar = lanes!(i => (m0.re * eur[i] - m0.im * eui[i]) + (m1.re * evr[i] - m1.im * evi[i]));
                let ai = lanes!(i => (m0.re * eui[i] + m0.im * eur[i]) + (m1.re * evi[i] + m1.im * evr[i]));
                let br = lanes!(i => (m2.re * eur[i] - m2.im * eui[i]) + (m3.re * evr[i] - m3.im * evi[i]));
                let bi = lanes!(i => (m2.re * eui[i] + m2.im * eur[i]) + (m3.re * evi[i] + m3.im * evr[i]));
                for c in 0..3 {
                    xr[c] = lanes!(i => ar[i] * u[c][i] + br[i] * v[c][i]);
                    xi[c] = lanes!(i => ai[i] * u[c][i] + bi[i] * v[c][i]);
                }
            }
        }
        PolKind::Iface(f) => {
            let mut sv: VL = [[0.0; L]; 3];
            let mut pin: VL = [[0.0; L]; 3];
            let mut pout: VL = [[0.0; L]; 3];
            let mut tr = [[0.0; L]; 2];
            let mut ti = [[0.0; L]; 2];
            for i in 0..L {
                let ki = get(kin, i);
                let mut s = cross(ki, get(nv, i));
                if !(norm(s) > 1e-12) {
                    s = cross(ki, [1.0, 0.0, 0.0]);
                }
                let s = normalize(s);
                put(&mut sv, i, s);
                put(&mut pin, i, cross(s, ki));
                let mut po = cross(s, get(kl, i));
                if mirror {
                    po = scale(po, -1.0);
                }
                put(&mut pout, i, po);
                if act[i] {
                    match f.eval_cos(cosi[i].abs().clamp(0.0, 1.0)) {
                        Ok(t) => {
                            tr[0][i] = t[0].re;
                            ti[0][i] = t[0].im;
                            tr[1][i] = t[1].re;
                            ti[1][i] = t[1].im;
                        }
                        Err(er) => {
                            pr.err[i].get_or_insert_with(|| er.at(sys.surf_label(j)));
                        }
                    }
                }
            }
            for q in 0..e.re.len() {
                let (xr, xi) = (&mut e.re[q], &mut e.im[q]);
                let d = |x: &VL, w: &VL| lanes!(i => x[0][i] * w[0][i] + x[1][i] * w[1][i] + x[2][i] * w[2][i]);
                let (esr, esi, epr, epi) = (d(xr, &sv), d(xi, &sv), d(xr, &pin), d(xi, &pin));
                let ar = lanes!(i => tr[0][i] * esr[i] - ti[0][i] * esi[i]);
                let ai = lanes!(i => tr[0][i] * esi[i] + ti[0][i] * esr[i]);
                let br = lanes!(i => tr[1][i] * epr[i] - ti[1][i] * epi[i]);
                let bi = lanes!(i => tr[1][i] * epi[i] + ti[1][i] * epr[i]);
                for c in 0..3 {
                    xr[c] = lanes!(i => ar[i] * sv[c][i] + br[i] * pout[c][i]);
                    xi[c] = lanes!(i => ai[i] * sv[c][i] + bi[i] * pout[c][i]);
                }
            }
        }
    }
}

/// 開口の外か（光線ごと）。円・遮蔽は半径の 2 乗で十分に離れていればそれで決め、
/// 境界の近く（と NaN・無限大）だけ [`trace::aperture_out`]（hypot）で決める
#[inline(always)]
fn aperture_out_l(a: Aperture, pl: &VL) -> B {
    let mut out = [false; L];
    // 半径の 2 乗の丸めの誤差（数 ulp）より十分大きい幅
    const M: f64 = 1e-12;
    #[inline(always)]
    fn ring(
        a: Aperture,
        pl: &VL,
        (dx, dy): (f64, f64),
        (lo, hi): (f64, f64),
        obsc: bool,
        out: &mut B,
    ) {
        let r2 = lanes!(i => {
            let (u, v) = (pl[0][i] - dx, pl[1][i] - dy);
            u * u + v * v
        });
        let (lo2, hi2) = (lo * lo, hi * hi);
        // inside: lo < r < hi が確か、outside: r < lo か r > hi が確か（どちらでもなければ hypot で）。
        // 分岐なし（& と |）で書いて 1 組をまとめて比べる
        let (has_lo, has_hi) = (lo > 0.0, hi > 0.0);
        let (hi_in, hi_out, lo_in, lo_out) = (
            hi2 * (1.0 - M),
            hi2 * (1.0 + M),
            lo2 * (1.0 + M),
            lo2 * (1.0 - M),
        );
        let mut unsure = false;
        for i in 0..L {
            let r = r2[i];
            let inside = has_hi & (r < hi_in) & (!has_lo | (r > lo_in));
            let outside = (has_lo & (r < lo_out)) | (has_hi & (r > hi_out) & (r < f64::INFINITY));
            out[i] = if obsc { inside } else { outside };
            unsure |= !(inside | outside);
        }
        if unsure {
            for i in 0..L {
                let r = r2[i];
                let inside = has_hi & (r < hi_in) & (!has_lo | (r > lo_in));
                let outside =
                    (has_lo & (r < lo_out)) | (has_hi & (r > hi_out) & (r < f64::INFINITY));
                if !(inside | outside) {
                    out[i] = trace::aperture_out(a, get(pl, i));
                }
            }
        }
    }
    match a {
        Aperture::Circular { min, max, dx, dy } => {
            ring(a, pl, (dx, dy), (min - 1e-9, max + 1e-9), false, &mut out)
        }
        Aperture::Obsc { min, max, dx, dy } => {
            ring(a, pl, (dx, dy), (min - 1e-9, max + 1e-9), true, &mut out)
        }
        // [`trace::aperture_out`] と同じ比較を光線ごとに（NaN は外にならない）
        Aperture::Rect { hx, hy, dx, dy } => {
            let (bx, by) = (hx + 1e-9, hy + 1e-9);
            for i in 0..L {
                out[i] = ((pl[0][i] - dx).abs() > bx) | ((pl[1][i] - dy).abs() > by);
            }
        }
        _ => {
            for i in 0..L {
                out[i] = trace::aperture_out(a, get(pl, i));
            }
        }
    }
    out
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
    let nan = [f64::NAN; L];
    let nan3 = [nan; 3];
    let p: VL = v3!(c => lanes!(i => p0[i][c]));
    let k: VL = v3!(c => lanes!(i => k0[i][c]));
    // 止まった面・けられた面の番号（f64 で持つと比較・選択が 4 本まとめてできる）
    let mut err = [0.0f64; L];
    let mut tir = [false; L];
    let mut vig = [0.0f64; L];
    let mut opl = [0.0; L];
    let mut lens_opd = [0.0; L];
    // 今いる面の局所座標の位置・方向
    let mut pl = nan3;
    let mut kl = nan3;
    for (ri, st) in plan.steps.iter().enumerate() {
        let (d, j) = (st.d, st.j);
        let jf = j as f64;
        let nprev = st.nprev;
        // ここまでに止まった光線（以後は NaN）
        let es: B = lanes_b(|i| err[i] != 0.0);
        match st.mv {
            Move::Shift(dz) => {
                for i in 0..L {
                    pl[2][i] += dz;
                }
            }
            Move::Rel(q) => {
                pl = mv_l(&q.r, &pl);
                for c in 0..3 {
                    for i in 0..L {
                        pl[c][i] += q.t[c];
                    }
                }
                kl = mv_l(&q.r, &kl);
            }
            Move::First => {
                let o = st.f.o;
                let dp = v3!(c => lanes!(i => p[c][i] - o[c]));
                pl = mtv_l(&st.f.r, &dp);
                kl = mtv_l(&st.f.r, &k);
            }
        }
        // 電場も方向と同じ変換で今の面の局所座標へ（止まった光線は最後に NaN にするので飛ばす）
        if let Some(e) = pe.as_mut() {
            match st.mv {
                Move::Shift(_) => {}
                Move::Rel(q) => e.mv(&q.r),
                Move::First => e.mtv(&st.f.r),
            }
        }
        // 頂点面へ
        let s = lanes!(i => -pl[2][i] / kl[2][i]);
        for i in 0..L {
            if err[i] == 0.0 && !s[i].is_finite() {
                err[i] = jf;
            }
        }
        pl = axpy(pl, kl, s);
        opl = lanes!(i => opl[i] + nprev * s[i]);
        // この面までに進んだ長さ（偏光の吸収）
        let mut s_tot = s;
        if crate::freeform::is_freeform(d.typ) {
            for i in 0..L {
                if err[i] == 0.0 {
                    let (pi, ki) = (get(&pl, i), get(&kl, i));
                    let (s, bad) = crate::freeform::isect(d, pi, ki, st.sig);
                    if bad {
                        err[i] = jf;
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
                        err[i] = jf;
                    }
                    put(&mut pl, i, add(pi, scale(ki, s)));
                    opl[i] += nprev * s;
                    s_tot[i] += s;
                }
            }
        } else if st.curved {
            let m: B = lanes_b(|i| err[i] == 0.0);
            let c = d.c;
            let mut s = [0.0; L];
            if c != 0.0 {
                let ff = lanes!(i => c * (pl[0][i] * pl[0][i] + pl[1][i] * pl[1][i]));
                let gg = lanes!(i => kl[2][i] - c * (pl[0][i] * kl[0][i] + pl[1][i] * kl[1][i]));
                let aa = lanes!(i => c * (kl[0][i] * kl[0][i] + kl[1][i] * kl[1][i]
                    + st.k1 * kl[2][i] * kl[2][i]));
                let disc = lanes!(i => gg[i] * gg[i] - aa[i] * ff[i]);
                s = lanes!(i => trace::conic_root(ff[i], gg[i], aa[i], disc[i].max(0.0), st.sig));
                for i in 0..L {
                    if m[i] && (!(disc[i] >= 0.0) || !s[i].is_finite()) {
                        err[i] = jf;
                    }
                }
            }
            if st.asph {
                let mut act: B = lanes_b(|i| m[i] && err[i] == 0.0);
                let newton = act;
                let mut ds = [0.0; L];
                for _ in 0..60 {
                    if !any(act) {
                        break;
                    }
                    let q = axpy(pl, kl, s);
                    let r2 = lanes!(i => q[0][i] * q[0][i] + q[1][i] * q[1][i]);
                    let (z, dz) = st.sag.eval(r2);
                    let dn = lanes!(i => (q[2][i] - z[i])
                        / (kl[2][i] - 2.0 * dz[i] * (q[0][i] * kl[0][i] + q[1][i] * kl[1][i])));
                    ds = sel(act, dn, ds);
                    s = sel(act, lanes!(i => s[i] - dn[i]), s);
                    act = lanes_b(|i| act[i] && dn[i].abs() > 1e-14);
                }
                for i in 0..L {
                    if newton[i] && !(ds[i].abs() <= 1e-10) {
                        err[i] = jf;
                    }
                }
            }
            pl = sel3(m, axpy(pl, kl, s), pl);
            opl = sel(m, lanes!(i => opl[i] + nprev * s[i]), opl);
            s_tot = sel(m, lanes!(i => s_tot[i] + s[i]), s_tot);
        }
        // 通ってきた媒質の吸収（振幅は内部透過率の 1/2 乗。[`trace::trace_core`] と同じ）
        if let (Some(pr), Some(e)) = (pol.as_deref(), pe.as_mut()) {
            let ps = &pr.surf[j];
            if ps.it.is_some() {
                let la = ps.la;
                let a = lanes!(i => if es[i] || s_tot[i] == 0.0 {
                    1.0
                } else {
                    (la * s_tot[i]).exp()
                });
                e.scale(&a);
            }
        }
        // 開口
        if let Some(a) = st.clip {
            let out = aperture_out_l(a, &pl);
            for i in 0..L {
                if out[i] && vig[i] == 0.0 && err[i] == 0.0 {
                    vig[i] = jf;
                }
            }
        }
        match d.typ {
            SurfType::CoordBrk => {}
            SurfType::Paraxial => {
                for i in 0..L {
                    if !es[i] {
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
                        let nv: VL = [[0.0; L], [0.0; L], [1.0; L]];
                        let cosi = lanes!(i => kl[0][i] * 0.0 + kl[1][i] * 0.0 + kl[2][i] * 1.0);
                        let act = lanes_b(|i| err[i] == 0.0);
                        pol_surface_l(plan.sys, pr, e, j, (&kl, &kl, &nv, &cosi), false, act);
                    }
                }
            }
            _ => {
                let nvec: VL = if crate::freeform::is_freeform(d.typ) {
                    let mut nv = [[0.0; L], [0.0; L], [1.0; L]];
                    for i in 0..L {
                        let (_, zx, zy) = crate::freeform::sag_xy(d, pl[0][i], pl[1][i]);
                        if !(zx.is_finite() && zy.is_finite()) && err[i] == 0.0 {
                            err[i] = jf;
                        }
                        nv[0][i] = -zx;
                        nv[1][i] = -zy;
                    }
                    nv
                } else if d.typ == SurfType::Toroidal {
                    let mut nv = [[0.0; L], [0.0; L], [1.0; L]];
                    for i in 0..L {
                        let (_, zx, zy) =
                            trace::sag_toroidal(d.c, d.k, &d.asph, d.cx, pl[0][i], pl[1][i]);
                        if !(zx.is_finite() && zy.is_finite()) && err[i] == 0.0 {
                            err[i] = jf;
                        }
                        nv[0][i] = -zx;
                        nv[1][i] = -zy;
                    }
                    nv
                } else if d.typ == SurfType::EvenAsph && st.asph {
                    let r2 = lanes!(i => pl[0][i] * pl[0][i] + pl[1][i] * pl[1][i]);
                    let (_, dz) = st.sag.eval(r2);
                    for i in 0..L {
                        if !dz[i].is_finite() && err[i] == 0.0 {
                            err[i] = jf;
                        }
                    }
                    [
                        lanes!(i => -2.0 * dz[i] * pl[0][i]),
                        lanes!(i => -2.0 * dz[i] * pl[1][i]),
                        [1.0; L],
                    ]
                } else if d.c == 0.0 {
                    [[0.0; L], [0.0; L], [1.0; L]]
                } else {
                    let c = d.c;
                    [
                        lanes!(i => -c * pl[0][i]),
                        lanes!(i => -c * pl[1][i]),
                        lanes!(i => 1.0 - st.ck1 * pl[2][i]),
                    ]
                };
                let inv = lanes!(i => 1.0 / (nvec[0][i] * nvec[0][i] + nvec[1][i] * nvec[1][i]
                    + nvec[2][i] * nvec[2][i]).sqrt());
                let nv: VL = v3!(c => lanes!(i => nvec[c][i] * inv[i]));
                let cosi =
                    lanes!(i => kl[0][i] * nv[0][i] + kl[1][i] * nv[1][i] + kl[2][i] * nv[2][i]);
                let kin = kl;
                if d.mirror {
                    kl = v3!(c => lanes!(i => kl[c][i] - nv[c][i] * (2.0 * cosi[i])));
                } else if d.typ != SurfType::Jones && st.nn != nprev {
                    let mu = nprev / st.nn;
                    let mu2 = mu * mu;
                    let disc = lanes!(i => 1.0 - mu2 * (1.0 - cosi[i] * cosi[i]));
                    for i in 0..L {
                        if disc[i] < 0.0 && err[i] == 0.0 {
                            err[i] = jf;
                            tir[i] = true;
                        }
                    }
                    let t = lanes!(i => cosi[i].signum() * disc[i].max(0.0).sqrt() - mu * cosi[i]);
                    kl = v3!(c => lanes!(i => kl[c][i] * mu + nv[c][i] * t[i]));
                }
                if let (Some(pr), Some(e)) = (pol.as_deref_mut(), pe.as_mut()) {
                    if !matches!(pr.surf[j].kind, PolKind::None) {
                        let act = lanes_b(|i| err[i] == 0.0);
                        pol_surface_l(plan.sys, pr, e, j, (&kin, &kl, &nv, &cosi), d.mirror, act);
                    }
                }
            }
        }
        let bad: B = lanes_b(|i| err[i] != 0.0);
        for i in 0..L {
            if bad[i] {
                for c in 0..3 {
                    pl[c][i] = f64::NAN;
                    kl[c][i] = f64::NAN;
                }
                opl[i] = f64::NAN;
            }
        }
        if let Some(r) = rec.as_deref_mut().filter(|r| r.wants(ri)) {
            let pg = mv_l(&st.f.r, &pl);
            for i in 0..L {
                let e = if es[i] {
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
            for q in 0..np {
                pr.e[i * np + q] = if err[i] != 0.0 || pr.err[i].is_some() {
                    trace::CNAN3
                } else {
                    e.get(i, q)
                };
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
        tir: tir[i],
        vig: vig[i] as usize,
        p: add(f.o, get(&pg, i)),
        k: get(&kg, i),
        lens_opd: lens_opd[i],
    })
}

#[inline(always)]
fn lanes_b(f: impl Fn(usize) -> bool) -> B {
    std::array::from_fn(f)
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
                let mut pl: VL = [[0.0; L]; 3];
                for i in 0..L {
                    let r = rs[i % rs.len()];
                    let t = ang + i as f64 * 0.7;
                    pl[0][i] = r * t.cos();
                    pl[1][i] = r * t.sin();
                }
                let out = aperture_out_l(a, &pl);
                for i in 0..L {
                    assert_eq!(out[i], trace::aperture_out(a, get(&pl, i)), "{a:?} {i}");
                }
            }
        }
    }
}
