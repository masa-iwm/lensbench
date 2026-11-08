//! モンテカルロ公差の設定の JSON: 簡易設定の名前（ブロックの id）、カタログの玉の既定、上書き、誤りの検出

use lensbench_core::tolerance::Dist;
use lensbench_core::System;
use lensbench_project::tolerance::{catalog_vendors, out_opts, result_json, spec_from_json};
use lensbench_project::{expand, Env, Project};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const P: &str = r#"{"glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.5875618}], "aperture": {"type": "epd", "value": 10},
  "fields": {"type": "angle", "points": [[0, 0], [0, 3]]},
  "blocks": [
    {"type": "lens", "id": "L1", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}},
    {"type": "air", "id": "d1", "t": 2},
    {"type": "stop"},
    {"type": "air", "t": 3},
    {"type": "group", "id": "G", "blocks": [
      {"type": "lens", "id": "L2", "source": {"surfaces": [{"radius": 50, "t": 3, "glass": "N-BK7"}, {"radius": 0}]}}]},
    {"type": "air", "t": 40}
  ]}"#;

fn sys() -> (Project, System) {
    let p = Project::from_json(P).unwrap();
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    let s = expand(&p, &env).unwrap();
    (p, s)
}

fn labels(v: &serde_json::Value, s: &System, vendors: &HashMap<String, String>) -> Vec<String> {
    let m = spec_from_json(v, vec![s.clone()], vendors).unwrap();
    m.tols.iter().map(|t| t.label.clone()).collect()
}

#[test]
fn names_and_catalog_defaults() {
    let (p, s) = sys();
    // ブロックの id が玉の名前になる。群 G は玉 L2 と同じ範囲なので玉として扱う
    let l = labels(&json!({}), &s, &HashMap::new());
    for k in [
        "L1 R1",
        "L1 R2",
        "L1 厚さ1",
        "L1 偏心x",
        "L1 くさびy",
        "L1 後の間隔",
        "L2 R1",
        "L2 傾きy",
    ] {
        assert!(l.iter().any(|x| x == k), "{k} が {l:?} にない");
    }
    // 平面（R=0）の曲率半径の公差は作らない、最後の玉の後の間隔（ピント）も作らない
    assert!(!l.iter().any(|x| x == "L2 R2" || x == "L2 後の間隔"));
    // カタログの玉: 曲率半径ごとではなく焦点距離（メーカーによらない既定 ±2%・中心厚 ±0.1）
    let mut vd = HashMap::new();
    vd.insert("L1".to_string(), "Acme Optics".to_string());
    let m = spec_from_json(&json!({}), vec![s.clone()], &vd).unwrap();
    let f = m.tols.iter().find(|t| t.label == "L1 焦点距離").unwrap();
    assert_eq!(f.params.len(), 2);
    assert!((f.hi - 0.02).abs() < 1e-15 && f.relative);
    assert!(!m.tols.iter().any(|t| t.label == "L1 R1"));
    let ct = m.tols.iter().find(|t| t.label == "L1 厚さ1").unwrap();
    assert_eq!(ct.hi, 0.1);
    // メーカーの公差（名前の一部・大文字小文字によらない）と、外径・焦点距離の規則。L1 は外径 = 既定の semi から
    let g = |v: serde_json::Value| {
        let m = spec_from_json(&v, vec![s.clone()], &vd).unwrap();
        let h = |l: &str| m.tols.iter().find(|t| t.label == l).map(|t| t.hi);
        (h("L1 厚さ1"), h("L1 焦点距離"), h("L1 くさびx"))
    };
    let (ct, fr, _) = g(
        json!({"catalog_tols": {"acme": {"thickness": 0.15, "focal_rel": 0.01}}, "lenses": {"L2": null}}),
    );
    assert_eq!((ct, fr), (Some(0.15), Some(0.01)));
    let (ct, fr, w) = g(
        json!({"catalog_tols": {"ACME": {"thickness": 0.15, "focal_rel": 0.01,
        "rules": [{"dia_lt": 1000, "focal_rel": 0.03}, {"dia_gt": 1000, "thickness": 0.5}, {"efl_gt": 10, "efl_lt": 1000, "wedge": 0.02}]}}}),
    );
    assert_eq!((ct, fr, w), (Some(0.15), Some(0.03), Some(0.02)));
    // 単レンズ（L1 は 1 枚）か貼り合わせか
    let (ct, _, _) = g(
        json!({"catalog_tols": {"ACME": {"rules": [{"cemented": true, "thickness": 0.2}, {"cemented": false, "thickness": 0.12}]}}}),
    );
    assert_eq!(ct, Some(0.12));
    // "*" はほかのメーカー
    let (ct, _, _) =
        g(json!({"catalog_tols": {"OTHER": {"thickness": 0.3}, "*": {"thickness": 0.2}}}));
    assert_eq!(ct, Some(0.2));
    // 規則の項目の誤り
    assert!(spec_from_json(
        &json!({"catalog_tols": {"ACME": {"rules": [{"dia_lt": 5, "thick": 1}]}}}),
        vec![s.clone()],
        &vd
    )
    .is_err());
    let m = spec_from_json(&json!({"lenses": {"L2": null}}), vec![s.clone()], &vd).unwrap();
    assert!(!m.tols.iter().any(|t| t.label.starts_with("L2")));
    // catalog_vendors はプロジェクトのカタログのブロックだけ
    assert!(catalog_vendors(&p).is_empty());
    let q = Project::from_json(&P.replace(
        r#"{"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7"}, {"radius": -60}]}"#,
        r#"{"vendor": "Acme Optics", "part": "P-1"}"#,
    ))
    .unwrap();
    assert_eq!(
        catalog_vendors(&q).get("L1").map(|x| x.as_str()),
        Some("Acme Optics")
    );
}

#[test]
fn explicit_tols_compensators_and_result() {
    let (_, s) = sys();
    let v = json!({
        "preset": "none", "trials": 6, "seed": 3, "dist": "uniform",
        "tols": [{"param": "L1.dy", "pm": 0.05, "dist": "end"}, {"params": ["1.r", "2.r"], "lo": -0.01, "hi": 0.01, "relative": true, "label": "f"}],
        "groups": {"G": {"decenter": 0.0}},
        "compensators": ["G.shift=-2:2"],
        "terms": [{"type": "paraxial", "q": "efl", "target": 0}],
        "percentiles": [50],
    });
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    assert_eq!(m.tols.len(), 2);
    assert_eq!(m.tols[1].label, "f");
    assert_eq!(m.problem.vars.len(), 1);
    // 既定の評価関数（視野 2 つ）＋ 足した項
    assert_eq!(m.problem.terms.len(), 3);
    let r = lensbench_core::tolerance::run(&m).unwrap();
    let out = result_json(&m, &r, &out_opts(&v).unwrap());
    assert_eq!(out["trials"], 6);
    assert_eq!(out["sensitivity"].as_array().unwrap().len(), 2);
    assert_eq!(out["merit"]["percentiles"].as_array().unwrap().len(), 1);
    // end の分布: ずれは ±0.05 のどちらか
    for t in &r.trials {
        assert!(t.deltas[0].abs() == 0.05);
    }
    // 誤り
    for bad in [
        json!({"trails": 1}),
        json!({"lenses": {"X9": null}}),
        json!({"tols": [{"param": "1.t"}]}),
        json!({"preset": "best"}),
        json!({"lenses": {"L1": {"wedgee": 1}}}),
    ] {
        assert!(
            spec_from_json(&bad, vec![s.clone()], &HashMap::new()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn compensator_per_config() {
    // 構成ごとの補償子: configs を書いた補償子はその構成だけに効く。構成ごとに別の値になる
    let (_, s) = sys();
    let s2 = s.clone();
    let v = json!({
        "preset": "none", "trials": 3, "seed": 5, "dist": "uniform", "sensitivity": false,
        "tols": [{"param": "L1.dy", "pm": 0.05}],
        "groups": {"G": {"decenter": 0.0}},
        "compensators": [{"param": "G.shift", "lo": -2, "hi": 2, "configs": [1]},
                         {"param": "G.shift", "lo": -2, "hi": 2, "configs": [2]}],
    });
    let m = spec_from_json(&v, vec![s.clone(), s2.clone()], &HashMap::new()).unwrap();
    assert_eq!(m.problem.vars[0].configs, Some(vec![0]));
    assert_eq!(m.problem.vars[1].configs, Some(vec![1]));
    let r = lensbench_core::tolerance::run(&m).unwrap();
    let out = result_json(&m, &r, &out_opts(&v).unwrap());
    assert_eq!(out["compensators"][1]["configs"], json!([2]));
    assert!(out["compensators"][0]["configs"] == json!([1]));
    for bad in [
        json!({"compensators": [{"param": "G.shift", "configs": [0]}]}),
        json!({"compensators": [{"param": "G.shift", "configs": 1}]}),
        json!({"compensators": [{"param": "G.shift", "config": [1]}]}),
        json!({"compensators": [{"param": "G.shift", "configs": [3]}], "trials": 1}),
    ] {
        let e = spec_from_json(&bad, vec![s.clone(), s2.clone()], &HashMap::new()).and_then(|m| {
            lensbench_core::tolerance::run(&m)
                .map_err(|e| e.to_string())
                .map(|_| ())
        });
        assert!(e.is_err(), "{bad}");
    }
}

#[test]
fn radial_pairs() {
    let (_, s) = sys();
    let v = json!({"preset": "commercial", "trials": 4, "dist": "normal", "radial": true, "sensitivity": false,
                   "tols": [{"param": "L1.dx", "pm": 0.1}, {"param": "L1.dy", "pm": 0.1}]});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    let d: Vec<Dist> = m.tols.iter().map(|t| t.dist).collect();
    assert!(d.contains(&Dist::DiscX) && d.contains(&Dist::Normal { cut: 2.0 }));
    let n = m.tols.len();
    assert_eq!(
        (m.tols[n - 2].dist, m.tols[n - 1].dist),
        (Dist::DiscX, Dist::DiscY)
    );
    let r = lensbench_core::tolerance::run(&m).unwrap();
    let out = result_json(&m, &r, &out_opts(&v).unwrap());
    assert!(out["tolerances"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["dist"] == "disc_x"));
    for t in &r.trials {
        let (x, y) = (t.deltas[n - 2], t.deltas[n - 1]);
        assert!(x.hypot(y) <= 0.1 + 1e-12);
    }
    let v = json!({"radial": 1});
    assert!(spec_from_json(&v, vec![s.clone()], &HashMap::new()).is_err());
    // 既定は組にしない
    let m = spec_from_json(&json!({}), vec![s], &HashMap::new()).unwrap();
    assert!(m
        .tols
        .iter()
        .all(|t| !matches!(t.dist, Dist::DiscX | Dist::DiscY)));
}
