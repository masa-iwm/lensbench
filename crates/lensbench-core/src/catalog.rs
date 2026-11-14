//! レンズカタログの差し込み口。
//!
//! CLI・GUI・Python が読む形式は [`register_source`] で足せる（既定は .zmx と .lbcat）。別の exe に組み込むときは、
//! 起動の最初に登録してから CLI・GUI の入口を呼ぶ。
//!
//! .lbcat は lensbench のレンズカタログの束（[`encode_lbcat`]）: メーカーのカタログ（ZMF など）を取り込みのときに
//! 1 回だけ変換したもの。実行時は解読せず、展開して読むだけ。

use crate::RwLockExt;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

/// カタログ中の 1 レンズ（面データは .zmx 断片のまま持つ）。
#[derive(Debug, Clone)]
pub struct CatalogLens {
    pub vendor: String,
    pub part: String,
    pub efl: f64,
    pub zmx: String,
}

/// .lbcat の形式の版
pub const LBCAT_VERSION: u32 = 1;

/// レンズカタログの束 .lbcat のバイト列。zlib で圧縮した UTF-8 の文で、1 行目が
/// `LBCAT <版>\t<メーカー>\t<変換元>`、続いてレンズごとに `<型番>\t<焦点距離>\t<.zmx のバイト数>` の行と .zmx の本文
/// （そのバイト数ちょうど）。焦点距離が数でないレンズは `nan`。型番・メーカー・変換元にタブ・改行は使えない
pub fn encode_lbcat(vendor: &str, source: &str, lenses: &[CatalogLens]) -> Result<Vec<u8>, String> {
    let clean = |what: &str, s: &str| {
        if s.contains(['\t', '\n', '\r']) {
            Err(format!("{what} {s:?} にタブ・改行がある"))
        } else {
            Ok(())
        }
    };
    clean("メーカー", vendor)?;
    clean("変換元", source)?;
    let mut out = format!("LBCAT {LBCAT_VERSION}\t{vendor}\t{source}\n").into_bytes();
    for l in lenses {
        clean("型番", &l.part)?;
        let efl = if l.efl.is_finite() {
            format!("{:?}", l.efl)
        } else {
            "nan".into()
        };
        out.extend(format!("{}\t{efl}\t{}\n", l.part, l.zmx.len()).bytes());
        out.extend(l.zmx.as_bytes());
    }
    Ok(miniz_oxide::deflate::compress_to_vec_zlib(&out, 9))
}

/// [`encode_lbcat`] のバイト列を読む。メーカーは束の値（空なら `default_vendor`）
pub fn decode_lbcat(bytes: &[u8], default_vendor: &str) -> Result<Vec<CatalogLens>, String> {
    let b = miniz_oxide::inflate::decompress_to_vec_zlib(bytes)
        .map_err(|e| format!("展開できない（{:?}）", e.status))?;
    let mut o = 0usize;
    let line = |o: &mut usize| -> Result<&str, String> {
        let end = b[*o..]
            .iter()
            .position(|&c| c == b'\n')
            .ok_or("行が途中で切れている")?;
        let s = std::str::from_utf8(&b[*o..*o + end]).map_err(|_| "UTF-8 でない行")?;
        *o += end + 1;
        Ok(s)
    };
    let head: Vec<String> = line(&mut o)?.split('\t').map(str::to_string).collect();
    let ver: u32 = head[0]
        .strip_prefix("LBCAT ")
        .and_then(|v| v.parse().ok())
        .ok_or("lensbench のレンズカタログの束でない")?;
    if ver == 0 || ver > LBCAT_VERSION {
        return Err(format!("形式の版 {ver} は読めない（{LBCAT_VERSION} まで）"));
    }
    let vendor = match head.get(1) {
        Some(v) if !v.trim().is_empty() => v.clone(),
        _ => default_vendor.to_string(),
    };
    let mut out = Vec::new();
    while o < b.len() {
        let f: Vec<String> = line(&mut o)?.split('\t').map(str::to_string).collect();
        let [part, efl, n] = &f[..] else {
            return Err(format!(
                "レンズの行の欄が 3 つでない（{} 本目）",
                out.len() + 1
            ));
        };
        let n: usize = n
            .parse()
            .map_err(|_| format!("{part}: .zmx のバイト数 {n}"))?;
        let end = o
            .checked_add(n)
            .filter(|&e| e <= b.len())
            .ok_or_else(|| format!("{part}: .zmx が途中で切れている"))?;
        let zmx = std::str::from_utf8(&b[o..end])
            .map_err(|_| format!("{part}: .zmx が UTF-8 でない"))?
            .to_string();
        o = end;
        out.push(CatalogLens {
            vendor: vendor.clone(),
            part: part.clone(),
            efl: efl.parse().unwrap_or(f64::NAN),
            zmx,
        });
    }
    Ok(out)
}

/// レンズカタログの束 .lbcat（[`encode_lbcat`]）。メーカーは束の値（なければファイル名）
pub struct LbcatSource;

impl LensCatalogSource for LbcatSource {
    fn extensions(&self) -> &[&str] {
        &["lbcat"]
    }
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String> {
        let bytes = crate::vfs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        decode_lbcat(&bytes, &stem).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// 外部のレンズカタログを読む実装が満たすトレイト。
pub trait LensCatalogSource {
    /// この実装が読める拡張子（小文字、ドットなし）。
    fn extensions(&self) -> &[&str];
    /// ファイルを読み、レンズの一覧を返す。
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String>;
}

/// .zmx 1 つを 1 レンズとするカタログ（自作のレンズや、カタログから書き出したレンズ）。
/// メーカーは親フォルダの名前、型番はファイル名（拡張子なし）。焦点距離はカタログの値がないので NaN。
pub struct ZmxFileSource;

impl LensCatalogSource for ZmxFileSource {
    fn extensions(&self) -> &[&str] {
        &["zmx"]
    }
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String> {
        let zmx = crate::glass::read_text(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = |p: Option<&Path>| {
            p.and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let part = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 相対パスでも親フォルダの名前が取れるように、絶対パスにしてから見る
        let abs = crate::vfs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Ok(vec![CatalogLens {
            vendor: name(abs.parent()),
            part,
            efl: f64::NAN,
            zmx,
        }])
    }
}

fn registered() -> &'static RwLock<Vec<Arc<dyn LensCatalogSource + Send + Sync>>> {
    static G: OnceLock<RwLock<Vec<Arc<dyn LensCatalogSource + Send + Sync>>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// プロセスで共通に使う形式を足す（同じ拡張子なら後から足したものを使う）。
/// [`LensCatalog::load_registered`] と、それを使う CLI・GUI・Python のすべてに効く
pub fn register_source(s: Arc<dyn LensCatalogSource + Send + Sync>) {
    registered().write_ok().push(s);
}

/// プロセスで共通に使う形式の一覧（足したものが先、最後に [`LbcatSource`]・[`ZmxFileSource`]）
pub fn sources() -> Vec<Arc<dyn LensCatalogSource + Send + Sync>> {
    let mut v: Vec<Arc<dyn LensCatalogSource + Send + Sync>> =
        registered().read_ok().iter().rev().cloned().collect();
    v.push(Arc::new(LbcatSource));
    v.push(Arc::new(ZmxFileSource));
    v
}

/// 読み込んだレンズカタログの集まり。メーカー・型番は大文字・小文字を区別しない。
#[derive(Debug, Clone, Default)]
pub struct LensCatalog {
    lenses: Vec<CatalogLens>,
    index: std::collections::HashMap<(String, String), usize>,
}

impl LensCatalog {
    /// ファイルまたはフォルダ（中のファイル、1 階層）を、拡張子が合う source で読む。
    /// 同じメーカー・型番は後に読んだものを使う。
    pub fn load(
        paths: &[std::path::PathBuf],
        sources: &[&dyn LensCatalogSource],
    ) -> Result<LensCatalog, String> {
        let mut c = LensCatalog::default();
        for p in paths {
            let mut files = Vec::new();
            if crate::vfs::is_dir(p) {
                let rd = crate::vfs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
                let known = |f: &std::path::PathBuf| {
                    let ext = f
                        .extension()
                        .map(|x| x.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    sources
                        .iter()
                        .any(|s| s.extensions().contains(&ext.as_str()))
                };
                files.extend(rd.iter().filter(|f| crate::vfs::is_file(f)).cloned());
                // カタログのファイルが 1 つも無ければ、メーカーのフォルダの親を渡したとみて 1 段下も探す
                if !files.iter().any(known) {
                    for d in rd.iter().filter(|d| crate::vfs::is_dir(d)) {
                        if let Ok(sub) = crate::vfs::read_dir(d) {
                            files.extend(
                                sub.into_iter()
                                    .filter(|f| crate::vfs::is_file(f) && known(f)),
                            );
                        }
                    }
                }
                files.sort();
            } else {
                files.push(p.clone());
            }
            for f in files {
                let ext = f
                    .extension()
                    .map(|x| x.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                let src = sources
                    .iter()
                    .find(|s| s.extensions().contains(&ext.as_str()));
                match src {
                    Some(s) => {
                        for l in s.load(&f)? {
                            c.add(l);
                        }
                    }
                    None if crate::vfs::is_dir(p) => {} // フォルダの中の関係のないファイルは飛ばす
                    None => return Err(format!("{}: 読めない形式のレンズカタログ", f.display())),
                }
            }
        }
        Ok(c)
    }

    /// [`load`](Self::load) を、プロセスで共通の形式（[`sources`]）で
    pub fn load_registered(paths: &[std::path::PathBuf]) -> Result<LensCatalog, String> {
        let src = sources();
        let refs: Vec<&dyn LensCatalogSource> = src
            .iter()
            .map(|s| s.as_ref() as &dyn LensCatalogSource)
            .collect();
        Self::load(paths, &refs)
    }

    fn key(vendor: &str, part: &str) -> (String, String) {
        (vendor.trim().to_uppercase(), part.trim().to_uppercase())
    }

    pub fn add(&mut self, l: CatalogLens) {
        let k = Self::key(&l.vendor, &l.part);
        match self.index.get(&k) {
            Some(&i) => self.lenses[i] = l,
            None => {
                self.index.insert(k, self.lenses.len());
                self.lenses.push(l);
            }
        }
    }

    pub fn get(&self, vendor: &str, part: &str) -> Option<&CatalogLens> {
        self.index
            .get(&Self::key(vendor, part))
            .map(|&i| &self.lenses[i])
    }

    /// (メーカー, 型番) → .zmx の本文（プロジェクトの展開に渡す形）
    pub fn zmx(&self, vendor: &str, part: &str) -> Result<String, String> {
        self.get(vendor, part)
            .map(|l| l.zmx.clone())
            .ok_or_else(|| format!("レンズカタログに {vendor} {part} がない"))
    }

    pub fn lenses(&self) -> &[CatalogLens] {
        &self.lenses
    }

    pub fn len(&self) -> usize {
        self.lenses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lenses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// .lbcat は書いたとおりに読み戻せる（焦点距離のビット・数でない値・複数行と非 ASCII の .zmx・空の束）
    #[test]
    fn lbcat_round_trips() {
        let lenses = vec![
            CatalogLens {
                vendor: "V".into(),
                part: "A-1".into(),
                efl: 0.1 + 0.2,
                zmx: "VERS 1\nNAME A-1\r\nGLAS N-BK7 µ\n".into(),
            },
            CatalogLens {
                vendor: "V".into(),
                part: "B 2".into(),
                efl: f64::NAN,
                zmx: String::new(),
            },
        ];
        let b = encode_lbcat("ACME", "ACME.ZMF", &lenses).unwrap();
        let r = decode_lbcat(&b, "fallback").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].vendor, "ACME");
        assert_eq!(r[0].part, "A-1");
        assert_eq!(r[0].efl.to_bits(), (0.1f64 + 0.2).to_bits());
        assert_eq!(r[0].zmx, lenses[0].zmx);
        assert!(r[1].efl.is_nan() && r[1].zmx.is_empty() && r[1].part == "B 2");
        let e = decode_lbcat(&encode_lbcat("", "x", &[]).unwrap(), "STEM").unwrap();
        assert!(e.is_empty());
        let one = encode_lbcat("", "x", &lenses[..1]).unwrap();
        assert_eq!(decode_lbcat(&one, "STEM").unwrap()[0].vendor, "STEM");
        assert!(encode_lbcat("A\tB", "x", &[]).is_err());
        assert!(decode_lbcat(b"not zlib", "x").is_err());
        // 壊れた束（.zmx のバイト数が巨大）は誤りで返す（桁あふれで落ちない）
        let bad = format!("LBCAT 1\tV\tS\nA\t1\t{}\n", usize::MAX);
        let z = miniz_oxide::deflate::compress_to_vec_zlib(bad.as_bytes(), 6);
        assert!(decode_lbcat(&z, "x").is_err());
    }
}
