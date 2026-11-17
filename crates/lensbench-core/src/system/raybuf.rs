//! 追跡の結果を列（SoA）で受ける再利用の入れ物
//!
//! [`System::try_trace_into`]・[`System::trace_launch_into`] は要る列（[`Cols`]）だけを書く。入れ物は呼び出しをまたいで使い回せる（確保は増えたときだけ）。
//! 値は [`System::trace_launch`] の [`Trace`] と同じビット。

use super::*;

/// 書く列（ビットの組）。状態（status）とけられ（vig）はいつも書く
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cols(u16);

impl Cols {
    /// 最後の面の局所座標の位置
    pub const XYZ: Cols = Cols(1);
    /// 最後の面の局所座標の方向
    pub const LMN: Cols = Cols(1 << 1);
    /// 幾何的な光路長
    pub const OPL: Cols = Cols(1 << 2);
    /// 大域の位置・方向
    pub const PK: Cols = Cols(1 << 3);
    /// 開始位置・方向（大域）
    pub const START: Cols = Cols(1 << 4);
    /// OpticStudio の生の光路長（[`Trace::opl_os`]）
    pub const OPL_OS: Cols = Cols(1 << 5);
    /// 理想レンズの位相
    pub const LENS_OPD: Cols = Cols(1 << 6);
    /// 全部
    pub const ALL: Cols = Cols((1 << 7) - 1);
    /// スポット（位置）
    pub const SPOT: Cols = Cols::XYZ;
    /// 波面（位置・方向・光路長・理想レンズの位相）
    pub const WAVE: Cols = Cols(1 | 1 << 1 | 1 << 2 | 1 << 6);

    pub const fn empty() -> Cols {
        Cols(0)
    }
    pub const fn has(self, c: Cols) -> bool {
        self.0 & c.0 == c.0
    }
}

impl std::ops::BitOr for Cols {
    type Output = Cols;
    fn bitor(self, o: Cols) -> Cols {
        Cols(self.0 | o.0)
    }
}

/// 追跡の結果の列。書かなかった列は空
#[derive(Debug, Clone, Default)]
pub struct RayBuf {
    /// 書く列（[`RayBuf::with_cols`] で決め、後からは変えない）
    cols: Cols,
    pub status: Vec<RayStatus>,
    /// けられた面（0 = けられていない）
    pub vig: Vec<u32>,
    pub xyz: Vec<V3>,
    pub lmn: Vec<V3>,
    pub opl: Vec<f64>,
    pub p: Vec<V3>,
    pub k: Vec<V3>,
    pub p0: Vec<V3>,
    pub k0: Vec<V3>,
    pub opl_os: Vec<f64>,
    pub lens_opd: Vec<f64>,
    /// (-1)^(通過した鏡の数)
    pub parity: f64,
    pub last: usize,
    pub hx_hy: (f64, f64),
}

impl RayBuf {
    pub fn new() -> RayBuf {
        RayBuf::default()
    }
    /// 列 cols を書く空の入れ物
    pub fn with_cols(cols: Cols) -> RayBuf {
        RayBuf {
            cols,
            ..Default::default()
        }
    }
    /// 書く列
    pub fn cols(&self) -> Cols {
        self.cols
    }
    /// 光線の数
    pub fn len(&self) -> usize {
        self.status.len()
    }
    pub fn is_empty(&self) -> bool {
        self.status.is_empty()
    }
    /// 光線 i が最後の面まで届き、けられていない（[`Trace::ok`] と同じ）
    pub fn ok(&self, i: usize) -> bool {
        self.status[i].is_ok() && self.vig[i] == 0
    }
    /// 届いた・止まった理由ごとの本数
    pub fn counts(&self) -> crate::trace::RayCounts {
        let mut c = crate::trace::RayCounts::default();
        for (st, &v) in self.status.iter().zip(&self.vig) {
            c.add(*st, v as usize);
        }
        c
    }
}

impl RayBuf {
    /// [`Trace`] から列を写す（`self.cols` の列）
    pub fn fill_from(&mut self, t: &Trace) {
        let c = self.cols;
        let r = &t.rays;
        self.status = r.iter().map(|x| x.status).collect();
        self.vig = r.iter().map(|x| x.vig as u32).collect();
        let pick = |on: bool, f: &dyn Fn(&RayOut) -> V3| -> Vec<V3> {
            if on {
                r.iter().map(f).collect()
            } else {
                Vec::new()
            }
        };
        let pickf = |on: bool, f: &dyn Fn(&RayOut) -> f64| -> Vec<f64> {
            if on {
                r.iter().map(f).collect()
            } else {
                Vec::new()
            }
        };
        self.xyz = pick(c.has(Cols::XYZ), &|x| x.xyz);
        self.lmn = pick(c.has(Cols::LMN), &|x| x.lmn);
        self.opl = pickf(c.has(Cols::OPL), &|x| x.opl);
        self.p = pick(c.has(Cols::PK), &|x| x.p);
        self.k = pick(c.has(Cols::PK), &|x| x.k);
        self.lens_opd = pickf(c.has(Cols::LENS_OPD), &|x| x.lens_opd);
        let st = c.has(Cols::START);
        self.p0 = if st { t.p0.clone() } else { Vec::new() };
        self.k0 = if st { t.k0.clone() } else { Vec::new() };
        self.opl_os = if c.has(Cols::OPL_OS) {
            t.opl_os.clone()
        } else {
            Vec::new()
        };
        self.parity = t.parity;
        self.last = t.last;
        self.hx_hy = t.hx_hy;
    }
}

/// [`System::trace_launch_core`] の結果のうち列でないもの
pub(super) struct Done {
    pub(super) parity: f64,
    pub(super) rec: Vec<RecSurf>,
    pub(super) last: usize,
    pub(super) hx_hy: (f64, f64),
}

/// 列の書き口: 光線 i は i 番目だけに書く（追跡の並列化で、各添字は 1 つのスレッドだけが書く）。生のポインタへの
/// 書き込みはここの 1 か所だけで、添字の範囲はいつも確かめる。使わない列（[`ColOut::off`]）には書かない
pub(super) struct ColOut<T: Copy> {
    p: *mut T,
    n: usize,
}
// 安全: 書く位置は添字ごとに別（各添字は 1 つのスレッドだけが書く）で、列は書き終えるまで動かさない・読まない
unsafe impl<T: Copy + Send> Sync for ColOut<T> {}
unsafe impl<T: Copy + Send> Send for ColOut<T> {}

impl<T: Copy> ColOut<T> {
    pub(super) fn off() -> Self {
        ColOut {
            p: std::ptr::null_mut(),
            n: 0,
        }
    }

    /// v を空にして n 個ぶん確保した領域（中身は未初期化）。v の長さは書き終えてから決める（[`Dst::finish`]）。
    /// on でなければ v を空にするだけで書かない
    fn spare(v: &mut Vec<T>, on: bool, n: usize) -> Self {
        v.clear();
        if !on {
            return ColOut::off();
        }
        v.reserve(n);
        ColOut {
            p: v.spare_capacity_mut()[..n].as_mut_ptr() as *mut T,
            n,
        }
    }

    /// v の今の中身に上書きする
    pub(super) fn over(v: &mut [T]) -> Self {
        ColOut {
            p: v.as_mut_ptr(),
            n: v.len(),
        }
    }

    #[inline(always)]
    pub(super) fn on(&self) -> bool {
        !self.p.is_null()
    }

    #[inline(always)]
    pub(super) fn put(&self, i: usize, v: T) {
        if self.p.is_null() {
            return;
        }
        assert!(i < self.n);
        // 安全: i < n は上で確かめ、領域は n 個ぶんある。i 番目は 1 つのスレッドだけが書く（型の約束）
        unsafe { self.p.add(i).write(v) }
    }
}

/// 光線ごとの結果の書き先（書かない列は [`ColOut::off`]）。光線 i は各列の i 番目だけに書く
pub(super) struct OutPtrs {
    ray: ColOut<RayOut>,
    status: ColOut<RayStatus>,
    vig: ColOut<u32>,
    xyz: ColOut<V3>,
    lmn: ColOut<V3>,
    opl: ColOut<f64>,
    p: ColOut<V3>,
    k: ColOut<V3>,
    p0: ColOut<V3>,
    k0: ColOut<V3>,
    opl_os: ColOut<f64>,
    lens_opd: ColOut<f64>,
    /// 無限遠物体の opl_os の基準（面 1 の原点・軸・その前の屈折率）
    os: Option<(V3, V3, f64)>,
}

impl OutPtrs {
    #[inline(always)]
    pub(super) fn put(&self, i: usize, r: &RayOut, p0: V3, k0: V3) {
        self.ray.put(i, *r);
        self.status.put(i, r.status);
        self.vig.put(i, r.vig as u32);
        self.xyz.put(i, r.xyz);
        self.lmn.put(i, r.lmn);
        self.opl.put(i, r.opl);
        self.p.put(i, r.p);
        self.k.put(i, r.k);
        self.p0.put(i, p0);
        self.k0.put(i, k0);
        self.lens_opd.put(i, r.lens_opd);
        if self.opl_os.on() {
            let v = match self.os {
                Some((o, ax, nb)) => r.opl - dot(sub(o, p0), ax) / dot(k0, ax) * nb,
                None => r.opl,
            };
            self.opl_os.put(i, v);
        }
    }
}

/// 結果の行き先: [`Trace`]（光線ごとの [`RayOut`]）か、列の入れ物
pub(super) enum Dst<'a> {
    Trace {
        rays: Vec<RayOut>,
        p0: Vec<V3>,
        k0: Vec<V3>,
        opl_os: Vec<f64>,
    },
    Buf(&'a mut RayBuf),
}

impl Dst<'_> {
    pub(super) fn trace() -> Dst<'static> {
        Dst::Trace {
            rays: Vec::new(),
            p0: Vec::new(),
            k0: Vec::new(),
            opl_os: Vec::new(),
        }
    }

    /// n 本ぶんの書き先を用意する
    pub(super) fn prepare(&mut self, n: usize, os: Option<(V3, V3, f64)>) -> OutPtrs {
        match self {
            Dst::Trace {
                rays,
                p0,
                k0,
                opl_os,
            } => OutPtrs {
                ray: ColOut::spare(rays, true, n),
                status: ColOut::off(),
                vig: ColOut::off(),
                xyz: ColOut::off(),
                lmn: ColOut::off(),
                opl: ColOut::off(),
                p: ColOut::off(),
                k: ColOut::off(),
                p0: ColOut::spare(p0, true, n),
                k0: ColOut::spare(k0, true, n),
                opl_os: ColOut::spare(opl_os, true, n),
                lens_opd: ColOut::off(),
                os,
            },
            Dst::Buf(b) => {
                let c = b.cols;
                OutPtrs {
                    ray: ColOut::off(),
                    status: ColOut::spare(&mut b.status, true, n),
                    vig: ColOut::spare(&mut b.vig, true, n),
                    xyz: ColOut::spare(&mut b.xyz, c.has(Cols::XYZ), n),
                    lmn: ColOut::spare(&mut b.lmn, c.has(Cols::LMN), n),
                    opl: ColOut::spare(&mut b.opl, c.has(Cols::OPL), n),
                    p: ColOut::spare(&mut b.p, c.has(Cols::PK), n),
                    k: ColOut::spare(&mut b.k, c.has(Cols::PK), n),
                    p0: ColOut::spare(&mut b.p0, c.has(Cols::START), n),
                    k0: ColOut::spare(&mut b.k0, c.has(Cols::START), n),
                    opl_os: ColOut::spare(&mut b.opl_os, c.has(Cols::OPL_OS), n),
                    lens_opd: ColOut::spare(&mut b.lens_opd, c.has(Cols::LENS_OPD), n),
                    os,
                }
            }
        }
    }

    /// 0..n の全部を書いたあとで長さを決める
    ///
    /// 安全: [`Self::prepare`] の後に、0..n のすべての i について [`OutPtrs::put`] を呼んだこと（未初期化のまま長さに
    /// 入れる所はここだけ）
    pub(super) unsafe fn finish(&mut self, n: usize) {
        unsafe fn set<T>(v: &mut Vec<T>, n: usize) {
            debug_assert!(v.capacity() >= n);
            // 安全: 呼び出し側の約束（0..n を書いた）
            unsafe { v.set_len(n) };
        }
        // 安全: 呼び出し側の約束
        unsafe {
            match self {
                Dst::Trace {
                    rays,
                    p0,
                    k0,
                    opl_os,
                } => {
                    set(rays, n);
                    set(p0, n);
                    set(k0, n);
                    set(opl_os, n);
                }
                Dst::Buf(b) => {
                    let c = b.cols;
                    set(&mut b.status, n);
                    set(&mut b.vig, n);
                    let cond = |on: bool| if on { n } else { 0 };
                    set(&mut b.xyz, cond(c.has(Cols::XYZ)));
                    set(&mut b.lmn, cond(c.has(Cols::LMN)));
                    set(&mut b.opl, cond(c.has(Cols::OPL)));
                    set(&mut b.p, cond(c.has(Cols::PK)));
                    set(&mut b.k, cond(c.has(Cols::PK)));
                    set(&mut b.p0, cond(c.has(Cols::START)));
                    set(&mut b.k0, cond(c.has(Cols::START)));
                    set(&mut b.opl_os, cond(c.has(Cols::OPL_OS)));
                    set(&mut b.lens_opd, cond(c.has(Cols::LENS_OPD)));
                }
            }
        }
    }
}
