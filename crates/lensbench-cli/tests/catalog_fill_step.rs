//! 未完のカタログ品を STEP で補う: lensbench が書き出した平行平板・平面鏡の STEP を測り、厚さ・径が
//! 作った値（仕様表の値に当たる）と合うか（公開 #70）

use std::process::Command;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 厚さ t・径 d の平板を置いた系の、平板の面（2〜3）だけを STEP に書き出して測る
fn measure(t: f64, d: f64, glass: &str) -> lensbench_project::catalog_fill::StepSize {
    let dir = std::env::temp_dir().join(format!(
        "lensbench-fill-step-{}-{t}-{d}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("p.json");
    std::fs::write(
        &p,
        serde_json::json!({
            "glass_catalogs": ["SCHOTT"],
            "wavelengths": [{ "um": 0.5876 }],
            "aperture": { "type": "epd", "value": d * 0.2 },
            "fields": { "points": [[0.0, 0.0]] },
            "blocks": [
                { "type": "stop" },
                { "type": "air", "t": 5.0 },
                { "type": "lens", "source": { "surfaces": [
                    { "t": t, "glass": glass, "semi": d / 2.0 }, { "semi": d / 2.0 } ] } },
                { "type": "air", "t": 10.0 },
            ],
        })
        .to_string(),
    )
    .unwrap();
    let step = dir.join("p.step");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .args([
            "layout",
            p.to_str().unwrap(),
            "--glass-dir",
            "tests/glass",
            "--rays",
            "0",
            "--first",
            "2",
            "--last",
            "3",
        ])
        .arg("--out")
        .arg(&step)
        .current_dir(root())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s = lensbench_project::catalog_fill::read_step(&std::fs::read_to_string(&step).unwrap())
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    s
}

#[test]
fn step_of_plates_matches_specs() {
    for (t, d) in [(3.0, 25.0), (6.35, 50.8), (1.1, 12.5)] {
        let s = measure(t, d, "N-BK7");
        assert!((s.thickness - t).abs() < 1e-6, "{t} {d}: {s:?}");
        assert_eq!(s.diameter.as_f64(), Some(d), "{t} {d}: {s:?}");
    }
}
