//! 光線の始め方・追跡の結果・記録の型

use super::raybuf::ColOut;
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
    pub e: PolField,
}

/// 光線ごと・入力の偏光ごとの電場を 1 本の配列に並べたもの（光線 i の偏光 m は `e[i][m]`）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolField {
    /// 光線あたりの入力の偏光の数
    np: usize,
    /// 光線の本数
    n: usize,
    data: Vec<CV3>,
}

impl PolField {
    /// 光線あたり np 個の電場を光線の順に並べた data から
    pub fn from_flat(np: usize, data: Vec<CV3>) -> Self {
        let n = data.len().checked_div(np).unwrap_or(0);
        assert_eq!(
            n * np,
            data.len(),
            "the field length must be a multiple of the polarizations"
        );
        PolField { np, n, data }
    }
    /// 光線の本数
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// 光線あたりの入力の偏光の数
    pub fn pols(&self) -> usize {
        self.np
    }
    /// 光線ごとの電場（入力の偏光の順）
    pub fn iter(&self) -> PolFieldIter<'_> {
        PolFieldIter { f: self, i: 0 }
    }
    /// 全部の電場（光線の順、光線ごとに入力の偏光の順）
    pub fn as_flat(&self) -> &[CV3] {
        &self.data
    }
    pub fn into_flat(self) -> Vec<CV3> {
        self.data
    }
}

/// [`PolField::iter`] の反復子
#[derive(Debug, Clone)]
pub struct PolFieldIter<'a> {
    f: &'a PolField,
    i: usize,
}

impl<'a> Iterator for PolFieldIter<'a> {
    type Item = &'a [CV3];
    fn next(&mut self) -> Option<&'a [CV3]> {
        let f = self.f;
        (self.i < f.n).then(|| {
            self.i += 1;
            &f.data[(self.i - 1) * f.np..self.i * f.np]
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let r = self.f.n - self.i;
        (r, Some(r))
    }
}

impl ExactSizeIterator for PolFieldIter<'_> {}

impl<'a> IntoIterator for &'a PolField {
    type Item = &'a [CV3];
    type IntoIter = PolFieldIter<'a>;
    fn into_iter(self) -> PolFieldIter<'a> {
        self.iter()
    }
}

impl std::ops::Index<usize> for PolField {
    type Output = [CV3];
    fn index(&self, i: usize) -> &[CV3] {
        assert!(i < self.n, "ray {i} out of {}", self.n);
        &self.data[i * self.np..(i + 1) * self.np]
    }
}

impl Trace {
    pub fn ok(&self, i: usize) -> bool {
        self.rays[i].status.is_ok() && self.rays[i].vig == 0
    }
    /// 届いた・止まった理由ごとの本数
    pub fn counts(&self) -> crate::trace::RayCounts {
        let mut c = crate::trace::RayCounts::default();
        for r in &self.rays {
            c.add(r.status, r.vig);
        }
        c
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

/// 記録の配列（面ごと）の書き口。追跡の並列化で、光線 i は各配列の i 番目だけに書く（[`ColOut`]）
pub(super) struct SoaPtrs<'a> {
    /// 記録の番号（無視しない面の順）→ 出力の面の番号（記録しない面は usize::MAX）
    slot: Vec<usize>,
    xyz: Vec<ColOut<'a, V3>>,
    lmn: Vec<ColOut<'a, V3>>,
    opl: Vec<ColOut<'a, f64>>,
    p: Vec<ColOut<'a, V3>>,
    pub(super) n: usize,
}

impl<'a> SoaPtrs<'a> {
    pub(super) fn new(r: &'a mut [RecSurf], slot: Vec<usize>, n: usize) -> SoaPtrs<'a> {
        let (mut xyz, mut lmn, mut opl, mut p) = (vec![], vec![], vec![], vec![]);
        for x in r {
            xyz.push(ColOut::over(&mut x.xyz));
            lmn.push(ColOut::over(&mut x.lmn));
            opl.push(ColOut::over(&mut x.opl));
            p.push(ColOut::over(&mut x.p));
        }
        SoaPtrs {
            slot,
            xyz,
            lmn,
            opl,
            p,
            n,
        }
    }
}

pub(super) struct SoaRow<'a> {
    pub(super) out: &'a SoaPtrs<'a>,
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
        // 光線 i は SoaRow を 1 つだけ持ち、i 番目には他の光線が書かない
        o.xyz[k].put(self.i, e.xyz);
        o.lmn[k].put(self.i, e.lmn);
        o.opl[k].put(self.i, e.opl);
        o.p[k].put(self.i, e.p);
    }
}

/// まとめて追う光線の組の記録（組の l 番目は光線 base + l。端の組の余りは書かない）
pub(super) struct SoaLanes<'a> {
    pub(super) out: &'a SoaPtrs<'a>,
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
