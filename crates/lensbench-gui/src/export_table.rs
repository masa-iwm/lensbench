//! 解析の結果（JSON）を表にして、テキスト（タブ区切り）・CSV で写す・保存する（#128）。
//!
//! 表の作り方（どの解析にも同じ規則）:
//! - オブジェクトの数・文字列などの値 → その階層の「項目・値」の表
//! - 同じ長さの数の並びが 2 つ以上あるオブジェクト（MTF の freqs・tangential・sagittal など）→ 並びの番号ごとに 1 行。
//!   同じオブジェクトの数・文字列の値は各行に繰り返す
//! - オブジェクトの並び（視野ごとのスポットなど）→ 1 つの表に（各オブジェクトを上の規則で行にして縦に積む）
//! - さらに中の並び・オブジェクトは、名前をつないだ別の表

use serde_json::{Map, Value};

/// 表 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// 結果の中の場所（例 `mtf`、`spot`、空は一番上）
    pub name: String,
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// 値を 1 つの欄の文字に（数は丸めない）
fn cell(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some(String::new()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn is_nums(v: &Value) -> Option<usize> {
    let a = v.as_array()?;
    (a.len() >= 2 && a.iter().all(|x| x.is_number() || x.is_null())).then_some(a.len())
}

/// 短い数の並び（座標など）は列に広げる長さの上限
const SHORT: usize = 4;

/// オブジェクトを行に。返すのは（列、行、並びの番号ごとの行か、表にしなかった中身）。
/// - 数・文字列の値 → 列
/// - 長さ n > SHORT の数の並び（か、n > 2 で同じ長さが 3 つ以上: MTF の freqs・T・S）→ 番号ごとの行（一番数の多い長さ）
/// - ほかの短い数の並び（座標など）→ `名前[1]`・`名前[2]` … の列
#[allow(clippy::type_complexity)]
fn object_rows(
    o: &Map<String, Value>,
) -> (Vec<String>, Vec<Vec<String>>, bool, Vec<(String, Value)>) {
    let mut lens: Vec<(usize, usize)> = Vec::new(); // (長さ, 数)
    for v in o.values() {
        if let Some(n) = is_nums(v) {
            match lens.iter_mut().find(|x| x.0 == n) {
                Some(x) => x.1 += 1,
                None => lens.push((n, 1)),
            }
        }
    }
    let n = lens
        .iter()
        .filter(|x| x.0 > SHORT || (x.0 > 2 && x.1 >= 3))
        .max_by_key(|x| (x.1, x.0))
        .map(|x| x.0);
    let mut cols = Vec::new();
    let mut base = Vec::new();
    let mut arrs: Vec<(&String, &Vec<Value>)> = Vec::new();
    let mut rest = Vec::new();
    for (k, v) in o {
        if let Some(c) = cell(v) {
            cols.push(k.clone());
            base.push(c);
        } else if n.is_some() && is_nums(v) == n {
            arrs.push((k, v.as_array().unwrap()));
        } else if is_nums(v).is_some_and(|m| m <= SHORT) {
            for (i, x) in v.as_array().unwrap().iter().enumerate() {
                cols.push(format!("{k}[{}]", i + 1));
                base.push(cell(x).unwrap_or_default());
            }
        } else {
            rest.push((k.clone(), v.clone()));
        }
    }
    cols.extend(arrs.iter().map(|x| x.0.clone()));
    let rows = match n {
        Some(n) => (0..n)
            .map(|i| {
                let mut r = base.clone();
                r.extend(arrs.iter().map(|(_, a)| cell(&a[i]).unwrap_or_default()));
                r
            })
            .collect(),
        None if !base.is_empty() => vec![base],
        None => Vec::new(),
    };
    (cols, rows, n.is_some(), rest)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}.{b}")
    }
}

fn walk(name: &str, v: &Value, top: bool, out: &mut Vec<Table>) {
    match v {
        Value::Object(o) => {
            let (cols, rows, series, rest) = object_rows(o);
            if !rows.is_empty() {
                if series || !top {
                    out.push(Table {
                        name: name.to_string(),
                        cols,
                        rows,
                    });
                } else {
                    // 一番上の値は「項目・値」の表に
                    out.push(Table {
                        name: name.to_string(),
                        cols: vec!["key".into(), "value".into()],
                        rows: cols
                            .into_iter()
                            .zip(rows.into_iter().next().unwrap_or_default())
                            .map(|(k, v)| vec![k, v])
                            .collect(),
                    });
                }
            }
            for (k, x) in rest {
                walk(&join(name, &k), &x, false, out);
            }
        }
        Value::Array(a) if a.iter().all(|x| x.is_object()) && !a.is_empty() => {
            let mut cols: Vec<String> = Vec::new();
            let mut parts = Vec::new();
            let mut rests = Vec::new();
            for x in a {
                let (c, r, _, rest) = object_rows(x.as_object().unwrap());
                for k in &c {
                    if !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
                parts.push((c, r));
                rests.push(rest);
            }
            let mut rows = Vec::new();
            for (c, r) in parts {
                for row in r {
                    rows.push(
                        cols.iter()
                            .map(|k| {
                                c.iter()
                                    .position(|x| x == k)
                                    .map(|i| row[i].clone())
                                    .unwrap_or_default()
                            })
                            .collect(),
                    );
                }
            }
            if !rows.is_empty() {
                out.push(Table {
                    name: name.to_string(),
                    cols,
                    rows,
                });
            }
            for (i, rest) in rests.into_iter().enumerate() {
                for (k, x) in rest {
                    walk(&join(&format!("{name}[{}]", i + 1), &k), &x, false, out);
                }
            }
        }
        Value::Array(a) => {
            if let Some(_n) = is_nums(v) {
                out.push(Table {
                    name: name.to_string(),
                    cols: vec![name.rsplit('.').next().unwrap_or(name).to_string()],
                    rows: a
                        .iter()
                        .map(|x| vec![cell(x).unwrap_or_default()])
                        .collect(),
                });
            } else {
                for (i, x) in a.iter().enumerate() {
                    walk(&format!("{name}[{}]", i + 1), x, false, out);
                }
            }
        }
        _ => {}
    }
}

/// 結果を表に
pub fn tables(v: &Value) -> Vec<Table> {
    let mut out = Vec::new();
    walk("", v, true, &mut out);
    out
}

/// 表を区切りの文字 sep で（CSV は ','、テキストは '\t'）。表が 2 つ以上なら、表ごとに `# 名前` の行と空行
pub fn delimited(ts: &[Table], sep: char) -> String {
    let q = |s: &str| -> String {
        if s.contains(sep) || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    let many = ts.len() > 1;
    for (i, t) in ts.iter().enumerate() {
        if many {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!(
                "# {}\n",
                if t.name.is_empty() { "-" } else { &t.name }
            ));
        }
        let line = |r: &[String]| {
            r.iter()
                .map(|x| q(x))
                .collect::<Vec<_>>()
                .join(&sep.to_string())
        };
        out.push_str(&line(&t.cols));
        out.push('\n');
        for r in &t.rows {
            out.push_str(&line(r));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// MTF: 周波数ごとの行に、視野と T・S が並ぶ
    #[test]
    fn mtf_rows_per_freq() {
        let v = json!({"unit": "cycles/mm", "mtf": [
            {"field": 1, "freqs": [0.0, 10.0, 20.0], "tangential": [1.0, 0.5, 0.2], "sagittal": [1.0, 0.6, 0.3]},
            {"field": 2, "freqs": [0.0, 10.0, 20.0], "tangential": [1.0, 0.4, 0.1], "sagittal": [1.0, 0.3, 0.1]}
        ]});
        let t = tables(&v);
        assert_eq!(t[0].cols, ["key", "value"]);
        assert_eq!(t[0].rows, [["unit", "cycles/mm"]]);
        assert_eq!(t[1].name, "mtf");
        assert_eq!(t[1].cols, ["field", "freqs", "tangential", "sagittal"]);
        assert_eq!(t[1].rows.len(), 6);
        assert_eq!(t[1].rows[4], ["2", "10.0", "0.4", "0.3"]);
        let csv = delimited(&t, ',');
        assert!(csv.contains("# mtf\nfield,freqs,tangential,sagittal\n1,0.0,1.0,1.0\n"));
        let tsv = delimited(&t[1..], '\t');
        assert!(tsv.starts_with("field\tfreqs\ttangential\tsagittal\n"));
    }

    /// 視野ごとのオブジェクト（スポット）は 1 行ずつ、中の並びは別の表
    #[test]
    fn spot_rows_and_nested() {
        let v = json!({"unit": "lens", "spot": [
            {"field": 1, "rms": 0.01, "centroid": [0.0, 0.1], "chief": [0.0, 0.0]},
            {"field": 2, "rms": 0.02, "centroid": [0.0, 0.2], "chief": [0.0, 0.0]}
        ]});
        let t = tables(&v);
        let s = t.iter().find(|x| x.name == "spot").unwrap();
        // centroid は長さ 2 の並び（座標）→ 列に広げる
        assert_eq!(
            s.cols,
            [
                "field",
                "rms",
                "centroid[1]",
                "centroid[2]",
                "chief[1]",
                "chief[2]"
            ]
        );
        assert_eq!(s.rows.len(), 2);
        assert_eq!(s.rows[1][..4], ["2", "0.02", "0.0", "0.2"]);
        // 文字の , は CSV で囲む
        let q = delimited(
            &[Table {
                name: String::new(),
                cols: vec!["a".into()],
                rows: vec![vec!["x,y".into()]],
            }],
            ',',
        );
        assert_eq!(q, "a\n\"x,y\"\n");
    }
}
