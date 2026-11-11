//! 品の CAD データ（公式の STEP）を 1 件だけ取って、キャッシュに置く（公開 #120）。
//!
//! カタログには機構部品の簡易外形（`mech`）だけを入れ、詳しい形の STEP はメーカーの配布元の URL（品の `cad.step`）
//! から、ユーザーが品を選んで求めたときにその 1 ファイルだけを取る。裏でまとめて取りには行かない。
//! キャッシュのファイルは品の名前と URL から決めるので、2 回目からはキャッシュを使い、URL が変われば取り直す。
//! 取った STEP は `catalog-step` と同じ読み方で測り、簡易外形との食い違いを返す。
//!
//! 共有の置き場（[`Store`]、自前の HTTP の口。Cloudflare R2 を Access で守った口など）を決めてあれば、配布元より先に
//! 置き場を見て、配布元から取った STEP は置き場にも上げる。置き場の鍵はキャッシュのファイル名
//! （`<品>-<URL のハッシュ>.step`）。`cad.step` の URL の無い品も、置き場に上げてあれば取れる。
//! 口は `GET <url>/<鍵>`（無ければ 404）と `PUT <url>/<鍵>`（本文は STEP）だけ。
use lensbench_project::{catalog_fill, UserItem};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 取るファイルの大きさの上限（STEP は 1 品あたり数 MB）
const MAX_BYTES: u64 = 64 << 20;

/// キャッシュのフォルダ（[`catalog_fill::cad_cache_dir`]。GUI は設定のフォルダの `cad` に決める）
pub fn default_cache_dir() -> PathBuf {
    catalog_fill::cad_cache_dir()
}

/// キャッシュのファイル（[`catalog_fill::cad_cache_path`]）
pub fn cache_path(dir: &Path, item: &str, url: &str) -> PathBuf {
    catalog_fill::cad_cache_path(dir, item, url)
}

/// STEP をどこから取ったか
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cache,
    /// 共有の置き場
    Store,
    /// 配布元（置き場は無いか、上げられなかった）
    Vendor,
    /// 配布元から取り、置き場にも上げた
    VendorUploaded,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Source::Cache => "cache",
            Source::Store => "store",
            Source::Vendor => "vendor",
            Source::VendorUploaded => "vendor_uploaded",
        }
    }
}

/// 取った STEP
#[derive(Debug, Clone)]
pub struct CadStep {
    pub item: String,
    pub url: String,
    /// キャッシュのファイル
    pub path: PathBuf,
    /// キャッシュにあった（取りに行かなかった）
    pub cached: bool,
    /// どこから取ったか
    pub source: Source,
    /// 置き場を読めなかった・上げられなかった理由（STEP は取れている）
    pub store_notes: Vec<String>,
    /// STEP から測った寸法（測れなければ None、理由は `notes`）
    pub measured: Option<catalog_fill::StepSize>,
    /// 簡易外形との食い違い・測れなかった理由（空なら合う）
    pub notes: Vec<String>,
}

impl CadStep {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "item": self.item,
            "url": self.url,
            "path": self.path.display().to_string(),
            "cached": self.cached,
            "source": self.source.key(),
            "store_notes": self.store_notes,
            "measured": self.measured.as_ref().map(|m| serde_json::to_value(m).unwrap_or_default()),
            "notes": self.notes,
        })
    }
}

/// 品の STEP を取る。キャッシュ → 置き場 → 配布元（`cad.step`）の順に見て、配布元から取ったら置き場にも上げる。
/// 品の簡易外形と測った寸法を比べる。
/// 取れなかったら Err（呼ぶ側は簡易外形のまま描き、品ページ `url` を示す）
pub fn fetch_item(dir: &Path, name: &str, item: &UserItem) -> Result<CadStep, String> {
    fetch_item_with(dir, name, item, store().as_ref())
}

/// [`fetch_item`] の置き場を指定する版（None は置き場を使わない）
pub fn fetch_item_with(
    dir: &Path,
    name: &str,
    item: &UserItem,
    store: Option<&Store>,
) -> Result<CadStep, String> {
    let url = item.cad.step.trim();
    if url.is_empty() && store.is_none() {
        return Err(format!("{name} に STEP の URL（cad.step）がない"));
    }
    let path = cache_path(dir, name, url);
    let cached = path.is_file();
    let mut source = Source::Cache;
    let mut store_notes = Vec::new();
    let text = if cached {
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?
    } else {
        let key = store_key(&path);
        let from_store = match store {
            Some(s) => match s.get(&key) {
                Ok(b) => b,
                Err(e) => {
                    store_notes.push(e);
                    None
                }
            },
            None => None,
        };
        let b = match from_store {
            Some(b) => {
                source = Source::Store;
                b
            }
            None if url.is_empty() => {
                let mut e = format!("{name} に STEP の URL（cad.step）がなく、置き場にもない");
                for n in &store_notes {
                    e.push_str(&format!("（{n}）"));
                }
                return Err(e);
            }
            None => {
                source = Source::Vendor;
                step_bytes(download(url)?)?
            }
        };
        write_cache(dir, &path, &b)?;
        if source == Source::Vendor {
            if let Some(s) = store {
                match s.put(&key, &b) {
                    Ok(()) => source = Source::VendorUploaded,
                    Err(e) => store_notes.push(e),
                }
            }
        }
        b
    };
    let (measured, notes) = measure(item, &text);
    Ok(CadStep {
        item: name.to_string(),
        url: url.to_string(),
        path,
        cached,
        source,
        store_notes,
        measured,
        notes,
    })
}

/// 途中で止まっても壊れたファイルを残さないよう、別名に書いてから置き換える
pub(crate) fn write_cache(dir: &Path, path: &Path, b: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, b).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// 置き場の鍵（キャッシュのファイル名）
pub fn store_key(cache_file: &Path) -> String {
    cache_file
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 共有の置き場（自前の HTTP の口）。`GET <url>/<鍵>` で STEP（無ければ 404）、`PUT <url>/<鍵>` で上げる。
/// Cloudflare Access で守った口には、サービストークン（`id`・`secret`）を `CF-Access-Client-Id`・
/// `CF-Access-Client-Secret` の見出しで渡す
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Store {
    /// 口の URL（例 `https://example.pages.dev/cad`）
    pub url: String,
    /// Access のサービストークンの Client ID（空なら送らない）
    pub id: String,
    /// Access のサービストークンの Client Secret
    pub secret: String,
}

static STORE: std::sync::Mutex<Option<Store>> = std::sync::Mutex::new(None);

static DEFAULT_STORE: std::sync::Mutex<Option<Store>> = std::sync::Mutex::new(None);

/// 組み込みの置き場（配る版がビルドのときに決めたもの）。設定・環境変数の無いときに使う
pub fn set_default_store(s: Option<Store>) {
    *DEFAULT_STORE.lock().unwrap_or_else(|e| e.into_inner()) =
        s.filter(|s| !s.url.trim().is_empty());
}

/// 置き場を決める（GUI の設定）。None・URL が空なら環境変数に戻す
pub fn set_store(s: Option<Store>) {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = s.filter(|s| !s.url.trim().is_empty());
}

/// 今の置き場: [`set_store`] で決めたもの、無ければ環境変数 `LENSBENCH_CAD_STORE`（URL）・
/// `LENSBENCH_CAD_STORE_ID`・`LENSBENCH_CAD_STORE_SECRET`、無ければ組み込みの置き場（[`set_default_store`]）。
/// どれも無ければ None（配布元だけ）
pub fn store() -> Option<Store> {
    if let Some(s) = STORE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Some(s);
    }
    let env = |k: &str| std::env::var(k).unwrap_or_default().trim().to_string();
    let url = env("LENSBENCH_CAD_STORE");
    if !url.is_empty() {
        return Some(Store {
            url,
            id: env("LENSBENCH_CAD_STORE_ID"),
            secret: env("LENSBENCH_CAD_STORE_SECRET"),
        });
    }
    DEFAULT_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 組み込みの置き場があるか（GUI の欄の説明に使う）
pub fn has_default_store() -> bool {
    DEFAULT_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

// secret を出さない
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("url", &self.url)
            .field("id", &self.id)
            .field("secret", &if self.secret.is_empty() { "" } else { "***" })
            .finish()
    }
}

impl Store {
    /// トークンを送るのは https の口だけ（試験の手元の口 http://127.0.0.1・localhost は許す）
    fn check_scheme(&self) -> Result<(), String> {
        let u = self.url.trim();
        // http は手元の口だけ（ホストが 127.0.0.1・localhost にちょうど一致）
        let local = u.strip_prefix("http://").is_some_and(|r| {
            let host = r.split(['/', ':']).next().unwrap_or("");
            host == "127.0.0.1" || host == "localhost"
        });
        if self.id.trim().is_empty() || u.starts_with("https://") || local {
            Ok(())
        } else {
            Err(format!("トークンを送る置き場は https だけ: {u}"))
        }
    }

    fn url_of(&self, key: &str) -> String {
        let k: String = key
            .chars()
            .map(|c| match c {
                // + はそのまま（受け口が道をデコードしなくても鍵の形に合う）
                // / は鍵の中の区切り（膜のモデルは coat/<膜>-<ハッシュ>.lbcb）
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '+' | '/' => c.to_string(),
                c => {
                    let mut b = [0u8; 4];
                    c.encode_utf8(&mut b)
                        .bytes()
                        .map(|x| format!("%{x:02X}"))
                        .collect()
                }
            })
            .collect();
        format!("{}/{k}", self.url.trim().trim_end_matches('/'))
    }

    fn agent() -> ureq::Agent {
        // Access のログインへの転送は追わずに誤りにする（ログインのページを STEP と取り違えない）
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .user_agent(concat!("lensbench/", env!("CARGO_PKG_VERSION")))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build()
            .into()
    }

    fn auth<B>(&self, r: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        if self.id.trim().is_empty() {
            r
        } else {
            r.header("CF-Access-Client-Id", self.id.trim())
                .header("CF-Access-Client-Secret", self.secret.trim())
        }
    }

    fn status_err(&self, what: &str, st: u16) -> String {
        match st {
            301..=303 | 307 | 308 | 401 | 403 => format!(
                "置き場に{what}ない（{st}、Access のサービストークンを確かめる）: {}",
                self.url
            ),
            _ => format!("置き場に{what}ない（{st}）: {}", self.url),
        }
    }

    /// 鍵の STEP（無ければ Ok(None)）
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match self.get_raw(key)? {
            None => Ok(None),
            Some(b) => step_bytes(b)
                .map(Some)
                .map_err(|e| format!("置き場の中身が STEP でない（{}）: {e}", self.url_of(key))),
        }
    }

    /// 鍵の中身（確かめない。無ければ Ok(None)）
    pub fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        self.check_scheme()?;
        let u = self.url_of(key);
        let mut r = self
            .auth(Self::agent().get(&u))
            .call()
            .map_err(|e| format!("置き場を読めない（{u}）: {e}"))?;
        let st = r.status().as_u16();
        if st == 404 {
            return Ok(None);
        }
        if st != 200 {
            return Err(self.status_err("届か", st));
        }
        let mut b = Vec::new();
        r.body_mut()
            .as_reader()
            .take(MAX_BYTES + 1)
            .read_to_end(&mut b)
            .map_err(|e| format!("置き場を読めない（{u}）: {e}"))?;
        if b.len() as u64 > MAX_BYTES {
            return Err(format!("置き場の中身が大きすぎる: {u}"));
        }
        Ok(Some(b))
    }

    /// 鍵で上げる（STEP・膜のモデル）
    pub fn put(&self, key: &str, step: &[u8]) -> Result<(), String> {
        self.check_scheme()?;
        let u = self.url_of(key);
        let r = self
            .auth(Self::agent().put(&u))
            .header(
                "Content-Type",
                if key.ends_with(".step") {
                    "application/step"
                } else {
                    "application/octet-stream"
                },
            )
            .send(step)
            .map_err(|e| format!("置き場に上げられない（{u}）: {e}"))?;
        let st = r.status().as_u16();
        if (200..300).contains(&st) {
            Ok(())
        } else {
            Err(self.status_err("上げられ", st))
        }
    }

    /// 鍵があるか（HEAD）
    pub fn has(&self, key: &str) -> Result<bool, String> {
        self.check_scheme()?;
        let u = self.url_of(key);
        let r = self
            .auth(Self::agent().head(&u))
            .call()
            .map_err(|e| format!("置き場を読めない（{u}）: {e}"))?;
        match r.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            st => Err(self.status_err("届か", st)),
        }
    }
}

/// 名前を比べる形（英数字だけを大文字で）
fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// 手元の STEP（.step・.stp・それを入れた .zip）のファイルを、カタログの品に当てる。ファイル名（拡張子を除く）の
/// 全体と、区切り（`_`・`-`・空白）で切った前の部分・後ろの部分のどれかが、品の名前と英数字だけで一致すれば当てる
/// （`edm_11-211`・`CM1L03_ttn170465-e0w`・`C16_AP_12.7_STP__C16-AP-12.7`・`Newport-M-100.zip` など）。
/// 1 つのファイルに品が 2 つ当たれば長い名前の方、1 つの品にファイルが 2 つ当たれば名前順で先の方。
/// 返すのは（品の名前、ファイル）の名前順の並びと、当たらなかったファイル
pub fn match_files<'a>(
    items: impl IntoIterator<Item = &'a str>,
    files: &[PathBuf],
) -> (Vec<(String, PathBuf)>, Vec<PathBuf>) {
    let mut by_norm: std::collections::HashMap<String, Vec<&str>> = Default::default();
    for n in items {
        let k = norm(n);
        if k.len() >= 3 {
            by_norm.entry(k).or_default().push(n);
        }
    }
    let mut files: Vec<PathBuf> = files.to_vec();
    files.sort();
    let mut got: std::collections::BTreeMap<String, PathBuf> = Default::default();
    let mut miss = Vec::new();
    for f in &files {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut cands = vec![stem.as_str()];
        for (i, c) in stem.char_indices() {
            if matches!(c, '_' | '-' | ' ') {
                cands.push(&stem[..i]);
                cands.push(&stem[i + c.len_utf8()..]);
            }
        }
        let best = cands
            .iter()
            .filter_map(|c| by_norm.get(&norm(c)))
            .flatten()
            .max_by_key(|n| (norm(n).len(), std::cmp::Reverse(**n)));
        match best {
            Some(n) => {
                got.entry(n.to_string()).or_insert_with(|| f.clone());
            }
            None => miss.push(f.clone()),
        }
    }
    (got.into_iter().collect(), miss)
}

/// フォルダの中の STEP（.step・.stp・.zip、下のフォルダも）
pub fn step_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .map(|x| {
                    let x = x.to_string_lossy().to_ascii_lowercase();
                    x == "step" || x == "stp" || x == "zip"
                })
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// 置き場に上げた結果（1 ファイル）
#[derive(Debug, Clone)]
pub struct Uploaded {
    pub key: String,
    pub file: PathBuf,
    /// 品の名前（キャッシュをそのまま上げたときは空）
    pub item: String,
    /// "uploaded"・"exists"（置き場にあったので上げない）・"dry_run"・"error"
    pub status: &'static str,
    pub error: String,
}

impl Uploaded {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "key": self.key, "file": self.file.display().to_string(), "item": self.item,
            "status": self.status, "error": self.error,
        })
    }
}

/// 1 ファイルを鍵で上げる（STEP か確かめる。overwrite でなければ置き場にある鍵は上げない）
pub fn upload_file(
    store: &Store,
    key: &str,
    file: &Path,
    item: &str,
    overwrite: bool,
    dry_run: bool,
) -> Uploaded {
    let mut u = Uploaded {
        key: key.to_string(),
        file: file.to_path_buf(),
        item: item.to_string(),
        status: "error",
        error: String::new(),
    };
    let r = (|| -> Result<&'static str, String> {
        let b = step_bytes(std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?)?;
        if dry_run {
            return Ok("dry_run");
        }
        if !overwrite && store.has(key)? {
            return Ok("exists");
        }
        store.put(key, &b)?;
        Ok("uploaded")
    })();
    match r {
        Ok(s) => u.status = s,
        Err(e) => u.error = e,
    }
    u
}

/// URL の中身（http・https。環境変数のプロキシを使う）
fn download(url: &str) -> Result<Vec<u8>, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("STEP の URL は http(s) だけ: {url}"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .user_agent(concat!("lensbench/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut r = agent
        .get(url)
        .call()
        .map_err(|e| format!("STEP を取れない（{url}）: {e}"))?;
    let mut b = Vec::new();
    r.body_mut()
        .as_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut b)
        .map_err(|e| format!("STEP を取れない（{url}）: {e}"))?;
    if b.len() as u64 > MAX_BYTES {
        return Err(format!(
            "STEP が大きすぎる（{} MB 超）: {url}",
            MAX_BYTES >> 20
        ));
    }
    Ok(b)
}

/// 取った中身から STEP の文字を取り出す（zip なら中の最初の .step・.stp）。STEP でなければ Err
/// （ログインの要るページの HTML など）
pub fn step_bytes(b: Vec<u8>) -> Result<Vec<u8>, String> {
    let b = if b.starts_with(b"PK\x03\x04") {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(b))
            .map_err(|e| format!("zip を読めない: {e}"))?;
        let name = z
            .file_names()
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.ends_with(".step") || l.ends_with(".stp")
            })
            .min()
            .map(str::to_string)
            .ok_or("zip に .step・.stp が無い")?;
        let f = z
            .by_name(&name)
            .map_err(|e| format!("zip の {name}: {e}"))?;
        let mut out = Vec::new();
        f.take(MAX_BYTES + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("zip の {name}: {e}"))?;
        if out.len() as u64 > MAX_BYTES {
            return Err(format!("zip の {name} が大きすぎる"));
        }
        out
    } else {
        b
    };
    let head = String::from_utf8_lossy(&b[..b.len().min(256)]).to_ascii_uppercase();
    if !head
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("ISO-10303-21")
    {
        return Err("取った中身が STEP（ISO-10303-21）でない（ログインの要るページなど）".into());
    }
    Ok(b)
}

pub use crate::cad_detail::{detail_of_step, install_detail_loader, load_detail, measure};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_depends_on_item_and_url() {
        let d = Path::new("c");
        let a = cache_path(d, "SM1L10", "https://example.com/a.step");
        assert!(a.starts_with(d));
        assert!(a
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("SM1L10-"));
        assert_ne!(a, cache_path(d, "SM1L10", "https://example.com/b.step"));
        assert_ne!(a, cache_path(d, "SM1L05", "https://example.com/a.step"));
        let w = cache_path(d, "a/b:c", "u");
        assert!(w
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("a_b_c-"));
    }

    #[test]
    fn match_files_finds_part_numbers_in_vendor_names() {
        let items = [
            "11-211",
            "CM1L03",
            "CMA1",
            "C16-AP-12.7",
            "C16-AP-12.7A",
            "M-100.5",
        ];
        let files: Vec<PathBuf> = [
            "edm_11-211.step",
            "CM1L03_ttn170465-e0w.step",
            "CMA1_cma1-e0w.step",
            "C16_AP_12.7_STP__C16-AP-12.7.stp",
            "C16_AP_12.7A_STP__C16-AP-12.7A.stp",
            "Newport-M-100_5.zip",
            "other.step",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let (got, miss) = match_files(items, &files);
        let g: Vec<(&str, &str)> = got
            .iter()
            .map(|(n, f)| (n.as_str(), f.to_str().unwrap()))
            .collect();
        assert_eq!(
            g,
            [
                ("11-211", "edm_11-211.step"),
                ("C16-AP-12.7", "C16_AP_12.7_STP__C16-AP-12.7.stp"),
                ("C16-AP-12.7A", "C16_AP_12.7A_STP__C16-AP-12.7A.stp"),
                ("CM1L03", "CM1L03_ttn170465-e0w.step"),
                ("CMA1", "CMA1_cma1-e0w.step"),
                ("M-100.5", "Newport-M-100_5.zip"),
            ]
        );
        assert_eq!(miss, [PathBuf::from("other.step")]);
    }

    #[test]
    fn store_url_escapes_key() {
        let s = Store {
            url: "https://x.example/cad/".into(),
            ..Default::default()
        };
        assert_eq!(s.url_of("A+B 1.step"), "https://x.example/cad/A+B%201.step");
        assert!(!format!(
            "{:?}",
            Store {
                secret: "S3".into(),
                ..s.clone()
            }
        )
        .contains("S3"));
        let t = |url: &str| {
            Store {
                url: url.into(),
                id: "i".into(),
                secret: "s".into(),
            }
            .check_scheme()
        };
        assert!(t("https://x/cad").is_ok());
        assert!(t("http://127.0.0.1:9/cad").is_ok());
        assert!(t("http://x.example/cad").is_err());
        assert!(t("http://localhost/cad").is_ok());
        assert!(t("http://localhost.example.com/cad").is_err());
        assert!(t("http://127.0.0.1.example.com/cad").is_err());
        let plain = Store {
            url: "http://x.example/cad".into(),
            ..Default::default()
        };
        assert!(plain.check_scheme().is_ok());
    }

    #[test]
    fn step_bytes_accepts_step_and_zip_and_rejects_html() {
        let step = b"ISO-10303-21;\nHEADER;\nENDSEC;\n".to_vec();
        assert_eq!(step_bytes(step.clone()).unwrap(), step);
        assert!(step_bytes(b"<!DOCTYPE html><html>".to_vec()).is_err());
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("readme.txt", o).unwrap();
            std::io::Write::write_all(&mut z, b"x").unwrap();
            z.start_file("part.STP", o).unwrap();
            std::io::Write::write_all(&mut z, &step).unwrap();
            z.finish().unwrap();
        }
        assert_eq!(step_bytes(buf.into_inner()).unwrap(), step);
    }
}
