//! 使ったガラスのカタログ（`catalogs`）: 保存ではカタログの名前とファイルの hash を残し（ガラスの式は書かない）、
//! 読むときにファイルを探してガラスを名前で引く。
//!
//! 保存では、このプロセスで読んだ AGF（[`glass::loaded_files`]）・既定のフォルダ（LENSBENCH_GLASSCAT）の順に、
//! 系のガラスと値が同じファイルを探して hash を書く。見つからなければ hash は空にし、注意を返す（[`unverified`]）。
//!
//! 読むときは渡したフォルダ（`glass_dirs`）・系ファイルのフォルダ・既定のフォルダ（LENSBENCH_GLASSCAT）・このプロセスで
//! 読んだ AGF の順に探し、最初に見つかったファイル（今のカタログ）を使う。hash が合うものを選ぶのではない。hash が
//! 書いたときと違えば注意を返し、hash が空なら値を確かめられないと注意を返す。見つからなければ誤り。

use super::CatalogRef;
use lensbench_core::glass::{self, Glass};
use lensbench_core::surface::Bire;
use lensbench_core::{vfs, LockExt, System};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Catalog = Arc<HashMap<String, Glass>>;

/// 系が使うガラス（名前 → 系のガラス）: 面の媒質、複屈折材の異常光、波長板の板、ゴーストの反射の媒質
fn used(s: &System) -> BTreeMap<String, &Glass> {
    let mut out = BTreeMap::new();
    let mut add = |name: &str| {
        let n = name.trim();
        if let Some(g) = s.glasses.get(n) {
            out.insert(n.to_string(), g);
        }
    };
    for d in &s.surfaces {
        if d.is_coord_break() {
            continue;
        }
        if !d.mirror && d.grin.is_none() {
            add(&d.mat);
            if matches!(d.bire, Some(Bire::In { .. })) {
                add(&Bire::e_name(&d.mat));
            }
        }
        if let Some(r) = &d.retarder {
            for p in &r.plates {
                add(&p.material);
                add(&Bire::e_name(&p.material));
            }
        }
        if let Some(g) = &d.reflect {
            add(&g.mat);
        }
    }
    out
}

/// ファイル名（拡張子を除く）が name の AGF か
fn is_catalog(p: &Path, name: &str) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("agf"))
        && p.file_stem()
            .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
}

/// カタログ name のファイルの候補（フォルダの順、各フォルダで最初のもの。その後にこのプロセスで読んだ AGF）
fn candidates(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if let Ok(rd) = vfs::read_dir(d) {
            if let Some(p) = rd.into_iter().find(|p| is_catalog(p, name)) {
                out.push(p);
            }
        }
    }
    out.extend(
        glass::loaded_files()
            .into_iter()
            .filter(|p| is_catalog(p, name)),
    );
    let mut seen = Vec::new();
    out.retain(|p| {
        let k = vfs::canonicalize(p).unwrap_or_else(|_| p.clone());
        let new = !seen.contains(&k);
        seen.push(k);
        new
    });
    out
}

/// ファイルの中身の SHA-256（"sha256:…"）。更新時刻と大きさが同じなら覚えた値
pub(crate) fn file_hash(p: &Path) -> std::io::Result<String> {
    type Cache = HashMap<PathBuf, (vfs::Stamp, String)>;
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    let st = vfs::stamp(p)?;
    if let Some((s, h)) = CACHE.lock_ok().as_ref().and_then(|c| c.get(p)) {
        if *s == st {
            return Ok(h.clone());
        }
    }
    let b = vfs::read(p)?;
    let hex: String = Sha256::digest(&b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let h = format!("sha256:{hex}");
    let mut c = CACHE.lock_ok();
    let c = c.get_or_insert_with(HashMap::new);
    if c.len() > 256 {
        c.clear();
    }
    c.insert(p.to_path_buf(), (st, h.clone()));
    Ok(h)
}

/// カタログの名前、系のガラスと値が同じファイル（見つからなければ None）、そのカタログから使うガラスの名前
type Source = (String, Option<(PathBuf, Catalog)>, Vec<String>);

/// 使ったカタログごとの [`Source`]
fn sources(s: &System) -> Vec<Source> {
    let mut by_cat: BTreeMap<String, Vec<(String, &Glass)>> = BTreeMap::new();
    for (n, g) in used(s) {
        by_cat.entry(g.catalog.clone()).or_default().push((n, g));
    }
    let dirs = glass::default_dirs();
    by_cat
        .into_iter()
        .map(|(cat, gs)| {
            let names: Vec<String> = gs.iter().map(|(n, _)| n.clone()).collect();
            let mut cands = glass::loaded_files()
                .into_iter()
                .filter(|p| is_catalog(p, &cat))
                .collect::<Vec<_>>();
            cands.extend(candidates(&cat, &dirs));
            let found = cands.into_iter().find_map(|p| {
                let m = glass::load_agf_shared(&p).ok()?;
                gs.iter()
                    .all(|(n, g)| m.get(n) == Some(*g))
                    .then_some((p, m))
            });
            (cat, found, names)
        })
        .collect()
}

/// 名前で引くと同じガラスになる並び: カタログ A のガラスを、前にあるカタログ B も持っていれば A を B の前へ
fn order(mut v: Vec<Source>) -> Vec<(String, Option<PathBuf>)> {
    for _ in 0..v.len() * v.len() {
        let mut moved = false;
        'find: for a in 0..v.len() {
            for b in 0..a {
                let Some((_, mb)) = &v[b].1 else { continue };
                if v[a].2.iter().any(|n| mb.contains_key(n)) {
                    let x = v.remove(a);
                    v.insert(b, x);
                    moved = true;
                    break 'find;
                }
            }
        }
        if !moved {
            break;
        }
    }
    v.into_iter().map(|(n, f, _)| (n, f.map(|x| x.0))).collect()
}

/// 系ファイルの `catalogs`
pub(crate) fn refs(s: &System) -> lensbench_core::Result<Vec<CatalogRef>> {
    order(sources(s))
        .into_iter()
        .map(|(name, f)| {
            let hash = match f {
                Some(p) => file_hash(&p).map_err(lensbench_core::Error::io_at(p.display()))?,
                None => String::new(),
            };
            Ok(CatalogRef { name, hash })
        })
        .collect()
}

/// 系のガラスと値が同じファイルが見つからなかったカタログへの注意（`catalogs` の hash が空になり、読むときに値を
/// 確かめられない。.zmx の系を、元のカタログが無いところや違うところで系ファイルにしたとき）
pub(crate) fn unverified(s: &System) -> Vec<String> {
    sources(s)
        .into_iter()
        .filter(|x| x.1.is_none())
        .map(|(name, _, gs)| {
            format!(
                "no file of glass catalog {} has the same values as the system's glasses ({}); the system file records no hash for it, so the values can't be checked when it is read",
                name,
                gs.join(", ")
            )
        })
        .collect()
}

/// 系のガラスの元のカタログのファイル（見つかったものだけ。pickle で別のプロセスに渡す）
pub(crate) fn files(s: &System) -> Vec<PathBuf> {
    order(sources(s)).into_iter().filter_map(|x| x.1).collect()
}

/// `catalogs` のカタログをフォルダ dirs（とこのプロセスで読んだ AGF）から読み、名前 → ガラスの表にする（前の
/// カタログが先）。返すのは (ガラス, 注意)
pub(crate) fn load(
    refs: &[CatalogRef],
    dirs: &[PathBuf],
) -> lensbench_core::Result<(HashMap<String, Glass>, Vec<String>)> {
    let mut out: HashMap<String, Glass> = HashMap::new();
    let mut notes = Vec::new();
    for r in refs {
        let cands = candidates(&r.name, dirs);
        // 探す順で最初に見つかったもの（今のカタログ）を使う
        let pick = cands.first().cloned();
        let Some(p) = pick else {
            return Err(lensbench_core::Error::Invalid(format!(
                "glass catalog {} was not found{}",
                r.name,
                glass::DIRS_HINT
            ))
            .with_code(lensbench_core::error::code::GLASS_NOT_FOUND));
        };
        if r.hash.is_empty() {
            notes.push(format!(
                "glass catalog {} has no hash in the system file, so its values can't be checked; the values are computed from the current catalog ({})",
                r.name,
                p.display()
            ));
        } else if file_hash(&p).ok().as_deref() != Some(r.hash.as_str()) {
            notes.push(format!(
                "glass catalog {} differs from when the file was saved ({}); the values are computed from the current catalog",
                r.name,
                p.display()
            ));
        }
        let g = glass::load_agf_shared(&p).map_err(lensbench_core::Error::io_at(p.display()))?;
        for (k, v) in g.iter() {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    Ok((out, notes))
}
