//! カタログを探すための一覧（ガラス・カタログレンズ）。serve の `glasses`・`lenses` と GUI のカタログブラウザが使う。

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use lensbench_core::elements::{self, Kind};
use lensbench_core::{glass, vfs, zmx};
use serde_json::{json, Value};

/// d・F・C 線 [µm]
const WL_D: f64 = 0.5875618;
const WL_F: f64 = 0.4861327;
const WL_C: f64 = 0.6562725;

/// 有限でなければ null（JSON に NaN は書けない）
fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

/// フォルダ（1 階層）の AGF を全部読み、ガラスの一覧を返す。nd・vd はカタログ温度の分散式から求める。
/// 同じフォルダが重ねて与えられても 1 回だけ読む。
pub fn glasses(dirs: &[PathBuf]) -> Value {
    let mut files: Vec<PathBuf> = Vec::new();
    for d in glass::catalog_dirs(dirs) {
        let Ok(rd) = vfs::read_dir(&d) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd
            .into_iter()
            .filter(|p| {
                vfs::is_file(p) && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("agf"))
            })
            .collect();
        v.sort();
        for p in v {
            let real = vfs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if !files
                .iter()
                .any(|q| vfs::canonicalize(q).ok().as_deref() == Some(&real))
            {
                files.push(p);
            }
        }
    }
    let mut catalogs = Vec::new();
    let mut out = Vec::new();
    for f in &files {
        let Ok(gs) = glass::load_agf(f) else {
            continue;
        };
        let mut names: Vec<&String> = gs.keys().collect();
        names.sort();
        for n in &names {
            let g = &gs[*n];
            let nd = g.n_cat(WL_D);
            let vd = (nd - 1.0) / (g.n_cat(WL_F) - g.n_cat(WL_C));
            out.push(json!({
                "name": g.name,
                "catalog": g.catalog,
                "nd": num(nd),
                "vd": num(vd),
                "formula": g.formula,
                "range_um": g.ld.map(|x| vec![x[0], x[1]]),
            }));
        }
        catalogs.push(json!({
            "name": f.file_stem().map(|s| s.to_string_lossy().into_owned()),
            "path": f.display().to_string(),
            "count": names.len(),
        }));
    }
    json!({ "catalogs": catalogs, "glasses": out })
}

/// ガラスカタログの読み込みの使い回し（GCAT の並び → ガラス）。玉ごとに AGF を読み直すと、
/// 大きなカタログ（数 MB）では 1 本ごとに数十 ms かかる
type GlassCache =
    std::collections::HashMap<Vec<String>, std::collections::HashMap<String, glass::Glass>>;

/// レンズカタログ 1 本の要約（焦点距離・径・厚さ・ガラス）。読めなければ error に理由
fn lens_summary(
    zmx_text: &str,
    efl_cat: f64,
    dirs: &[PathBuf],
    cache: &mut GlassCache,
) -> Result<Value, String> {
    let raw = zmx::parse_text(zmx_text).map_err(|e| e.to_string())?;
    let cats: Vec<String> = raw
        .sysl
        .get("GCAT")
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_else(|| vec!["SCHOTT".into()]);
    let all = cache
        .entry(cats.clone())
        .or_insert_with(|| glass::load_catalogs(&cats, &glass::catalog_dirs(dirs)).0);
    // 使うガラスだけを渡す（全部を写すと、それも玉ごとに重い）
    let mut names: Vec<String> = raw.surfs.iter().filter_map(|q| q.glas.clone()).collect();
    names.extend(
        raw.mce
            .iter()
            .filter(|m| m.0.eq_ignore_ascii_case("GLSS"))
            .map(|m| m.3.clone()),
    );
    let mut sub = std::collections::HashMap::new();
    for n in names {
        let n = n.trim();
        if n.is_empty() || n.eq_ignore_ascii_case("MIRROR") || sub.contains_key(n) {
            continue;
        }
        // モデルガラスなど見つからない名前は飛ばす（無いガラスは全部を渡しても無いので、読み込みが同じ誤りにする）
        let found = all.get(n).map(|g| (n.to_string(), g)).or_else(|| {
            all.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(k, g)| (k.clone(), g))
        });
        if let Some((k, g)) = found {
            sub.insert(k, g.clone());
        }
    }
    let s = zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            glasses: Some(sub),
            // カタログの玉の径はファイルの値
            fixed_semi: true,
            ..Default::default()
        },
        &glass::catalog_dirs(dirs),
        None,
    )
    .map_err(|e| e.to_string())?;
    system_summary(&s, efl_cat)
}

/// 系の最初の玉（玉が無ければ鏡）の要約
fn system_summary(s: &lensbench_core::system::System, efl_cat: f64) -> Result<Value, String> {
    let els = elements::split(s).map_err(|e| e.to_string())?;
    let lenses: Vec<_> = els.iter().filter(|e| e.kind == Kind::Lens).collect();
    // 玉の無いカタログの鏡: 鏡の面の曲率半径・コーニック・径
    if lenses.is_empty() {
        if let Some(m) = els.iter().find(|e| e.kind == Kind::Mirror) {
            let q = m.surfaces.iter().find(|q| q.mirror).ok_or("鏡の面がない")?;
            let r = if q.c == 0.0 { 0.0 } else { 1.0 / q.c };
            return Ok(json!({
                "efl": num(if efl_cat.is_finite() {
                    efl_cat
                } else if q.c == 0.0 {
                    f64::INFINITY
                } else {
                    -r / 2.0
                }),
                "diameter": num(2.0 * q.semi),
                "thickness": 0.0,
                "elements": 0,
                "pieces": 0,
                "surfaces": m.surfaces.len(),
                "glasses": [],
                "mirror": true,
                "kind": "mirror",
                "radius": num(r),
                "conic": num(q.k),
            }));
        }
    }
    let e = lenses.first().ok_or("玉がない")?;
    let efl = if efl_cat.is_finite() {
        efl_cat
    } else {
        e.efl(s.primary)
    };
    let semi = e.surfaces.iter().map(|q| q.semi).fold(0.0, f64::max);
    let mut mats: Vec<String> = Vec::new();
    for m in e.materials() {
        if !mats.iter().any(|x| x == m) {
            mats.push(m.to_string());
        }
    }
    Ok(json!({
        "efl": num(efl),
        "diameter": num(2.0 * semi),
        "thickness": num(e.length()),
        "elements": lenses.len(),
        // ガラスの枚数（貼り合わせは 1 つの玉に 2 枚以上）
        "pieces": lenses.iter().map(|l| l.surfaces.len().saturating_sub(1)).sum::<usize>(),
        "surfaces": e.surfaces.len(),
        "glasses": mats,
        "mirror": e.surfaces.iter().any(|q| q.mirror),
        "kind": "lens",
        // 最初の玉の前・後ろの面の曲率 [1/mm]（GUI の断面で無限共役の側を見る）
        "curv": [
            num(e.surfaces.first().map_or(0.0, |q| q.c)),
            num(e.surfaces.last().map_or(0.0, |q| q.c)),
        ],
    }))
}

/// 前もって作った一覧（[`set_prepared`]）: レンズカタログ・ガラスのフォルダの並び → 一覧の JSON のファイル
type Prepared = Vec<(Vec<PathBuf>, Vec<PathBuf>, PathBuf)>;
static PREPARED: std::sync::Mutex<Prepared> = std::sync::Mutex::new(Vec::new());

/// [`lenses`] がこのフォルダの並び（レンズカタログ・ガラス、順も同じ）で呼ばれたら、読む代わりに file の一覧
/// （[`lenses`] の結果の JSON。`.gz` も読める）を返す。Web 版で、同梱のメーカーのカタログの一覧をビルドのときに
/// 作っておく（数万品を開くたびに読むと数秒かかる）。file が読めなければ、いつもどおり読む
pub fn set_prepared(paths: Vec<PathBuf>, glass_dirs: Vec<PathBuf>, file: PathBuf) {
    let mut p = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    p.retain(|x| x.0 != paths || x.1 != glass_dirs);
    p.push((paths, glass_dirs, file));
}

fn prepared(paths: &[PathBuf], glass_dirs: &[PathBuf]) -> Option<Value> {
    let file = PREPARED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|x| x.0 == paths && x.1 == glass_dirs)
        .map(|x| x.2.clone())?;
    let v: Value = serde_json::from_str(&lensbench_project::read_text(&file).ok()?).ok()?;
    v["lenses"].is_array().then_some(v)
}

/// レンズカタログ（.zmx のファイル・フォルダ）を読み、レンズの一覧を返す。
/// 焦点距離はカタログの値（無ければ主波長の近軸の値）、径は面の半径の最大の 2 倍、厚さは中心厚の合計。
/// 前もって作った一覧があればそれ（[`set_prepared`]）
pub fn lenses(paths: &[PathBuf], glass_dirs: &[PathBuf]) -> Result<Value, String> {
    if let Some(v) = prepared(paths, glass_dirs) {
        return Ok(v);
    }
    let dir = CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match dir {
        Some(d) => lenses_cached(paths, glass_dirs, &d),
        None => read_lenses(paths, glass_dirs),
    }
}

fn lenses_cached(
    paths: &[PathBuf],
    glass_dirs: &[PathBuf],
    dir: &std::path::Path,
) -> Result<Value, String> {
    // 覚えた一覧: 同じフォルダの並びで、中のファイル（とこのプログラム）の更新時刻・大きさが同じで、
    // 一覧を作るときに読んだそのほかのファイル（ユーザーカタログの品が参照するフォルダの外の .zmx・
    // カタログ・ガラスなど、公開 #67）も変わっていなければそれを使う
    let (key, covered) = cache_key(paths, glass_dirs);
    let file = dir.join(CACHE_FILE);
    let deps_same = |v: &Value| {
        v["deps"].as_array().is_some_and(|a| {
            a.iter().all(|d| {
                d[0].as_str()
                    .is_some_and(|p| d[1].as_str() == Some(&stamp_text(std::path::Path::new(p))))
            })
        })
    };
    if let Some(v) = lensbench_project::read_text(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v["key"].as_str() == Some(&key) && v["lenses"].is_array() && deps_same(v))
    {
        return Ok(json!({ "lenses": v["lenses"] }));
    }
    let (v, read) = vfs::recording(|| read_lenses(paths, glass_dirs));
    let v = v?;
    let deps: Vec<Value> = read
        .iter()
        .filter(|p| !covered.contains(*p))
        .map(|p| json!([p.display().to_string(), stamp_text(p)]))
        .collect();
    // 書けなくても一覧は返す（次も読み直すだけ）
    let _ = vfs::create_dir_all(dir).and_then(|_| {
        vfs::write(
            &file,
            json!({ "key": key, "deps": deps, "lenses": v["lenses"] }).to_string(),
        )
    });
    Ok(v)
}

/// ファイル・フォルダの更新時刻 [ns]・大きさ（無ければ None）の文字列
fn stamp_text(p: &std::path::Path) -> String {
    let st = vfs::stamp(p).ok();
    let t = st
        .as_ref()
        .and_then(|x| x.0)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos());
    format!("{:?}\t{:?}", t, st.map(|x| x.1))
}

/// 一覧を覚えておくファイルの名前（[`set_cache_dir`] のフォルダの中）
pub const CACHE_FILE: &str = "lens-list-cache.json";
static CACHE_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// [`lenses`] の一覧をこのフォルダに覚えておき、次に同じフォルダの並びで呼ばれ、中のファイルの更新時刻・
/// 大きさが変わっていなければ読み直さない（GUI の設定のフォルダ。数万品のカタログを読むと数秒かかる）
pub fn set_cache_dir(dir: Option<PathBuf>) {
    *CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// 一覧の元になるもの（フォルダの並び、中のファイル 2 階層まで、ガラスのファイル、このプログラム）の更新時刻・
/// 大きさから作る鍵と、鍵に入れたファイル・フォルダ（一覧を作るときに読んだものの中で、別に調べなくてよいもの）
fn cache_key(
    paths: &[PathBuf],
    glass_dirs: &[PathBuf],
) -> (String, std::collections::BTreeSet<PathBuf>) {
    use std::fmt::Write;
    let mut s = String::new();
    let mut covered = std::collections::BTreeSet::new();
    let mut stamp = |p: &std::path::Path, s: &mut String| {
        let _ = writeln!(s, "{}\t{}", p.display(), stamp_text(p));
        covered.insert(p.to_path_buf());
    };
    // フォルダもたどったもの（中の並びは鍵に入る）
    fn files(p: &std::path::Path, depth: u32, out: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
        if vfs::is_dir(p) {
            if depth == 0 {
                return;
            }
            dirs.push(p.to_path_buf());
            let mut v = vfs::read_dir(p).unwrap_or_default();
            v.sort();
            for q in v {
                files(&q, depth - 1, out, dirs);
            }
        } else {
            out.push(p.to_path_buf());
        }
    }
    let mut dirs = Vec::new();
    let _ = writeln!(s, "{}", env!("CARGO_PKG_VERSION"));
    if let Ok(exe) = std::env::current_exe() {
        stamp(&exe, &mut s);
    }
    s.push_str("lens\n");
    for p in paths {
        let mut v = Vec::new();
        let _ = writeln!(s, "{}", p.display());
        files(p, 2, &mut v, &mut dirs);
        for f in v {
            stamp(&f, &mut s);
        }
    }
    s.push_str("glass\n");
    for d in glass_dirs {
        let _ = writeln!(s, "{}", d.display());
    }
    for d in glass::catalog_dirs(glass_dirs) {
        let mut v = Vec::new();
        files(&d, 1, &mut v, &mut dirs);
        for f in v {
            stamp(&f, &mut s);
        }
    }
    covered.extend(dirs);
    // 長い文をそのまま残すと大きいので、ハッシュ（FNV-1a 64 bit、版によらず同じ）と長さに縮める
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (format!("{h:016x}-{}", s.len()), covered)
}

fn read_lenses(paths: &[PathBuf], glass_dirs: &[PathBuf]) -> Result<Value, String> {
    let cat = LensCatalog::load_registered(paths)?;
    let mut cache = GlassCache::new();
    let mut out: Vec<Value> = cat
        .lenses()
        .iter()
        .map(|l| {
            let mut v = json!({ "vendor": l.vendor, "part": l.part });
            match lens_summary(&l.zmx, l.efl, glass_dirs, &mut cache) {
                Ok(Value::Object(m)) => v.as_object_mut().unwrap().extend(m),
                Ok(_) => {}
                Err(e) => v["error"] = json!(e),
            }
            v
        })
        .collect();
    let mut cats = Vec::new();
    for f in json_catalogs(paths) {
        let (rows, cat) = user_items(&f, glass_dirs, &cat);
        out.extend(rows);
        cats.extend(cat);
    }
    // 同じメーカーのカタログの url_template・overlay を全部の行に（ZMF・.zmx の玉にも）
    let refs: Vec<&lensbench_project::UserCatalog> = cats.iter().collect();
    for row in &mut out {
        lensbench_project::catalog_spec::apply_overlays(row, &refs);
        if row.get("category").map_or(true, Value::is_null) {
            use lensbench_project::catalog_spec::{row_category, ItemKind};
            let k = ItemKind::parse(row_category(row)).unwrap_or(ItemKind::Other);
            row["category"] = json!(k.as_str());
        }
    }
    Ok(json!({ "lenses": out }))
}

/// 一覧（[`lenses`]）を絞り込む（CLI catalog-search・serve の catalog_search）
pub fn search(
    paths: &[PathBuf],
    glass_dirs: &[PathBuf],
    q: &lensbench_project::catalog_spec::CatalogQuery,
) -> Result<Value, String> {
    let mut v = lenses(paths, glass_dirs)?;
    if let Some(a) = v["lenses"].as_array_mut() {
        a.retain(|r| q.matches(r));
    }
    Ok(v)
}

/// レンズカタログのフォルダ（1 階層）・ファイルの中の lensbench 形式のカタログ（ユーザーカタログの .json。
/// gzip で圧縮した .json.gz も読む。同じ名前の .json と .json.gz があれば .json だけ）
fn json_catalogs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let is_json = |p: &PathBuf| vfs::is_file(p) && lensbench_project::is_catalog_file(p);
    let mut out = Vec::new();
    for p in paths {
        if vfs::is_dir(p) {
            let Ok(rd) = vfs::read_dir(p) else {
                continue;
            };
            let mut v: Vec<PathBuf> = rd.into_iter().filter(is_json).collect();
            v.sort();
            let plain: Vec<PathBuf> = v.iter().filter(|x| !is_gz(x)).cloned().collect();
            v.retain(|x| !is_gz(x) || !plain.contains(&x.with_extension("")));
            out.extend(v);
        } else if is_json(p) {
            out.push(p.clone());
        }
    }
    out
}

fn is_gz(p: &std::path::Path) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gz"))
}

/// lensbench 形式のカタログの品の一覧。メーカーはカタログの `vendor`（無ければフォルダの名前）、型番は項目の名前。
/// 鏡・BS のブロック 1 つだけの項目は鏡・BS の品、ほかは玉（最初の玉の要約）。
/// `source` に組合せから参照するカタログのファイル（絶対パス）と項目
/// 読めたカタログも返す（overlay を当てるため）
fn user_items(
    path: &PathBuf,
    glass_dirs: &[PathBuf],
    lens_cat: &LensCatalog,
) -> (Vec<Value>, Option<lensbench_project::UserCatalog>) {
    use lensbench_project::{Block, UserCatalog};
    let abs = vfs::canonicalize(path).unwrap_or_else(|_| path.clone());
    let Ok(txt) = lensbench_project::read_text(&abs) else {
        return (Vec::new(), None);
    };
    // ユーザーカタログでない .json（系のファイルなど）は飛ばす。品が無く overlay だけのカタログも読む
    let v: Value = match serde_json::from_str(&txt) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), None),
    };
    if !v.get("items").is_some_and(Value::is_object)
        && !v.get("overlay").is_some_and(Value::is_object)
    {
        return (Vec::new(), None);
    }
    let folder = abs
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cat = match UserCatalog::from_json(&txt) {
        Ok(c) => c,
        Err(e) => {
            return (
                vec![json!({
                    "vendor": folder, "part": abs.file_name().map(|s| s.to_string_lossy().into_owned()),
                    "error": e,
                })],
                None,
            )
        }
    };
    let vendor = if cat.vendor.trim().is_empty() {
        folder
    } else {
        cat.vendor.clone()
    };
    let rows = cat
        .items
        .iter()
        .map(|(name, it)| {
            let mut row = json!({
                "vendor": vendor,
                "part": name,
                "description": it.description,
                "source": { "catalog": abs.display().to_string(), "item": name },
            });
            if let Some(k) = it.kind {
                row["category"] = json!(k.as_str());
            }
            if !it.url.is_empty() {
                row["url"] = json!(it.url);
            }
            if !it.specs.is_empty() {
                row["specs"] = json!(it.specs);
            }
            if !it.tags.is_empty() {
                row["tags"] = json!(it.tags);
            }
            if !it.source.is_empty() {
                row["spec_source"] = json!(it.source);
            }
            if !it.checked.is_empty() {
                row["checked"] = json!(it.checked);
            }
            let summary = match (it.mech.as_ref(), it.blocks.as_slice()) {
                // 機構部品: 面が無く寸法だけ
                (Some(m), []) => {
                    let id = m.bores.iter().map(|b| b.diameter).fold(f64::INFINITY, f64::min);
                    let len = if m.length > 0.0 {
                        m.length
                    } else {
                        m.bores.iter().map(|b| b.z1).fold(0.0, f64::max)
                            - m.bores.iter().map(|b| b.z0).fold(f64::INFINITY, f64::min).min(0.0)
                    };
                    Ok(json!({
                        "kind": "mech", "mirror": false, "efl": Value::Null,
                        "diameter": num(if m.outer_diameter > 0.0 { m.outer_diameter } else { id }),
                        "thickness": num(len),
                        "elements": 0, "pieces": 0, "glasses": [],
                        "mech": m,
                        "inner_diameter": num(id),
                    }))
                }
                (_, [Block::Mirror {
                    radius,
                    conic,
                    semi,
                    coat,
                    ..
                }]) => {
                    let r = radius.unwrap_or(0.0);
                    Ok(json!({
                        "kind": "mirror", "mirror": true,
                        "efl": num(if r == 0.0 { f64::INFINITY } else { -r / 2.0 }),
                        "diameter": num(2.0 * semi), "thickness": 0.0,
                        "elements": 0, "pieces": 0, "glasses": [],
                        "radius": r, "conic": conic, "coat": coat,
                    }))
                }
                (_, [Block::Splitter { spec, .. }]) => {
                    use lensbench_project::SplitForm;
                    let (d, t, pieces) = match spec.form {
                        SplitForm::Cube => (spec.size, spec.size, 2),
                        SplitForm::Plate => (2.0 * spec.semi, spec.thickness, 1),
                        SplitForm::Ideal => (2.0 * spec.semi, 0.0, 0),
                    };
                    let mut sp = serde_json::to_value(spec).unwrap_or_default();
                    for k in ["t", "r", "d", "arm", "angle"] {
                        sp.as_object_mut().map(|m| m.remove(k));
                    }
                    Ok(json!({
                        "kind": "splitter", "mirror": false,
                        "efl": Value::Null, "diameter": num(d), "thickness": num(t),
                        "elements": 0, "pieces": pieces,
                        "glasses": if spec.glass.is_empty() { vec![] } else { vec![spec.glass.clone()] },
                        "coat": spec.coat, "splitter": sp,
                    }))
                }
                // 空気中の面の並びだけ（Jones 面の偏光子・波長板など）: 玉が無いので面から。ガラスのある面の
                // 並びは玉の要約へ回し、そこで「面の並びは空気中の面だけ」のエラーにする
                (_, [Block::Surfaces { surfaces, .. }])
                    if surfaces.iter().all(|q| q.glass.is_none()) =>
                {
                    let semi = surfaces.iter().map(|q| q.semi.value()).fold(0.0, f64::max);
                    Ok(json!({
                        "kind": "surfaces", "mirror": false, "efl": Value::Null,
                        "diameter": num(2.0 * semi),
                        "thickness": surfaces.iter().map(|q| q.t).sum::<f64>(),
                        "elements": 0, "pieces": 0, "surfaces": surfaces.len(), "glasses": [],
                        "jones": surfaces.iter().any(|q| q.jones.is_some() || q.polarizer.is_some() || q.waveplate.is_some()),
                    }))
                }
                _ => user_lens_summary(&abs, name, &it.glass_catalogs, glass_dirs, lens_cat),
            };
            match summary {
                Ok(Value::Object(m)) => row.as_object_mut().unwrap().extend(m),
                Ok(_) => {}
                Err(e) => row["error"] = json!(e),
            }
            row
        })
        .collect();
    (rows, Some(cat))
}

/// lensbench 形式のカタログの玉の要約（項目を 1 つ置いた系を作って見る）。品がレンズカタログの玉
/// （`source: {vendor, part}`）を参照していれば、一覧のレンズカタログ（ZMF・.zmx）から引く
fn user_lens_summary(
    catalog: &std::path::Path,
    item: &str,
    glass_catalogs: &[String],
    glass_dirs: &[PathBuf],
    lens_cat: &LensCatalog,
) -> Result<Value, String> {
    let p = lensbench_project::Project::from_json(
        &json!({
            "glass_catalogs": glass_catalogs,
            "wavelengths": [{ "um": WL_D }],
            "aperture": { "type": "epd", "value": 1.0 },
            "fields": { "points": [[0.0, 0.0]] },
            "blocks": [
                { "type": "stop" },
                { "type": "air", "t": 1.0 },
                { "type": "user", "catalog": catalog.display().to_string(), "item": item },
                { "type": "air", "t": 10.0 },
            ],
        })
        .to_string(),
    )?;
    let env = lensbench_project::Env {
        base: catalog
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: glass_dirs.to_vec(),
        catalog: Some(&|v: &str, q: &str| lens_cat.zmx(v, q)),
    };
    let s = lensbench_project::expand(&p, &env)?;
    system_summary(&s, f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn glass_list_has_nd_vd() {
        let v = glasses(&[root().join("tests/glass")]);
        let g = v["glasses"].as_array().unwrap();
        let bk7 = g.iter().find(|x| x["name"] == "N-BK7").expect("N-BK7");
        assert_eq!(bk7["catalog"], "SCHOTT");
        assert!((bk7["nd"].as_f64().unwrap() - 1.5168).abs() < 1e-4);
        assert!((bk7["vd"].as_f64().unwrap() - 64.17).abs() < 0.05);
        // 同じフォルダを 2 回与えても重ねない
        let w = glasses(&[root().join("tests/glass"), root().join("tests/glass")]);
        assert_eq!(w["glasses"].as_array().unwrap().len(), g.len());
    }

    #[test]
    fn lens_list_has_efl() {
        let v = lenses(
            &[root().join("examples/lenses/LB")],
            &[root().join("tests/glass")],
        )
        .unwrap();
        let l = v["lenses"].as_array().unwrap();
        assert_eq!(l.len(), 5);
        let p = l.iter().find(|x| x["part"] == "PCX-100").unwrap();
        assert_eq!(p["vendor"], "LB");
        assert!(p.get("error").is_none(), "{p}");
        assert!(p["efl"].as_f64().unwrap() > 0.0);
        assert!(p["diameter"].as_f64().unwrap() > 0.0);
        let a = l.iter().find(|x| x["part"] == "ACH-45").unwrap();
        assert_eq!(a["glasses"], json!(["N-SK16", "F2"]));
        // 貼り合わせは 2 枚、単レンズは 1 枚
        assert_eq!(a["pieces"], 2);
        assert_eq!(p["pieces"], 1);
    }

    /// 覚えた一覧: 2 回目は覚えたものを返し、フォルダの中のファイルが変わったら読み直す
    #[test]
    fn lens_list_cache_follows_files() {
        let tmp = std::env::temp_dir().join(format!("lensbench-lcache-{}", std::process::id()));
        let d = tmp.join("LB");
        let cache = tmp.join("cache");
        vfs::create_dir_all(&d).unwrap();
        for f in ["PCX-100.zmx", "ACH-45.zmx"] {
            std::fs::copy(root().join("examples/lenses/LB").join(f), d.join(f)).unwrap();
        }
        let g = [root().join("tests/glass")];
        let a = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        assert_eq!(a["lenses"].as_array().unwrap().len(), 2);
        assert!(vfs::is_file(cache.join(CACHE_FILE)));
        // 覚えたものを返す（目印を書き込んで確かめる）
        let mut c: Value =
            serde_json::from_str(&vfs::read_to_string(cache.join(CACHE_FILE)).unwrap()).unwrap();
        c["lenses"][0]["part"] = json!("CACHED");
        vfs::write(cache.join(CACHE_FILE), c.to_string()).unwrap();
        let b = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        assert_eq!(b["lenses"][0]["part"], "CACHED");
        // 別のフォルダの並びでは使わない
        let e = lenses_cached(&[d.clone(), d.clone()], &g, &cache).unwrap();
        assert_ne!(e["lenses"][0]["part"], "CACHED");
        // ファイルを足すと読み直す
        std::fs::copy(
            root().join("examples/lenses/LB/PCX-100.zmx"),
            d.join("PCX-101.zmx"),
        )
        .unwrap();
        let f = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
        assert_eq!(f["lenses"].as_array().unwrap().len(), 3);
        assert!(f["lenses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["part"] != "CACHED"));
    }

    /// 覚えた一覧: ユーザーカタログの品が見ているフォルダの外の .zmx を参照していても、その .zmx が変われば
    /// 読み直す（公開 #67）。変わらなければ覚えたものを返す
    #[test]
    fn lens_list_cache_follows_files_outside_the_folders() {
        let tmp = std::env::temp_dir().join(format!("lensbench-lcache-out-{}", std::process::id()));
        let (d, out, cache) = (tmp.join("LB2"), tmp.join("outside"), tmp.join("cache"));
        vfs::create_dir_all(&d).unwrap();
        vfs::create_dir_all(&out).unwrap();
        let lb = root().join("examples/lenses/LB");
        std::fs::copy(lb.join("PCX-100.zmx"), out.join("lens.zmx")).unwrap();
        vfs::write(
            d.join("mine.json"),
            json!({"version": 1, "vendor": "ME", "items": {"Z1": {
                "glass_catalogs": ["SCHOTT"],
                "blocks": [{"type": "zmx", "id": "Z", "path": "../outside/lens.zmx", "stop": false}]}}})
            .to_string(),
        )
        .unwrap();
        let g = [root().join("tests/glass")];
        let efl = |v: &Value| v["lenses"][0]["efl"].as_f64().unwrap();
        let a = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        assert!(a["lenses"][0].get("error").is_none(), "{a}");
        let e0 = efl(&a);
        // 参照先が変わらなければ覚えたものを返す（目印で確かめる）
        let mut c: Value =
            serde_json::from_str(&vfs::read_to_string(cache.join(CACHE_FILE)).unwrap()).unwrap();
        assert!(c["deps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x[0].as_str().unwrap().ends_with("lens.zmx")));
        c["lenses"][0]["part"] = json!("CACHED");
        vfs::write(cache.join(CACHE_FILE), c.to_string()).unwrap();
        let b = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        assert_eq!(b["lenses"][0]["part"], "CACHED");
        // フォルダの外の .zmx を別の玉に替えると読み直す
        std::fs::copy(lb.join("ACH-45.zmx"), out.join("lens.zmx")).unwrap();
        let f = lenses_cached(std::slice::from_ref(&d), &g, &cache).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
        assert_eq!(f["lenses"][0]["part"], "Z1");
        assert!((efl(&f) - e0).abs() > 1.0, "{e0} {f}");
    }

    /// 前もって作った一覧: 同じフォルダの並びのときだけ使う。読めなければいつもどおり読む
    #[test]
    fn prepared_lens_list() {
        let tmp = std::env::temp_dir().join(format!("lensbench-prep-{}", std::process::id()));
        vfs::create_dir_all(&tmp).unwrap();
        let lb = tmp.join("only-here-LB");
        vfs::create_dir_all(&lb).unwrap();
        std::fs::copy(
            root().join("examples/lenses/LB/PCX-100.zmx"),
            lb.join("PCX-100.zmx"),
        )
        .unwrap();
        let g = vec![root().join("tests/glass")];
        let idx = tmp.join("index.json");
        set_prepared(vec![lb.clone()], g.clone(), idx.clone());
        // 一覧のファイルがまだ無い: 読む
        assert_eq!(
            lenses(std::slice::from_ref(&lb), &g).unwrap()["lenses"][0]["part"],
            "PCX-100"
        );
        vfs::write(&idx, r#"{"lenses":[{"vendor":"X","part":"PREP"}]}"#).unwrap();
        assert_eq!(
            lenses(std::slice::from_ref(&lb), &g).unwrap()["lenses"][0]["part"],
            "PREP"
        );
        // ガラスの並びが違えば使わない
        let w = lenses(std::slice::from_ref(&lb), &[]).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
        assert_eq!(w["lenses"][0]["part"], "PCX-100");
    }

    /// 玉の無いカタログの鏡は、鏡の面の曲率半径・径を返す（エラーにしない）
    #[test]
    fn catalog_mirror_is_listed() {
        let dir = std::env::temp_dir().join(format!("lensbench-mirror-{}", std::process::id()));
        let d = dir.join("MIR");
        lensbench_core::vfs::create_dir_all(&d).unwrap();
        let src =
            lensbench_core::vfs::read_to_string(root().join("examples/lenses/LB/PCX-100.zmx"))
                .or_else(|_| {
                    let b =
                        lensbench_core::vfs::read(root().join("examples/lenses/LB/PCX-100.zmx"))?;
                    let u: Vec<u16> = b[2..]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    Ok::<_, std::io::Error>(String::from_utf16_lossy(&u))
                })
                .unwrap();
        // PCX-100 の 1 面目を凹面鏡（R = −200）にし、2 面目を消す
        let head = &src[..src.find("SURF 1").unwrap()];
        let text = format!(
            "{head}SURF 1\n  TYPE STANDARD\n  STOP\n  CURV -0.005\n  DISZ -100\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n  DIAM 12.7 0 0 0 1 \"\"\nSURF 2\n  TYPE STANDARD\n  CURV 0\n  DISZ 0\n  DIAM 0 0 0 0 1 \"\"\n"
        );
        lensbench_core::vfs::write(d.join("CM-200.zmx"), text).unwrap();
        let v = lenses(std::slice::from_ref(&d), &[root().join("tests/glass")]).unwrap();
        let m = &v["lenses"][0];
        std::fs::remove_dir_all(&dir).ok();
        assert!(m.get("error").is_none(), "{m}");
        assert_eq!(m["mirror"], true);
        assert_eq!(m["pieces"], 0);
        assert!((m["radius"].as_f64().unwrap() + 200.0).abs() < 1e-9, "{m}");
        assert!((m["efl"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{m}");
        assert!((m["diameter"].as_f64().unwrap() - 25.4).abs() < 1e-9, "{m}");
    }
    /// lensbench 形式のカタログ（ユーザーカタログの .json）: BS・鏡・玉
    #[test]
    fn json_catalog_parts() {
        let v = lenses(
            &[root().join("examples/lenses/LBP")],
            &[root().join("tests/glass")],
        )
        .unwrap();
        let l = v["lenses"].as_array().unwrap();
        let get = |p: &str| {
            l.iter()
                .find(|x| x["part"] == p)
                .unwrap_or_else(|| panic!("{p}"))
        };
        let bs = get("BS-127");
        assert_eq!(bs["vendor"], "LB");
        assert_eq!(bs["kind"], "splitter");
        assert_eq!(bs["diameter"], 12.7);
        assert_eq!(bs["pieces"], 2);
        assert_eq!(bs["splitter"]["form"], "cube");
        assert!(bs["splitter"].get("arm").is_none());
        assert!(bs["source"]["catalog"]
            .as_str()
            .unwrap()
            .ends_with("parts.json"));
        assert_eq!(get("BSP-254")["pieces"], 1);
        let pm = get("PM-254");
        assert_eq!(pm["kind"], "mirror");
        assert_eq!(pm["radius"], 0.0);
        assert!(pm["efl"].is_null());
        assert_eq!(get("CM-254-100")["efl"], 100.0);
        let sl = get("SL-254-50");
        assert_eq!(sl["kind"], "lens");
        assert!(sl.get("error").is_none(), "{sl}");
        assert!((sl["efl"].as_f64().unwrap() - 50.0).abs() < 2.0, "{sl}");
        assert_eq!(sl["glasses"], json!(["N-BK7"]));
        // 種類・仕様・リンク
        assert_eq!(bs["category"], "splitter");
        assert_eq!(bs["url"], "https://example.com/lb/bs-127");
        assert_eq!(bs["specs"]["split_ratio"], "50:50");
        assert_eq!(bs["spec_source"], "架空の例");
        assert_eq!(bs["checked"], "2026-11-09");
        // url の無い品はカタログの url_template から
        assert_eq!(pm["url"], "https://example.com/lb/PM-254");
        assert_eq!(sl["category"], "lens");
        let w = get("WIN-254-3");
        assert_eq!(w["category"], "window");
        assert_eq!(w["kind"], "lens");
        assert!(w.get("error").is_none(), "{w}");
        // 面の並びだけの品（Jones 面の偏光子）
        let pol = get("POL-254");
        assert_eq!(pol["category"], "polarizer");
        assert_eq!(pol["kind"], "surfaces");
        assert_eq!(pol["jones"], true);
        assert_eq!(pol["diameter"], 25.4);
        assert!(pol.get("error").is_none(), "{pol}");
        // 系のファイルなど、ユーザーカタログでない .json は出さない
        let w = lenses(
            &[root().join("examples/projects")],
            &[root().join("tests/glass")],
        )
        .unwrap();
        assert!(w["lenses"].as_array().unwrap().is_empty());
    }

    /// 同じメーカーの .json の overlay・url_template を .zmx の玉にも当て、絞り込む
    #[test]
    fn overlay_and_search() {
        use lensbench_project::catalog_spec::{CatalogQuery, ItemKind, SpecFilter};
        let paths = [
            root().join("examples/lenses/LB"),
            root().join("examples/lenses/LBP"),
        ];
        let glass = [root().join("tests/glass")];
        let v = lenses(&paths, &glass).unwrap();
        let l = v["lenses"].as_array().unwrap();
        let a = l.iter().find(|x| x["part"] == "ACH-45").unwrap();
        assert_eq!(a["category"], "lens");
        assert_eq!(a["url"], "https://example.com/lb/ACH-45");
        assert_eq!(a["specs"]["coating"], "AR 400-700 nm");
        assert_eq!(a["specs"]["wavelength_nm"], json!([400, 700]));
        assert_eq!(a["tags"], json!(["achromat"]));
        let b = l.iter().find(|x| x["part"] == "BCX-50").unwrap();
        assert!(b.get("specs").is_none());
        let parts = |q: CatalogQuery| -> Vec<String> {
            let v = search(&paths, &glass, &q).unwrap();
            v["lenses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x["part"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            parts(CatalogQuery {
                kinds: vec![ItemKind::Polarizer, ItemKind::Waveplate],
                ..Default::default()
            }),
            ["POL-254", "QWP-254-633"]
        );
        assert_eq!(
            parts(CatalogQuery {
                specs: vec![SpecFilter::parse("extinction_ratio>=1000").unwrap()],
                ..Default::default()
            }),
            ["PBS-127", "POL-254"]
        );
        // 一覧の値（efl）でも比べる
        assert_eq!(
            parts(CatalogQuery {
                kinds: vec![ItemKind::Lens],
                specs: vec![SpecFilter::parse("efl=40..60").unwrap()],
                ..Default::default()
            }),
            ["ACH-45", "BCX-50", "SL-254-50"]
        );
        assert_eq!(
            parts(CatalogQuery {
                text: "achromat".into(),
                ..Default::default()
            }),
            ["ACH-45"]
        );
    }

    /// 膜付きの鏡・BS の膜の名前
    #[test]
    fn json_catalog_coat() {
        let dir = std::env::temp_dir().join(format!("lensbench-coat-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        lensbench_core::vfs::write(
            dir.join("c.json"),
            r#"{"items": {
                "M": {"blocks": [{"type": "mirror", "semi": 5, "coat": "HR"}]},
                "B": {"blocks": [{"type": "splitter", "id": "BS", "size": 10, "glass": "N-BK7", "coat": "BS3", "ar": "AR2"}]}
            }}"#,
        )
        .unwrap();
        let v = lenses(std::slice::from_ref(&dir), &[]).unwrap();
        let l = v["lenses"].as_array().unwrap();
        let m = l.iter().find(|x| x["part"] == "M").unwrap();
        assert_eq!(m["coat"], "HR");
        let b = l.iter().find(|x| x["part"] == "B").unwrap();
        assert_eq!(b["coat"], "BS3");
        assert_eq!(b["splitter"]["ar"], "AR2");
        // メーカーはフォルダの名前
        assert_eq!(
            b["vendor"],
            dir.file_name().unwrap().to_string_lossy().as_ref()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 面の並びのブロック 1 つの品: 空気中の面だけなら面の要約、ガラスのある面ならエラーの行
    #[test]
    fn json_catalog_surfaces_glass_is_error() {
        let dir = std::env::temp_dir().join(format!("lensbench-surfs-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        lensbench_core::vfs::write(
            dir.join("s.json"),
            r#"{"items": {
                "G": {"blocks": [{"type": "surfaces", "surfaces": [{"radius": 51.5, "t": 4, "glass": "N-BK7", "semi": 12.7}, {"semi": 12.7}]}]},
                "A": {"blocks": [{"type": "surfaces", "surfaces": [{"t": 1, "semi": 5}, {"semi": 5}]}]}
            }}"#,
        )
        .unwrap();
        let v = lenses(std::slice::from_ref(&dir), &[root().join("tests/glass")]).unwrap();
        let l = v["lenses"].as_array().unwrap();
        let g = l.iter().find(|x| x["part"] == "G").unwrap();
        assert!(g["error"].is_string(), "{g}");
        let a = l.iter().find(|x| x["part"] == "A").unwrap();
        assert_eq!(a["kind"], "surfaces");
        assert!(a.get("error").is_none(), "{a}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// レンズカタログの玉を参照する品（source: {vendor, part}）は、一覧のレンズカタログから玉を引いて要約する（#90）
    #[test]
    fn json_item_refers_to_catalog_lens() {
        let dir = std::env::temp_dir().join(format!("lensbench-catref-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        lensbench_core::vfs::write(
            dir.join("r.json"),
            r#"{"vendor": "LB", "items": {
                "PCX-100-AR": {"description": "PCX-100 に AR", "blocks": [{"type": "lens", "source": {"vendor": "LB", "part": "PCX-100"}}]}
            }}"#,
        )
        .unwrap();
        let g = [root().join("tests/glass")];
        let v = read_lenses(&[root().join("examples/lenses/LB"), dir.clone()], &g).unwrap();
        let l = v["lenses"].as_array().unwrap();
        let base = l.iter().find(|x| x["part"] == "PCX-100").unwrap();
        let r = l.iter().find(|x| x["part"] == "PCX-100-AR").unwrap();
        assert!(r.get("error").is_none(), "{r}");
        for k in ["diameter", "thickness"] {
            assert_eq!(r[k], base[k], "{k}: {r} {base}");
        }
        let (a, b) = (r["efl"].as_f64().unwrap(), base["efl"].as_f64().unwrap());
        assert!((a - b).abs() < 1e-6 * b.abs(), "efl {a} {b}");
        // レンズカタログのフォルダが無ければ誤り（引けない）
        let v = read_lenses(std::slice::from_ref(&dir), &g).unwrap();
        let r = &v["lenses"].as_array().unwrap()[0];
        assert!(r["error"].is_string(), "{r}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
