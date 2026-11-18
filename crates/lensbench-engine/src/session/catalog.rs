//! カタログ: 一覧・検索（glasses・lenses・catalog_search）、ユーザーカタログ、品の STEP（cad_step）、未完の品の補い（catalog_fill）

use super::*;
use serde::{Deserialize, Serialize};

/// `glasses`・`lenses` の params（名前は load と同じ）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowseReq {
    pub glass_dirs: Vec<PathBuf>,
    /// `lenses` だけ
    pub lens_catalogs: Vec<PathBuf>,
}

/// `catalog_search` の params: カタログ（`glass_dirs`・`lens_catalogs`）と条件。`kinds` は品の種類の名前、
/// `specs` は `名前>=値` などの並び、`fits` は入る径（mm）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchReq {
    pub glass_dirs: Vec<PathBuf>,
    pub lens_catalogs: Vec<PathBuf>,
    pub kinds: Vec<String>,
    pub vendor: String,
    pub text: String,
    pub specs: Vec<String>,
    pub fits: Option<f64>,
}

impl BrowseReq {
    /// `glasses`・`lenses` の params（知らない鍵は誤り）
    pub(super) fn parse(method: &str, p: &Value) -> Result<BrowseReq, RpcErr> {
        let keys: &[&str] = match method {
            "glasses" => &["glass_dirs"],
            _ => &["glass_dirs", "lens_catalogs"],
        };
        known_keys(p, keys)?;
        Ok(BrowseReq {
            glass_dirs: str_list(&p["glass_dirs"], "glass_dirs")?,
            lens_catalogs: str_list(&p["lens_catalogs"], "lens_catalogs")?,
        })
    }
}

impl SearchReq {
    /// `catalog_search` の params（知らない鍵・知らない種類・読めない条件は誤り）
    pub(super) fn parse(p: &Value) -> Result<SearchReq, RpcErr> {
        known_keys(
            p,
            &[
                "glass_dirs",
                "lens_catalogs",
                "kinds",
                "vendor",
                "text",
                "specs",
                "fits",
            ],
        )?;
        let strs = |k: &str| -> Result<Vec<String>, RpcErr> {
            Ok(str_list(&p[k], k)?
                .into_iter()
                .map(|x| x.to_string_lossy().into_owned())
                .collect())
        };
        let r = SearchReq {
            glass_dirs: str_list(&p["glass_dirs"], "glass_dirs")?,
            lens_catalogs: str_list(&p["lens_catalogs"], "lens_catalogs")?,
            kinds: strs("kinds")?,
            vendor: p["vendor"].as_str().unwrap_or("").to_string(),
            text: p["text"].as_str().unwrap_or("").to_string(),
            specs: strs("specs")?,
            fits: p["fits"].as_f64(),
        };
        r.query()?;
        Ok(r)
    }

    /// 探す条件
    fn query(&self) -> Result<lensbench_catalog::spec::CatalogQuery, RpcErr> {
        use lensbench_catalog::spec::{CatalogQuery, ItemKind, SpecFilter};
        let kinds = self
            .kinds
            .iter()
            .map(|k| {
                ItemKind::parse(k).ok_or_else(|| RpcErr::params(format!("kinds: unknown kind {k}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let specs = self
            .specs
            .iter()
            .map(|s| SpecFilter::parse(s).map_err(RpcErr::params))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CatalogQuery {
            kinds,
            vendor: self.vendor.clone(),
            text: self.text.clone(),
            specs,
            fits: self.fits,
        })
    }
}

fn read_all(sess: &Session, v: &[PathBuf]) -> Result<Vec<PathBuf>, RpcErr> {
    v.iter().map(|x| sess.files.check_path(x, false)).collect()
}

/// ガラスの一覧（`glasses`）
pub(super) fn glasses(sess: &Session, r: &BrowseReq) -> Result<Value, RpcErr> {
    Ok(crate::browse::glasses(&read_all(sess, &r.glass_dirs)?))
}

/// レンズの一覧（`lenses`）
pub(super) fn lenses(sess: &Session, r: &BrowseReq) -> Result<Value, RpcErr> {
    let dirs = read_all(sess, &r.glass_dirs)?;
    let cat = read_all(sess, &r.lens_catalogs)?;
    Ok(crate::browse::lenses(&cat, &dirs)?)
}

/// カタログを探す（`catalog_search`）
pub(super) fn catalog_search(sess: &Session, r: &SearchReq) -> Result<Value, RpcErr> {
    let dirs = read_all(sess, &r.glass_dirs)?;
    let cat = read_all(sess, &r.lens_catalogs)?;
    Ok(crate::browse::search(&cat, &dirs, &r.query()?)?)
}

/// ユーザーカタログの項目の一覧（名前・説明・ブロックの数）
fn catalog_listing(path: &str, cat: &lensbench_project::UserCatalog) -> Value {
    let items: Vec<Value> = cat
        .items
        .iter()
        .map(|(k, it)| {
            json!({ "name": k, "description": it.description, "glass_catalogs": it.glass_catalogs,
                    "blocks": it.blocks().len() })
        })
        .collect();
    json!({ "catalog": path, "vendor": cat.vendor, "items": items })
}

/// 系 s の面 from〜to（1〜像面の 1 つ前）と物体面・像面だけの系（save_range）
pub fn range_system(s: &System, from: usize, to: usize) -> lensbench_core::error::Result<System> {
    let img = s.image();
    if from == 0 || from > to || to >= img {
        return Err(lensbench_core::error::Error::Invalid(format!(
            "surfaces to write {from} to {to} (1 to {})",
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

/// `catalog_add` の params: 読み込んだプロジェクト（状態を当てて）のブロック `blocks`（id の並び。1 つか範囲の
/// 最初と最後）をユーザーカタログ `catalog` の項目 `name` にする。面の表の変更は焼き込む。ファイルが無ければ作る
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogAddReq {
    pub catalog: String,
    pub name: String,
    pub blocks: Vec<String>,
    pub description: String,
    /// 同じ名前の項目を置き換える
    pub replace: bool,
    pub vendor: Option<String>,
}

/// `catalog_edit` の params: 項目 `name` の名前・説明の変更か削除
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogEditReq {
    pub catalog: String,
    pub name: String,
    pub rename: Option<String>,
    pub description: Option<String>,
    pub delete: bool,
    pub vendor: Option<String>,
}

/// params のユーザーカタログのファイル
pub(super) fn catalog_of(method: &str, p: &Value) -> Result<String, RpcErr> {
    p["catalog"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| RpcErr::params(format!("{method} needs catalog (the user catalog file)")))
}

/// params の項目の名前（前後の空白は除く）
fn item_name(method: &str, p: &Value) -> Result<String, RpcErr> {
    p["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RpcErr::params(format!("{method} needs name (the item name)")))
}

impl CatalogAddReq {
    pub(super) fn parse(p: &Value) -> Result<CatalogAddReq, RpcErr> {
        let catalog = catalog_of("catalog_add", p)?;
        let name = item_name("catalog_add", p)?;
        let blocks: Vec<String> = match (&p["blocks"], &p["block"]) {
            (Value::Array(a), _) => a
                .iter()
                .map(|x| x.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .ok_or_else(|| RpcErr::params("blocks must be a list of ids"))?,
            (_, Value::String(b)) => vec![b.clone()],
            _ => {
                return Err(RpcErr::params(
                    "catalog_add needs blocks (a list of block ids)",
                ))
            }
        };
        Ok(CatalogAddReq {
            catalog,
            name,
            blocks,
            description: p["description"].as_str().unwrap_or_default().to_string(),
            replace: p["replace"].as_bool() == Some(true),
            vendor: p["vendor"].as_str().map(str::to_string),
        })
    }
}

impl CatalogEditReq {
    pub(super) fn parse(p: &Value) -> Result<CatalogEditReq, RpcErr> {
        Ok(CatalogEditReq {
            catalog: catalog_of("catalog_edit", p)?,
            name: item_name("catalog_edit", p)?,
            rename: p["rename"].as_str().map(|s| s.trim().to_string()),
            description: p["description"].as_str().map(str::to_string),
            delete: p["delete"].as_bool() == Some(true),
            vendor: p["vendor"].as_str().map(str::to_string),
        })
    }
}

fn set_vendor(cat: &mut lensbench_project::UserCatalog, vendor: &Option<String>) {
    if let Some(v) = vendor {
        cat.vendor = v.trim().to_string();
    }
}

/// ユーザーカタログの項目の一覧（`catalog_items`）
pub(super) fn catalog_items(sess: &Session, path: &str) -> Result<Value, RpcErr> {
    let file = sess.files.check_path(Path::new(path), false)?;
    let cat = lensbench_project::UserCatalog::load(&file)?;
    Ok(catalog_listing(path, &cat))
}

/// 読み込んだプロジェクトのブロックをユーザーカタログの項目に（`catalog_add`）
pub(super) fn catalog_add(sess: &Session, r: &CatalogAddReq) -> Result<Value, RpcErr> {
    use lensbench_project::UserCatalog;
    let (path, name) = (r.catalog.as_str(), r.name.as_str());
    let file = sess.files.check_path(Path::new(path), true)?;
    let Current { spec, src, log, .. } = sess.current_full()?;
    if !is_project(&spec.file) {
        return Err(RpcErr::app(
            "catalog_add only works when a project file (combination) is loaded",
        ));
    }
    let cat = UserCatalog::load_or_new(&file)?;
    if cat.items.contains_key(name) && !r.replace {
        return Err(RpcErr::app(format!(
            "item {name} already exists (replace it with replace: true)"
        )));
    }
    // 読んだときのプロジェクトから（ファイルを読み直さない）
    let mut pr = src
        .project()
        .ok_or_else(|| {
            RpcErr::app("catalog_add only works when a project file (combination) is loaded")
        })?
        .clone();
    if let Some(st) = &spec.state {
        pr = pr.with_state(st)?;
    }
    let lc = lensbench_core::catalog::LensCatalog::load_registered(&spec.lens_catalog)?;
    let look = |v: &str, q: &str| lc.zmx(v, q);
    let env = lensbench_project::Env {
        step_files: None,
        base: spec
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: spec.glass_dir.clone(),
        catalog: Some(&look),
    };
    let item = lensbench_project::blocks_item(&pr, &env, &r.blocks, &r.description)?;
    let cat = UserCatalog::update(&file, |cat| {
        if cat.items.contains_key(name) && !r.replace {
            return Err(
                format!("item {name} already exists (replace it with replace: true)").into(),
            );
        }
        cat.items.insert(name.to_string(), item);
        set_vendor(cat, &r.vendor);
        Ok(())
    })?;
    let mut out = catalog_listing(path, &cat);
    out["added"] = json!(name);
    // 品はプロジェクトファイルの中身から作る（展開した系への set・edit はブロックへ戻せない）
    if !log.is_empty() {
        out["notes"] = json!([
            "changes after loading (set, edit) are not in the item (items are made from the project file; save and reload first to include them)"
        ]);
    }
    Ok(out)
}

/// ユーザーカタログの項目の名前・説明の変更と削除（`catalog_edit`）
pub(super) fn catalog_edit(sess: &Session, r: &CatalogEditReq) -> Result<Value, RpcErr> {
    let (path, name) = (r.catalog.as_str(), r.name.as_str());
    let file = sess.files.check_path(Path::new(path), true)?;
    let mut cat = lensbench_project::UserCatalog::load(&file)?;
    if !cat.items.contains_key(name) {
        return Err(RpcErr::app(format!("no item {name}")));
    }
    if r.delete {
        cat.items.remove(name);
    } else {
        if let Some(d) = &r.description {
            cat.items
                .get_mut(name)
                .expect("the item exists (looked up above)")
                .description = d.clone();
        }
        if let Some(to) = r.rename.as_deref().map(str::trim) {
            if to.is_empty() {
                return Err(RpcErr::params("empty rename name"));
            }
            if to != name {
                if cat.items.contains_key(to) {
                    return Err(RpcErr::app(format!("item {to} already exists")));
                }
                let it = cat
                    .items
                    .remove(name)
                    .expect("the item exists (looked up above)");
                cat.items.insert(to.to_string(), it);
            }
        }
    }
    set_vendor(&mut cat, &r.vendor);
    cat.save(&file)?;
    Ok(catalog_listing(path, &cat))
}

/// `cad_step` の params: ユーザーカタログ `catalog` の品 `item` の STEP を取り、`cache`（既定は決まった場所）に置く
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CadStepReq {
    pub catalog: String,
    pub item: String,
    pub cache: Option<String>,
}

impl CadStepReq {
    pub(super) fn parse(p: &Value) -> Result<CadStepReq, RpcErr> {
        Ok(CadStepReq {
            catalog: need_str("cad_step", p, "catalog")?,
            item: need_str("cad_step", p, "item")?,
            cache: p["cache"]
                .as_str()
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string),
        })
    }
}

/// params の k の文字列（前後の空白を除いて空でないこと）
fn need_str(method: &str, p: &Value, k: &str) -> Result<String, RpcErr> {
    p[k].as_str()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            // 訳の表が引けるように文は method ごとに書く
            RpcErr::params(match method {
                "cad_step" => format!("cad_step needs {k}"),
                _ => format!("catalog_fill needs {k}"),
            })
        })
}

/// `cad_step` → {item, url, path, cached, measured, notes}。取れなければ誤りで、品ページの URL を添える
pub(super) fn cad_step(sess: &Session, r: &CadStepReq) -> Result<Value, RpcErr> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (sess, r);
        Err(RpcErr::app(
            "getting STEP files is not available in the web version",
        ))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let src = sess.files.check_path(Path::new(&r.catalog), false)?;
        let name = r.item.as_str();
        let cat = lensbench_project::UserCatalog::load(&src)?;
        let it = cat
            .items
            .get(name)
            .ok_or_else(|| RpcErr::app(format!("no item {name} in {}", src.display())))?;
        let dir = match &r.cache {
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
                    RpcErr::app(format!("{e} (product page: {})", it.url))
                }
            })
    }
}

/// `catalog_fill` の params: カタログ `catalog` の未完の品 `item` に `thickness`・`diameter`（数か [幅, 高さ]、mm）を
/// 足して、補足のカタログ `out`（ユーザーカタログ。無ければ作る）に同じ型番・同じメーカーの普通の品として書く。
/// `from` は値の決め方（page・step・manual）。枠付きの品は `frame`（機構部品の寸法、z は枠の端から）で枠も
/// 書ける（`frame_reversed` なら光は枠の z の大きい端から入る）。`spec` は仕様表の値（食い違う寸法は notes に）、
/// `glass_visible` は枠の穴からガラスが見えるか。元のカタログは書き換えない。`out` をレンズカタログのフォルダに
/// 置けば、次の一覧では未完の品の代わりに並ぶ
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogFillReq {
    pub catalog: String,
    pub out: String,
    pub item: String,
    pub thickness: Option<f64>,
    /// 数か [幅, 高さ]
    pub diameter: Option<Value>,
    pub frame: Option<lensbench_project::MechSpec>,
    pub from: String,
    pub frame_reversed: bool,
    pub spec: Option<Value>,
    pub glass_visible: bool,
}

impl CatalogFillReq {
    pub(super) fn parse(p: &Value) -> Result<CatalogFillReq, RpcErr> {
        let catalog = need_str("catalog_fill", p, "catalog")?;
        let out = need_str("catalog_fill", p, "out")?;
        let item = need_str("catalog_fill", p, "item")?;
        let frame = match p.get("frame").filter(|v| !v.is_null()) {
            Some(v) => Some(
                serde_json::from_value::<lensbench_project::MechSpec>(v.clone())
                    .map_err(|e| RpcErr::params(format!("frame: {e}")))?,
            ),
            None => None,
        };
        Ok(CatalogFillReq {
            catalog,
            out,
            item,
            thickness: p["thickness"].as_f64(),
            diameter: p.get("diameter").filter(|v| !v.is_null()).cloned(),
            frame,
            from: p["from"].as_str().unwrap_or("").to_string(),
            frame_reversed: p["frame_reversed"].as_bool().unwrap_or(false),
            spec: p.get("spec").filter(|v| v.is_object()).cloned(),
            glass_visible: p["glass_visible"].as_bool().unwrap_or(false),
        })
    }
}

/// `catalog_fill` → {catalog, item, vendor, specs, notes, frame, frame_reversed}
pub(super) fn catalog_fill(sess: &Session, r: &CatalogFillReq) -> Result<Value, RpcErr> {
    use lensbench_catalog::fill as cf;
    use lensbench_project::UserCatalog;
    let src = sess.files.check_path(Path::new(&r.catalog), false)?;
    let out = sess.files.check_path(Path::new(&r.out), true)?;
    let name = r.item.as_str();
    let cat = UserCatalog::load(&src)?;
    let it = cat
        .items
        .get(name)
        .ok_or_else(|| RpcErr::app(format!("no item {name} in {}", src.display())))?;
    if !it.is_incomplete() {
        return Err(RpcErr::app(format!("{name} is not an incomplete item")));
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
    let mut filled = cf::complete(
        it,
        r.thickness,
        r.diameter.clone(),
        r.frame.clone(),
        cf::FillFrom::parse(&r.from),
        &today,
    )
    .map_err(RpcErr::params)?;
    if let lensbench_project::ItemBody::Optic { frame: Some(f), .. } = &mut filled.body {
        f.reversed = r.frame_reversed;
    }
    // 仕様表の値と食い違う寸法は notes に残す
    if let Some(sv) = &r.spec {
        let spec = cf::SpecValues::from_value(sv);
        let num = |k: &str| filled.specs.get(k).and_then(Value::as_f64);
        let diffs = cf::spec_check(
            num("thickness"),
            num("diameter"),
            filled.frame().map(|f| &f.spec),
            r.glass_visible,
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
            "{} is a supplementary catalog of maker {} (put {vendor} items in another file)",
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
        "frame": filled.frame().map(|f| &f.spec),
        "frame_reversed": filled.frame().is_some_and(|f| f.reversed),
    }))
}
