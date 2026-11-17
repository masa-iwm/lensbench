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
        match crate::combo::Combo::from_zmx(
            std::path::Path::new(f),
            &list("glass_dirs"),
            &list("lens_catalogs"),
        ) {
            Ok(c) => out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?,
            Err(e) => out["why"] = Value::from(e),
        }
        return Ok(out);
    }
    let b = lensbench_core::vfs::read(f).map_err(|e| format!("{f}: {e}"))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| format!("{f}: {e}"))?;
    // 系ファイル（面の並び）も .zmx と同じく、系をまるごと取り込む要素（書ければ要素に分ける）の組合せにする
    if v.get("blocks").is_none() {
        let mut out = serde_json::json!({ "gui": Value::Null, "blocks": false, "zmx": true });
        match crate::combo::Combo::from_zmx(
            std::path::Path::new(f),
            &list("glass_dirs"),
            &list("lens_catalogs"),
        ) {
            Ok(c) => out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?,
            Err(e) => out["why"] = Value::from(e),
        }
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
        if let Ok(c) = crate::combo::Combo::from_project(&v, base) {
            out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[cfg(test)]
#[test]
fn doc_peek_reads_gui_and_blocks() {
    let dir = lensbench_core::vfs::temp_dir().join("lensbench-gui-doc-peek");
    lensbench_core::vfs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    lensbench_core::vfs::write(&f, r#"{"blocks": [], "gui": {"view": {"state": "x"}}}"#).unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "gui": { "view": { "state": "x" } }, "blocks": true })
    );
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
    let a0 = lensbench_core::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_core::zmx::LoadOptions {
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
