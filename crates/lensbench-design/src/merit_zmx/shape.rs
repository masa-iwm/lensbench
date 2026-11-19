//! 形の行（TTGT・TTLT・TTVA の包む厚み、PMGT・PMLT・PMVA の面タイプのパラメータ、VOLU・TMAS の体積・質量）を
//! surface・volume の項にする。OpticStudio 26.1 の実測（面はクリア半径の外で平ら）に合わせる

use super::{data, INT1, INT2, TARGET};
use lensbench_formats::zmx::merit::MeritRow;
use serde_json::{json, Map, Value};

/// BFSD の Data 0〜7 の bfs の項の q
pub(super) const BFSD: [&str; 8] = [
    "curv",
    "radius",
    "vertex",
    "depth",
    "volume",
    "slope",
    "depth_rms",
    "slope_rms",
];

/// BFSD の Data d の値のレンズ単位から mm への倍率
pub(super) fn bfsd_scale(d: i64, unit: f64) -> f64 {
    match d {
        0 => 1.0 / unit,
        4 => unit.powi(3),
        5 | 7 => 1.0,
        _ => unit,
    }
}

/// 1 つの行の項（種類のキーと目標）。写せなければ Err に理由
pub(super) fn kind(r: &MeritRow, unit: f64, image: usize) -> Result<Map<String, Value>, String> {
    let num = |i: usize| r.num(i).map_err(|e| e.to_string());
    let int = |i: usize| r.int(i).map_err(|e| e.to_string());
    let mut m = Map::new();
    let target = num(TARGET)?;
    let op = r.op.as_str();
    match op {
        // TTGT・TTLT・TTVA: Int1 の面の包む厚み（Int2 の Code 0〜3 = +y・+x・−y・−x の向き。Data2 の Mode は玉の半径か
        // クリア半径だが、面はクリア半径の外で平らなのでどちらも同じ値。OpticStudio 26.1 で実測）
        "TTGT" | "TTLT" | "TTVA" => {
            let j = int(INT1)?;
            if j < 1 || j as usize >= image {
                return Err(format!("surface {j}"));
            }
            let (x, y) = match int(INT2)? {
                0 => (0.0, 1.0),
                1 => (1.0, 0.0),
                2 => (0.0, -1.0),
                3 => (-1.0, 0.0),
                c => return Err(format!("code {c} (0 to 3)")),
            };
            m.insert("type".into(), json!("surface"));
            m.insert("surf".into(), json!(j));
            m.insert("q".into(), json!("total"));
            if (x, y) != (0.0, 1.0) {
                m.insert("x".into(), json!(x));
                m.insert("y".into(), json!(y));
            }
            let key = match op {
                "TTGT" => "min",
                "TTLT" => "max",
                _ => "target",
            };
            m.insert(key.into(), json!(target * unit));
        }
        // PMGT・PMLT・PMVA: Int1 の面のパラメータ Int2（.zmx の PARM の番号）
        "PMGT" | "PMLT" | "PMVA" => {
            let j = int(INT1)?;
            let n = int(INT2)?;
            if j < 0 || j as usize > image {
                return Err(format!("surface {j}"));
            }
            if !(0..=255).contains(&n) {
                return Err(format!("parameter {n}"));
            }
            m.insert("type".into(), json!("surface"));
            m.insert("surf".into(), json!(j));
            m.insert("q".into(), json!("param"));
            m.insert("param".into(), json!(n));
            let key = match op {
                "PMGT" => "min",
                "PMLT" => "max",
                _ => "target",
            };
            m.insert(key.into(), json!(target));
        }
        // VOLU・TMAS: Int1 から Int2 の面の要素の体積 [cm³]・硝材の質量 [g]（Data2 の Mode 0 は玉の半径、1 はクリア半径）
        "VOLU" | "TMAS" => {
            let s1 = int(INT1)?;
            let s2 = int(INT2)?;
            if s1 < 1 || s2 < s1 || s2 as usize >= image {
                return Err(format!("surfaces {s1} to {s2}"));
            }
            m.insert("type".into(), json!("volume"));
            m.insert("surf".into(), json!(s1));
            if s2 != s1 {
                m.insert("to".into(), json!(s2));
            }
            if op == "TMAS" {
                m.insert("mass".into(), json!(true));
            }
            match int(data(2))? {
                0 => {}
                1 => {
                    m.insert("clear".into(), json!(true));
                }
                k => return Err(format!("mode {k} (0 or 1)")),
            }
            m.insert("target".into(), json!(target));
        }
        // CVOL: Int1 から Int2 の面を包む最小の円柱の体積 [レンズ単位³]（頂点の位置と半径から、矢高は入れない。Data2 の Mode 0 は
        // 玉の半径、1 はクリア半径）。lensbench の項は面 Int1 から Int2 − 1 の要素を包む円柱の cm³
        "CVOL" => {
            let s1 = int(INT1)?;
            let s2 = int(INT2)?;
            if s1 < 1 || s2 <= s1 || s2 as usize > image - 1 {
                return Err(format!(
                    "surfaces {s1} to {s2} (the volume of one surface is 0)"
                ));
            }
            m.insert("type".into(), json!("volume"));
            m.insert("cylinder".into(), json!(true));
            m.insert("surf".into(), json!(s1));
            if s2 - 1 != s1 {
                m.insert("to".into(), json!(s2 - 1));
            }
            match int(data(2))? {
                0 => {}
                1 => {
                    m.insert("clear".into(), json!(true));
                }
                k => return Err(format!("mode {k} (0 or 1)")),
            }
            m.insert("target".into(), json!(target * unit.powi(3) * 1e-3));
        }
        // TSAG: Int1 の面の、点 (Data1, Data2, Data3) から傾き (Data4, Data5, Data6) [deg] の向きに面まで測った距離。Int2 の
        // Mode 0 は面をクリア半径の外で平らにする（玉の半径がクリア半径より大きい面だけ。OpticStudio 26.1 で実測、チップゾーンは
        // 読まない）、1 は面の式のまま
        "TSAG" => {
            let j = int(INT1)?;
            if j < 1 || j as usize >= image {
                return Err(format!("surface {j}"));
            }
            let flat = match int(INT2)? {
                0 => true,
                1 => false,
                k => return Err(format!("mode {k} (0 or 1)")),
            };
            let p = [
                num(data(1))? * unit,
                num(data(2))? * unit,
                num(data(3))? * unit,
            ];
            let t = [num(data(4))?, num(data(5))?, num(data(6))?];
            m.insert("type".into(), json!("tsag"));
            m.insert("surf".into(), json!(j));
            if p != [0.0; 3] {
                m.insert("point".into(), json!(p));
            }
            if t != [0.0; 3] {
                m.insert("tilt".into(), json!(t));
            }
            if flat {
                m.insert("flat".into(), json!(true));
            }
            m.insert("target".into(), json!(target * unit));
        }
        // BFSD: Int1 の面の、半径 Data1〜Data2（両方 0 はクリア半径まで）の最適球の量 Int2（0〜7）
        "BFSD" => {
            let j = int(INT1)?;
            if j < 1 || j as usize >= image {
                return Err(format!("surface {j}"));
            }
            let d = int(INT2)?;
            let Some(q) = usize::try_from(d).ok().and_then(|d| BFSD.get(d)) else {
                return Err(format!("data {d} (0 to 7)"));
            };
            let (a, b) = (num(data(1))?, num(data(2))?);
            m.insert("type".into(), json!("bfs"));
            m.insert("surf".into(), json!(j));
            m.insert("q".into(), json!(q));
            if a != 0.0 {
                m.insert("rmin".into(), json!(a * unit));
            }
            if b != 0.0 {
                m.insert("rmax".into(), json!(b * unit));
            }
            m.insert("target".into(), json!(target * bfsd_scale(d, unit)));
        }
        _ => unreachable!("not a shape row"),
    }
    Ok(m)
}
