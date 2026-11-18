//! ガラスカタログ（AGF 形式）と屈折率。
//!
//! 分散式 1〜13、カタログ温度での空気に対する相対屈折率、系の温度・気圧への換算（カタログの dn/dT と
//! 空気の屈折率による）。OpticStudio の INDX と 1e-15 で一致する。

use crate::{LockExt, RwLockExt};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Glass {
    pub name: String,
    pub catalog: String,
    /// 分散式の番号（AGF の 1〜13。0 は利用者の材料で、屈折率は拡張の表のカタログ `catalog` のモデル `name` が
    /// 返す（係数 `c` は空。[`crate::usermaterial`]）。その拡張の無いプロセスでは屈折率を求められない）
    pub formula: i32,
    pub c: Vec<f64>,
    /// D0 D1 D2 E0 E1 λtk Tref
    pub td: [f64; 7],
    pub ld: Option<[f64; 2]>,
    /// 波長 [µm]、内部透過率、厚さ [mm]
    pub it: Vec<[f64; 3]>,
}

impl Glass {
    /// カタログ温度での相対屈折率（分散式）。利用者の材料（[`crate::usermaterial`]）はモデルの基準温度・1 atm で
    pub fn n_cat(&self, l: f64) -> f64 {
        if self.formula == crate::usermaterial::USER_FORMULA {
            return crate::usermaterial::index(self, l, self.td[6], 1.0);
        }
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
        // 利用者の材料は空気・温度の換算もモデルが受け持つ
        if self.formula == crate::usermaterial::USER_FORMULA {
            return crate::usermaterial::index(self, wl, t, p);
        }
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

/// テキストファイルを読む（文字コードは [`decode_text`]）。
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let raw = crate::vfs::read(path)?;
    Ok(decode_text(&raw))
}

/// 読んだテキストの文字コード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEncoding {
    /// UTF-16（BOM で LE・BE を判定。OpticStudio の .zmx）
    Utf16,
    /// UTF-8（BOM の有無を問わない）。ASCII だけのファイルもこれ
    Utf8,
    /// Shift_JIS（日本語の Windows の ANSI、CP932）
    ShiftJis,
    /// Windows-1252（欧文の Windows の ANSI。Latin-1 の上位）
    Windows1252,
}

/// バイト列をテキストにする: BOM（UTF-16 LE/BE・UTF-8）→ BOM なしで UTF-8 として正しければ UTF-8 → Shift_JIS として
/// 誤りなく読めれば Shift_JIS → Windows-1252。以前は BOM のないものを 1 バイト = 1 文字（Latin-1）で読み、UTF-8・
/// CP932 の日本語の面の名前が化けたまま UTF-16 で保存されていた
pub fn decode_text(raw: &[u8]) -> String {
    decode_text_enc(raw).0
}

/// [`decode_text`] と同じで、判定した文字コードも返す
pub(crate) fn decode_text_enc(raw: &[u8]) -> (String, TextEncoding) {
    if raw.len() >= 2 && (raw[..2] == [0xff, 0xfe] || raw[..2] == [0xfe, 0xff]) {
        let le = raw[0] == 0xff;
        let u: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return (String::from_utf16_lossy(&u), TextEncoding::Utf16);
    }
    let body = raw.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(raw);
    if let Ok(s) = std::str::from_utf8(body) {
        return (s.to_string(), TextEncoding::Utf8);
    }
    let (s, had_errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(body);
    // Windows-1252 の ° µ ± などの 1 バイトは Shift_JIS では半角カナに読める。全角の文字が 1 つもなく半角カナだけなら
    // Windows-1252 とみる（日本語の .zmx の名前・コメントは全角を含む）
    let halfwidth_only = s.chars().any(|c| ('\u{ff61}'..='\u{ff9f}').contains(&c))
        && !s
            .chars()
            .any(|c| c as u32 >= 0x3000 && !('\u{ff61}'..='\u{ff9f}').contains(&c));
    if !had_errors && !halfwidth_only {
        return (s.into_owned(), TextEncoding::ShiftJis);
    }
    let (s, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(body);
    (s.into_owned(), TextEncoding::Windows1252)
}

/// AGF を読む（名前 → ガラス）。
///
/// 読んだ結果はプロセスの中で使い回す（ファイルの更新時刻と大きさが同じなら読み直さない）。カタログの玉ごと・
/// 組合せを動かすたびの系の作り直しで、数 MB の AGF を毎回読むと 1 回に数十 ms かかるため
pub fn load_agf(path: &Path) -> std::io::Result<HashMap<String, Glass>> {
    Ok((*load_agf_shared(path)?).clone())
}

/// これまでに読んだ AGF のファイル（正規化したパス、読んだ順）。系ファイルに書くとき、系のガラスがどのカタログの
/// ファイルから来たかを探すのに使う（lensbench-project の sysfile）。読込の使い回しの表と違って空にしない
static LOADED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// これまでにこのプロセスで読んだ AGF のファイル（読んだ順）
pub fn loaded_files() -> Vec<PathBuf> {
    LOADED.lock_ok().clone()
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
    if let Some((t, n, g)) = cache.lock_ok().get(&key) {
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
    {
        let mut l = LOADED.lock_ok();
        if !l.contains(&key) {
            l.push(key.clone());
        }
    }
    let mut c = cache.lock_ok();
    bound_cache(&mut c);
    c.insert(key, (stamp.0, stamp.1, g.clone()));
    Ok(g)
}

/// 読込の使い回しの表の大きさの上限（長く動く serve・GUI でフォルダ・カタログの組を変え続けても
/// 増え続けない）。超えたら空にする（次から読み直すだけで、結果は変わらない）
const CACHE_MAX: usize = 64;

fn bound_cache<K, V>(c: &mut HashMap<K, V>) {
    if c.len() >= CACHE_MAX {
        c.clear();
    }
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
        let at = |what: &str| format!("{cat}.AGF line {} ({what})", ln + 1);
        let f = |s: &str| -> crate::Result<f64> {
            // 欠けた値を "-" と書くカタログがある（0 とみなす）
            if s == "-" {
                return Ok(0.0);
            }
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("{}: value {s:?} is not a number", at(t[0]))))
        };
        let nums = |v: &[&str]| v.iter().map(|v| f(v)).collect::<crate::Result<Vec<f64>>>();
        match t[0] {
            "NM" if t.len() >= 3 => {
                flush(cur.take(), &mut out);
                let x = f(t[2])?;
                if x.fract() != 0.0 || !(1.0..=13.0).contains(&x) {
                    return Err(Error::NotImplemented(format!(
                        "{}: glass {} dispersion formula {} is not supported (1 to 13)",
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

pub mod builtin;

/// フォルダ `dirs` を順に見て、カタログ `name` の AGF（大文字小文字は問わない）を探す
pub(crate) fn find_agf(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let want = format!("{}.AGF", name.to_uppercase());
    dirs.iter().find_map(|d| {
        crate::vfs::read_dir(d).ok()?.into_iter().find(|e| {
            e.file_name()
                .map(|n| n.to_string_lossy().to_uppercase())
                .unwrap_or_default()
                == want
        })
    })
}

/// GCAT の順にカタログを探して読む（拡張の表に足した材料のカタログ（[`crate::usermaterial`]）が同じ名前のフォルダの
/// AGF より先、フォルダに無いカタログは同梱のもの、[`builtin`]）。同じ名前のガラスは先のカタログのもの。
/// 見つからないカタログ名は戻り値の 2 番目に返す（読めなかったカタログは「名前（理由）」の形で）。
pub fn load_catalogs(names: &[String], dirs: &[PathBuf]) -> (HashMap<String, Glass>, Vec<String>) {
    let (g, missing) = load_catalogs_shared(names, dirs);
    ((*g).clone(), missing)
}

/// カタログで名前を限ったガラスの名前（`名前@カタログ`）。同じ名前で値の違うガラスが前のカタログにあるとき、後の
/// カタログのものをこの名前で引く（.zmx の GCAT と、プロジェクトの glass_catalogs が違うときなど）
pub fn qualified(name: &str, catalog: &str) -> String {
    format!("{name}@{catalog}")
}

/// [`qualified`] の名前のカタログの中の名前（`@` の前。限っていなければそのまま）
pub fn base_name(name: &str) -> &str {
    name.split_once('@').map_or(name, |x| x.0)
}

/// カタログ g（名前 cat）のガラスを表 out に足す: 無い名前はそのまま、前のカタログに同じ名前で値の違うガラスが
/// あれば [`qualified`] の名前で（前のものは名前だけで引ける）
pub fn merge_catalog(out: &mut HashMap<String, Glass>, g: &HashMap<String, Glass>, cat: &str) {
    for (k, v) in g {
        match out.get(k) {
            None => {
                out.insert(k.clone(), v.clone());
            }
            Some(h) if h != v => {
                let c = if v.catalog.is_empty() {
                    cat
                } else {
                    v.catalog.as_str()
                };
                out.entry(qualified(k, c)).or_insert_with(|| v.clone());
            }
            Some(_) => {}
        }
    }
}

/// 表に無い [`qualified`] の名前（`名前@カタログ`）を足す: 表の 名前 がそのカタログのガラスならそれを、無ければ
/// カタログを dirs（無ければ同梱のもの）から読んで引く。引けなかった名前を返す
pub fn resolve_qualified<'a>(
    table: &mut HashMap<String, Glass>,
    names: impl IntoIterator<Item = &'a str>,
    dirs: &[PathBuf],
) -> Vec<String> {
    let mut missing = Vec::new();
    for n in names {
        let Some((base, cat)) = n.split_once('@') else {
            continue;
        };
        // 名前@カタログ#2: 同じ名前のカタログ 2 つとぶつかったときに付けた番号（引くのはそのカタログ）
        let cat = cat.split_once('#').map_or(cat, |x| x.0);
        if table.contains_key(n) {
            continue;
        }
        let g = match table
            .get(base)
            .filter(|g| g.catalog.eq_ignore_ascii_case(cat))
        {
            Some(g) => Some(g.clone()),
            None => load_catalogs_shared(&[cat.to_string()], dirs)
                .0
                .get(base)
                .cloned(),
        };
        match g {
            Some(g) => {
                table.insert(n.to_string(), g);
            }
            None => missing.push(n.to_string()),
        }
    }
    missing
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
    if let Some((files, g)) = cache.lock_ok().get(&key) {
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
    // 読めなかった AGF・同梱のカタログを使ったら使い回さない（直したら・置いたら読み直す）
    let mut complete = true;
    for name in names {
        // 拡張の表に足した材料のカタログ（フォルダの AGF より先。表は計算ごとに違いうるので使い回さない）
        if let Some(g) = crate::usermaterial::catalog(name) {
            merge_catalog(&mut out, &g, name);
            complete = false;
            continue;
        }
        let Some(e) = find_agf(name, dirs) else {
            // フォルダのどこにも無ければ同梱のカタログ。後でフォルダに置かれたら読み直すよう使い回さない
            match builtin::catalog(name) {
                Some(g) => {
                    merge_catalog(&mut out, &g, name);
                    complete = false;
                }
                None => missing.push(name.clone()),
            }
            continue;
        };
        match (crate::vfs::stamp(&e), load_agf_shared(&e)) {
            (Ok(st), Ok(g)) => {
                merge_catalog(&mut out, &g, name);
                files.push((e.clone(), st));
            }
            (Err(err), _) | (_, Err(err)) => {
                // 読めなかったカタログは理由つきで「見つからない」に入れる（黙って飛ばさない）
                complete = false;
                missing.push(format!("{name} ({err})"));
            }
        }
    }
    let out = Arc::new(out);
    if missing.is_empty() && complete {
        let mut c = cache.lock_ok();
        bound_cache(&mut c);
        c.insert(key, (files, out.clone()));
    }
    (out, missing)
}

/// [`load_catalogs_shared`] の見つからない・読めなかったカタログを、ガラスが無いときの誤りに添える文にする
/// （空なら空。読めなかった理由が「カタログにない」の誤りに出るように）
pub fn missing_note(missing: &[String]) -> String {
    if missing.is_empty() {
        String::new()
    } else {
        let e = missing.join(", ");
        format!(" (catalogs not found or unreadable: {e})")
    }
}

/// ガラスがカタログに無いときの文の後ろに足す案内
pub const DIRS_HINT: &str =
    ". Pass catalog folders with --glass-dir (CLI), glass_dirs (serve, Python) or LENSBENCH_GLASSCAT";

/// 引数の後に探す既定のガラスのフォルダ（[`set_default_dirs`]。入口が環境変数 LENSBENCH_GLASSCAT から決める）
static DEFAULT_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// 引数の後に探す既定のガラスのフォルダを決める。CLI・Python・GUI の入口が [`crate::init_from_env`] で
/// LENSBENCH_GLASSCAT から決める（core は環境変数を読まない）
pub fn set_default_dirs(dirs: Vec<PathBuf>) {
    *DEFAULT_DIRS.write_ok() = dirs;
}

/// 既定のガラスのフォルダ（[`set_default_dirs`]）
pub fn default_dirs() -> Vec<PathBuf> {
    DEFAULT_DIRS.read_ok().clone()
}

/// ガラスの検索パス: 引数、既定のフォルダ（[`set_default_dirs`]、入口が LENSBENCH_GLASSCAT から決める）
pub fn catalog_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = extra.to_vec();
    v.extend(default_dirs());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 検索パスは引数の後に既定のフォルダ（入口が決める）。core は環境変数を読まない
    #[test]
    fn default_dirs_follow_the_arguments() {
        let none = PathBuf::from("/lensbench-no-such-glass-dir");
        set_default_dirs(vec![none.clone()]);
        let v = catalog_dirs(&[PathBuf::from("a")]);
        set_default_dirs(Vec::new());
        assert_eq!(v, [PathBuf::from("a"), none]);
        assert_eq!(catalog_dirs(&[]), Vec::<PathBuf>::new());
    }
    /// 名前@カタログ と、番号を付けた 名前@カタログ#2 は、表にあるそのカタログのガラスを引く
    #[test]
    fn qualified_names_resolve_by_catalog() {
        let txt = "NM TESTG 2 0 1.5 60 0 1\nCD 1.0 0.006 0.2 0.02 1.0 100\nTD 0 0 0 0 0 0 20\n";
        let mut t = parse_agf(txt, "TEST").unwrap();
        let missing = resolve_qualified(&mut t, ["TESTG@TEST", "TESTG@TEST#2", "TESTG@OTHER"], &[]);
        assert_eq!(t["TESTG@TEST"], t["TESTG"]);
        assert_eq!(t["TESTG@TEST#2"], t["TESTG"]);
        assert_eq!(missing, vec!["TESTG@OTHER".to_string()]);
    }
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
    /// 文字コードの判定: UTF-16（BOM）・UTF-8（BOM の有無）・Shift_JIS・Windows-1252
    #[test]
    fn decode_encodings() {
        let s = "NAME 試験レンズ\nCOMM 前玉\n";
        let mut u16le = vec![0xff, 0xfe];
        u16le.extend(s.encode_utf16().flat_map(|c| c.to_le_bytes()));
        let mut u16be = vec![0xfe, 0xff];
        u16be.extend(s.encode_utf16().flat_map(|c| c.to_be_bytes()));
        let mut u8bom = vec![0xef, 0xbb, 0xbf];
        u8bom.extend(s.as_bytes());
        let (sj, _, _) = encoding_rs::SHIFT_JIS.encode(s);
        for (b, e) in [
            (u16le, TextEncoding::Utf16),
            (u16be, TextEncoding::Utf16),
            (s.as_bytes().to_vec(), TextEncoding::Utf8),
            (u8bom, TextEncoding::Utf8),
            (sj.into_owned(), TextEncoding::ShiftJis),
        ] {
            assert_eq!(decode_text_enc(&b), (s.to_string(), e));
        }
        // Latin-1 の é（0xE9）単独は Shift_JIS として読めないので Windows-1252
        assert_eq!(
            decode_text_enc(b"COMM caf\xe9\n"),
            ("COMM café\n".to_string(), TextEncoding::Windows1252)
        );
        // ° µ ±（0xB0 0xB5 0xB1）は Shift_JIS では半角カナに読めるが、全角がなければ Windows-1252
        assert_eq!(
            decode_text_enc(b"COMM 5\xb0 1\xb5m \xb10.1\n"),
            ("COMM 5° 1µm ±0.1\n".to_string(), TextEncoding::Windows1252)
        );
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
        assert!(e.message().unwrap_or_default().contains("line 2"), "{e}");
        // 手で組んだ未対応の式は panic せず NaN
        let mut g = parse_agf(&format!("NM G 2 0 1.5 60\n{ok}"), "T")
            .unwrap()
            .remove("G")
            .unwrap();
        g.formula = 99;
        assert!(g.n_cat(0.55).is_nan());
    }

    /// AGF が読めなかったときは、ガラスが無いという誤りに読めなかった理由が出る
    #[test]
    fn xct01_unreadable_catalog_reason_reaches_missing_glass_error() {
        let d = std::env::temp_dir().join(format!("lb-xct01-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("BADCAT.AGF"), "NM G 99 0 1.5 60\nCD 1.0 0.006 0.2\n").unwrap();
        let zmx = d.join("s.zmx");
        std::fs::write(
            &zmx,
            "MODE SEQ\nUNIT MM X W X CM MR CPMR\nENPD 10\nGCAT BADCAT\nWAVM 1 0.55 1\nSURF 0\n  TYPE STANDARD\n  DISZ INFINITY\nSURF 1\n  TYPE STANDARD\n  CURV 0.01\n  DISZ 5\n  GLAS G 0 0 1.5 60 0 0 0 0 0 0\n  STOP\nSURF 2\n  TYPE STANDARD\n  DISZ 100\nSURF 3\n  TYPE STANDARD\n",
        )
        .unwrap();
        let e = crate::zmx::load(&zmx)
            .unwrap_err()
            .message()
            .unwrap_or_default()
            .to_string();
        let _ = std::fs::remove_dir_all(&d);
        assert!(e.contains("is not in the catalogs"), "{e}");
        assert!(
            e.contains("BADCAT") && e.contains("dispersion formula 99"),
            "{e}"
        );
        // 日本語の表示: 添え書き（{why}）まで訳す
        let ja = crate::i18n::translate(e.strip_prefix("invalid: ").unwrap_or(&e));
        assert!(ja.contains("硝材 G（面 1）がカタログにありません"), "{ja}");
        assert!(
            ja.contains("（見つからない・読めなかったカタログ: BADCAT"),
            "{ja}"
        );
        assert!(
            ja.ends_with("環境変数 LENSBENCH_GLASSCAT で渡してください"),
            "{ja}"
        );
    }
}
