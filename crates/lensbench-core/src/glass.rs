//! ガラスカタログ（AGF 形式）と屈折率。
//!
//! 分散式 1〜13、カタログ温度での空気に対する相対屈折率、系の温度・気圧への換算（カタログの dn/dT と
//! 空気の屈折率による）。OpticStudio の INDX と 1e-15 で一致する。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
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
            f => panic!("分散式 {f} は未対応（{}）", self.name),
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

/// 空気の屈折率（15℃基準、温度・気圧で換算）。
pub fn air(wl: f64, t: f64, p: f64) -> f64 {
    let l2 = wl * wl;
    let n15 = 1.0
        + (6432.8 + 2949810.0 * l2 / (146.0 * l2 - 1.0) + 25540.0 * l2 / (41.0 * l2 - 1.0)) * 1e-8;
    1.0 + (n15 - 1.0) * p / (1.0 + 3.4785e-3 * (t - 15.0))
}

/// UTF-16（BOM あり）か Latin-1 のテキストを読む。
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let raw = std::fs::read(path)?;
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
pub fn load_agf(path: &Path) -> std::io::Result<HashMap<String, Glass>> {
    let txt = read_text(path)?;
    let cat = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(parse_agf(&txt, &cat))
}

pub fn parse_agf(txt: &str, cat: &str) -> HashMap<String, Glass> {
    let mut out = HashMap::new();
    let mut cur: Option<Glass> = None;
    let f = |s: &str| s.parse::<f64>().unwrap_or(0.0);
    let flush = |g: Option<Glass>, out: &mut HashMap<String, Glass>| {
        if let Some(g) = g {
            if !g.c.is_empty() {
                out.insert(g.name.clone(), g);
            }
        }
    };
    for line in txt.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        match t[0] {
            "NM" if t.len() >= 3 => {
                flush(cur.take(), &mut out);
                cur = Some(Glass {
                    name: t[1].to_string(),
                    catalog: cat.to_string(),
                    formula: f(t[2]) as i32,
                    c: Vec::new(),
                    td: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 20.0],
                    ld: None,
                    it: Vec::new(),
                });
            }
            "CD" => {
                if let Some(g) = cur.as_mut() {
                    g.c = t[1..].iter().map(|v| f(v)).collect();
                    g.c.resize(g.c.len() + 10, 0.0);
                }
            }
            "TD" => {
                if let Some(g) = cur.as_mut() {
                    let mut v: Vec<f64> = t[1..].iter().map(|v| f(v)).collect();
                    v.resize(7, 0.0);
                    g.td.copy_from_slice(&v[..7]);
                }
            }
            "IT" if t.len() >= 4 => {
                if let Some(g) = cur.as_mut() {
                    g.it.push([f(t[1]), f(t[2]), f(t[3])]);
                }
            }
            "LD" if t.len() >= 3 => {
                if let Some(g) = cur.as_mut() {
                    g.ld = Some([f(t[1]), f(t[2])]);
                }
            }
            _ => {}
        }
    }
    flush(cur.take(), &mut out);
    out
}

/// GCAT の順にカタログを探して読む。同じ名前のガラスは先のカタログのもの。
/// 見つからないカタログ名は戻り値の 2 番目に返す。
pub fn load_catalogs(names: &[String], dirs: &[PathBuf]) -> (HashMap<String, Glass>, Vec<String>) {
    let mut out: HashMap<String, Glass> = HashMap::new();
    let mut missing = Vec::new();
    for name in names {
        let want = format!("{}.AGF", name.to_uppercase());
        let mut found = false;
        'dirs: for d in dirs {
            let Ok(rd) = std::fs::read_dir(d) else {
                continue;
            };
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().to_uppercase() == want {
                    if let Ok(g) = load_agf(&e.path()) {
                        for (k, v) in g {
                            out.entry(k).or_insert(v);
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
        let g = parse_agf(txt, "TEST");
        let g = &g["TESTG"];
        let l2: f64 = 0.5875618f64.powi(2);
        let expect = (1.0 + l2 / (l2 - 0.006) + 0.2 * l2 / (l2 - 0.02) + l2 / (l2 - 100.0)).sqrt();
        assert!((g.n(0.5875618, 20.0, 1.0) - expect).abs() < 1e-15);
    }
}
