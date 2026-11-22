//! .zmx の評価関数（[`lensbench_formats::zmx::merit`] の行）を評価関数の項の JSON（[`crate::merit`] の表）に写す。
//!
//! 写せる演算子は lensbench の項で同じ量を表せるものだけ。写せない行・意味の違う行は行番号つきの注意にして飛ばし、
//! 残りで評価関数を作る。重み 0 の行は評価関数に効かないので黙って飛ばす（OPGT・OPLT から参照されるときは使う）。
//! CONF の行の後の行はその構成の項になる。長さの目標はレンズ単位から mm に直す。
//!
//! OpticStudio 26.1（Double Gauss 28）との照合: 近軸量・REAX〜REAC・OPDC は 1e-9 以内で同じ値。RSCE・RSCH・RWCE・RWCH は
//! 求積の腕の数が違う（OpticStudio は 2·Ring、lensbench の項は 4·rings + 2。腕を 2·rings にすると単色・多色とも 1e-12 で
//! 一致するが、OpticStudio は Hx = 0 の視野で奇数の Ring のとき腕の置き方を変え、最大 2% 違う）ので、腕を増やして収束した
//! lensbench の値にする。MTFT・MTFS の標本（64 × 64 など）は lensbench の MTF の求積点の数と意味が違い、OpticStudio の
//! 低い標本は収束していないので渡さない（lensbench が収束する点の数を選ぶ）。ZERN の標本は格子の点の数として渡す。

use lensbench_formats::zmx::merit::{MeritFunction, MeritRow};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

mod calc;
mod shape;
mod wizard;
mod write;

pub use write::rows_from_terms;

/// .zmx から写した評価関数
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZmxMerit {
    /// 項の JSON（[`crate::merit::terms_from_json`] で読める）
    pub terms: Vec<Value>,
    /// 項ごとの元の行の番号（[`MeritRow::row`]。項の誤りを行の番号で知らせるのに使う）
    pub sources: Vec<usize>,
    /// 飛ばした行などの注意
    pub notes: Vec<String>,
    /// 項が使う構成の数（CONF の最大、CONF が無ければ 1）
    pub configs: usize,
}

/// 行を写すのに使う系の様子（[`ZmxSys::of`]）。既定は回転対称で、EFLA の単レンズを確かめない
#[derive(Debug, Clone, PartialEq)]
pub struct ZmxSys {
    /// 面の後の媒質が空気でないか（EFLA は面 Surf の前が空気・後がガラスで Surf + 1 の後が空気の単レンズだけ読む。空なら確かめない）
    pub glass: Vec<bool>,
    /// 回転対称か（でなければ X の向きの EFLX・CARD は読まない。lensbench の近軸追跡は Y-Z 面）
    pub symmetric: bool,
    /// 主波長が多重構成で変わるか（変われば EFFL・PIMH などの波長は主波長と同じ番号でも番号のまま読む。
    /// OpticStudio は Wave 0 を置いたときの主波長の番号にして保存する）
    pub primary_varies: bool,
    /// 波長の数（SPCH の Maxw 0 は最後の波長。0 なら分からない）
    pub waves: usize,
    /// 視野が角度か（グリッド歪曲の係数 ABCD・DISA は角の tan での微分で長さ。高さの視野では長さの比で単位がない）
    pub angle_field: bool,
    /// 多重構成の表の行（MCOV などの Op# の順、[`MeritFunction::mce`]）。[`terms_from_zmx`] が評価関数から入れる
    pub mce: Vec<(String, i64, i64)>,
    /// 構成の数（ZTHI は構成をまたぐ項で、すべての構成を使う）
    pub configs: usize,
    /// 視野ごとに軸外か（1 始まりの視野 i は `off_axis[i - 1]`。軸外の DENC・DENF に知らせを出す。空なら視野 1 だけ軸上とみなす）
    pub off_axis: Vec<bool>,
}

impl Default for ZmxSys {
    fn default() -> Self {
        ZmxSys {
            glass: Vec::new(),
            symmetric: true,
            primary_varies: false,
            waves: 0,
            angle_field: true,
            mce: Vec::new(),
            configs: 1,
            off_axis: Vec::new(),
        }
    }
}

impl ZmxSys {
    pub fn of(s: &lensbench_core::System) -> ZmxSys {
        ZmxSys {
            glass: s
                .surfaces
                .iter()
                .map(|f| f.mat.is_medium() && !f.mat.is_air())
                .collect(),
            symmetric: s.symmetric(),
            primary_varies: s.configs().is_some_and(|c| {
                c.rows
                    .iter()
                    .any(|r| r.param.kind == lensbench_core::param::Kind::Primary)
            }),
            waves: s.wavelengths.len(),
            angle_field: matches!(s.field_type, lensbench_core::system::FieldType::Angle),
            mce: Vec::new(),
            configs: s.configs().map_or(1, |c| c.n),
            off_axis: s.fields.iter().map(|f| f.x != 0.0 || f.y != 0.0).collect(),
        }
    }
}

/// 行の欄の位置（`名前 Int1 Int2 Data1 Data2 Data3 Data4 目標 重み Data5 Data6`、0 始まりで名前の後から）。
/// OpticStudio 26.1 で実測: 列の Param1〜Param6 の後に目標・重み、Param7・Param8 はその後ろ（ZERN の Vertex で確かめた）
const INT1: usize = 0;
const INT2: usize = 1;
const TARGET: usize = 6;
const WEIGHT: usize = 7;

/// Data k（1 始まり）の欄の位置
fn data(k: usize) -> usize {
    if k <= 4 {
        k + 1
    } else {
        k + 3
    }
}

/// 近軸量の演算子と lensbench の量の名前、長さか（目標をレンズ単位から mm に直す）
const PARAXIAL: &[(&str, &str, bool)] = &[
    ("EFFL", "efl", true),
    ("ENPP", "enpp", true),
    ("EPDI", "epd", true),
    ("EXPD", "expd", true),
    ("EXPP", "expp", true),
    ("ISFN", "isfn", false),
    ("ISNA", "isna", false),
    ("PIMH", "pimh", true),
    ("PMAG", "pmag", false),
    ("WFNO", "wfno", false),
    ("OBSN", "obsn", false),
    ("TOTR", "totr", true),
    ("AMAG", "amag", false),
    ("LINV", "linv", true),
    ("PETZ", "petz", true),
    ("PETC", "petc", false),
    ("WLEN", "wlen", false),
];

/// 1 本の光線の演算子と lensbench の量、長さか
const RAY: &[(&str, &str, bool)] = &[
    ("REAX", "x", true),
    ("REAY", "y", true),
    ("REAZ", "z", true),
    ("REAA", "l", false),
    ("REAB", "m", false),
    ("REAC", "n", false),
    ("TRAX", "trax", true),
    ("TRAY", "tray", true),
    ("TRAR", "trar", true),
    // 面 Surf の TRAR（書き出しは像面なら TRAR）
    ("TRAI", "trar", true),
    ("OPTH", "opth", true),
    ("RAGX", "gx", true),
    ("RAGY", "gy", true),
    ("RAGZ", "gz", true),
    ("RAGA", "gl", false),
    ("RAGB", "gm", false),
    ("RAGC", "gn", false),
    ("REAR", "r", true),
    ("RANG", "ang", false),
    ("RETX", "tanx", false),
    ("RETY", "tany", false),
    ("RENA", "nl", false),
    ("RENB", "nm", false),
    ("RENC", "nn", false),
    ("RAID", "aoi", false),
    ("RAED", "aoe", false),
    ("RAIN", "cosi", false),
    ("RAEN", "cose", false),
    ("ANAX", "anax", false),
    ("ANAY", "anay", false),
    ("ANAR", "anar", false),
    ("DXDX", "dxdx", true),
    ("DXDY", "dxdy", true),
    ("DYDX", "dydx", true),
    ("DYDY", "dydy", true),
    ("FCGS", "fcgs", true),
    ("FCGT", "fcgt", true),
];

/// 像面だけの光線の演算子（Int1 は使わない）
fn image_only(op: &str) -> bool {
    op != "TRAI"
        && ["TRA", "ANA", "DXD", "DYD", "FCG"]
            .iter()
            .any(|p| op.starts_with(p))
}

/// 近軸光線の演算子と ray の項の量、長さか（Surf・Wave・Hx・Hy・Px・Py は RAY と同じ欄）
const PAR: &[(&str, &str, bool)] = &[
    ("PARX", "x", true),
    ("PARY", "y", true),
    ("PARZ", "z", true),
    ("PARA", "l", false),
    ("PARB", "m", false),
    ("PARC", "n", false),
    ("PARR", "r", true),
    ("PANA", "nl", false),
    ("PANB", "nm", false),
    ("PANC", "nn", false),
    ("PATX", "tanx", false),
    ("PATY", "tany", false),
];

/// 標本の番号（1 が 32 × 32、1 つ増すごとに 2 倍）から瞳の標本数
fn sampling(s: i64) -> Result<usize, String> {
    if !(1..=6).contains(&s) {
        return Err(format!("sampling {s} (1 to 6)"));
    }
    Ok(32 << (s - 1))
}

/// 視野の番号（1 始まり）
fn field_no(f: i64) -> Result<usize, String> {
    if f < 1 {
        return Err(format!("field {f}"));
    }
    Ok(f as usize)
}

/// 読めるが値が OpticStudio と違うと分かっている行の注意（OpticStudio 26.1 の実測）
fn read_differs(r: &MeritRow, sys: &ZmxSys) -> Option<&'static str> {
    let num = |i: usize| r.num(i).unwrap_or(0.0);
    match r.op.as_str() {
        "DENC" | "DENF" if off_axis(sys, num(data(1))) => Some(
            "OpticStudio measures the encircled energy of an off-axis field on the plane normal to the chief ray, lensbench on the image plane (about 1% apart in the check); the term may differ",
        ),
        "RSRH" if num(data(1)) != 0.0 && num(data(2)) != 0.0 => Some(
            "the value of OpticStudio for a field with both Hx and Hy not 0 could not be reproduced; the term may differ",
        ),
        "ANAX" | "ANAY" if num(INT2) == 0.0 => {
            Some("Wave 0 is read as the primary wavelength (OpticStudio returns 0)")
        }
        "FCGT" if num(data(2)) != 0.0 => Some(FCG_DIFF),
        "FCGS" if num(data(1)) != 0.0 => Some(FCG_DIFF),
        "GENC" | "GENF" if num(data(4)) == 2.0 => Some(
            "the value of OpticStudio about the vertex could not be reproduced (about 0.6% larger in the check); the term may differ",
        ),
        _ => None,
    }
}

/// 視野 f（1 始まり、.zmx の Field の欄）が軸外か
fn off_axis(sys: &ZmxSys, f: f64) -> bool {
    let i = f as usize;
    match sys.off_axis.get(i.wrapping_sub(1)) {
        Some(&o) => o,
        None => i > 1,
    }
}

const FCG_DIFF: &str =
    "lensbench uses a central difference; OpticStudio's one-sided difference differs by about 1e-4";

/// 行を写す途中の状態
struct Ctx<'a> {
    unit: f64,
    primary: usize,
    image: usize,
    sys: &'a ZmxSys,
    rows: &'a [MeritRow],
    notes: Vec<String>,
}

impl Ctx<'_> {
    fn skip(&mut self, r: &MeritRow, why: &str) {
        self.notes.push(format!(
            ".zmx merit function row {} ({}, line {}): {why}; skipped",
            r.row, r.op, r.line
        ));
    }
}

/// 1 つの行の項（重み・構成・目標の向きを除く種類のキーと目標）。写せなければ Err に理由
fn kind_of(
    r: &MeritRow,
    unit: f64,
    primary: usize,
    image: usize,
    sys: &ZmxSys,
) -> Result<Map<String, Value>, String> {
    let num = |i: usize| r.num(i).map_err(|e| e.to_string());
    let int = |i: usize| r.int(i).map_err(|e| e.to_string());
    let mut m = Map::new();
    let target = num(TARGET)?;
    let wave = |w: i64| -> Result<Value, String> {
        match w {
            0 => Ok(Value::Null),
            w if w > 0 => Ok(json!(w)),
            w => Err(format!("wavelength {w}")),
        }
    };
    let norm_field = || -> Result<Value, String> { Ok(json!([num(data(1))?, num(data(2))?])) };
    let op = r.op.as_str();
    if let Some((_, q, len)) = PARAXIAL.iter().find(|(o, _, _)| *o == op) {
        // EFFL・PIMH・PMAG などは Int2 が波長（0 は主波長）。EFFL・PIMH はその波長の近軸値（OpticStudio 26.1 で実測）。
        // ほかの量の波長は確かめていないので主波長だけ読む
        let w = int(INT2)?;
        m.insert("type".into(), json!("paraxial"));
        m.insert("q".into(), json!(q));
        let reads = matches!(*q, "efl" | "pimh" | "amag" | "linv" | "petz" | "petc");
        if *q == "wlen" {
            // WLEN の Wave 0 は波長 1（OpticStudio 26.1 で実測）
            m.insert("wave".into(), json!(w.max(1)));
        } else if w != 0 && w as usize != primary {
            if !reads || w < 0 {
                return Err(format!(
                    "wavelength {w} (lensbench reads the wavelength only for EFFL, PIMH, AMAG, LINV, PETZ and PETC)"
                ));
            }
            m.insert("wave".into(), json!(w));
        } else if w != 0 && reads && sys.primary_varies {
            m.insert("wave".into(), json!(w));
        }
        m.insert(
            "target".into(),
            json!(if *len { target * unit } else { target }),
        );
        return Ok(m);
    }
    let par = PAR.iter().find(|(o, _, _)| *o == op);
    if let Some((_, q, len)) = RAY.iter().find(|(o, _, _)| *o == op).or(par) {
        m.insert("type".into(), json!("ray"));
        if par.is_some() {
            m.insert("paraxial".into(), json!(true));
        }
        // TRAX・TRAY・TRAR・ANA*・DXDX〜DYDY・FCGS・FCGT は像面（Int1 は使わない）
        if !image_only(op) {
            let j = int(INT1)?;
            if op == "TRAI" && j < 1 {
                return Err(format!(
                    "surface {j} (TRAI of surface 0 is not the image surface)"
                ));
            }
            m.insert("surf".into(), json!(j));
        }
        m.insert("wave".into(), wave(int(INT2)?)?);
        m.insert("field".into(), norm_field()?);
        m.insert("px".into(), json!(num(data(3))?));
        m.insert("py".into(), json!(num(data(4))?));
        m.insert("q".into(), json!(q));
        m.insert(
            "target".into(),
            json!(if *len { target * unit } else { target }),
        );
        return Ok(m);
    }
    match op {
        // EFLX・EFLY: 面 Surf1 から Surf2 までの焦点距離（主波長）。CARD: 同じ範囲の基点（Wave は Data1、Orientation は
        // Data2、Data は Data3）。EFLA: 面 Surf と Surf + 1 の単レンズの焦点距離（波長 Int2）。OpticStudio 26.1 で実測
        "EFLX" | "EFLY" | "CARD" | "EFLA" => {
            let (s1, s2) = if op == "EFLA" {
                let j = int(INT1)?;
                (j, j + 1)
            } else {
                (int(INT1)?, int(INT2)?)
            };
            if s1 < 1 || s2 < s1 || s2 as usize >= image {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            let x = match op {
                "EFLX" => true,
                "CARD" => match int(data(2))? {
                    0 => false,
                    1 => true,
                    o => return Err(format!("orientation {o}")),
                },
                _ => false,
            };
            if x && !sys.symmetric {
                return Err(
                    "the X orientation of a system that is not rotationally symmetric (lensbench's paraxial trace is in the Y-Z plane)"
                        .into(),
                );
            }
            if op == "EFLA" && !sys.glass.is_empty() {
                let g = |j: i64| sys.glass.get(j as usize).copied().unwrap_or(false);
                if !(!g(s1 - 1) && g(s1) && !g(s2)) {
                    return Err(format!(
                        "surfaces {s1} and {s2} are not a single lens in air (OpticStudio gives 1e8)"
                    ));
                }
            }
            let (q, w) = match op {
                "CARD" => {
                    let d = int(data(3))?;
                    let q = usize::try_from(d)
                        .ok()
                        .and_then(lensbench_optim::CardQ::from_card_data)
                        .ok_or(format!("data {d} (0 to 11)"))?;
                    (q.name(), int(data(1))?)
                }
                "EFLA" => ("efl", int(INT2)?),
                _ => ("efl", 0),
            };
            m.insert("type".into(), json!("cardinal"));
            m.insert("q".into(), json!(q));
            m.insert("from".into(), json!(s1));
            m.insert("to".into(), json!(s2));
            m.insert("wave".into(), wave(w)?);
            m.insert("target".into(), json!(target * unit));
        }
        // PLEN: 面 Int1 から Int2 までの光路長（主波長）。MXRE・MNRE・MXRI・MNRI: その面ごとの射出角・入射角 [度] の上限・下限
        "PLEN" | "MXRE" | "MNRE" | "MXRI" | "MNRI" => {
            let (s1, s2) = (int(INT1)?, int(INT2)?);
            if s1 < i64::from(op != "PLEN") || s2 < s1 || s2 as usize > image {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            m.insert("type".into(), json!("ray"));
            m.insert("surf".into(), json!(s1));
            m.insert("to".into(), json!(s2));
            if op != "PLEN" {
                m.insert("wave".into(), wave(int(data(5))?)?);
            }
            m.insert("field".into(), norm_field()?);
            m.insert("px".into(), json!(num(data(3))?));
            m.insert("py".into(), json!(num(data(4))?));
            let (q, key, t) = match op {
                "PLEN" => ("opth", "target", target * unit),
                _ => (
                    if op.ends_with('E') { "aoe" } else { "aoi" },
                    if op.starts_with("MX") { "max" } else { "min" },
                    target,
                ),
            };
            m.insert("q".into(), json!(q));
            m.insert(key.into(), json!(t));
        }
        // CTGT・CTLT・CTVA: Int1 の面の中心の厚み ≥・≤・＝ 目標。TTHI: Int1 から Int2 の面の厚みの和
        "CTGT" | "CTLT" | "CTVA" | "TTHI" => {
            let s1 = int(INT1)?;
            let s2 = if op == "TTHI" { int(INT2)? } else { s1 };
            if s1 < 1 || s2 < s1 || s2 as usize >= image {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            m.insert("type".into(), json!("thickness"));
            m.insert("surf".into(), json!(s1));
            if s2 != s1 {
                m.insert("to".into(), json!(s2));
            }
            let key = match op {
                "CTGT" => "min",
                "CTLT" => "max",
                _ => "target",
            };
            m.insert(key.into(), json!(target * unit));
        }
        // ETGT・ETLT・ETVA: Int1 の面の縁の厚み ≥・≤・＝ 目標。Int2 の Code は測る向き（0 +y・1 +x・2 −y・3 −x、面の局所座標）。
        // Data2 の Mode 0 は矢高を面の形が終わる半径（クリア半径 + チップゾーン）で、1 はクリア半径で測る（OpticStudio 26.1 で実測）
        "ETGT" | "ETLT" | "ETVA" => {
            let j = int(INT1)?;
            if j < 1 || j as usize >= image {
                return Err(format!("surface {j}"));
            }
            m.insert("type".into(), json!("thickness"));
            m.insert("surf".into(), json!(j));
            m.insert("edge".into(), json!(true));
            match int(INT2)? {
                0 => {}
                1 => {
                    m.insert("axis".into(), json!("+x"));
                }
                2 => {
                    m.insert("axis".into(), json!("-y"));
                }
                3 => {
                    m.insert("axis".into(), json!("-x"));
                }
                c => return Err(format!("code {c} (0 to 3)")),
            }
            match int(data(2))? {
                0 => {}
                1 => {
                    m.insert("clear".into(), json!(true));
                }
                k => return Err(format!("mode {k} (0 or 1)")),
            }
            let key = match op {
                "ETGT" => "min",
                "ETLT" => "max",
                _ => "target",
            };
            m.insert(key.into(), json!(target * unit));
        }
        // 範囲の境界: Int1〜Int2 の面（像面は除く）のうち T すべて・A 空気・G ガラスの面の中心（C）・縁（E）の厚みを
        // 1 面ずつ目標と比べる。縁は Data1 の Zone（0 は半直径、ほかは半直径に対する割合）
        "MNCT" | "MXCT" | "MNCA" | "MXCA" | "MNCG" | "MXCG" | "MNET" | "MXET" | "MNEA" | "MXEA"
        | "MNEG" | "MXEG" => {
            let s1 = int(INT1)?;
            let s2 = int(INT2)?.min(image as i64 - 1);
            if s1 < 1 || s2 < s1 {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            m.insert("type".into(), json!("thickness"));
            m.insert("surf".into(), json!(s1));
            m.insert("to".into(), json!(s2));
            if &op[2..3] == "E" {
                m.insert("edge".into(), json!(true));
                let z = num(data(1))?;
                if z != 0.0 {
                    m.insert("zone".into(), json!(z));
                }
            }
            let each = match &op[3..] {
                "A" => "air",
                "G" => "glass",
                _ => "all",
            };
            m.insert("each".into(), json!(each));
            let key = if op.starts_with("MN") { "min" } else { "max" };
            m.insert(key.into(), json!(target * unit));
        }
        "TTGT" | "TTLT" | "TTVA" | "PMGT" | "PMLT" | "PMVA" | "VOLU" | "TMAS" | "CVOL" | "TSAG"
        | "BFSD" => return shape::kind(r, unit, image),
        // 面の量: CV 曲率・CO コーニック・DM 径（Mode 0 は玉の径）の ≥・≤・＝、矢高・点の厚み・大域座標
        "CVGT" | "CVLT" | "CVVA" | "COGT" | "COLT" | "COVA" | "DMGT" | "DMLT" | "DMVA" | "SAGX"
        | "SAGY" | "SSAG" | "STHI" | "GLCX" | "GLCY" | "GLCZ" | "GLCA" | "GLCB" | "GLCC"
        | "GCOS" | "GTCE" | "TCVA" | "TCGT" | "TCLT" => {
            let j = int(INT1)?;
            let last = if op == "STHI" { image - 1 } else { image };
            if j < 0 || j as usize > last {
                return Err(format!("surface {j}"));
            }
            let (q, xy) = match op {
                "CVGT" | "CVLT" | "CVVA" => ("curv", None),
                "COGT" | "COLT" | "COVA" => ("conic", None),
                "DMGT" | "DMLT" | "DMVA" if int(data(2))? == 0 => ("mech_diam", None),
                "DMGT" | "DMLT" | "DMVA" => ("diam", None),
                "SAGX" => ("sagx", None),
                "SAGY" => ("sagy", None),
                // SSAG: Data1・Data2 が点、Data3〜Data5（Off-Axis・Remove・BFS）は 0 だけ
                "SSAG" => {
                    if (3..=5).any(|k| num(data(k)).unwrap_or(0.0) != 0.0) {
                        return Err("SSAG with Off-Axis, Remove or BFS".into());
                    }
                    ("sag", Some((num(data(1))?, num(data(2))?)))
                }
                // STHI: Data1・Data2 が点（Data3 の Mode は回転対称の系で 0・1 とも同じ値）
                "STHI" => ("thick", Some((num(data(1))?, num(data(2))?))),
                "GLCX" => ("gx", None),
                "GLCY" => ("gy", None),
                "GLCZ" => ("gz", None),
                "GLCA" => ("ga", None),
                "GLCB" => ("gb", None),
                "GLCC" => ("gc", None),
                // 面の後のガラスの相対価格・熱膨張係数（カタログの OD・ED の行、空気は 0。OpticStudio 26.1 で実測）
                "GCOS" => ("cost", None),
                "GTCE" => ("tce", None),
                // 面の TCE（レンズデータの列。硝材の面でも面の値で、OpticStudio 26.1 の実測で硝材の面は 0）
                _ => ("surf_tce", None),
            };
            m.insert("type".into(), json!("surface"));
            m.insert("surf".into(), json!(j));
            m.insert("q".into(), json!(q));
            if let Some((x, y)) = xy {
                m.insert("x".into(), json!(x * unit));
                m.insert("y".into(), json!(y * unit));
            }
            let key = match &op[2..] {
                "GT" => "min",
                "LT" => "max",
                _ => "target",
            };
            let len = !matches!(
                q,
                "curv" | "conic" | "ga" | "gb" | "gc" | "cost" | "tce" | "surf_tce"
            );
            // 曲率は 1/レンズ単位
            let t = match q {
                "curv" => target / unit,
                _ if len => target * unit,
                _ => target,
            };
            m.insert(key.into(), json!(t));
        }
        // POWR・YNIP・INDX: 面 Int1、波長 Int2（INDX の 0 は波長 1。OpticStudio 26.1 の実測）
        "POWR" | "YNIP" | "INDX" => {
            let j = int(INT1)?;
            if j < i64::from(op == "YNIP") || j as usize > image {
                return Err(format!("surface {j}"));
            }
            let w = match int(INT2)? {
                0 if op == "INDX" => 1,
                w => w,
            };
            m.insert("type".into(), json!("surface"));
            m.insert("surf".into(), json!(j));
            m.insert(
                "q".into(),
                json!(match op {
                    "POWR" => "power",
                    "YNIP" => "ynip",
                    _ => "index",
                }),
            );
            m.insert("wave".into(), wave(w)?);
            let t = match op {
                "POWR" => target / unit,
                "YNIP" => target * unit,
                _ => target,
            };
            m.insert("target".into(), json!(t));
        }
        "RSCE" | "RSCH" | "RWCE" | "RWCH" => {
            if target != 0.0 {
                return Err(format!(
                    "a target other than 0 ({target}) can't be written as a spot or wavefront term"
                ));
            }
            let spot = op.starts_with("RS");
            m.insert(
                "type".into(),
                json!(if spot { "spot" } else { "wavefront" }),
            );
            m.insert("rings".into(), json!(int(INT1)?.max(1)));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), norm_field()?);
            m.insert(
                "reference".into(),
                json!(if op.ends_with('E') {
                    "centroid"
                } else {
                    "chief"
                }),
            );
            m.insert("quadrature".into(), json!("opticstudio"));
        }
        // RSRE・RSRH・RWRE・RWRH: 瞳の 4 分の 1 ごとに Samp × Samp の格子（1 辺 2·Samp の格子の項。OpticStudio 26.1 で実測）
        "RSRE" | "RSRH" | "RWRE" | "RWRH" => {
            if target != 0.0 {
                return Err(format!(
                    "a target other than 0 ({target}) can't be written as a spot or wavefront term"
                ));
            }
            let n = int(INT1)?;
            if n < 1 {
                return Err(format!("sampling {n} (1 or more)"));
            }
            m.insert(
                "type".into(),
                json!(if op.starts_with("RS") {
                    "spot"
                } else {
                    "wavefront"
                }),
            );
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), norm_field()?);
            m.insert(
                "reference".into(),
                json!(if op.ends_with('E') {
                    "centroid"
                } else {
                    "chief"
                }),
            );
            m.insert("quadrature".into(), json!("grid"));
            m.insert("n".into(), json!(2 * n));
        }
        // SPCH: 帯 Zone の実光線の軸上色収差 − 近軸の軸上色収差（Minw から Maxw、0 は最初・最後の波長）。2 つの
        // longitudinal の項の差の calc にする（OpticStudio 26.1 で実測）
        "SPCH" => {
            let z = num(data(1))?;
            if !(0.0..=1.0).contains(&z) {
                return Err(format!("zone {z} (0 to 1)"));
            }
            let w1 = match int(INT1)? {
                0 => 1,
                w => w,
            };
            let w2 = match int(INT2)? {
                0 if sys.waves == 0 => return Err("Maxw 0 needs the number of wavelengths".into()),
                0 => sys.waves as i64,
                w => w,
            };
            let lona = |zone: f64| -> Result<Value, String> {
                Ok(
                    json!({"type": "longitudinal", "wave": wave(w1)?, "wave2": wave(w2)?, "zone": zone, "target": 0.0}),
                )
            };
            m.insert("type".into(), json!("calc"));
            m.insert("op".into(), json!("diff"));
            m.insert("args".into(), json!([lona(z)?, lona(0.0)?]));
            m.insert("target".into(), json!(target * unit));
        }
        "OPDC" => {
            m.insert("type".into(), json!("opd"));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), norm_field()?);
            m.insert("px".into(), json!(num(data(3))?));
            m.insert("py".into(), json!(num(data(4))?));
            m.insert("target".into(), json!(target));
        }
        // MTF*・MSW*・GMT*・MTH*（末尾の T・S・A が子午・球欠・平均）: Int1 標本（1 が 32 × 32、1 つ増すごとに 2 倍）、
        // Int2 波長、Data1 視野の番号、Data2 周波数 [cycles/mm]。MTF* の Data3 Grid と GMT* の Data4 Grid は求め方の違い
        // （0 は疎な積分、1 は MTF の解析の格子）で、lensbench は同じ値（収束した値）を返す。MTF* の Data4 Data Type は 0
        // （変調）だけ。GMT* の Data3（!Scl）は 0 で回折限界の MTF をかける。MTH* の Data3 偏光・Data4 全構成は 0、Data5 像の
        // 間隔は 0（既定）だけ
        "MTFT" | "MTFS" | "MTFA" | "MSWT" | "MSWS" | "MSWA" | "GMTT" | "GMTS" | "GMTA" | "MTHT"
        | "MTHS" | "MTHA" => {
            let kind = match &op[..3] {
                "MTF" => {
                    if !matches!(int(data(3))?, 0 | 1) || int(data(4))? != 0 {
                        return Err(
                            "only the MTF modulation (Grid 0 or 1, Data Type 0) is supported"
                                .into(),
                        );
                    }
                    "diffraction"
                }
                "MSW" => "square",
                "GMT" => {
                    if !matches!(int(data(4))?, 0 | 1) {
                        return Err(format!("Grid {}", int(data(4))?));
                    }
                    m.insert("diff_limit".into(), json!(int(data(3))? == 0));
                    "geometric"
                }
                _ => {
                    if int(data(3))? != 0 || int(data(4))? != 0 || num(data(5))? != 0.0 {
                        return Err("only the Huygens MTF without polarization, of the current configuration, with the default image delta is supported".into());
                    }
                    "huygens"
                }
            };
            m.insert("type".into(), json!("mtf"));
            if kind != "diffraction" {
                m.insert("kind".into(), json!(kind));
            }
            sampling(int(INT1)?)?;
            m.insert("wave".into(), wave(int(INT2)?)?);
            let f = int(data(1))?;
            if f == 0 {
                return Err("field 0 (the diffraction limit) is not supported".into());
            }
            m.insert("field".into(), json!(field_no(f)?));
            if unit != 1.0 {
                return Err("MTF frequencies in lens units other than mm are not checked".into());
            }
            m.insert("freq".into(), json!(num(data(2))?));
            let dir = match op.as_bytes()[3] {
                b'T' => "tan",
                b'S' => "sag",
                _ => "avg",
            };
            m.insert("dir".into(), json!(dir));
            m.insert("target".into(), json!(target));
        }
        // GENC・GENF（幾何）・DENC・DENF（回折）: Int1 標本、Int2 波長（0 は多色）、Data1 視野の番号、Data2 割合（*NC）か
        // 大きさ [µm]（*NF）、Data3 形（1 円・2 x・3 y・4 正方形）、Data4 中心（GEN*: 0 主光線・1 重心・2 原点・3 広がりの中心、
        // DEN*: 0〜2 が FFT の主光線・重心・原点、3〜5 が Huygens の同じ順）。GEN* の Data5（No Diff Lim）が 0 なら円でエアリー像の
        // 割合をかける（OpticStudio 26.1 は円のほかの形ではかけない）。DEN* の Data5・Data6（像の標本・間隔）と標本は読まず、
        // lensbench の既定（収束した値）にする。大きさは µm（OpticStudio の値と同じ）を mm にする
        "GENC" | "GENF" | "DENC" | "DENF" => {
            let shape = match int(data(3))? {
                1 => "circle",
                2 => "x",
                3 => "y",
                4 => "square",
                k => return Err(format!("Type {k} (1 to 4)")),
            };
            let refp = int(data(4))?;
            let (method, center) = match (&op[..3], refp) {
                ("GEN", 0..=3) => ("geometric", refp),
                ("DEN", 0..=2) => ("fft", refp),
                ("DEN", 3..=5) => ("huygens", refp - 3),
                _ => return Err(format!("Refp {refp}")),
            };
            m.insert("type".into(), json!("energy"));
            m.insert("method".into(), json!(method));
            if method == "geometric" && shape == "circle" && int(data(5))? == 0 {
                m.insert("airy".into(), json!(true));
            }
            m.insert("shape".into(), json!(shape));
            m.insert(
                "center".into(),
                json!(["chief", "centroid", "vertex", "middle"][center as usize]),
            );
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), json!(field_no(int(data(1))?)?));
            let v = num(data(2))?;
            if op.ends_with('C') {
                m.insert("size_at".into(), json!(v));
                m.insert("target".into(), json!(target * 1e-3));
            } else {
                m.insert("fraction_at".into(), json!(v * 1e-3));
                m.insert("target".into(), json!(target));
            }
        }
        // STRH: Int1 標本、Int2 波長（0 は多色）、Data1 視野の番号、Data3 偏光・Data4 全構成は 0 だけ。標本は読まず lensbench の
        // 既定（自動の瞳の格子の FFT の PSF）
        "STRH" => {
            if int(data(3))? != 0 || int(data(4))? != 0 {
                return Err("only the Strehl ratio without polarization, of the current configuration, is supported".into());
            }
            m.insert("type".into(), json!("strehl"));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), json!(field_no(int(data(1))?)?));
            m.insert("target".into(), json!(target));
        }
        // RELI・EFNO: Int1 標本、Int2 波長（0 は波長 1。OpticStudio 26.1 で実測）、Data1 視野の番号、Data2 偏光（0 だけ）。
        // 標本は読まず lensbench の既定（瞳の縁 128 点）
        // MCOV・MCOG・MCOL: Int1 の多重構成の行（Op#）の、構成 Int2（Cfg#）の値。lensbench は構成 Cfg# の系のパラメータの
        // param の項（MCOG・MCOL は min・max）
        "MCOV" | "MCOG" | "MCOL" => {
            let k = int(INT1)?;
            let c = int(INT2)?;
            let Some((mop, j, parm)) = (k >= 1).then(|| sys.mce.get(k as usize - 1)).flatten()
            else {
                return Err(format!("multi-configuration row {k}"));
            };
            if c < 1 {
                return Err(format!("configuration {c}"));
            }
            let (name, scale) = match mop.as_str() {
                "THIC" => (format!("{j}.t"), unit),
                "SDIA" => (format!("{j}.semi"), unit),
                "CRVT" => (format!("{j}.c"), 1.0 / unit),
                "CONN" => (format!("{j}.k"), 1.0),
                "PRAM" => (format!("{j}.p{parm}"), 1.0),
                "WAVE" => (format!("wave{j}"), 1.0),
                "WLWT" => (format!("wwt{j}"), 1.0),
                "PRWV" => ("primary".to_string(), 1.0),
                "APER" => ("aper".to_string(), 1.0),
                "XFIE" => (format!("fx{j}"), 1.0),
                "YFIE" => (format!("fy{j}"), 1.0),
                "FLWT" => (format!("fwt{j}"), 1.0),
                "FVDX" => (format!("vdx{j}"), 1.0),
                "FVDY" => (format!("vdy{j}"), 1.0),
                "FVCX" => (format!("vcx{j}"), 1.0),
                "FVCY" => (format!("vcy{j}"), 1.0),
                "FVAN" | "FTAN" => (format!("van{j}"), 1.0),
                "SATP" => ("atype".to_string(), 1.0),
                "STPS" => ("stop".to_string(), 1.0),
                "IGNR" => (format!("{j}.ignore"), 1.0),
                "APMX" => (format!("{j}.apmax"), unit),
                "APMN" => (format!("{j}.apmin"), unit),
                "APDX" => (format!("{j}.apdx"), unit),
                "APDY" => (format!("{j}.apdy"), unit),
                "APTP" => (format!("{j}.aptype"), 1.0),
                "CBOR" => (format!("{j}.beforeorder"), 1.0),
                "CAOR" => (format!("{j}.afterorder"), 1.0),
                "MIND" => (format!("{j}.nd"), 1.0),
                "MABB" => (format!("{j}.vd"), 1.0),
                "MDPG" => (format!("{j}.dpgf"), 1.0),
                "TCEX" => (format!("{j}.tce"), 1.0),
                "MCSD" => (format!("{j}.mech"), unit),
                "CHZN" => (format!("{j}.chip"), unit),
                "CSP1" | "CSP2" | "TSP1" | "TSP2" | "TSP3" => {
                    (format!("{j}.{}", mop.to_ascii_lowercase()), 1.0)
                }
                "PSP1" | "PSP2" | "PSP3" => {
                    (format!("{j}.{}_{parm}", mop.to_ascii_lowercase()), 1.0)
                }
                "CBDX" | "CBDY" | "CADX" | "CADY" => (
                    format!(
                        "{j}.{}{}",
                        if mop.starts_with("CB") {
                            "before"
                        } else {
                            "after"
                        },
                        u8::from(mop.ends_with('Y'))
                    ),
                    unit,
                ),
                "CBTX" | "CBTY" | "CBTZ" | "CATX" | "CATY" | "CATZ" => (
                    format!(
                        "{j}.{}{}",
                        if mop.starts_with("CB") {
                            "before"
                        } else {
                            "after"
                        },
                        2 + "XYZ".find(&mop[3..]).unwrap_or(0)
                    ),
                    1.0,
                ),
                "TEMP" => ("temp".to_string(), 1.0),
                "PRES" => ("pres".to_string(), 1.0),
                "AFOC" => ("afocal".to_string(), 1.0),
                "APDT" => ("apodtype".to_string(), 1.0),
                "APDF" => ("apod".to_string(), 1.0),
                "RAAM" => ("aiming".to_string(), 1.0),
                "OPDR" => ("ropd".to_string(), 1.0),
                "PSHX" | "PSHY" | "PSHZ" => (mop.to_ascii_lowercase(), unit),
                "PSCX" | "PSCY" => (mop.to_ascii_lowercase(), 1.0),
                o => return Err(format!("the multi-configuration operand {o} of row {k}")),
            };
            m.insert("type".into(), json!("param"));
            m.insert("name".into(), json!(name));
            m.insert("config".into(), json!(c));
            let key = match op {
                "MCOG" => "min",
                "MCOL" => "max",
                _ => "target",
            };
            m.insert(key.into(), json!(target * scale));
        }
        // ZTHI: Int1 から Int2 の面（像面は除く）の厚みの和の、すべての構成での最大 − 最小 ≤ 目標。lensbench は厚みの項の
        // spread の項の max（OpticStudio 26.1 で実測: 1 つの構成なら 0、Int2 の像面は飛ばす）
        "ZTHI" => {
            let s1 = int(INT1)?;
            let s2 = int(INT2)?.min(image as i64 - 1);
            if s1 < 1 || s2 < s1 {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            let mut t = json!({"type": "thickness", "surf": s1});
            if s2 != s1 {
                t["to"] = json!(s2);
            }
            m.insert("type".into(), json!("spread"));
            m.insert("term".into(), t);
            m.insert("max".into(), json!(target * unit));
        }
        "RELI" | "EFNO" => {
            if int(data(2))? != 0 {
                return Err("the illumination with polarization is not supported".into());
            }
            m.insert("type".into(), json!("illum"));
            m.insert(
                "q".into(),
                json!(if op == "RELI" { "relative" } else { "fno" }),
            );
            m.insert("wave".into(), wave(int(INT2)?.max(1))?);
            m.insert("field".into(), json!(field_no(int(data(1))?)?));
            m.insert("target".into(), json!(target));
        }
        // ZERN: Int1 項、Int2 波長、Data1 標本、Data2 視野の番号、Data3 種類（0 Fringe・1 Standard）、Data4 Epsilon
        // （輪帯）、Data5 Vertex
        "ZERN" => {
            let kind = match int(data(3))? {
                0 => "fringe",
                1 => "standard",
                k => return Err(format!("Zernike type {k} (only Fringe 0 and Standard 1)")),
            };
            if num(data(4))? != 0.0 || int(data(5))? != 0 {
                return Err("annular Zernike (Epsilon) and Vertex are not supported".into());
            }
            let term = int(INT1)?;
            if term < 1 {
                return Err(format!("Zernike term {term}"));
            }
            m.insert("type".into(), json!("zernike"));
            m.insert("term".into(), json!(term));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("n".into(), json!(sampling(int(data(1))?)?));
            m.insert("field".into(), json!(field_no(int(data(2))?)?));
            m.insert("kind".into(), json!(kind));
            // 当てはめる項の数は Fringe がいつも 37（OpticStudio 26.1）。Standard は隣の ZERN の行で決まるので terms_from_zmx で入れる
            if kind == "fringe" {
                m.insert("fit".into(), json!(37));
            }
            m.insert("target".into(), json!(target));
        }
        // CENX・CENY: Int1 面、Int2 波長、Data1 視野の番号、Data2 偏光、Data3 標本（格子）。lensbench の重心は像面だけで、
        // 光線はスポットの項と同じガウス求積（格子の標本とは近似の仕方が違う）
        // SPHA・COMA・ASTI・FCUR・DIST: 面 Int1（0 は和）・波長 Int2 のザイデル収差係数 [波]。DIST の Data1（Absolute）が
        // 0 でなければ最大の視野の主光線の実像高 − 近軸像高、Surf 0 なら近軸像高に対する %（OpticStudio 26.1 で実測）
        "SPHA" | "COMA" | "ASTI" | "FCUR" | "DIST" => {
            let j = int(INT1)?;
            if j < 0 || j as usize > image {
                return Err(format!("surface {j}"));
            }
            let abs = if op == "DIST" {
                num(data(1))? != 0.0
            } else {
                false
            };
            if op == "DIST" && (j == 0 || abs) {
                m.insert("type".into(), json!("distortion"));
                m.insert("wave".into(), wave(int(INT2)?)?);
                m.insert("absolute".into(), json!(abs));
                m.insert(
                    "target".into(),
                    json!(if abs { target * unit } else { target }),
                );
            } else {
                m.insert("type".into(), json!("seidel"));
                m.insert("q".into(), json!(op.to_ascii_lowercase()));
                m.insert("surf".into(), json!(j));
                m.insert("wave".into(), wave(int(INT2)?)?);
                m.insert("target".into(), json!(target));
            }
        }
        // DIMX: 視野 Int1（0 は最大の視野）までの歪曲の絶対値の最大、波長 Int2、Data1 の Absolute。DISC: 波長 Int2・
        // Absolute Data1 の校正した f-θ 歪曲。DISG: 基準の視野 Int1（0 は軸上）、波長 Int2（負なら長さ）、Hx・Hy の
        // 主光線の一般の歪曲（Px・Py は値を変えない）。OpticStudio 26.1 で実測
        "DIMX" | "DISC" | "DISG" => {
            m.insert("type".into(), json!("distortion"));
            let w = int(INT2)?;
            let abs = if op == "DISG" {
                w < 0
            } else {
                num(data(1))? != 0.0
            };
            m.insert("wave".into(), wave(w.abs())?);
            m.insert("absolute".into(), json!(abs));
            match op {
                "DIMX" => {
                    let f = int(INT1)?;
                    if f != 0 {
                        m.insert("field".into(), json!(field_no(f)?));
                    }
                    m.insert("peak".into(), json!(true));
                }
                "DISC" => {
                    m.insert("ftheta".into(), json!(true));
                }
                _ => {
                    m.insert("field".into(), norm_field()?);
                    let f = int(INT1)?;
                    if f != 0 {
                        m.insert("reference".into(), json!(field_no(f)?));
                    }
                }
            }
            m.insert(
                "target".into(),
                json!(if abs { target * unit } else { target }),
            );
        }
        // ABCD: 基準の視野 Int1 でのグリッド歪曲の係数（Data1 の 0〜3 が A〜D）、波長 Int2。DISA: 基準 Int1・波長 Int2・
        // 向き Data1（0 動径・1 x・2 y）・視野 Data2 の主光線の歪曲 [%] を、Data3〜Data6 の A〜D で予測して。OpticStudio 26.1 で実測
        "ABCD" | "DISA" => {
            m.insert("type".into(), json!("distortion"));
            m.insert("wave".into(), wave(int(INT2)?)?);
            let f = int(INT1)?;
            if f != 0 {
                m.insert("reference".into(), json!(field_no(f)?));
            }
            let k = int(data(1))?;
            if op == "ABCD" {
                let c = ["a", "b", "c", "d"]
                    .get(k as usize)
                    .filter(|_| k >= 0)
                    .ok_or(format!("data {k} (0 to 3)"))?;
                m.insert("coef".into(), json!(c));
                let u = if sys.angle_field { unit } else { 1.0 };
                m.insert("target".into(), json!(target * u));
            } else {
                let c = ["radial", "x", "y"]
                    .get(k as usize)
                    .filter(|_| k >= 0)
                    .ok_or(format!("data {k} (0 to 2)"))?;
                m.insert("field".into(), json!(field_no(int(data(2))?)?));
                let abcd = [num(data(3))?, num(data(4))?, num(data(5))?, num(data(6))?];
                let u = if sys.angle_field { unit } else { 1.0 };
                m.insert("abcd".into(), json!(abcd.map(|v| v * u)));
                m.insert("component".into(), json!(c));
                m.insert("target".into(), json!(target));
            }
        }
        // BSER: 軸上の視野の主光線の像面での動径 / 有効焦点距離（波長 Int2）。ray の r と paraxial の efl の比の calc
        "BSER" => {
            m.insert("type".into(), json!("calc"));
            m.insert("op".into(), json!("div"));
            let r = json!({"type": "ray", "wave": wave(int(INT2)?)?, "field": [0.0, 0.0], "px": 0.0, "py": 0.0, "q": "r", "target": 0.0});
            m.insert(
                "args".into(),
                json!([r, {"type": "paraxial", "q": "efl", "target": 0.0}]),
            );
            m.insert("target".into(), json!(target));
        }
        // LONA: 波長 Int1・帯 Data1。AXCL: 波長 Int1・Int2・帯 Data1（Int2 の距離 − Int1 の距離）。LACL: 波長 Int1・Int2
        // （Int2 の主光線の高さ − Int1）。OpticStudio 26.1 で実測
        "LONA" | "AXCL" => {
            let z = num(data(1))?;
            if !(0.0..=1.0).contains(&z) {
                return Err(format!("zone {z} (0 to 1)"));
            }
            m.insert("type".into(), json!("longitudinal"));
            m.insert("wave".into(), wave(int(INT1)?)?);
            if op == "AXCL" {
                // Wave2 の 0 も主波長（差なので null にしない）
                let w2 = match int(INT2)? {
                    0 => primary as i64,
                    w => w,
                };
                m.insert("wave2".into(), wave(w2)?);
            }
            m.insert("zone".into(), json!(z));
            m.insert("target".into(), json!(target * unit));
        }
        "LACL" => {
            let w2 = match int(INT2)? {
                0 => primary as i64,
                w => w,
            };
            m.insert("type".into(), json!("lateral_color"));
            m.insert("wave".into(), wave(int(INT1)?)?);
            m.insert("wave2".into(), json!(w2));
            m.insert("target".into(), json!(target * unit));
        }
        // MNSD・MXSD: 面 Int1〜Int2 の半直径を目標と比べてはみ出しを足す（OpticStudio 26.1 で実測、値は目標 ± Σ はみ出し）。
        // MNIN〜MXPD は面の後のガラスの nd・νd・ΔPgF で、ガラスの面だけ（同じく実測）
        "MNSD" | "MXSD" | "MNIN" | "MXIN" | "MNAB" | "MXAB" | "MNPD" | "MXPD" => {
            let s1 = int(INT1)?;
            let s2 = int(INT2)?.min(image as i64 - 1);
            if s1 < 1 || s2 < s1 {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            let side = if op.starts_with("MN") { "min" } else { "max" };
            if op.ends_with("SD") {
                m.insert("type".into(), json!("semi"));
                m.insert(side.into(), json!(target * unit));
            } else {
                m.insert("type".into(), json!("surface"));
                let q = match &op[2..] {
                    "IN" => "nd",
                    "AB" => "vd",
                    _ => "dpgf",
                };
                m.insert("q".into(), json!(q));
                m.insert(side.into(), json!(target));
            }
            m.insert("surf".into(), json!(s1));
            m.insert("to".into(), json!(s2));
        }
        // SFNO・TFNO: 視野 Int1・波長 Int2 の実光線の F ナンバー（OpticStudio 26.1 で実測）
        "SFNO" | "TFNO" => {
            m.insert("type".into(), json!("fnum"));
            m.insert("field".into(), json!(field_no(int(INT1)?)?));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert(
                "dir".into(),
                json!(if op == "SFNO" { "sag" } else { "tan" }),
            );
            m.insert("target".into(), json!(target));
        }
        "CENX" | "CENY" => {
            if int(INT1)? as usize != image {
                return Err("only the centroid on the image surface is supported".into());
            }
            if int(data(2))? != 0 {
                return Err("polarization is not supported".into());
            }
            m.insert("type".into(), json!("centroid"));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), json!(field_no(int(data(1))?)?));
            m.insert("q".into(), json!(if op == "CENX" { "x" } else { "y" }));
            m.insert("target".into(), json!(target * unit));
        }
        // ZPLM: Int1 マクロの番号（ZPLnn.ZPL）、Int2 OPTRETURN の番号、Data1〜4 が PVHX・PVHY・PVPX・PVPY
        "ZPLM" => {
            let n = int(INT1)?;
            if !(0..=99).contains(&n) {
                return Err(format!("macro number {n} (0 to 99)"));
            }
            m.insert("type".into(), json!("zplm"));
            m.insert("macro".into(), json!(n));
            m.insert("data".into(), json!(int(INT2)?));
            for (k, i) in ["hx", "hy", "px", "py"].iter().zip(1..) {
                m.insert((*k).into(), json!(num(data(i))?));
            }
            m.insert("target".into(), json!(target));
        }
        _ => return Err("this operand is not supported".into()),
    }
    Ok(m)
}

/// .zmx の評価関数を項の JSON に写す。primary は系の主波長の番号（1 始まり）、image は像面の番号、sys は系の様子
pub fn terms_from_zmx(mf: &MeritFunction, primary: usize, image: usize, sys: &ZmxSys) -> ZmxMerit {
    let sys = &ZmxSys {
        mce: mf.mce.clone(),
        ..sys.clone()
    };
    let mut cx = Ctx {
        unit: mf.lens_unit_mm,
        primary,
        image,
        sys,
        rows: &mf.rows,
        notes: Vec::new(),
    };
    // 行ごとの構成（CONF n の後の行は構成 n。OPGT・OPLT は参照する行の構成で計算する）
    let mut config = 1i64;
    let mut configs = 1usize;
    let mut row_config = Vec::with_capacity(mf.rows.len());
    for r in &mf.rows {
        if r.op == "CONF" {
            match r.int(INT1) {
                Ok(c) if c >= 1 => {
                    config = c;
                    configs = configs.max(c as usize);
                }
                Ok(c) => cx.skip(r, &format!("configuration {c}")),
                Err(e) => cx.skip(r, &e.to_string()),
            }
        }
        row_config.push(config);
    }
    let flow = flow(&mf.rows, &mut cx);
    // Wizard の光線の行は視野ごとのスポット・波面の項にまとめる
    let mut wiz = wizard::group(&mf.rows, &row_config, cx.unit, cx.sys.symmetric);
    cx.notes.append(&mut wiz.notes);
    let mut terms = Vec::new();
    let mut sources = Vec::new();
    for (i, r) in mf.rows.iter().enumerate() {
        let Some(fields) = &flow[i] else {
            continue;
        };
        if wiz.used[i] {
            for m in wiz.terms.remove(&i).unwrap_or_default() {
                terms.push(Value::Object(m));
                sources.push(r.row);
            }
            continue;
        }
        if r.op == "BLNK" || r.op == "CONF" {
            continue;
        }
        if !r.readable() {
            cx.skip(r, "can't read the merit function line");
            continue;
        }
        let weight = match r.num(WEIGHT) {
            Ok(w) => w,
            Err(e) => {
                cx.skip(r, &e.to_string());
                continue;
            }
        };
        if weight == 0.0 {
            continue;
        }
        if weight < 0.0 {
            cx.skip(
                r,
                "a negative weight (a Lagrange constraint in OpticStudio) is not supported",
            );
            continue;
        }
        // OPGT・OPLT: Int1 の行の量 ≥・≤ 目標
        let (src, side, config) = match r.op.as_str() {
            "OPGT" | "OPLT" => {
                let n = match r.int(INT1) {
                    Ok(n) => n,
                    Err(e) => {
                        cx.skip(r, &e.to_string());
                        continue;
                    }
                };
                let k = (n >= 1)
                    .then(|| n as usize - 1)
                    .filter(|&k| k < cx.rows.len() && k != i);
                let Some(k) = k else {
                    cx.skip(r, &format!("no row {n} in the merit function"));
                    continue;
                };
                let side = if r.op == "OPGT" { "min" } else { "max" };
                (k, Some(side), row_config[k])
            }
            _ => (i, None, row_config[i]),
        };
        let at = calc::Rows {
            rows: cx.rows,
            row_config: &row_config,
            unit: cx.unit,
            primary: cx.primary,
            image: cx.image,
            sys: cx.sys,
            nodes: Default::default(),
        };
        let (src_i, src) = (src, &cx.rows[src]);
        let mut m = match at.kind(src_i) {
            Ok(m) => m,
            Err(e) => {
                let why = if side.is_some() {
                    format!("the referenced row {} ({}) is {e}", src.row, src.op)
                } else {
                    e
                };
                cx.skip(r, &why);
                continue;
            }
        };
        if let Some(why) = read_differs(src, cx.sys) {
            cx.notes.push(format!(
                ".zmx merit function row {} ({}, line {}): {why}",
                src.row, src.op, src.line
            ));
        }
        // DIST は Absolute のときだけ長さ
        // ABCD は角度の視野で長さ
        let len = is_length(&src.op)
            || m.get("absolute") == Some(&json!(true))
            || (src.op == "ABCD" && cx.sys.angle_field);
        // 値が µm の行（lensbench の項は mm）
        let um = ["GENC", "DENC"].contains(&src.op.as_str());
        if let Some(side) = side {
            let single = m.get("type").and_then(Value::as_str) != Some("zplm");
            let Some(_) = m.remove("target").filter(|_| single) else {
                cx.skip(
                    r,
                    &format!(
                        "the referenced row {} ({}) has no single value",
                        src.row, src.op
                    ),
                );
                continue;
            };
            let t = r.num(TARGET).unwrap_or(0.0);
            let t = if um {
                t * 1e-3
            } else if src.op == "CVOL" {
                t * cx.unit.powi(3) * 1e-3
            } else if len {
                t * cx.unit
            } else {
                t
            };
            m.insert(side.into(), json!(t));
        }
        // 長さの項の残差は mm になるので、重みをレンズ単位の 2 乗で割って OpticStudio と同じ釣り合いにする
        // 曲率（1/レンズ単位）の残差は逆に、重みにレンズ単位の 2 乗をかける
        let weight = if um {
            weight * 1e6
        } else if src.op == "CVOL" {
            // レンズ単位³ を cm³ に
            weight / (cx.unit.powi(3) * 1e-3).powi(2)
        } else if src.op == "BFSD" {
            weight / shape::bfsd_scale(src.int(INT2).unwrap_or(1), cx.unit).powi(2)
        } else if len {
            weight / (cx.unit * cx.unit)
        } else if ["CVGT", "CVLT", "CVVA", "POWR"].contains(&src.op.as_str()) {
            weight * cx.unit * cx.unit
        } else {
            weight
        };
        m.insert("weight".into(), json!(weight));
        // MCOV などは行の構成でなく Cfg# の構成、ZTHI はすべての構成
        if m.get("type").and_then(Value::as_str) == Some("spread") {
            configs = configs.max(cx.sys.configs);
        } else if let Some(c) = m.get("config").and_then(Value::as_u64) {
            configs = configs.max(c as usize);
        } else if config != 1 {
            m.insert("config".into(), json!(config));
        }
        let mut v = Value::Object(m);
        if !fields.is_empty() {
            modified_fields(&mut v, fields);
        }
        terms.push(v);
        sources.push(r.row);
    }
    ZmxMerit {
        terms,
        sources,
        notes: cx.notes,
        configs,
    }
}

/// 行ごとの、評価する行か（ENDX の後・GOTO で飛ばす行は None）と、その行で FDMO が変えている視野（番号 → 正規化視野）。
/// OpticStudio 26.1 で実測: ENDX の後の行は 0。GOTO n は自分の行から行 n の手前までを飛ばす（n が自分の行以下なら何も
/// 飛ばさない）。FDMO は FDRE（または評価の終わり）まで、視野の番号で指す行の視野を変え、Hx・Hy を直接書いた行は変えない
fn flow(rows: &[MeritRow], cx: &mut Ctx) -> Vec<Option<BTreeMap<i64, (f64, f64)>>> {
    let mut out = Vec::with_capacity(rows.len());
    let mut fields = BTreeMap::new();
    let mut skip_to = 0usize;
    let mut end = false;
    for (i, r) in rows.iter().enumerate() {
        if end || i + 1 < skip_to {
            out.push(None);
            continue;
        }
        let int = |k: usize| r.int(k).map_err(|e| e.to_string());
        let num = |k: usize| r.num(k).map_err(|e| e.to_string());
        match r.op.as_str() {
            "ENDX" => end = true,
            "GOTO" => match int(INT1) {
                Ok(n) if n as usize > i + 1 => skip_to = n as usize,
                Ok(_) => {}
                Err(e) => cx.skip(r, &e),
            },
            "FDMO" => {
                let f = (|| -> Result<(i64, f64, f64, bool), String> {
                    let vig = (3..=6).any(|k| num(data(k)).unwrap_or(0.0) != 0.0);
                    Ok((int(INT1)?, num(data(1))?, num(data(2))?, vig))
                })();
                match f {
                    Ok((f, hx, hy, vig)) => {
                        if vig {
                            cx.notes.push(format!(
                                ".zmx merit function row {} (FDMO, line {}): the vignetting factors are not used",
                                r.row, r.line
                            ));
                        }
                        fields.insert(f, (hx, hy));
                    }
                    Err(e) => cx.skip(r, &e),
                }
            }
            "FDRE" => match int(INT1) {
                Ok(f) => {
                    fields.remove(&f);
                }
                Err(e) => cx.skip(r, &e),
            },
            _ => {}
        }
        if matches!(r.op.as_str(), "ENDX" | "GOTO" | "FDMO" | "FDRE") {
            out.push(None);
        } else {
            out.push(Some(fields.clone()));
        }
    }
    out
}

/// 項（と calc の引数の項）の視野の番号のうち FDMO が変えたものを正規化視野にする
fn modified_fields(v: &mut Value, fields: &BTreeMap<i64, (f64, f64)>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                let hit = x.as_i64().and_then(|n| fields.get(&n));
                match hit {
                    Some(&(hx, hy)) if k == "field" || k == "reference" => *x = json!([hx, hy]),
                    _ => modified_fields(x, fields),
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| modified_fields(x, fields)),
        _ => {}
    }
}

/// 値が長さ（レンズ単位）の演算子
fn is_length(op: &str) -> bool {
    PARAXIAL
        .iter()
        .chain(RAY)
        .chain(PAR)
        .any(|(o, _, l)| *o == op && *l)
        || [
            "CENX", "CENY", "RSCE", "RSCH", "CTGT", "CTLT", "CTVA", "TTHI", "TRAC", "TRAD", "TRAE",
            "TRCX", "TRCY", "ETGT", "ETLT", "ETVA", "MNCT", "MXCT", "MNCA", "MXCA", "MNCG", "MXCG",
            "MNET", "MXET", "MNEA", "MXEA", "MNEG", "MXEG", "DMGT", "DMLT", "DMVA", "SAGX", "SAGY",
            "SSAG", "STHI", "GLCX", "GLCY", "GLCZ", "PLEN", "YNIP", "EFLX", "EFLY", "EFLA", "CARD",
            "LONA", "AXCL", "LACL", "MNSD", "MXSD", "TTGT", "TTLT", "TTVA", "ZTHI", "TSAG",
        ]
        .contains(&op)
}

/// OpticStudio 26.1 の ZERN（Standard）が当てはめる項の数: 隣り合う ZERN の行（Term・目標・重みのほかが同じ）の一番大きい
/// Term と 11 の大きい方（マニュアルの記述を Double Gauss 28 で実測。Term 4 の 1 行は 11 項、隣に Term 37 の行があれば 37 項）
fn zern_standard_fit(rows: &[MeritRow], k: usize) -> usize {
    let same = |r: &MeritRow| {
        r.op == "ZERN"
            && (1..10)
                .filter(|&j| j != TARGET && j != WEIGHT)
                .all(|j| r.num(j).ok() == rows[k].num(j).ok())
    };
    let lo = (0..k)
        .rev()
        .take_while(|&j| same(&rows[j]))
        .last()
        .unwrap_or(k);
    let hi = (k + 1..rows.len())
        .take_while(|&j| same(&rows[j]))
        .last()
        .unwrap_or(k);
    rows[lo..=hi]
        .iter()
        .filter_map(|r| r.int(INT1).ok())
        .map(|t| t.max(0) as usize)
        .max()
        .unwrap_or(0)
        .max(11)
}

#[cfg(test)]
mod tests;
