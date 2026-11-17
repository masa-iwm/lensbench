//! カタログに無いガラスで開けなかった系を、モデルガラスで代用して開く

/// エラーの文から、カタログに無いガラスの名前（「硝材 N-SK16（面 1）がカタログにありません」、英語の表示では
/// "glass N-SK16 (surface 1) is not in the catalogs"・"no glass XYZ9 (…)" など）
pub(crate) fn missing_glass_name(e: &str) -> Option<String> {
    for word in ["硝材 ", "ガラス "] {
        if let Some(i) = e.find(word) {
            let rest = &e[i + word.len()..];
            let end = rest.find(['（', ' ', '）'])?;
            let tail = &rest[end..];
            return [
                "がカタログにありません",
                "がありません",
                "がカタログにない",
                "がない",
            ]
            .iter()
            .any(|t| tail.contains(t))
            .then(|| rest[..end].to_string());
        }
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

/// 開けなかった系の誤りから、カタログに無いガラスの名前（符号が別の種類なら None）
pub(crate) fn missing_glass_of(e: &crate::worker::CallErr) -> Option<String> {
    if e.code.is_some() && !e.is(lensbench_core::error::code::GLASS_NOT_FOUND) {
        return None;
    }
    missing_glass_name(e)
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

    /// 符号があれば、ガラスが無い誤りのときだけ名前を探す（文にガラスの言葉があっても）
    #[test]
    fn missing_glass_follows_the_code() {
        use crate::worker::CallErr;
        let m = "glass N-BK7 (surface 1) is not in the catalogs";
        assert_eq!(
            missing_glass_of(&CallErr::coded(m, "glass.not_found")).as_deref(),
            Some("N-BK7")
        );
        assert_eq!(missing_glass_of(&CallErr::coded(m, "file.not_found")), None);
        assert_eq!(missing_glass_of(&m.into()).as_deref(), Some("N-BK7"));
    }

    /// エラーの文から無いガラスの名前を取り、.zmx の GLAS の行をモデルガラスに替える
    #[test]
    fn model_glass_substitution() {
        assert_eq!(
            missing_glass_name("load: 硝材 N-SK16（面 1）がカタログにありません").as_deref(),
            Some("N-SK16")
        );
        assert_eq!(
            missing_glass_name("edit: 面 3: 硝材 XYZ9 がありません（…）").as_deref(),
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
