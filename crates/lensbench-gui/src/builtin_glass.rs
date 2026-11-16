//! 同梱のガラスカタログ（[`lensbench_core::glass::builtin`]、SCHOTT・OHARA の分散式のガラス）から取ったガラスを
//! 知らせる（GUX-01）。カタログはフォルダに無いときに core が使う（CLI-31）ので、GUI はフォルダを足さない。
use lensbench_core::glass::builtin::FILES;
use std::sync::OnceLock;

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

    /// 同梱から取ったガラス（設定に SCHOTT.AGF が無いとき）だけを知らせる（GUX-01）
    #[test]
    fn gux01_used_lists_builtin_glasses() {
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
