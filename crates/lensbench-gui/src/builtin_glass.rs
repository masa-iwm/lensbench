//! 同梱のガラスカタログ（SCHOTT・OHARA の Sellmeier のガラス、refractiveindex.info の CC0 のデータから
//! `scripts/make_builtin_glass.py` で作る）。何も設定していなくても普通の .zmx が開けるように、設定のガラスの
//! フォルダの後ろに足す（同じ名前の AGF は設定のフォルダのものが先、GUX-01）。
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;

const FILES: [(&str, &str); 2] = [
    ("SCHOTT.AGF", include_str!("../assets/glass/SCHOTT.AGF")),
    ("OHARA.AGF", include_str!("../assets/glass/OHARA.AGF")),
];

static DIR: OnceLock<String> = OnceLock::new();

/// 起動時に 1 回: `base`（設定のフォルダ。無ければ一時フォルダ）の `glass-builtin` に書く（中身が同じなら書かない）。
/// 書けなければ足さない
pub fn install(base: Option<PathBuf>) {
    let d = base
        .unwrap_or_else(|| std::env::temp_dir().join("lensbench"))
        .join("glass-builtin");
    let ok = std::fs::create_dir_all(&d).is_ok()
        && FILES.iter().all(|(n, t)| {
            let p = d.join(n);
            std::fs::read(&p).is_ok_and(|b| b == t.as_bytes()) || std::fs::write(&p, t).is_ok()
        });
    if ok {
        let _ = DIR.set(d.display().to_string());
    }
}

/// serve に渡すガラスのフォルダ: 設定のフォルダの後ろに同梱のフォルダ
pub fn dirs(v: &[String]) -> Value {
    let mut out = v.to_vec();
    if let Some(d) = DIR.get() {
        if !out.contains(d) {
            out.push(d.clone());
        }
    }
    json!(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同梱の AGF は読めて、基準の系のガラス（N-BK7・F2・N-SK16・N-LAK9・S-FSL5）を含む（GUX-01）
    #[test]
    fn gux01_builtin_catalogs_parse_and_cover_common_glasses() {
        let s = lensbench_core::glass::parse_agf(FILES[0].1, "SCHOTT").unwrap();
        let o = lensbench_core::glass::parse_agf(FILES[1].1, "OHARA").unwrap();
        assert!(s.len() > 100 && o.len() > 100, "{} {}", s.len(), o.len());
        for n in ["N-BK7", "F2", "N-SK16", "N-LAK9"] {
            assert!(s.contains_key(n), "{n}");
        }
        assert!(o.contains_key("S-FSL5"));
        let n = s["N-BK7"].index(0.5875618, 20.0, 1.0, false);
        assert!((n - 1.5168).abs() < 2e-5, "{n}");
        let d = std::env::temp_dir().join(format!("lb-gux01-{}", std::process::id()));
        install(Some(d.clone()));
        let v = dirs(&["a".into()]);
        assert_eq!(v[0], "a");
        assert!(v[1].as_str().unwrap().ends_with("glass-builtin"), "{v}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
