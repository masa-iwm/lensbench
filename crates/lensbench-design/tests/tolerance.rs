//! モンテカルロ公差の設定の JSON: 簡易設定の名前（ブロックの id）、カタログの玉の既定、上書き、誤りの検出

use lensbench_core::param::{Kind, Param};
use lensbench_core::System;
use lensbench_design::montecarlo::Dist;
use lensbench_design::tolerance::{
    catalog_vendors, compensators_csv, out_opts, result_json, reversed_lenses, spec_from_json,
    spec_from_json_with, tilt_pairs,
};
use lensbench_project::{expand, Env, Project};
use serde_json::json;
use std::collections::{HashMap, HashSet};
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
    let r = lensbench_design::montecarlo::run(&m).unwrap();
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
    let r = lensbench_design::montecarlo::run(&m).unwrap();
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
            lensbench_design::montecarlo::run(&m)
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
    assert!(d.contains(&Dist::DiscX { normal: None }) && d.contains(&Dist::Normal { cut: 2.0 }));
    let n = m.tols.len();
    assert_eq!(
        (m.tols[n - 2].dist, m.tols[n - 1].dist),
        (Dist::DiscX { normal: None }, Dist::DiscY { normal: None })
    );
    let r = lensbench_design::montecarlo::run(&m).unwrap();
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
    let v = json!({"radial": "end"});
    assert!(spec_from_json(&v, vec![s.clone()], &HashMap::new()).is_err());
    // 円の中の 2 次元正規分布
    let v = json!({"trials": 4, "radial": "normal:3", "sensitivity": false,
                   "tols": [{"param": "L1.dx", "pm": 0.1}, {"param": "L1.dy", "pm": 0.1}]});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    let n = m.tols.len();
    assert_eq!(
        (m.tols[n - 2].dist, m.tols[n - 1].dist),
        (
            Dist::DiscX { normal: Some(3.0) },
            Dist::DiscY { normal: Some(3.0) }
        )
    );
    let r = lensbench_design::montecarlo::run(&m).unwrap();
    let out = result_json(&m, &r, &out_opts(&v).unwrap());
    assert!(out["tolerances"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["dist"] == "disc_y:normal:3"));
    // 公差ごとの分布でも書ける
    let v = json!({"preset": "none", "catalog": false, "trials": 2, "sensitivity": false,
                   "tols": [{"param": "L1.dx", "pm": 0.1, "dist": "disc_x:normal"},
                            {"param": "L1.dy", "pm": 0.1, "dist": "disc_y:normal"}]});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    assert!(lensbench_design::montecarlo::run(&m).is_ok());
    // 既定は組にしない
    let m = spec_from_json(&json!({}), vec![s], &HashMap::new()).unwrap();
    assert!(m
        .tols
        .iter()
        .all(|t| !matches!(t.dist, Dist::DiscX { .. } | Dist::DiscY { .. })));
}

/// 鏡の back で往復する系（貼り合わせ L1・単レンズ L2 を通って平面鏡で戻る）
const DP: &str = r#"{"glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.5875618}], "aperture": {"type": "na", "value": 0.05},
  "fields": {"type": "object_height", "points": [[0, 0]]},
  "object_distance": 0,
  "blocks": [
    {"type": "air", "t": 50},
    {"type": "lens", "id": "L1", "source": {"surfaces": [
      {"radius": 60, "t": 5, "glass": "N-BK7"}, {"radius": -40, "t": 2, "glass": "F2"}, {"radius": -120}]},
     "stop": true},
    {"type": "air", "t": 10},
    {"type": "lens", "id": "L2", "source": {"surfaces": [{"radius": 80, "t": 3, "glass": "N-BK7"}, {"radius": 0}]}},
    {"type": "air", "t": 20},
    {"type": "mirror", "id": "M", "back": true, "semi": 20}
  ]}"#;

#[test]
fn double_pass_lens_errors_on_both_passes() {
    let p = Project::from_json(DP).unwrap();
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    let s = expand(&p, &env).unwrap();
    let m = spec_from_json(&json!({"seed": 3}), vec![s.clone()], &HashMap::new()).unwrap();
    // 玉は往復で 1 つ（L1・L2）。戻りの範囲を別の玉（lens3 …）にしない
    let names: Vec<&str> = m.tols.iter().map(|t| t.label.as_str()).collect();
    assert!(
        names
            .iter()
            .all(|l| l.starts_with("L1 ") || l.starts_with("L2 ")),
        "{names:?}"
    );
    for k in [
        "L1 厚さ1",
        "L1 厚さ2",
        "L1 屈折率2",
        "L1 くさびx",
        "L1 偏心y",
        "L1 後の間隔",
        "L2 R1",
    ] {
        assert!(names.contains(&k), "{k} が {names:?} にない");
    }
    // 往きの面と戻りの同じ面の組（往き j ↔ 戻り j'）を名前から作る
    let mut pairs = Vec::new(); // (往きの面, 戻りの面)
    let n = s.image();
    let mirror = (1..n).find(|&j| s.surfaces[j].mirror).unwrap();
    for j in 1..mirror {
        pairs.push((j, 2 * mirror - j));
    }
    // 1 つずつ公差の端を与え、往きと戻りが同じ玉として変わったかを見る
    for (i, t) in m.tols.iter().enumerate() {
        let mut d = vec![0.0; m.tols.len()];
        d[i] = t.hi;
        let q = &lensbench_design::montecarlo::perturbed(&m, &d).unwrap()[0];
        for &(a, b) in &pairs {
            let (sa, sb, qa, qb) = (
                &s.surfaces[a],
                &s.surfaces[b],
                &q.surfaces[a],
                &q.surfaces[b],
            );
            // 曲率は同じ比（面の向きで符号は同じ）
            if sa.c != 0.0 {
                assert!(
                    (qa.c / sa.c - qb.c / sb.c).abs() < 1e-12,
                    "{}: 面 {a}・{b} の曲率",
                    t.label
                );
            }
            // くさび（面の傾き）は同じ値
            for k in [Kind::Tx, Kind::Ty] {
                let g = |x: &System, j| x.get_param(Param::new(j, k)).unwrap();
                assert!(
                    (g(q, a) - g(q, b)).abs() < 1e-15,
                    "{}: 面 {a}・{b} の傾き",
                    t.label
                );
            }
            // 面 a の後の媒質 ↔ 面 b−1 の後の媒質: 厚さは逆の符号で同じだけ、屈折率のずれは同じ
            let (ta, tb) = (qa.t - sa.t, q.surfaces[b - 1].t - s.surfaces[b - 1].t);
            assert!(
                (ta + tb).abs() < 1e-12,
                "{}: 面 {a}・{} の厚さ {ta} {tb}",
                t.label,
                b - 1
            );
            assert!(
                (qa.dn - q.surfaces[b - 1].dn).abs() < 1e-15,
                "{}: 面 {a} の屈折率",
                t.label
            );
            assert!(
                (qa.disp - q.surfaces[b - 1].disp).abs() < 1e-15,
                "{}: 面 {a} の分散",
                t.label
            );
        }
    }
    // 偏心・傾きは往復の範囲を持つブロックの動き
    let u = m.tols.iter().find(|t| t.label == "L2 偏心x").unwrap();
    assert_eq!(s.param_name(u.params[0]), "L2.dx");
    // 結果の JSON: 厚さの公差は戻りの面に -1
    let r = lensbench_design::montecarlo::run(&lensbench_design::montecarlo::McSpec {
        trials: 2,
        ..m.clone()
    })
    .unwrap();
    let out = result_json(&m, &r, &out_opts(&json!({})).unwrap());
    let th = out["tolerances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["label"] == "L2 厚さ1")
        .unwrap();
    assert_eq!(th["scale"], json!([1.0, -1.0]));
}

/// くさびの面は玉のもとの向きの後ろ（wedge_face "front" なら前）。裏返して置いた玉・往復する玉でも同じ面
#[test]
fn wedge_face_follows_lens_orientation() {
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    // L2（R80 と平面）。後ろの面は平面
    for rev in [false, true] {
        let src = if rev {
            DP.replace(
                r#"{"type": "lens", "id": "L2","#,
                r#"{"type": "lens", "id": "L2", "reversed": true,"#,
            )
        } else {
            DP.to_string()
        };
        let p = Project::from_json(&src).unwrap();
        let s = expand(&p, &env).unwrap();
        let r = reversed_lenses(&p);
        assert_eq!(r.contains("L2"), rev);
        // 往きと戻りの面がすべて平面（front なら曲面）
        let flat = |v: serde_json::Value, r: &HashSet<String>| -> Vec<bool> {
            let m = spec_from_json_with(&v, vec![s.clone()], &HashMap::new(), r).unwrap();
            let t = m.tols.iter().find(|t| t.label == "L2 くさびx").unwrap();
            assert_eq!(t.params.len(), 2, "往きと戻り");
            t.params
                .iter()
                .map(|q| s.surfaces[q.surf].c == 0.0)
                .collect()
        };
        assert_eq!(flat(json!({}), &r), [true, true], "rev {rev}");
        assert_eq!(flat(json!({"wedge_face": "back"}), &r), [true, true]);
        assert_eq!(flat(json!({"wedge_face": "front"}), &r), [false, false]);
        // 玉ごとの上書き（組の名前で上書きしても全体の wedge_face は残る）
        assert_eq!(
            flat(json!({"lenses": {"L2": {"wedge_face": "front"}}}), &r),
            [false, false]
        );
        assert_eq!(
            flat(
                json!({"wedge_face": "front", "lenses": {"L2": "precision"}}),
                &r
            ),
            [false, false]
        );
        // 系の玉・群に裏返しの印があるので、名前を渡さなくても同じ
        assert_eq!(flat(json!({}), &HashSet::new()), [true, true]);
        // 印の無い系（Python で作った系など）: 名前で渡せば同じ、渡さなければ系の中の面の順の最後の面
        let mut s1 = s.clone();
        s1.units.iter_mut().for_each(|u| u.flipped = false);
        let flat1 = |r: &HashSet<String>| -> Vec<bool> {
            let m = spec_from_json_with(&json!({}), vec![s1.clone()], &HashMap::new(), r).unwrap();
            let t = m.tols.iter().find(|t| t.label == "L2 くさびx").unwrap();
            t.params
                .iter()
                .map(|q| s1.surfaces[q.surf].c == 0.0)
                .collect()
        };
        assert_eq!(flat1(&r), [true, true]);
        assert_eq!(flat1(&HashSet::new()), [!rev, !rev]);
        // id の無い裏返した玉（lens2 …）も同じ（#192）
        let src2 = src.replace(r#""id": "L2", "#, "");
        let p2 = Project::from_json(&src2).unwrap();
        let s2 = expand(&p2, &env).unwrap();
        assert!(reversed_lenses(&p2).is_empty());
        let m = spec_from_json(&json!({}), vec![s2.clone()], &HashMap::new()).unwrap();
        let t = m
            .tols
            .iter()
            .find(|t| t.label.ends_with(" くさびx") && !t.label.starts_with("L1"))
            .unwrap();
        let f: Vec<bool> = t
            .params
            .iter()
            .map(|q| s2.surfaces[q.surf].c == 0.0)
            .collect();
        // id の無い玉は往きと戻りが別の玉（lens2・lens3）になるので、往きの玉の面だけ見る
        assert_eq!(f, [true], "{} rev {rev}", t.label);
    }
    let (_, s) = sys();
    for bad in [
        json!({"wedge_face": "rear"}),
        json!({"lenses": {"L1": {"wedge_face": 1}}}),
    ] {
        assert!(spec_from_json(&bad, vec![s.clone()], &HashMap::new()).is_err());
    }
}

#[test]
fn groups_on_lens_blocks() {
    // 玉 1 つのブロック（往復する玉も）に groups を書くと、玉の公差と別に組立の偏心・傾き・移動を足す
    let p = Project::from_json(DP).unwrap();
    let env = Env {
        base: root(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    let s = expand(&p, &env).unwrap();
    let v = json!({"preset": "none", "groups": {"L1": {"decenter": 0.2, "tilt": 0.1}, "L2": {"shift": 0.05}}});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    let names: Vec<&str> = m.tols.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(
        names,
        [
            "L1 群の偏心x",
            "L1 群の偏心y",
            "L1 群の傾きx",
            "L1 群の傾きy",
            "L2 群の移動"
        ]
    );
    assert_eq!(s.param_name(m.tols[0].params[0]), "L1.dx");
    assert_eq!(s.param_name(m.tols[4].params[0]), "L2.shift");
    // 玉の公差（preset）とは名前を分けて両方入る
    let v = json!({"preset": "commercial", "groups": {"L1": {"decenter": 0.2}}});
    let names = labels(&v, &s, &HashMap::new());
    assert!(names.iter().any(|l| l == "L1 偏心x") && names.iter().any(|l| l == "L1 群の偏心x"));
    // 片道の系の玉 1 つのブロックでも
    let (_, s1) = sys();
    let names = labels(
        &json!({"preset": "none", "groups": {"L1": {"tilt": 0.1}}}),
        &s1,
        &HashMap::new(),
    );
    assert_eq!(names, ["L1 群の傾きx", "L1 群の傾きy"]);
}

#[test]
fn explicit_tol_scale() {
    let (_, s) = sys();
    let v = json!({"preset": "none", "tols": [{"params": ["1.t", "2.t"], "pm": 0.1, "scale": [1, -1]}]});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    assert_eq!(m.tols[0].scale, vec![1.0, -1.0]);
    let q = &lensbench_design::montecarlo::perturbed(&m, &[0.1]).unwrap()[0];
    assert!((q.surfaces[1].t - s.surfaces[1].t - 0.1).abs() < 1e-12);
    assert!((q.surfaces[2].t - s.surfaces[2].t + 0.1).abs() < 1e-12);
    let bad =
        json!({"preset": "none", "tols": [{"params": ["1.t", "2.t"], "pm": 0.1, "scale": [1]}]});
    assert!(spec_from_json(&bad, vec![s], &HashMap::new()).is_err());
}

#[test]
fn compensator_values_tilt_magnitude_and_csv() {
    // 補償子の試行ごとの値（試行の順）・傾き tx・ty の組の大きさ（分）・試行ごとの CSV（#164）
    let (_, s) = sys();
    let v = json!({
        "preset": "none", "trials": 5, "seed": 2, "dist": "uniform", "sensitivity": false,
        "tols": [{"param": "L1.dy", "pm": 0.05}, {"param": "L1.dx", "pm": 0.05}],
        "compensators": ["G.tx=-1:1", "G.ty=-1:1", "G.shift=-2:2"],
        "percentiles": [50, 95],
    });
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    let r = lensbench_design::montecarlo::run(&m).unwrap();
    let out = result_json(&m, &r, &out_opts(&v).unwrap());
    let comps = out["compensators"].as_array().unwrap();
    assert_eq!(comps.len(), 3);
    for (q, c) in comps.iter().enumerate() {
        let vals = c["values"].as_array().unwrap();
        assert_eq!(vals.len(), 5);
        for (t, x) in r.trials.iter().zip(vals) {
            assert_eq!(x.as_f64(), t.eval.as_ref().map(|e| e.comp[q]));
        }
    }
    let tm = out["tilt_magnitudes"].as_array().unwrap();
    assert_eq!(tm.len(), 1);
    assert_eq!(tm[0]["name"], "G.tilt");
    assert_eq!(tm[0]["x"], "G.tx");
    assert_eq!(tm[0]["y"], "G.ty");
    assert_eq!(tm[0]["unit"], "arcmin");
    assert_eq!(tm[0]["stats"]["percentiles"].as_array().unwrap().len(), 2);
    for (i, t) in r.trials.iter().enumerate() {
        let e = t.eval.as_ref().unwrap();
        let want = 60.0 * e.comp[0].hypot(e.comp[1]);
        assert!((tm[0]["values"][i].as_f64().unwrap() - want).abs() < 1e-12);
    }
    let merits = out["merit_values"].as_array().unwrap();
    assert_eq!(merits.len(), 5);
    let csv = compensators_csv(&out).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "trial,merit,G.tx,G.ty,G.shift,G.tilt [arcmin]");
    assert_eq!(lines.len(), 6);
    let row: Vec<f64> = lines[3].split(',').map(|x| x.parse().unwrap()).collect();
    assert_eq!(row[0], 3.0);
    assert_eq!(row[1], merits[2].as_f64().unwrap());
    assert_eq!(row[4], comps[2]["values"][2].as_f64().unwrap());
    // 構成の違う tx・ty は組にしない。CSV は結果に試行の値がないと誤り
    let names = vec!["5.tx".to_string(), "5.ty".to_string(), "G.tx".to_string()];
    let p = s.param("5.tx").unwrap();
    let vars = vec![
        lensbench_core::optim::Var::new(p),
        lensbench_core::optim::Var::new(p).in_configs(vec![0]),
        lensbench_core::optim::Var::new(p),
    ];
    assert!(tilt_pairs(&vars, &names).is_empty());
    assert_eq!(tilt_pairs(&vars[..1], &names[..1]).len(), 0);
    assert!(compensators_csv(&json!({"compensators": []})).is_err());
}

/// 光源の波長のずれ（#163）: "wave" はすべての波長、"wave1" は 1 つ
#[test]
fn wavelength_tolerance_param() {
    let (_, s) = sys();
    let v = json!({"preset": "none", "tols": [
        {"param": "wave", "pm": 0.01},
        {"param": "wave1", "pm": 0.005, "label": "λ1"}
    ]});
    let m = spec_from_json(&v, vec![s.clone()], &HashMap::new()).unwrap();
    assert_eq!(m.tols.len(), 2);
    assert_eq!(m.tols[0].params.len(), s.wavelengths.len());
    assert_eq!(m.tols[0].label, "wave");
    assert_eq!((m.tols[0].lo, m.tols[0].hi), (-0.01, 0.01));
    assert_eq!(m.tols[1].params.len(), 1);
    assert!(spec_from_json(
        &json!({"preset": "none", "tols": [{"param": "wave2", "pm": 0.01}]}),
        vec![s.clone()],
        &HashMap::new()
    )
    .is_err());
}
