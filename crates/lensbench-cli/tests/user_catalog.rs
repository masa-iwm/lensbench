//! serve のユーザーカタログの操作（GUI の保存）: 読み込んだプロジェクトのブロック・範囲の保存、一覧、
//! 名前・説明の変更、削除。系の読み込みはプロセスで 1 つなので、このファイルのテストは 1 つにまとめる

use lensbench_engine::Session;
use serde_json::{json, Value};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn call(sv: &Session, m: &str, p: Value) -> Result<Value, String> {
    sv.call(m, &p, Some(1)).map_err(|e| format!("{e:?}"))
}

#[test]
fn save_list_rename_delete() {
    let sv = Session::new();
    let dir = std::env::temp_dir().join(format!("lb-ucat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = dir.join("combo.json");
    std::fs::write(
        &proj,
        json!({
            "glass_catalogs": ["SCHOTT"],
            "wavelengths": [{"um": 0.5875618}], "primary": 1,
            "aperture": {"type": "epd", "value": 8},
            "fields": {"type": "angle", "points": [[0, 0]]},
            "blocks": [
                {"type": "stop", "semi": 4}, {"type": "air", "t": 2},
                {"type": "lens", "id": "L1", "source": {"surfaces": [
                    {"radius": 60, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -80, "semi": 8}]}},
                {"type": "air", "t": 3},
                {"type": "lens", "id": "L2", "source": {"surfaces": [
                    {"radius": -40, "t": 2, "glass": "F2", "semi": 8}, {"radius": 200, "semi": 8}]}},
                {"type": "air", "t": 50}],
            "surface_edits": [{"block": "L2", "surface": 0, "fields": {"r": -45.0}}]
        })
        .to_string(),
    )
    .unwrap();
    let glass = root().join("tests/glass");
    call(&sv, "load", json!({ "file": proj, "glass_dirs": [glass] })).unwrap();
    let cat = dir.join("my.json").display().to_string();
    // 1 つ（面の表の変更を焼き込む）と範囲
    let v = call(&sv,
        "catalog_add",
        json!({ "catalog": cat, "name": "second", "blocks": ["L2"], "description": "edited", "vendor": "MY" }),
    )
    .unwrap();
    assert_eq!(v["added"], "second");
    assert_eq!(v["vendor"], "MY");
    let v = call(
        &sv,
        "catalog_add",
        json!({ "catalog": cat, "name": "pair", "blocks": ["L1", "L2"] }),
    )
    .unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    let txt = std::fs::read_to_string(&cat).unwrap();
    let c: Value = serde_json::from_str(&txt).unwrap();
    assert_eq!(
        c["items"]["second"]["blocks"][0]["source"]["surfaces"][0]["radius"],
        -45.0
    );
    assert_eq!(c["items"]["pair"]["blocks"].as_array().unwrap().len(), 3);
    // 同じ名前は replace が要る
    let e = call(
        &sv,
        "catalog_add",
        json!({ "catalog": cat, "name": "pair", "blocks": ["L1"] }),
    )
    .unwrap_err();
    assert!(e.contains("もうある"), "{e}");
    call(
        &sv,
        "catalog_add",
        json!({ "catalog": cat, "name": "pair", "blocks": ["L1"], "replace": true }),
    )
    .unwrap();
    // 名前・説明の変更、削除
    let v = call(
        &sv,
        "catalog_edit",
        json!({ "catalog": cat, "name": "second", "rename": "L2 edited", "description": "d2" }),
    )
    .unwrap();
    let names: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["L2 edited", "pair"]);
    assert_eq!(v["items"][0]["description"], "d2");
    let e = call(
        &sv,
        "catalog_edit",
        json!({ "catalog": cat, "name": "pair", "rename": "L2 edited" }),
    )
    .unwrap_err();
    assert!(e.contains("もうある"), "{e}");
    let v = call(
        &sv,
        "catalog_edit",
        json!({ "catalog": cat, "name": "pair", "delete": true }),
    )
    .unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let v = call(&sv, "catalog_items", json!({ "catalog": cat })).unwrap();
    assert_eq!(v["items"][0]["name"], "L2 edited");
    std::fs::remove_dir_all(&dir).ok();
}
