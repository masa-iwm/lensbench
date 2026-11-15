//! カタログに無いガラスで開けなかった系を、モデルガラスで代用して開く（GUX-01）

/// エラーの文から、カタログに無いガラスの名前（「ガラス N-SK16（面 1）がカタログにない」、英語の表示では
/// "glass N-SK16 (surface 1) is not in the catalogs"・"no glass XYZ9 (…)" など）
pub(crate) fn missing_glass_name(e: &str) -> Option<String> {
    if let Some(i) = e.find("ガラス ") {
        let rest = &e[i + "ガラス ".len()..];
        let end = rest.find(['（', ' ', '）'])?;
        let tail = &rest[end..];
        return (tail.contains("がカタログにない") || tail.contains("がない"))
            .then(|| rest[..end].to_string());
    }
    // 「glass 名前 (surface 1) is not in the catalogs」は終わりの句の手前から探す（前置きのファイル名に
    // glass があっても取り違えない）。「no glass 名前 (…)」は no glass の後
    let (i, len) = match e.find(" is not in the catalogs") {
        Some(k) => (e[..k].rfind("glass ")?, "glass ".len()),
        None => (e.find("no glass ")?, "no glass ".len()),
    };
    let rest = &e[i + len..];
    let end = rest.find([' ', '(', ')']).unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
}

/// .zmx の GLAS の行のうち names のガラスを、行にある nd・νd のモデルガラス（___BLANK、ソルブ 1）に替える。
/// 替えた名前（nd が無い行は替えない）
pub(crate) fn model_glass_zmx(text: &str, names: &[String]) -> (String, Vec<String>) {
    let mut done = Vec::new();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let hit = t.first() == Some(&"GLAS")
            && t.get(1)
                .is_some_and(|n| names.iter().any(|x| x.eq_ignore_ascii_case(n)))
            && t.get(2) != Some(&"2")
            && t.get(4)
                .and_then(|x| x.parse::<f64>().ok())
                .is_some_and(|nd| nd > 1.0);
        if hit {
            let mut u: Vec<String> = t.iter().map(|x| x.to_string()).collect();
            done.push(u[1].clone());
            u[1] = "___BLANK".into();
            u[2] = "1".into();
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(indent);
            out.push_str(&u.join(" "));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    (out, done)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// エラーの文から無いガラスの名前を取り、.zmx の GLAS の行をモデルガラスに替える（GUX-01）
    #[test]
    fn model_glass_substitution() {
        assert_eq!(
            missing_glass_name("load: 不正: ガラス N-SK16（面 1）がカタログにない").as_deref(),
            Some("N-SK16")
        );
        assert_eq!(
            missing_glass_name("edit: 面 3: 不正: ガラス XYZ9 がない（…）").as_deref(),
            Some("XYZ9")
        );
        assert_eq!(missing_glass_name("ほかのエラー"), None);
        assert_eq!(
            missing_glass_name(
                "load: invalid: glass N-SK16 (surface 1) is not in the catalogs. Pass …"
            )
            .as_deref(),
            Some("N-SK16")
        );
        assert_eq!(
            missing_glass_name("edit: surface 3: invalid: no glass XYZ9 (add catalogs: …)")
                .as_deref(),
            Some("XYZ9")
        );
        assert_eq!(missing_glass_name("glass catalog folder"), None);
        assert_eq!(
            missing_glass_name(
                "my glass test.zmx: invalid: glass N-SK16 (surface 1) is not in the catalogs"
            )
            .as_deref(),
            Some("N-SK16")
        );
        let z = "SURF 1\n  GLAS N-SK16 0 0 1.62041 60.32 0 0 0 0 0 0\nSURF 2\n  GLAS F2 2 1 0 0\n";
        let (out, done) = model_glass_zmx(z, &["N-SK16".into(), "F2".into()]);
        assert_eq!(done, vec!["N-SK16".to_string()]);
        assert!(out.contains("  GLAS ___BLANK 1 0 1.62041 60.32"));
        // ピックアップ（ソルブ 2）は替えない
        assert!(out.contains("GLAS F2 2 1 0 0"));
    }
}
