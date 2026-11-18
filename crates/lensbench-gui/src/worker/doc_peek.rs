//! 下書きの Worker の `doc_peek`: 開くファイルを組合せにできるか見る（worker.rs から分けた）

use serde_json::Value;

/// 下書きの Worker の `doc_peek` {file}: プロジェクトの .json の `gui`（組合せ・パネルの設定）と、ブロックで
/// 書いたか（`blocks`）。組合せを同梱していないブロックのプロジェクトで、組合せで同じ系を書ければその組合せ（`combo`）
pub(super) fn doc_peek(p: &Value) -> Result<Value, String> {
    let f = p["file"].as_str().ok_or("doc_peek needs file")?;
    let list = |k: &str| -> Vec<std::path::PathBuf> {
        p[k].as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(Into::into)
            .collect()
    };
    // .zmx は組合せで同じ系を書ければ組合せにする（文書モデル 2 段目）。書けなければ今までどおり系として開く
    if !lensbench_project::open::is_json(std::path::Path::new(f)) {
        let mut out = serde_json::json!({ "gui": Value::Null, "blocks": false, "zmx": true });
        peek_file(f, &list("glass_dirs"), &list("lens_catalogs"), &mut out)?;
        return Ok(out);
    }
    let b = lensbench_core::vfs::read(f).map_err(|e| format!("{f}: {e}"))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| format!("{f}: {e}"))?;
    // 系ファイル（面の並び）も .zmx と同じく、系をまるごと取り込む要素（書ければ要素に分ける）の組合せにする
    if v.get("blocks").is_none() {
        let mut out = serde_json::json!({ "gui": Value::Null, "blocks": false, "zmx": true });
        peek_file(f, &list("glass_dirs"), &list("lens_catalogs"), &mut out)?;
        return Ok(out);
    }
    let mut out = serde_json::json!({
        "gui": v.get("gui").cloned().unwrap_or(Value::Null),
        "blocks": v.get("blocks").is_some(),
    });
    // ブロックのプロジェクトは、組合せで書ければ組合せにする（文書モデル 1 段目・4 段目。ブロックが元）
    if v.get("blocks").is_some() {
        let abs = crate::combo_from::absolute(std::path::Path::new(f));
        let base = abs.parent().unwrap_or(std::path::Path::new(""));
        match crate::combo::Combo::from_project(&v, base) {
            Ok(c) => out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?,
            Err(e) => {
                // 読めないプロジェクト（stage が load）は、開く側が系として読んで誤りを出す
                let load = lensbench_project::Project::from_json(&v.to_string()).is_err();
                out["stage"] = Value::from(if load { "load" } else { "combo" });
                out["why"] = Value::from(e);
            }
        }
    }
    Ok(out)
}

/// .zmx・システムファイルを組合せにして out の combo に。できなければ理由 why と止まった段 stage（load: 系として
/// 読めない、combo: 読めるが組合せで書けない）、読めなかった構成 config。読込は 1 回だけ（読めないファイルは
/// 開く側が系として読んで、硝材が無いときの案内などを出す）
fn peek_file(
    f: &str,
    glass_dirs: &[std::path::PathBuf],
    lens_catalogs: &[std::path::PathBuf],
    out: &mut Value,
) -> Result<(), String> {
    let path = std::path::Path::new(f);
    match crate::combo::Combo::from_zmx(path, glass_dirs, lens_catalogs) {
        Ok(c) => out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?,
        Err(e) => {
            out["stage"] = Value::from(e.stage());
            out["why"] = Value::from(e.to_string());
            if let crate::combo_from::OpenErr::Load {
                config: Some(k), ..
            } = e
            {
                out["config"] = Value::from(k);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn doc_peek_reads_gui_and_blocks() {
    let dir = lensbench_core::vfs::temp_dir().join("lensbench-gui-doc-peek");
    lensbench_core::vfs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    lensbench_core::vfs::write(&f, r#"{"blocks": [], "gui": {"view": {"state": "x"}}}"#).unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    // 読めないプロジェクト（wavelengths が無い）は、系として読んで誤りを出す
    assert_eq!(v["gui"], serde_json::json!({ "view": { "state": "x" } }));
    assert_eq!(v["blocks"], true);
    assert_eq!(v["stage"], "load", "{v}");
    // ブロックのプロジェクト: 組合せで書ければ combo
    let ex = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/projects");
    let peek = |n: &str| {
        let f = ex.join(n).display().to_string();
        doc_peek(&serde_json::json!({ "file": f })).unwrap()
    };
    assert_eq!(
        peek("P8_catalog_two.json")["combo"]["picks"][0]["part"],
        "PCX-100"
    );
    // 組合せで書けない（入れ子の群）なら combo は無い
    lensbench_core::vfs::write(&f, r#"{"wavelengths": [{"um": 0.55}], "aperture": {"type": "epd", "value": 2},
        "blocks": [{"type": "group", "id": "A", "blocks": [{"type": "group", "id": "B", "blocks": [
            {"type": "lens", "stop": true, "source": {"surfaces": [{"radius": 20, "t": 3, "glass": "N-BK7"}, {"radius": -20}]}}]}]},
            {"type": "air", "t": 9}]}"#)
    .unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert!(v.get("combo").is_none());
    assert_eq!(v["stage"], "combo", "{v}");
    // 読めない系ファイルは combo なし（理由 why）。系ファイルは .zmx と同じく組合せにする
    lensbench_core::vfs::write(&f, r#"{"surfaces": []}"#).unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert!(v.get("combo").is_none() && v["why"].is_string(), "{v}");
    assert_eq!(
        (&v["blocks"], &v["zmx"]),
        (&serde_json::json!(false), &serde_json::json!(true))
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let glass = root.join("tests/glass");
    let a0 = lensbench_formats::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_formats::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![glass.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    // 作業フォルダ（メモリ）でなく本物のファイルに書く（組合せのブロックはファイルの名前で指す）
    let real = std::env::temp_dir().join(format!("lb-doc-peek-sys-{}.json", std::process::id()));
    std::fs::write(
        &real,
        lensbench_project::sysfile::to_string(&a0, &[]).unwrap(),
    )
    .unwrap();
    let v = doc_peek(&serde_json::json!({ "file": real.display().to_string(),
        "glass_dirs": [glass.display().to_string()] }))
    .unwrap();
    std::fs::remove_file(&real).ok();
    assert!(
        v["combo"]["picks"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "{v}"
    );
    assert!(
        doc_peek(&serde_json::json!({ "file": dir.join("none.json").display().to_string() }))
            .is_err()
    );
    lensbench_core::vfs::remove_file(&f).ok();
}

/// リポジトリの .zmx で、読めるのに組合せで開けないものが無い（読めないもの・ここのガラスで読めないものは除く）。
/// 別のフォルダ（手元の .zmx など）を見るときは LB_PEEK_ROOT、足すガラスのフォルダは LB_PEEK_GLASS（: 区切り）。
/// 開かないファイルの組合せのプロジェクトを LB_PEEK_DUMP のフォルダに書く。LB_PEEK_UNREAD で読めない・計算できない理由を出す
#[cfg(test)]
#[test]
fn readable_files_open_as_combos() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let glass = root.join("tests/glass");
    // tests/bs_configs は SCHOTT を LBSCH の名前で引く
    let lbsch = std::env::temp_dir().join(format!("lb-peek-lbsch-{}", std::process::id()));
    std::fs::create_dir_all(&lbsch).unwrap();
    std::fs::copy(glass.join("SCHOTT.AGF"), lbsch.join("LBSCH.AGF")).unwrap();
    let mut files = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zmx")) {
                out.push(p);
            }
        }
    }
    let scan = std::env::var("LB_PEEK_ROOT").map_or(root.clone(), Into::into);
    walk(&scan, &mut files);
    files.sort();
    // LB_PEEK_GLASS（OS のパスの区切り）を先に探す: tests/glass の小さいカタログが実物のカタログを隠さないように
    let mut dirs: Vec<String> = std::env::var_os("LB_PEEK_GLASS")
        .map(|x| {
            std::env::split_paths(&x)
                .map(|p| p.display().to_string())
                .collect()
        })
        .unwrap_or_default();
    dirs.extend([glass.display().to_string(), lbsch.display().to_string()]);
    let (mut opened, mut unread, mut unsupported) = (0usize, 0usize, 0usize);
    // 計算できない理由（読めて計算できれば None）
    fn not_computable(f: &std::path::Path, dirs: &[String]) -> Option<String> {
        let lo = lensbench_formats::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: dirs.iter().map(Into::into).collect(),
            ..Default::default()
        };
        if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("zmx")) {
            return None;
        }
        match lensbench_formats::zmx::load_configs(f, &lo, &[]) {
            Err(e) => Some(e.to_string()),
            Ok(c) => c
                .iter()
                .find_map(|s| s.validate().err().map(|e| e.to_string())),
        }
    }
    let mut bad = Vec::new();
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    for f in &files {
        let v =
            doc_peek(&serde_json::json!({ "file": f.display().to_string(), "glass_dirs": dirs }));
        let Ok(v) = v else {
            unread += 1;
            continue;
        };
        if v.get("combo").is_some() {
            opened += 1;
        } else if v["stage"] == "load" {
            unread += 1;
            if std::env::var_os("LB_PEEK_UNREAD").is_some() {
                eprintln!("not readable: {}: {}", f.display(), v["why"]);
            }
        } else if let Some(e) = not_computable(f, &dirs) {
            // 読めるが追跡できない（未対応の理想レンズの置き方など）か、多重構成のどれかの構成が読めない: 理由を出して
            // 開かない（組合せの不足ではない）
            unsupported += 1;
            if std::env::var_os("LB_PEEK_UNREAD").is_some() {
                eprintln!("not computable: {}: {e}", f.display());
            }
        } else {
            let w = v["why"].as_str().unwrap_or_default().to_string();
            bad.push(format!("{}: {w}", f.display()));
            // LB_PEEK_DUMP のフォルダに、組合せにしたプロジェクト（.zmx は絶対パス）を書く。CLI の info で元と比べる
            if let Some(d) = std::env::var_os("LB_PEEK_DUMP") {
                let lo = lensbench_formats::zmx::LoadOptions {
                    cfg: 1,
                    catalog_dirs: dirs.iter().map(Into::into).collect(),
                    ..Default::default()
                };
                let p = lensbench_formats::zmx::load_configs(f, &lo, &[])
                    .and_then(|c| lensbench_project::zmx_project(&c, &f.display().to_string()));
                // 書けないもの（理由は上の why）は飛ばす
                if let Ok(p) = p {
                    let stem = f.file_stem().unwrap_or_default().to_string_lossy();
                    let out = std::path::Path::new(&d).join(format!("{stem}.project.json"));
                    std::fs::write(out, serde_json::to_string_pretty(&p).unwrap()).unwrap();
                }
            }
            // 理由は数字を伏せてまとめる
            let key: String = w
                .chars()
                .map(|c| if c.is_ascii_digit() { 'N' } else { c })
                .collect();
            *why.entry(key.chars().take(160).collect()).or_default() += 1;
        }
    }
    std::fs::remove_dir_all(&lbsch).ok();
    let mut by: Vec<_> = why.into_iter().collect();
    by.sort_by_key(|a| std::cmp::Reverse(a.1));
    let summary = format!(
        "{} files: {opened} opened, {unread} not readable, {unsupported} not computable, {} readable but not opened\n{}",
        files.len(),
        bad.len(),
        by.iter()
            .map(|(w, n)| format!("{n:6}  {w}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("{summary}");
    assert!(bad.is_empty(), "{summary}\n{}", bad.join("\n"));
    assert!(files.len() > 30, "{}", files.len());
}

/// 物体面のコメントと、物体面を元にするピックアップのある .zmx も組合せで開く（どちらも組合せの系に残る）
#[cfg(test)]
#[test]
fn zmx_with_object_comment_and_pickup_opens_as_combo() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let b = std::fs::read(root.join("tests/refs/A0/system.zmx")).unwrap();
    let u: Vec<u16> = b[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let t = String::from_utf16(&u).unwrap();
    let t = t.replacen("SURF 0\r\n", "SURF 0\r\n  COMM OBJ\r\n", 1);
    // 面 2 のコーニックを物体面から
    let at = t.find("SURF 3\r\n").unwrap();
    let t = format!("{}  PKUP 0 1.0 0\r\n{}", &t[..at], &t[at..]);
    let f = std::env::temp_dir().join(format!("lb-peek-obj-{}.zmx", std::process::id()));
    std::fs::write(&f, &t).unwrap();
    let v = doc_peek(&serde_json::json!({
        "file": f.display().to_string(),
        "glass_dirs": [root.join("tests/glass").display().to_string()],
    }))
    .unwrap();
    std::fs::remove_file(&f).ok();
    assert!(v.get("combo").is_some(), "{v}");
}

/// 構成 1 は読めて構成 2 だけ読めない多重構成の .zmx は、stage が load で読めない構成の番号 config を返す
/// （開く側はこれで構成 1 だけを系として開かずに理由を出す）
#[cfg(test)]
#[test]
fn zmx_with_unreadable_second_config_names_it() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let b = std::fs::read(root.join("tests/refs/A0/system.zmx")).unwrap();
    let u: Vec<u16> = b[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let t = String::from_utf16(&u).unwrap();
    // 面 1 の硝材を構成 2 だけ、どのカタログにも無い名前にする
    let t = t.replacen(
        "MNUM 1 1\r\n",
        "MNUM 2 1\r\nGLSS   1   1 N-SK16 0 0 0 1 1 0 0.0 \"\" 0\r\nGLSS   1   2 NO-SUCH-GLASS 0 0 0 1 1 0 0.0 \"\" 0\r\n",
        1,
    );
    let f = std::env::temp_dir().join(format!("lb-peek-cfg2-{}.zmx", std::process::id()));
    std::fs::write(&f, &t).unwrap();
    let v = doc_peek(&serde_json::json!({
        "file": f.display().to_string(),
        "glass_dirs": [root.join("tests/glass").display().to_string()],
    }))
    .unwrap();
    std::fs::remove_file(&f).ok();
    assert!(v.get("combo").is_none(), "{v}");
    assert_eq!(
        (&v["stage"], &v["config"]),
        (&serde_json::json!("load"), &serde_json::json!(2)),
        "{v}"
    );
}
