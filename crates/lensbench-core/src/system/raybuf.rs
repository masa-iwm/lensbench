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

/// 光線ごとの結果の書き先（列の先頭。書かない列は空）。光線 i は各列の i 番目だけに書く
pub(super) struct OutPtrs {
    n: usize,
    ray: *mut RayOut,
    status: *mut RayStatus,
    vig: *mut u32,
    xyz: *mut V3,
    lmn: *mut V3,
    opl: *mut f64,
    p: *mut V3,
    k: *mut V3,
    p0: *mut V3,
    k0: *mut V3,
    opl_os: *mut f64,
    lens_opd: *mut f64,
    /// 無限遠物体の opl_os の基準（面 1 の原点・軸・その前の屈折率）
    os: Option<(V3, V3, f64)>,
}
// 安全: 書く位置は光線ごとに別で、列は追跡の間は動かさない（長さは書き終えてから決める）
unsafe impl Sync for OutPtrs {}

fn spare<T>(v: &mut Vec<T>, on: bool, n: usize) -> *mut T {
    if !on {
        v.clear();
        return std::ptr::null_mut();
    }
    v.clear();
    v.reserve(n);
    v.spare_capacity_mut()[..n].as_mut_ptr() as *mut T
}

impl OutPtrs {
    #[inline(always)]
    pub(super) fn put(&self, i: usize, r: &RayOut, p0: V3, k0: V3) {
        debug_assert!(i < self.n);
        // 安全: i < n（呼び出し側の約束）で、各列は n 個ぶん確保してある
        unsafe {
            let w = |q: *mut V3, v: V3| {
                if !q.is_null() {
                    q.add(i).write(v)
                }
            };
            let wf = |q: *mut f64, v: f64| {
                if !q.is_null() {
                    q.add(i).write(v)
                }
            };
            if !self.ray.is_null() {
                self.ray.add(i).write(*r);
            }
            if !self.status.is_null() {
                self.status.add(i).write(r.status);
                self.vig.add(i).write(r.vig as u32);
            }
            w(self.xyz, r.xyz);
            w(self.lmn, r.lmn);
            wf(self.opl, r.opl);
            w(self.p, r.p);
            w(self.k, r.k);
            w(self.p0, p0);
            w(self.k0, k0);
            wf(self.lens_opd, r.lens_opd);
            if !self.opl_os.is_null() {
                let v = match self.os {
                    Some((o, ax, nb)) => r.opl - dot(sub(o, p0), ax) / dot(k0, ax) * nb,
                    None => r.opl,
                };
                self.opl_os.add(i).write(v);
            }
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
                n,
                ray: spare(rays, true, n),
                status: std::ptr::null_mut(),
                vig: std::ptr::null_mut(),
                xyz: std::ptr::null_mut(),
                lmn: std::ptr::null_mut(),
                opl: std::ptr::null_mut(),
                p: std::ptr::null_mut(),
                k: std::ptr::null_mut(),
                p0: spare(p0, true, n),
                k0: spare(k0, true, n),
                opl_os: spare(opl_os, true, n),
                lens_opd: std::ptr::null_mut(),
                os,
            },
            Dst::Buf(b) => {
                let c = b.cols;
                OutPtrs {
                    n,
                    ray: std::ptr::null_mut(),
                    status: spare(&mut b.status, true, n),
                    vig: spare(&mut b.vig, true, n),
                    xyz: spare(&mut b.xyz, c.has(Cols::XYZ), n),
                    lmn: spare(&mut b.lmn, c.has(Cols::LMN), n),
                    opl: spare(&mut b.opl, c.has(Cols::OPL), n),
                    p: spare(&mut b.p, c.has(Cols::PK), n),
                    k: spare(&mut b.k, c.has(Cols::PK), n),
                    p0: spare(&mut b.p0, c.has(Cols::START), n),
                    k0: spare(&mut b.k0, c.has(Cols::START), n),
                    opl_os: spare(&mut b.opl_os, c.has(Cols::OPL_OS), n),
                    lens_opd: spare(&mut b.lens_opd, c.has(Cols::LENS_OPD), n),
                    os,
                }
            }
        }
    }

    /// 0..n の全部を書いたあとで長さを決める
    ///
    /// 安全: [`Self::prepare`] の後に、0..n のすべての i について [`OutPtrs::put`] を呼んだこと
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
