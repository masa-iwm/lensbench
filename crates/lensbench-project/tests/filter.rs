//! フィルタ・ダイクロイックの透過特性: プロジェクト・ユーザーカタログの `coatings` が TABLE 膜として
//! 偏光の追跡に効く

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::FieldSel;
use lensbench_project::{expand_path, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env(base: PathBuf) -> Env<'static> {
    Env {
        base,
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// 軸上の光線の強度透過率（x と y の直線偏光の平均）
fn transmission(s: &lensbench_core::System, w: usize) -> f64 {
    let coat = Coatings::for_system(s).unwrap();
    let one = C64::new(1.0, 0.0);
    let zero = C64::new(0.0, 0.0);
    let t = s
        .try_trace_pol_opts(
            FieldSel::Index(0),
            &[0.0],
            &[0.0],
            Some(w),
            &[[one, zero], [zero, one]],
            &coat,
            false,
        )
        .unwrap();
    let e = &t.e[0];
    (e[0]
        .iter()
        .chain(e[1].iter())
        .map(|c| c.norm_sqr())
        .sum::<f64>())
        / 2.0
}

fn project(coatings: &str, blocks: &str) -> Project {
    Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.532}}, {{"um": 0.6}}, {{"um": 0.45}}],
        "aperture": {{"type": "epd", "value": 2}}, "fields": {{"points": [[0, 0]]}},
        "coatings": {coatings}, "blocks": {blocks}}}"#
    ))
    .unwrap()
}

/// 帯域フィルタの面: 帯域内（532 nm）は t、帯域外（600 nm）は 10^−od。裏面（膜なし）のフレネルは同じくらい
#[test]
fn bandpass_on_plate() {
    let p = project(
        r#"{"BP532": {"type": "bandpass", "center_nm": 532, "fwhm_nm": 10, "t": 0.9, "od": 3}}"#,
        r#"[{"type": "stop"}, {"type": "air", "t": 5},
           {"type": "lens", "source": {"surfaces": [{"t": 2, "glass": "N-BK7", "coat": "BP532"}, {}]}},
           {"type": "air", "t": 10}]"#,
    );
    let s = expand_path(&p, &env(root()), None).unwrap().0;
    assert!(s.coat_tables.contains_key("BP532"));
    let (tin, tout) = (transmission(&s, 0), transmission(&s, 1));
    // 裏面のフレネル（N-BK7 → 空気 ≈ 4 %）を除く
    let fres = |n: f64| 1.0 - ((n - 1.0) / (n + 1.0)).powi(2);
    let n = |w: usize| s.surfaces[2].n[w];
    assert!((tin / fres(n(0)) - 0.9).abs() < 1e-6, "{tin}");
    assert!((tout / fres(n(1)) - 1e-3).abs() < 1e-8, "{tout}");
    // 仕様の誤り（半値全幅が中心波長以上）は展開で誤り
    let p = project(
        r#"{"BP": {"type": "bandpass", "center_nm": 532, "fwhm_nm": 600}}"#,
        "[]",
    );
    assert!(expand_path(&p, &env(root()), None).is_err());
}

/// ユーザーカタログの coatings: 品の面の coat から使え、プロジェクトの同じ名前が優先
#[test]
fn catalog_coatings() {
    let dir = std::env::temp_dir().join(format!("lbfilt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("filters.json"),
        r#"{"version": 1, "coatings": {"ND1": {"type": "nd", "od": 1, "absorptive": true}},
          "items": {"NE10A": {"kind": "filter", "blocks": [{"type": "lens", "source": {"surfaces": [
            {"t": 1, "glass": "N-BK7", "coat": "ND1"}, {"semi": 5}]}}]}}}"#,
    )
    .unwrap();
    let blocks = r#"[{"type": "stop"}, {"type": "air", "t": 5},
        {"type": "user", "catalog": "filters.json", "item": "NE10A"}, {"type": "air", "t": 10}]"#;
    let s = expand_path(&project("{}", blocks), &env(dir.clone()), None)
        .unwrap()
        .0;
    let fres = |n: f64| 1.0 - ((n - 1.0) / (n + 1.0)).powi(2);
    let t = transmission(&s, 0);
    assert!((t / fres(s.surfaces[2].n[0]) - 0.1).abs() < 1e-9, "{t}");
    let s = expand_path(
        &project(
            r#"{"ND1": {"type": "nd", "od": 2, "absorptive": true}}"#,
            blocks,
        ),
        &env(dir.clone()),
        None,
    )
    .unwrap()
    .0;
    let t = transmission(&s, 0);
    assert!((t / fres(s.surfaces[2].n[0]) - 0.01).abs() < 1e-9, "{t}");
    std::fs::remove_dir_all(&dir).ok();
}

/// ダイクロイック BS（ロングパス 500 nm、45°）: 透過の経路は長い波長、反射の経路は短い波長が通る
#[test]
fn dichroic_splits_wavelengths() {
    let p = project(
        r#"{"DMLP500": {"type": "longpass", "edge_nm": 500, "width_nm": 10, "t": 0.95, "od": 3}}"#,
        r#"[{"type": "stop"}, {"type": "air", "t": 10},
           {"type": "splitter", "id": "BS", "form": "ideal", "coat": "DMLP500", "angle": [45, 0]},
           {"type": "air", "t": 20}]"#,
    );
    let t = expand_path(&p, &env(root()), Some("T")).unwrap().0;
    let r = expand_path(&p, &env(root()), Some("R")).unwrap().0;
    // 532 nm・600 nm は透過、450 nm は反射（45° でエッジは 500·√(1 − (sin45°/2)²) ≈ 468 nm へ）
    for w in [0, 1] {
        assert!(transmission(&t, w) > 0.9, "{w} {}", transmission(&t, w));
        assert!(transmission(&r, w) < 0.1, "{w} {}", transmission(&r, w));
    }
    assert!(transmission(&t, 2) < 0.01, "{}", transmission(&t, 2));
    assert!(transmission(&r, 2) > 0.95, "{}", transmission(&r, 2));
}
