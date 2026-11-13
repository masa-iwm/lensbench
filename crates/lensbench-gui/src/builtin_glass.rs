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
    if write_to(&d) {
        let _ = DIR.set(d.display().to_string());
    }
}

/// フォルダ `d` に同梱の AGF を書く（中身が同じなら書かない）。書けたら true
fn write_to(d: &std::path::Path) -> bool {
    std::fs::create_dir_all(d).is_ok()
        && FILES.iter().all(|(n, t)| {
            let p = d.join(n);
            std::fs::read(&p).is_ok_and(|b| b == t.as_bytes()) || std::fs::write(&p, t).is_ok()
        })
}

/// serve に渡すガラスのフォルダ: 設定のフォルダの後ろに同梱のフォルダ
pub fn dirs(v: &[String]) -> Value {
    dirs_with(v, DIR.get().map(String::as_str))
}

fn dirs_with(v: &[String], builtin: Option<&str>) -> Value {
    let mut out = v.to_vec();
    if let Some(d) = builtin {
        if !out.iter().any(|x| x == d) {
            out.push(d.to_string());
        }
    }
    json!(out)
}

/// 同梱のカタログのガラスの名前（カタログ名、ガラス名）
fn names() -> &'static Vec<(String, String)> {
    static N: OnceLock<Vec<(String, String)>> = OnceLock::new();
    N.get_or_init(|| {
        FILES
            .iter()
            .flat_map(|(f, t)| {
                let cat = f.trim_end_matches(".AGF");
                t.lines()
                    .filter_map(|l| l.strip_prefix("NM "))
                    .filter_map(|l| l.split_whitespace().next())
                    .map(move |n| (cat.to_string(), n.to_string()))
            })
            .collect()
    })
}

/// 系の面の材料のうち、同梱のカタログから取ったもの（設定のフォルダに同じ名前の AGF が無いカタログのガラス）。
/// 同梱のカタログは屈折率だけ（内部透過率・熱膨張が無く、温度係数の無いガラスもある）なので、使ったら知らせる
pub fn used(materials: &[&str], user_agf: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in materials {
        let hit = names().iter().any(|(c, n)| {
            n.eq_ignore_ascii_case(m) && !user_agf.iter().any(|u| u.eq_ignore_ascii_case(c))
        });
        if hit && !out.iter().any(|x| x.eq_ignore_ascii_case(m)) {
            out.push(m.to_string());
        }
    }
    out
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
        assert!(write_to(&d) && write_to(&d));
        assert!(d.join("SCHOTT.AGF").is_file());
        let _ = std::fs::remove_dir_all(&d);
        let v = dirs_with(&["a".into()], Some("b"));
        assert_eq!(v, json!(["a", "b"]));
        assert_eq!(dirs_with(&["b".into()], Some("b")), json!(["b"]));
        // 同梱から取ったガラス（設定に SCHOTT.AGF が無いとき）だけを知らせる
        assert_eq!(
            used(&["N-BK7", "AIR", "n-bk7", ""], &[]),
            vec!["N-BK7".to_string()]
        );
        assert!(used(&["N-BK7"], &["schott".into()]).is_empty());
        assert_eq!(
            used(&["S-FSL5"], &["SCHOTT".into()]),
            vec!["S-FSL5".to_string()]
        );
    }
}
