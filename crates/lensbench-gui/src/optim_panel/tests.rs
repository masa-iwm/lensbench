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
