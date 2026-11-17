//! 取った品の STEP の立体を、書き出す STEP に写す。
//!
//! 品の STEP の殻（`CLOSED_SHELL`）から辿れる実体を番号を付け直して写し、点（`CARTESIAN_POINT`）は置き方の
//! 変換で動かし、向き（`DIRECTION`）は回し、長さ（円・円柱の半径、`VECTOR` の大きさなど）は単位の倍率を掛ける
//! （書き出す STEP は mm。複合の実体 `(A(…) B(…))` は部分ごとに）。面の上の 2 次元の曲線（`PCURVE`）は単位を直せないので写さず、辺の曲線は
//! 3 次元の曲線（`SURFACE_CURVE`・`SEAM_CURVE` の 1 つ目）にする。
use lensbench_core::math::V3;
use lensbench_step::{refs, split, StepFile};
use std::collections::{BTreeSet, HashMap};

/// 品の STEP（[`crate::detail::Detail::step`]）
#[derive(Debug, Clone)]
pub struct StepSource {
    /// 読んだ STEP（殻から辿れる実体だけ。[`StepFile::subset`]）。部品ごとに読み直さない
    pub file: std::sync::Arc<StepFile>,
    /// 長さの単位の倍率（mm へ）
    pub unit_mm: f64,
    /// 殻の実体の番号と、STEP の座標から部品の座標 [mm] への変換（3 行 4 列、回転 × 単位の倍率と移動）
    pub shells: Vec<(u64, [[f64; 4]; 3])>,
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
        "PARABOLA" => &[2],
        "ELLIPSE" | "HYPERBOLA" | "TOROIDAL_SURFACE" | "DEGENERATE_TOROIDAL_SURFACE" => &[2, 3],
        _ => &[],
    }
}

/// 複合の実体（`(A(…) B(…))`）の部分の長さの引数の位置。部分はその型が自分で足した引数だけを持つ
/// （CIRCLE の部分は半径だけ、など。継いだ名前・位置は上の型の部分にある）
fn own_length_args(ty: &str) -> &'static [usize] {
    match ty {
        "CIRCLE" | "PARABOLA" | "CYLINDRICAL_SURFACE" | "CONICAL_SURFACE" | "SPHERICAL_SURFACE" => {
            &[0]
        }
        "ELLIPSE" | "HYPERBOLA" | "TOROIDAL_SURFACE" => &[0, 1],
        "VECTOR" | "OFFSET_SURFACE" | "OFFSET_CURVE_3D" => &[1],
        _ => &[],
    }
}

/// 実数を持つが長さではない（単位を直さなくてよい）型: B スプラインの節点（径数）・重み、部分の型の名前だけのもの
fn unit_free(ty: &str) -> bool {
    matches!(
        ty,
        "B_SPLINE_CURVE_WITH_KNOTS"
            | "B_SPLINE_SURFACE_WITH_KNOTS"
            | "RATIONAL_B_SPLINE_CURVE"
            | "RATIONAL_B_SPLINE_SURFACE"
            | "B_SPLINE_CURVE"
            | "B_SPLINE_SURFACE"
            | "UNIFORM_CURVE"
            | "UNIFORM_SURFACE"
            | "QUASI_UNIFORM_CURVE"
            | "QUASI_UNIFORM_SURFACE"
            | "BEZIER_CURVE"
            | "BEZIER_SURFACE"
    )
}

/// 文字列の外に実数（小数点のある数）があるか
fn has_real(args: &str) -> bool {
    let b = args.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\'' {
            in_str = !in_str;
        } else if !in_str && c == b'#' {
            // 参照の番号は飛ばす
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        } else if !in_str && c.is_ascii_digit() {
            let st = i;
            while i < b.len()
                && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'E' | b'e' | b'+' | b'-'))
            {
                i += 1;
            }
            if args[st..i].contains('.') {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

/// 単位を直す表に無く、実数を持つ型（長さの引数かもしれないのに直さなかった）なら Some(型の名前)
fn unscaled_type<'a>(ty: &'a str, args: &[&str], own: bool) -> Option<&'a str> {
    let known = if own {
        !own_length_args(ty).is_empty()
    } else {
        !length_args(ty).is_empty()
    };
    (!known && !unit_free(ty) && args.iter().any(|a| has_real(a))).then_some(ty)
}

/// 複合の実体の本体（`(A(…) B(…))`）を部分 `A(…)` の並びに分ける。複合でなければ None
fn complex_parts(body: &str) -> Option<Vec<&str>> {
    let inner = body.trim().strip_prefix('(')?.strip_suffix(')')?;
    let b = inner.as_bytes();
    let mut parts = Vec::new();
    let (mut depth, mut in_str, mut st) = (0i64, false, None);
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\'' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    parts.push(inner[st?..=i].trim());
                    st = None;
                }
            }
            c if !in_str && depth == 0 && st.is_none() && !c.is_ascii_whitespace() => st = Some(i),
            _ => {}
        }
    }
    (depth == 0 && st.is_none()).then_some(parts)
}

/// 長さの引数 `idx` に単位の倍率 k を掛けた `ty(args)`
fn scaled(ty: &str, args: &[&str], idx: &[usize], k: f64, real: fn(f64) -> String) -> String {
    let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    for &i in idx {
        if let Some(x) = a.get(i).and_then(|s| s.parse::<f64>().ok()) {
            a[i] = real(x * k);
        }
    }
    format!("{ty}({})", a.join(","))
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
/// （回転の行列の列が軸、原点）。写せなければ None。単位の倍率 k が 1 でないとき、長さを直す表に無く実数を持つ
/// 型の名前を `unscaled` に足す（長さなら mm に直っていない）
#[allow(clippy::too_many_arguments)]
pub fn embed_shell(
    sink: &mut impl Sink,
    ents: &StepFile,
    (shell, m): (u64, &[[f64; 4]; 3]),
    k: f64,
    place: (&[[f64; 3]; 3], V3),
    name: &str,
    real: fn(f64) -> String,
    unscaled: &mut BTreeSet<String>,
) -> Option<usize> {
    // 辺の曲線: SURFACE_CURVE・SEAM_CURVE は 3 次元の曲線に置き換える
    let alias = |id: u64| -> u64 {
        ents.get(id)
            .and_then(|e| split(&e.body))
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
        let body = &ents.get(id)?.body;
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
        let body = &ents.get(id)?.body;
        let edited: String = match split(body) {
            Some((ty @ ("CARTESIAN_POINT" | "DIRECTION"), args)) => {
                let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                if let Some(v) = a.get(1).and_then(|s| nums(s)).filter(|v| v.len() == 3) {
                    let v = [v[0], v[1], v[2]];
                    a[1] = triple(if ty == "DIRECTION" { dir(v) } else { point(v) });
                }
                format!("{ty}({})", a.join(","))
            }
            Some((ty, args)) => {
                if k != 1.0 {
                    unscaled.extend(unscaled_type(ty, &args, false).map(String::from));
                }
                scaled(ty, &args, length_args(ty), k, real)
            }
            // 複合の実体: 部分ごとに長さを直す
            None => match complex_parts(body) {
                Some(parts) => {
                    let v: Vec<String> = parts
                        .iter()
                        .map(|p| match split(p) {
                            Some((ty, args)) => {
                                if k != 1.0 {
                                    unscaled
                                        .extend(unscaled_type(ty, &args, true).map(String::from));
                                }
                                scaled(ty, &args, own_length_args(ty), k, real)
                            }
                            None => p.to_string(),
                        })
                        .collect();
                    format!("({})", v.join(" "))
                }
                None => body.clone(),
            },
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

    fn real(x: f64) -> String {
        format!("{x:?}")
    }

    /// 双曲線・放物線と、複合の実体の部分の長さにも単位の倍率を掛ける
    #[test]
    fn lengths_in_conics_and_complex_entities_are_scaled() {
        let (ty, a) = split("HYPERBOLA('',#5,2.,1.5)").unwrap();
        assert_eq!(
            scaled(ty, &a, length_args(ty), 25.4, real),
            "HYPERBOLA('',#5,50.8,38.099999999999994)"
        );
        let (ty, a) = split("PARABOLA('',#5,1.)").unwrap();
        assert_eq!(
            scaled(ty, &a, length_args(ty), 25.4, real),
            "PARABOLA('',#5,25.4)"
        );
        let body = "( CONIC() CIRCLE(2.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() REPRESENTATION_ITEM('') )";
        let parts = complex_parts(body).unwrap();
        assert_eq!(parts.len(), 5);
        let (ty, a) = split(parts[1]).unwrap();
        assert_eq!(
            scaled(ty, &a, own_length_args(ty), 25.4, real),
            "CIRCLE(50.8)"
        );
        assert!(complex_parts("CIRCLE('',#1,2.)").is_none());
        assert_eq!(
            complex_parts("(A('x)y') B(1,(2,3)))").unwrap(),
            ["A('x)y')", "B(1,(2,3))"]
        );
    }

    /// 長さを直す表に無く実数を持つ型を拾う。B スプラインの節点・参照の番号・文字列の中の数は数えない
    #[test]
    fn unknown_types_with_reals_are_reported() {
        let t = |b: &str, own: bool| {
            let (ty, a) = split(b).unwrap();
            unscaled_type(ty, &a, own).map(String::from)
        };
        assert_eq!(
            t(
                "TRIMMED_CURVE('',#4,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(2.5)),.T.,.PARAMETER.)",
                false
            )
            .as_deref(),
            Some("TRIMMED_CURVE")
        );
        assert_eq!(t("B_SPLINE_CURVE_WITH_KNOTS('',3,(#1,#2,#3,#4),.UNSPECIFIED.,.F.,.F.,(4,4),(0.,1.),.UNSPECIFIED.)", false), None);
        assert_eq!(t("CIRCLE('',#5,2.)", false), None);
        assert_eq!(t("EDGE_CURVE('1.5',#12,#13,#14,.T.)", false), None);
        assert_eq!(t("SURFACE_OF_REVOLUTION('',#3,#4)", false), None);
        assert_eq!(t("CIRCLE(2.)", true), None);
        assert_eq!(t("MYSTERY(1.5)", true).as_deref(), Some("MYSTERY"));
    }
}
