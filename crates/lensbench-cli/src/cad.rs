//! 品の CAD データ（公式の STEP）を 1 件だけ取って、キャッシュに置く（公開 #120）。
//!
//! カタログには機構部品の簡易外形（`mech`）だけを入れ、詳しい形の STEP はメーカーの配布元の URL（品の `cad.step`）
//! から、ユーザーが品を選んで求めたときにその 1 ファイルだけを取る。裏でまとめて取りには行かない。
//! キャッシュのファイルは品の名前と URL から決めるので、2 回目からはキャッシュを使い、URL が変われば取り直す。
//! 取った STEP は `catalog-step` と同じ読み方で測り、簡易外形との食い違いを返す。
use lensbench_project::{catalog_fill, UserItem};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 取るファイルの大きさの上限（STEP は 1 品あたり数 MB）
const MAX_BYTES: u64 = 64 << 20;

/// キャッシュのフォルダ: 環境変数 `LENSBENCH_CAD_CACHE`、なければ OS の設定の場所
/// （Windows は `%APPDATA%\lensbench\cad`、ほかは `$XDG_CACHE_HOME/lensbench/cad` か `~/.cache/lensbench/cad`）
pub fn default_cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LENSBENCH_CAD_CACHE").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("lensbench")
        .join("cad")
}

/// キャッシュのファイル: `<品の名前（ファイル名に使えない文字は _）>-<URL の FNV-1a 64 の 16 進>.step`
pub fn cache_path(dir: &Path, item: &str, url: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let name: String = item
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    dir.join(format!("{name}-{h:016x}.step"))
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
            "measured": self.measured.as_ref().map(|m| serde_json::to_value(m).unwrap_or_default()),
            "notes": self.notes,
        })
    }
}

/// 品の `cad.step` を取る（キャッシュにあればそれ）。品の簡易外形と測った寸法を比べる。
/// 取れなかったら Err（呼ぶ側は簡易外形のまま描き、品ページ `url` を示す）
pub fn fetch_item(dir: &Path, name: &str, item: &UserItem) -> Result<CadStep, String> {
    let url = item.cad.step.trim();
    if url.is_empty() {
        return Err(format!("{name} に STEP の URL（cad.step）がない"));
    }
    let path = cache_path(dir, name, url);
    let cached = path.is_file();
    let text = if cached {
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?
    } else {
        let b = step_bytes(download(url)?)?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // 途中で止まっても壊れたファイルを残さないよう、別名に書いてから置き換える
        let tmp = path.with_extension("step.part");
        std::fs::write(&tmp, &b).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        b
    };
    let (measured, notes) = match catalog_fill::read_step(&String::from_utf8_lossy(&text)) {
        Ok(s) => {
            let notes = match &item.mech {
                Some(m) => catalog_fill::cad_check(m, &s),
                None => Vec::new(),
            };
            (Some(s), notes)
        }
        Err(e) => (None, vec![format!("STEP を測れない: {e}")]),
    };
    Ok(CadStep {
        item: name.to_string(),
        url: url.to_string(),
        path,
        cached,
        measured,
        notes,
    })
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
