//! カタログ: 一覧・検索（glasses・lenses・catalog_search）、ユーザーカタログ、品の STEP（cad_step）、未完の品の補い（catalog_fill）

use super::*;

/// カタログの一覧（`glasses`・`lenses`）
pub(super) fn browse_of(sess: &Session, method: &str, p: &Value) -> Result<Value, RpcErr> {
    let read_all = |k: &str| -> Result<Vec<PathBuf>, RpcErr> {
        str_list(&p[k], k)?
            .iter()
            .map(|x| sess.files.check_path(x, false))
            .collect()
    };
    let dirs = read_all("glass_dir")?;
    if method == "glasses" {
        return Ok(crate::browse::glasses(&dirs));
    }
    let cat = read_all("lens_catalog")?;
    if method == "catalog_search" {
        let q = catalog_query(p)?;
        return Ok(crate::browse::search(&cat, &dirs, &q)?);
    }
    Ok(crate::browse::lenses(&cat, &dirs)?)
}

/// `catalog_search` の条件: `kinds`（品の種類の並び）・`vendor`・`text`・`specs`（`名前>=値` などの並び）
fn catalog_query(p: &Value) -> Result<lensbench_project::catalog_spec::CatalogQuery, RpcErr> {
    use lensbench_project::catalog_spec::{CatalogQuery, ItemKind, SpecFilter};
    let strs = |k: &str| -> Result<Vec<String>, RpcErr> {
        Ok(str_list(&p[k], k)?
            .into_iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect())
    };
    let kinds = strs("kinds")?
        .iter()
        .map(|k| {
            ItemKind::parse(k).ok_or_else(|| RpcErr::params(format!("kinds: 知らない種類 {k}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let specs = strs("specs")?
        .iter()
        .map(|s| SpecFilter::parse(s).map_err(RpcErr::params))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CatalogQuery {
        kinds,
        vendor: p["vendor"].as_str().unwrap_or("").to_string(),
        text: p["text"].as_str().unwrap_or("").to_string(),
        specs,
        fits: p["fits"].as_f64(),
    })
}

/// ユーザーカタログの項目の一覧（名前・説明・ブロックの数）
fn catalog_listing(path: &str, cat: &lensbench_project::UserCatalog) -> Value {
    let items: Vec<Value> = cat
        .items
        .iter()
        .map(|(k, it)| {
            json!({ "name": k, "description": it.description, "glass_catalogs": it.glass_catalogs,
                    "blocks": it.blocks.len() })
        })
        .collect();
    json!({ "catalog": path, "vendor": cat.vendor, "items": items })
}

/// 系 s の面 from〜to（1〜像面の 1 つ前）と物体面・像面だけの系（save_range）
pub fn range_system(s: &System, from: usize, to: usize) -> lensbench_core::error::Result<System> {
    let img = s.image();
    if from == 0 || from > to || to >= img {
        return Err(lensbench_core::error::Error::Invalid(format!(
            "書き出す面 {from}〜{to}（1〜{}）",
            img - 1
        )));
    }
    let mut s = s.clone();
    if let Some(stop) = s.surfaces.iter().position(|d| d.stop) {
        if !(from..=to).contains(&stop) {
            s.set_stop(from)?;
        }
    }
    for j in (to + 1..img).rev() {
        s.delete_surface(j)?;
    }
    for j in (1..from).rev() {
        s.delete_surface(j)?;
    }
    Ok(s)
}

/// ユーザーカタログの操作（GUI の保存）
/// - `catalog_add` {catalog, name, blocks: [id…]（1 つか範囲の最初と最後）, description?, replace?, vendor?}:
///   読み込んだプロジェクト（状態を当てて）のブロックを項目にする。面の表の変更は焼き込む。ファイルが無ければ作る
/// - `catalog_items` {catalog}: 項目の一覧
/// - `catalog_edit` {catalog, name, rename?, description?, delete?, vendor?}
pub(super) fn user_catalog(sess: &Session, method: &str, p: &Value) -> Result<Value, RpcErr> {
    use lensbench_project::UserCatalog;
    let path = p["catalog"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            RpcErr::params(format!(
                "{method} には catalog（ユーザーカタログのファイル）"
            ))
        })?;
    let file = sess
        .files
        .check_path(Path::new(path), method != "catalog_items")?;
    let name = || {
        p["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RpcErr::params(format!("{method} には name（項目の名前）")))
    };
    let set_vendor = |cat: &mut UserCatalog| {
        if let Some(v) = p["vendor"].as_str() {
            cat.vendor = v.trim().to_string();
        }
    };
    match method {
        "catalog_items" => {
            let cat = UserCatalog::load(&file)?;
            Ok(catalog_listing(path, &cat))
        }
        "catalog_add" => {
            let name = name()?;
            let ids: Vec<String> = match (&p["blocks"], &p["block"]) {
                (Value::Array(a), _) => a
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(|| RpcErr::params("blocks は id の並び"))?,
                (_, Value::String(b)) => vec![b.clone()],
                _ => {
                    return Err(RpcErr::params(
                        "catalog_add には blocks（ブロックの id の並び）",
                    ))
                }
            };
            let (_, spec) = sess.current()?;
            if !is_project(&spec.file) {
                return Err(RpcErr::app(
                    "catalog_add はプロジェクトファイル（組合せ）を読み込んだときだけ",
                ));
            }
            let cat = UserCatalog::load_or_new(&file)?;
            if cat.items.contains_key(name) && p["replace"].as_bool() != Some(true) {
                return Err(RpcErr::app(format!(
                    "項目 {name} はもうある（置き換えは replace: true）"
                )));
            }
            let txt = lensbench_core::vfs::read_to_string(&spec.file)
                .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
            let mut pr = lensbench_project::Project::from_json(&txt)?;
            if let Some(st) = &spec.state {
                pr = pr.with_state(st)?;
            }
            let lc = lensbench_core::catalog::LensCatalog::load_registered(&spec.lens_catalog)?;
            let look = |v: &str, q: &str| lc.zmx(v, q);
            let env = lensbench_project::Env {
                base: spec
                    .file
                    .parent()
                    .map(|x| x.to_path_buf())
                    .unwrap_or_default(),
                glass_dirs: spec.glass_dir.clone(),
                catalog: Some(&look),
            };
            let desc = p["description"].as_str().unwrap_or_default();
            let item = lensbench_project::blocks_item(&pr, &env, &ids, desc)?;
            let replace = p["replace"].as_bool() == Some(true);
            let cat = UserCatalog::update(&file, |cat| {
                if cat.items.contains_key(name) && !replace {
                    return Err(format!(
                        "項目 {name} はもうある（置き換えは replace: true）"
                    ));
                }
                cat.items.insert(name.to_string(), item);
                set_vendor(cat);
                Ok(())
            })?;
            let mut out = catalog_listing(path, &cat);
            out["added"] = json!(name);
            // 品はプロジェクトファイルの中身から作る（展開した系への set・edit はブロックへ戻せない）
            if !sess.current_log().is_empty() {
                out["notes"] = json!([
                    "読み込んだ後の変更（set・edit）は品に入らない（品はプロジェクトファイルから作る。変更を入れるには先に保存して読み直す）"
                ]);
            }
            Ok(out)
        }
        _ => {
            let name = name()?;
            let mut cat = UserCatalog::load(&file)?;
            if !cat.items.contains_key(name) {
                return Err(RpcErr::app(format!("項目 {name} はない")));
            }
            if p["delete"].as_bool() == Some(true) {
                cat.items.remove(name);
            } else {
                if let Some(d) = p["description"].as_str() {
                    cat.items.get_mut(name).unwrap().description = d.to_string();
                }
                if let Some(to) = p["rename"].as_str().map(str::trim) {
                    if to.is_empty() {
                        return Err(RpcErr::params("rename の名前が空"));
                    }
                    if to != name {
                        if cat.items.contains_key(to) {
                            return Err(RpcErr::app(format!("項目 {to} はもうある")));
                        }
                        let it = cat.items.remove(name).unwrap();
                        cat.items.insert(to.to_string(), it);
                    }
                }
            }
            set_vendor(&mut cat);
            cat.save(&file)?;
            Ok(catalog_listing(path, &cat))
        }
    }
}

/// `catalog_fill`: カタログ `catalog` の未完の品 `item` に `thickness`・`diameter`（数か [幅, 高さ]、mm）を足して、
/// 補足のカタログ `out`（ユーザーカタログ。無ければ作る）に同じ型番・同じメーカーの普通の品として書く。
/// `from` は値の決め方（page・step・manual）。枠付きの品は `frame`（機構部品の寸法、z は枠の端から）で枠も
/// 書ける（`frame_reversed` なら光は枠の z の大きい端から入る）。元のカタログは書き換えない。`out` をレンズカタログのフォルダに
/// 置けば、次の一覧では未完の品の代わりに並ぶ
/// {catalog, item, cache?} → {item, url, path, cached, measured, notes}。取れなければ誤りで、品ページの URL を添える
pub(super) fn cad_step(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (sess, p);
        Err(RpcErr::app("STEP の取得は Web 版では使えない"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let s = |k: &str| {
            p[k].as_str()
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .ok_or_else(|| RpcErr::params(format!("cad_step には {k}")))
        };
        let src = sess.files.check_path(Path::new(s("catalog")?), false)?;
        let name = s("item")?;
        let cat = lensbench_project::UserCatalog::load(&src)?;
        let it = cat
            .items
            .get(name)
            .ok_or_else(|| RpcErr::app(format!("{} に項目 {name} がない", src.display())))?;
        let dir = match p["cache"].as_str().map(str::trim).filter(|x| !x.is_empty()) {
            Some(d) => sess.files.check_path(Path::new(d), true)?,
            // 既定の場所も --root の中でなければ書かない
            None => sess
                .files
                .check_path(&lensbench_cad::default_cache_dir(), true)?,
        };
        lensbench_cad::fetch_item(&dir, name, it)
            .map(|r| r.to_json())
            .map_err(|e| {
                if it.url.is_empty() {
                    RpcErr::app(e)
                } else {
                    RpcErr::app(format!("{e}（品ページ: {}）", it.url))
                }
            })
    }
}

pub(super) fn catalog_fill(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    use lensbench_project::{catalog_fill as cf, UserCatalog};
    let s = |k: &str| {
        p[k].as_str()
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .ok_or_else(|| RpcErr::params(format!("catalog_fill には {k}")))
    };
    let src = sess.files.check_path(Path::new(s("catalog")?), false)?;
    let out = sess.files.check_path(Path::new(s("out")?), true)?;
    let name = s("item")?;
    let cat = UserCatalog::load(&src)?;
    let it = cat
        .items
        .get(name)
        .ok_or_else(|| RpcErr::app(format!("{} に項目 {name} がない", src.display())))?;
    if !it.is_incomplete() {
        return Err(RpcErr::app(format!("{name} は未完の品でない")));
    }
    let vendor = if cat.vendor.trim().is_empty() {
        src.parent()
            .and_then(|d| d.file_name())
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        cat.vendor.clone()
    };
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| cf::date_of_unix(d.as_secs()))
        .unwrap_or_default();
    let frame = match p.get("frame").filter(|v| !v.is_null()) {
        Some(v) => Some(
            serde_json::from_value::<lensbench_project::MechSpec>(v.clone())
                .map_err(|e| RpcErr::params(format!("frame: {e}")))?,
        ),
        None => None,
    };
    let mut filled = cf::complete(
        it,
        p["thickness"].as_f64(),
        p.get("diameter").filter(|v| !v.is_null()).cloned(),
        frame,
        cf::FillFrom::parse(p["from"].as_str().unwrap_or("")),
        &today,
    )
    .map_err(RpcErr::params)?;
    if filled.frame.is_some() {
        filled.frame_reversed = p["frame_reversed"].as_bool().unwrap_or(false);
    }
    // 仕様表の値と食い違う寸法は notes に残す
    if let Some(sv) = p.get("spec").filter(|v| v.is_object()) {
        let spec = cf::SpecValues::from_value(sv);
        let num = |k: &str| filled.specs.get(k).and_then(Value::as_f64);
        let diffs = cf::spec_check(
            num("thickness"),
            num("diameter"),
            filled.frame.as_ref(),
            p["glass_visible"].as_bool().unwrap_or(false),
            &spec,
        );
        for d in diffs.iter().filter(|d| !d.ok()) {
            filled.notes.push(d.note());
        }
        // 有効径は枠の穴でなく仕様表の値（品に無ければ）
        if let (Some(ca), false) = (spec.clear_aperture, it.specs.contains_key("clear_aperture")) {
            filled
                .specs
                .insert("clear_aperture".into(), serde_json::json!(ca));
        }
    }
    let mut fc = UserCatalog::load_or_new(&out)?;
    if !fc.vendor.is_empty() && fc.vendor != vendor {
        return Err(RpcErr::app(format!(
            "{} はメーカー {} の補足のカタログ（{vendor} の品は別のファイルに）",
            out.display(),
            fc.vendor
        )));
    }
    fc.vendor = vendor.clone();
    // 膜の名前などを引けるよう、元のカタログの膜も写す
    for (k, v) in &cat.coatings {
        fc.coatings.entry(k.clone()).or_insert_with(|| v.clone());
    }
    fc.items.insert(name.to_string(), filled.clone());
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        lensbench_core::vfs::create_dir_all(d)
            .map_err(|e| RpcErr::app(format!("{}: {e}", d.display())))?;
    }
    fc.save(&out)?;
    Ok(json!({
        "catalog": out.display().to_string(),
        "item": name,
        "vendor": vendor,
        "specs": filled.specs,
        "notes": filled.notes,
        "frame": filled.frame,
        "frame_reversed": filled.frame_reversed,
    }))
}
