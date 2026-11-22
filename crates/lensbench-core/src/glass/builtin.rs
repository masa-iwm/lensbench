//! 同梱のガラスカタログ（SCHOTT・OHARA の分散式のガラス、refractiveindex.info の CC0 のデータから
//! `scripts/make_builtin_glass.py` で作る）。ガラスのフォルダのどこにも無いカタログの最後の候補にする
//! （[`super::load_catalogs_shared`]）ので、何も設定していなくても GUI・CLI・serve・Python で普通の系が開ける
//! 。屈折率だけ（内部透過率・熱膨張が無く、温度係数の無いガラスもある）なので、使ったら知らせる
//! （[`note`]）。同じ名前の AGF がフォルダにあればそちらが先。
use super::Glass;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// カタログ 1 つのガラス（名前 → ガラス）
pub type Glasses = Arc<HashMap<String, Glass>>;

/// 同梱の AGF（ファイル名、中身）
pub const FILES: [(&str, &str); 2] = [
    ("SCHOTT.AGF", include_str!("../../assets/glass/SCHOTT.AGF")),
    ("OHARA.AGF", include_str!("../../assets/glass/OHARA.AGF")),
];

/// 同梱のカタログの名前（大文字、"SCHOTT" など）
pub fn catalogs() -> impl Iterator<Item = &'static str> {
    FILES.iter().map(|(f, _)| f.trim_end_matches(".AGF"))
}

/// 同梱の AGF の本文の sha256（[`FILES`] と同じ順、`sha256:` と 16 進）。系のファイルに書くカタログの hash を
/// 起動のたびに求めないよう、ここに持つ（中身と合うことは lensbench-project の試験で確かめる。AGF を作り直したら直す）
pub const SHA256: [&str; 2] = [
    "sha256:a4d4e7e0bc2802044118e1a08befb4f8d7cdb2385c4d20dcc2660ad6a73ecf20",
    "sha256:f699cbfe20731c294c36e25053a98f360e17874d80d89168d8658f89137fb600",
];

/// 同梱のカタログ `name`（大文字小文字は問わない）の [`FILES`] の位置
fn index(name: &str) -> Option<usize> {
    FILES
        .iter()
        .position(|(f, _)| f.trim_end_matches(".AGF").eq_ignore_ascii_case(name))
}

/// 同梱のカタログ `name`（大文字小文字は問わない）の AGF の本文の hash（[`SHA256`]）。無ければ None
pub fn sha256(name: &str) -> Option<&'static str> {
    index(name).map(|i| SHA256[i])
}

/// 同梱のカタログ `name`（大文字小文字は問わない）のガラス。無ければ None。
/// カタログは使う時に 1 つずつ読む（SCHOTT だけの系で OHARA を読まない）
pub fn catalog(name: &str) -> Option<Glasses> {
    static C: [OnceLock<Glasses>; FILES.len()] = [const { OnceLock::new() }; FILES.len()];
    let i = index(name)?;
    Some(
        C[i].get_or_init(|| {
            let (f, t) = FILES[i];
            // 同梱のデータは試験（below）で読めることを確かめている
            let mut g = super::parse_agf(t, f.trim_end_matches(".AGF")).unwrap_or_default();
            // 同梱の AGF の NM の状態の欄は作るスクリプトが一律に書いた値で、メーカーの状態ではない
            for x in g.values_mut() {
                x.status = None;
            }
            Arc::new(g)
        })
        .clone(),
    )
}

/// カタログ `names` のうち、フォルダ `dirs` に無く同梱のものを使うカタログ
pub fn used_catalogs(names: &[String], dirs: &[PathBuf]) -> Vec<String> {
    names
        .iter()
        .filter(|n| catalog(n).is_some() && super::find_agf(n, dirs).is_none())
        .cloned()
        .collect()
}

/// 系の材料 `materials` のうち、同梱のカタログから取ったもの（カタログの並び `names` の先のカタログを優先し、
/// フォルダ `dirs` にある AGF のガラスは除く）
pub fn used_glasses(materials: &[&str], names: &[String], dirs: &[PathBuf]) -> Vec<String> {
    let from_builtin = used_catalogs(names, dirs);
    if from_builtin.is_empty() {
        return Vec::new();
    }
    // 先に並んだフォルダのカタログにあるガラスは同梱のものではない
    let (user, _) = super::load_catalogs_shared(
        &names
            .iter()
            .filter(|n| !from_builtin.contains(n))
            .cloned()
            .collect::<Vec<_>>(),
        dirs,
    );
    let mut out: Vec<String> = Vec::new();
    for m in materials {
        let u = m.to_uppercase();
        let hit = !user.contains_key(&u)
            && from_builtin
                .iter()
                .any(|c| catalog(c).is_some_and(|g| g.contains_key(&u)));
        if hit && !out.iter().any(|x| x.eq_ignore_ascii_case(m)) {
            out.push(m.to_string());
        }
    }
    out
}

/// 同梱のカタログから取ったガラスがあれば、その知らせ（読込の注意 [`crate::System::import_notes`] に入れる）
pub fn note(materials: &[&str], names: &[String], dirs: &[PathBuf]) -> Option<String> {
    let used = used_glasses(materials, names, dirs);
    (!used.is_empty()).then(|| {
        format!(
            "glass {} came from the built-in catalog (index only; no internal transmittance or thermal expansion). Pass the maker catalogs with --glass-dir (CLI), glass_dirs (serve, Python) or LENSBENCH_GLASSCAT",
            used.join(", ")
        )
    })
}

/// 系 `s` の面の材料について [`note`]
pub fn note_for(s: &crate::Prescription, names: &[String], dirs: &[PathBuf]) -> Option<String> {
    let mats: Vec<String> = s.surfaces.iter().map(|x| x.mat.name()).collect();
    let mats: Vec<&str> = mats.iter().map(String::as_str).collect();
    note(&mats, names, dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同梱の AGF は読めて、基準の系のガラス（N-BK7・F2・N-SK16・N-LAK9・S-FSL5）を含む
    #[test]
    fn gux01_builtin_catalogs_parse_and_cover_common_glasses() {
        for (f, t) in FILES {
            let g = super::super::parse_agf(t, f.trim_end_matches(".AGF")).unwrap();
            assert!(g.len() > 100, "{f}: {}", g.len());
        }
        let s = catalog("schott").unwrap();
        let o = catalog("OHARA").unwrap();
        for n in ["N-BK7", "F2", "N-SK16", "N-LAK9"] {
            assert!(s.contains_key(n), "{n}");
        }
        assert!(o.contains_key("S-FSL5"));
        assert!(catalog("HOYA").is_none());
        let n = s["N-BK7"].index(0.5875618, 20.0, 1.0, false);
        assert!((n - 1.5168).abs() < 2e-5, "{n}");
    }

    /// フォルダに無いカタログだけ同梱から取り、取ったガラスを知らせる
    #[test]
    fn cli31_fallback_and_note() {
        let none = [PathBuf::from("/lensbench-no-such-glass-dir")];
        let names = vec!["SCHOTT".to_string(), "OHARA".to_string()];
        let (g, missing) = super::super::load_catalogs_shared(&names, &none);
        assert!(missing.is_empty(), "{missing:?}");
        assert!(g.contains_key("N-BK7") && g.contains_key("S-FSL5"));
        let (_, missing) = super::super::load_catalogs_shared(&["HOYA".into()], &none);
        assert_eq!(missing, ["HOYA"]);
        assert_eq!(used_catalogs(&names, &none), names);
        assert_eq!(
            used_glasses(&["N-BK7", "AIR", "n-bk7", ""], &names, &none),
            ["N-BK7"]
        );
        let t = note(&["S-FSL5"], &names, &none).unwrap();
        assert!(t.contains("S-FSL5") && t.contains("--glass-dir"), "{t}");
        assert!(note(&["AIR"], &names, &none).is_none());
        // フォルダにある SCHOTT は同梱のものではない
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass");
        if dir.join("SCHOTT.AGF").is_file() || dir.join("schott.agf").is_file() {
            assert!(used_glasses(&["N-BK7"], &names, &[dir]).is_empty());
        }
    }
}
