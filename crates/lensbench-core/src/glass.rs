//! ガラスカタログ（AGF 形式）と屈折率。
//!
//! 分散式 1〜13、カタログ温度での空気に対する相対屈折率、系の温度・気圧への換算（カタログの dn/dT と
//! 空気の屈折率による）。OpticStudio の INDX と 1e-15 で一致する。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Glass {
    pub name: String,
    pub catalog: String,
    pub formula: i32,
    pub c: Vec<f64>,
    /// D0 D1 D2 E0 E1 λtk Tref
    pub td: [f64; 7],
    pub ld: Option<[f64; 2]>,
    /// 波長 [µm]、内部透過率、厚さ [mm]
    pub it: Vec<[f64; 3]>,
}

impl Glass {
    /// カタログ温度での相対屈折率（分散式）。
    pub fn n_cat(&self, l: f64) -> f64 {
        let c = &self.c;
        let l2 = l * l;
        match self.formula {
            1 => (c[0]
                + c[1] * l2
                + c[2] / l2
                + c[3] / l2.powi(2)
                + c[4] / l2.powi(3)
                + c[5] / l2.powi(4))
            .sqrt(),
            2 => {
                (1.0 + c[0] * l2 / (l2 - c[1]) + c[2] * l2 / (l2 - c[3]) + c[4] * l2 / (l2 - c[5]))
                    .sqrt()
            }
            3 => {
                let lh = 1.0 / (l2 - 0.028);
                c[0] + c[1] * lh + c[2] * lh * lh + c[3] * l2 + c[4] * l2 * l2 + c[5] * l2.powi(3)
            }
            4 => (1.0 + c[0] + c[1] * l2 / (l2 - c[2] * c[2]) + c[3] / (l2 - c[4] * c[4])).sqrt(),
            5 => c[0] + c[1] / l + c[2] / l.powf(3.5),
            6 => (1.0
                + (0..4)
                    .map(|i| c[2 * i] * l2 / (l2 - c[2 * i + 1]))
                    .sum::<f64>())
            .sqrt(),
            7 => (c[0] + c[1] / (l2 - c[2]) - c[3] * l2).sqrt(),
            8 => (c[0] + c[1] * l2 / (l2 - c[2]) - c[3] * l2).sqrt(),
            9 => (c[0] + c[1] * l2 / (l2 - c[2]) + c[3] * l2 / (l2 - c[4])).sqrt(),
            10 => (c[0] + c[1] * l2 + (2..8).map(|k| c[k] / l2.powi(k as i32 - 1)).sum::<f64>())
                .sqrt(),
            11 => (1.0
                + (0..5)
                    .map(|i| c[2 * i] * l2 / (l2 - c[2 * i + 1]))
                    .sum::<f64>())
            .sqrt(),
            12 => (c[0]
                + c[1] * l2
                + c[2] / l2
                + c[3] / l2.powi(2)
                + c[4] / l2.powi(3)
                + c[5] / l2.powi(4)
                + c[6] * l2 * l2
                + c[7] * l2.powi(3))
            .sqrt(),
            13 => (c[0]
                + c[1] * l2
                + c[2] * l2 * l2
                + (3..9).map(|k| c[k] / l2.powi(k as i32 - 2)).sum::<f64>())
            .sqrt(),
            // 読込（parse_agf）で 1〜13 以外は止めているので、ここへは手で組んだ Glass だけが来る。panic せず NaN にする
            // （追跡は NaN の屈折率を不正として止める）
            _ => f64::NAN,
        }
    }

    /// 系の温度 t [℃]・気圧 p [atm] での相対屈折率（系の空気 = 1）。
    pub fn n(&self, wl: f64, t: f64, p: f64) -> f64 {
        let [d0, d1, d2, e0, e1, ltk, tref] = self.td;
        if t == tref && p == 1.0 {
            return self.n_cat(wl);
        }
        let a_sys = air(wl, t, p);
        let a_ref = air(wl, tref, 1.0);
        let lc = wl * a_sys / a_ref;
        let nr = self.n_cat(lc);
        let dt = t - tref;
        let dn = (nr * nr - 1.0) / (2.0 * nr)
            * (d0 * dt
                + d1 * dt * dt
                + d2 * dt.powi(3)
                + (e0 * dt + e1 * dt * dt) / (lc * lc - ltk.signum() * ltk * ltk));
        (nr * a_ref + dn) / a_sys
    }

    /// 系の中での屈折率（「Adjust Index Data To Environment」がオフならカタログの値のまま）。
    pub fn index(&self, wl: f64, t: f64, p: f64, adjust: bool) -> f64 {
        if adjust {
            self.n(wl, t, p)
        } else {
            self.n(wl, self.td[6], 1.0)
        }
    }
}

/// OpticStudio のモデルガラス（nd・vd・ΔPgF）の屈折率（λ [µm]）。
///
/// Conrady の形 n = A + B/λ + C/λ^3.5 で、係数は nd − 1、(nd − 1)/vd、ΔPgF (nd − 1)/vd の一次式。
/// OpticStudio 2026 R1.01 の INDX（128 組 × 24 波長、0.35〜2.3 µm）から決め、すべて 1e-15 以内で一致する
///。この波長範囲の外は外挿（未照合）。d 線での値は nd とわずかに違う（OpticStudio も同じ）。
pub fn model_index(nd: f64, vd: f64, dpgf: f64, wl: f64) -> f64 {
    let (a, b) = (1.0 / wl, wl.powf(-3.5));
    let g = nd - 1.0;
    let d = g / vd;
    1.0 + g * (0.985_115_460_58 + 0.011_634_007_14 * a - 7.643_260_3e-4 * b)
        + d * (-1.431_304_222 + 0.499_823_474 * a + 0.090_308_377 * b)
        + dpgf * d * (8.849_31 - 6.916_77 * a + 0.454_415 * b)
}

/// 空気の屈折率（15℃基準、温度・気圧で換算）。
pub fn air(wl: f64, t: f64, p: f64) -> f64 {
    let l2 = wl * wl;
    let n15 = 1.0
        + (6432.8 + 2949810.0 * l2 / (146.0 * l2 - 1.0) + 25540.0 * l2 / (41.0 * l2 - 1.0)) * 1e-8;
    1.0 + (n15 - 1.0) * p / (1.0 + 3.4785e-3 * (t - 15.0))
}

/// UTF-16（BOM あり）か Latin-1 のテキストを読む。
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let raw = crate::vfs::read(path)?;
    Ok(decode_text(&raw))
}

pub fn decode_text(raw: &[u8]) -> String {
    if raw.len() >= 2 && (raw[..2] == [0xff, 0xfe] || raw[..2] == [0xfe, 0xff]) {
        let le = raw[0] == 0xff;
        let u: Vec<u16> = raw[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&u)
    } else {
        raw.iter().map(|&b| b as char).collect()
    }
}

/// AGF を読む（名前 → ガラス）。
///
/// 読んだ結果はプロセスの中で使い回す（ファイルの更新時刻と大きさが同じなら読み直さない）。カタログの玉ごと・
/// 組合せを動かすたびの系の作り直しで、数 MB の AGF を毎回読むと 1 回に数十 ms かかるため
pub fn load_agf(path: &Path) -> std::io::Result<HashMap<String, Glass>> {
    Ok((*load_agf_shared(path)?).clone())
}

/// [`load_agf`] と同じだが、使い回しの結果をそのまま共有して返す（写さない）
pub fn load_agf_shared(path: &Path) -> std::io::Result<Arc<HashMap<String, Glass>>> {
    type Cache = HashMap<
        PathBuf,
        (
            Option<std::time::SystemTime>,
            u64,
            Arc<HashMap<String, Glass>>,
        ),
    >;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let stamp = crate::vfs::stamp(path)?;
    let key = crate::vfs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((t, n, g)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if (*t, *n) == stamp {
            return Ok(g.clone());
        }
    }
    let txt = read_text(path)?;
    let cat = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let g = Arc::new(
        parse_agf(&txt, &cat)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?,
    );
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, (stamp.0, stamp.1, g.clone()));
    Ok(g)
}

/// AGF の本文を読む。分散式の番号が 1〜13 でない・数として読めない欄はその場で誤りにする（以前は読込を通り、追跡で
/// panic していた）
pub fn parse_agf(txt: &str, cat: &str) -> crate::Result<HashMap<String, Glass>> {
    use crate::Error;
    let mut out = HashMap::new();
    let mut cur: Option<Glass> = None;
    let flush = |g: Option<Glass>, out: &mut HashMap<String, Glass>| {
        if let Some(g) = g {
            if !g.c.is_empty() {
                out.insert(g.name.clone(), g);
            }
        }
    };
    for (ln, line) in txt.lines().enumerate() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        let at = |what: &str| format!("{cat}.AGF の {} 行目（{what}）", ln + 1);
        let f = |s: &str| -> crate::Result<f64> {
            // 欠けた値を "-" と書くカタログがある（0 とみなす）
            if s == "-" {
                return Ok(0.0);
            }
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("{}: 数として読めない値 {s:?}", at(t[0]))))
        };
        let nums = |v: &[&str]| v.iter().map(|v| f(v)).collect::<crate::Result<Vec<f64>>>();
        match t[0] {
            "NM" if t.len() >= 3 => {
                flush(cur.take(), &mut out);
                let x = f(t[2])?;
                if x.fract() != 0.0 || !(1.0..=13.0).contains(&x) {
                    return Err(Error::NotImplemented(format!(
                        "{}: ガラス {} の分散式 {} は未対応（1〜13）",
                        at("NM"),
                        t[1],
                        t[2]
                    )));
                }
                cur = Some(Glass {
                    name: t[1].to_string(),
                    catalog: cat.to_string(),
                    formula: x as i32,
                    c: Vec::new(),
                    td: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 20.0],
                    ld: None,
                    it: Vec::new(),
                });
            }
            "CD" => {
                if let Some(g) = cur.as_mut() {
                    g.c = nums(&t[1..])?;
                    let need = n_coef(g.formula);
                    // 書いていない後ろの係数は 0（AGF は 10 個書くのが普通だが、短い行もある）
                    g.c.resize(g.c.len().max(need).max(10), 0.0);
                }
            }
            "TD" => {
                if let Some(g) = cur.as_mut() {
                    let mut v = nums(&t[1..])?;
                    v.resize(7, 0.0);
                    g.td.copy_from_slice(&v[..7]);
                }
            }
            "IT" if t.len() >= 4 => {
                if let Some(g) = cur.as_mut() {
                    g.it.push([f(t[1])?, f(t[2])?, f(t[3])?]);
                }
            }
            "LD" if t.len() >= 3 => {
                if let Some(g) = cur.as_mut() {
                    g.ld = Some([f(t[1])?, f(t[2])?]);
                }
            }
            _ => {}
        }
    }
    flush(cur.take(), &mut out);
    Ok(out)
}

/// 分散式ごとの係数の数
fn n_coef(formula: i32) -> usize {
    match formula {
        5 => 3,
        7 | 8 => 4,
        4 | 9 => 5,
        1..=3 => 6,
        6 | 10 | 12 => 8,
        13 => 9,
        11 => 10,
        _ => 0,
    }
}

/// GCAT の順にカタログを探して読む。同じ名前のガラスは先のカタログのもの。
/// 見つからないカタログ名は戻り値の 2 番目に返す（読めなかったカタログは「名前（理由）」の形で）。
pub fn load_catalogs(names: &[String], dirs: &[PathBuf]) -> (HashMap<String, Glass>, Vec<String>) {
    let (g, missing) = load_catalogs_shared(names, dirs);
    ((*g).clone(), missing)
}

/// [`load_catalogs`] と同じだが、結果を共有して返す。同じカタログ名・フォルダの組は使い回す（見つかった AGF の
/// 更新時刻と大きさが同じで、見つからないカタログがないとき）。差し替え探索などで系を組むたびに、フォルダを
/// 探して数千種のガラスの表を作り直さないため
pub fn load_catalogs_shared(
    names: &[String],
    dirs: &[PathBuf],
) -> (Arc<HashMap<String, Glass>>, Vec<String>) {
    type Key = (Vec<String>, Vec<PathBuf>);
    type Entry = (
        Vec<(PathBuf, crate::vfs::Stamp)>,
        Arc<HashMap<String, Glass>>,
    );
    static CACHE: OnceLock<Mutex<HashMap<Key, Entry>>> = OnceLock::new();
    let key: Key = (names.to_vec(), dirs.to_vec());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((files, g)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if files
            .iter()
            .all(|(p, st)| crate::vfs::stamp(p).is_ok_and(|s| s == *st))
        {
            return (g.clone(), Vec::new());
        }
    }
    let mut out: HashMap<String, Glass> = HashMap::new();
    let mut missing = Vec::new();
    let mut files = Vec::new();
    // 読めなかった AGF があれば使い回さない（直したら読み直す）
    let mut complete = true;
    for name in names {
        let want = format!("{}.AGF", name.to_uppercase());
        let mut found = false;
        'dirs: for d in dirs {
            let Ok(rd) = crate::vfs::read_dir(d) else {
                continue;
            };
            for e in rd {
                if e.file_name()
                    .map(|n| n.to_string_lossy().to_uppercase())
                    .unwrap_or_default()
                    == want
                {
                    match (crate::vfs::stamp(&e), load_agf_shared(&e)) {
                        (Ok(st), Ok(g)) => {
                            for (k, v) in g.iter() {
                                out.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                            files.push((e.clone(), st));
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            // 読めなかったカタログは理由つきで「見つからない」に入れる（黙って飛ばさない）
                            complete = false;
                            missing.push(format!("{name}（{err}）"));
                        }
                    }
                    found = true;
                    break 'dirs;
                }
            }
        }
        if !found {
            missing.push(name.clone());
        }
    }
    let out = Arc::new(out);
    if missing.is_empty() && complete {
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (files, out.clone()));
    }
    (out, missing)
}

/// ガラスの検索パス: 引数、環境変数 LENSBENCH_GLASSCAT（OS のパス区切り）。
pub fn catalog_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = extra.to_vec();
    if let Some(e) = std::env::var_os("LENSBENCH_GLASSCAT") {
        v.extend(std::env::split_paths(&e));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn air_index_reasonable() {
        let n = air(0.5875618, 20.0, 1.0);
        assert!((n - 1.000271).abs() < 2e-6, "{n}");
    }
    #[test]
    fn sellmeier_fictitious() {
        // 架空のガラス（Sellmeier 1）
        let txt = "NM TESTG 2 0 1.5 60 0 1\nCD 1.0 0.006 0.2 0.02 1.0 100\nTD 0 0 0 0 0 0 20\n";
        let g = parse_agf(txt, "TEST").unwrap();
        let g = &g["TESTG"];
        let l2: f64 = 0.5875618f64.powi(2);
        let expect = (1.0 + l2 / (l2 - 0.006) + 0.2 * l2 / (l2 - 0.02) + l2 / (l2 - 100.0)).sqrt();
        assert!((g.n(0.5875618, 20.0, 1.0) - expect).abs() < 1e-15);
    }
    /// 未対応の分散式・数として読めない欄は読込で誤り（以前は読込を通り、追跡で panic した）
    #[test]
    fn bad_formula_is_error() {
        let ok = "CD 1.0 0.006 0.2 0.02 1.0 100\n";
        for nm in [
            "NM G 99 0 1.5 60\n",
            "NM G 0 0 1.5 60\n",
            "NM G abc 0 1.5 60\n",
            "NM G 2.5 0 1.5 60\n",
        ] {
            let e = parse_agf(&format!("{nm}{ok}"), "T").unwrap_err();
            assert!(
                matches!(
                    e,
                    crate::Error::NotImplemented(_) | crate::Error::Invalid(_)
                ),
                "{nm}: {e}"
            );
        }
        let e = parse_agf("NM G 2 0 1.5 60\nCD 1.0 x 0.2\n", "T").unwrap_err();
        assert!(e.to_string().contains("2 行目"), "{e}");
        // 手で組んだ未対応の式は panic せず NaN
        let mut g = parse_agf(&format!("NM G 2 0 1.5 60\n{ok}"), "T")
            .unwrap()
            .remove("G")
            .unwrap();
        g.formula = 99;
        assert!(g.n_cat(0.55).is_nan());
    }
}
