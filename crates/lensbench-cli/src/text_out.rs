//! 結果の JSON の文字の出力（`--json` なし）

use serde_json::Value;

/// JSON をそのまま読める形で出す（GUI ができるまでの簡易表示）
pub(crate) fn print_text(w: &mut dyn std::io::Write, v: &Value, ind: usize) -> std::io::Result<()> {
    let pad = "  ".repeat(ind);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) if !is_flat(x) => {
                        writeln!(w, "{pad}{k}:")?;
                        print_text(w, x, ind + 1)?;
                    }
                    _ => writeln!(w, "{pad}{k}: {}", fmt(x))?,
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                writeln!(w, "{pad}-")?;
                print_text(w, x, ind + 1)?;
            }
        }
        _ => writeln!(w, "{pad}{}", fmt(v))?,
    }
    Ok(())
}

fn is_flat(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|x| x.is_number() || is_flat_pair(x)),
        _ => false,
    }
}

fn is_flat_pair(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.iter().all(Value::is_number))
}

/// 数の表示: 有効 6 桁。ふつうの大きさ（1e-4 〜 1e6）は小数で、ほかは指数で
fn fmt_f64(f: f64) -> String {
    if f == 0.0 || !f.is_finite() {
        return format!("{f}");
    }
    let a = f.abs();
    if (1e-4..1e6).contains(&a) {
        let digits = (5 - a.log10().floor() as i32).max(0) as usize;
        let t = format!("{f:.digits$}");
        if t.contains('.') {
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            t
        }
    } else {
        format!("{f:.5e}")
    }
}

fn fmt(v: &Value) -> String {
    match v {
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                fmt_f64(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::Array(a) => format!("[{}]", a.iter().map(fmt).collect::<Vec<_>>().join(", ")),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
