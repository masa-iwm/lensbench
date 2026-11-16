//! 光線の始め方・追跡の結果・記録の型

use super::*;

/// 光線の始め方
#[derive(Debug, Clone, Copy)]
pub enum Launch<'a> {
    /// 物体と正規化瞳座標（ケラレ係数 vig、None なら視野 hxy のもの（[`System::vig_factors`]）。エイミングは系の設定どおり）。
    /// signed なら、入射瞳が物体より前（ENPP + 物体距離 < 0）の有限物体系で瞳座標の符号を反転して与える
    /// （px = 1 が絞りの同じ側を通り、入射瞳が無限遠をまたいでも向きが連続。既定の false は OpticStudio と同じ）
    Pupil {
        spec: ObjSpec,
        signed: bool,
        px: &'a [f64],
        py: &'a [f64],
        vig: Option<[f64; 5]>,
        hxy: Option<(f64, f64)>,
    },
    /// 物体と、絞り面の局所座標の点 (x, y) [mm]。その点を通る実光線を光線ごとに解く
    /// （系のエイミングの設定・入射瞳の位置によらない）。解けない光線は絞りで止まった扱い
    Stop {
        spec: ObjSpec,
        x: &'a [f64],
        y: &'a [f64],
    },
    /// 開始位置と方向（大域座標。方向は規格化する）。光路長は開始位置から測る
    Rays { p: &'a [V3], k: &'a [V3] },
}

/// 視野の指定: 定義済み視野の番号か、正規化視野 (hx, hy)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldSel {
    Index(usize),
    Norm(f64, f64),
}
impl From<usize> for FieldSel {
    fn from(i: usize) -> Self {
        FieldSel::Index(i)
    }
}
impl From<(f64, f64)> for FieldSel {
    fn from(h: (f64, f64)) -> Self {
        FieldSel::Norm(h.0, h.1)
    }
}

/// 光線の束の追跡結果
#[derive(Debug, Clone)]
pub struct Trace {
    pub rays: Vec<RayOut>,
    /// (-1)^(通過した鏡の数)
    pub parity: f64,
    /// 開始位置・方向（大域）
    pub p0: Vec<V3>,
    pub k0: Vec<V3>,
    /// OpticStudio の生の光路長（無限遠物体では面 1 の物体側座標系の z = 0 面から）
    pub opl_os: Vec<f64>,
    /// 面ごとの記録（record のとき）。無視しない面 1..=last（[`RecSel::Surfs`] ならそのうち指定の面）の順
    pub rec: Vec<RecSurf>,
    pub last: usize,
    pub hx_hy: (f64, f64),
}

/// 偏光追跡の結果
#[derive(Debug, Clone)]
pub struct PolTrace {
    pub trace: Trace,
    /// 光線ごと・入力の偏光ごとの電場（最後の面の局所座標、既定は伝搬の位相込み）。追跡が止まった光線は NaN
    pub e: Vec<Vec<CV3>>,
}

impl Trace {
    pub fn ok(&self, i: usize) -> bool {
        self.rays[i].status.is_ok() && self.rays[i].vig == 0
    }
    /// 開口を無視して像面まで届いたか（けられても追跡は続く）。主光線などの基準の光線はこちらで見る
    /// （OpticStudio と同じく、基準の光線は開口・遮蔽を無視する。中心遮蔽の系など）
    pub fn traced(&self, i: usize) -> bool {
        self.rays[i].status.is_ok()
    }
    /// 面 j の記録（無視する面と、record でない追跡では None）
    pub fn rec_at(&self, j: usize) -> Option<&RecSurf> {
        self.rec
            .binary_search_by_key(&j, |r| r.surf)
            .ok()
            .map(|k| &self.rec[k])
    }
}

/// [`System::trace_spec`] の選択（既定: ケラレ係数なし・像面まで・記録なし・視野の正規化座標なし）
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceOpts {
    /// ケラレ係数（VDX, VDY, VCX, VCY, VAN）
    pub vig: Option<[f64; 5]>,
    /// 止める面（None は像面）
    pub upto: Option<usize>,
    /// 面ごとの記録を残す
    pub record: bool,
    /// 視野の正規化座標（浮動開口・ケラレの補間に使う）
    pub hxy: Option<(f64, f64)>,
}

impl TraceOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn vig(self, v: [f64; 5]) -> Self {
        self.vig_opt(Some(v))
    }
    pub fn vig_opt(mut self, v: Option<[f64; 5]>) -> Self {
        self.vig = v;
        self
    }
    pub fn upto(self, j: usize) -> Self {
        self.upto_opt(Some(j))
    }
    pub fn upto_opt(mut self, j: Option<usize>) -> Self {
        self.upto = j;
        self
    }
    pub fn record(self) -> Self {
        self.record_if(true)
    }
    pub fn record_if(mut self, b: bool) -> Self {
        self.record = b;
        self
    }
    pub fn hxy(self, h: (f64, f64)) -> Self {
        self.hxy_opt(Some(h))
    }
    pub fn hxy_opt(mut self, h: Option<(f64, f64)>) -> Self {
        self.hxy = h;
        self
    }
}

/// 面ごとの記録（record）をどの面について残すか
#[derive(Debug, Clone, Copy)]
pub enum RecSel<'a> {
    /// 記録しない
    Off,
    /// 無視しない面 1..=last のすべて
    All,
    /// 指定した面だけ（面の番号。無視する面・last より後の面は除かれる）
    Surfs(&'a [usize]),
}

impl From<bool> for RecSel<'_> {
    fn from(b: bool) -> Self {
        if b {
            RecSel::All
        } else {
            RecSel::Off
        }
    }
}

/// 記録の配列（面ごと）の先頭。追跡の並列化で、光線 i は各配列の i 番目だけに書く
pub(super) struct SoaPtrs {
    /// 記録の番号（無視しない面の順）→ 出力の面の番号（記録しない面は usize::MAX）
    pub(super) slot: Vec<usize>,
    pub(super) xyz: Vec<*mut V3>,
    pub(super) lmn: Vec<*mut V3>,
    pub(super) opl: Vec<*mut f64>,
    pub(super) p: Vec<*mut V3>,
    pub(super) n: usize,
}
// 安全: 書く位置は光線ごとに別（SoaRow::put を参照）で、配列は追跡の間は動かさない
unsafe impl Sync for SoaPtrs {}

impl SoaPtrs {
    pub(super) fn new(r: &mut [RecSurf], slot: Vec<usize>, n: usize) -> SoaPtrs {
        SoaPtrs {
            slot,
            xyz: r.iter_mut().map(|x| x.xyz.as_mut_ptr()).collect(),
            lmn: r.iter_mut().map(|x| x.lmn.as_mut_ptr()).collect(),
            opl: r.iter_mut().map(|x| x.opl.as_mut_ptr()).collect(),
            p: r.iter_mut().map(|x| x.p.as_mut_ptr()).collect(),
            n,
        }
    }
}

pub(super) struct SoaRow<'a> {
    pub(super) out: &'a SoaPtrs,
    pub(super) i: usize,
}

impl trace::RecSink for SoaRow<'_> {
    #[inline(always)]
    fn wants(&self, k: usize) -> bool {
        self.out.slot.get(k).is_some_and(|&s| s != usize::MAX)
    }
    #[inline(always)]
    fn put(&mut self, k: usize, e: RecEntry) {
        let o = self.out;
        let k = match o.slot.get(k) {
            Some(&s) if s != usize::MAX => s,
            _ => return,
        };
        assert!(k < o.xyz.len() && self.i < o.n);
        // 安全: 範囲は上で確かめた。光線 i は SoaRow を 1 つだけ持ち、i 番目には他の光線が書かない
        unsafe {
            *o.xyz[k].add(self.i) = e.xyz;
            *o.lmn[k].add(self.i) = e.lmn;
            *o.opl[k].add(self.i) = e.opl;
            *o.p[k].add(self.i) = e.p;
        }
    }
}

/// まとめて追う光線の組の記録（組の l 番目は光線 base + l。端の組の余りは書かない）
pub(super) struct SoaLanes<'a> {
    pub(super) out: &'a SoaPtrs,
    pub(super) base: usize,
}

impl batch::LaneSink for SoaLanes<'_> {
    #[inline(always)]
    fn wants(&self, k: usize) -> bool {
        self.out.slot.get(k).is_some_and(|&s| s != usize::MAX)
    }
    #[inline(always)]
    fn put(&mut self, lane: usize, k: usize, e: RecEntry) {
        let i = self.base + lane;
        if i < self.out.n {
            trace::RecSink::put(&mut SoaRow { out: self.out, i }, k, e);
        }
    }
}

/// 1 つの面での全光線の記録（量ごとに光線の順に並べる）。止まった光線は NaN
#[derive(Debug, Clone)]
pub struct RecSurf {
    pub surf: usize,
    /// (-1)^(この面までに通った鏡の数)
    pub parity: f64,
    /// 面の局所座標での交点
    pub xyz: Vec<V3>,
    /// 面の後の方向（局所座標、物理的な向き）
    pub lmn: Vec<V3>,
    /// 面までの光路長
    pub opl: Vec<f64>,
    /// 大域の交点
    pub p: Vec<V3>,
}

impl RecSurf {
    pub fn entry(&self, i: usize) -> RecEntry {
        RecEntry {
            surf: self.surf,
            xyz: self.xyz[i],
            lmn: self.lmn[i],
            opl: self.opl[i],
            p: self.p[i],
        }
    }
}
