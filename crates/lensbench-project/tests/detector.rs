//! 検出器のブロック（登録した種類、`lensbench_project::detectors`）: 読込・書き出し・展開・誤り
use lensbench_project::detectors::{self, DetectorKind};
use lensbench_project::{expand, Block, Env, Project};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("examples/projects"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn check_toy(m: &Map<String, Value>) -> Result<(), String> {
    for k in m.keys() {
        if k != "pixel" {
            return Err(format!("知らない項目 {k}"));
        }
    }
    Ok(())
}

fn register() {
    detectors::register(DetectorKind {
        name: "toy_camera",
        doc: "試しの検出器",
        check: check_toy,
        schema: Some(|| json!({"type": "object", "properties": {"pixel": {"type": "number"}}, "additionalProperties": false})),
    })
    .unwrap();
}

fn project(blocks: Value) -> Value {
    json!({
        "glass_catalogs": ["SCHOTT"],
        "wavelengths": [{"um": 0.55}],
        "aperture": {"type": "epd", "value": 4},
        "fields": {"type": "angle", "points": [[0, 0]]},
        "blocks": blocks,
    })
}

fn lens() -> Value {
    json!({"type": "lens", "id": "L1", "source": {"surfaces": [
        {"radius": 51.68, "t": 4, "glass": "N-BK7", "semi": 8},
        {"radius": -51.68, "semi": 8}]}})
}

#[test]
fn detector_is_read_written_and_adds_no_surface() {
    register();
    let det = json!({"type": "toy_camera", "id": "C", "pixel": 0.005});
    let v = project(json!([{"type": "stop", "semi": 2}, lens(), {"type": "air", "t": 48}, det]));
    let p = Project::from_json(&v.to_string()).unwrap();
    let d = p.detector().unwrap();
    assert_eq!((d.kind.as_str(), d.id.as_str()), ("toy_camera", "C"));
    assert_eq!(d.params["pixel"], json!(0.005));
    // 書き出しは元の形（"type" は種類の名前）
    let w: Value = serde_json::from_str(&p.to_json()).unwrap();
    assert_eq!(
        w["blocks"][3],
        json!({"type": "toy_camera", "id": "C", "pixel": 0.005})
    );
    assert_eq!(
        Project::from_json(&p.to_json()).unwrap().detector(),
        Some(d)
    );
    // 検出器を除いても同じ系
    let a = expand(&p, &env()).unwrap();
    let mut q = p.clone();
    assert!(matches!(q.blocks.pop(), Some(Block::Detector(_))));
    assert_eq!(a.surfaces.len(), expand(&q, &env()).unwrap().surfaces.len());
    // 登録した種類はスキーマに載る
    let sc = lensbench_project::schema::schema("project").unwrap();
    let blocks = sc
        .pointer("/$defs/Block/oneOf")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(blocks
        .iter()
        .any(|b| b.pointer("/properties/type/const") == Some(&json!("toy_camera"))));
}

#[test]
fn detector_errors() {
    register();
    // 種類の項目の誤り（登録した確かめ）
    let v = project(json!([lens(), {"type": "toy_camera", "nosie": 1}]));
    let e = Project::from_json(&v.to_string()).unwrap_err();
    assert!(e.contains("nosie"), "{e}");
    // 登録していない種類
    let v = project(json!([lens(), {"type": "no_such_camera"}]));
    let e = Project::from_json(&v.to_string()).unwrap_err();
    assert!(
        e.contains("知らないブロックの種類") && e.contains("toy_camera"),
        "{e}"
    );
    // 群の中も確かめる
    let v = project(json!([{"type": "group", "blocks": [{"type": "no_such_camera"}]}]));
    assert!(Project::from_json(&v.to_string()).is_err());
    // 組み込みのブロックの誤りは今までどおりの説明
    let v = project(json!([{"type": "air"}]));
    let e = Project::from_json(&v.to_string()).unwrap_err();
    assert!(e.contains("missing field `t`"), "{e}");
    // 最後のブロックだけ
    let v = project(json!([{"type": "toy_camera"}, lens()]));
    let p = Project::from_json(&v.to_string()).unwrap();
    let e = expand(&p, &env()).unwrap_err();
    assert!(e.contains("検出器"), "{e}");
    // 組み込みの名前は登録できない
    assert!(detectors::register(DetectorKind {
        name: "lens",
        doc: "",
        check: check_toy,
        schema: None,
    })
    .is_err());
}
