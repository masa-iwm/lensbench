//! 取った品の STEP の立体を、書き出す STEP に写す（#121）。
//!
//! 品の STEP の殻（`CLOSED_SHELL`）から辿れる実体を番号を付け直して写し、点（`CARTESIAN_POINT`）は置き方の
//! 変換で動かし、向き（`DIRECTION`）は回し、長さ（円・円柱の半径、`VECTOR` の大きさなど）は単位の倍率を掛ける
//! （書き出す STEP は mm）。面の上の 2 次元の曲線（`PCURVE`）は単位を直せないので写さず、辺の曲線は
//! 3 次元の曲線（`SURFACE_CURVE`・`SEAM_CURVE` の 1 つ目）にする。
use lensbench_core::math::V3;
use std::collections::HashMap;

/// 品の STEP（[`crate::detail::Detail::step`]）
#[derive(Debug, Clone)]
pub struct StepSource {
    /// STEP の文字（全部）
    pub text: std::sync::Arc<String>,
    /// 長さの単位の倍率（mm へ）
    pub unit_mm: f64,
    /// 殻の実体の番号と、STEP の座標から部品の座標 [mm] への変換（3 行 4 列、回転 × 単位の倍率と移動）
    pub shells: Vec<(u64, [[f64; 4]; 3])>,
}

/// 実体の番号 → 本体の文字（`=` の後ろ、`;` の前）
pub fn entities(text: &str) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    let text = strip_comments(text);
    let text = text.as_str();
    let data = match text.find("DATA;") {
        Some(i) => &text[i + 5..],
        None => text,
    };
    let b = data.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // 次の #番号=
        while i < b.len() && b[i] != b'#' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let st = i + 1;
        let mut j = st;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let id: Option<u64> = data[st..j].parse().ok();
        let mut k = j;
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if id.is_none() || k >= b.len() || b[k] != b'=' {
            i = j.max(i + 1);
            continue;
        }
        // 本体: 文字列の外の ; まで
        let body0 = k + 1;
        let mut q = body0;
        let mut in_str = false;
        while q < b.len() {
            match b[q] {
                b'\'' => in_str = !in_str,
                b';' if !in_str => break,
                _ => {}
            }
            q += 1;
        }
        out.insert(id.unwrap(), data[body0..q].trim().replace(['\r', '\n'], ""));
        i = q + 1;
    }
    out
}

/// 注釈 `/* … */` を空白に（文字 '…' の中は残す）。注釈の中の実体を読まないように
fn strip_comments(t: &str) -> String {
    if !t.contains("/*") {
        return t.to_string();
    }
    let mut out = String::with_capacity(t.len());
    let mut it = t.chars().peekable();
    let mut in_str = false;
    while let Some(c) = it.next() {
        if in_str {
            out.push(c);
            if c == '\'' {
                in_str = false;
            }
        } else if c == '\'' {
            in_str = true;
            out.push(c);
        } else if c == '/' && it.peek() == Some(&'*') {
            it.next();
            let mut prev = ' ';
            for d in it.by_ref() {
                if prev == '*' && d == '/' {
                    break;
                }
                prev = d;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// 本体の参照（文字列の外の #番号）
fn refs(body: &str) -> Vec<u64> {
    let b = body.as_bytes();
    let mut v = Vec::new();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' => in_str = !in_str,
            b'#' if !in_str => {
                let st = i + 1;
                let mut j = st;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if let Ok(n) = body[st..j].parse() {
                    v.push(n);
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    v
}

/// 単純な実体の型と、上の段の引数（文字のまま）。複合の実体（`(` で始まる）は None
fn split(body: &str) -> Option<(&str, Vec<&str>)> {
    let open = body.find('(')?;
    let ty = body[..open].trim();
    if ty.is_empty() || !body.ends_with(')') {
        return None;
    }
    let inner = &body[open + 1..body.len() - 1];
    let b = inner.as_bytes();
    let mut args = Vec::new();
    let (mut depth, mut in_str, mut st) = (0i32, false, 0);
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\'' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => depth -= 1,
            b',' if !in_str && depth == 0 => {
                args.push(inner[st..i].trim());
                st = i + 1;
            }
            _ => {}
        }
    }
    args.push(inner[st..].trim());
    Some((ty, args))
}

/// 長さの引数の位置（単位の倍率を掛ける）
fn length_args(ty: &str) -> &'static [usize] {
    match ty {
        "CIRCLE"
        | "CYLINDRICAL_SURFACE"
        | "CONICAL_SURFACE"
        | "SPHERICAL_SURFACE"
        | "VECTOR"
        | "OFFSET_SURFACE"
        | "OFFSET_CURVE_3D" => &[2],
        "ELLIPSE" | "TOROIDAL_SURFACE" | "DEGENERATE_TOROIDAL_SURFACE" => &[2, 3],
        _ => &[],
    }
}

fn nums(s: &str) -> Option<Vec<f64>> {
    let s = s.trim().strip_prefix('(')?.strip_suffix(')')?;
    s.split(',').map(|x| x.trim().parse::<f64>().ok()).collect()
}

/// 写した実体を足す先（番号は 1 から並ぶ。[`crate::step`] の実体の並び）
pub trait Sink {
    /// 次に足す実体の番号
    fn next_id(&self) -> usize;
    fn push(&mut self, body: String) -> usize;
}

/// 殻を 1 つ写し、`MANIFOLD_SOLID_BREP` の番号を返す。`place` は部品の座標 [mm] から書き出す座標への変換
/// （回転の行列の列が軸、原点）。写せなければ None
pub fn embed_shell(
    sink: &mut impl Sink,
    ents: &HashMap<u64, String>,
    (shell, m): (u64, &[[f64; 4]; 3]),
    k: f64,
    place: (&[[f64; 3]; 3], V3),
    name: &str,
    real: fn(f64) -> String,
) -> Option<usize> {
    // 辺の曲線: SURFACE_CURVE・SEAM_CURVE は 3 次元の曲線に置き換える
    let alias = |id: u64| -> u64 {
        ents.get(&id)
            .and_then(|b| split(b))
            .filter(|(t, _)| *t == "SURFACE_CURVE" || *t == "SEAM_CURVE")
            .and_then(|(_, a)| {
                a.get(1)
                    .and_then(|x| x.trim().strip_prefix('#')?.parse().ok())
            })
            .unwrap_or(id)
    };
    // 辿れる実体（深さ優先、写す順は葉から）
    let mut order: Vec<u64> = Vec::new();
    let mut seen: HashMap<u64, ()> = HashMap::new();
    let mut stack = vec![(alias(shell), false)];
    while let Some((id, done)) = stack.pop() {
        if done {
            order.push(id);
            continue;
        }
        if seen.insert(id, ()).is_some() {
            continue;
        }
        let body = ents.get(&id)?;
        stack.push((id, true));
        for r in refs(body) {
            let r = alias(r);
            if !seen.contains_key(&r) {
                stack.push((r, false));
            }
        }
    }
    // 番号の付け直し（葉から順に足す）
    let base = sink.next_id();
    let newid: HashMap<u64, usize> = order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, base + i))
        .collect();
    // 変換: 部品の座標 p = M·x、書き出す座標 = R·p + o
    let (r, o) = place;
    let lin = |x: V3| -> V3 {
        let p = [
            m[0][0] * x[0] + m[0][1] * x[1] + m[0][2] * x[2],
            m[1][0] * x[0] + m[1][1] * x[1] + m[1][2] * x[2],
            m[2][0] * x[0] + m[2][1] * x[1] + m[2][2] * x[2],
        ];
        [
            r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2],
            r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2],
            r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2],
        ]
    };
    let point = |x: V3| -> V3 {
        let a = lin(x);
        let t = [m[0][3], m[1][3], m[2][3]];
        let tt = [
            r[0][0] * t[0] + r[0][1] * t[1] + r[0][2] * t[2],
            r[1][0] * t[0] + r[1][1] * t[1] + r[1][2] * t[2],
            r[2][0] * t[0] + r[2][1] * t[1] + r[2][2] * t[2],
        ];
        [
            a[0] + tt[0] + o[0],
            a[1] + tt[1] + o[1],
            a[2] + tt[2] + o[2],
        ]
    };
    let dir = |x: V3| -> V3 {
        let a = lin(x);
        let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        if n > 0.0 {
            [a[0] / n, a[1] / n, a[2] / n]
        } else {
            a
        }
    };
    let triple = |v: V3| format!("({},{},{})", real(v[0]), real(v[1]), real(v[2]));
    for &id in &order {
        let body = &ents[&id];
        let edited: String = match split(body) {
            Some((ty, args)) => {
                let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                match ty {
                    "CARTESIAN_POINT" => {
                        if let Some(v) = a.get(1).and_then(|s| nums(s)).filter(|v| v.len() == 3) {
                            a[1] = triple(point([v[0], v[1], v[2]]));
                        }
                    }
                    "DIRECTION" => {
                        if let Some(v) = a.get(1).and_then(|s| nums(s)).filter(|v| v.len() == 3) {
                            a[1] = triple(dir([v[0], v[1], v[2]]));
                        }
                    }
                    _ => {
                        for &i in length_args(ty) {
                            if let Some(x) = a.get(i).and_then(|s| s.parse::<f64>().ok()) {
                                a[i] = real(x * k);
                            }
                        }
                    }
                }
                format!("{ty}({})", a.join(","))
            }
            None => body.clone(),
        };
        // 参照の番号を付け直す（文字列の外）
        let b = edited.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(edited.len());
        let (mut i, mut in_str) = (0, false);
        while i < b.len() {
            let c = b[i];
            if c == b'\'' {
                in_str = !in_str;
            }
            if c == b'#' && !in_str {
                let st = i + 1;
                let mut j = st;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let n: u64 = edited[st..j].parse().ok()?;
                out.extend_from_slice(format!("#{}", newid.get(&alias(n))?).as_bytes());
                i = j;
                continue;
            }
            out.push(c);
            i += 1;
        }
        sink.push(String::from_utf8_lossy(&out).into_owned());
    }
    let sh = newid.get(&alias(shell))?;
    Some(sink.push(format!("MANIFOLD_SOLID_BREP('{name}',#{sh})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_and_split() {
        let t = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=CARTESIAN_POINT('a;b',(1.,2.,\n3.));\n#2 = CIRCLE('',#3,2.5);\n#3=(A()B(#1));\nENDSEC;";
        let e = entities(t);
        assert_eq!(e[&1], "CARTESIAN_POINT('a;b',(1.,2.,3.))");
        assert_eq!(split(&e[&2]).unwrap(), ("CIRCLE", vec!["''", "#3", "2.5"]));
        assert!(split(&e[&3]).is_none());
        assert_eq!(refs(&e[&3]), vec![1]);
        // 注釈の中の実体は読まない（文字の中の /* は注釈でない）
        let t = "DATA;\n#5=CIRCLE('',#4,12.7);\n/* #5=CIRCLE('',#4,99.); */\n#6=X('/*');\nENDSEC;";
        let e = entities(t);
        assert_eq!(e[&5], "CIRCLE('',#4,12.7)");
        assert_eq!(e[&6], "X('/*')");
    }
}
