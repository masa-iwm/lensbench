//! 組合せの系の絞り・ソルブ・ピックアップ（surface_ops）

use lensbench_project::{expand_report, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn project(stop_block: bool, ops: &str) -> Result<(lensbench_core::System, Vec<String>), String> {
    let stop = if stop_block {
        r#"{"type": "stop", "semi": 2.5}, {"type": "air", "t": 2},"#
    } else {
        ""
    };
    let txt = format!(
        r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
        "aperture": {{"type": "epd", "value": 5}},
        "blocks": [{stop}
          {{"type": "lens", "id": "L1", "source": {{"surfaces": [
            {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -50, "semi": 10}}]}}}},
          {{"type": "air", "t": 10}},
          {{"type": "lens", "id": "L2", "source": {{"surfaces": [
            {{"radius": 40, "t": 3, "glass": "N-BK7", "semi": 10}}, {{"radius": -60, "semi": 10}}]}}}},
          {{"type": "air", "t": 30}}],
        "surface_ops": {ops} }}"#
    );
    expand_report(&Project::from_json(&txt)?, &env())
}

#[test]
fn stop_solve_pickup_by_block() {
    let ops = r#"[
        {"op": "set_stop", "surface": {"block": "L1", "surface": 1}},
        {"op": "add_pickup", "dst": {"block": "L2", "surface": 1}, "src": {"block": "L2", "surface": 0}, "what": "c", "scale": -1},
        {"op": "add_solve", "surface": {"block": "L2", "surface": 1}, "kind": "marginal_height", "value": 0}
    ]"#;
    // 絞りのブロックが無くても、面に置けばよい
    let (s, warn) = project(false, ops).unwrap();
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.stop(), 2);
    assert!((s.surfaces[4].c + s.surfaces[3].c).abs() < 1e-15);
    assert_eq!(s.solves.len(), 1);
    assert_eq!(s.solves[0].surf, 4);
    // 像面に周辺光線が集まる（厚さを解いた）
    assert!((s.surfaces[4].t - 30.0).abs() > 1e-6);
    // 絞りのブロックがあっても面に移る
    let (s, _) = project(true, ops).unwrap();
    assert_eq!(s.stop(), 3);
    assert_eq!(s.surfaces.iter().filter(|d| d.stop).count(), 1);
}

#[test]
fn missing_block_warns_and_bad_ops_fail() {
    let (_, warn) = project(
        true,
        r#"[{"op": "add_solve", "surface": {"block": "X", "surface": 0}}]"#,
    )
    .unwrap();
    assert!(warn.iter().any(|w| w.contains("X")), "{warn:?}");
    let e = project(
        true,
        r#"[{"op": "set_stop", "surface": {"block": "L1", "surface": 5}}]"#,
    )
    .unwrap_err();
    assert!(e.contains("面 5"), "{e}");
    let e = project(true, r#"[{"op": "delete_surface", "surface": 1}]"#).unwrap_err();
    assert!(e.contains("delete_surface"), "{e}");
    // 絞りが無く set_stop も無ければ従来どおりのエラー
    assert!(project(false, "[]").unwrap_err().contains("絞り"));
    // 絞りが無く、set_stop のブロックが無い（状態で外した・id の誤り）ときもエラー（仮の絞りのままにしない）
    let e = project(
        false,
        r#"[{"op": "set_stop", "surface": {"block": "X", "surface": 0}}]"#,
    )
    .unwrap_err();
    assert!(e.contains("絞りが 0 個"), "{e}");
}
