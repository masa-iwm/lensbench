//! 結果の JSON の文字の出力（`--json` なし）

use serde_json::Value;

/// 1 行を表示の言語で出す（先頭の字下げはそのまま、残りを訳す）
pub(crate) fn say(s: &str) {
    let body = s.trim_start();
    let pad = &s[..s.len() - body.len()];
    println!("{pad}{}", lensbench_core::i18n::tr(body));
}

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

/// 公差の結果（`merit_values` を持つ）を画面用に要点だけにする。--save・serve・GUI には全体を渡す
pub(crate) fn tolerance_brief(out: &Value) -> Value {
    let mut out = out.clone();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    if !o.contains_key("merit_values") {
        return out;
    }
    o.remove("cdf");
    o.remove("tolerances");
    o.remove("merit_values");
    for k in ["compensators", "tilt_magnitudes"] {
        if let Some(Value::Array(a)) = o.get_mut(k) {
            for t in a {
                if let Some(t) = t.as_object_mut() {
                    t.remove("values");
                }
            }
        }
    }
    for k in ["sensitivity", "correlation"] {
        if let Some(Value::Array(a)) = o.get_mut(k) {
            a.truncate(10);
            // 項目名は英語が元（JSON はそのまま）。文字の出力では表示の言語で
            for t in a.iter_mut() {
                if let Some(l) = t.get_mut("label") {
                    if let Some(x) = l.as_str() {
                        *l = Value::String(lensbench_core::i18n::tr_item(x));
                    }
                }
            }
        }
    }
    if let Some(Value::Array(a)) = o.get_mut("terms") {
        for t in a {
            if let Some(t) = t.as_object_mut() {
                t.remove("cdf");
            }
        }
    }
    out
}

/// .zmx に書けなかったもの（[`lensbench_formats::zmx_write::write_notes`]）を標準エラーに出す
pub(crate) fn warn_notes(notes: Vec<String>) {
    for n in notes {
        eprintln!("{}", lensbench_core::i18n::warning(n));
    }
}
