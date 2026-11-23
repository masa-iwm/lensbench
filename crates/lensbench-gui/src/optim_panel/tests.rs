//! 最適化のパネルの試験

use super::*;

/// 欄の無い鍵の値は、表を通して保存しても読んだ形のまま（数に見える文字列も文字列のまま）
#[test]
fn columnless_keys_keep_their_json_type() {
    let v = json!([{"type": "paraxial", "q": "efl", "min": 1.0,
        "note": "3", "flag": true, "n": 3, "list": [1, 2], "text": "abc", "quoted": "\"x\""}]);
    let rows = term_rows(&v).unwrap();
    let back = term_json(&rows[0]).unwrap();
    for k in ["note", "flag", "n", "list", "text", "quoted"] {
        assert_eq!(back[k], v[0][k], "{k}");
    }
}

/// DLS の設定（QR・勾配・境界・止まった光線）は既定と違うときだけ保存し、読み戻せる
#[test]
fn dls_settings_round_trip() {
    let mut p = OptPanel::new();
    let v = p.params().unwrap();
    for k in ["qr", "gtol", "active_set", "stopped_as_vignetted"] {
        assert!(v.get(k).is_none(), "{k}");
    }
    p.qr = true;
    p.active_set = true;
    p.stopped_as_vignetted = true;
    p.gtol = 1e-6;
    let v = p.params().unwrap();
    let d: lensbench_design::dls_spec::OptimizeParams = serde_json::from_value(v.clone()).unwrap();
    assert!(d.qr && d.active_set && d.stopped_as_vignetted);
    assert_eq!(d.gtol, 1e-6);
    let mut q = OptPanel::new();
    q.set_params(&v).unwrap();
    assert!(q.qr && q.active_set && q.stopped_as_vignetted);
    assert_eq!(q.gtol, 1e-6);
}

/// 範囲の境界だけの項（semi・each の thickness）も欄から min・max を書ける
#[test]
fn bound_fields() {
    let mut t = TermRow::new("semi");
    t.set("max", "5");
    assert_eq!(
        term_json(&t).unwrap(),
        json!({"type": "semi", "surf": 1, "max": 5.0})
    );
    let mut t = TermRow::new("thickness");
    t.set("each", "air");
    t.set("min", "0.5");
    assert_eq!(
        term_json(&t).unwrap(),
        json!({"type": "thickness", "surf": 1, "each": "air", "min": 0.5})
    );
}

/// 評価関数の単位は項が同じ単位のときだけ（スポットは µm、波面は波、混ざると無し）
#[test]
fn gux20_merit_unit() {
    let mut p = OptPanel {
        merit: "spot".into(),
        ..Default::default()
    };
    assert_eq!(merit_unit(&p), (1000.0, "µm"));
    p.terms.push(TermRow::new("centroid"));
    assert_eq!(merit_unit(&p).1, "µm");
    p.terms.push(TermRow::new("wavefront"));
    assert_eq!(merit_unit(&p).1, "");
    p.merit = "none".into();
    p.terms.retain(|t| t.ty == "wavefront");
    assert_eq!(merit_unit(&p).1, tr("waves"));
    p.terms.clear();
    assert_eq!(merit_unit(&p).1, "");
}

/// 変数の行に面の値（t・r・c・k）の今の値を出す
#[test]
fn gux21_var_value_reads_surface_rows() {
    let row = |r: f64, t: f64| crate::lens_data::Row {
        r,
        t,
        k: -1.0,
        ..Default::default()
    };
    let rows = vec![row(f64::INFINITY, 0.0), row(50.0, 4.0)];
    assert_eq!(var_value(&rows, "1.t"), Some(4.0));
    assert_eq!(var_value(&rows, " 1.r"), Some(50.0));
    assert_eq!(var_value(&rows, "1.c"), Some(0.02));
    assert_eq!(var_value(&rows, "1.k"), Some(-1.0));
    assert_eq!(var_value(&rows, "0.r"), Some(f64::INFINITY));
    assert_eq!(var_value(&rows, "0.c"), Some(0.0));
    assert_eq!(var_value(&rows, "5.t"), None);
    assert_eq!(var_value(&rows, "G1.shift"), None);
}

#[test]
fn params_round_trip() {
    let mut p = OptPanel::new();
    p.vars.push(VarRow {
        name: "7.t".into(),
        lo: "0.5".into(),
        hi: "".into(),
        configs: "1, 2".into(),
    });
    let mut t = TermRow::new("ray");
    t.set("field", "0, 0.5");
    t.weight = "2".into();
    p.terms.push(t);
    let mut q = TermRow::new("paraxial");
    q.set("target", "50");
    p.terms.push(q);
    let mut z = TermRow::new("zplm");
    z.set("macro", "lbEval");
    z.set("data", "3");
    z.set("macro_dirs", "C:\\a, b; /c");
    p.terms.push(z);
    let mut z = TermRow::new("zplm");
    z.set("macro", "7");
    p.terms.push(z);
    let mut g = TermRow::new("plugin");
    g.set("args", r#"{"f": 60, "passes": 1}"#);
    g.set("target", "0.001");
    p.terms.push(g);
    p.configs = "1, 2".into();
    p.merit = "none".into();
    let v = p.params().unwrap();
    assert_eq!(
        v["vars"][0],
        json!({"name": "7.t", "lo": 0.5, "configs": [1, 2]})
    );
    assert_eq!(
        v["terms"][0],
        json!({"type": "ray", "field": [0.0, 0.5], "py": 1.0, "q": "y", "target": 0.0, "weight": 2.0})
    );
    assert_eq!(
        v["terms"][1],
        json!({"type": "paraxial", "q": "efl", "target": 50.0})
    );
    assert_eq!(
        v["terms"][2],
        json!({"type": "zplm", "macro": "lbEval", "data": 3, "macro_dirs": ["C:\\a, b", "/c"]})
    );
    assert_eq!(
        v["terms"][3],
        json!({"type": "zplm", "macro": 7, "data": 0})
    );
    assert_eq!(v["configs"], json!([1, 2]));
    // 項の JSON は CLI と同じ読み方で読める
    let mut back = OptPanel::new();
    back.set_params(&v).unwrap();
    assert_eq!(back.params().unwrap(), v);
    // 間違い
    let mut bad = OptPanel::new();
    let mut t = TermRow::new("paraxial");
    t.set("target", "abc");
    bad.terms.push(t);
    assert!(bad.params().is_err());
    let mut bad = OptPanel::new();
    let mut t = TermRow::new("plugin");
    t.set("args", "[1, 2]");
    bad.terms.push(t);
    assert!(bad.params().is_err());
}

#[test]
fn combo_edits() {
    assert_eq!(combo_edit("7.t", 2.0).unwrap(), (7, "t", json!(2.0)));
    assert_eq!(combo_edit("3.c", 0.02).unwrap(), (3, "r", json!(50.0)));
    assert_eq!(combo_edit("3.c", 0.0).unwrap(), (3, "r", json!(0.0)));
    assert!(combo_edit("lens1.tx", 1.0).is_err());
    assert!(combo_edit("3.dx", 1.0).is_err());
}

/// 大域の探索（始点が 2 以上のときだけ）とガラスの置き換えの設定を保存し、読み戻せる。設定の型として読める
#[test]
fn search_settings_round_trip() {
    let mut p = OptPanel::new();
    let v = p.params().unwrap();
    for k in ["starts", "seed", "keep", "spread", "glass"] {
        assert!(v.get(k).is_none(), "{k}");
    }
    p.starts = 12;
    p.seed = 7;
    p.keep = 3;
    p.glass_on = true;
    // 面が無ければ誤り
    assert!(p.params().is_err());
    p.glass_surfaces = "1, 3,6".into();
    p.glass_catalogs = "SCHOTT, OHARA".into();
    p.glass_candidates = 5;
    let v = p.params().unwrap();
    let d: lensbench_design::dls_spec::OptimizeParams = serde_json::from_value(v.clone()).unwrap();
    assert_eq!((d.starts, d.seed, d.keep), (12, 7, 3));
    let g = d.glass.unwrap();
    assert_eq!(g.surfaces, vec![1, 3, 6]);
    assert_eq!(g.catalogs, vec!["SCHOTT", "OHARA"]);
    assert_eq!((g.candidates, g.passes), (5, 2));
    let mut q = OptPanel::new();
    q.set_params(&v).unwrap();
    assert_eq!((q.starts, q.seed, q.keep), (12, 7, 3));
    assert!(q.glass_on);
    assert_eq!(q.glass_surfaces, "1, 3, 6");
    assert_eq!(q.glass_catalogs, "SCHOTT, OHARA");
    assert_eq!(q.glass_candidates, 5);
}

/// 結果の置き換えた硝材: 面ごとに最後の材料、元に戻すときは最初の材料
#[test]
fn result_glasses_first_and_last() {
    use lensbench_engine::session::views::{GlassChangeView, GlassView};
    let ch = |j: usize, from: &str, to: &str| GlassChangeView {
        surface: j,
        from: from.into(),
        to: to.into(),
        ..Default::default()
    };
    let r = OptimizeView {
        glass: Some(GlassView {
            changes: vec![
                ch(1, "1.6,58", "N-SK16"),
                ch(3, "F2", "N-F2"),
                ch(1, "N-SK16", "N-SK14"),
            ],
            materials: vec![(1, "N-SK14".into()), (3, "N-F2".into())],
            ..Default::default()
        }),
        ..Default::default()
    };
    let one = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
    assert_eq!(
        result_glass_configs(&r, 0),
        vec![
            GlassApply {
                surface: 1,
                per: vec![one("1.6,58", "N-SK14")]
            },
            GlassApply {
                surface: 3,
                per: vec![one("F2", "N-F2")]
            },
        ]
    );
    assert!(result_glass_configs(&OptimizeView::default(), 1).is_empty());
}

/// ガラスの置き換えの候補の絞り込みと近さの刻みを保存し、読み戻せる。数でない上限は誤り
#[test]
fn glass_filter_settings_round_trip() {
    let mut p = OptPanel::new();
    p.glass_on = true;
    p.glass_surfaces = "3".into();
    let v = p.params().unwrap();
    for k in [
        "max_cost",
        "max_cr",
        "status",
        "exclude",
        "nd_step",
        "dpgf_step",
    ] {
        assert!(v["glass"].get(k).is_none(), "{k}");
    }
    p.glass_max_cost = "2.5".into();
    p.glass_max_resist[0] = "2".into();
    p.glass_max_resist[4] = "3".into();
    p.glass_status = [true, true, false, false, false];
    p.glass_exclude = "N-LASF9, F2@OHARA".into();
    p.glass_nd_step = 0.02;
    p.glass_dpgf_step = "0.005".into();
    let v = p.params().unwrap();
    let d: lensbench_design::dls_spec::OptimizeParams = serde_json::from_value(v.clone()).unwrap();
    let g = d.glass.unwrap();
    assert_eq!(
        (g.max_cost, g.max_cr, g.max_fr, g.max_pr),
        (Some(2.5), Some(2.0), None, Some(3.0))
    );
    assert_eq!(g.status, vec!["standard", "preferred"]);
    assert_eq!(g.exclude, vec!["N-LASF9", "F2@OHARA"]);
    assert_eq!(
        (g.nd_step, g.vd_step, g.dpgf_step),
        (0.02, 1.0, Some(0.005))
    );
    let mut q = OptPanel::new();
    q.set_params(&v).unwrap();
    assert_eq!(q.glass_max_cost, "2.5");
    assert_eq!(q.glass_max_resist, ["2", "", "", "", "3"].map(String::from));
    assert_eq!(q.glass_status, p.glass_status);
    assert_eq!(q.glass_exclude, "N-LASF9, F2@OHARA");
    assert_eq!(
        (q.glass_nd_step, q.glass_dpgf_step.as_str()),
        (0.02, "0.005")
    );
    p.glass_max_cost = "x".into();
    assert!(p.params().is_err());
}

/// 構成ごとの置き換え: 構成ごとの最後の材料と、置き換えなかった構成は None。固定の dn・disp は 0 にし、元に戻すときは前の値
#[test]
fn result_glasses_configs_and_resets() {
    use lensbench_engine::session::views::{GlassChangeView, GlassView};
    let r = OptimizeView {
        glass: Some(GlassView {
            changes: vec![
                GlassChangeView {
                    surface: 3,
                    from: "1.7,30".into(),
                    to: "N-F2".into(),
                    configs: Some(vec![2]),
                    ..Default::default()
                },
                GlassChangeView {
                    surface: 6,
                    from: "N-SK16".into(),
                    to: "N-SK14".into(),
                    configs: Some(vec![1, 3]),
                    ..Default::default()
                },
            ],
            materials: vec![(3, "F2".into()), (6, "N-SK14".into())],
            config_materials: vec![(3, vec!["F2".into(), "N-F2".into(), "F2".into()])],
            reset: vec![("6.dn".into(), 0.01)],
            ..Default::default()
        }),
        ..Default::default()
    };
    let one = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
    assert_eq!(
        result_glass_configs(&r, 3),
        vec![
            GlassApply {
                surface: 3,
                per: vec![None, one("1.7,30", "N-F2"), None]
            },
            GlassApply {
                surface: 6,
                per: vec![one("N-SK16", "N-SK14"), None, one("N-SK16", "N-SK14")]
            },
        ]
    );
    assert_eq!(result_resets(&r, false), vec![("6.dn".to_string(), 0.0)]);
    assert_eq!(result_resets(&r, true), vec![("6.dn".to_string(), 0.01)]);
}

/// 状態への置き換えの書き方: 基本の系にも書く・状態だけに書く・前の硝材が分からない状態があるとき
#[test]
fn plan_glass_writes_base_and_states() {
    let s = |x: &str| x.to_string();
    let users = |v: &[(&str, usize)]| v.iter().map(|(n, c)| (s(n), *c)).collect::<Vec<_>>();
    let hits = |v: &[(&str, &str, &str)]| {
        v.iter()
            .map(|(n, a, b)| (s(n), s(a), s(b)))
            .collect::<Vec<_>>()
    };
    // 面の材料に従う状態がどれも同じ硝材に: 面の材料だけ（上書きを持つ C はその上書き）
    let p = plan_glass(
        &users(&[("A", 1), ("B", 1)]),
        &hits(&[("A", "X", "Y"), ("B", "X", "Y"), ("C", "Z", "W")]),
        1,
        false,
    );
    assert_eq!(p.base, Some((s("X"), s("Y"))));
    assert_eq!(p.writes, vec![(s("C"), s("W"))]);
    // 違う硝材に: 面は基本の構成の最初の状態 A の硝材、B は上書き
    let u = users(&[("A", 1), ("B", 1)]);
    let h = hits(&[("A", "X", "Y"), ("B", "X", "V")]);
    let p = plan_glass(&u, &h, 1, false);
    assert_eq!(p.base, Some((s("X"), s("Y"))));
    assert_eq!(p.writes, vec![(s("B"), s("V"))]);
    // 状態だけに書く
    let p = plan_glass(&u, &h, 1, true);
    assert_eq!(p.base, None);
    assert_eq!(p.writes, vec![(s("A"), s("Y")), (s("B"), s("V"))]);
    assert!(!p.base_kept);
    // 置き換えなかった状態 D（同じ構成）は前の硝材で上書きして保つ
    let p = plan_glass(
        &users(&[("A", 1), ("D", 1)]),
        &hits(&[("A", "X", "Y")]),
        1,
        false,
    );
    assert_eq!(p.base, Some((s("X"), s("Y"))));
    assert_eq!(p.writes, vec![(s("D"), s("X"))]);
    // 置き換えなかった状態 E が別の構成: 前の硝材が分からないので状態だけ
    let p = plan_glass(
        &users(&[("A", 1), ("B", 2), ("E", 3)]),
        &hits(&[("A", "X", "Y"), ("B", "Z", "V")]),
        1,
        false,
    );
    assert_eq!(p.base, None);
    assert!(p.base_kept);
    assert_eq!(p.writes, vec![(s("A"), s("Y")), (s("B"), s("V"))]);
    // 基本の構成の状態を置き換えなかった: 基本の系はそのまま
    let p = plan_glass(
        &users(&[("A", 1), ("B", 2)]),
        &hits(&[("B", "Z", "V")]),
        1,
        false,
    );
    assert_eq!(p.base, None);
    assert!(!p.base_kept);
    assert_eq!(p.writes, vec![(s("B"), s("V"))]);
}
