//! 項の JSON を .zmx のメリットファンクションの行にする（[`super::terms_from_zmx`] の逆）

use super::*;

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
        if crate::merit::is_comment(t) {
            let text = t.get("text").and_then(Value::as_str).unwrap_or_default();
            rows.push(MeritRow::blank(text));
            continue;
        }
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
        // 値が µm の行は、mm の項と同じ釣り合いになるよう重みを 1e-6 倍（読むときの逆）
        let weight = if ["GENC", "DENC", "CVOL"].contains(&out.0) {
            weight * 1e-6
        } else {
            weight
        };
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
        // 偶数の格子は RSRE・RSRH・RWRE・RWRH と同じ点
        "spot" | "wavefront" if grid_samp(t).is_some() => None,
        "spot" | "wavefront" => (str_of("quadrature") != Some("opticstudio")).then(|| {
            "OpticStudio's pupil quadrature (2·Ring arms) differs from the term's, so its value differs"
                .to_string()
        }),
        _ => None,
    }
}

/// 格子の求積で 1 辺の升が偶数の spot・wavefront の項の、RSRE・RSRH・RWRE・RWRH の Samp（瞳の 4 分の 1 ごとに Samp × Samp。
/// OpticStudio 26.1 で実測）
fn grid_samp(t: &Value) -> Option<u64> {
    if t.get("quadrature").and_then(Value::as_str) != Some("grid") {
        return None;
    }
    let n = t.get("n").and_then(Value::as_u64)?;
    (n >= 2 && n % 2 == 0).then_some(n / 2)
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
            let samp = grid_samp(t);
            let op = match (ty.as_str(), chief, samp.is_some()) {
                ("spot", false, false) => "RSCE",
                ("spot", true, false) => "RSCH",
                (_, false, false) => "RWCE",
                (_, true, false) => "RWCH",
                ("spot", false, true) => "RSRE",
                ("spot", true, true) => "RSRH",
                (_, false, true) => "RWRE",
                (_, true, true) => "RWRH",
            };
            let (hx, hy) = field()?;
            let rings = samp.map_or(num("rings").unwrap_or(3.0), |n| n as f64);
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
            let mut op = *op;
            if image_only(op) {
                if surf != s.image() as f64 {
                    // 像面でない面の TRAR は TRAI（Surf 0 は OpticStudio で像面にならない）
                    if op != "TRAR" || surf < 1.0 {
                        return Err(format!("{op} of OpticStudio is on the image surface only"));
                    }
                    op = "TRAI";
                } else {
                    surf = 0.0;
                }
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
        // 求め方で MTF*・MSW*・GMT*・MTH*、向きで末尾の T・S・A
        "mtf" => {
            let kind = t
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("diffraction");
            let geo = kind == "geometric";
            let pre = match kind {
                "square" => "MSW",
                "geometric" => "GMT",
                "huygens" => "MTH",
                _ => "MTF",
            };
            // GMT* の Data3（!Scl）は 1 で回折限界の MTF をかけない
            let no_scale = geo && t.get("diff_limit").and_then(Value::as_bool) == Some(false);
            let p = [
                samp(num("n")),
                wave(),
                field_no()?,
                num("freq").unwrap_or(0.0),
                if no_scale { 1.0 } else { 0.0 },
                0.0,
                0.0,
                0.0,
            ];
            // [子午, 球欠, 平均, 両方]
            let ops: [&'static str; 4] = match pre {
                "MSW" => ["MSWT", "MSWS", "MSWA", "MSWT,MSWS"],
                "GMT" => ["GMTT", "GMTS", "GMTA", "GMTT,GMTS"],
                "MTH" => ["MTHT", "MTHS", "MTHA", "MTHT,MTHS"],
                _ => ["MTFT", "MTFS", "MTFA", "MTFT,MTFS"],
            };
            match t.get("dir").and_then(Value::as_str).unwrap_or("both") {
                "tan" => (ops[0], p, bound(1.0)),
                "sag" => (ops[1], p, bound(1.0)),
                "avg" => (ops[2], p, bound(1.0)),
                _ => (ops[3], p, Bound::Both(num("target").unwrap_or(1.0))),
            }
        }
        // 幾何は GENC・GENF、fft は DENC・DENF（Refp 0〜2）、huygens は DENC・DENF（Refp 3〜5）。大きさは µm
        "energy" => {
            let str_of = |k: &str| t.get(k).and_then(Value::as_str);
            let method = str_of("method").unwrap_or("geometric");
            let shape = match str_of("shape").unwrap_or("circle") {
                "x" => 2.0,
                "y" => 3.0,
                "square" => 4.0,
                _ => 1.0,
            };
            let center = match str_of("center").unwrap_or("chief") {
                "centroid" => 1.0,
                "vertex" => 2.0,
                "middle" if method == "geometric" => 3.0,
                "middle" => {
                    return Err("OpticStudio's DENC and DENF have no middle of the spot".into())
                }
                _ => 0.0,
            };
            let airy = t.get("airy").and_then(Value::as_bool) == Some(true);
            let (size, v) = match (num("size_at"), num("fraction_at")) {
                (Some(f), None) => (true, f),
                (None, Some(r)) => (false, r * 1e3),
                _ => return Err("an energy term needs one of size_at and fraction_at".into()),
            };
            let (op, refp, d5) = match (method, size) {
                ("geometric", true) => ("GENC", center, if airy { 0.0 } else { 1.0 }),
                ("geometric", false) => ("GENF", center, if airy { 0.0 } else { 1.0 }),
                ("huygens", true) => ("DENC", center + 3.0, 0.0),
                ("huygens", false) => ("DENF", center + 3.0, 0.0),
                (_, true) => ("DENC", center, 0.0),
                (_, false) => ("DENF", center, 0.0),
            };
            let p = [samp(num("n")), wave(), field_no()?, v, shape, refp, d5, 0.0];
            let b = match bound(0.0) {
                Bound::Target(x) if size => Bound::Target(x * 1e3),
                Bound::Side(sd, x) if size => Bound::Side(sd, x * 1e3),
                b => b,
            };
            (op, p, b)
        }
        // 標本は OpticStudio の値が lensbench に近づく高め（STRH は Samp 5 = 瞳 512、RELI・EFNO は上限の 100）
        "strehl" => (
            "STRH",
            [5.0, wave(), field_no()?, 0.0, 0.0, 0.0, 0.0, 0.0],
            bound(1.0),
        ),
        // 波長は番号で（OpticStudio の Wave 0 は波長 1）
        "illum" => {
            let op = match t.get("q").and_then(Value::as_str) {
                Some("relative") => "RELI",
                Some("fno") => "EFNO",
                q => return Err(format!("illum q {q:?}")),
            };
            let w = if wave() == 0.0 { primary } else { wave() };
            (
                op,
                [100.0, w, field_no()?, 0.0, 0.0, 0.0, 0.0, 0.0],
                bound(0.0),
            )
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
        // 既定（最大の視野・軸上の基準）は DIST、coef は ABCD、abcd は DISA、peak は DIMX、ftheta は DISC、視野か基準があれば
        // DISG（Wave が負で長さ）
        "distortion" => {
            let w = if wave() == 0.0 { primary } else { wave() };
            let abs = t.get("absolute").and_then(Value::as_bool) == Some(true);
            let a = if abs { 1.0 } else { 0.0 };
            let flag = |k: &str| t.get(k).and_then(Value::as_bool) == Some(true);
            let reference = match t.get("reference") {
                None => 0.0,
                Some(v) => match v.as_u64() {
                    Some(i) if i >= 1 && (i as usize) <= s.fields.len() => i as f64,
                    _ => return Err("OpticStudio needs a field number for the reference".into()),
                },
            };
            if let Some(c) = t.get("coef").and_then(Value::as_str) {
                let k = ["a", "b", "c", "d"]
                    .iter()
                    .position(|x| *x == c)
                    .ok_or(format!("coefficient {c}"))?;
                (
                    "ABCD",
                    [reference, w, k as f64, 0.0, 0.0, 0.0, 0.0, 0.0],
                    bound(0.0),
                )
            } else if let Some(v) = t.get("abcd").and_then(Value::as_array) {
                let x = |i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                let comp = match t
                    .get("component")
                    .and_then(Value::as_str)
                    .unwrap_or("radial")
                {
                    "x" => 1.0,
                    "y" => 2.0,
                    _ => 0.0,
                };
                if abs {
                    return Err("DISA of OpticStudio gives only percent (not absolute)".into());
                }
                let p = [reference, w, comp, field_no()?, x(0), x(1), x(2), x(3)];
                ("DISA", p, bound(0.0))
            } else if t
                .get("component")
                .and_then(Value::as_str)
                .is_some_and(|c| c != "radial")
            {
                return Err("only the radial distortion without abcd can be written".into());
            } else if flag("ftheta") {
                ("DISC", [0.0, w, a, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
            } else if flag("peak") {
                let f = if t.get("field").is_some() {
                    field_no()?
                } else {
                    0.0
                };
                ("DIMX", [f, w, a, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
            } else if t.get("field").is_some() || reference != 0.0 {
                let (hx, hy) = if t.get("field").is_some() {
                    field()?
                } else {
                    (0.0, 1.0)
                };
                let w = if abs { -w } else { w };
                (
                    "DISG",
                    [reference, w, hx, hy, 0.0, 0.0, 0.0, 0.0],
                    bound(0.0),
                )
            } else {
                ("DIST", [0.0, w, a, 0.0, 0.0, 0.0, 0.0, 0.0], bound(0.0))
            }
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
        "semi" => {
            let s1 = num("surf").unwrap_or(1.0);
            let s2 = num("to").unwrap_or(s1);
            let p = [s1, s2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            match bound(0.0) {
                Bound::Side("OPGT", b) => ("MNSD", p, Bound::Target(b)),
                Bound::Side(_, b) => ("MXSD", p, Bound::Target(b)),
                _ => return Err("a semi term needs min or max".into()),
            }
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
        // 厚みの和の広がりの max は ZTHI
        "spread" => {
            let inner = t.get("term").ok_or("no term")?;
            let thick = inner.get("type").and_then(Value::as_str) == Some("thickness")
                && inner.get("edge").and_then(Value::as_bool) != Some(true)
                && inner.get("each").is_none();
            match (thick, bound(0.0)) {
                (true, Bound::Side("OPLT", b)) => {
                    let s1 = inner.get("surf").and_then(Value::as_f64).unwrap_or(1.0);
                    let s2 = inner.get("to").and_then(Value::as_f64).unwrap_or(s1);
                    (
                        "ZTHI",
                        [s1, s2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        Bound::Target(b),
                    )
                }
                _ => {
                    return Err(
                        "only the max of the spread of a thickness term can be written (ZTHI)"
                            .into(),
                    )
                }
            }
        }
        "surface" if t.get("to").is_some() => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let s1 = num("surf").unwrap_or(1.0);
            let s2 = num("to").unwrap_or(s1);
            let p = [s1, s2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            let (mn, mx) = match q {
                "semi" => ("MNSD", "MXSD"),
                "nd" => ("MNIN", "MXIN"),
                "vd" => ("MNAB", "MXAB"),
                "dpgf" => ("MNPD", "MXPD"),
                _ => return Err(format!("surface quantity {q} with to")),
            };
            match bound(0.0) {
                Bound::Side("OPGT", b) => (mn, p, Bound::Target(b)),
                Bound::Side(_, b) => (mx, p, Bound::Target(b)),
                _ => return Err("a surface term with to needs min or max".into()),
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
                "total" => {
                    // Code 0〜3 = +y・+x・−y・−x の向き
                    p[1] = match (x, y) {
                        (0.0, y) if y >= 0.0 => 0.0,
                        (x, 0.0) if x > 0.0 => 1.0,
                        (0.0, _) => 2.0,
                        (_, 0.0) => 3.0,
                        _ => return Err(
                            "only the total thickness along +y, +x, −y or −x can be written (TTVA)"
                                .into(),
                        ),
                    };
                    ("TTVA", "TTGT", "TTLT")
                }
                "param" => {
                    p[1] = num("param").unwrap_or(0.0);
                    ("PMVA", "PMGT", "PMLT")
                }
                "gx" => ("GLCX", "", ""),
                "gy" => ("GLCY", "", ""),
                "gz" => ("GLCZ", "", ""),
                "ga" => ("GLCA", "", ""),
                "gb" => ("GLCB", "", ""),
                "gc" => ("GLCC", "", ""),
                "cost" => ("GCOS", "", ""),
                "tce" => ("GTCE", "", ""),
                "surf_tce" => ("TCVA", "TCGT", "TCLT"),
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
        "tsag" => {
            let v3 = |k: &str| -> [f64; 3] {
                let a = t.get(k).and_then(Value::as_array);
                let x = |i: usize| {
                    a.and_then(|a| a.get(i))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                };
                [x(0), x(1), x(2)]
            };
            let (pt, tl) = (v3("point"), v3("tilt"));
            let flat = t.get("flat") == Some(&Value::Bool(true));
            let p = [
                num("surf").unwrap_or(1.0),
                f64::from(!flat),
                pt[0],
                pt[1],
                pt[2],
                tl[0],
                tl[1],
                tl[2],
            ];
            ("TSAG", p, bound(0.0))
        }
        "bfs" => {
            let q = t.get("q").and_then(Value::as_str).unwrap_or("");
            let Some(d) = super::shape::BFSD.iter().position(|x| *x == q) else {
                return Err(format!("bfs q {q}"));
            };
            let p = [
                num("surf").unwrap_or(1.0),
                d as f64,
                num("rmin").unwrap_or(0.0),
                num("rmax").unwrap_or(0.0),
                0.0,
                0.0,
                0.0,
                0.0,
            ];
            ("BFSD", p, bound(0.0))
        }
        "volume" => {
            let s1 = num("surf").unwrap_or(1.0);
            let s2 = num("to").unwrap_or(s1);
            let clear = t.get("clear") == Some(&Value::Bool(true));
            if t.get("cylinder") == Some(&Value::Bool(true)) {
                // CVOL は面 Surf1 から Surf2 を包む円柱の mm³
                let p = [s1, s2 + 1.0, 0.0, f64::from(clear), 0.0, 0.0, 0.0, 0.0];
                let b = match bound(0.0) {
                    Bound::Target(x) => Bound::Target(x * 1e3),
                    Bound::Side(o, x) => Bound::Side(o, x * 1e3),
                    b => b,
                };
                return Ok(("CVOL", p, b));
            }
            let op = if t.get("mass") == Some(&Value::Bool(true)) {
                "TMAS"
            } else {
                "VOLU"
            };
            let p = [s1, s2, 0.0, f64::from(clear), 0.0, 0.0, 0.0, 0.0];
            (op, p, bound(0.0))
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
