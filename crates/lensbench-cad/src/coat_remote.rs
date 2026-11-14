//! 膜のモデル（`.lbcb` など）を共有の置き場から取る（配る版がリリースの大きさを抑えるため、膜のモデルを外したとき）。
//!
//! 膜ファイルのフォルダに目録 `lbcb-remote.json`（`{"version": 1, "files": {"<膜の名前>": "<中身の FNV-1a 64 の 16 進>"}}`）
//! を置くと、フォルダに無い膜のモデルを置き場（[`crate::store`]、品の STEP と同じ口）の鍵
//! `coat/<膜の名前>-<ハッシュ>.lbcb` から取り、キャッシュ（品の STEP のキャッシュの隣の `coat`）に置いて読む。
//! 中身のハッシュを確かめるので、作り直した膜は別の鍵になり、古いキャッシュを使わない。
//! 目録と置き場への上げは `lensbench coat-upload`、まとめて取ってフォルダに置くのは `lensbench coat-fetch`
//! （Python など、置き場を見ない使い方のため）。
use crate as cad;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 目録のファイル名
pub const INDEX: &str = "lbcb-remote.json";

/// 中身のハッシュ（FNV-1a 64 の 16 進）
pub fn hash_hex(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 置き場の鍵
pub fn key_of(name: &str, hash: &str) -> String {
    format!("coat/{name}-{hash}.lbcb")
}

/// 目録（膜の名前、ハッシュ）。無ければ空
pub fn read_index(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let p = dir.join(INDEX);
    if !p.is_file() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            .map_err(|e| format!("{}: {e}", p.display()))?;
    let mut out = Vec::new();
    for (k, h) in v["files"].as_object().into_iter().flatten() {
        let h = h.as_str().unwrap_or("");
        if h.len() == 16 && h.bytes().all(|c| c.is_ascii_hexdigit()) && !k.contains(['/', '\\']) {
            out.push((k.clone(), h.to_ascii_lowercase()));
        }
    }
    Ok(out)
}

/// キャッシュのフォルダ
pub fn cache_dir() -> PathBuf {
    lensbench_project::catalog_fill::cad_cache_dir().join("coat")
}

/// 1 つ取る（キャッシュにあればそれ）。ハッシュが合わなければ誤り
pub fn fetch(store: Option<&cad::Store>, name: &str, hash: &str) -> Result<PathBuf, String> {
    let dir = cache_dir();
    let path = dir.join(format!("{name}-{hash}.lbcb"));
    if path.is_file() {
        return Ok(path);
    }
    let s = store.ok_or("置き場が無い（LENSBENCH_CAD_STORE）")?;
    let key = key_of(name, hash);
    let b = s
        .get_raw(&key)?
        .ok_or_else(|| format!("置き場に膜 {name} のモデルが無い（{key}）"))?;
    if hash_hex(&b) != hash {
        return Err(format!("置き場の膜 {name} の中身が目録と違う（{key}）"));
    }
    cad::write_cache(&dir, &path, &b)?;
    Ok(path)
}

/// [`lensbench_core::coating::set_remote_loader`] に渡す口。取れなければ None（初回だけ誤りを stderr に出す）
pub fn remote_loader(dir: &Path, coat: &str) -> Option<PathBuf> {
    static MISSES: Mutex<Option<HashMap<String, ()>>> = Mutex::new(None);
    let idx = read_index(dir).ok()?;
    let (name, hash) = idx.iter().find(|(n, _)| n.eq_ignore_ascii_case(coat))?;
    let k = key_of(name, hash);
    if MISSES
        .lock()
        .ok()?
        .get_or_insert_with(HashMap::new)
        .contains_key(&k)
    {
        return None;
    }
    match fetch(cad::store().as_ref(), name, hash) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("膜 {name} のモデルを取れない: {e}");
            MISSES
                .lock()
                .ok()?
                .get_or_insert_with(HashMap::new)
                .insert(k, ());
            None
        }
    }
}

/// 膜のモデルを置き場から取れるようにする（CLI・serve・GUI の起動時）
pub fn install() {
    lensbench_core::coating::set_remote_loader(remote_loader);
}

/// フォルダの `.lbcb` の目録を作り（`out`、省略でフォルダの `lbcb-remote.json`）、置き場に無いものを上げる
/// （dry_run なら上げない）。返すのは（膜、鍵、"uploaded"・"exists"・"dry_run"・"error"、誤り）
pub fn upload_dir(
    dir: &Path,
    out: Option<&Path>,
    store: Option<&cad::Store>,
    dry_run: bool,
) -> Result<Vec<(String, String, &'static str, String)>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("lbcb"))
        })
        .collect();
    files.sort();
    let mut index = serde_json::Map::new();
    let mut rows = Vec::new();
    for f in files {
        let name = f
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let b = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let h = hash_hex(&b);
        let key = key_of(&name, &h);
        index.insert(name.clone(), serde_json::Value::String(h));
        let r = if dry_run {
            Ok("dry_run")
        } else {
            match store {
                None => Err("置き場が無い（LENSBENCH_CAD_STORE）".to_string()),
                Some(s) => s.has(&key).and_then(|has| {
                    if has {
                        Ok("exists")
                    } else {
                        s.put(&key, &b).map(|_| "uploaded")
                    }
                }),
            }
        };
        rows.push(match r {
            Ok(st) => (name, key, st, String::new()),
            Err(e) => (name, key, "error", e),
        });
    }
    let out = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join(INDEX));
    let text = serde_json::to_string_pretty(&serde_json::json!({ "version": 1, "files": index }))
        .map_err(|e| e.to_string())?;
    std::fs::write(&out, text + "\n").map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(rows)
}

/// 目録の膜のモデルをすべて取り、フォルダに `<膜>.lbcb` として置く（あるものは飛ばす）。
/// 返すのは（膜、"fetched"・"exists"・"error"、誤り）
pub fn fetch_dir(
    dir: &Path,
    store: Option<&cad::Store>,
) -> Result<Vec<(String, &'static str, String)>, String> {
    let idx = read_index(dir)?;
    if idx.is_empty() {
        return Err(format!("{} に目録 {INDEX} が無い", dir.display()));
    }
    let mut rows = Vec::new();
    for (name, hash) in idx {
        let dst = dir.join(format!("{name}.lbcb"));
        if dst.is_file() {
            rows.push((name, "exists", String::new()));
            continue;
        }
        let r = fetch(store, &name, &hash).and_then(|p| {
            std::fs::copy(&p, &dst)
                .map(|_| ())
                .map_err(|e| format!("{}: {e}", dst.display()))
        });
        rows.push(match r {
            Ok(()) => (name, "fetched", String::new()),
            Err(e) => (name, "error", e),
        });
    }
    Ok(rows)
}
