//! 評価関数の項（[`Term`]・[`TermKind`]）と、項の量の種類
use super::*;
use crate::pupil::quad_len;

/// MTF の向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MtfDir {
    Tan,
    Sag,
    /// 子午・球欠の 2 つの残差
    Both,
}

/// スポット・波面の項の瞳のガウス求積の点の置き方（r² に Gauss-Legendre の rings 本の輪 × 等間隔の腕）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Quad {
    /// 腕 4 rings + 2 本（[`lensbench_core::analysis::pupil_gauss`]。少ない rings でも角度の求積が収束に近い）
    #[default]
    Lensbench,
    /// OpticStudio 26.1 の RSCE・RSCH・RWCE・RWCH と同じ: 腕 2 rings 本を 90° 回して置く
    /// （[`lensbench_core::analysis::pupil_gauss_os`]。.zmx から読んだ行の項）
    OpticStudio,
    /// 腕を与えた数だけ 0° から等間隔に置く（腕の角度 a·360°/arms）。OpticStudio の評価関数の Wizard の
    /// ガウス求積（GQ）と同じ
    Arms(usize),
    /// 輪を使わず、瞳を n × n の正方の格子に分けた升の中心のうち瞳の中の点を等しい重みで（OpticStudio の評価関数の
    /// Wizard の Rectangular Array と同じ）
    Grid(usize),
}

/// 厚みの項で面ごとに比べるとき（OpticStudio の範囲の境界 MNCT・MXCT・MNCA・MNEG など）: medium の面だけを数え、
/// 下限なら残差 −Σmax(0, target − x_i)、上限（upper）なら Σmax(0, x_i − target)。値（残差 + target）は OpticStudio の Value と同じ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Each {
    pub medium: Medium,
    pub upper: bool,
}

/// 厚みの境界が数える面の後ろの媒質
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Medium {
    /// すべての面（MNCT など）
    #[default]
    All,
    /// 空気（自分の媒質のない座標ブレーク・鏡も。MNCA など）
    Air,
    /// 空気でない自分の媒質のある面（ガラスなど。MNCG など）
    Glass,
}

/// スポットの項が測る向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpotAxis {
    /// x と y（スポットの半径）
    #[default]
    Both,
    /// x だけ
    X,
    /// y だけ
    Y,
}

/// 項の種類
#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    /// RMS スポット半径 [レンズ単位]。wave None なら重みのある全波長。quad は瞳の求積の点の置き方、axis は測る向き
    Spot {
        field: FieldSel,
        wave: Option<usize>,
        rings: usize,
        reference: RmsRef,
        quad: Quad,
        axis: SpotAxis,
    },
    /// RMS 波面収差 [波]。wave None なら重みのある全波長（波長ごとの分散を波長の重みで平均）
    Wavefront {
        field: FieldSel,
        wave: Option<usize>,
        rings: usize,
        reference: RmsRef,
        quad: Quad,
    },
    /// 周波数 freq [cycles/mm] の MTF − target（n は瞳の標本数、[`lensbench_core::analysis::mtf`]）
    Mtf {
        field: FieldSel,
        wave: Option<usize>,
        n: Option<usize>,
        freq: f64,
        dir: MtfDir,
        target: f64,
    },
    /// 近軸量 − target（主波長の近軸追跡。[`System::paraxial`]）。ズーム軌跡の焦点距離・倍率の条件
    Paraxial { q: ParaxQ, target: f64 },
    /// 像面の光束の重心の x（`y` が false）か y − target [レンズ単位]。光線・重みはスポットの項と同じ（ガウス求積、
    /// rings、wave None なら重みのある全波長）。けられた光線は除く。鏡で像点を合わせる補償など
    Centroid {
        field: FieldSel,
        wave: Option<usize>,
        rings: usize,
        y: bool,
        target: f64,
    },
    /// 1 本の実光線の量 − target。視野 field・正規化瞳座標 (px, py)・波長 wave（None なら主波長）の光線を
    /// 面 surf（None なら像面）まで追い、その面の局所座標での値（[`RayQ`]）。光線が止まると評価できない
    Ray {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        surf: Option<usize>,
        q: RayQ,
        target: f64,
    },
    /// 1 本の光線の OPD [波] − target（[`lensbench_core::analysis::opd`]、参照球の中心は主波長の主光線。OpticStudio の OPDC）
    Opd {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        target: f64,
    },
    /// Zernike 係数 [波] − target（[`lensbench_core::analysis::zernike`]、自波長の主光線基準、格子 n × n）。term は 1 始まり。
    /// fit は当てはめる項の数（term 以上。None は term と 37 の大きい方）
    Zernike {
        field: FieldSel,
        wave: Option<usize>,
        kind: ZernKind,
        term: usize,
        fit: Option<usize>,
        n: usize,
        target: f64,
    },
    /// 単一モードファイバへの結合効率（偏光なし）− target（[`lensbench_core::fiber::fiber_coupling`]、有限物体の系）
    Coupling {
        wave: Option<usize>,
        opts: FiberOpts,
        target: f64,
    },
    /// 面 surf から to までの中心の厚みの和 [レンズ単位] − target（surf == to なら 1 面の厚み。OpticStudio の CTVA・TTHI）。
    /// edge が Some(z) なら縁の厚み（面 i と i + 1 の矢高を、それぞれの半直径 × z の高さで。OpticStudio の ETVA）。
    /// each なら面ごとの厚みを target と比べたはみ出しの和（[`Each`]）。面の番号は System の面の番号（物体面 0）
    Thickness {
        surf: usize,
        to: usize,
        target: f64,
        edge: Option<f64>,
        each: Option<Each>,
    },
    /// 利用者の項（残差はそのまま。目標との差は利用者の関数の中でとる）
    User(UserTermRef),
}

/// 光線の量（[`TermKind::Ray`]）。面の局所座標
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RayQ {
    X,
    Y,
    Z,
    /// 方向余弦
    L,
    M,
    N,
}

impl RayQ {
    pub fn parse(s: &str) -> Result<RayQ> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "x" | "reax" => RayQ::X,
            "y" | "reay" => RayQ::Y,
            "z" | "reaz" => RayQ::Z,
            "l" | "reaa" => RayQ::L,
            "m" | "reab" => RayQ::M,
            "n" | "reac" => RayQ::N,
            _ => invalid!("ray quantity {s} (x, y, z, l, m, n)"),
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            RayQ::X => "x",
            RayQ::Y => "y",
            RayQ::Z => "z",
            RayQ::L => "l",
            RayQ::M => "m",
            RayQ::N => "n",
        }
    }
}

/// 近軸量など系全体の量（[`TermKind::Paraxial`]）。値は [`lensbench_core::report::system_data`]（OpticStudio の
/// System Data と同じ定義、名前は OpticStudio の演算子に合わせる）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParaxQ {
    /// 有効焦点距離 [レンズ単位]（EFFL）
    Efl,
    /// 近軸倍率（有限物体、PMAG）
    Pmag,
    /// 入射瞳の位置（ENPP）
    Enpp,
    /// 入射瞳径（EPDI）
    Epd,
    /// 射出瞳の位置（EXPP）
    Expp,
    /// 射出瞳径（EXPD）
    Expd,
    /// 実効 F ナンバー（実光線、WFNO）
    Wfno,
    /// 近軸の実効 F ナンバー（PWFN）
    Pwfn,
    /// 像空間 F ナンバー（ISFN）
    Isfn,
    /// 近軸像高（PIMH）
    Pimh,
    /// 全長（面 1 から像面の手前の面まで、TOTR）
    Totr,
    /// 物体空間の NA（OBSN）
    Obsn,
    /// 像空間の NA（ISNA）
    Isna,
}

impl ParaxQ {
    pub const ALL: [ParaxQ; 13] = [
        ParaxQ::Efl,
        ParaxQ::Pmag,
        ParaxQ::Enpp,
        ParaxQ::Epd,
        ParaxQ::Expp,
        ParaxQ::Expd,
        ParaxQ::Wfno,
        ParaxQ::Pwfn,
        ParaxQ::Isfn,
        ParaxQ::Pimh,
        ParaxQ::Totr,
        ParaxQ::Obsn,
        ParaxQ::Isna,
    ];
    pub fn parse(s: &str) -> Result<ParaxQ> {
        let t = s.trim().to_ascii_lowercase();
        Ok(match t.as_str() {
            "effl" => ParaxQ::Efl,
            "mag" => ParaxQ::Pmag,
            "epdi" => ParaxQ::Epd,
            _ => match ParaxQ::ALL.iter().find(|q| q.name() == t) {
                Some(q) => *q,
                None => invalid!("paraxial {s} ({})", ParaxQ::ALL.map(|q| q.name()).join(",")),
            },
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            ParaxQ::Efl => "efl",
            ParaxQ::Pmag => "pmag",
            ParaxQ::Enpp => "enpp",
            ParaxQ::Epd => "epd",
            ParaxQ::Expp => "expp",
            ParaxQ::Expd => "expd",
            ParaxQ::Wfno => "wfno",
            ParaxQ::Pwfn => "pwfn",
            ParaxQ::Isfn => "isfn",
            ParaxQ::Pimh => "pimh",
            ParaxQ::Totr => "totr",
            ParaxQ::Obsn => "obsn",
            ParaxQ::Isna => "isna",
        }
    }
    /// 系の値
    pub fn value(&self, s: &System) -> f64 {
        let p = s.paraxial();
        match self {
            ParaxQ::Efl => return p.efl,
            ParaxQ::Pmag => return p.pmag,
            ParaxQ::Enpp => return p.enpp,
            ParaxQ::Epd => return p.epd,
            ParaxQ::Expp => return p.expp,
            ParaxQ::Pwfn => return p.wfno,
            ParaxQ::Isfn => return p.isfn,
            _ => {}
        }
        let d = lensbench_core::report::system_data(s);
        match self {
            ParaxQ::Expd => d.expd,
            ParaxQ::Wfno => d.wfno,
            ParaxQ::Pimh => d.pimh,
            ParaxQ::Totr => d.totr,
            ParaxQ::Obsn => d.obsn,
            ParaxQ::Isna => d.isna,
            _ => unreachable!(),
        }
    }
}

/// 利用者が定義する項（OpticStudio の UDOP 相当）。Python の関数などを包む
pub trait UserTerm: Send + Sync {
    /// 名前（表示用）
    fn name(&self) -> &str;
    /// 残差の数（系によらず一定）
    fn rows(&self) -> usize;
    /// 系の並びの残差をまとめて求める（系ごとに rows 個）。差分のヤコビアンでは、ずらした系をすべて 1 回で渡す
    fn eval(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>>;
    /// 残差の偏微分（行優先 [残差][params]）。None なら差分にする
    fn grad(&self, _s: &System, _params: &[Param]) -> Result<Option<Vec<f64>>> {
        Ok(None)
    }
}

/// [`UserTerm`] の共有の参照（同じ物を指すとき等しい）
#[derive(Clone)]
pub struct UserTermRef(pub Arc<dyn UserTerm>);

impl std::fmt::Debug for UserTermRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UserTerm({}, {} 行)", self.0.name(), self.0.rows())
    }
}

impl PartialEq for UserTermRef {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl UserTermRef {
    /// 系の並びを評価し、行数を確かめる
    pub(crate) fn eval_checked(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        let v = self.0.eval(systems)?;
        if v.len() != systems.len() {
            invalid!(
                "user term {}: {} results for {} systems",
                self.0.name(),
                systems.len(),
                v.len()
            );
        }
        for r in &v {
            if r.len() != self.0.rows() {
                invalid!(
                    "user term {}: {} residuals (expected {})",
                    self.0.name(),
                    r.len(),
                    self.0.rows()
                );
            }
            if r.iter().any(|x| !x.is_finite()) {
                invalid!("user term {}: non-finite residual", self.0.name());
            }
        }
        Ok(v)
    }
}

impl TermKind {
    /// 1 つの値と目標の差を残差にする項の目標
    pub(crate) fn target(&self) -> Option<f64> {
        match self {
            TermKind::Mtf { dir, target, .. } if *dir != MtfDir::Both => Some(*target),
            TermKind::Paraxial { target, .. }
            | TermKind::Ray { target, .. }
            | TermKind::Centroid { target, .. }
            | TermKind::Opd { target, .. }
            | TermKind::Zernike { target, .. }
            | TermKind::Coupling { target, .. }
            | TermKind::Thickness { target, .. } => Some(*target),
            _ => None,
        }
    }
}

/// 目標の向き（目標のある項だけ。[`TermKind`] の target）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    /// 値 = 目標（残差は 値 − 目標）
    #[default]
    Eq,
    /// 値 ≥ 目標（下回ったときだけ 値 − 目標、ほかは 0）
    Min,
    /// 値 ≤ 目標（上回ったときだけ 値 − 目標、ほかは 0）
    Max,
}

impl Side {
    /// 片側の条件を満たしていれば残差（とその行のヤコビアン）を 0 にする
    pub(crate) fn active(self, r: f64) -> bool {
        match self {
            Side::Eq => true,
            Side::Min => r < 0.0,
            Side::Max => r > 0.0,
        }
    }
}

/// 評価関数の項
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    /// 構成の番号（[`Problem::configs`]）
    pub config: usize,
    pub weight: f64,
    pub kind: TermKind,
    /// 目標の向き（[`Side::Min`]・[`Side::Max`] は目標のある項だけ）
    pub side: Side,
}

impl Term {
    /// 片側の条件の項で、条件を満たしている残差を 0 にする（j はその項の行のヤコビアン）
    pub(crate) fn clamp(&self, r: &mut [f64], j: Option<&mut Vec<f64>>) {
        if self.side == Side::Eq || r.len() != 1 || self.side.active(r[0]) {
            return;
        }
        r[0] = 0.0;
        if let Some(j) = j {
            j.iter_mut().for_each(|v| *v = 0.0);
        }
    }
}

/// 項の値（[`Problem::term_values`]）。値の意味は項の種類で違う
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TermValue {
    /// 目標のある項（MTF・近軸量・光線・OPD・Zernike・結合効率・重心）: 量そのもの
    Quantity(f64),
    /// 目標のない項（スポット・波面・利用者の項）: 残差の √Σr²（スポット・波面では RMS そのもの）
    Rms(f64),
}

impl TermValue {
    /// 数の値
    pub fn value(self) -> f64 {
        match self {
            TermValue::Quantity(v) | TermValue::Rms(v) => v,
        }
    }
    /// 出力用の種類の名前（`quantity`・`rms`）
    pub fn kind_name(self) -> &'static str {
        match self {
            TermValue::Quantity(_) => "quantity",
            TermValue::Rms(_) => "rms",
        }
    }
}

/// 項の光線の数（[`Problem::ray_counts`]）。波長ごとの瞳の光線を足したもの
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RayCounts {
    /// 追跡した光線
    pub total: usize,
    /// 開口でけられた光線（評価関数から除く）
    pub vignetted: usize,
    /// 追跡が止まった光線（全反射・面に届かない）
    pub stopped: usize,
}

impl Problem {
    /// 瞳の光線を並べる項（スポット・波面・重心）の光線の数（けられた・止まった光線。ほかの項は None）。
    /// 評価関数は光線がけられる・戻るたびに跳ぶので、開口の縁で最適化が振れるときの目安に
    pub fn ray_counts(&self, x: &[f64]) -> Result<Vec<Option<RayCounts>>> {
        let sys = self.systems(x)?;
        map_maybe_par(&self.terms, |t| {
            let (field, wave, rings, quad) = match &t.kind {
                TermKind::Spot {
                    field,
                    wave,
                    rings,
                    quad,
                    ..
                }
                | TermKind::Wavefront {
                    field,
                    wave,
                    rings,
                    quad,
                    ..
                } => (*field, *wave, *rings, *quad),
                TermKind::Centroid {
                    field, wave, rings, ..
                } => (*field, *wave, *rings, Quad::Lensbench),
                _ => return Ok(None),
            };
            let s = &sys[t.config];
            let pg = crate::pupil::apodized_gauss(s, field, rings, quad, false)?.0;
            let mut c = RayCounts::default();
            for w in waves(s, wave) {
                let tr = s.try_trace(field, &pg.0, &pg.1, Some(w), None, false)?;
                c.total += tr.rays.len();
                c.stopped += tr.rays.iter().filter(|r| r.status.surf().is_some()).count();
                c.vignetted += tr
                    .rays
                    .iter()
                    .filter(|r| r.status.is_ok() && r.vig != 0)
                    .count();
            }
            Ok(Some(c))
        })
    }
}

/// 項の残差（重みを掛ける前）から項の値
pub(super) fn term_value(k: &TermKind, r: &[f64]) -> TermValue {
    match k.target() {
        Some(target) => TermValue::Quantity(r[0] + target),
        None => TermValue::Rms(r.iter().map(|v| v * v).sum::<f64>().sqrt()),
    }
}

/// 項の残差の数
pub(super) fn term_rows(s: &System, k: &TermKind) -> usize {
    match k {
        TermKind::Spot {
            wave, rings, quad, ..
        } => 2 * waves(s, *wave).len() * quad_len(*rings, *quad),
        TermKind::Wavefront {
            wave, rings, quad, ..
        } => waves(s, *wave).len() * quad_len(*rings, *quad),
        TermKind::Mtf { dir, .. } => {
            if *dir == MtfDir::Both {
                2
            } else {
                1
            }
        }
        TermKind::Paraxial { .. }
        | TermKind::Ray { .. }
        | TermKind::Centroid { .. }
        | TermKind::Opd { .. }
        | TermKind::Zernike { .. }
        | TermKind::Coupling { .. }
        | TermKind::Thickness { .. } => 1,
        TermKind::User(u) => u.0.rows(),
    }
}
