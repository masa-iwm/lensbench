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

mod calc;
mod wizard;

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
}

impl Default for ZmxSys {
    fn default() -> Self {
        ZmxSys {
            glass: Vec::new(),
            symmetric: true,
            primary_varies: false,
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
];

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
        if w != 0 && w as usize != primary {
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
        // TRAX・TRAY・TRAR は像面（Int1 は使わない）
        if !op.starts_with("TRA") {
            m.insert("surf".into(), json!(int(INT1)?));
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
        // ETGT・ETLT・ETVA: Int1 の面の縁の厚み（半直径の矢高）≥・≤・＝ 目標
        "ETGT" | "ETLT" | "ETVA" => {
            let j = int(INT1)?;
            if j < 1 || j as usize >= image {
                return Err(format!("surface {j}"));
            }
            m.insert("type".into(), json!("thickness"));
            m.insert("surf".into(), json!(j));
            m.insert("edge".into(), json!(true));
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
        // 面の量: CV 曲率・CO コーニック・DM 径（Mode 0 は玉の径）の ≥・≤・＝、矢高・点の厚み・大域座標
        "CVGT" | "CVLT" | "CVVA" | "COGT" | "COLT" | "COVA" | "DMGT" | "DMLT" | "DMVA" | "SAGX"
        | "SAGY" | "SSAG" | "STHI" | "GLCX" | "GLCY" | "GLCZ" => {
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
                _ => ("gz", None),
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
            let len = !matches!(q, "curv" | "conic");
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
        "OPDC" => {
            m.insert("type".into(), json!("opd"));
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), norm_field()?);
            m.insert("px".into(), json!(num(data(3))?));
            m.insert("py".into(), json!(num(data(4))?));
            m.insert("target".into(), json!(target));
        }
        // MTFT・MTFS: Int1 標本（1 が 32 × 32、1 つ増すごとに 2 倍）、Int2 波長、Data1 視野の番号、Data2 周波数
        // [cycles/mm]、Data3 Grid（0 は FFT）、Data4 Data Type（0 は変調）
        "MTFT" | "MTFS" => {
            if int(data(3))? != 0 || int(data(4))? != 0 {
                return Err(
                    "only the FFT MTF modulation (Grid 0, Data Type 0) is supported".into(),
                );
            }
            m.insert("type".into(), json!("mtf"));
            sampling(int(INT1)?)?;
            m.insert("wave".into(), wave(int(INT2)?)?);
            m.insert("field".into(), json!(field_no(int(data(1))?)?));
            if unit != 1.0 {
                return Err("MTF frequencies in lens units other than mm are not checked".into());
            }
            m.insert("freq".into(), json!(num(data(2))?));
            m.insert(
                "dir".into(),
                json!(if op == "MTFT" { "tan" } else { "sag" }),
            );
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
    // Wizard の光線の行は視野ごとのスポット・波面の項にまとめる
    let mut wiz = wizard::group(&mf.rows, &row_config, cx.unit);
    cx.notes.append(&mut wiz.notes);
    let mut terms = Vec::new();
    let mut sources = Vec::new();
    for (i, r) in mf.rows.iter().enumerate() {
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
        // DIST は Absolute のときだけ長さ
        let len = is_length(&src.op) || m.get("absolute") == Some(&json!(true));
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
            m.insert(side.into(), json!(if len { t * cx.unit } else { t }));
        }
        // 長さの項の残差は mm になるので、重みをレンズ単位の 2 乗で割って OpticStudio と同じ釣り合いにする
        // 曲率（1/レンズ単位）の残差は逆に、重みにレンズ単位の 2 乗をかける
        let weight = if len {
            weight / (cx.unit * cx.unit)
        } else if ["CVGT", "CVLT", "CVVA", "POWR"].contains(&src.op.as_str()) {
            weight * cx.unit * cx.unit
        } else {
            weight
        };
        m.insert("weight".into(), json!(weight));
        if config != 1 {
            m.insert("config".into(), json!(config));
        }
        terms.push(Value::Object(m));
        sources.push(r.row);
    }
    ZmxMerit {
        terms,
        sources,
        notes: cx.notes,
        configs,
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
            "LONA", "AXCL", "LACL",
        ]
        .contains(&op)
}

/// 項の JSON（[`crate::merit`] の表）を .zmx のメリットファンクションの行にする（[`terms_from_zmx`] の逆）。
/// systems は構成の系（視野の番号を正規化視野に直す・主波長・像面に使う）。OpticStudio のオペランドで表せない項
/// （coupling・plugin・名前のマクロの zplm・視野を正規化視野で書いた MTF・Zernike・重心・lensbench だけの量 pwfn）は
/// 注意にして飛ばす。min・max の項は重み 0 の行と、それを参照する OPGT・OPLT の 2 行にする。構成が変わるところに CONF。
/// before は前に置く行（.zmx から読んだメリットファンクションなど）で、行の番号と構成はその続きにする
pub fn rows_from_terms(
    terms: &[Value],
    systems: &[lensbench_core::System],
    before: &[MeritRow],
) -> (Vec<MeritRow>, Vec<String>) {
    let mut rows: Vec<MeritRow> = Vec::new();
    let mut notes = Vec::new();
    let mut config = before
        .iter()
        .rev()
        .find(|r| r.op == "CONF")
        .and_then(|r| r.int(INT1).ok())
        .map_or(1, |c| c.max(1) as u64);
    for (i, t) in terms.iter().enumerate() {
        let c = t.get("config").and_then(Value::as_u64).unwrap_or(1);
        let Some(s) = (c >= 1).then(|| systems.get(c as usize - 1)).flatten() else {
            notes.push(format!(
                "term {}: no configuration {c}; not written to the .zmx",
                i + 1
            ));
            continue;
        };
        let out = match row_of(t, s) {
            Ok(r) => r,
            Err(e) => {
                notes.push(format!("term {}: {e}; not written to the .zmx", i + 1));
                continue;
            }
        };
        if let Some(e) = os_value_differs(t) {
            notes.push(format!("term {}: {e}", i + 1));
        }
        if c != config {
            rows.push(MeritRow::new(
                "CONF",
                [c as f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                0.0,
                0.0,
            ));
            config = c;
        }
        let weight = t.get("weight").and_then(Value::as_f64).unwrap_or(1.0);
        match out {
            (op, p, Bound::Target(target)) => rows.push(MeritRow::new(op, p, target, weight)),
            (op, p, Bound::Side(side, b)) => {
                rows.push(MeritRow::new(op, p, b, 0.0));
                let n = (before.len() + rows.len()) as f64;
                let z = [n, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
                rows.push(MeritRow::new(side, z, b, weight));
            }
            (op, p, Bound::Both(target)) => {
                for o in op.split(',') {
                    rows.push(MeritRow::new(o, p, target, weight));
                }
            }
        }
    }
    (rows, notes)
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

/// 書いた行の OpticStudio の値が項の値と違うとき、その理由: Zernike の当てはめる項の数が OpticStudio の ZERN（Fringe は 37、
/// 1 行だけの Standard は求める項と 11 の大きい方）と違う、スポット・波面の瞳の求積が OpticStudio と違う
fn os_value_differs(t: &Value) -> Option<String> {
    let str_of = |k: &str| t.get(k).and_then(Value::as_str);
    match str_of("type")? {
        "zernike" => {
            let term = t.get("term").and_then(Value::as_u64)?;
            let fit = t
                .get("fit")
                .and_then(Value::as_u64)
                .unwrap_or(term.max(37));
            let os = if str_of("kind") == Some("standard") {
                term.max(11)
            } else {
                37
            };
            (fit != os).then(|| {
                format!("OpticStudio's ZERN fits {os} terms, so its value differs from the term's")
            })
        }
        "spot" | "wavefront" => (str_of("quadrature") != Some("opticstudio")).then(|| {
            "OpticStudio's pupil quadrature (2·Ring arms) differs from the term's, so its value differs"
                .to_string()
        }),
        _ => None,
    }
}

/// 項の目標
enum Bound {
    Target(f64),
    /// OPGT・OPLT と境界
    Side(&'static str, f64),
    /// 2 行（MTF の both: 演算子はカンマ区切り）
    Both(f64),
}

/// 1 つの項の行（演算子, P1〜P8, 目標）
fn row_of(
    t: &Value,
    s: &lensbench_core::System,
) -> Result<(&'static str, [f64; 8], Bound), String> {
    let ty = t
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let num = |k: &str| t.get(k).and_then(Value::as_f64);
    let bound = |default: f64| -> Bound {
        if let Some(b) = num("min") {
            Bound::Side("OPGT", b)
        } else if let Some(b) = num("max") {
            Bound::Side("OPLT", b)
        } else {
            Bound::Target(num("target").unwrap_or(default))
        }
    };
    // 波長: null は 0（多色）。主波長の意味の null は呼ぶ側で直す
    let wave = || -> f64 { t.get("wave").and_then(Value::as_u64).unwrap_or(0) as f64 };
    let primary = (s.primary + 1) as f64;
    let field = || -> Result<(f64, f64), String> {
        match t.get("field") {
            Some(Value::Array(a)) if a.len() == 2 => {
                Ok((a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)))
            }
            Some(v) => match v.as_u64() {
                Some(i) if i >= 1 && (i as usize) <= s.fields.len() => Ok(s.hxhy(i as usize - 1)),
                _ => Err(format!("field {v}")),
            },
            None => Err("no field".into()),
        }
    };
    let field_no = || -> Result<f64, String> {
        match t.get("field").and_then(Value::as_u64) {
            Some(i) if i >= 1 => Ok(i as f64),
            _ => Err("OpticStudio needs a field number here (not a normalized field)".into()),
        }
    };
    let samp = |n: Option<f64>| -> f64 {
        // 32 × 32 が 1、2 倍ごとに 1 つ増す（ほかの数はそれを超えない最大の標本）
        let n = n.unwrap_or(64.0).max(32.0);
        ((n / 32.0).log2().floor() + 1.0).min(6.0)
    };
    Ok(match ty.as_str() {
        "paraxial" => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let q = lensbench_optim::ParaxQ::parse(q)
                .map_err(|e| e.to_string())?
                .name();
            let Some((op, _, _)) = PARAXIAL.iter().find(|(_, n, _)| *n == q) else {
                return Err(format!("paraxial {q} has no OpticStudio operand"));
            };
            // 波長のない近軸量は主波長（Wave 0 も OpticStudio 26.1 では主波長だが、番号で書く方が確か）
            let w = match t.get("wave").and_then(Value::as_u64) {
                Some(w) => w as f64,
                None => primary,
            };
            let p = [0.0, w, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            (op, p, bound(0.0))
        }
        "spot" | "wavefront" => {
            let chief = t.get("reference").and_then(Value::as_str) == Some("chief");
            let op = match (ty.as_str(), chief) {
                ("spot", false) => "RSCE",
                ("spot", true) => "RSCH",
                (_, false) => "RWCE",
                (_, true) => "RWCH",
            };
            let (hx, hy) = field()?;
            let rings = num("rings").unwrap_or(3.0);
            (
                op,
                [rings, wave(), hx, hy, 0.0, 0.0, 0.0, 0.0],
                Bound::Target(0.0),
            )
        }
        "opd" => {
            let (hx, hy) = field()?;
            let w = if wave() == 0.0 { primary } else { wave() };
            let p = [
                0.0,
                w,
                hx,
                hy,
                num("px").unwrap_or(0.0),
                num("py").unwrap_or(0.0),
                0.0,
                0.0,
            ];
            ("OPDC", p, bound(0.0))
        }
        // 面の間の光路長（PLEN、主波長だけ）と、面ごとの入射角・射出角の境界（MXRI・MNRI・MXRE・MNRE）
        "ray" if t.get("to").is_some() => {
            let (hx, hy) = field()?;
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let w = if wave() == 0.0 { primary } else { wave() };
            let mut p = [
                num("surf").unwrap_or(0.0),
                num("to").unwrap_or(0.0),
                hx,
                hy,
                num("px").unwrap_or(0.0),
                num("py").unwrap_or(0.0),
                0.0,
                0.0,
            ];
            match (q, bound(0.0)) {
                ("opth", b) if w == primary => ("PLEN", p, b),
                ("opth", _) => {
                    return Err("PLEN of OpticStudio is for the primary wavelength".into())
                }
                ("aoi" | "aoe", Bound::Side(side, b)) => {
                    p[6] = w;
                    let op = match (side == "OPGT", q == "aoe") {
                        (true, true) => "MNRE",
                        (true, false) => "MNRI",
                        (false, true) => "MXRE",
                        (false, false) => "MXRI",
                    };
                    (op, p, Bound::Target(b))
                }
                _ => {
                    return Err(format!(
                        "a ray term with to and {q} has no OpticStudio operand"
                    ))
                }
            }
        }
        "ray" => {
            let q = t
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let table = if t.get("paraxial") == Some(&Value::Bool(true)) {
                PAR
            } else {
                RAY
            };
            let Some((op, _, _)) = table
                .iter()
                .find(|(o, n, _)| *n == q || o.eq_ignore_ascii_case(&q))
            else {
                return Err(format!("ray quantity {q}"));
            };
            let (hx, hy) = field()?;
            let mut surf = num("surf").unwrap_or(s.image() as f64);
            if op.starts_with("TRA") {
                if surf != s.image() as f64 {
                    return Err(format!("{op} of OpticStudio is on the image surface only"));
                }
                surf = 0.0;
            }
            let w = if wave() == 0.0 { primary } else { wave() };
            let p = [
                surf,
                w,
                hx,
                hy,
                num("px").unwrap_or(0.0),
                num("py").unwrap_or(0.0),
                0.0,
                0.0,
            ];
            (op, p, bound(0.0))
        }
        "zernike" => {
            let kind = match t.get("kind").and_then(Value::as_str).unwrap_or("fringe") {
                "standard" => 1.0,
                _ => 0.0,
            };
            let w = if wave() == 0.0 { primary } else { wave() };
            let p = [
                num("term").unwrap_or(1.0),
                w,
                samp(num("n")),
                field_no()?,
                kind,
                0.0,
                0.0,
                0.0,
            ];
            ("ZERN", p, bound(0.0))
        }
        "mtf" => {
            let p = [
                samp(num("n")),
                wave(),
                field_no()?,
                num("freq").unwrap_or(0.0),
                0.0,
                0.0,
                0.0,
                0.0,
            ];
            match t.get("dir").and_then(Value::as_str).unwrap_or("both") {
                "tan" => ("MTFT", p, bound(1.0)),
                "sag" => ("MTFS", p, bound(1.0)),
                _ => ("MTFT,MTFS", p, Bound::Both(num("target").unwrap_or(1.0))),
            }
        }
        "seidel" => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let q = lensbench_optim::SeidelQ::parse(q).map_err(|e| e.to_string())?;
            let j = num("surf").unwrap_or(0.0);
            if q == lensbench_optim::SeidelQ::Dist && j == 0.0 {
                return Err("the sum of the Seidel distortion has no OpticStudio operand (DIST of surface 0 is the real distortion)".into());
            }
            let w = if wave() == 0.0 { primary } else { wave() };
            let op = match q.name() {
                "spha" => "SPHA",
                "coma" => "COMA",
                "asti" => "ASTI",
                "fcur" => "FCUR",
                _ => "DIST",
            };
            (op, [j, w, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
        }
        "distortion" => {
            let w = if wave() == 0.0 { primary } else { wave() };
            let abs = t.get("absolute").and_then(Value::as_bool) == Some(true);
            let p = [0.0, w, if abs { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0];
            ("DIST", p, bound(0.0))
        }
        "longitudinal" => {
            let w = if wave() == 0.0 { primary } else { wave() };
            let z = num("zone").unwrap_or(0.0);
            match t.get("wave2").and_then(Value::as_f64) {
                Some(w2) => ("AXCL", [w, w2, z, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0)),
                None => ("LONA", [w, 0.0, z, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0)),
            }
        }
        "lateral_color" => {
            let w = if wave() == 0.0 { primary } else { wave() };
            let w2 = num("wave2").unwrap_or(primary);
            ("LACL", [w, w2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
        }
        "fnum" => {
            let op = match t.get("dir").and_then(Value::as_str) {
                Some("sag") => "SFNO",
                _ => "TFNO",
            };
            let w = if wave() == 0.0 { primary } else { wave() };
            (
                op,
                [field_no()?, w, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                bound(0.0),
            )
        }
        "centroid" => {
            let op = if t.get("q").and_then(Value::as_str) == Some("y") {
                "CENY"
            } else {
                "CENX"
            };
            let p = [
                s.image() as f64,
                wave(),
                field_no()?,
                0.0,
                5.0,
                0.0,
                0.0,
                0.0,
            ];
            (op, p, bound(0.0))
        }
        "thickness" => {
            let s1 = num("surf").unwrap_or(0.0);
            let s2 = num("to").unwrap_or(s1);
            let mut p = [s1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            let edge = t.get("edge") == Some(&Value::Bool(true));
            let zone = num("zone").unwrap_or(1.0);
            if let Some(e) = t.get("each").and_then(Value::as_str) {
                // 面ごとの境界: MN・MX ＋ C（中心）・E（縁）＋ T（すべて）・A（空気）・G（ガラス）。Zone 0 は半直径
                p[1] = s2;
                if edge {
                    p[2] = if zone == 1.0 { 0.0 } else { zone };
                }
                let lower = match bound(0.0) {
                    Bound::Side(s, b) => (s == "OPGT", b),
                    _ => return Err("a thickness term with each needs min or max".into()),
                };
                let op = match (lower.0, edge, e) {
                    (true, false, "air") => "MNCA",
                    (true, false, "glass") => "MNCG",
                    (true, false, _) => "MNCT",
                    (false, false, "air") => "MXCA",
                    (false, false, "glass") => "MXCG",
                    (false, false, _) => "MXCT",
                    (true, true, "air") => "MNEA",
                    (true, true, "glass") => "MNEG",
                    (true, true, _) => "MNET",
                    (false, true, "air") => "MXEA",
                    (false, true, "glass") => "MXEG",
                    (false, true, _) => "MXET",
                };
                (op, p, Bound::Target(lower.1))
            } else if edge {
                if s2 != s1 || zone != 1.0 {
                    return Err("only the edge thickness of one surface at its semi-diameter can be written (ETGT, ETLT, ETVA)".into());
                }
                match bound(0.0) {
                    Bound::Side("OPGT", b) => ("ETGT", p, Bound::Target(b)),
                    Bound::Side(_, b) => ("ETLT", p, Bound::Target(b)),
                    b => ("ETVA", p, b),
                }
            } else if s2 != s1 {
                p[1] = s2;
                ("TTHI", p, bound(0.0))
            } else {
                // 1 面の min・max は CTGT・CTLT の 1 行で書ける
                match bound(0.0) {
                    Bound::Side("OPGT", b) => ("CTGT", p, Bound::Target(b)),
                    Bound::Side(_, b) => ("CTLT", p, Bound::Target(b)),
                    b => ("CTVA", p, b),
                }
            }
        }
        "surface" => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let mut p = [
                num("surf").unwrap_or(0.0),
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ];
            let (x, y) = (num("x").unwrap_or(0.0), num("y").unwrap_or(0.0));
            // GT・LT のある量は 1 行、ほかは VA の行（min・max は OPGT・OPLT）
            let (va, gt, lt) = match q {
                "curv" => ("CVVA", "CVGT", "CVLT"),
                "conic" => ("COVA", "COGT", "COLT"),
                "diam" | "mech_diam" => {
                    p[3] = if q == "diam" { 1.0 } else { 0.0 };
                    ("DMVA", "DMGT", "DMLT")
                }
                "sagx" => ("SAGX", "", ""),
                "sagy" => ("SAGY", "", ""),
                "sag" => {
                    (p[1], p[2], p[3]) = (0.0, x, y);
                    ("SSAG", "", "")
                }
                "thick" => {
                    (p[2], p[3]) = (x, y);
                    ("STHI", "", "")
                }
                "gx" => ("GLCX", "", ""),
                "gy" => ("GLCY", "", ""),
                "gz" => ("GLCZ", "", ""),
                "power" | "ynip" | "index" => {
                    p[1] = if wave() == 0.0 { primary } else { wave() };
                    let op = match q {
                        "power" => "POWR",
                        "ynip" => "YNIP",
                        _ => "INDX",
                    };
                    (op, "", "")
                }
                _ => return Err(format!("surface quantity {q}")),
            };
            match bound(0.0) {
                Bound::Side("OPGT", b) if !gt.is_empty() => (gt, p, Bound::Target(b)),
                Bound::Side(_, b) if !lt.is_empty() => (lt, p, Bound::Target(b)),
                b => (va, p, b),
            }
        }
        "cardinal" => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let q = lensbench_optim::CardQ::parse(q).map_err(|e| e.to_string())?;
            let last = (1..s.image())
                .rev()
                .find(|&j| !s.surfaces[j].ignore)
                .unwrap_or(1);
            let from = num("from").unwrap_or(1.0);
            let to = num("to").unwrap_or(last as f64);
            let w = if wave() == 0.0 { primary } else { wave() };
            match q.card_data() {
                // EFLY は主波長だけ
                None if w == primary => {
                    ("EFLY", [from, to, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
                }
                None => {
                    return Err(
                        "efl at a wavelength other than the primary has no OpticStudio operand"
                            .into(),
                    )
                }
                Some(d) => (
                    "CARD",
                    [from, to, w, 0.0, d as f64, 0.0, 0.0, 0.0],
                    bound(0.0),
                ),
            }
        }
        "zplm" => match t.get("macro").and_then(Value::as_u64) {
            Some(m) if m <= 99 && t.get("file").is_none() => {
                let p = [
                    m as f64,
                    num("data").unwrap_or(0.0),
                    num("hx").unwrap_or(0.0),
                    num("hy").unwrap_or(0.0),
                    num("px").unwrap_or(0.0),
                    num("py").unwrap_or(0.0),
                    0.0,
                    0.0,
                ];
                ("ZPLM", p, bound(0.0))
            }
            _ => return Err("only a ZPL macro given by number can be written as ZPLM".into()),
        },
        t => return Err(format!("{t} terms have no OpticStudio operand")),
    })
}

#[cfg(test)]
mod tests;
