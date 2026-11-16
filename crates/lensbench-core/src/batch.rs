//! 実光線の追跡のカーネル（1 本ずつと、[`L`] 本ずつまとめた SIMD の両方）。
//!
//! カーネル（[`kernel`]）は光線の組の型 [`lanes::Lanes`] についてジェネリックに 1 本だけ書き、
//! 幅 1（[`lanes::One`]、1 本ずつの追跡）と幅 L（[`lanes::Wide`]、まとめ追跡）に展開する（KRN-06）。
//! 光線の間で値を混ぜず、分岐は光線ごとの選択に置き換えるだけなので、結果は幅によらず光線ごとに
//! 同じビット（FMA などの融合はしない。`tests/batch.rs` で確かめる）。光線の組は入力の順に L 本ずつに
//! 固定するので、スレッド数にもよらない。
//!
//! 光線が面に当たらない・全反射するとその面で止まり（err = 面番号、以後 NaN）、開口の外に出た光線は
//! 先へ進む（vig = 最初の面）。OpticStudio のバッチ追跡と同じ扱い。
//!
//! ネイティブでは組は光線の配列で、演算は素直なループ（コンパイラのベクトル化に任せる）。x86_64 では AVX が
//! あれば 4 本を 1 命令で計算する版を実行時に選ぶ（AVX は演算の結果を変えない。FMA は有効にしない）。
//! wasm ＋ simd128 では f64x2 の命令を名指しする（LLVM が wasm ではこのループをベクトル化しないため。公開 #99）。
//! トロイダル面・自由曲面・理想レンズ・Fresnel・Binary 2・GRIN の媒質・利用者定義の面・複屈折材の板・開口の
//! 際どい判定など、まれな・重い処理はその面だけ光線ごとのスカラーで行う（系全体は組のまま）。

use crate::action::Act;
use crate::math::*;
use crate::shape::SurfKind;
use crate::surface::Surface;
use crate::system::{Frame, Frames, Rel, System};
use crate::trace::{RayOut, RecEntry, RecSink};
use crate::LockExt;
use std::sync::{Arc, Mutex};

/// 1 回にまとめて追う光線の数（AVX の 4 本 × 4。多いほど面ごとの準備の手間が薄まる）。
/// wasm は 8 本（f64x2 × 4）: 16 本では値の組がレジスタに載らず、読み書きが増えて遅い（公開 #99）
#[cfg(not(target_arch = "wasm32"))]
pub const L: usize = 16;
#[cfg(target_arch = "wasm32")]
pub const L: usize = 8;

/// n 本の追跡をまとめて追うか（少ない本数は 1 本ずつの方が速い）。まとめ追跡は
/// [`crate::par::with_batch`] で切れる（結果は同じビットなので、切り分け・比較のためだけに使う）
pub(crate) fn use_for(n: usize) -> bool {
    n >= L / 4 && crate::par::batch_enabled()
}

mod kernel;
pub(crate) mod lanes;
mod pol;

pub(crate) use pol::PolLanes;

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
}

/// GRIN の媒質の中を進む面（媒質の始まりの面 gj の局所座標で進め、この面との交点で止める）
#[derive(Clone, Copy)]
struct GrinIn {
    /// 媒質の始まりの面
    gj: usize,
    /// 面 gj の局所座標 → この面の局所座標（回転 rr、移動 tt）
    rr: M3,
    tt: V3,
    /// 直前の面が座標ブレークのとき、その局所座標 → 面 gj の局所座標
    back: Option<(M3, V3)>,
    /// 進める長さの上限
    max_len: f64,
}

/// 面ごとの準備（光線によらない量）
struct Step {
    j: usize,
    f: Frame,
    mv: Move,
    /// 面の屈折率の表の値（GRIN の媒質の始まりの面は交点で求め直す。鏡・座標ブレークは前の値を引き継ぐ）
    n_tab: f64,
    /// この面の前後の屈折率がどの光線でも同じなら (前, 後)。GRIN の媒質が絡む面は None（光線ごと）
    n_uniform: Option<(f64, f64)>,
    kind: SurfKind,
    sag: Sag,
    /// (1 + k)（交点の二次方程式）と c (1 + k)（法線）
    k1: f64,
    ck1: f64,
    /// 理想レンズの物体側の共役距離
    conj: f64,
    /// この面に向かう空間の進む向き（[`trace::space_signs`]）
    sig: f64,
    /// この面の前が GRIN の媒質
    grin: Option<GrinIn>,
    /// この面の後ろから GRIN の媒質（屈折率は交点で求める）
    grin_eval: bool,
    /// Fresnel の基準点（[`Frames::fres_ref`]）
    fres_ref: [f64; 2],
}

/// 面の後ろの屈折率が前の値を引き継ぐか（座標ブレーク、鏡。理想レンズ・利用者定義の面の鏡は表の値）
pub(crate) fn takes_prev(d: &Surface) -> bool {
    d.is_coord_break() || d.mirror && !d.action().is_ideal()
}

/// 面 1..=last の追跡の準備（波長 w）。光線によらない量を先に求めておく
pub struct Plan<'a> {
    sys: &'a System,
    lam: f64,
    /// 面 1 の前の屈折率
    n_first: f64,
    /// Floating 開口でけるか（開口は追跡のたびに面から読む。自動の半直径は changed() なしで変わる）
    floating_clip: bool,
    steps: Arc<[Step]>,
}

/// 系に持つ追跡の準備（(波長, 最後の面, Floating 開口でけるか), 面ごとの準備）の並び
#[derive(Default)]
pub(crate) struct PlanCache(Mutex<Vec<(PlanKey, Arc<[Step]>)>>);

/// (波長, 最後の面, Floating 開口でけるか)
type PlanKey = (usize, usize, bool);

impl std::fmt::Debug for PlanCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlanCache({})", self.0.lock_ok().len())
    }
}

impl<'a> Plan<'a> {
    /// 面 1 から last まで（物体面は通らない）。fr が系の座標系なら、準備を系に持って使い回す
    /// （系を変えると changed() でキャッシュごと外れる）
    pub fn new(
        sys: &'a System,
        fr: &Frames,
        w: usize,
        last: usize,
        floating_clip: bool,
    ) -> Plan<'a> {
        let key = (w, last, floating_clip);
        let shared = std::ptr::eq(fr, sys.frames());
        let cached = shared
            .then(|| {
                let c = sys.plan_cache().0.lock_ok();
                c.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
            })
            .flatten();
        let steps = cached.unwrap_or_else(|| {
            let v: Arc<[Step]> = Self::steps(sys, fr, w, last).into();
            if shared {
                sys.plan_cache().0.lock_ok().push((key, v.clone()));
            }
            v
        });
        Plan {
            sys,
            lam: sys.wavelengths[w],
            n_first: sys.index_before(1, w),
            floating_clip,
            steps,
        }
    }

    fn steps(sys: &System, fr: &Frames, w: usize, last: usize) -> Vec<Step> {
        let mut steps: Vec<Step> = Vec::new();
        let sigs = sys.space_signs();
        let kinds = sys.surf_kinds();
        let mut nprev = sys.index_before(1, w);
        // 前の面の後ろの媒質の屈折率が光線ごとに変わるか（GRIN）
        let mut nprev_var = false;
        // GRIN の媒質の始まりの面（座標ブレークを挟んで続く）
        let mut grin_from: Option<usize> = None;
        // 直前の面
        let mut prev: Option<usize> = None;
        for j in 1..=last {
            let d = &sys.surfaces[j];
            if d.ignore {
                continue;
            }
            // 座標系の無い面（無限遠の物体面）は通らない
            let Some(f) = fr.surf[j] else { continue };
            let grin = grin_from.take().map(|gj| {
                let fg = fr.surf[gj].expect("frame of a traced surface");
                let rr = mm(&transpose(&f.r), &fg.r);
                let tt = mtv(&f.r, sub(fg.o, f.o));
                let pj = prev.unwrap_or(gj);
                let back = (pj != gj).then(|| {
                    let fp = fr.surf[pj].expect("frame of a traced surface");
                    (mm(&transpose(&fg.r), &fp.r), mtv(&fg.r, sub(fp.o, fg.o)))
                });
                GrinIn {
                    gj,
                    rr,
                    tt,
                    back,
                    max_len: 100.0 + 100.0 * sys.surfaces[gj].t.abs(),
                }
            });
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
            let grin_eval = d.grin.is_some() && !d.mirror;
            // GRIN の媒質の中の面は前の屈折率が光線ごと（交点の値）
            let n_var_before = nprev_var || grin.is_some();
            let n_tab = d.n[w];
            let nn = if takes_prev(d) { nprev } else { n_tab };
            let n_var_after = grin_eval || (n_var_before && takes_prev(d));
            steps.push(Step {
                j,
                f,
                mv,
                n_tab,
                n_uniform: (!n_var_before && !n_var_after).then_some((nprev, nn)),
                kind: kinds[j],
                sag: Sag::new(d),
                k1: 1.0 + d.k,
                ck1: d.c * (1.0 + d.k),
                sig: sigs[j],
                conj: if kinds[j].act == Act::Paraxial {
                    sys.lens_conjugate(j)
                } else {
                    0.0
                },
                grin,
                grin_eval,
                fres_ref: fr.fres_ref.get(j).copied().unwrap_or([0.0; 2]),
            });
            // 座標ブレークの後ろも同じ GRIN の媒質
            if let Some(g) = steps.last().expect("at least one step").grin {
                if d.is_coord_break() {
                    grin_from = Some(g.gj);
                }
            }
            if grin_eval {
                grin_from = Some(j);
            }
            nprev = nn;
            nprev_var = n_var_after;
            prev = Some(j);
        }
        steps
    }

    /// 追う面が 1 つもない
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// 面ごとの記録を光線ごとに受け取る（lane は組の中の番号、k は無視しない面の通し番号）
pub(crate) trait LaneSink {
    fn put(&mut self, lane: usize, k: usize, e: RecEntry);
    /// 無視しない面の通し番号 k の記録を受け取るか（受け取らない面は記録の値を作らない。既定は全面）
    #[inline(always)]
    fn wants(&self, _k: usize) -> bool {
        true
    }
}

/// 記録しない
pub(crate) struct NoRec;
impl LaneSink for NoRec {
    #[inline(always)]
    fn put(&mut self, _: usize, _: usize, _: RecEntry) {}
}

/// 1 本の追跡の記録を [`RecSink`] へ
struct OneSink<'r, R: RecSink + ?Sized>(&'r mut R);
impl<R: RecSink + ?Sized> LaneSink for OneSink<'_, R> {
    #[inline(always)]
    fn put(&mut self, _: usize, k: usize, e: RecEntry) {
        self.0.put(k, e);
    }
    #[inline(always)]
    fn wants(&self, k: usize) -> bool {
        self.0.wants(k)
    }
}

/// 1 本の光線（大域の位置 p・方向 k、光路長 0）を plan のとおりに追跡する。rec には無視しない面ごとに
/// 1 つずつ（面 1..=last の順に）書く
pub(crate) fn trace_one<R: RecSink + ?Sized>(
    plan: &Plan,
    p: V3,
    k: V3,
    rec: Option<&mut R>,
) -> RayOut {
    let mut sink = rec.map(OneSink);
    kernel::run::<lanes::One, 1, _>(plan, &[p], &[k], sink.as_mut(), None)[0]
}

/// [`trace_one`] の記録なし
pub(crate) fn trace_one_plain(plan: &Plan, p: V3, k: V3) -> RayOut {
    trace_one::<[RecEntry]>(plan, p, k, None)
}

/// [`trace_one`] に偏光を加えたもの（電場 pol.e を面ごとに更新する。pol は 1 本分）
pub(crate) fn trace_one_pol<R: RecSink + ?Sized>(
    plan: &Plan,
    p: V3,
    k: V3,
    rec: Option<&mut R>,
    pol: &mut PolLanes<1>,
) -> RayOut {
    let mut sink = rec.map(OneSink);
    kernel::run::<lanes::One, 1, _>(plan, &[p], &[k], sink.as_mut(), Some(pol))[0]
}

/// L 本の光線（大域の位置 p・方向 k、光路長 0）を plan のとおりに追跡する。
/// 結果は光線ごとに [`trace_one`] と同じビット
pub(crate) fn trace_lanes<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
) -> [RayOut; L] {
    dispatch(plan, p, k, rec, None)
}

/// [`trace_lanes`] に偏光を加えたもの（電場 pol.e を面ごとに更新する）。結果は光線ごとに
/// [`trace_one_pol`] と同じビット
pub(crate) fn trace_lanes_pol<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: &mut PolLanes<L>,
) -> [RayOut; L] {
    dispatch(plan, p, k, rec, Some(pol))
}

#[inline(always)]
fn dispatch<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: Option<&mut PolLanes<L>>,
) -> [RayOut; L] {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx") {
            // 安全: AVX があることを上で確かめた
            return unsafe { run_avx(plan, p, k, rec, pol) };
        }
    }
    kernel::run::<lanes::Wide, L, _>(plan, p, k, rec, pol)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn run_avx<R: LaneSink>(
    plan: &Plan,
    p: &[V3; L],
    k: &[V3; L],
    rec: Option<&mut R>,
    pol: Option<&mut PolLanes<L>>,
) -> [RayOut; L] {
    kernel::run::<lanes::Wide, L, _>(plan, p, k, rec, pol)
}
