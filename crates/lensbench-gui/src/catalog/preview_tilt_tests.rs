//! 傾いた鏡のカタログのプレビュー

use super::*;
use serde_json::json;

/// 45° の平面鏡のプレビュー: 絞りは鏡の手前の端より前、光線は絞りを通ってから鏡で折れる
#[test]
fn tilted_mirror_preview_stop_before_mirror() {
    let r = LensRow {
        vendor: "V".into(),
        part: "M".into(),
        efl: f64::INFINITY,
        diameter: 25.4,
        thickness: 6.35,
        mirror: Some((f64::INFINITY, 0.0)),
        ..Default::default()
    };
    let p = preview_project(&r);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let glass = root.join("tests/glass").display().to_string();
    let dir = std::env::temp_dir().join(format!("lensbench-gui-mirprev-{}", std::process::id()));
    lensbench_core::vfs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    lensbench_core::vfs::write(&f, p.to_string()).unwrap();
    let lay = crate::combo::LayoutData::parse(
        &lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dirs": [glass] }),
            &json!({ "rays": 5 }),
        )
        .unwrap(),
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(!lay.rays.is_empty());
    // 光線は z = 0（絞り）から +z へ進んでから折れる（絞りより前で鏡に当たらない）
    for (_, pts) in &lay.rays {
        assert!(pts[1][0] > 0.5, "{pts:?}");
    }
}

/// カタログの玉は既定でロック、鏡・BS はロックしない
#[test]
fn catalog_lens_locked_by_default() {
    let lens = LensRow {
        vendor: "V".into(),
        part: "L".into(),
        efl: 50.0,
        diameter: 25.4,
        thickness: 3.0,
        pieces: 1,
        ..Default::default()
    };
    assert!(pick_from(&lens).locked);
    let mirror = LensRow {
        mirror: Some((f64::INFINITY, 0.0)),
        ..lens
    };
    assert!(!pick_from(&mirror).locked);
}

/// 45° の平板 BS（径 25.4、厚さ 1）のプレビュー: 板は前後に径 × sin45° 広がるので、絞りは板の手前の端より前、
/// 光線は絞りを通ってから板を貫く（板より後ろから始まらない）
#[test]
fn plate_splitter_preview_stop_before_plate() {
    let r = LensRow {
        vendor: "V".into(),
        part: "BSP".into(),
        kind: RowKind::Splitter,
        efl: f64::NAN,
        diameter: 25.4,
        thickness: 1.0,
        splitter: Some(json!({
            "type": "splitter", "id": "BS", "form": "plate", "thickness": 1.0,
            "semi": 12.7, "glass": "N-BK7", "ratio": 0.5
        })),
        ..Default::default()
    };
    let p = preview_project(&r);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let glass = root.join("tests/glass").display().to_string();
    let dir = std::env::temp_dir().join(format!("lensbench-gui-bsprev-{}", std::process::id()));
    lensbench_core::vfs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    lensbench_core::vfs::write(&f, p.to_string()).unwrap();
    let lay = crate::combo::LayoutData::parse(
        &lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dirs": [glass] }),
            &json!({ "rays": 5 }),
        )
        .unwrap(),
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(!lay.rays.is_empty());
    // 光線は絞り（z = 0）から +z へ進んでから板に当たる。板が絞りより前にはみ出すと、後ろ向きに板まで戻る線になる
    // （B の口は軸上で板の厚さの所にあり、板の縁の側では裏面より少し手前になるので、厚さほどの戻りは許す）
    for (_, pts) in &lay.rays {
        assert!(pts[1][0] > 0.5, "{pts:?}");
        for w in pts.windows(2) {
            assert!(w[1][0] >= w[0][0] - 1.0, "{pts:?}");
        }
    }
}
