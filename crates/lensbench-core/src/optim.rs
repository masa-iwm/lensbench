//! 減衰最小二乗（DLS、Levenberg–Marquardt）による最適化。
//!
//! 変数は系のパラメータ（[`crate::param`] の [`Param`]: 間隔・傾き・偏心・曲率・玉や群の動きなど）。
//! 評価関数は残差の二乗和 Σ r²。項（[`Term`]）ごとに残差の並びを作り、重みの平方根を掛ける。
//! 評価関数の値（[`Problem::merit`]）は √(Σ r² / Σ 重み)。既定の項（スポット・波面の RMS）では
//! 項ごとの Σ r² が 重み × RMS² になるので、視野の重みで平均した RMS になる（OpticStudio の既定の評価関数と同じ形）。
//!
//! 項:
//! - [`TermKind::Spot`]: RMS スポット半径。光線ごとの (x − 基準, y − 基準) を残差にする（ガウス求積、
//!   [`crate::sweep::rms`] と同じ光線・重み・基準）
//! - [`TermKind::Wavefront`]: RMS 波面収差 [波]。光線ごとの OPD から、ピストン（重心基準なら方向余弦の差に比例する項も）を
//!   除いたもの（[`crate::sweep::rms`] と同じ定義）
//! - [`TermKind::Mtf`]: 1 つの周波数の MTF と目標の差
//! - [`TermKind::Paraxial`]: 近軸量（焦点距離・倍率・瞳・F ナンバーなど、[`ParaxQ`]）と目標の差
//! - [`TermKind::Centroid`]: 瞳の光線の像面の位置の重心（x か y）と目標の差（光線はスポットの項と同じ、解析的な微分）
//! - [`TermKind::Ray`]: 1 本の実光線の、面の局所座標での位置・方向余弦と目標の差（OpticStudio の REAX〜REAC 相当）
//! - [`TermKind::Opd`]: 1 本の光線の OPD [波] と目標の差（参照球は主波長の主光線、OpticStudio の OPDC 相当）
//! - [`TermKind::Zernike`]: OPD の Zernike 係数 [波] と目標の差（[`crate::analysis::zernike`]）
//! - [`TermKind::Coupling`]（単一モードファイバへの結合効率と目標の差、[`crate::fiber::fiber_coupling`]）
//! - [`TermKind::User`]: 利用者が定義する項（[`UserTerm`]。Python の関数など）。系の並びをまとめて評価する
//!
//! 項は JSON でも書ける（`lensbench_project::merit`、CLI の `--term`・Python の `terms=`）。
//!
//! 光線の項の残差は、けられた光線を 0 にする（RMS から除くのと同じ）。追跡が止まった光線（全反射・面に届かない）が
//! あると評価できない（エラー。最適化ではその一歩を捨てて減衰を強める）。
//!
//! ヤコビアン（[`Problem::jacobian`]）は、スポット・波面・光線・OPD では解析的な微分を使う: 着地点の微分
//! [`System::land_grad`]と光路長の微分 [`System::opl_grad`]。開始の光線を固定した微分なので、
//! 変数で光線の始め方（物体側の開始位置・方向。入射瞳・エイミング・視野の定義で決まる）が変わる列は、
//! 中心差分にする（変数ごとに、少しずらした系の開始の光線を比べて判定する）。波面の参照球の半径（射出瞳の位置）と
//! 像空間の屈折率の変化は、近軸の量の差分で足す。像面が平面でない系の波面・OPD と、光線の z、MTF・近軸量・
//! Zernike・結合効率は差分にする。利用者の項は、微分を返す関数（[`UserTerm::grad`]）があればそれを使い、
//! なければ差分の系をまとめて 1 回で評価する（[`UserTerm::eval`]）。
//! 差分は変数ごとに並列に、項は項ごとに並列に計算する（[`crate::par::with_threads`] に従う）。
//!
//! 境界（[`Var::lo`]・[`Var::hi`]）は、一歩ごとに変数を範囲に切り詰め、境界にあって外へ向かう変数はその一歩では動かさない。
//!
//! 複数の構成（[`Problem::configs`]、多重構成・ズームの各状態）を同時に扱える。変数ごとに効く構成を選べる
//! （[`Var::configs`]。既定はすべて）。カタログの差し替え探索・ズーム軌跡・モンテカルロは
//! この [`dls`] を繰り返し呼ぶ形で使う。状態を持たないので、一括実行の各タスクの中から呼んでよい。

use crate::analysis::{mtf_only, pupil_gauss_shared, to_ref, MtfMethod, ZernKind};
use crate::error::Result;
use crate::fiber::FiberOpts;
use crate::invalid;
use crate::math::*;
use crate::param::{Kind, Param};
use crate::surface::SurfType;
use crate::sweep::RmsRef;
use crate::system::{FieldSel, System, Trace};
use rayon::prelude::*;
use std::sync::Arc;

/// 最適化の変数
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub param: Param,
    /// 効く構成の番号（None ならすべて）
    pub configs: Option<Vec<usize>>,
    /// 範囲（既定は ±∞）
    pub lo: f64,
    pub hi: f64,
}

impl Var {
    pub fn new(param: Param) -> Var {
        Var {
            param,
            configs: None,
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
        }
    }
    pub fn bounds(mut self, lo: f64, hi: f64) -> Var {
        self.lo = lo;
        self.hi = hi;
        self
    }
    pub fn in_configs(mut self, c: Vec<usize>) -> Var {
        self.configs = Some(c);
        self
    }
    fn applies(&self, c: usize) -> bool {
        self.configs.as_ref().map_or(true, |v| v.contains(&c))
    }
}

/// MTF の向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MtfDir {
    Tan,
    Sag,
    /// 子午・球欠の 2 つの残差
    Both,
}

/// 項の種類
#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    /// RMS スポット半径 [レンズ単位]。wave None なら重みのある全波長
    Spot {
        field: FieldSel,
        wave: Option<usize>,
        rings: usize,
        reference: RmsRef,
    },
    /// RMS 波面収差 [波]。wave None なら重みのある全波長（波長ごとの分散を波長の重みで平均）
    Wavefront {
        field: FieldSel,
        wave: Option<usize>,
        rings: usize,
        reference: RmsRef,
    },
    /// 周波数 freq [cycles/mm] の MTF − target（n は瞳の標本数、[`crate::analysis::mtf`]）
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
    /// 1 本の光線の OPD [波] − target（[`crate::analysis::opd`]、参照球の中心は主波長の主光線。OpticStudio の OPDC）
    Opd {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        target: f64,
    },
    /// Zernike 係数 [波] − target（[`crate::analysis::zernike`]、自波長の主光線基準、格子 n × n）。term は 1 始まり
    Zernike {
        field: FieldSel,
        wave: Option<usize>,
        kind: ZernKind,
        term: usize,
        n: usize,
        target: f64,
    },
    /// 単一モードファイバへの結合効率（偏光なし）− target（[`crate::fiber::fiber_coupling`]、有限物体の系）
    Coupling {
        wave: Option<usize>,
        opts: FiberOpts,
        target: f64,
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
            _ => invalid!("光線の量 {s}（x・y・z・l・m・n）"),
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

/// 近軸量など系全体の量（[`TermKind::Paraxial`]）。値は [`crate::report::system_data`]（OpticStudio の
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
    /// 物体空間の NA（OBNA）
    Obna,
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
        ParaxQ::Obna,
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
                None => invalid!("近軸量 {s}（{}）", ParaxQ::ALL.map(|q| q.name()).join("・")),
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
            ParaxQ::Obna => "obna",
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
        let d = crate::report::system_data(s);
        match self {
            ParaxQ::Expd => d.expd,
            ParaxQ::Wfno => d.wfno,
            ParaxQ::Pimh => d.pimh,
            ParaxQ::Totr => d.totr,
            ParaxQ::Obna => d.obna,
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
    fn eval_checked(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        let v = self.0.eval(systems)?;
        if v.len() != systems.len() {
            invalid!(
                "利用者の項 {}: 系 {} 個に結果 {} 個",
                self.0.name(),
                systems.len(),
                v.len()
            );
        }
        for r in &v {
            if r.len() != self.0.rows() {
                invalid!(
                    "利用者の項 {}: 残差 {} 個（{} 個のはず）",
                    self.0.name(),
                    r.len(),
                    self.0.rows()
                );
            }
            if r.iter().any(|x| !x.is_finite()) {
                invalid!("利用者の項 {}: 有限でない残差", self.0.name());
            }
        }
        Ok(v)
    }
}

impl TermKind {
    /// 1 つの値と目標の差を残差にする項の目標
    fn target(&self) -> Option<f64> {
        match self {
            TermKind::Mtf { dir, target, .. } if *dir != MtfDir::Both => Some(*target),
            TermKind::Paraxial { target, .. }
            | TermKind::Ray { target, .. }
            | TermKind::Centroid { target, .. }
            | TermKind::Opd { target, .. }
            | TermKind::Zernike { target, .. }
            | TermKind::Coupling { target, .. } => Some(*target),
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
    fn active(self, r: f64) -> bool {
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
    fn clamp(&self, r: &mut [f64], j: Option<&mut Vec<f64>>) {
        if self.side == Side::Eq || r.len() != 1 || self.side.active(r[0]) {
            return;
        }
        r[0] = 0.0;
        if let Some(j) = j {
            j.iter_mut().for_each(|v| *v = 0.0);
        }
    }
}

/// 差分の +h の系と -h の系（前進差分では -h はなし）
/// 差分の +h・−h の系（境界の外に出る・作れない側は None）
type FdSystems = (Option<System>, Option<System>);

/// ヤコビアンの求め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JacMode {
    /// 解析的に求められる列は解析的に、ほかは中心差分。ただし項ごとに、解析的に求められる列が
    /// [`Problem::analytic_min`] 個より少なければ中心差分（解析的な微分は列の数によらない手間が大きく、
    /// 列が少ないと差分の方が速い）
    #[default]
    Auto,
    /// すべて中心差分
    FiniteDiff,
    /// すべて前進差分（中心差分の半分の追跡で済むが、誤差は刻みの 1 次）。射出瞳の揺れのため刻みを大きく取る
    /// 波面の項（回転対称でない系など）は中心差分のまま
    Forward,
}

impl JacMode {
    /// 名前（"auto"・"fd"・"forward"、大文字小文字は問わない）から
    pub fn parse(s: &str) -> Option<JacMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(JacMode::Auto),
            "fd" => Some(JacMode::FiniteDiff),
            "forward" => Some(JacMode::Forward),
            _ => None,
        }
    }

    /// 名前
    pub fn name(self) -> &'static str {
        match self {
            JacMode::Auto => "auto",
            JacMode::FiniteDiff => "fd",
            JacMode::Forward => "forward",
        }
    }

    /// 選べる名前（エラーの表示用）
    pub const NAMES: &'static str = "auto・fd・forward";
}

/// 既定の評価関数の種類（[`Problem::add_default`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultMerit {
    Spot,
    Wavefront,
}

/// 最適化の問題
#[derive(Debug, Clone)]
pub struct Problem {
    /// 構成（多重構成・ズームの各状態）。1 つなら単一の系
    pub configs: Vec<System>,
    pub vars: Vec<Var>,
    pub terms: Vec<Term>,
    pub jac: JacMode,
    /// 差分の刻みの相対値（刻み = rel × max(|値|, 種類ごとの目安)）
    pub fd_rel: f64,
    /// 回転対称でない系の波面の項の差分の刻みの相対値。射出瞳を実光線の外挿で求める（OpticStudio と同じ）ので
    /// 評価関数に 1e-8 程度の揺れがあり、小さい刻みでは差分が揺れに負ける
    pub fd_rel_noisy: f64,
    /// [`JacMode::Auto`] で解析的に求める列の最少の数（項ごと。これより少なければ中心差分）。
    /// 既定 16 は、11 面の系のスポットで解析的と中心差分の時間が並ぶ列の数。0 なら常に解析的
    pub analytic_min: usize,
}

/// 項の残差（重みを掛ける前）と、解析的に求めた列のヤコビアン（行優先 [行][params]）
type TermOut = (Vec<f64>, Option<Vec<f64>>);

impl Problem {
    pub fn new(configs: Vec<System>) -> Problem {
        Problem {
            configs,
            vars: Vec::new(),
            terms: Vec::new(),
            jac: JacMode::Auto,
            fd_rel: 1e-6,
            fd_rel_noisy: 1e-3,
            analytic_min: 16,
        }
    }

    /// 単一の系
    pub fn single(s: System) -> Problem {
        Problem::new(vec![s])
    }

    /// 文字列で変数を足す（[`System::param`] の書き方。最初の構成で解釈する）
    pub fn add_var(&mut self, spec: &str) -> Result<&mut Var> {
        let p = self.configs.first().map(|s| s.param(spec));
        let Some(p) = p else {
            invalid!("構成がない");
        };
        self.vars.push(Var::new(p?));
        Ok(self.vars.last_mut().unwrap())
    }

    /// 既定の評価関数: 構成ごと・定義済みの視野ごとに、重みのある全波長の RMS の項を足す（重みは視野の重み、
    /// 重み 0 の視野は除く）
    pub fn add_default(&mut self, kind: DefaultMerit, rings: usize, reference: RmsRef) {
        for (c, s) in self.configs.iter().enumerate() {
            for (i, f) in s.fields.iter().enumerate() {
                if !(f.w > 0.0) {
                    continue;
                }
                let field = FieldSel::Index(i);
                let k = match kind {
                    DefaultMerit::Spot => TermKind::Spot {
                        field,
                        wave: None,
                        rings,
                        reference,
                    },
                    DefaultMerit::Wavefront => TermKind::Wavefront {
                        field,
                        wave: None,
                        rings,
                        reference,
                    },
                };
                self.terms.push(Term {
                    side: Default::default(),
                    config: c,
                    weight: f.w,
                    kind: k,
                });
            }
        }
    }

    /// 問題の確かめ（構成・変数・項の番号と値）
    pub fn check(&self) -> Result<()> {
        if self.configs.is_empty() {
            invalid!("構成がない");
        }
        if self.terms.is_empty() {
            invalid!("評価関数の項がない");
        }
        for (q, v) in self.vars.iter().enumerate() {
            if v.param.kind.is_object() {
                invalid!("変数 {q}: 物点の移動は系のデータでない");
            }
            if v.param.kind.is_wave() {
                invalid!("変数 {q}: 波長は変数にできない（公差の項目だけ）");
            }
            if !(v.lo <= v.hi) {
                invalid!("変数 {q}: 範囲 [{}, {}]", v.lo, v.hi);
            }
            let mut any = false;
            for (c, s) in self.configs.iter().enumerate() {
                if v.applies(c) {
                    s.check_param(v.param)?;
                    any = true;
                }
            }
            if !any {
                invalid!("変数 {q}: 効く構成がない");
            }
            if self.vars[..q]
                .iter()
                .any(|u| u.param == v.param && overlap(u, v))
            {
                invalid!("変数 {q}: 同じパラメータが重なる構成で 2 回");
            }
        }
        for (i, t) in self.terms.iter().enumerate() {
            let Some(s) = self.configs.get(t.config) else {
                invalid!("項 {i}: 構成 {}（{} 個）", t.config, self.configs.len());
            };
            if !(t.weight >= 0.0) {
                invalid!("項 {i}: 重み {}", t.weight);
            }
            let (field, wave) = match &t.kind {
                TermKind::Spot {
                    field, wave, rings, ..
                }
                | TermKind::Wavefront {
                    field, wave, rings, ..
                }
                | TermKind::Centroid {
                    field, wave, rings, ..
                } => {
                    if *rings == 0 {
                        invalid!("項 {i}: リング数は 1 以上");
                    }
                    (*field, *wave)
                }
                TermKind::Mtf {
                    field, wave, freq, ..
                } => {
                    if !(*freq >= 0.0) {
                        invalid!("項 {i}: 周波数 {freq}");
                    }
                    (*field, *wave)
                }
                TermKind::Paraxial { q, target } => {
                    if !target.is_finite() {
                        invalid!("項 {i}: {} の目標 {target}", q.name());
                    }
                    if !q.value(s).is_finite() {
                        invalid!("項 {i}: 系の {} が有限でない", q.name());
                    }
                    continue;
                }
                TermKind::Ray {
                    field, wave, surf, ..
                } => {
                    if let Some(j) = surf {
                        if *j == 0 || *j > s.image() {
                            invalid!("項 {i}: 面 {j}（1〜{}）", s.image());
                        }
                    }
                    (*field, Some(wave.unwrap_or(s.primary)))
                }
                TermKind::Opd { field, wave, .. } => (*field, Some(wave.unwrap_or(s.primary))),
                TermKind::Zernike {
                    field,
                    wave,
                    kind,
                    term,
                    n,
                    ..
                } => {
                    let max = if *kind == ZernKind::Fringe { 37 } else { 2016 };
                    if *term == 0 || *term > max {
                        invalid!("項 {i}: Zernike の項 {term}（1〜{max}）");
                    }
                    if *n < 8 {
                        invalid!("項 {i}: Zernike の格子 {n}");
                    }
                    (*field, Some(wave.unwrap_or(s.primary)))
                }
                TermKind::Coupling { wave, .. } => {
                    if s.infinite() {
                        invalid!("項 {i}: ファイバ結合は有限物体の系だけ");
                    }
                    (FieldSel::Norm(0.0, 0.0), Some(wave.unwrap_or(s.primary)))
                }
                TermKind::User(u) => {
                    if u.0.rows() == 0 {
                        invalid!("項 {i}: 利用者の項 {} の残差が 0 個", u.0.name());
                    }
                    continue;
                }
            };
            if let Some(tg) = t.kind.target() {
                if !tg.is_finite() {
                    invalid!("項 {i}: 目標 {tg}");
                }
            } else if t.side != Side::Eq {
                invalid!("項 {i}: 片側の条件（min・max）は目標のある項だけ");
            }
            if let FieldSel::Index(f) = field {
                if f >= s.fields.len() {
                    invalid!("項 {i}: 視野 {f}（{} 個）", s.fields.len());
                }
            }
            if let Some(w) = wave {
                if w >= s.wavelengths.len() {
                    invalid!("項 {i}: 波長 {w}（{} 個）", s.wavelengths.len());
                }
            }
            if wave.is_none() && waves(s, None).is_empty() {
                invalid!("項 {i}: 重みのある波長がない");
            }
        }
        Ok(())
    }

    /// 変数の初期値（効く最初の構成の値）
    pub fn x0(&self) -> Result<Vec<f64>> {
        self.vars
            .iter()
            .map(|v| {
                let c = (0..self.configs.len()).find(|&c| v.applies(c)).unwrap_or(0);
                self.configs[c].get_param(v.param)
            })
            .collect()
    }

    /// 変数の値 x での各構成の系
    pub fn systems(&self, x: &[f64]) -> Result<Vec<System>> {
        (0..self.configs.len()).map(|c| self.system(c, x)).collect()
    }

    /// 結果として返す系（自動の半直径を求め直す）
    fn final_systems(&self, x: &[f64]) -> Result<Vec<System>> {
        let mut v = self.systems(x)?;
        v.iter_mut().for_each(System::update_semi);
        Ok(v)
    }

    fn system(&self, c: usize, x: &[f64]) -> Result<System> {
        if x.len() != self.vars.len() {
            invalid!("変数の値 {} 個（変数は {} 個）", x.len(), self.vars.len());
        }
        let ch: Vec<(Param, f64)> = self
            .vars
            .iter()
            .zip(x)
            .filter(|(v, _)| v.applies(c))
            // 今の値と同じものは変えない（傾き偏心のない面に偏心 0 を与えると、回転対称でない系の扱いになり
            // 射出瞳の求め方が変わるため。初期値の系を元の系と同じにする）
            .filter(|(v, &xv)| self.configs[c].get_param(v.param).ok() != Some(xv))
            .map(|(v, &xv)| (v.param, xv))
            .collect();
        if ch.is_empty() {
            Ok(self.configs[c].clone())
        } else {
            self.configs[c].with_params_fast(&ch)
        }
    }

    /// 項ごとの残差の行数（系の形と項で決まり、変数の値によらない）
    pub fn term_rows(&self) -> Vec<usize> {
        self.terms
            .iter()
            .map(|t| term_rows(&self.configs[t.config], &t.kind))
            .collect()
    }

    /// 残差の数
    pub fn n_residuals(&self) -> usize {
        self.term_rows().iter().sum()
    }

    fn weight_sum(&self) -> f64 {
        self.terms.iter().map(|t| t.weight).sum()
    }

    /// 残差（重みの平方根を掛けたもの、項の順に並べる）
    pub fn residuals(&self, x: &[f64]) -> Result<Vec<f64>> {
        let sys = self.systems(x)?;
        let parts = map_maybe_par(&self.terms, |t| {
            let mut r = eval_term(&sys[t.config], &t.kind, None)?.0;
            t.clamp(&mut r, None);
            let a = t.weight.sqrt();
            r.iter_mut().for_each(|v| *v *= a);
            Ok(r)
        })?;
        Ok(parts.concat())
    }

    /// 評価関数の値 √(Σ r² / Σ 重み)
    pub fn merit(&self, x: &[f64]) -> Result<f64> {
        let r = self.residuals(x)?;
        Ok(self.merit_of(&r))
    }

    /// 残差から評価関数の値
    pub fn merit_of(&self, r: &[f64]) -> f64 {
        let ws = self.weight_sum();
        let ss: f64 = r.iter().map(|v| v * v).sum();
        if ws > 0.0 {
            (ss / ws).sqrt()
        } else {
            ss.sqrt()
        }
    }

    /// 項ごとの評価関数の値（その項の √(Σ r² / 重み)。スポット・波面の項では RMS そのもの）
    pub fn term_values(&self, x: &[f64]) -> Result<Vec<f64>> {
        let sys = self.systems(x)?;
        map_maybe_par(&self.terms, |t| {
            let r = eval_term(&sys[t.config], &t.kind, None)?.0;
            let ss: f64 = r.iter().map(|v| v * v).sum();
            Ok(match t.kind.target() {
                Some(target) => r[0] + target,
                None => ss.sqrt(),
            })
        })
    }

    /// 残差とヤコビアン（行優先 [残差][変数]）
    pub fn jacobian(&self, x: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
        self.check()?;
        let nv = self.vars.len();
        if x.len() != nv {
            invalid!("変数の値 {} 個（変数は {nv} 個）", x.len());
        }
        let sys = self.systems(x)?;
        let rows = self.term_rows();
        let steps: Vec<f64> = self
            .vars
            .iter()
            .zip(x)
            .map(|(v, &xv)| self.fd_rel * xv.abs().max(typical(v.param.kind)))
            .collect();
        // 項ごと: 残差・解析的な列・差分が要る変数
        struct Part {
            r: Vec<f64>,
            j: Vec<f64>,
            fd: Vec<usize>,
        }
        let parts: Vec<Part> = map_maybe_par(&self.terms, |t| {
            let s = &sys[t.config];
            let act: Vec<usize> = (0..nv)
                .filter(|&q| self.vars[q].applies(t.config))
                .collect();
            if let TermKind::User(u) = &t.kind {
                let r = u.eval_checked(&[s])?.remove(0);
                let m = r.len();
                let mut j = vec![0.0; m * nv];
                let ps: Vec<Param> = act.iter().map(|&q| self.vars[q].param).collect();
                let g = if self.jac == JacMode::Auto && !ps.is_empty() {
                    u.0.grad(s, &ps)?
                } else {
                    None
                };
                let fd = match g {
                    Some(g) => {
                        if g.len() != m * ps.len() {
                            invalid!(
                                "利用者の項 {}: 微分 {} 個（{m}×{} のはず）",
                                u.0.name(),
                                g.len(),
                                ps.len()
                            );
                        }
                        for (k, &q) in act.iter().enumerate() {
                            for i in 0..m {
                                j[i * nv + q] = g[i * ps.len() + k];
                            }
                        }
                        Vec::new()
                    }
                    None => act,
                };
                return Ok(Part { r, j, fd });
            }
            let analytic: Vec<usize> = if self.jac == JacMode::Auto && supports_analytic(s, &t.kind)
            {
                act.iter()
                    .copied()
                    .filter(|&q| {
                        !launch_moves(s, &t.kind, self.vars[q].param, x[q], steps[q])
                            .unwrap_or(true)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let analytic = if analytic.len() < self.analytic_min {
                Vec::new()
            } else {
                analytic
            };
            let ps: Vec<Param> = analytic.iter().map(|&q| self.vars[q].param).collect();
            let (r, ja, ok) = if ps.is_empty() {
                (eval_term(s, &t.kind, None)?.0, None, false)
            } else {
                match eval_term(s, &t.kind, Some(&ps)) {
                    Ok((r, j)) => (r, j, true),
                    // 解析的な微分に対応しない面など: すべて差分に
                    Err(_) => (eval_term(s, &t.kind, None)?.0, None, false),
                }
            };
            let m = r.len();
            let mut j = vec![0.0; m * nv];
            let mut fd: Vec<usize> = act.clone();
            if ok {
                let ja = ja.unwrap();
                for (k, &q) in analytic.iter().enumerate() {
                    for i in 0..m {
                        j[i * nv + q] = ja[i * ps.len() + k];
                    }
                }
                fd.retain(|q| !analytic.contains(q));
            }
            Ok(Part { r, j, fd })
        })?;
        // 差分の列: 変数ごとに ±h の系を作り、その変数を差分で求める項をまとめて評価
        let fd_vars: Vec<usize> = (0..nv)
            .filter(|q| parts.iter().any(|p| p.fd.contains(q)))
            .collect();
        let forward = self.jac == JacMode::Forward;
        let cols: Vec<Vec<(usize, Vec<f64>)>> = map_maybe_par(&fd_vars, |&q| {
            let mut out = Vec::new();
            // (構成, 刻みの種類) ごとの +h と -h の系（前進差分では -h はなし）
            let mut cache: Vec<[Option<FdSystems>; 2]> = vec![[None, None]; self.configs.len()];
            for (ti, t) in self.terms.iter().enumerate() {
                if !parts[ti].fd.contains(&q) || matches!(t.kind, TermKind::User(_)) {
                    continue;
                }
                let noisy = noisy_fd(&sys[t.config], &t.kind, self.vars[q].param.kind);
                let h = steps[q]
                    * if noisy {
                        self.fd_rel_noisy / self.fd_rel
                    } else {
                        1.0
                    };
                let slot = &mut cache[t.config][noisy as usize];
                let (lo, hi) = (self.vars[q].lo, self.vars[q].hi);
                if slot.is_none() {
                    // 境界の外へは出さない（厚さの下限 0 で x − h が負になるなど）。出る側・系が作れない側は片側差分に
                    let up_ok = x[q] + h <= hi;
                    let dn_ok = x[q] - h >= lo;
                    let mk = |d: f64| {
                        let mut xs = x.to_vec();
                        xs[q] += d;
                        self.system(t.config, &xs)
                    };
                    let sp = if up_ok { mk(h).ok() } else { None };
                    // 刻みを大きく取る項（noisy）は前進差分だと誤差が大きいので中心差分のまま
                    let want_m = !(forward && !noisy && sp.is_some());
                    let sm = if want_m && dn_ok { mk(-h).ok() } else { None };
                    if sp.is_none() && sm.is_none() {
                        // どちらの側も作れない: 元の誤りを返す
                        mk(if up_ok { h } else { -h })?;
                        crate::invalid!("差分の系を作れない（変数 {}）", q + 1);
                    }
                    *slot = Some((sp, sm));
                }
                let (sp, sm) = slot.as_ref().unwrap();
                let rp = match sp {
                    Some(sp) => eval_term(sp, &t.kind, None).map(|v| v.0),
                    None => Err(crate::Error::Invalid(String::new())),
                };
                let rm = match sm {
                    Some(sm) => eval_term(sm, &t.kind, None).map(|v| v.0),
                    None => Err(crate::Error::Invalid(String::new())),
                };
                let r0 = &parts[ti].r;
                let d: Vec<f64> = match (rp, rm) {
                    (Ok(rp), Ok(rm)) => rp
                        .iter()
                        .zip(&rm)
                        .map(|(a, b)| (a - b) / (2.0 * h))
                        .collect(),
                    // 片側だけ: 元の系の残差（parts の r、重みを掛ける前）との差
                    (Ok(rp), Err(_)) => rp.iter().zip(r0).map(|(a, b)| (a - b) / h).collect(),
                    (Err(_), Ok(rm)) => r0.iter().zip(&rm).map(|(a, b)| (a - b) / h).collect(),
                    (Err(e), Err(e2)) => {
                        return Err(if sp.is_some() { e } else { e2 });
                    }
                };
                out.push((ti, d));
            }
            Ok(out)
        })?;
        let mut parts = parts;
        for (k, &q) in fd_vars.iter().enumerate() {
            for (ti, d) in &cols[k] {
                for (i, v) in d.iter().enumerate() {
                    parts[*ti].j[i * nv + q] = *v;
                }
            }
        }
        // 利用者の項の差分: ずらした系をすべて作り、項ごとに 1 回でまとめて評価する
        for (ti, t) in self.terms.iter().enumerate() {
            let TermKind::User(u) = &t.kind else {
                continue;
            };
            let fd = parts[ti].fd.clone();
            if fd.is_empty() {
                continue;
            }
            if forward {
                let ps: Vec<System> = map_maybe_par(&fd, |&q| {
                    let mut xp = x.to_vec();
                    xp[q] += steps[q];
                    self.system(t.config, &xp)
                })?;
                let refs: Vec<&System> = ps.iter().collect();
                let v = u.eval_checked(&refs)?;
                for (k, &q) in fd.iter().enumerate() {
                    for i in 0..v[k].len() {
                        parts[ti].j[i * nv + q] = (v[k][i] - parts[ti].r[i]) / steps[q];
                    }
                }
                continue;
            }
            let pm: Vec<(System, System)> = map_maybe_par(&fd, |&q| {
                let mut xp = x.to_vec();
                let mut xm = x.to_vec();
                xp[q] += steps[q];
                xm[q] -= steps[q];
                Ok((self.system(t.config, &xp)?, self.system(t.config, &xm)?))
            })?;
            let refs: Vec<&System> = pm.iter().flat_map(|(a, b)| [a, b]).collect();
            let v = u.eval_checked(&refs)?;
            for (k, &q) in fd.iter().enumerate() {
                let (rp, rm) = (&v[2 * k], &v[2 * k + 1]);
                for i in 0..rp.len() {
                    parts[ti].j[i * nv + q] = (rp[i] - rm[i]) / (2.0 * steps[q]);
                }
            }
        }
        let mut r = Vec::with_capacity(rows.iter().sum());
        let mut j = Vec::with_capacity(r.capacity() * nv);
        for (mut p, t) in parts.into_iter().zip(&self.terms) {
            t.clamp(&mut p.r, Some(&mut p.j));
            let a = t.weight.sqrt();
            r.extend(p.r.iter().map(|v| v * a));
            j.extend(p.j.iter().map(|v| v * a));
        }
        Ok((r, j))
    }
}

fn overlap(a: &Var, b: &Var) -> bool {
    match (&a.configs, &b.configs) {
        (Some(x), Some(y)) => x.iter().any(|c| y.contains(c)),
        _ => true,
    }
}

/// 差分が射出瞳の揺れに負ける組（回転対称でない系か、対称を崩す変数での波面の項）
fn noisy_fd(s: &System, k: &TermKind, v: Kind) -> bool {
    matches!(
        k,
        TermKind::Wavefront { .. } | TermKind::Opd { .. } | TermKind::Zernike { .. }
    ) && (!s.symmetric()
        || matches!(v, Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz)
        || v.is_unit_motion())
}

/// 差分の刻みの目安（種類ごとの値の大きさ）
fn typical(k: Kind) -> f64 {
    match k {
        Kind::Curv => 1e-2,
        Kind::Radius => 10.0,
        Kind::Conic => 1.0,
        Kind::Asph(i) => 1e-3 / 10f64.powi(2 * i as i32 + 2),
        Kind::Zern(_) => 1e-3,
        Kind::Dn => 1e-3,
        Kind::Disp => 1e-2,
        _ => 1.0,
    }
}

fn map_maybe_par<T: Sync, R: Send>(
    v: &[T],
    f: impl Fn(&T) -> Result<R> + Sync + Send,
) -> Result<Vec<R>> {
    if v.len() > 1 && !crate::par::sequential() {
        v.par_iter().map(crate::par::scoped(f)).collect()
    } else {
        v.iter().map(f).collect()
    }
}

/// 項で使う波長（単色はその波長、多色は重みのある波長）
fn waves(s: &System, w: Option<usize>) -> Vec<usize> {
    match w {
        Some(w) => vec![w],
        None => (0..s.wavelengths.len())
            .filter(|&i| s.weights[i] > 0.0)
            .collect(),
    }
}

fn term_rows(s: &System, k: &TermKind) -> usize {
    match k {
        TermKind::Spot { wave, rings, .. } => {
            2 * waves(s, *wave).len() * pupil_gauss_shared(*rings).0.len()
        }
        TermKind::Wavefront { wave, rings, .. } => {
            waves(s, *wave).len() * pupil_gauss_shared(*rings).0.len()
        }
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
        | TermKind::Coupling { .. } => 1,
        TermKind::User(u) => u.0.rows(),
    }
}

/// 像面が平面（波面の解析的な微分の前提）
fn flat_image(s: &System) -> bool {
    let d = &s.surfaces[s.image()];
    matches!(d.typ, SurfType::Standard | SurfType::EvenAsph)
        && d.c == 0.0
        && d.asph.iter().all(|&a| a == 0.0)
}

fn supports_analytic(s: &System, k: &TermKind) -> bool {
    match k {
        TermKind::Spot { .. } | TermKind::Centroid { .. } => true,
        TermKind::Wavefront { .. } | TermKind::Opd { .. } => {
            flat_image(s) && (s.ropd == 1 || s.ropd == 2)
        }
        TermKind::Ray { q, .. } => *q != RayQ::Z,
        _ => false,
    }
}

/// 開始の光線を比べる瞳の点
const PROBE: [(f64, f64); 4] = [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (-0.7, -0.7)];

/// 物体の座標系での開始の光線（位置・方向を並べたもの）
fn launch_of(s: &System, field: FieldSel, w: usize) -> Result<Vec<f64>> {
    let px: Vec<f64> = PROBE.iter().map(|p| p.0).collect();
    let py: Vec<f64> = PROBE.iter().map(|p| p.1).collect();
    let t = s.try_trace(field, &px, &py, Some(w), Some(1.min(s.image())), false)?;
    let f = s.object_frame();
    let rt = transpose(&f.r);
    let mut v = Vec::new();
    for i in 0..t.p0.len() {
        v.extend(mv(&rt, sub(t.p0[i], f.o)));
        v.extend(mv(&rt, t.k0[i]));
    }
    Ok(v)
}

/// 変数 p（値 v）を h ずらすと、項の光線の始め方が変わるか
fn launch_moves(s: &System, k: &TermKind, p: Param, v: f64, h: f64) -> Result<bool> {
    let (field, wave) = match k {
        TermKind::Spot { field, wave, .. }
        | TermKind::Wavefront { field, wave, .. }
        | TermKind::Centroid { field, wave, .. }
        | TermKind::Ray { field, wave, .. }
        | TermKind::Opd { field, wave, .. } => (*field, *wave),
        _ => return Ok(true),
    };
    let s2 = s.with_params_fast(&[(p, v + h)])?;
    let mut ws = waves(s, wave);
    if !ws.contains(&s.primary) {
        ws.push(s.primary);
    }
    for w in ws {
        let a = launch_of(s, field, w)?;
        let b = launch_of(&s2, field, w)?;
        let scale = a.iter().fold(1.0f64, |m, x| m.max(x.abs()));
        if a.iter()
            .zip(&b)
            .any(|(x, y)| !((x - y).abs() <= 1e-12 * scale))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 追跡（止まった光線があればエラー）
fn trace_ok(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: usize,
    record: bool,
) -> Result<Trace> {
    let t = s.try_trace(field, px, py, Some(w), None, record)?;
    if let Some(i) = t.rays.iter().position(|r| r.err != 0) {
        invalid!(
            "光線が止まった（波長 {w}、瞳 ({:.3}, {:.3})、面 {}）",
            px[i],
            py[i],
            t.rays[i].err
        );
    }
    Ok(t)
}

/// 項の残差（重みを掛ける前）。ps を与えると、その列の解析的なヤコビアンも
fn eval_term(s: &System, k: &TermKind, ps: Option<&[Param]>) -> Result<TermOut> {
    match k {
        TermKind::Spot {
            field,
            wave,
            rings,
            reference,
        } => spot_term(s, *field, *wave, *rings, *reference, ps),
        TermKind::Centroid {
            field,
            wave,
            rings,
            y,
            target,
        } => centroid_term(s, *field, *wave, *rings, *y, *target, ps),
        TermKind::Wavefront {
            field,
            wave,
            rings,
            reference,
        } => wfe_term(s, *field, *wave, *rings, *reference, ps),
        TermKind::Mtf {
            field,
            wave,
            n,
            freq,
            dir,
            target,
        } => {
            // 片方の向きだけの項はその向きだけ求める（値は両方求めたときと同じ）
            let only = match dir {
                MtfDir::Tan => Some(1),
                MtfDir::Sag => Some(0),
                MtfDir::Both => None,
            };
            let m = mtf_only(s, *field, *wave, *n, &[*freq], MtfMethod::default(), only)?;
            let r = match dir {
                MtfDir::Tan => vec![m.tan[0] - target],
                MtfDir::Sag => vec![m.sag[0] - target],
                MtfDir::Both => vec![m.tan[0] - target, m.sag[0] - target],
            };
            if r.iter().any(|v| !v.is_finite()) {
                invalid!("MTF を計算できない");
            }
            Ok((r, None))
        }
        TermKind::Paraxial { q, target } => {
            let v = q.value(s);
            if !v.is_finite() {
                invalid!("系の {} が有限でない", q.name());
            }
            Ok((vec![v - target], None))
        }
        TermKind::Ray {
            field,
            wave,
            px,
            py,
            surf,
            q,
            target,
        } => ray_term(s, *field, *wave, *px, *py, *surf, *q, *target, ps),
        TermKind::Opd {
            field,
            wave,
            px,
            py,
            target,
        } => opd_term(s, *field, *wave, *px, *py, *target, ps),
        TermKind::Zernike {
            field,
            wave,
            kind,
            term,
            n,
            target,
        } => {
            let z = crate::analysis::zernike(s, *field, *wave, *kind, *term, *n, false)?;
            let v = z.coef[term - 1];
            if !v.is_finite() {
                invalid!("Zernike 係数を計算できない");
            }
            Ok((vec![v - target], None))
        }
        TermKind::Coupling { wave, opts, target } => {
            let c = crate::fiber::fiber_coupling(s, wave.unwrap_or(s.primary), opts, None)?;
            if !c.eta.is_finite() {
                invalid!("結合効率を計算できない");
            }
            Ok((vec![c.eta - target], None))
        }
        TermKind::User(u) => Ok((u.eval_checked(&[s])?.remove(0), None)),
    }
}

/// 光線の量の項
#[allow(clippy::too_many_arguments)]
fn ray_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    px: f64,
    py: f64,
    surf: Option<usize>,
    q: RayQ,
    target: f64,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    let w = wave.unwrap_or(s.primary);
    let j = surf.unwrap_or(s.image());
    let t = s.try_trace(field, &[px], &[py], Some(w), Some(j), ps.is_some())?;
    let x = &t.rays[0];
    if x.err != 0 {
        invalid!(
            "光線が止まった（波長 {w}、瞳 ({px:.3}, {py:.3})、面 {}）",
            x.err
        );
    }
    let v = match q {
        RayQ::X => x.xyz[0],
        RayQ::Y => x.xyz[1],
        RayQ::Z => x.xyz[2],
        RayQ::L => x.lmn[0],
        RayQ::M => x.lmn[1],
        RayQ::N => x.lmn[2],
    };
    let jac = match ps {
        Some(ps) if q != RayQ::Z => {
            let g = s.land_grad(&t, w, ps, None)?;
            let d: Vec<f64> = (0..ps.len())
                .map(|k| {
                    let g = &g[k * 4..k * 4 + 4];
                    match q {
                        RayQ::X => g[0],
                        RayQ::Y => g[1],
                        RayQ::L => g[2],
                        RayQ::M => g[3],
                        // l² + m² + n² = 1
                        _ => -(x.lmn[0] * g[2] + x.lmn[1] * g[3]) / x.lmn[2],
                    }
                })
                .collect();
            if d.iter().any(|v| !v.is_finite()) {
                invalid!("光線の微分を求められない");
            }
            Some(d)
        }
        Some(_) => invalid!("光線の z は差分"),
        None => None,
    };
    Ok((vec![v - target], jac))
}

/// 1 本の光線の OPD の項（[`crate::analysis::opd`] の ref_primary = true と同じ値）
fn opd_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    px: f64,
    py: f64,
    target: f64,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    if !s.afocal && s.ropd != 1 && s.ropd != 2 {
        invalid!("OPD の基準 {}（射出瞳・無限遠のみ）", s.ropd);
    }
    let w = wave.unwrap_or(s.primary);
    let rec = ps.is_some();
    let t = trace_ok(s, field, &[px], &[py], w, rec)?;
    let tc = trace_ok(s, field, &[0.0], &[0.0], w, rec)?;
    let wr = s.primary;
    let tp = if wr == w {
        tc.clone()
    } else {
        trace_ok(s, field, &[0.0], &[0.0], wr, rec)?
    };
    let (pc, kc) = (tp.rays[0].xyz, tp.rays[0].lmn);
    let c = &tc.rays[0];
    let x = &t.rays[0];
    let wl = s.wavelengths[w] * 1e-3;
    let lc = to_ref(s, w, pc, kc, c.xyz, c.lmn, c.opl) + c.lens_opd;
    let v = (lc - (to_ref(s, w, pc, kc, x.xyz, x.lmn, x.opl) + x.lens_opd)) / wl;
    if !v.is_finite() {
        invalid!("OPD を計算できない");
    }
    let Some(ps) = ps else {
        return Ok((vec![v - target], None));
    };
    if [x, c, &tp.rays[0]].iter().any(|r| r.lens_opd != 0.0) {
        invalid!("理想レンズの系");
    }
    let ni = s.image_index(w);
    let nr = s.image_index(wr);
    let gl = s.land_grad(&t, w, ps, None)?;
    let go = s.opl_grad(&t, w, ps)?;
    let glc = s.land_grad(&tc, w, ps, None)?;
    let goc = s.opl_grad(&tc, w, ps)?;
    let (glp, gop) = if wr == w {
        (glc.clone(), goc.clone())
    } else {
        (s.land_grad(&tp, wr, ps, None)?, s.opl_grad(&tp, wr, ps)?)
    };
    let mut d = Vec::with_capacity(ps.len());
    for (q, &p) in ps.iter().enumerate() {
        let (de, dn) = pupil_index_d(s, p, w)?;
        let (dpc, dkc, _) = ray_d(nr, kc, &glp[q * 4..q * 4 + 4], gop[q]);
        let (dqc, dkcw, doc) = ray_d(ni, c.lmn, &glc[q * 4..q * 4 + 4], goc[q]);
        let dlc = to_ref_d(
            s,
            w,
            pc,
            kc,
            c.xyz,
            c.lmn,
            (dpc, dkc, dqc, dkcw, doc, dn, de),
        );
        let (dq, dk, dop) = ray_d(ni, x.lmn, &gl[q * 4..q * 4 + 4], go[q]);
        let dl = to_ref_d(s, w, pc, kc, x.xyz, x.lmn, (dpc, dkc, dq, dk, dop, dn, de));
        d.push((dlc - dl) / wl);
    }
    if d.iter().any(|v| !v.is_finite()) {
        invalid!("OPD の微分を求められない");
    }
    Ok((vec![v - target], Some(d)))
}

/// 射出瞳の位置と像空間の屈折率（波長 w）の、パラメータ p での微分（中心差分）
fn pupil_index_d(s: &System, p: Param, w: usize) -> Result<(f64, f64)> {
    let v = s.get_param(p)?;
    // 回転対称な系の射出瞳は近軸で滑らか。回転対称でない系は実光線の外挿で求めるので 1e-8 程度の
    // 揺れがあり、刻みを大きめにする（中心差分なので、偏心 0 で近軸と実光線の求め方が切り替わる分も
    // 打ち消し合う）
    let at = |h: f64| -> Result<(System, System)> {
        Ok((
            s.with_params_fast(&[(p, v + h)])?,
            s.with_params_fast(&[(p, v - h)])?,
        ))
    };
    let sc = v.abs().max(typical(p.kind));
    let mut h = 1e-6 * sc;
    let (mut a, mut b) = at(h)?;
    if !(a.symmetric() && b.symmetric()) {
        h = 1e-3 * sc;
        (a, b) = at(h)?;
    }
    Ok((
        (a.exit_pupil() - b.exit_pupil()) / (2.0 * h),
        (a.image_index(w) - b.image_index(w)) / (2.0 * h),
    ))
}

fn spot_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    rings: usize,
    reference: RmsRef,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    let pg = pupil_gauss_shared(rings);
    let (px, py, wts) = (&pg.0, &pg.1, &pg.2);
    let n = px.len();
    let ws = waves(s, wave);
    let rec = ps.is_some();
    let np = ps.map_or(0, |p| p.len());
    let mut traces = Vec::with_capacity(ws.len());
    let mut grads = Vec::with_capacity(ws.len());
    for &w in &ws {
        let t = trace_ok(s, field, px, py, w, rec)?;
        if let Some(ps) = ps {
            grads.push(s.land_grad(&t, w, ps, None)?);
        }
        traces.push(t);
    }
    // 重み（spot() と同じ: 単色は波長の重みによらない）
    let mut wt = vec![0.0; ws.len() * n];
    for (k, &w) in ws.iter().enumerate() {
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[w] };
        for i in 0..n {
            if traces[k].ok(i) {
                wt[k * n + i] = wts[i] * kw;
            }
        }
    }
    let sw: f64 = wt.iter().sum();
    if !(sw > 0.0) {
        invalid!("有効な光線がない（すべてけられた）");
    }
    let xy = |k: usize, i: usize| (traces[k].rays[i].xyz[0], traces[k].rays[i].xyz[1]);
    // 基準とその微分
    let (cx, cy, dc): (f64, f64, Vec<[f64; 2]>) = match reference {
        RmsRef::Centroid => {
            let (mut cx, mut cy) = (0.0, 0.0);
            let mut dc = vec![[0.0; 2]; np];
            for k in 0..ws.len() {
                for i in 0..n {
                    let a = wt[k * n + i];
                    if a == 0.0 {
                        continue;
                    }
                    let (x, y) = xy(k, i);
                    cx += a * x;
                    cy += a * y;
                    for (q, d) in dc.iter_mut().enumerate() {
                        let g = &grads[k][(i * np + q) * 4..];
                        d[0] += a * g[0];
                        d[1] += a * g[1];
                    }
                }
            }
            dc.iter_mut().for_each(|d| {
                d[0] /= sw;
                d[1] /= sw;
            });
            (cx / sw, cy / sw, dc)
        }
        RmsRef::Chief => {
            let wc = wave.unwrap_or(s.primary);
            let t = trace_ok(s, field, &[0.0], &[0.0], wc, rec)?;
            if !t.traced(0) {
                invalid!("主光線が像面に届かない");
            }
            let mut dc = vec![[0.0; 2]; np];
            if let Some(ps) = ps {
                let g = s.land_grad(&t, wc, ps, None)?;
                for (q, d) in dc.iter_mut().enumerate() {
                    *d = [g[q * 4], g[q * 4 + 1]];
                }
            }
            (t.rays[0].xyz[0], t.rays[0].xyz[1], dc)
        }
    };
    let m = 2 * ws.len() * n;
    let mut r = vec![0.0; m];
    let mut j = ps.map(|_| vec![0.0; m * np]);
    for k in 0..ws.len() {
        for i in 0..n {
            let a = wt[k * n + i];
            if a == 0.0 {
                continue;
            }
            let a = (a / sw).sqrt();
            let (x, y) = xy(k, i);
            let row = 2 * (k * n + i);
            r[row] = a * (x - cx);
            r[row + 1] = a * (y - cy);
            if let Some(j) = j.as_mut() {
                for q in 0..np {
                    let g = &grads[k][(i * np + q) * 4..];
                    j[row * np + q] = a * (g[0] - dc[q][0]);
                    j[(row + 1) * np + q] = a * (g[1] - dc[q][1]);
                }
            }
        }
    }
    Ok((r, j))
}

/// 像面の光束の重心（スポットの項と同じ光線・重み）の x か y − target と、その解析的な微分
fn centroid_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    rings: usize,
    y: bool,
    target: f64,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    let pg = pupil_gauss_shared(rings);
    let (px, py, wts) = (&pg.0, &pg.1, &pg.2);
    let n = px.len();
    let ws = waves(s, wave);
    let np = ps.map_or(0, |p| p.len());
    let c = usize::from(y);
    let (mut sw, mut sc) = (0.0, 0.0);
    let mut dc = vec![0.0; np];
    for &w in &ws {
        let t = trace_ok(s, field, px, py, w, ps.is_some())?;
        let g = match ps {
            Some(ps) => Some(s.land_grad(&t, w, ps, None)?),
            None => None,
        };
        let kw = if ws.len() == 1 { 1.0 } else { s.weights[w] };
        for i in 0..n {
            if !t.ok(i) {
                continue;
            }
            let a = wts[i] * kw;
            sw += a;
            sc += a * t.rays[i].xyz[c];
            if let Some(g) = &g {
                for (q, d) in dc.iter_mut().enumerate() {
                    *d += a * g[(i * np + q) * 4 + c];
                }
            }
        }
    }
    if !(sw > 0.0) {
        invalid!("有効な光線がない（すべてけられた）");
    }
    let j = ps.map(|_| dc.iter().map(|d| d / sw).collect());
    Ok((vec![sc / sw - target], j))
}

/// to_ref（[`crate::analysis`]）の値の微分。d は (dpc, dkc, dq, dk, dopl, dni, dexpp)
#[allow(clippy::too_many_arguments)]
fn to_ref_d(
    s: &System,
    w: usize,
    pc: V3,
    kc: V3,
    q: V3,
    k: V3,
    d: (V3, V3, V3, V3, f64, f64, f64),
) -> f64 {
    let (dpc, dkc, dq, dk, dopl, dni, dexpp) = d;
    let ni = s.image_index(w);
    let expp = s.exit_pupil();
    let dd = sub(q, pc);
    let ddd = sub(dq, dpc);
    if s.afocal {
        // opl + ni a / b、a = (pe − q)·kc、pe = 射出瞳の位置で主光線の上の点（無限遠なら pc）
        let (pe, dpe) = if expp.is_finite() && expp.abs() <= 1e9 {
            let u = (expp - pc[2]) / kc[2];
            let du = (dexpp - dpc[2]) / kc[2] - u * dkc[2] / kc[2];
            (
                add(pc, scale(kc, u)),
                add(add(dpc, scale(dkc, u)), scale(kc, du)),
            )
        } else {
            (pc, dpc)
        };
        let a = dot(sub(pe, q), kc);
        let da = dot(sub(dpe, dq), kc) + dot(sub(pe, q), dkc);
        let b = dot(k, kc);
        let db = dot(dk, kc) + dot(k, dkc);
        return dopl + dni * a / b + ni * (da / b - a * db / (b * b));
    }
    if s.ropd == 1 || !expp.is_finite() || expp.abs() > 1e9 {
        // opl − ni a / b
        let a = dot(dd, kc);
        let b = dot(k, kc);
        let da = dot(ddd, kc) + dot(dd, dkc);
        let db = dot(dk, kc) + dot(k, dkc);
        return dopl - dni * a / b - ni * (da / b - a * db / (b * b));
    }
    let sc = (expp - pc[2]) / kc[2];
    let dsc = (dexpp - dpc[2]) / kc[2] - sc * dkc[2] / kc[2];
    let rr = sc.abs();
    let drr = if sc < 0.0 { -dsc } else { dsc };
    let b = dot(dd, k);
    let db = dot(ddd, k) + dot(dd, dk);
    let c = dot(dd, dd) - rr * rr;
    let dc = 2.0 * dot(dd, ddd) - 2.0 * rr * drr;
    let e = b * b - c;
    let (disc, ddisc) = if e > 0.0 {
        let disc = e.sqrt();
        (disc, (2.0 * b * db - dc) / (2.0 * disc))
    } else {
        (0.0, 0.0)
    };
    let t = if sc < 0.0 { -b - disc } else { -b + disc };
    let dt = if sc < 0.0 { -db - ddisc } else { -db + ddisc };
    dopl + dni * t + ni * dt
}

/// 着地点の微分 g（[x, y, l, m]）と光路長の微分 go から、平面の像面での (dq, dk, dopl)。
/// 光路長は開始を固定した微分: 端点を固定した微分 + n (l dx + m dy)
fn ray_d(ni: f64, lmn: V3, g: &[f64], go: f64) -> (V3, V3, f64) {
    let dq = [g[0], g[1], 0.0];
    let dn = -(lmn[0] * g[2] + lmn[1] * g[3]) / lmn[2];
    let dk = [g[2], g[3], dn];
    (dq, dk, go + ni * (lmn[0] * g[0] + lmn[1] * g[1]))
}

fn wfe_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    rings: usize,
    reference: RmsRef,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    let pg = pupil_gauss_shared(rings);
    let (px, py, wts) = (&pg.0, &pg.1, &pg.2);
    let n = px.len();
    let ws = waves(s, wave);
    let rec = ps.is_some();
    let np = ps.map_or(0, |p| p.len());
    // 波長の重み（rms() と同じ: 単色は 1、多色は波長の重みで平均）
    let kw: Vec<f64> = ws
        .iter()
        .map(|&w| if ws.len() == 1 { 1.0 } else { s.weights[w] })
        .collect();
    let ksum: f64 = kw.iter().sum();
    // 近軸の量の微分（射出瞳の位置・像空間の屈折率）
    let par_d: Vec<(f64, Vec<f64>)> = match ps {
        Some(ps) => ps
            .iter()
            .map(|&p| {
                let v = s.get_param(p)?;
                // 回転対称な系の射出瞳は近軸で滑らか。回転対称でない系は実光線の外挿で求めるので 1e-8 程度の
                // 揺れがあり、刻みを大きめにする（中心差分なので、偏心 0 で近軸と実光線の求め方が切り替わる分も
                // 打ち消し合う）
                let at = |h: f64| -> Result<(System, System)> {
                    Ok((
                        s.with_params_fast(&[(p, v + h)])?,
                        s.with_params_fast(&[(p, v - h)])?,
                    ))
                };
                let sc = v.abs().max(typical(p.kind));
                let mut h = 1e-6 * sc;
                let (mut a, mut b) = at(h)?;
                if !(a.symmetric() && b.symmetric()) {
                    h = 1e-3 * sc;
                    (a, b) = at(h)?;
                }
                let de = (a.exit_pupil() - b.exit_pupil()) / (2.0 * h);
                let dn: Vec<f64> = ws
                    .iter()
                    .map(|&w| (a.image_index(w) - b.image_index(w)) / (2.0 * h))
                    .collect();
                Ok((de, dn))
            })
            .collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let m = ws.len() * n;
    let mut r = vec![0.0; m];
    let mut jac = ps.map(|_| vec![0.0; m * np]);
    // sweep::rms と同じ定義（OpticStudio の RMS 波面）: 単色は自波長の主光線、多色は主波長の主光線を参照球の中心に。
    // chief は波長ごとのピストンを、centroid はそれに加えて方向余弦の差 (L − Lc, M − Mc) の係数（多色は全波長で共通、
    // Lc は参照の主光線）を重み付き最小二乗で除く
    let poly = wave.is_none();
    let nw = ws.len();
    let nt = if reference == RmsRef::Centroid { 2 } else { 0 };
    let nu = nw + nt;
    // 有効な光線ごと: (波長 k, 光線 i, OPD, L − Lc, M − Mc, 波長の中の重み q)
    let mut rows: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::new();
    // 微分（rows と同じ順）: dOPD・d(L − Lc)・d(M − Mc) [パラメータ]
    let mut drows: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> = Vec::new();
    for (k, &w) in ws.iter().enumerate() {
        let t = trace_ok(s, field, px, py, w, rec)?;
        let tc = trace_ok(s, field, &[0.0], &[0.0], w, rec)?;
        let wr = if poly { s.primary } else { w };
        let tp = if wr == w {
            tc.clone()
        } else {
            trace_ok(s, field, &[0.0], &[0.0], wr, rec)?
        };
        let c = &tc.rays[0];
        let (pc, kc) = (tp.rays[0].xyz, tp.rays[0].lmn);
        let lc = to_ref(s, w, pc, kc, c.xyz, c.lmn, c.opl) + c.lens_opd;
        let wl = s.wavelengths[w] * 1e-3;
        let opd: Vec<f64> = t
            .rays
            .iter()
            .map(|x| (lc - (to_ref(s, w, pc, kc, x.xyz, x.lmn, x.opl) + x.lens_opd)) / wl)
            .collect();
        let idx: Vec<usize> = (0..n).filter(|&i| t.ok(i) && opd[i].is_finite()).collect();
        let sw: f64 = idx.iter().map(|&i| wts[i]).sum();
        if idx.is_empty() || !(sw > 0.0) {
            invalid!("有効な光線がない（すべてけられた）");
        }
        for &i in &idx {
            let x = &t.rays[i];
            // アフォーカルの傾きの基底は瞳座標（sweep::rms と同じ）
            let (bl, bm) = if s.afocal {
                (px[i], py[i])
            } else {
                (x.lmn[0] - kc[0], x.lmn[1] - kc[1])
            };
            rows.push((k, i, opd[i], bl, bm, wts[i] / sw));
        }
        if let Some(ps) = ps {
            if t.rays
                .iter()
                .chain(&tc.rays)
                .chain(&tp.rays)
                .any(|x| x.lens_opd != 0.0)
            {
                invalid!("理想レンズの系");
            }
            let ni = s.image_index(w);
            let gl = s.land_grad(&t, w, ps, None)?;
            let go = s.opl_grad(&t, w, ps)?;
            let glc = s.land_grad(&tc, w, ps, None)?;
            let goc = s.opl_grad(&tc, w, ps)?;
            let (glp, gop) = if wr == w {
                (glc.clone(), goc.clone())
            } else {
                (s.land_grad(&tp, wr, ps, None)?, s.opl_grad(&tp, wr, ps)?)
            };
            let nr = s.image_index(wr);
            let mut d: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> =
                vec![(vec![0.0; np], vec![0.0; np], vec![0.0; np]); idx.len()];
            for q in 0..np {
                let (de, ref dn) = par_d[q];
                // 参照の主光線（参照球の中心）と、この波長の主光線
                let (dpc, dkc, _) = ray_d(nr, kc, &glp[q * 4..q * 4 + 4], gop[q]);
                let (dqc, dkc_w, doc) = ray_d(ni, c.lmn, &glc[q * 4..q * 4 + 4], goc[q]);
                let dlc = to_ref_d(
                    s,
                    w,
                    pc,
                    kc,
                    c.xyz,
                    c.lmn,
                    (dpc, dkc, dqc, dkc_w, doc, dn[k], de),
                );
                for (j, &i) in idx.iter().enumerate() {
                    let x = &t.rays[i];
                    let (dq, dk, dop) = ray_d(
                        ni,
                        x.lmn,
                        &gl[(i * np + q) * 4..(i * np + q) * 4 + 4],
                        go[i * np + q],
                    );
                    let dl = to_ref_d(
                        s,
                        w,
                        pc,
                        kc,
                        x.xyz,
                        x.lmn,
                        (dpc, dkc, dq, dk, dop, dn[k], de),
                    );
                    d[j].0[q] = (dlc - dl) / wl;
                    if !s.afocal {
                        d[j].1[q] = dk[0] - dkc[0];
                        d[j].2[q] = dk[1] - dkc[1];
                    }
                }
            }
            drows.extend(d);
        }
    }
    // 当てはめの基底（未知数: 波長ごとのピストン nw 個 + 傾き nt 個）
    let basis = |row: &(usize, usize, f64, f64, f64, f64)| -> Vec<f64> {
        let mut b = vec![0.0; nu];
        b[row.0] = 1.0;
        if nt == 2 {
            b[nw] = row.3;
            b[nw + 1] = row.4;
        }
        b
    };
    let bs: Vec<Vec<f64>> = rows.iter().map(basis).collect();
    let mut g = vec![vec![0.0; nu]; nu];
    let mut v = vec![0.0; nu];
    for (row, b) in rows.iter().zip(&bs) {
        for a in 0..nu {
            v[a] += row.5 * b[a] * row.2;
            for c in 0..nu {
                g[a][c] += row.5 * b[a] * b[c];
            }
        }
    }
    let cf = crate::sweep::solve(g.clone(), v);
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let e: Vec<f64> = rows
        .iter()
        .zip(&bs)
        .map(|(row, b)| row.2 - dot(b, &cf))
        .collect();
    let amp: Vec<f64> = rows
        .iter()
        .map(|row| (kw[row.0] / ksum * row.5).sqrt())
        .collect();
    for (j, row) in rows.iter().enumerate() {
        r[row.0 * n + row.1] = amp[j] * e[j];
    }
    if let Some(jm) = jac.as_mut() {
        // e = y − B c、c = G⁻¹ Bᵀ Q y の微分:
        // de = (dy − dB c) − B dc、dc = G⁻¹ (dBᵀ Q e + Bᵀ Q (dy − dB c))
        for q in 0..np {
            let dbc = |j: usize| -> f64 {
                if nt == 2 {
                    drows[j].1[q] * cf[nw] + drows[j].2[q] * cf[nw + 1]
                } else {
                    0.0
                }
            };
            let u: Vec<f64> = (0..rows.len()).map(|j| drows[j].0[q] - dbc(j)).collect();
            let mut rhs = vec![0.0; nu];
            for (j, row) in rows.iter().enumerate() {
                for a in 0..nu {
                    rhs[a] += row.5 * bs[j][a] * u[j];
                }
                if nt == 2 {
                    rhs[nw] += row.5 * drows[j].1[q] * e[j];
                    rhs[nw + 1] += row.5 * drows[j].2[q] * e[j];
                }
            }
            let dc = crate::sweep::solve(g.clone(), rhs);
            for (j, row) in rows.iter().enumerate() {
                jm[(row.0 * n + row.1) * np + q] = amp[j] * (u[j] - dot(&bs[j], &dc));
            }
        }
    }
    Ok((r, jac))
}

// ------------------------------------------------------------------------------------------ DLS

/// DLS の設定
#[derive(Debug, Clone, Copy)]
pub struct DlsOpts {
    /// 反復の上限
    pub max_iter: usize,
    /// 1 回の反復での評価関数（Σ r²）の相対的な減りがこれ未満で止める
    pub ftol: f64,
    /// 変数の変化（目安の大きさで割った値）の最大がこれ未満で止める
    pub xtol: f64,
    /// 減衰の初期値（JᵀJ の対角に対する比）
    pub damping: f64,
}

impl Default for DlsOpts {
    fn default() -> Self {
        DlsOpts {
            max_iter: 50,
            ftol: 1e-10,
            xtol: 1e-12,
            damping: 1e-3,
        }
    }
}

/// 止まった理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// 評価関数がほとんど減らない
    Converged,
    /// 変数がほとんど動かない
    SmallStep,
    /// どの減衰でも評価関数が減らない（局所解）
    NoProgress,
    /// 反復の上限
    MaxIter,
    /// 呼び出し側が止めた
    Cancelled,
    /// 変数がない
    NoVars,
    /// 途中でヤコビアンが求まらなかった（差分の系が追跡できないなど。理由は [`DlsResult::note`]）。それまでの
    /// 最良の x・系を返す
    JacobianFailed,
}

impl Stop {
    /// 出力用の名前
    pub fn as_str(self) -> &'static str {
        match self {
            Stop::Converged => "converged",
            Stop::SmallStep => "small_step",
            Stop::NoProgress => "no_progress",
            Stop::MaxIter => "max_iter",
            Stop::Cancelled => "cancelled",
            Stop::NoVars => "no_vars",
            Stop::JacobianFailed => "jacobian_failed",
        }
    }
}

/// 反復ごとの進み具合
#[derive(Debug, Clone)]
pub struct Progress {
    pub iter: usize,
    pub merit: f64,
    pub x: Vec<f64>,
    pub damping: f64,
    /// 残差を評価した回数（ヤコビアンの分を除く）
    pub evals: usize,
}

/// DLS の結果
#[derive(Debug, Clone)]
pub struct DlsResult {
    pub x0: Vec<f64>,
    pub x: Vec<f64>,
    pub merit0: f64,
    pub merit: f64,
    /// 反復ごとの評価関数の値（初期値を含む）
    pub history: Vec<f64>,
    pub iterations: usize,
    pub evals: usize,
    pub stop: Stop,
    /// 止まった理由の補足（[`Stop::JacobianFailed`] の誤りの文など）
    pub note: Option<String>,
    /// 最後の反復で評価関数に効かないとみなして動かさなかった変数の番号（0 始まり。差分の雑音しかない列）
    pub inactive: Vec<usize>,
    /// 最適化した系（構成ごと）
    pub systems: Vec<System>,
}

/// 評価関数に効かない変数の判定: 目安の大きさだけ動かしたときの残差の変化 ‖J_q‖·目安 が残差の EFFECT_TOL 倍に
/// 満たない変数は動かさない
const EFFECT_TOL: f64 = 1e-9;

/// この比（[`EFFECT_TOL`] と同じ量）より小さい列は、差分の丸めの雑音だけの列かを [`fd_noise`] で確かめる。
/// 差分の刻みは目安の 1e-6 倍で、追跡の丸めによる雑音の列はこの比で 3e-8〜5e-6（A0 の空気の中の絞り面の曲率・
/// 円錐係数で実測、波面の評価関数）。比だけで切ると、目安の大きさが合わない小さいレンズの高次の非球面係数など
/// 本当に効く弱い変数まで外れるので、比は疑いの目安にだけ使う
const SUSPECT_TOL: f64 = 1e-3;

/// 変数 q の列 J_q が差分の雑音だけか: 刻みを 1000 倍にした中心差分の列と比べ、1/10 未満なら雑音（線形に効く
/// 変数なら刻みによらず同じ列になり、雑音なら刻みに反比例して小さくなる）。以前はこの雑音の列が自由な変数になり、
/// 減衰が効かずに大きく動いて（A0 の絞り面が R = −4.9 mm など）系に書かれた
fn fd_noise(p: &Problem, x: &[f64], r: &[f64], j: &[f64], q: usize, scale: f64) -> bool {
    let nv = x.len();
    let h = 1e3 * p.fd_rel * scale;
    let v = &p.vars[q];
    let at = |d: f64| -> Option<Vec<f64>> {
        let xv = x[q] + d;
        if xv < v.lo || xv > v.hi {
            return None;
        }
        let mut xs = x.to_vec();
        xs[q] = xv;
        p.residuals(&xs).ok().filter(|rr| rr.len() == r.len())
    };
    let big: Vec<f64> = match (at(h), at(-h)) {
        (Some(rp), Some(rm)) => rp
            .iter()
            .zip(&rm)
            .map(|(a, b)| (a - b) / (2.0 * h))
            .collect(),
        (Some(rp), None) => rp.iter().zip(r).map(|(a, b)| (a - b) / h).collect(),
        (None, Some(rm)) => r.iter().zip(&rm).map(|(a, b)| (a - b) / h).collect(),
        // 確かめられなければ今の列のまま
        (None, None) => return false,
    };
    let n_small: f64 = (0..r.len())
        .map(|i| j[i * nv + q].powi(2))
        .sum::<f64>()
        .sqrt();
    let n_big: f64 = big.iter().map(|v| v * v).sum::<f64>().sqrt();
    n_big < 0.1 * n_small
}

/// 減衰最小二乗で最適化する
pub fn dls(p: &Problem, o: &DlsOpts) -> Result<DlsResult> {
    dls_with(p, o, |_| true)
}

/// [`dls`] で、反復ごとに progress を呼ぶ（false を返すと止める）
pub fn dls_with(
    p: &Problem,
    o: &DlsOpts,
    mut progress: impl FnMut(&Progress) -> bool,
) -> Result<DlsResult> {
    p.check()?;
    let nv = p.vars.len();
    let x0 = p.x0()?;
    let mut x: Vec<f64> = x0
        .iter()
        .zip(&p.vars)
        .map(|(&v, var)| v.clamp(var.lo, var.hi))
        .collect();
    let mut evals = 0;
    if nv == 0 {
        let r = p.residuals(&x)?;
        let m = p.merit_of(&r);
        return Ok(DlsResult {
            x0: x0.clone(),
            x: x0,
            merit0: m,
            merit: m,
            history: vec![m],
            iterations: 0,
            evals: 1,
            stop: Stop::NoVars,
            note: None,
            inactive: Vec::new(),
            systems: p.final_systems(&x)?,
        });
    }
    let (mut r, mut j) = p.jacobian(&x)?;
    let mut f = dot_n(&r, &r);
    let merit0 = p.merit_of(&r);
    let mut history = vec![merit0];
    let mut mu = o.damping;
    let mut nu = 2.0;
    let mut stop = Stop::MaxIter;
    let mut note = None;
    let mut inactive = Vec::new();
    let mut iterations = 0;
    let scale: Vec<f64> = p
        .vars
        .iter()
        .zip(&x0)
        .map(|(v, &xv)| xv.abs().max(typical(v.param.kind)))
        .collect();
    for it in 0..o.max_iter {
        iterations = it + 1;
        let m = r.len();
        // A = JᵀJ, g = Jᵀr
        let mut a = vec![0.0; nv * nv];
        let mut g = vec![0.0; nv];
        for i in 0..m {
            let row = &j[i * nv..(i + 1) * nv];
            for q in 0..nv {
                g[q] += row[q] * r[i];
                for s in 0..=q {
                    a[q * nv + s] += row[q] * row[s];
                }
            }
        }
        for q in 0..nv {
            for s in 0..q {
                a[s * nv + q] = a[q * nv + s];
            }
        }
        // 境界にあって外へ向かう変数と、評価関数にほとんど効かない変数（目安の大きさだけ動かしても残差の変化が
        // 残差の 1e-9 未満。差分の丸め誤差だけの列で大きく動かさないため）は動かさない
        let rn2 = f.sqrt();
        inactive = (0..nv)
            .filter(|&q| {
                let ratio = a[q * nv + q].sqrt() * scale[q] / rn2.max(f64::MIN_POSITIVE);
                !(ratio > EFFECT_TOL)
                    || (ratio < SUSPECT_TOL && fd_noise(p, &x, &r, &j, q, scale[q]))
            })
            .collect();
        evals += 2
            * (0..nv)
                .filter(|&q| {
                    let ratio = a[q * nv + q].sqrt() * scale[q] / rn2.max(f64::MIN_POSITIVE);
                    ratio > EFFECT_TOL && ratio < SUSPECT_TOL
                })
                .count();
        let effective = |q: usize| !inactive.contains(&q);
        let free: Vec<usize> = (0..nv)
            .filter(|&q| {
                let v = &p.vars[q];
                !((x[q] <= v.lo && g[q] > 0.0) || (x[q] >= v.hi && g[q] < 0.0)) && effective(q)
            })
            .collect();
        if free.is_empty() {
            stop = Stop::NoProgress;
            break;
        }
        let mut accepted = None;
        for _ in 0..30 {
            let nf = free.len();
            let mut af = vec![0.0; nf * nf];
            let mut bf = vec![0.0; nf];
            for (u, &q) in free.iter().enumerate() {
                bf[u] = -g[q];
                for (w, &s) in free.iter().enumerate() {
                    af[u * nf + w] = a[q * nv + s];
                }
                af[u * nf + u] += mu * a[q * nv + q];
            }
            let Some(d) = cholesky_solve(&mut af, &mut bf, nf) else {
                mu *= nu;
                nu *= 2.0;
                continue;
            };
            let mut xn = x.clone();
            for (u, &q) in free.iter().enumerate() {
                xn[q] = (x[q] + d[u]).clamp(p.vars[q].lo, p.vars[q].hi);
            }
            let step: Vec<f64> = (0..nv).map(|q| xn[q] - x[q]).collect();
            if step.iter().zip(&scale).all(|(s, c)| s.abs() <= o.xtol * c) {
                stop = Stop::SmallStep;
                break;
            }
            evals += 1;
            let rn = match p.residuals(&xn) {
                Ok(rn) => rn,
                Err(_) => {
                    mu *= nu;
                    nu *= 2.0;
                    continue;
                }
            };
            let fn_ = dot_n(&rn, &rn);
            // 予測される減り: −(2 gᵀδ + δᵀAδ)
            let mut pred = 0.0;
            for q in 0..nv {
                let mut ad = 0.0;
                for s in 0..nv {
                    ad += a[q * nv + s] * step[s];
                }
                pred -= 2.0 * g[q] * step[q] + step[q] * ad;
            }
            if fn_.is_finite() && fn_ < f {
                let rho = if pred > 0.0 { (f - fn_) / pred } else { 1.0 };
                mu *= (1.0 - (2.0 * rho - 1.0).powi(3)).max(1.0 / 3.0);
                nu = 2.0;
                accepted = Some((xn, rn, fn_));
                break;
            }
            mu *= nu;
            nu *= 2.0;
        }
        let Some((xn, rn, fn_)) = accepted else {
            if stop != Stop::SmallStep {
                stop = Stop::NoProgress;
            }
            break;
        };
        let rel = (f - fn_) / f.max(f64::MIN_POSITIVE);
        x = xn;
        f = fn_;
        r = rn;
        history.push(p.merit_of(&r));
        let pr = Progress {
            iter: iterations,
            merit: *history.last().unwrap(),
            x: x.clone(),
            damping: mu,
            evals,
        };
        if !progress(&pr) {
            stop = Stop::Cancelled;
            break;
        }
        if rel < o.ftol {
            stop = Stop::Converged;
            break;
        }
        if it + 1 < o.max_iter {
            // 途中のヤコビアンの失敗は、それまでの改善を捨てずに止める
            match p.jacobian(&x) {
                Ok(v) => (r, j) = v,
                Err(e) => {
                    stop = Stop::JacobianFailed;
                    note = Some(e.to_string());
                    break;
                }
            }
        }
    }
    Ok(DlsResult {
        x0,
        merit0,
        merit: p.merit_of(&r),
        history,
        iterations,
        evals,
        stop,
        note,
        inactive,
        systems: p.final_systems(&x)?,
        x,
    })
}

fn dot_n(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 対称正定値の n×n を Cholesky で解く（正定値でなければ None）
fn cholesky_solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<Vec<f64>> {
    for c in 0..n {
        let mut d = a[c * n + c];
        for k in 0..c {
            d -= a[c * n + k] * a[c * n + k];
        }
        if !(d > 0.0) {
            return None;
        }
        let d = d.sqrt();
        a[c * n + c] = d;
        for rr in c + 1..n {
            let mut v = a[rr * n + c];
            for k in 0..c {
                v -= a[rr * n + k] * a[c * n + k];
            }
            a[rr * n + c] = v / d;
        }
    }
    let mut y = b.to_vec();
    for i in 0..n {
        for k in 0..i {
            y[i] -= a[i * n + k] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    for i in (0..n).rev() {
        for k in i + 1..n {
            y[i] -= a[k * n + i] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    Some(y)
}
