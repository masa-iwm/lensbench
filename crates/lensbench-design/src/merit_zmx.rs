//! .zmx の評価関数（[`lensbench_core::zmx::merit`] の行）を評価関数の項の JSON（[`crate::merit`] の表）に写す。
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

use lensbench_core::zmx::merit::{MeritFunction, MeritRow};
use serde_json::{json, Map, Value};

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
];

/// 1 本の光線の演算子と lensbench の量、長さか
const RAY: &[(&str, &str, bool)] = &[
    ("REAX", "x", true),
    ("REAY", "y", true),
    ("REAZ", "z", true),
    ("REAA", "l", false),
    ("REAB", "m", false),
    ("REAC", "n", false),
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
        // EFFL・PIMH・PMAG などは Int2 が波長（0 は主波長）。lensbench の近軸量は主波長だけ
        let w = int(INT2)?;
        if w != 0 && w as usize != primary {
            return Err(format!(
                "wavelength {w} (lensbench's paraxial terms use the primary wavelength {primary})"
            ));
        }
        m.insert("type".into(), json!("paraxial"));
        m.insert("q".into(), json!(q));
        m.insert(
            "target".into(),
            json!(if *len { target * unit } else { target }),
        );
        return Ok(m);
    }
    if let Some((_, q, len)) = RAY.iter().find(|(o, _, _)| *o == op) {
        m.insert("type".into(), json!("ray"));
        m.insert("surf".into(), json!(int(INT1)?));
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

/// .zmx の評価関数を項の JSON に写す。primary は系の主波長の番号（1 始まり）、image は像面の番号
pub fn terms_from_zmx(mf: &MeritFunction, primary: usize, image: usize) -> ZmxMerit {
    let mut cx = Ctx {
        unit: mf.lens_unit_mm,
        primary,
        image,
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
    let mut terms = Vec::new();
    let mut sources = Vec::new();
    for (i, r) in mf.rows.iter().enumerate() {
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
                (&cx.rows[k], Some(side), row_config[k])
            }
            _ => (r, None, row_config[i]),
        };
        let mut m = match kind_of(src, cx.unit, cx.primary, cx.image) {
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
        if src.op == "ZERN" && m.get("kind").and_then(Value::as_str) == Some("standard") {
            m.insert("fit".into(), json!(zern_standard_fit(cx.rows, src.row - 1)));
        }
        let len = is_length(&src.op);
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
        let weight = if len {
            weight / (cx.unit * cx.unit)
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
    PARAXIAL.iter().chain(RAY).any(|(o, _, l)| *o == op && *l)
        || ["CENX", "CENY", "RSCE", "RSCH"].contains(&op)
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
            (op, [0.0; 8], bound(0.0))
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
        "ray" => {
            let q = t
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let Some((op, _, _)) = RAY
                .iter()
                .find(|(o, n, _)| *n == q || o.eq_ignore_ascii_case(&q))
            else {
                return Err(format!("ray quantity {q}"));
            };
            let (hx, hy) = field()?;
            let surf = num("surf").unwrap_or(s.image() as f64);
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
mod tests {
    use super::*;
    use lensbench_core::zmx::merit::merit_function_text;

    /// 面の後に評価関数を置いた .zmx（OpticStudio 26.1 の書き方、Double Gauss 28 で作った行を A0 に合わせたもの）
    fn zmx(unit: &str, merit: &[&str]) -> String {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let src = lensbench_core::glass::read_text(&root.join("tests/refs/A0/system.zmx")).unwrap();
        let mut out = String::new();
        for l in src.lines() {
            // A0 の空の評価関数（BLNK 1 行）は除く
            if l.starts_with("BLNK") {
                continue;
            }
            if l.starts_with("TOL ") {
                for m in merit {
                    out += m;
                    out += "\n";
                }
            }
            out += &l.replace("UNIT MM", &format!("UNIT {unit}"));
            out += "\n";
        }
        out
    }

    #[test]
    fn rows_to_terms() {
        let rows = [
            "DMFS 0 0 0 0 0 0 0 0 0 0",
            "BLNK comment",
            "EFFL 0 2 0 0 0 0 100 1 0 0",
            "EFFL 0 1 0 0 0 0 100 1 0 0",
            "EXPD 0 0 0 0 0 0 25 0 0 0",
            "RSCE 3 0 0 0.69999999999999996 0 0 0 1 0 0",
            "RSCH 2 1 0 1 0 0 0.001 2 0 0",
            "OPDC 0 2 0 1 0.29999999999999999 0.69999999999999996 0 1.2 0 0",
            "ZERN 11 2 2 2 1 0 0.01 0.80000000000000004 0 0",
            "ZERN 4 1 3 1 2 0.29999999999999999 0.20000000000000001 1.5 1 0",
            "MTFS 2 0 3 50 0 0 0.40000000000000002 2 0 0",
            "REAY 5 1 0 0.5 0 -1 -5 0.29999999999999999 0 0",
            "CENY 9 1 3 0 5 0 14 0.59999999999999998 0 0",
            "EFFL 0 2 0 0 0 0 90 0 0 0",
            "OPGT 14 0 0 0 0 0 90 2 0 0",
            "OPLT 6 0 0 0 0 0 1 1 0 0",
            "CTGT 2 2 0 0 0 0 1.5 3 0 0",
            "TOTR 0 0 0 0 0 0 80 -1 0 0",
            "CONF 2 0 0 0 0 0 0 0 0 0",
            "WFNO 0 0 0 0 0 0 4 0.69999999999999996 0 0",
            "wfno 0 0 0 0 0 0 4 1 0 0",
        ];
        let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
        let z = terms_from_zmx(&mf, 2, 9);
        let want = vec![
            json!({"type": "paraxial", "q": "efl", "target": 100.0, "weight": 1.0}),
            json!({"type": "spot", "rings": 3, "wave": null, "field": [0.0, 0.7], "reference": "centroid", "quadrature": "opticstudio", "weight": 1.0}),
            json!({"type": "opd", "wave": 2, "field": [0.0, 1.0], "px": 0.3, "py": 0.7, "target": 0.0, "weight": 1.2}),
            json!({"type": "zernike", "term": 11, "wave": 2, "n": 64, "field": 2, "kind": "standard", "fit": 11, "target": 0.01, "weight": 0.8}),
            json!({"type": "mtf", "wave": null, "field": 3, "freq": 50.0, "dir": "sag", "target": 0.4, "weight": 2.0}),
            json!({"type": "ray", "surf": 5, "wave": 1, "field": [0.0, 0.5], "px": 0.0, "py": -1.0, "q": "y", "target": -5.0, "weight": 0.3}),
            json!({"type": "centroid", "wave": 1, "field": 3, "q": "y", "target": 14.0, "weight": 0.6}),
            json!({"type": "paraxial", "q": "efl", "min": 90.0, "weight": 2.0}),
            json!({"type": "paraxial", "q": "wfno", "target": 4.0, "weight": 0.7, "config": 2}),
        ];
        assert_eq!(z.terms, want);
        assert_eq!(z.configs, 2);
        let skipped: Vec<usize> = z
            .notes
            .iter()
            .map(|n| n.split_whitespace().nth(4).unwrap().parse().unwrap())
            .collect();
        // EFFL の波長 1（主波長でない）・RSCH の目標・Annular の ZERN・目標のない項への OPLT・CTGT・負の重み・読めない行
        assert_eq!(skipped, [4, 7, 10, 16, 17, 18, 21], "{:?}", z.notes);
        // 長さの目標はレンズ単位から mm に、MTF はレンズ単位が mm のときだけ
        let mf = merit_function_text(&zmx("IN", &rows)).unwrap();
        let z = terms_from_zmx(&mf, 2, 9);
        assert_eq!(z.terms[0]["target"], json!(2540.0));
        // 長さの項の重みは単位の 2 乗で割る（OpticStudio と同じ釣り合い）
        assert_eq!(z.terms[0]["weight"], json!(1.0 / (25.4 * 25.4)));
        let ray = z.terms.iter().find(|t| t["type"] == "ray").unwrap();
        assert_eq!(ray["target"], json!(-127.0));
        assert!(z.terms.iter().all(|t| t["type"] != "mtf"));
        let i = z.terms.iter().position(|t| t.get("min").is_some()).unwrap();
        assert_eq!(z.terms[i]["min"], json!(90.0 * 25.4));
    }

    #[test]
    fn terms_can_be_read() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let s = lensbench_core::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_core::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let rows = [
            "EFFL 0 0 0 0 0 0 50 1 0 0",
            "RWCE 3 0 0 0.69999999999999996 0 0 0 0.5 0 0",
            "ZERN 9 0 1 1 0 0 0 1 0 0",
            "MTFT 2 0 1 30 0 0 0.6 1 0 0",
            "REAX 9 0 0 1 0 1 0 1 0 0",
            "CENX 9 0 1 0 5 0 0 1 0 0",
            "OPLT 1 0 0 0 0 0 60 1 0 0",
        ];
        let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
        let z = terms_from_zmx(&mf, s.primary + 1, s.image());
        assert!(z.notes.is_empty(), "{:?}", z.notes);
        let t = crate::merit::terms_from_json(&Value::Array(z.terms), std::slice::from_ref(&s))
            .unwrap();
        let mut p = lensbench_optim::Problem::single(s.clone());
        p.terms = t;
        p.check().unwrap();
        let v = p.term_values(&[]).unwrap();
        assert_eq!(v.len(), 7);
        assert_eq!(v[0].value(), s.paraxial().efl);
    }

    /// 項 → 行 → 項で同じ項に戻る（表せない項は注意）
    #[test]
    fn terms_to_rows_and_back() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s =
            lensbench_core::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let terms = vec![
            json!({"type": "paraxial", "q": "efl", "target": 50.0, "weight": 2.0}),
            json!({"type": "spot", "rings": 4, "wave": null, "field": [0.0, 0.5], "reference": "chief", "quadrature": "opticstudio", "weight": 1.0}),
            json!({"type": "opd", "wave": 1, "field": [0.0, 1.0], "px": 0.3, "py": 0.7, "target": 0.0, "weight": 1.0}),
            json!({"type": "zernike", "term": 9, "wave": 1, "n": 128, "field": 1, "kind": "fringe", "fit": 37, "target": 0.1, "weight": 1.0}),
            json!({"type": "mtf", "wave": null, "field": 1, "freq": 30.0, "dir": "tan", "target": 0.5, "weight": 1.0}),
            json!({"type": "ray", "surf": 3, "wave": 1, "field": [0.0, 1.0], "px": 0.0, "py": 1.0, "q": "m", "target": 0.0, "weight": 1.0}),
            json!({"type": "paraxial", "q": "totr", "max": 60.0, "weight": 4.0}),
            json!({"type": "centroid", "wave": null, "field": 1, "q": "x", "target": 0.0, "weight": 1.0, "config": 2}),
        ];
        let bad = [
            json!({"type": "paraxial", "q": "pwfn", "target": 1.0}),
            json!({"type": "mtf", "field": [0.0, 1.0], "freq": 30.0}),
            json!({"type": "coupling"}),
        ];
        let all: Vec<Value> = terms.iter().chain(&bad).cloned().collect();
        let systems = [s.clone(), s.clone()];
        let before = [MeritRow::blank("from the zmx")];
        let (rows, notes) = rows_from_terms(&all, &systems, &before);
        assert_eq!(notes.len(), 3, "{notes:?}");
        // 行を .zmx に書いて読み直す
        let mut txt = String::new();
        for r in before.iter().chain(&rows) {
            txt += &r.text();
            txt += "\n";
        }
        let z = zmx("MM", &txt.lines().collect::<Vec<_>>());
        let mf = merit_function_text(&z).unwrap();
        assert_eq!(mf.rows.len(), rows.len() + 1);
        let back = terms_from_zmx(&mf, s.primary + 1, s.image());
        assert!(back.notes.is_empty(), "{:?}", back.notes);
        // MTF の標本は渡さない
        let mut want = terms.clone();
        want[4].as_object_mut().unwrap().remove("n");
        assert_eq!(back.terms, want);
    }

    /// Standard の ZERN は隣り合う ZERN の行（Term のほかが同じ）の一番大きい Term と 11 の大きい方で当てはめる（OpticStudio 26.1）。
    /// lensbench の項から書く行は、当てはめる項の数・瞳の求積が OpticStudio と違えば注意にする
    #[test]
    fn zern_fit_follows_opticstudio() {
        let rows = [
            "ZERN 4 2 2 3 1 0 0 1 0 0",
            "ZERN 22 2 2 3 1 0 0.5 1 0 0",
            "ZERN 5 2 2 2 1 0 0 1 0 0",
            "ZERN 4 2 2 3 0 0 0 1 0 0",
        ];
        let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
        let z = terms_from_zmx(&mf, 2, 9);
        let fit: Vec<_> = z.terms.iter().map(|t| t["fit"].as_u64().unwrap()).collect();
        assert_eq!(fit, [22, 22, 11, 37]);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let s = lensbench_core::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_core::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let t = |fit: Option<u64>| {
            let mut v = json!({"type": "zernike", "term": 5, "field": 1, "kind": "standard"});
            if let Some(f) = fit {
                v["fit"] = json!(f);
            }
            v
        };
        let spot = |q: &str| json!({"type": "spot", "field": [0.0, 1.0], "quadrature": q});
        let all = [t(Some(11)), t(None), spot("opticstudio"), spot("lensbench")];
        let (rows, notes) = rows_from_terms(&all, &[s], &[]);
        assert_eq!(rows.len(), 4);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].starts_with("term 2:"), "{notes:?}");
        assert!(notes[1].starts_with("term 4:"), "{notes:?}");
    }
}
