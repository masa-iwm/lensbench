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
    /// 腕 4 rings + 2 本（[`lensbench_analysis::analysis::pupil_gauss`]。少ない rings でも角度の求積が収束に近い）
    #[default]
    Lensbench,
    /// OpticStudio 26.1 の RSCE・RSCH・RWCE・RWCH と同じ: 腕 2 rings 本を 90° 回して置く
    /// （[`lensbench_analysis::analysis::pupil_gauss_os`]。.zmx から読んだ行の項）
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

/// 計算の項の演算
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcOp {
    /// 和（1 つ以上）
    Sum,
    /// a − b
    Diff,
    /// 積（1 つ以上）
    Prod,
    /// a / b（b = 0 は誤り）
    Div,
    /// |a|
    Abs,
    /// √|a|（OpticStudio の SQRT と同じく負の値も絶対値の根）
    Sqrt,
    /// 1 / a（a = 0 は誤り）
    Reci,
    /// 最大・最小（1 つ以上）
    Max,
    Min,
    /// √(Σ a²)（1 つ以上）
    Qsum,
    /// 三角関数（角度はラジアン。asin・acos の定義域の外は誤り）
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
}

impl CalcOp {
    /// 項の JSON の名前
    pub fn name(self) -> &'static str {
        match self {
            CalcOp::Sum => "sum",
            CalcOp::Diff => "diff",
            CalcOp::Prod => "prod",
            CalcOp::Div => "div",
            CalcOp::Abs => "abs",
            CalcOp::Sqrt => "sqrt",
            CalcOp::Reci => "reci",
            CalcOp::Max => "max",
            CalcOp::Min => "min",
            CalcOp::Qsum => "qsum",
            CalcOp::Sin => "sin",
            CalcOp::Cos => "cos",
            CalcOp::Tan => "tan",
            CalcOp::Asin => "asin",
            CalcOp::Acos => "acos",
            CalcOp::Atan => "atan",
        }
    }

    /// 引数の値 v に当てた値（引数の数は [`CalcOp::arity`] のとおりとする）
    pub fn apply(self, v: &[f64]) -> Result<f64> {
        Ok(match self {
            CalcOp::Sum => v.iter().sum(),
            CalcOp::Diff => v[0] - v[1],
            CalcOp::Prod => v.iter().product(),
            CalcOp::Div | CalcOp::Reci => {
                let (a, b) = if self == CalcOp::Div {
                    (v[0], v[1])
                } else {
                    (1.0, v[0])
                };
                if b == 0.0 {
                    invalid!("division by zero in a calc term");
                }
                a / b
            }
            CalcOp::Abs => v[0].abs(),
            CalcOp::Sqrt => v[0].abs().sqrt(),
            CalcOp::Max => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            CalcOp::Min => v.iter().copied().fold(f64::INFINITY, f64::min),
            CalcOp::Qsum => v.iter().map(|a| a * a).sum::<f64>().sqrt(),
            CalcOp::Sin => v[0].sin(),
            CalcOp::Cos => v[0].cos(),
            CalcOp::Tan => v[0].tan(),
            CalcOp::Asin => v[0].asin(),
            CalcOp::Acos => v[0].acos(),
            CalcOp::Atan => v[0].atan(),
        })
    }

    /// 引数の数（下限, 上限）
    pub fn arity(self) -> (usize, usize) {
        match self {
            CalcOp::Sum | CalcOp::Prod | CalcOp::Max | CalcOp::Min | CalcOp::Qsum => {
                (1, usize::MAX)
            }
            CalcOp::Diff | CalcOp::Div => (2, 2),
            _ => (1, 1),
        }
    }
}

/// 計算の項の引数
#[derive(Debug, Clone, PartialEq)]
pub enum CalcArg {
    /// 1 つの値の項の値（項と同じ構成で）
    Term(Box<TermKind>),
    Const(f64),
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
    /// 周波数 freq [cycles/mm] の MTF − target（n は瞳の標本数、[`lensbench_analysis::analysis::mtf`]）
    Mtf {
        field: FieldSel,
        wave: Option<usize>,
        n: Option<usize>,
        freq: f64,
        dir: MtfDir,
        target: f64,
    },
    /// 近軸量 − target（[`System::paraxial`]）。ズーム軌跡の焦点距離・倍率の条件。wave（0 始まり）は efl・pimh だけに
    /// 書け、その波長を主波長にした系の値（OpticStudio の EFFL・PIMH の Wave）。None は主波長
    Paraxial {
        q: ParaxQ,
        wave: Option<usize>,
        target: f64,
    },
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
    /// 1 本の近軸光線の面 surf での量 − target（q は x・y・z・l・m・n・r・nl・nm・nn・tanx・tany。OpticStudio の
    /// PARX〜PATY）。微分は差分
    ParaxRay {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        surf: usize,
        q: RayQ,
        target: f64,
    },
    /// 1 本の光線の面 surf から to までの量 − target（微分は差分）。q は opth（面の間の光路長、OpticStudio の PLEN）か、
    /// aoi・aoe（面ごとの入射角・射出角 [度] を target と比べ、はみ出しの和 + target。upper は上限か。MXRI・MNRI・
    /// MXRE・MNRE）
    RayPath {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        surf: usize,
        to: usize,
        q: RayQ,
        upper: Option<bool>,
        target: f64,
    },
    /// 1 本の光線の OPD [波] − target（[`lensbench_analysis::analysis::opd`]、参照球の中心は主波長の主光線。OpticStudio の OPDC）
    Opd {
        field: FieldSel,
        wave: Option<usize>,
        px: f64,
        py: f64,
        target: f64,
    },
    /// Zernike 係数 [波] − target（[`lensbench_analysis::analysis::zernike`]、自波長の主光線基準、格子 n × n）。term は 1 始まり。
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
    /// 単一モードファイバへの結合効率（偏光なし）− target（[`lensbench_analysis::fiber::fiber_coupling`]、有限物体の系）
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
    /// 面 surf の量 − target（[`SurfQ`]。(x, y) は矢高・厚みを測る点 [レンズ単位]）。微分は差分
    Surface {
        surf: usize,
        q: SurfQ,
        x: f64,
        y: f64,
        /// 波長（power・ynip・index だけ。None は主波長）
        wave: Option<usize>,
        target: f64,
    },
    /// 面 from から to（1 始まり、to は像面の手前まで）の近軸の系の基点・焦点距離 − target（[`CardQ`]、波長 wave、
    /// None は主波長）。微分は差分
    Cardinal {
        from: usize,
        to: usize,
        q: CardQ,
        wave: Option<usize>,
        target: f64,
    },
    /// 視野 field・波長 wave（None は主波長）の実光線の F ナンバー − target（tan は瞳の y の縁、sag は x の縁の光線。
    /// OpticStudio の TFNO・SFNO）。微分は差分
    FNum {
        field: FieldSel,
        wave: Option<usize>,
        sag: bool,
        target: f64,
    },
    /// 面 surf（0 は全部の面の和）のザイデル収差係数 [波] − target（[`SeidelQ`]、波長 wave、None は主波長。
    /// OpticStudio の SPHA・COMA・ASTI・FCUR・DIST（面の寄与））。微分は差分
    Seidel {
        surf: usize,
        q: SeidelQ,
        wave: Option<usize>,
        target: f64,
    },
    /// 最大の視野の主光線の歪曲 − target（absolute なら実像高 − 近軸像高 [レンズ単位]、でなければ近軸像高に対する %。
    /// OpticStudio の DIST の Surf 0 と Absolute）。微分は差分
    Distortion {
        wave: Option<usize>,
        absolute: bool,
        target: f64,
    },
    /// 軸上の光線（zone 0 は近軸周辺光線、ほかは正規化瞳座標 (0, zone) の実光線）が光軸を横切る点の、像面からの距離
    /// [レンズ単位] − target（波長 wave、None は主波長。OpticStudio の LONA）。wave2 があればその波長の距離との差
    /// （wave2 − wave、AXCL）。微分は差分
    Longitudinal {
        wave: Option<usize>,
        wave2: Option<usize>,
        zone: f64,
        target: f64,
    },
    /// 最大の視野の近軸主光線の像面での高さの、波長 wave2 と wave（None は主波長）の差 [レンズ単位] − target
    /// （OpticStudio の LACL）。微分は差分
    LateralColor {
        wave: Option<usize>,
        wave2: usize,
        target: f64,
    },
    /// 計算の項: 引数（1 つの値の項の値か定数）の op − target（OpticStudio の SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI、
    /// CONS は定数の引数）。引数の項の target は使わない（値そのもの）。微分は差分
    Calc {
        op: CalcOp,
        args: Vec<CalcArg>,
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
    /// 主波長の主光線（瞳の中心）からの x・y のずれと、その長さ（同じ面。OpticStudio の TRAX・TRAY・TRAR）
    TraX,
    TraY,
    TraR,
    /// 光路長 [レンズ単位]（OpticStudio の OPTH）。無限遠物体では、同じ波長の主光線が面 1 を通る点を通り光線に垂直な
    /// 面から測る（OpticStudio 26.1 の実測どおり）。有限物体では物点から
    Opth,
    /// 大域の交点・面の後の方向余弦（OpticStudio の RAGX〜RAGC）
    Gx,
    Gy,
    Gz,
    Gl,
    Gm,
    Gn,
    /// 局所の交点の √(x² + y²)（REAR）
    R,
    /// 面の後の方向と局所の z 軸の角 [rad]（RANG）
    Ang,
    /// 面の後の方向の x・y の傾き l/n・m/n（RETX・RETY）
    TanX,
    TanY,
    /// 交点での面の法線（局所、−z 寄りの向き。RENA〜RENC）
    Nl,
    Nm,
    Nn,
    /// 入射角・射出角 [度]（RAID・RAED）と、その cos（RAIN・RAEN）
    Aoi,
    Aoe,
    CosI,
    CosE,
}

impl RayQ {
    /// 面ごとの記録から求める量
    pub fn recorded(&self) -> bool {
        !matches!(
            self,
            RayQ::X
                | RayQ::Y
                | RayQ::Z
                | RayQ::L
                | RayQ::M
                | RayQ::N
                | RayQ::TraX
                | RayQ::TraY
                | RayQ::TraR
                | RayQ::Opth
        )
    }
    /// 微分を差分で求める量
    pub fn by_diff(&self) -> bool {
        matches!(self, RayQ::Z | RayQ::Opth) || self.recorded()
    }
    pub fn parse(s: &str) -> Result<RayQ> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "x" | "reax" => RayQ::X,
            "y" | "reay" => RayQ::Y,
            "z" | "reaz" => RayQ::Z,
            "l" | "reaa" => RayQ::L,
            "m" | "reab" => RayQ::M,
            "n" | "reac" => RayQ::N,
            "trax" => RayQ::TraX,
            "tray" => RayQ::TraY,
            "trar" => RayQ::TraR,
            "opth" => RayQ::Opth,
            "gx" => RayQ::Gx,
            "gy" => RayQ::Gy,
            "gz" => RayQ::Gz,
            "gl" => RayQ::Gl,
            "gm" => RayQ::Gm,
            "gn" => RayQ::Gn,
            "r" => RayQ::R,
            "ang" => RayQ::Ang,
            "tanx" => RayQ::TanX,
            "tany" => RayQ::TanY,
            "nl" => RayQ::Nl,
            "nm" => RayQ::Nm,
            "nn" => RayQ::Nn,
            "aoi" => RayQ::Aoi,
            "aoe" => RayQ::Aoe,
            "cosi" => RayQ::CosI,
            "cose" => RayQ::CosE,
            _ => invalid!(
                "ray quantity {s} (x, y, z, l, m, n, trax, tray, trar, opth, gx, gy, gz, gl, gm, gn, r, ang, tanx, tany, nl, nm, nn, aoi, aoe, cosi, cose)"
            ),
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
            RayQ::TraX => "trax",
            RayQ::TraY => "tray",
            RayQ::TraR => "trar",
            RayQ::Opth => "opth",
            RayQ::Gx => "gx",
            RayQ::Gy => "gy",
            RayQ::Gz => "gz",
            RayQ::Gl => "gl",
            RayQ::Gm => "gm",
            RayQ::Gn => "gn",
            RayQ::R => "r",
            RayQ::Ang => "ang",
            RayQ::TanX => "tanx",
            RayQ::TanY => "tany",
            RayQ::Nl => "nl",
            RayQ::Nm => "nm",
            RayQ::Nn => "nn",
            RayQ::Aoi => "aoi",
            RayQ::Aoe => "aoe",
            RayQ::CosI => "cosi",
            RayQ::CosE => "cose",
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
    /// 角倍率: 像空間と物体空間の近軸主光線の傾きの比（AMAG）
    Amag,
    /// ラグランジュ不変量: 絞りの後の n · 近軸主光線の傾き · 近軸周辺光線の高さ（LINV）
    Linv,
    /// ペッツバール和の曲率 −n′ Σ (n′ − n) c / (n n′) [1/レンズ単位]（PETC）と、その逆数の半径（PETZ）
    Petc,
    Petz,
}

impl ParaxQ {
    pub const ALL: [ParaxQ; 17] = [
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
        ParaxQ::Amag,
        ParaxQ::Linv,
        ParaxQ::Petc,
        ParaxQ::Petz,
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
            ParaxQ::Amag => "amag",
            ParaxQ::Linv => "linv",
            ParaxQ::Petc => "petc",
            ParaxQ::Petz => "petz",
        }
    }
    /// 波長 wave（0 始まり、None は主波長）を主波長にした系の値。主波長を変えた系を作れなければ NaN
    pub fn value_at(&self, s: &System, wave: Option<usize>) -> f64 {
        if let Some(v) = self.at_wave(s, wave.unwrap_or(s.primary)) {
            return v;
        }
        match wave {
            Some(w) if w != s.primary => {
                let mut d = s.clone();
                match d.edit_raw(|rx| {
                    rx.primary = w;
                    Ok(())
                }) {
                    Ok(()) => self.value(&d),
                    Err(_) => f64::NAN,
                }
            }
            _ => self.value(s),
        }
    }
    /// 波長を選べる量（efl・pimh・amag・linv・petc・petz。OpticStudio の Wave）
    pub fn has_wave(&self) -> bool {
        matches!(
            self,
            ParaxQ::Efl | ParaxQ::Pimh | ParaxQ::Amag | ParaxQ::Linv | ParaxQ::Petc | ParaxQ::Petz
        )
    }
    /// 波長 w で近軸光線を追って求める量（主波長の入射瞳・EPD のまま。OpticStudio 26.1 の実測どおり）
    fn at_wave(&self, s: &System, w: usize) -> Option<f64> {
        let p = s.paraxial();
        let img = s.image();
        // 近軸主光線（物体空間の傾き 1、または有限物体で高さ 1）は p.chief と同じ打ち出しを波長 w で
        let chief = || {
            let (y0, u0) = (p.chief.0[1], p.chief.1[0]);
            s.ptrace(y0, u0, w, None)
        };
        Some(match self {
            ParaxQ::Amag => {
                let (_, uc) = chief();
                uc[img - 1] / p.chief.1[0]
            }
            // ミラーの後の符号・ペッツバール和のミラーの項は OpticStudio で確かめていないので、ミラーのある系は値を持たない
            ParaxQ::Linv if mirror_in(s, 1, s.stop()) => f64::NAN,
            ParaxQ::Petc | ParaxQ::Petz if mirror_in(s, 1, img - 1) => f64::NAN,
            ParaxQ::Linv => {
                // 主光線は最大の視野へ（傾き・高さは視野に比例）
                let (y0, u0) = (p.marginal.0[1], p.marginal.1[0]);
                let (ym, _) = s.ptrace(y0, u0, w, None);
                let (_, uc) = chief();
                let st = s.stop();
                s.n(st)[w] * uc[st] * ym[st] * chief_scale(s)
            }
            ParaxQ::Petc | ParaxQ::Petz => {
                let mut sum = 0.0;
                for j in 1..img {
                    let f = &s.surfaces[j];
                    if f.ignore || f.is_coord_break() {
                        continue;
                    }
                    let (n1, n2) = (s.index_before(j, w), s.n(j)[w]);
                    sum += (n2 - n1) * f.shape().paraxial_curv() / (n1 * n2);
                }
                let c = -s.image_index(w) * sum;
                if *self == ParaxQ::Petc {
                    c
                } else {
                    1.0 / c
                }
            }
            _ => return None,
        })
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
    pub fn target(&self) -> Option<f64> {
        match self {
            TermKind::Mtf { dir, target, .. } if *dir != MtfDir::Both => Some(*target),
            TermKind::Paraxial { target, .. }
            | TermKind::Ray { target, .. }
            | TermKind::RayPath { target, .. }
            | TermKind::ParaxRay { target, .. }
            | TermKind::Centroid { target, .. }
            | TermKind::Opd { target, .. }
            | TermKind::Zernike { target, .. }
            | TermKind::Coupling { target, .. }
            | TermKind::Thickness { target, .. }
            | TermKind::Surface { target, .. }
            | TermKind::Cardinal { target, .. }
            | TermKind::FNum { target, .. }
            | TermKind::Seidel { target, .. }
            | TermKind::Distortion { target, .. }
            | TermKind::Longitudinal { target, .. }
            | TermKind::LateralColor { target, .. }
            | TermKind::Calc { target, .. } => Some(*target),
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
        | TermKind::RayPath { .. }
        | TermKind::ParaxRay { .. }
        | TermKind::Centroid { .. }
        | TermKind::Opd { .. }
        | TermKind::Zernike { .. }
        | TermKind::Coupling { .. }
        | TermKind::Thickness { .. }
        | TermKind::Surface { .. }
        | TermKind::Cardinal { .. }
        | TermKind::FNum { .. }
        | TermKind::Seidel { .. }
        | TermKind::Distortion { .. }
        | TermKind::Longitudinal { .. }
        | TermKind::LateralColor { .. }
        | TermKind::Calc { .. } => 1,
        TermKind::User(u) => u.0.rows(),
    }
}

/// 面 a から b までにミラーがあるか（無視する面を除く）
pub(super) fn mirror_in(s: &System, a: usize, b: usize) -> bool {
    s.surfaces
        .get(a..=b)
        .unwrap_or(&[])
        .iter()
        .any(|f| !f.ignore && f.is_mirror())
}

/// 近軸主光線（[`lensbench_core::system::Paraxial::chief`]、物体空間の傾き 1 か物体の高さ 1）を最大の視野に合わせる倍率
fn chief_scale(s: &System) -> f64 {
    use lensbench_core::system::FieldType;
    let hmax = s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max);
    match s.field_type {
        FieldType::Angle => hmax.to_radians().tan(),
        _ => hmax,
    }
}
